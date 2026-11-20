use crate::{Notices, OpParams};

/// Per-call options.
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct CallOptions {
    /// Who is calling.
    pub caller: Option<Caller>,
    /// The trace context to pass to the hook.
    pub trace: TraceParent,
    /// Where the op's notices go.
    pub notices: Notices,
    /// The name of the transport, for the `json` path.
    pub transport: Option<String>,
}

/// Identifies the caller of an operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caller(pub String);

/// W3C trace context headers, carried untouched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceParent {
    /// The `traceparent` header.
    pub traceparent: Option<String>,
    /// The `tracestate` header.
    pub tracestate: Option<String>,
    /// The `baggage` header.
    pub baggage: Option<String>,
}

impl CallOptions {
    /// Returns these options with the caller set.
    pub fn with_caller(mut self, caller: Caller) -> Self {
        self.caller = Some(caller);
        self
    }

    /// Returns these options with the trace context set.
    pub fn with_trace(mut self, trace: TraceParent) -> Self {
        self.trace = trace;
        self
    }

    /// Returns these options with the notice handle set.
    pub fn with_notices(mut self, notices: Notices) -> Self {
        self.notices = notices;
        self
    }

    /// Returns these options with the transport name set.
    pub fn with_transport(mut self, transport: impl Into<String>) -> Self {
        self.transport = Some(transport.into());
        self
    }
}

/// A typed handle on one registered operation, for `call_typed`.
pub trait Op {
    /// The registry context.
    type Ctx;
    /// The parameter type.
    type Params: OpParams;
    /// The output (the item type for streams).
    type Output: Send + 'static;
    /// The operation name.
    const NAME: &'static str;
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
