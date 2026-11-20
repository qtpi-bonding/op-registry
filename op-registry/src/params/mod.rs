//! The `OpParams` trait, field metadata and usage errors.

#[doc(hidden)]
pub mod base64;
#[doc(hidden)]
pub mod rt;
mod types;

pub use types::{Input, Json, Patch};

use std::io::Read;

use serde_json::Value;

/// Per-parse state shared by the parameter decoders.
#[derive(Default)]
pub struct ParseCx {
    /// Where `-` and `@-` read from; `None` on paths without stdin.
    pub(crate) stdin: Option<Box<dyn Read + Send>>,
    /// The first field that read stdin.
    pub(crate) stdin_used_by: Option<String>,
    /// Secret values collected while decoding, for redaction.
    pub secrets: Vec<Vec<u8>>,
}

impl std::fmt::Debug for ParseCx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParseCx")
            .field("stdin", &self.stdin.is_some())
            .field("stdin_used_by", &self.stdin_used_by)
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

impl ParseCx {
    /// A context whose stdin is `reader`.
    pub fn with_stdin(reader: impl Read + Send + 'static) -> Self {
        Self {
            stdin: Some(Box::new(reader)),
            ..Self::default()
        }
    }

    /// Reads all of stdin for `field`. Only one field may do so per parse.
    pub fn read_stdin(&mut self, field: &str) -> anyhow::Result<Vec<u8>> {
        if let Some(first) = &self.stdin_used_by {
            return Err(usage(UsageError::StdinTwice {
                first: first.clone(),
                second: field.to_owned(),
            }));
        }
        let Some(reader) = self.stdin.as_mut() else {
            return Err(usage(UsageError::InvalidField {
                name: field.to_owned(),
                reason: "stdin is not available on this path".to_owned(),
            }));
        };
        self.stdin_used_by = Some(field.to_owned());
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(|e| {
            usage(UsageError::InvalidField {
                name: field.to_owned(),
                reason: format!("cannot read stdin: {e}"),
            })
        })?;
        Ok(bytes)
    }
}

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
    /// The refs of this struct: non-subject refs first, then the subject.
    fn ref_slots(&mut self) -> Vec<crate::RefSlot<'_>> {
        Vec::new()
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
