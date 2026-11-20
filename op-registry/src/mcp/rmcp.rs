//! rmcp adapter: serves a registry's tools for one audience.

use super::{call_tool, tool_error, tool_list, ToolSpec};
use crate::{Audience, CallOptions, Notice, NoticeSink, Notices, Registry, Resolve, TraceParent};
use ::rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult, LoggingLevel,
    LoggingMessageNotificationParam, Meta, PaginatedRequestParams, ProgressNotificationParam,
    ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use ::rmcp::service::RequestContext;
use ::rmcp::{ErrorData, RoleServer, ServerHandler};
use futures_util::future::{select, Either};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

/// Serves a registry over rmcp for one audience.
///
/// Tools are the registry's [`tool_list`] for that audience. Calls go through
/// [`call_tool`], so they take the one pipeline with `CallPath::Mcp`. The
/// caller stays `None`: who is calling is the application's concern.
pub struct RegistryServer<C> {
    registry: Arc<Registry<C>>,
    ctx: CtxSource<C>,
    audience: Audience,
    info: Implementation,
    instructions: Option<String>,
}

/// Where a call's context comes from: one shared context, or a factory run
/// once per `tools/call`.
enum CtxSource<C> {
    Shared(Arc<C>),
    Factory(Arc<dyn Fn() -> Arc<C> + Send + Sync>),
}

impl<C> CtxSource<C> {
    fn get(&self) -> Arc<C> {
        match self {
            CtxSource::Shared(ctx) => ctx.clone(),
            CtxSource::Factory(factory) => factory(),
        }
    }
}

impl<C: Resolve + Send + Sync + 'static> RegistryServer<C> {
    /// Creates a server for `audience`, reporting `info` as its identity.
    pub fn new(
        registry: Arc<Registry<C>>,
        ctx: Arc<C>,
        audience: Audience,
        info: Implementation,
    ) -> Self {
        Self {
            registry,
            ctx: CtxSource::Shared(ctx),
            audience,
            info,
            instructions: None,
        }
    }

    /// Creates a server that builds a fresh context for every `tools/call`
    /// by running `factory`.
    pub fn with_ctx_factory(
        registry: Arc<Registry<C>>,
        factory: Arc<dyn Fn() -> Arc<C> + Send + Sync>,
        audience: Audience,
        info: Implementation,
    ) -> Self {
        Self {
            registry,
            ctx: CtxSource::Factory(factory),
            audience,
            info,
            instructions: None,
        }
    }

    /// Returns this server with `text` as the instructions `get_info` sends.
    pub fn with_instructions(mut self, text: impl Into<String>) -> Self {
        self.instructions = Some(text.into());
        self
    }
}

fn object(schema: &Value) -> Arc<Map<String, Value>> {
    Arc::new(match schema {
        Value::Object(map) => map.clone(),
        _ => Map::from_iter([("type".to_owned(), json!("object"))]),
    })
}

fn to_tool(spec: ToolSpec) -> Tool {
    let mut tool = Tool::new(spec.name, spec.description, object(&spec.input_schema)).annotate(
        ToolAnnotations::new()
            .read_only(spec.annotations.read_only_hint)
            .destructive(spec.annotations.destructive_hint),
    );
    if let Some(Value::Object(schema)) = spec.output_schema {
        tool = tool.with_raw_output_schema(Arc::new(schema));
    }
    tool
}

fn trace_of(meta: &Meta) -> TraceParent {
    let get = |key: &str| meta.0.get(key).and_then(Value::as_str).map(str::to_owned);
    TraceParent {
        traceparent: get("traceparent"),
        tracestate: get("tracestate"),
        baggage: get("baggage"),
    }
}

struct ChannelSink(UnboundedSender<Notice>);

impl NoticeSink for ChannelSink {
    fn send(&self, notice: Notice) {
        let _ = self.0.send(notice);
    }
}

// Text notices use MCP logging, which rmcp 1.8 marks deprecated (SEP-2577) but
// is still the protocol's channel for them.
#[allow(deprecated)]
async fn forward(
    context: &RequestContext<RoleServer>,
    token: &Option<::rmcp::model::ProgressToken>,
    notice: Notice,
) {
    match notice {
        Notice::Text(text) => {
            let _ = context
                .peer
                .notify_logging_message(LoggingMessageNotificationParam::new(
                    LoggingLevel::Info,
                    Value::String(text),
                ))
                .await;
        }
        Notice::Progress { done, total, label } => {
            let Some(token) = token else { return };
            let mut param = ProgressNotificationParam::new(token.clone(), done as f64);
            if let Some(total) = total {
                param = param.with_total(total as f64);
            }
            if !label.is_empty() {
                param = param.with_message(label);
            }
            let _ = context.peer.notify_progress(param).await;
        }
    }
}

#[allow(deprecated)] // enable_logging, see `forward`
impl<C: Resolve + Send + Sync + 'static> ServerHandler for RegistryServer<C> {
    fn get_info(&self) -> ServerInfo {
        let info = ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_logging()
                .build(),
        )
        .with_server_info(self.info.clone());
        match &self.instructions {
            Some(text) => info.with_instructions(text.clone()),
            None => info,
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(
            tool_list(&self.registry, self.audience)
                .into_iter()
                .map(to_tool)
                .collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let meta = request.meta.clone().unwrap_or_else(|| context.meta.clone());
        let token = meta.get_progress_token();
        let (sender, mut receiver) = unbounded_channel();
        let opts = CallOptions::default()
            .with_trace(trace_of(&meta))
            .with_notices(Notices::new(Arc::new(ChannelSink(sender))));
        let args = Value::Object(request.arguments.unwrap_or_default());

        let ctx = self.ctx.get();
        let call = call_tool(
            &self.registry,
            &ctx,
            self.audience,
            &request.name,
            args,
            opts,
        );
        let mut call = std::pin::pin!(call);
        // Forward notices while the call runs, in the order they were sent.
        let result = loop {
            match select(call.as_mut(), std::pin::pin!(receiver.recv())).await {
                Either::Left((result, _)) => break result,
                Either::Right((Some(notice), _)) => forward(&context, &token, notice).await,
                Either::Right((None, _)) => break call.as_mut().await,
            }
        };
        while let Ok(notice) = receiver.try_recv() {
            forward(&context, &token, notice).await;
        }

        Ok(match result {
            // `structured` also adds the compact JSON as the text block.
            Ok(value @ Value::Object(_)) => CallToolResult::structured(value),
            // Structured content is for objects only.
            Ok(value) => CallToolResult::success(vec![Content::text(value.to_string())]),
            Err(error) => {
                let failure = tool_error(&error, "mcp", "internal error");
                CallToolResult::structured_error(json!({"error": {
                    "code": failure.code,
                    "class": failure.class,
                    "message": failure.message,
                    "fingerprint": failure.fingerprint,
                }}))
            }
        })
    }
}
