use serde_json::Value;

/// Per-parse state shared by the parameter decoders (filled in by later tasks).
#[derive(Debug, Default)]
pub struct ParseCx {}

pub trait OpParams: Sized {
    fn schema() -> Value;
    fn from_json(value: Value, cx: &mut ParseCx) -> anyhow::Result<Self>;
    fn clap_args(cmd: clap::Command) -> clap::Command;
    fn from_matches(matches: &clap::ArgMatches, cx: &mut ParseCx) -> anyhow::Result<Self>;
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
}

#[doc(hidden)]
pub fn usage(error: UsageError) -> anyhow::Error {
    coded_error::into_anyhow(error)
}
