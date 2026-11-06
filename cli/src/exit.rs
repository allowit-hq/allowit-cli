//! Process exit codes. Every command
//! uses this one table.
use crate::error::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub(crate) enum Exit {
    Ok = 0,
    Internal = 1,
    Usage = 2,
    /// Configuration, authentication, or not supported on this rail.
    Config = 3,
    /// Rejected by the contract, for example a paused wallet or a wrong asset.
    Rejected = 4,
    /// Uncertain or stale: retry with the same request ID.
    Uncertain = 5,
    Replayed = 6,
    OwnerSignature = 10,
    OwnerInput = 11,
    Pending = 12,
    Denied = 20,
    Interrupted = 130,
}

pub(crate) const TABLE: &[(Exit, &str)] = &[
    (Exit::Ok, "ok"),
    (Exit::Internal, "internal error"),
    (Exit::Usage, "usage error or invalid input"),
    (
        Exit::Config,
        "configuration, authentication or not supported on this rail",
    ),
    (
        Exit::Rejected,
        "rejected by the contract (paused, wrong asset)",
    ),
    (
        Exit::Uncertain,
        "uncertain or stale: retry with the same ID",
    ),
    (Exit::Replayed, "replayed"),
    (Exit::OwnerSignature, "owner signature needed"),
    (Exit::OwnerInput, "waiting for owner input"),
    (Exit::Pending, "pending"),
    (Exit::Denied, "denied by the policy"),
    (Exit::Interrupted, "interrupted"),
];

impl Exit {
    pub(crate) fn code(self) -> i32 {
        self as i32
    }
    /// The symbolic name used in JSON errors.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Exit::Ok => "ok",
            Exit::Internal => "internal",
            Exit::Usage => "usage",
            Exit::Config => "config",
            Exit::Rejected => "rejected",
            Exit::Uncertain => "uncertain",
            Exit::Replayed => "replayed",
            Exit::OwnerSignature => "owner_signature",
            Exit::OwnerInput => "owner_input",
            Exit::Pending => "pending",
            Exit::Denied => "denied",
            Exit::Interrupted => "interrupted",
        }
    }
    pub(crate) fn from_code(code: i32) -> Exit {
        TABLE
            .iter()
            .map(|(e, _)| *e)
            .find(|e| e.code() == code)
            .unwrap_or(Exit::Internal)
    }
}

/// Convert shared execution state to the process contract owned by this CLI.
pub(crate) fn execution_status(status: allowit_sdk_engine::runtime::Status) -> Exit {
    use allowit_sdk_engine::runtime::Status;
    match status {
        Status::Confirmed => Exit::Ok,
        Status::Failed => Exit::Denied,
        Status::SubmissionUnknown => Exit::Uncertain,
        Status::Prepared | Status::Submitted => Exit::Pending,
    }
}

/// The error for a verb whose backend is not ready on this rail.
pub(crate) fn not_available(verb: &str, rail: &str) -> Error {
    Error::unsupported(format!("{verb} is not available on {rail} yet"))
}

/// The error for a command that needs a rail when none is selected.
pub(crate) fn no_rail() -> Error {
    Error::config("no rail selected; run allowit rail ls, then allowit rail use RAIL")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn execution_states_keep_the_existing_cli_codes() {
        use allowit_sdk_engine::runtime::Status;
        for (state, code) in [
            (Status::Confirmed, 0),
            (Status::Failed, 20),
            (Status::SubmissionUnknown, 5),
            (Status::Prepared, 12),
            (Status::Submitted, 12),
        ] {
            assert_eq!(execution_status(state).code(), code);
        }
    }
    #[test]
    fn table_codes_are_unique_and_round_trip() {
        for (i, (exit, _)) in TABLE.iter().enumerate() {
            assert_eq!(Exit::from_code(exit.code()), *exit);
            assert!(TABLE[..i].iter().all(|(e, _)| e.code() != exit.code()));
        }
        assert_eq!(Exit::from_code(99), Exit::Internal);
        assert_eq!(not_available("wallet close", "stellar:testnet").code, 3);
        assert_eq!(no_rail().code, 3);
        assert_eq!(Exit::Denied.name(), "denied");
        for (status, code) in [(401, 3), (403, 3), (404, 4), (500, 4)] {
            assert_eq!(Error::api(status, "x").code, code, "HTTP {status}");
        }
    }
}
