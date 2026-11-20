//! Process-wide settings of a binary and the context the CLI builds from them.

use crate::{Caller, FieldMeta};

/// The global flags of one binary: process settings such as endpoints and
/// directories, accepted before or after the verb. Derive it with
/// `#[derive(OpGlobals)]`; the field rules are those of `OpParams`, plus
/// `#[op(env = "...")]` for an environment fallback.
pub trait OpGlobals: Sized + Send + Sync + 'static {
    /// What the derive knows about each field.
    fn fields() -> Vec<FieldMeta>;
    /// Adds the arguments to a command; every one is `global(true)`.
    fn add_args(cmd: clap::Command) -> clap::Command;
    /// Reads the globals from parsed matches.
    fn from_matches(m: &clap::ArgMatches) -> anyhow::Result<Self>;
}

/// A context the command line can build from its globals.
pub trait CliContext: Sized + Send + Sync {
    /// The binary's global flags.
    type Globals: OpGlobals;

    /// Builds the context. Cheap: it makes no connection.
    fn from_globals(globals: &Self::Globals) -> anyhow::Result<Self>;

    /// Who the CLI reports as the caller of every op.
    fn caller(&self) -> Option<Caller> {
        None
    }
}
