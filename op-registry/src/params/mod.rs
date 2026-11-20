//! The `OpParams` trait, field metadata and usage errors.

#[doc(hidden)]
pub mod rt;

use serde_json::Value;

/// Per-parse state shared by the parameter decoders (filled in by later tasks).
#[derive(Debug, Default)]
pub struct ParseCx {}

/// How a field is presented on the command line and in the schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldKind {
    /// One value (`String`, numbers, `PathBuf`, enums), possibly optional.
    Scalar,
    /// A `bool` switch.
    Switch,
    /// An `Option<bool>` with `--x` and `--no-x`.
    TriState,
    /// A `Vec<T>`: a repeated flag.
    Repeated,
    /// A `BTreeMap<String, String>`: a repeated `k=v` flag.
    Map,
    /// Arbitrary JSON.
    Json,
    /// Bytes read from a path or stdin.
    Input,
    /// A reference to another object.
    Ref,
    /// A secret read from a file.
    SecretFile,
}

/// What the derive knows about one field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldMeta {
    /// The Rust field name.
    pub rust_name: &'static str,
    /// The long flag without its dashes (`agent-id`).
    pub flag: String,
    /// The JSON key (`agentId`).
    pub key: String,
    /// The field's doc comment, lines joined by spaces.
    pub help: String,
    pub kind: FieldKind,
    /// Must be given: no default, not optional.
    pub required: bool,
    pub secret: bool,
    /// The shared param group the field came from, if any.
    pub from_group: Option<&'static str>,
}

/// One parameter struct decoded from clap matches or JSON.
pub trait OpParams: Sized + Send + 'static {
    fn fields() -> Vec<FieldMeta>;
    fn input_schema() -> Value;
    fn add_args(cmd: clap::Command) -> clap::Command;
    fn from_matches(m: &clap::ArgMatches, cx: &mut ParseCx) -> anyhow::Result<Self>;
    fn from_json(v: Value, cx: &mut ParseCx) -> anyhow::Result<Self>;
    fn validate(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn secrets(&self) -> Vec<Vec<u8>> {
        Vec::new()
    }
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "op_registry.usage")]
#[non_exhaustive]
pub enum UsageError {
    #[error("missing field {name}")]
    #[coded(code = "missing", class = Logical)]
    MissingField { name: String },
    #[error("invalid field {name}: {reason}")]
    #[coded(code = "invalid", class = Logical)]
    InvalidField { name: String, reason: String },
    #[error("unknown key {key} for {param}")]
    #[coded(code = "unknown_key", class = Logical)]
    UnknownKey { param: String, key: String },
    #[error("repeated key {key} for {param}")]
    #[coded(code = "repeated_key", class = Logical)]
    RepeatedKey { param: String, key: String },
    #[error("exactly one of the {group} fields is required")]
    #[coded(code = "one_of", class = Logical)]
    OneOf { group: String },
    #[error("{field} is required when {other} is given")]
    #[coded(code = "requires_if", class = Logical)]
    RequiresIf { field: String, other: String },
    #[error("{first} and {second} both read stdin")]
    #[coded(code = "stdin_twice", class = Logical)]
    StdinTwice { first: String, second: String },
}

#[doc(hidden)]
pub fn usage(error: UsageError) -> anyhow::Error {
    coded_error::into_anyhow(error)
}
