//! Plain-data MCP-shaped adapters for a registry.

use crate::{Audience, CallError, CallOptions, OpKind, Registry};

/// The tool contract exposed for an operation.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
}

/// Lists unary operations available to an audience, in registry order.
pub fn tool_list<C: Sync>(registry: &Registry<C>, audience: Audience) -> Vec<ToolSpec> {
    registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.kind != OpKind::Stream && descriptor.audience == audience)
        .map(|descriptor| ToolSpec {
            name: descriptor.name.clone(),
            description: descriptor.summary.clone(),
            input_schema: descriptor.input_schema.clone(),
            output_schema: descriptor.output_schema.clone().unwrap_or_default(),
        })
        .collect()
}

/// Error returned by the tool dispatch layer.
#[derive(Debug)]
pub struct ToolError(pub anyhow::Error);

impl std::fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for ToolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl ToolError {
    pub fn into_inner(self) -> anyhow::Error {
        self.0
    }
}

/// Calls a unary operation exposed as a tool.
pub async fn call_tool<C: Sync>(
    registry: &Registry<C>,
    ctx: &C,
    audience: Audience,
    name: &str,
    args: serde_json::Value,
    opts: CallOptions,
) -> Result<serde_json::Value, ToolError> {
    let descriptor = registry.get(name).ok_or_else(|| {
        ToolError(coded_error::into_anyhow(CallError::UnknownOperation {
            name: name.to_owned(),
        }))
    })?;

    if descriptor.kind == OpKind::Stream {
        return Err(ToolError(coded_error::into_anyhow(CallError::WrongShape {
            name: name.to_owned(),
        })));
    }
    if descriptor.audience != audience {
        return Err(ToolError(coded_error::into_anyhow(
            CallError::OutsideAudience {
                name: name.to_owned(),
            },
        )));
    }

    registry
        .call(name, args, ctx, opts)
        .await
        .map_err(ToolError)
}

/// A redacted, transport-neutral tool failure.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolFailure {
    pub code: String,
    pub class: coded_error::ErrorClass,
    pub message: String,
    pub fingerprint: String,
}

/// Converts a tool error to its caller-safe representation.
///
/// `fallback_message` is the message used when the error carries no coded message.
pub fn tool_error(err: &ToolError, source: &str, fallback_message: &str) -> ToolFailure {
    let (_, info, message) =
        coded_error::http::to_http_anyhow_redacted(&err.0, source, fallback_message);
    ToolFailure {
        code: info.code,
        class: info.class,
        message,
        fingerprint: info.fingerprint,
    }
}
