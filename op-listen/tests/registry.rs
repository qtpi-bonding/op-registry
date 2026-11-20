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
    assert_eq!(
        d.output_schema,
        <op_listen::ListenFrame<(), String> as op_registry::OutputSchema>::output_schema()
    );
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
        assert_eq!(code_of(&err), "op_listen.bad_cursor");

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
