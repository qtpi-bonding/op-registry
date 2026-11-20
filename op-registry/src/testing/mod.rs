//! Checks the library gives every consumer, behind the `testing` feature.
//!
//! Each `assert_*` panics with a report of every problem it found, never only
//! the first.

mod casing;
mod codes;
mod help;
mod offline;
mod parity;
mod refs;
mod sample;
mod secrets;
mod snapshot;

pub use casing::{assert_casing, Casing};
pub(crate) use codes::check_strict;
pub use codes::{assert_error_codes, assert_exit_table, check_exit_table, strict_codes, Fixture};
pub use help::{assert_help_complete, HelpRules};
pub use offline::assert_local_ops_offline;
pub use parity::{assert_canonical, assert_parity, parity_report, Mount, ParityLine, Reach};
pub use refs::assert_unknown_ref_not_found;
pub use sample::sample_params;
pub use secrets::{assert_secrets_not_echoed, SecretSentinel};
pub use snapshot::{assert_manual_snapshot, check_manual_snapshot};

/// The longest a kit call of a stream op may run without an item or an end.
pub(crate) const KIT_STREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The options every kit call carries: reconnecting streams give up after
/// their first failed connection, and a caller, so ops that need one (listen
/// ops) reach their body.
pub(crate) fn kit_options() -> crate::CallOptions {
    crate::CallOptions::default()
        .with_max_retries(0)
        .with_caller(crate::Caller(KIT_CALLER.to_owned()))
}

/// The caller label of every kit call, and the JSON transport name.
const KIT_CALLER: &str = "op-registry-kit";

/// The path the kit calls `d` on: the first of json, mcp and cli that `d`
/// declares. `None` when `d` is declared on typed only, which the kit cannot
/// call.
pub(crate) fn kit_path(d: &crate::Descriptor) -> Option<crate::CallPath> {
    use crate::{CallPath, Surface};
    if d.paths.contains(Surface::Json) {
        Some(CallPath::Transport(KIT_CALLER.to_owned()))
    } else if d.paths.contains(Surface::Mcp) {
        Some(CallPath::Mcp)
    } else if d.paths.contains(Surface::Cli) {
        Some(CallPath::Cli)
    } else {
        None
    }
}

/// The line a runner prints, and adds to its report, for an op it cannot call.
pub(crate) fn typed_only_line(title: &str, d: &crate::Descriptor) -> String {
    format!("{title}: {}: typed only: not run", d.name)
}

/// Like [`fail_on`], and prints each of `skipped` to stderr; a report carries
/// them after its problems.
pub(crate) fn fail_on_noting(title: &str, problems: &[String], skipped: &[String]) {
    for line in skipped {
        eprintln!("{line}");
    }
    if problems.is_empty() {
        return;
    }
    let mut all = problems.to_vec();
    all.extend(skipped.iter().cloned());
    fail_on(title, &all);
}

/// Panics with `title` and one indented line per problem; does nothing when
/// there are none.
pub(crate) fn fail_on(title: &str, problems: &[String]) {
    if problems.is_empty() {
        return;
    }
    let mut report = format!("{title}: {} problem(s)\n", problems.len());
    for problem in problems {
        report.push_str("  - ");
        report.push_str(&problem.replace('\n', "\n    "));
        report.push('\n');
    }
    panic!("{report}");
}
