use crate::{ParseCx, Paths, Registry};
use anyhow::Result;
use futures_util::stream::BoxStream;
use serde_json::Value;

#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct CallOptions {
    pub caller: Option<Caller>,
}

/// Identifies the caller of an operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caller(pub String);

impl CallOptions {
    /// Returns these options with the caller set.
    pub fn with_caller(mut self, caller: Caller) -> Self {
        self.caller = Some(caller);
        self
    }
}

/// A resumption point for a streaming operation.
///
/// Serializes as the plain underlying string.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Cursor(String);

impl Cursor {
    /// Creates a cursor from its textual form.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Returns the cursor's textual form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Cursor {
    /// Writes the cursor's textual form.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Errors raised by the registry dispatch layer.
#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "op_registry.call")]
#[non_exhaustive]
pub enum CallError {
    #[error("unknown operation {name}")]
    #[coded(code = "unknown_operation", class = Logical)]
    UnknownOperation { name: String },
    #[error("operation {name} has the wrong call shape")]
    #[coded(code = "wrong_shape", class = Logical)]
    WrongShape { name: String },
    #[error("operation {name} is not available to this audience")]
    #[coded(code = "outside_audience", class = Logical)]
    OutsideAudience { name: String },
    #[error("operation {name} cannot be called on this path")]
    #[coded(code = "wrong_path", class = Logical)]
    WrongPath { name: String },
}

impl<C: Sync> Registry<C> {
    /// Calls a unary operation by name with JSON arguments.
    ///
    /// A minimal v2 path (decode, run, serialise); Task 6 replaces it with the
    /// full pipeline.
    pub async fn call(&self, name: &str, args: Value, ctx: &C, opts: CallOptions) -> Result<Value> {
        let entry = self.entry(name)?;
        if entry.descriptor.paths == Paths::TypedOnly {
            return Err(coded_error::into_anyhow(CallError::WrongPath {
                name: name.to_owned(),
            }));
        }
        let params = (entry.decode_json)(args, &mut ParseCx::default())?;
        self.run_unary(name, params, ctx, opts).await
    }

    /// Calls a streaming operation by name with JSON arguments.
    pub async fn call_stream<'a>(
        &self,
        name: &str,
        args: Value,
        ctx: &'a C,
        opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> Result<BoxStream<'a, Result<Value>>> {
        let entry = self.entry(name)?;
        if entry.descriptor.paths == Paths::TypedOnly {
            return Err(coded_error::into_anyhow(CallError::WrongPath {
                name: name.to_owned(),
            }));
        }
        let params = (entry.decode_json)(args, &mut ParseCx::default())?;
        self.run_stream(name, params, ctx, opts, cursor).await
    }
}
