use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum CliError {
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("API error: {0}")]
    Api(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Room not found: {0}")]
    RoomNotFound(String),

    #[error("Not a member of room: {0}")]
    NotMember(String),
}

/// Exit status when a long-running command stops because River re-keyed the
/// room contract underneath it.
///
/// 75 is `EX_TEMPFAIL` from sysexits.h: "a temporary failure; the user is
/// invited to retry". That is exactly the contract — restart riverctl and it
/// follows the new generation — and it lets a supervisor or wrapper script tell
/// "restart me" apart from a real failure without parsing stderr.
pub const EXIT_ROOM_CONTRACT_REKEYED: u8 = 75;

/// River re-keyed the room contract while a long-running command was running.
///
/// Not a failure of the command. The process stops so it can be restarted, and
/// the restarted process follows the new generation through the normal startup
/// path. See [`crate::pointer::Recheck`].
#[derive(Error, Debug)]
#[error(
    "River re-keyed the room contract while this command was running.\n  \
     was: {from}\n  now: {to}\n\
     Exiting with status {code} so it can be restarted; a restarted riverctl follows the new \
     generation automatically.",
    code = EXIT_ROOM_CONTRACT_REKEYED
)]
pub struct RoomContractRekeyed {
    pub from: String,
    pub to: String,
}

/// The process exit status for a command that ended in `err`.
///
/// Split out of `main` so the mapping is testable. Anything other than a re-key
/// is status 1, which is what `main` returning `Err` produced before this
/// existed, so no other failure changes its status.
pub fn exit_code_for(err: &anyhow::Error) -> u8 {
    if err.downcast_ref::<RoomContractRekeyed>().is_some() {
        EXIT_ROOM_CONTRACT_REKEYED
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rekeyed() -> RoomContractRekeyed {
        RoomContractRekeyed {
            from: "old".to_string(),
            to: "new".to_string(),
        }
    }

    #[test]
    fn a_rekey_exits_with_the_restart_status_and_everything_else_with_one() {
        assert_eq!(exit_code_for(&rekeyed().into()), 75);
        assert_eq!(exit_code_for(&anyhow::anyhow!("an ordinary failure")), 1);
    }

    /// Commands add `.context(...)` as errors propagate. If that hid the re-key,
    /// a supervisor would see status 1 and treat a routine restart as a crash.
    #[test]
    fn the_restart_status_survives_added_context() {
        let wrapped = anyhow::Error::from(rekeyed()).context("while streaming messages");
        assert_eq!(exit_code_for(&wrapped), 75);
    }

    /// The message names both generations and says what happens next, because
    /// the operator's first question is whether this is a crash.
    #[test]
    fn the_message_says_it_is_a_restart_not_a_crash() {
        let msg = rekeyed().to_string();
        assert!(
            msg.contains("was: old") && msg.contains("now: new"),
            "{msg}"
        );
        assert!(msg.contains("status 75"), "{msg}");
        assert!(msg.contains("restarted"), "{msg}");
    }
}
