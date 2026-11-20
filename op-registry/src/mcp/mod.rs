//! Plain-data MCP adapters for a registry: tool specs and tool calls.

use crate::{
    Audience, CallError, CallFailure, CallOptions, CallPath, OpKind, Paths, Registry, Resolve,
};
use serde::Serialize;
use serde_json::Value;

#[cfg(feature = "rmcp")]
pub mod rmcp;

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
/// audience that is not hidden, not a stream and not typed-only or CLI-only. Both halves
/// of a list pair are tools.
pub fn tool_list<C: Sync>(registry: &Registry<C>, audience: Audience) -> Vec<ToolSpec> {
    registry
        .descriptors()
        .into_iter()
        .filter(|d| {
            d.audience == audience && !d.hidden && d.kind != OpKind::Stream && d.paths == Paths::All
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

/// Calls an op as a tool: the one pipeline with `CallPath::Mcp`. A hidden op
/// is callable.
pub async fn call_tool<C: Resolve>(
    registry: &Registry<C>,
    ctx: &C,
    audience: Audience,
    name: &str,
    args: Value,
    opts: CallOptions,
) -> Result<Value, CallFailure> {
    let plain = |error: CallError| CallFailure::plain(coded_error::into_anyhow(error));
    let descriptor = registry.get(name).ok_or_else(|| {
        plain(CallError::UnknownOperation {
            name: name.to_owned(),
        })
    })?;
    if descriptor.audience != audience {
        return Err(plain(CallError::OutsideAudience {
            name: name.to_owned(),
        }));
    }
    let invocation = registry
        .call_json(name, args, ctx, opts, CallPath::Mcp, false)
        .await;
    invocation
        .result
        .map_err(|error| CallFailure::new(error, invocation.redactor))
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
pub fn tool_error(err: &CallFailure, source: &str, fallback_message: &str) -> ToolFailure {
    let (_, info, message) =
        coded_error::http::to_http_anyhow_redacted(err.error(), source, fallback_message);
    ToolFailure {
        code: info.code,
        class: info.class,
        message: err.redactor().redact(&message),
        fingerprint: info.fingerprint,
    }
}
