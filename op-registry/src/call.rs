use crate::Registry;
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
}

impl<C: Sync> Registry<C> {
    /// Calls a unary operation by name.
    pub async fn call(&self, name: &str, args: Value, ctx: &C, opts: CallOptions) -> Result<Value> {
        let entry = self.entry(name)?;
        match &entry.call {
            crate::registry::EntryCall::Unary(call) => call(ctx, args, opts).await,
            crate::registry::EntryCall::Stream(_) => {
                Err(coded_error::into_anyhow(CallError::WrongShape {
                    name: name.to_owned(),
                }))
            }
        }
    }

    /// Calls a streaming operation by name.
    pub async fn call_stream<'a>(
        &self,
        name: &str,
        args: Value,
        ctx: &'a C,
        opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> Result<BoxStream<'a, Result<Value>>> {
        let entry = self.entry(name)?;
        match &entry.call {
            crate::registry::EntryCall::Stream(call) => call(ctx, args, opts, cursor).await,
            crate::registry::EntryCall::Unary(_) => {
                Err(coded_error::into_anyhow(CallError::WrongShape {
                    name: name.to_owned(),
                }))
            }
        }
    }
}

/// Dispatches a unary operation.
pub async fn call<C: Sync>(
    registry: &Registry<C>,
    name: &str,
    args: Value,
    ctx: &C,
    opts: CallOptions,
) -> Result<Value> {
    registry.call(name, args, ctx, opts).await
}

/// Dispatches a streaming operation.
pub async fn call_stream<'a, C: Sync>(
    registry: &'a Registry<C>,
    name: &str,
    args: Value,
    ctx: &'a C,
    opts: CallOptions,
    cursor: Option<Cursor>,
) -> Result<BoxStream<'a, Result<Value>>> {
    registry.call_stream(name, args, ctx, opts, cursor).await
}
