use crate::state::member::{MemberId, AuthorizedMember, Member, MembersV1};
use crate::state::ChatRoomParametersV1;
use crate::util::{sign_struct, verify_struct};
use crate::ChatRoomStateV1;
use crate::state::configuration::AuthorizedConfigurationV1;
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use freenet_scaffold::util::{fast_hash, FastHash};
use freenet_scaffold::ComposableState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::SystemTime;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BansV1(pub Vec<AuthorizedUserBan>);

impl BansV1 {
    fn get_invalid_bans(
        &self,
        parent_state: &ChatRoomStateV1,
        parameters: &ChatRoomParametersV1,
    ) -> HashMap<BanId, String> {
        let member_map = parent_state.members.members_by_member_id();
        let mut invalid_bans = HashMap::new();

        for ban in &self.0 {
            let banning_member = match member_map.get(&ban.banned_by) {
                Some(member) => member,
                None => {
                    invalid_bans.insert(
                        ban.id(),
                        "Banning member not found in member list".to_string(),
                    );
                    continue;
                }
            };

            let banned_member = match member_map.get(&ban.ban.banned_user) {
                Some(member) => member,
                None => {
                    invalid_bans.insert(
                        ban.id(),
                        "Banned member not found in member list".to_string(),
                    );
                    continue;
                }
            };

            if ban.banned_by != parameters.owner_id() {
                // No need to check invite chain if banner is owner
                let member_invite_chain = match parent_state
                    .members
                    .get_invite_chain(banning_member, parameters)
                {
                    Ok(chain) => chain,
                    Err(e) => {
                        invalid_bans.insert(ban.id(), format!("Error getting invite chain: {}", e));
                        continue;
                    }
                };

                if !member_invite_chain
                    .iter()
                    .any(|m| m.member.id() == banned_member.member.id())
                {
                    invalid_bans.insert(
                        ban.id(),
                        "Banner is not in the invite chain of the banned member".to_string(),
                    );
                    continue;
                }
            }
        }

        let extra_bans =
            self.0.len() as isize - parent_state.configuration.configuration.max_user_bans as isize;
        if extra_bans > 0 {
            // Add oldest extra bans to invalid bans
            let mut extra_bans_vec = self.0.clone();
            extra_bans_vec.sort_by_key(|ban| ban.ban.banned_at);
            extra_bans_vec.reverse();
            for ban in extra_bans_vec.iter().take(extra_bans as usize) {
                invalid_bans.insert(ban.id(), "Exceeded maximum number of user bans".to_string());
            }
        }

        invalid_bans
    }
}

impl Default for BansV1 {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl ComposableState for BansV1 {
    type ParentState = ChatRoomStateV1;
    type Summary = Vec<BanId>;
    type Delta = Vec<AuthorizedUserBan>;
    type Parameters = ChatRoomParametersV1;

    fn verify(
        &self,
        parent_state: &Self::ParentState,
        parameters: &Self::Parameters,
    ) -> Result<(), String> {
        if !self.get_invalid_bans(parent_state, parameters).is_empty() {
            return Err("Invalid bans".to_string());
        }

        Ok(())
    }

    fn summarize(
        &self,
        _parent_state: &Self::ParentState,
        _parameters: &Self::Parameters,
    ) -> Self::Summary {
        self.0.iter().map(|ban| ban.id()).collect()
    }

    fn delta(
        &self,
        _parent_state: &Self::ParentState,
        _parameters: &Self::Parameters,
        old_state_summary: &Self::Summary,
    ) -> Option<Self::Delta> {
        // Identify bans in self.0 that are not in old_state_summary
        let delta = self.0
            .iter()
            .filter(|ban| !old_state_summary.contains(&ban.id()))
            .cloned()
            .collect::<Vec<_>>();
        if delta.is_empty() {
            None
        } else {
            Some(delta)
        }
    }

    fn apply_delta(
        &mut self,
        parent_state: &Self::ParentState,
        parameters: &Self::Parameters,
        delta: &Self::Delta,
    ) -> Result<(), String> {
        // Create a temporary BansV1 with the new bans
        let mut temp_bans = self.clone();
        temp_bans.0.extend(delta.iter().cloned());

        // Verify the temporary state
        if let Err(e) = temp_bans.verify(parent_state, parameters) {
            return Err(format!("Invalid delta: {}", e));
        }

        // If verification passes, update the actual state
        self.0 = temp_bans.0;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AuthorizedUserBan {
    pub ban: UserBan,
    pub banned_by: MemberId,
    pub signature: Signature,
}

impl Eq for AuthorizedUserBan {}

impl Hash for AuthorizedUserBan {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.signature.to_bytes().hash(state);
    }
}

impl AuthorizedUserBan {
    pub fn new(ban: UserBan, banned_by: MemberId, banner_signing_key: &SigningKey) -> Self {
        assert_eq!(
            MemberId::new(&banner_signing_key.verifying_key()),
            banned_by
        );

        let signature = sign_struct(&ban, banner_signing_key);

        Self {
            ban,
            banned_by,
            signature,
        }
    }

    pub fn verify_signature(&self, banner_verifying_key: &VerifyingKey) -> Result<(), String> {
        verify_struct(&self.ban, &self.signature, banner_verifying_key)
            .map_err(|e| format!("Invalid ban signature: {}", e))
    }

    pub fn id(&self) -> BanId {
        BanId(fast_hash(&self.signature.to_bytes()))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UserBan {
    pub owner_member_id: MemberId,
    pub banned_at: SystemTime,
    pub banned_user: MemberId,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Hash, Debug)]
pub struct BanId(pub FastHash);

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use std::time::Duration;

    fn create_test_chat_room_state() -> ChatRoomStateV1 {
        // Create a minimal ChatRoomStateV1 for testing
        ChatRoomStateV1 {
            configuration: AuthorizedConfigurationV1::default(),
            members: MembersV1::default(),
            member_info: Default::default(),
            recent_messages: Default::default(),
            upgrade: Default::default(),
            bans: Default::default(),
        }
    }

    fn create_test_parameters() -> ChatRoomParametersV1 {
        // Create minimal ChatRoomParametersV1 for testing
        ChatRoomParametersV1 {
            owner: MemberId::new(&SigningKey::generate(&mut rand::thread_rng()).verifying_key()),
        }
    }

    #[test]
    fn test_bans_verify() {
        let mut state = create_test_chat_room_state();
        let params = create_test_parameters();

        // Create some test members
        let owner_key = SigningKey::generate(&mut rand::thread_rng());
        let owner_id = MemberId::new(&owner_key.verifying_key());
        let member1_key = SigningKey::generate(&mut rand::thread_rng());
        let member1_id = MemberId::new(&member1_key.verifying_key());
        let member2_key = SigningKey::generate(&mut rand::thread_rng());
        let member2_id = MemberId::new(&member2_key.verifying_key());

        // Add members to the state
        state.members.members.push(AuthorizedMember::new(Member::new(owner_id.clone()), &owner_key));
        state.members.members.push(AuthorizedMember::new(Member::new(member1_id.clone()), &member1_key));
        state.members.members.push(AuthorizedMember::new(Member::new(member2_id.clone()), &member2_key));

        // Test 1: Valid ban by owner
        let ban1 = AuthorizedUserBan::new(
            UserBan {
                owner_member_id: owner_id.clone(),
                banned_at: SystemTime::now(),
                banned_user: member1_id.clone(),
            },
            owner_id.clone(),
            &owner_key,
        );

        let bans = BansV1(vec![ban1]);
        assert!(bans.verify(&state, &params).is_ok());

        // Test 2: Invalid ban (banning member not in member list)
        let invalid_key = SigningKey::generate(&mut rand::thread_rng());
        let invalid_id = MemberId::new(&invalid_key.verifying_key());
        let invalid_ban = AuthorizedUserBan::new(
            UserBan {
                owner_member_id: owner_id.clone(),
                banned_at: SystemTime::now(),
                banned_user: member2_id.clone(),
            },
            invalid_id,
            &invalid_key,
        );

        let invalid_bans = BansV1(vec![invalid_ban]);
        assert!(invalid_bans.verify(&state, &params).is_err());

        // Test 3: Exceeding max_user_bans
        let mut many_bans = Vec::new();
        for _ in 0..6 {
            many_bans.push(AuthorizedUserBan::new(
                UserBan {
                    owner_member_id: owner_id.clone(),
                    banned_at: SystemTime::now(),
                    banned_user: member1_id.clone(),
                },
                owner_id.clone(),
                &owner_key,
            ));
        }
        let too_many_bans = BansV1(many_bans);
        assert!(too_many_bans.verify(&state, &params).is_err());
    }

    #[test]
    fn test_bans_summarize() {
        let state = create_test_chat_room_state();
        let params = create_test_parameters();

        let key = SigningKey::generate(&mut rand::thread_rng());
        let id = MemberId::new(&key.verifying_key());

        let ban1 = AuthorizedUserBan::new(
            UserBan {
                owner_member_id: id.clone(),
                banned_at: SystemTime::now(),
                banned_user: id.clone(),
            },
            id.clone(),
            &key,
        );

        let ban2 = AuthorizedUserBan::new(
            UserBan {
                owner_member_id: id.clone(),
                banned_at: SystemTime::now() + Duration::from_secs(1),
                banned_user: id.clone(),
            },
            id.clone(),
            &key,
        );

        let bans = BansV1(vec![ban1.clone(), ban2.clone()]);
        let summary = bans.summarize(&state, &params);

        assert_eq!(summary.len(), 2);
        assert!(summary.contains(&ban1.id()));
        assert!(summary.contains(&ban2.id()));
    }

    #[test]
    fn test_bans_delta() {
        let state = create_test_chat_room_state();
        let params = create_test_parameters();

        let key = SigningKey::generate(&mut rand::thread_rng());
        let id = MemberId::new(&key.verifying_key());

        let ban1 = AuthorizedUserBan::new(
            UserBan {
                owner_member_id: id.clone(),
                banned_at: SystemTime::now(),
                banned_user: id.clone(),
            },
            id.clone(),
            &key,
        );

        let ban2 = AuthorizedUserBan::new(
            UserBan {
                owner_member_id: id.clone(),
                banned_at: SystemTime::now() + Duration::from_secs(1),
                banned_user: id.clone(),
            },
            id.clone(),
            &key,
        );

        let bans = BansV1(vec![ban1.clone(), ban2.clone()]);
        
        // Test 1: Empty old summary
        let empty_summary = Vec::new();
        let delta = bans.delta(&state, &params, &empty_summary);
        assert_eq!(delta, Some(vec![ban1.clone(), ban2.clone()]));

        // Test 2: Partial old summary
        let partial_summary = vec![ban1.id()];
        let delta = bans.delta(&state, &params, &partial_summary);
        assert_eq!(delta, Some(vec![ban2.clone()]));

        // Test 3: Full old summary
        let full_summary = vec![ban1.id(), ban2.id()];
        let delta = bans.delta(&state, &params, &full_summary);
        assert_eq!(delta, None);
    }

    #[test]
    fn test_bans_apply_delta() {
        let mut state = create_test_chat_room_state();
        let params = create_test_parameters();

        let owner_key = SigningKey::generate(&mut rand::thread_rng());
        let owner_id = MemberId::new(&owner_key.verifying_key());
        let member_key = SigningKey::generate(&mut rand::thread_rng());
        let member_id = MemberId::new(&member_key.verifying_key());

        // Add members to the state
        state.members.members.push(AuthorizedMember::new(Member::new(owner_id.clone()), &owner_key));
        state.members.members.push(AuthorizedMember::new(Member::new(member_id.clone()), &member_key));

        let mut bans = BansV1::default();

        let new_ban = AuthorizedUserBan::new(
            UserBan {
                owner_member_id: owner_id.clone(),
                banned_at: SystemTime::now(),
                banned_user: member_id.clone(),
            },
            owner_id.clone(),
            &owner_key,
        );

        // Test 1: Apply valid delta
        let delta = vec![new_ban.clone()];
        assert!(bans.apply_delta(&state, &params, &delta).is_ok());
        assert_eq!(bans.0.len(), 1);
        assert_eq!(bans.0[0], new_ban);

        // Test 2: Apply invalid delta (duplicate ban)
        let invalid_delta = vec![new_ban.clone()];
        assert!(bans.apply_delta(&state, &params, &invalid_delta).is_err());
        assert_eq!(bans.0.len(), 1); // State should not change

        // Test 3: Apply delta exceeding max_user_bans
        let mut many_bans = Vec::new();
        for _ in 0..5 {
            many_bans.push(AuthorizedUserBan::new(
                UserBan {
                    owner_member_id: owner_id.clone(),
                    banned_at: SystemTime::now(),
                    banned_user: member_id.clone(),
                },
                owner_id.clone(),
                &owner_key,
            ));
        }
        assert!(bans.apply_delta(&state, &params, &many_bans).is_err());
        assert_eq!(bans.0.len(), 1); // State should not change
    }

    #[test]
    fn test_authorized_user_ban() {
        let owner_key = SigningKey::generate(&mut rand::thread_rng());
        let owner_id = MemberId::new(&owner_key.verifying_key());
        let member_key = SigningKey::generate(&mut rand::thread_rng());
        let member_id = MemberId::new(&member_key.verifying_key());

        let ban = UserBan {
            owner_member_id: owner_id.clone(),
            banned_at: SystemTime::now(),
            banned_user: member_id.clone(),
        };

        let authorized_ban = AuthorizedUserBan::new(ban.clone(), owner_id.clone(), &owner_key);

        // Test 1: Verify signature
        assert!(authorized_ban.verify_signature(&owner_key.verifying_key()).is_ok());

        // Test 2: Verify signature with wrong key
        let wrong_key = SigningKey::generate(&mut rand::thread_rng());
        assert!(authorized_ban.verify_signature(&wrong_key.verifying_key()).is_err());

        // Test 3: Check ban ID
        let id1 = authorized_ban.id();
        let id2 = authorized_ban.id();
        assert_eq!(id1, id2);

        // Test 4: Different bans should have different IDs
        let another_ban = AuthorizedUserBan::new(
            UserBan {
                owner_member_id: owner_id.clone(),
                banned_at: SystemTime::now() + Duration::from_secs(1),
                banned_user: member_id.clone(),
            },
            owner_id.clone(),
            &owner_key,
        );
        assert_ne!(authorized_ban.id(), another_ban.id());
    }
}
