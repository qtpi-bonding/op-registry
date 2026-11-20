//! The exit table: which process exit code an error or an ok output maps to.

use crate::Exit;

/// Success.
pub const OK: u8 = 0;
/// The command line or arguments were wrong.
pub const USAGE: u8 = 2;
/// The request is wrong or conflicts with state (sysexits DATAERR).
pub const LOGICAL: u8 = 65;
/// The infrastructure is broken (sysexits UNAVAILABLE).
pub const INFRASTRUCTURE: u8 = 69;
/// Not classified (sysexits SOFTWARE).
pub const UNKNOWN: u8 = 70;
/// Retrying may succeed (sysexits TEMPFAIL).
pub const TRANSIENT: u8 = 75;

/// The exit code of a failure: 2 for a usage or command-line code, otherwise
/// the sysexits code of its class (an uncoded error counts as unknown).
pub fn exit_code(err: &anyhow::Error) -> u8 {
    let facts = coded_error::facts_anyhow(err);
    if facts.code.starts_with("op_registry.usage.") || facts.code.starts_with("op_registry.cli.") {
        return USAGE;
    }
    coded_error::cli::ExitTable::SYSEXITS.code_for(facts.class)
}

/// The exit code of a successful output: 0 stays 0, a declared code in
/// 3..=63 is returned as is, and anything else is 70.
pub fn ok_exit(declared: &[Exit], code: u8) -> u8 {
    if code == OK {
        OK
    } else if (3..=63).contains(&code) && declared.iter().any(|e| e.code == code) {
        code
    } else {
        UNKNOWN
    }
}
