//! The command line: the tree built from a registry, and parsing into a
//! selected op. Dispatch and the runner come in the next layer.

mod parse;
mod tree;

pub use parse::{parse, Parsed, UsageFailure};
pub use tree::command;

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
