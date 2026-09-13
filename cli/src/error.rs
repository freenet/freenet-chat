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

/// River's pointer names a different room-contract generation than the one a
/// long-running command started with — usually because River re-keyed while it
/// ran, sometimes because the command started on a fallback it could not verify.
///
/// Not a failure of the command. The process stops so it can be restarted, and
/// the restarted process follows the new generation through the normal startup
/// path. See [`crate::pointer::Recheck`].
#[derive(Error, Debug)]
#[error(
    "River's pointer now names a different room-contract generation than this command started \
     with.\n  was: {from}\n  now: {to}\n\
     Exiting with status {code} so it can be restarted; a restarted riverctl uses the current \
     generation.",
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

/// What `main` writes to stderr, and the status it exits with, for a command that
/// ended in `err`.
///
/// Split out of `main` so the whole mapping is testable, not just the status. A
/// re-key prints its own message rather than the `Error:` debug dump, because it
/// is a restart request and should not read as a crash. Every other error prints
/// `Error: {err:?}` and exits 1 — exactly what `main` returning `Result` produced
/// before, so no other command's output or status changes.
pub fn report(err: &anyhow::Error) -> (String, u8) {
    match err.downcast_ref::<RoomContractRekeyed>() {
        Some(rekeyed) => (rekeyed.to_string(), EXIT_ROOM_CONTRACT_REKEYED),
        None => (format!("Error: {err:?}"), exit_code_for(err)),
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

    /// A re-key prints its own message, names both generations, and says what
    /// happens next — because the operator's first question is whether it crashed.
    #[test]
    fn a_rekey_reports_a_restart_not_a_crash() {
        let (msg, code) = report(&rekeyed().into());
        assert_eq!(code, 75);
        assert!(
            !msg.starts_with("Error:"),
            "a restart must not read as a crash: {msg}"
        );
        assert!(
            msg.contains("was: old") && msg.contains("now: new"),
            "{msg}"
        );
        assert!(
            msg.contains("status 75") && msg.contains("restarted"),
            "{msg}"
        );
    }

    /// Every other error reports exactly as `main` returning `Result` did: the
    /// `Error:` debug dump and status 1. This is what keeps the change from
    /// altering any other command.
    #[test]
    fn every_other_error_reports_exactly_as_before() {
        let err = anyhow::anyhow!("room not found").context("while listing messages");
        let (msg, code) = report(&err);
        assert_eq!(code, 1);
        assert_eq!(msg, format!("Error: {err:?}"));
    }
}
