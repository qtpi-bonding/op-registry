use crate::{Audience, CallPath, Notices, OpParams, Redactor};

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
    /// A hint for reconnecting streams: how many consecutive failed
    /// connections they may retry. `None` leaves the stream's own default.
    pub max_retries: Option<u32>,
    /// The path the call took; the library sets it before the op runs.
    pub(crate) path: Option<CallPath>,
    /// The audience of the server making the call, set by `mcp::call_tool`.
    pub(crate) serving: Option<Audience>,
    /// The call's redactor. The pipeline extends the handle already here with
    /// the call's secrets and never replaces it, so an op that passes a
    /// clone of its options to a nested call shares one handle with it.
    pub(crate) redactor: Redactor,
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

    /// Returns these options with the reconnect limit hint set.
    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = Some(n);
        self
    }

    /// Registers a secret the op learned while running (a key file it read):
    /// the value, its trimmed form and each trimmed line are replaced by
    /// `<secret>` in every error line, notice, hook and outcome of the call,
    /// and in those of the calls that share these options (a clone shares
    /// the handle).
    ///
    /// Register a secret before you print anything that may contain it: what
    /// was already sent is not redacted after the fact.
    pub fn redact(&self, bytes: &[u8]) {
        self.redactor.add(bytes);
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

/// A failed call together with the redactor built from its parameters.
///
/// `Display` and `Debug` print the redacted message only. The raw error,
/// which may carry a secret, stays reachable through [`CallFailure::error`]
/// for callers that downcast it; they must not print it.
pub struct CallFailure {
    error: anyhow::Error,
    redactor: Redactor,
}

impl CallFailure {
    pub(crate) fn new(error: anyhow::Error, redactor: Redactor) -> Self {
        Self { error, redactor }
    }

    /// A failure before any parameters exist: nothing to redact.
    pub(crate) fn plain(error: anyhow::Error) -> Self {
        Self::new(error, Redactor::default())
    }

    /// The raw error. It may carry secret values: never print it.
    pub fn error(&self) -> &anyhow::Error {
        &self.error
    }

    /// The raw error, by value. It may carry secret values: never print it.
    pub fn into_inner(self) -> anyhow::Error {
        self.error
    }

    /// The redactor of the call that failed.
    pub fn redactor(&self) -> &Redactor {
        &self.redactor
    }

    /// The whole context chain of the error with the call's secrets redacted.
    pub fn redacted_message(&self) -> String {
        self.redactor.redact(&format!("{:#}", self.error))
    }
}

impl std::fmt::Display for CallFailure {
    /// The redacted message (`{:#}` gives the whole context chain).
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = if formatter.alternate() {
            format!("{:#}", self.error)
        } else {
            self.error.to_string()
        };
        formatter.write_str(&self.redactor.redact(&text))
    }
}

impl std::fmt::Debug for CallFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallFailure")
            .field("code", &coded_error::facts_anyhow(&self.error).code)
            .field("message", &self.redacted_message())
            .finish()
    }
}

/// The code, class and overrides of the inner error.
impl coded_error::CodedError for CallFailure {
    fn code(&self) -> &'static str {
        coded_error::facts_anyhow(&self.error).code
    }

    fn class(&self) -> coded_error::ErrorClass {
        coded_error::facts_anyhow(&self.error).class
    }

    fn overrides(&self) -> coded_error::Overrides {
        coded_error::facts_anyhow(&self.error).overrides
    }
}

impl CallFailure {
    /// An `anyhow::Error` that keeps the code and class and whose `Display`
    /// is redacted. A plain `?` into `anyhow::Error` loses the code (the
    /// source chain is empty on purpose): on a typed path, use
    /// `.map_err(CallFailure::into_coded)?`.
    pub fn into_coded(self) -> anyhow::Error {
        coded_error::into_anyhow(self)
    }
}

/// The source chain is left out: its messages are not redacted.
impl std::error::Error for CallFailure {}
