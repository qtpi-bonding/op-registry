//! Checks the library gives every consumer, behind the `testing` feature.
//!
//! Each `assert_*` panics with a report of every problem it found, never only
//! the first.

mod casing;
mod help;
mod parity;
mod sample;

pub use casing::{assert_casing, Casing};
pub use help::{assert_help_complete, HelpRules};
pub use parity::{assert_canonical, assert_parity, parity_report, Mount, ParityLine, Reach};
pub use sample::sample_params;

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
