//! Plain-data MCP adapters for a registry: tool specs and tool calls.

use crate::{
    Audience, CallError, CallOptions, CallPath, OpKind, Paths, Redactor, Registry, Resolve,
};
use serde::Serialize;
use serde_json::Value;

/// The tool contract exposed for an operation.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    pub annotations: ToolAnnotations,
}

/// The MCP behaviour hints derived from an op's access.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    pub read_only_hint: bool,
    pub destructive_hint: bool,
}

/// Lists the tools of an audience, in registry order: every op of that
/// audience that is not hidden, not a stream and not typed-only. Both halves
/// of a list pair are tools.
pub fn tool_list<C: Sync>(registry: &Registry<C>, audience: Audience) -> Vec<ToolSpec> {
    registry
        .descriptors()
        .into_iter()
        .filter(|d| {
            d.audience == audience
                && !d.hidden
                && d.kind != OpKind::Stream
                && d.paths != Paths::TypedOnly
        })
        .map(|d| ToolSpec {
            name: d.name.clone(),
            description: if d.long_help.is_empty() {
                d.summary.to_owned()
            } else {
                format!("{}\n\n{}", d.summary, d.long_help)
            },
            input_schema: d.input_schema.clone(),
            output_schema: d.output_schema.clone(),
            annotations: ToolAnnotations {
                read_only_hint: d.access.reads && !d.access.writes && !d.access.destructive,
                destructive_hint: d.access.destructive,
            },
        })
        .collect()
}

/// Error returned by the tool dispatch layer, with the redactor of the call
/// that failed.
#[derive(Debug)]
pub struct ToolError {
    error: anyhow::Error,
    redactor: Redactor,
}

impl ToolError {
    fn plain(error: impl coded_error::CodedError + Send + Sync + 'static) -> Self {
        Self {
            error: coded_error::into_anyhow(error),
            redactor: Redactor::default(),
        }
    }

    /// The underlying error.
    pub fn into_inner(self) -> anyhow::Error {
        self.error
    }

    /// The underlying error, by reference.
    pub fn error(&self) -> &anyhow::Error {
        &self.error
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.error, formatter)
    }
}

impl std::error::Error for ToolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

/// Calls an op as a tool: the one pipeline with `CallPath::Mcp`. A hidden op
/// is callable.
pub async fn call_tool<C: Resolve>(
    registry: &Registry<C>,
    ctx: &C,
    audience: Audience,
    name: &str,
    args: Value,
    opts: CallOptions,
) -> Result<Value, ToolError> {
    let descriptor = registry.get(name).ok_or_else(|| {
        ToolError::plain(CallError::UnknownOperation {
            name: name.to_owned(),
        })
    })?;
    if descriptor.audience != audience {
        return Err(ToolError::plain(CallError::OutsideAudience {
            name: name.to_owned(),
        }));
    }
    let invocation = registry
        .call_json(name, args, ctx, opts, CallPath::Mcp, false)
        .await;
    invocation.result.map_err(|error| ToolError {
        error,
        redactor: invocation.redactor,
    })
}

/// A redacted, transport-neutral tool failure.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolFailure {
    pub code: String,
    pub class: coded_error::ErrorClass,
    pub message: String,
    pub fingerprint: String,
}

/// Converts a tool error to its caller-safe representation: the message is
/// redacted by class and then by the call's own secrets.
///
/// `fallback_message` is the message used when the error carries no coded message.
pub fn tool_error(err: &ToolError, source: &str, fallback_message: &str) -> ToolFailure {
    let (_, info, message) =
        coded_error::http::to_http_anyhow_redacted(&err.error, source, fallback_message);
    ToolFailure {
        code: info.code,
        class: info.class,
        message: err.redactor.redact(&message),
        fingerprint: info.fingerprint,
    }
}
