#![cfg(feature = "rmcp")]
//! The rmcp adapter, driven by a real rmcp client over an in-memory duplex.

use futures_util::future::BoxFuture;
use op_registry::mcp::rmcp::RegistryServer;
use op_registry::{Access, Audience, CallHook, CallInfo, CallOptions, OpParams, OpSpec, Registry};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientInfo, Implementation,
    LoggingMessageNotificationParam, Meta, ProgressNotificationParam, ProgressToken,
};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct Ctx;
impl op_registry::Resolve for Ctx {}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct Echo {
    /// Who to greet.
    host_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
struct Out {
    greeted_host: String,
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.app")]
enum AppError {
    #[error("the app failed")]
    #[coded(code = "failed", class = Logical)]
    Failed,
}

fn echo(_c: &Ctx, p: Echo, _o: CallOptions) -> BoxFuture<'_, anyhow::Result<Out>> {
    Box::pin(async move {
        Ok(Out {
            greeted_host: p.host_name.unwrap_or_default(),
        })
    })
}

fn boom(_c: &Ctx, _p: Echo, _o: CallOptions) -> BoxFuture<'_, anyhow::Result<Out>> {
    Box::pin(async { Err(coded_error::into_anyhow(AppError::Failed)) })
}

fn chatty(_c: &Ctx, _p: Echo, o: CallOptions) -> BoxFuture<'_, anyhow::Result<Out>> {
    Box::pin(async move {
        o.notices.notice("hello");
        o.notices.progress(1, Some(4), "step");
        o.notices.progress(2, None, "step");
        Ok(Out {
            greeted_host: String::new(),
        })
    })
}

#[derive(Default)]
struct Recorder(Mutex<Vec<op_registry::TraceParent>>);
impl CallHook for Recorder {
    fn around<'a>(&'a self, info: &CallInfo, call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()> {
        self.0.lock().unwrap().push(info.trace.clone());
        call
    }
}

fn registry(rec: Arc<Recorder>) -> Registry<Ctx> {
    let mut r: Registry<Ctx> = Registry::new();
    let mut read = OpSpec::<Echo, Out>::new("read_it", "Reads it.");
    read.access = Access::reads();
    r.push_op(read, echo);
    let mut wipe = OpSpec::<Echo, Out>::new("wipe_it", "Wipes it.");
    wipe.access = Access::destructive();
    r.push_op(wipe, echo);
    let mut owner = OpSpec::<Echo, Out>::new("owner_op", "Owner.");
    owner.audience = Audience::Owner;
    r.push_op(owner, echo);
    r.push_op(OpSpec::<Echo, Out>::new("boom", "Fails."), boom);
    r.push_op(OpSpec::<Echo, Out>::new("chatty", "Talks."), chatty);
    r.set_call_hook(rec);
    r
}

#[derive(Clone, Default)]
struct Client {
    progress: Arc<Mutex<Vec<ProgressNotificationParam>>>,
    logs: Arc<Mutex<Vec<LoggingMessageNotificationParam>>>,
}

impl ClientHandler for Client {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::default()
    }
    fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _c: NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        self.progress.lock().unwrap().push(params);
        std::future::ready(())
    }
    fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _c: NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        self.logs.lock().unwrap().push(params);
        std::future::ready(())
    }
}

async fn connect(rec: Arc<Recorder>) -> (RunningService<RoleClient, Client>, Client) {
    let (a, b) = tokio::io::duplex(64 * 1024);
    let server = RegistryServer::new(
        Arc::new(registry(rec)),
        Arc::new(Ctx),
        Audience::Agent,
        Implementation::new("test-server", "0.0.1"),
    );
    tokio::spawn(async move {
        let running = server.serve(a).await.unwrap();
        let _ = running.waiting().await;
    });
    let client = Client::default();
    let running = client.clone().serve(b).await.unwrap();
    (running, client)
}

fn args(v: Value) -> serde_json::Map<String, Value> {
    v.as_object().unwrap().clone()
}

async fn call(
    c: &RunningService<RoleClient, Client>,
    name: &'static str,
    a: Value,
    meta: Option<Meta>,
) -> CallToolResult {
    let mut p = CallToolRequestParams::new(name).with_arguments(args(a));
    p.meta = meta;
    c.call_tool(p).await.unwrap()
}

#[tokio::test]
async fn lists_audience_tools_with_annotations() {
    let (c, _) = connect(Arc::default()).await;
    let tools = c.list_all_tools().await.unwrap();
    let names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    assert_eq!(names, vec!["read_it", "wipe_it", "boom", "chatty"]);
    let read = &tools[0];
    assert_eq!(read.description.as_deref(), Some("Reads it."));
    let ann = read.annotations.as_ref().unwrap();
    assert_eq!(ann.read_only_hint, Some(true));
    assert_eq!(ann.destructive_hint, Some(false));
    assert_eq!(
        tools[1].annotations.as_ref().unwrap().destructive_hint,
        Some(true)
    );
    assert!(read.input_schema.contains_key("properties"));
    assert!(read.input_schema["properties"]
        .as_object()
        .unwrap()
        .contains_key("hostName"));
    #[cfg(feature = "schema")]
    assert!(read.output_schema.is_some());
    let info = c.peer_info().unwrap();
    assert_eq!(info.server_info.name, "test-server");
}

#[tokio::test]
async fn call_returns_structured_content() {
    let (c, _) = connect(Arc::default()).await;
    let r = call(&c, "read_it", json!({"hostName": "mars"}), None).await;
    assert_ne!(r.is_error, Some(true));
    assert_eq!(r.structured_content, Some(json!({"greetedHost": "mars"})));
    let text = r.content[0].as_text().unwrap().text.clone();
    assert_eq!(text, r#"{"greetedHost":"mars"}"#);
}

#[tokio::test]
async fn error_is_tool_error_with_code() {
    let (c, _) = connect(Arc::default()).await;
    let r = call(&c, "boom", json!({}), None).await;
    assert_eq!(r.is_error, Some(true));
    let err = &r.structured_content.unwrap()["error"];
    assert_eq!(err["code"], "test.app.failed");
    assert_eq!(err["class"], "logical");
    assert_eq!(err["message"], "the app failed");
    assert!(err["fingerprint"].is_string());

    let r = call(&c, "owner_op", json!({}), None).await;
    assert_eq!(r.is_error, Some(true));
    assert_eq!(
        r.structured_content.unwrap()["error"]["code"],
        "op_registry.call.outside_audience"
    );
}

#[tokio::test]
async fn meta_traceparent_reaches_call_info() {
    let rec = Arc::new(Recorder::default());
    let (c, _) = connect(rec.clone()).await;
    let mut meta = Meta::new();
    meta.0.insert("traceparent".into(), json!("00-abc-def-01"));
    meta.0.insert("tracestate".into(), json!("k=v"));
    meta.0.insert("baggage".into(), json!("a=b"));
    call(&c, "read_it", json!({}), Some(meta)).await;
    call(&c, "read_it", json!({}), None).await;
    let seen = rec.0.lock().unwrap();
    assert_eq!(seen[0].traceparent.as_deref(), Some("00-abc-def-01"));
    assert_eq!(seen[0].tracestate.as_deref(), Some("k=v"));
    assert_eq!(seen[0].baggage.as_deref(), Some("a=b"));
    assert_eq!(seen[1], op_registry::TraceParent::default());
}

#[tokio::test]
async fn progress_notice_reaches_client() {
    let (c, client) = connect(Arc::default()).await;
    let meta = Meta::with_progress_token(ProgressToken(rmcp::model::NumberOrString::Number(7)));
    let r = call(&c, "chatty", json!({}), Some(meta)).await;
    assert_ne!(r.is_error, Some(true));
    // notifications are delivered in order, before or just after the result
    for _ in 0..50 {
        if client.progress.lock().unwrap().len() >= 2 && !client.logs.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let progress = client.progress.lock().unwrap();
    assert_eq!(progress.len(), 2);
    assert_eq!(progress[0].progress, 1.0);
    assert_eq!(progress[0].total, Some(4.0));
    assert_eq!(progress[0].message.as_deref(), Some("step"));
    assert_eq!(progress[1].total, None);
    let logs = client.logs.lock().unwrap();
    assert_eq!(logs[0].data, json!("hello"));
}

// DEC-6: a context per call, and server instructions.

struct Numbered(usize);
impl op_registry::Resolve for Numbered {}

fn whoami(c: &Numbered, _p: Echo, _o: CallOptions) -> BoxFuture<'_, anyhow::Result<Out>> {
    Box::pin(async move {
        // Yield so two concurrent calls overlap.
        tokio::task::yield_now().await;
        Ok(Out {
            greeted_host: c.0.to_string(),
        })
    })
}

fn numbered_registry() -> Registry<Numbered> {
    let mut r: Registry<Numbered> = Registry::new();
    r.push_op(OpSpec::<Echo, Out>::new("whoami", "Who."), whoami);
    r
}

async fn connect_server(server: RegistryServer<Numbered>) -> RunningService<RoleClient, Client> {
    let (a, b) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = server.serve(a).await.unwrap();
        let _ = running.waiting().await;
    });
    Client::default().serve(b).await.unwrap()
}

#[tokio::test]
async fn ctx_factory_runs_per_call() {
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let made = counter.clone();
    let server = RegistryServer::with_ctx_factory(
        Arc::new(numbered_registry()),
        Arc::new(move || {
            Arc::new(Numbered(
                made.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            ))
        }),
        Audience::Agent,
        Implementation::new("test-server", "0.0.1"),
    );
    let c = connect_server(server).await;
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 0);
    let (one, two) = tokio::join!(
        call(&c, "whoami", json!({}), None),
        call(&c, "whoami", json!({}), None)
    );
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 2);
    let mut seen: Vec<String> = [one, two]
        .iter()
        .map(|r| {
            r.structured_content.clone().unwrap()["greetedHost"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    seen.sort();
    assert_eq!(seen, vec!["0", "1"]);
}

#[tokio::test]
async fn instructions_in_get_info() {
    let server = RegistryServer::new(
        Arc::new(numbered_registry()),
        Arc::new(Numbered(7)),
        Audience::Agent,
        Implementation::new("test-server", "0.0.1"),
    )
    .with_instructions("Use whoami first.");
    let c = connect_server(server).await;
    let info = c.peer_info().unwrap();
    assert_eq!(info.instructions.as_deref(), Some("Use whoami first."));
}

#[tokio::test]
async fn new_server_unchanged() {
    let server = RegistryServer::new(
        Arc::new(numbered_registry()),
        Arc::new(Numbered(7)),
        Audience::Agent,
        Implementation::new("test-server", "0.0.1"),
    );
    let c = connect_server(server).await;
    assert_eq!(c.peer_info().unwrap().instructions, None);
    for _ in 0..2 {
        let r = call(&c, "whoami", json!({}), None).await;
        assert_eq!(r.structured_content, Some(json!({"greetedHost": "7"})));
    }
}
