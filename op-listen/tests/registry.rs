#![allow(clippy::err_expect, clippy::ok_expect)]
mod common;

use common::*;
use op_listen::listen_entry;
use op_registry::mcp;
use op_registry::testing::assert_parity;
use op_registry::{
    cli, Audience, CallOptions, Caller, CliContext, Cursor, OpGlobals, OpSpec, Registry,
};
use op_registry::{OpParams, Ref};
use serde_json::json;
use std::sync::Arc;

#[derive(OpGlobals)]
pub struct NoGlobals {}

impl CliContext for Ctx {
    type Globals = NoGlobals;
    fn from_globals(_: &NoGlobals) -> anyhow::Result<Self> {
        Ok(Ctx)
    }
}

fn toy() -> Registry<Ctx> {
    let mut registry = Registry::new();
    registry.set_bin("app", "0.0.0", "toy");
    registry.group("log", "Log.");
    let l = LogListener::new(vec![(1, "a"), (2, "b"), (3, "c")]);
    listen_entry(
        &mut registry,
        Arc::new(l),
        OpSpec::new("log listen", "listens to the log"),
    );
    registry
}

#[test]
fn listen_entry_output_schema_is_the_frame_schema() {
    let registry = toy();
    let d = registry.get("log_listen").expect("registered");
    // Every frame variant is an object, so the descriptor marks the root `"type": "object"`.
    let mut expected =
        <op_listen::ListenFrame<(), String> as op_registry::OutputSchema>::output_schema()
            .expect("frame schema");
    expected["type"] = json!("object");
    assert_eq!(d.output_schema, Some(expected));
}

#[test]
fn listen_entry_is_a_cli_only_streaming_op_and_absent_from_the_tool_list() {
    let registry = toy();
    let d = registry.get("log_listen").expect("registered");
    assert_eq!(d.kind, op_registry::OpKind::Stream);
    assert!(
        d.output_schema
            .as_ref()
            .unwrap()
            .to_string()
            .contains("frame"),
        "output schema must describe the frames: {}",
        d.output_schema.as_ref().unwrap()
    );

    for audience in [Audience::Agent, Audience::Owner, Audience::Dev] {
        assert!(
            mcp::tool_list(&registry, audience)
                .iter()
                .all(|t| t.name != "log_listen"),
            "{audience:?}"
        );
    }

    let cmd = cli::command(&registry);
    assert_parity(&registry, &[]);
    cmd.clone().debug_assert();
    // The listen leaf resumes from a cursor, so it takes `--cursor`.
    let argv: Vec<std::ffi::OsString> = ["app", "log", "listen", "--topic", "t", "--cursor", "0:2"]
        .into_iter()
        .map(Into::into)
        .collect();
    let (parsed, _) = cli::parse(&registry, &argv).ok().expect("--cursor parses");
    assert!(
        matches!(parsed, cli::Parsed::Op { ref name, cursor: Some(_), .. } if name == "log_listen")
    );

    futures_executor::block_on(async {
        // A bad cursor argument is a coded error.
        let err = registry
            .call_stream(
                "log_listen",
                json!({"topic": "t"}),
                &Ctx,
                CallOptions::default().with_caller(Caller("tester".to_owned())),
                Some(Cursor::new("garbage")),
            )
            .await
            .err()
            .expect("bad cursor must fail");
        assert_eq!(code_of(err.error()), "op_listen.bad_cursor");

        // A good cursor resumes after it.
        let s = registry
            .call_stream(
                "log_listen",
                json!({"topic": "t"}),
                &Ctx,
                CallOptions::default().with_caller(Caller("tester".to_owned())),
                Some(Cursor::new("0:2")),
            )
            .await
            .unwrap();
        assert_eq!(
            frames_json(s).await,
            vec![json!({"frame": "item", "item": "c", "cursor": "0:3"})]
        );
    });
}

#[tokio::test]
async fn kit_listen_op_reaches_body() {
    let listener = Arc::new(LogListener::new(vec![(1, "a")]));
    let mut registry = Registry::new();
    registry.set_bin("app", "0.0.0", "toy");
    registry.group("log", "Log.");
    listen_entry(
        &mut registry,
        listener.clone(),
        OpSpec::new("log listen", "listens to the log"),
    );
    op_registry::testing::assert_error_codes(
        &registry,
        &Ctx,
        &[op_registry::testing::Fixture {
            op: "log_listen",
            args: json!({"topic": "t"}),
        }],
    )
    .await;
    // The kit's caller let the call past `op_listen.no_caller` to the listener.
    assert!(!listener.authorize_calls().is_empty());
}

#[derive(Clone, Debug, serde::Serialize, OpParams)]
struct Pick {
    #[op(subject = "topic", id_only)]
    id: Option<Ref>,
}

#[derive(Clone, Debug, serde::Serialize, OpParams)]
struct PickGroup {
    #[op(flatten)]
    pick: Pick,
    name: Option<String>,
}

struct PickListener(LogListener);

impl op_listen::Listener<Ctx> for PickListener {
    type Params = PickGroup;
    type Snapshot = ();
    type Item = String;

    fn topic(&self, params: &PickGroup) -> String {
        params
            .pick
            .id
            .as_ref()
            .map_or_else(|| "none".to_owned(), |r| r.id().to_owned())
    }
    fn epoch(&self) -> u64 {
        0
    }
    fn authorize<'a>(
        &'a self,
        ctx: &'a Ctx,
        caller: &'a Caller,
        topic: &'a str,
    ) -> futures_util::future::BoxFuture<'a, anyhow::Result<op_listen::Grant>> {
        self.0.authorize(ctx, caller, topic)
    }
    fn open<'a>(
        &'a self,
        ctx: &'a Ctx,
        _params: PickGroup,
        cursor: Option<op_listen::ListenCursor>,
        floor: Option<op_listen::ListenCursor>,
    ) -> futures_util::future::BoxFuture<'a, anyhow::Result<op_listen::Opened<'a, (), String>>>
    {
        self.0.open(ctx, params_topic(), cursor, floor)
    }
}

fn params_topic() -> TopicParams {
    params("t")
}

#[tokio::test]
async fn listen_entry_with_optional_subject_group() {
    let listener = Arc::new(PickListener(LogListener::new(vec![(1, "a")])));
    let mut registry = Registry::new();
    registry.set_bin("app", "0.0.0", "toy");
    registry.group("log", "Log.");
    listen_entry(
        &mut registry,
        listener.clone(),
        OpSpec::new("log pick", "listens to a picked topic"),
    );
    let caller = || CallOptions::default().with_caller(Caller("tester".to_owned()));
    // The optional subject is absent: no slot, the stream opens.
    let s = registry
        .call_stream("log_pick", json!({}), &Ctx, caller(), None)
        .await
        .unwrap();
    assert_eq!(
        frames_json(s).await,
        vec![json!({"frame": "item", "item": "a", "cursor": "0:1"})]
    );
    // Present: id_only passes it through to the listener.
    let s = registry
        .call_stream("log_pick", json!({"id": "t9"}), &Ctx, caller(), None)
        .await
        .unwrap();
    assert_eq!(frames_json(s).await.len(), 1);
    assert_eq!(
        listener.0.authorize_calls(),
        [
            ("tester".to_owned(), "none".to_owned()),
            ("tester".to_owned(), "t9".to_owned())
        ]
    );
    // `--id` is a flag of the leaf, and `id` is not required.
    let cmd = cli::command(&registry);
    let argv: Vec<std::ffi::OsString> =
        ["app", "log", "pick"].into_iter().map(Into::into).collect();
    assert!(cli::parse(&registry, &argv).ok().is_some());
    cmd.clone().debug_assert();
}

// ---- listen_entry_following ----

struct CliCtx;
impl op_registry::Resolve for CliCtx {}
impl CliContext for CliCtx {
    type Globals = NoGlobals;
    fn from_globals(_: &NoGlobals) -> anyhow::Result<Self> {
        Ok(CliCtx)
    }
    fn caller(&self) -> Option<Caller> {
        Some(Caller("cli".to_owned()))
    }
}

/// A listener on `CliCtx` over an in-memory log; with `revoke` the periodic
/// recheck stops the stream after its replay.
struct CliLog(LogListener);

impl op_listen::Listener<CliCtx> for CliLog {
    type Params = TopicParams;
    type Snapshot = ();
    type Item = String;

    fn topic(&self, params: &TopicParams) -> String {
        params.topic.clone()
    }
    fn epoch(&self) -> u64 {
        0
    }
    fn authorize<'a>(
        &'a self,
        _ctx: &'a CliCtx,
        caller: &'a Caller,
        topic: &'a str,
    ) -> futures_util::future::BoxFuture<'a, anyhow::Result<op_listen::Grant>> {
        self.0.authorize(&Ctx, caller, topic)
    }
    fn still_allowed<'a>(
        &'a self,
        _ctx: &'a CliCtx,
        caller: &'a Caller,
        topic: &'a str,
    ) -> futures_util::future::BoxFuture<'a, bool> {
        self.0.still_allowed(&Ctx, caller, topic)
    }
    fn open<'a>(
        &'a self,
        _ctx: &'a CliCtx,
        params: TopicParams,
        cursor: Option<op_listen::ListenCursor>,
        floor: Option<op_listen::ListenCursor>,
    ) -> futures_util::future::BoxFuture<'a, anyhow::Result<op_listen::Opened<'a, (), String>>>
    {
        self.0.open(&Ctx, params, cursor, floor)
    }
}

fn following_registry(listener: Arc<CliLog>) -> Registry<CliCtx> {
    let mut registry = Registry::new();
    registry.set_bin("app", "0.0.0", "toy");
    registry.group("log", "Log.");
    op_listen::listen_entry_following(
        &mut registry,
        listener,
        OpSpec::new("log follow", "follows the log"),
    );
    registry
}

#[tokio::test]
async fn following_entry_prints_ndjson_and_exits_65_after_stop() {
    let mut inner = LogListener::new(vec![(1, "a"), (2, "b")])
        .with_grant(grant(None, Some(std::time::Duration::from_millis(5))));
    inner.tail_pending = true;
    inner
        .allowed
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let registry = following_registry(Arc::new(CliLog(inner)));
    let argv: Vec<std::ffi::OsString> = ["app", "log", "follow", "--topic", "t"]
        .into_iter()
        .map(Into::into)
        .collect();
    let (io, cap) = cli::Io::memory(b"", false, false);
    let code = cli::run_with_io(&registry, argv, &cli::NoHooks, io).await;
    assert_eq!(code, 65, "stderr: {}", cap.stderr());
    let lines: Vec<serde_json::Value> = cap
        .stdout()
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON value per line"))
        .collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(
        lines[0],
        json!({"frame": "item", "item": "a", "cursor": "0:1"})
    );
    assert_eq!(
        lines[1],
        json!({"frame": "item", "item": "b", "cursor": "0:2"})
    );
    assert_eq!(lines[2]["frame"], "stop");
    assert_eq!(lines[2]["code"], "op_listen.revoked");
    assert!(
        cap.stderr().contains("op_listen.stopped"),
        "{}",
        cap.stderr()
    );
}

#[tokio::test]
async fn following_entry_passes_params_and_cursor() {
    let mut inner = LogListener::new(vec![(1, "a"), (2, "b"), (3, "c")])
        .with_grant(grant(None, Some(std::time::Duration::from_millis(5))));
    inner.tail_pending = true;
    inner
        .allowed
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let listener = Arc::new(CliLog(inner));
    let registry = following_registry(listener.clone());
    let mut s = registry
        .call_stream(
            "log_follow",
            json!({"topic": "t9"}),
            &CliCtx,
            CallOptions::default()
                .with_caller(Caller("tester".to_owned()))
                .with_max_retries(0),
            Some(Cursor::new("0:2")),
        )
        .await
        .unwrap();
    let mut seen = Vec::new();
    let mut end = None;
    while let Some(next) = futures_util::StreamExt::next(&mut s).await {
        match next {
            Ok(v) => seen.push(v),
            Err(e) => end = Some(e),
        }
    }
    assert_eq!(
        seen[0],
        json!({"frame": "item", "item": "c", "cursor": "0:3"})
    );
    assert_eq!(seen[1]["frame"], "stop");
    assert_eq!(
        code_of(end.expect("ends with an error").error()),
        "op_listen.stopped"
    );
    assert_eq!(
        listener.0.authorize_calls(),
        [("tester".to_owned(), "t9".to_owned())]
    );
    assert_eq!(
        listener.0.open_calls(),
        [(Some(op_listen::ListenCursor::new(0, 2)), None)]
    );
}

#[test]
fn following_entry_lists_stopped_in_descriptor_and_manual() {
    let registry = following_registry(Arc::new(CliLog(LogListener::new(vec![]))));
    let d = registry.get("log_follow").expect("registered");
    let stopped = d
        .errors
        .iter()
        .find(|e| e.code == "op_listen.stopped")
        .expect("descriptor lists op_listen.stopped");
    assert_eq!(stopped.class, Some(coded_error::ErrorClass::Logical));
    let manual = op_registry::manual::render(&registry, &Default::default()).unwrap();
    assert!(
        manual.contains("| op_listen.stopped | logical | 65 |"),
        "{manual}"
    );
}
