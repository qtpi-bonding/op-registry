#![allow(clippy::err_expect, clippy::ok_expect)]
mod common;

use common::*;
use op_listen::listen_entry;
use op_registry::mcp;
use op_registry::parity::cli_surface;
use op_registry::{cli, Audience, CallOptions, Caller, Cursor, Descriptor, Registry};
use serde_json::json;
use std::sync::Arc;

fn toy() -> Registry<Ctx> {
    let mut registry = Registry::new();
    let l = LogListener::new(vec![(1, "a"), (2, "b"), (3, "c")]);
    listen_entry(
        &mut registry,
        Arc::new(l),
        Descriptor::new("log", "listen").description("listens to the log"),
    );
    registry
}

#[test]
fn listen_entry_is_a_cli_only_streaming_op_and_absent_from_the_tool_list() {
    let registry = toy();
    let d = registry.get("log_listen").expect("registered");
    assert!(d.streaming);
    assert!(
        d.output_schema.to_string().contains("frame"),
        "output schema must describe the frames: {}",
        d.output_schema
    );

    for audience in [Audience::Agent, Audience::Owner, Audience::Dev] {
        assert!(
            mcp::tool_list(&registry, audience)
                .iter()
                .all(|t| t.name != "log_listen"),
            "{audience:?}"
        );
    }

    let cmd = cli::command(&registry, "app", "toy");
    assert!(cli_surface(&cmd).names.contains("log_listen"));
    cmd.clone().debug_assert();

    futures_executor::block_on(async {
        // A bad cursor argument is a coded error.
        let err = registry
            .call_stream(
                "log_listen",
                json!({"topic": "t"}),
                &Ctx,
                CallOptions::default().with_caller(Caller("tester".to_owned())),
                Some(Cursor("garbage".to_owned())),
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
                Some(Cursor("0:2".to_owned())),
            )
            .await
            .unwrap();
        assert_eq!(
            frames_json(s).await,
            vec![json!({"frame": "item", "item": "c", "cursor": "0:3"})]
        );
    });
}
