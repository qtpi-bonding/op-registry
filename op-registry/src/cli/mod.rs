//! The command line: the tree built from a registry, and parsing into a
//! selected op, and the runner that dispatches it.

mod extra;
mod fallback;
mod io;
mod parse;
mod run;
mod tree;

pub use fallback::{edit_distance, FallbackFlag};
#[cfg(feature = "secrets")]
pub use io::Prompter;
pub use io::{Captured, Io, SharedWriter};
pub use parse::{parse, Parsed, UsageFailure};
pub use run::{dispatch, run, run_with_io, select, NoHooks, Outcome, RunHooks, Selected};
pub use tree::command;

pub(crate) use extra::Extra;
pub(crate) use fallback::Fallback;
#[cfg(feature = "manual")]
pub(crate) use tree::{lookup, on_cli};

/// The id of the library's `--pretty` argument.
pub(crate) const ID_PRETTY: &str = "op_registry_pretty";
/// The id of the library's `--compact` argument.
pub(crate) const ID_COMPACT: &str = "op_registry_compact";
/// The id of the library's `--text` argument.
pub(crate) const ID_TEXT: &str = "op_registry_text";

/// Errors produced when a command line does not identify an operation.
#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "op_registry.cli")]
#[non_exhaustive]
pub enum CliError {
    #[error("unknown or missing command {name}")]
    #[coded(code = "unknown_command", class = Logical)]
    UnknownCommand { name: String },
}
