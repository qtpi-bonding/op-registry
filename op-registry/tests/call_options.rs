#![allow(clippy::err_expect, clippy::ok_expect)]
use futures_util::{future::BoxFuture, stream::BoxStream, FutureExt, StreamExt};
use op_registry::{
    registry, CallOptions, Caller, CliHooks, Cursor, Descriptor, OpParams, Registry,
};
use serde_json::json;

struct Ctx;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, OpParams)]
struct Empty {
    #[allow(dead_code)]
    value: Option<i32>,
}

#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
struct Who {
    caller: Option<String>,
}

fn whoami<'a>(
    _: &'a Ctx,
    _: Empty,
    _: Option<Cursor>,
    opts: CallOptions,
) -> BoxFuture<'a, anyhow::Result<BoxStream<'a, anyhow::Result<Who>>>> {
    async move {
        let who = Who {
            caller: opts.caller.map(|c| c.0),
        };
        Ok(futures_util::stream::iter(vec![Ok(who)]).boxed())
    }
    .boxed()
}

fn toy() -> Registry<Ctx> {
    let mut registry = Registry::new();
    registry.push_stream_borrowed::<Empty, Who, _>(
        Descriptor::new("who", "am").streaming(true),
        whoami,
        CliHooks::for_params::<Empty>(),
    );
    registry
}

async fn caller_seen(registry: &Registry<Ctx>, opts: CallOptions) -> Vec<serde_json::Value> {
    registry
        .call_stream("who_am", json!({}), &Ctx, opts, None)
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await
}

#[test]
fn call_options_default_has_no_caller() {
    let opts = CallOptions::default();
    assert_eq!(opts.caller, None);
    assert_eq!(Caller::default(), Caller(String::new()));
}

#[test]
fn call_options_carry_the_caller() {
    let opts = CallOptions::default().with_caller(Caller("alice".to_owned()));
    assert_eq!(opts.caller, Some(Caller("alice".to_owned())));
    assert_eq!(opts.clone().caller, opts.caller);

    let registry = toy();
    futures_executor::block_on(async {
        assert_eq!(
            caller_seen(&registry, opts).await,
            vec![json!({"caller": "alice"})]
        );
        assert_eq!(
            caller_seen(&registry, CallOptions::default()).await,
            vec![json!({"caller": null})]
        );
    });
}

async fn who_unary(_: &Ctx, _: Empty, opts: CallOptions) -> anyhow::Result<Who> {
    Ok(Who {
        caller: opts.caller.map(|c| c.0),
    })
}

async fn who_stream(
    _: &Ctx,
    _: Empty,
    _: Option<Cursor>,
    opts: CallOptions,
) -> anyhow::Result<impl futures_util::Stream<Item = anyhow::Result<Who>>> {
    let who = Who {
        caller: opts.caller.map(|c| c.0),
    };
    Ok(futures_util::stream::iter(vec![Ok(who)]))
}

async fn plain(_: &Ctx, _: Empty) -> anyhow::Result<Who> {
    Ok(Who { caller: None })
}

fn alice() -> CallOptions {
    CallOptions::default().with_caller(Caller("alice".to_owned()))
}

#[test]
fn op_with_sees_the_caller() {
    let registry = registry! {
        Ctx = Ctx;
        entries: [{
            noun: "who", verb: "unary", description: "caller",
            access: reads, audience: Agent, params: Empty, output: Who,
            op_with: who_unary,
        }]
    };
    assert!(!registry.get("who_unary").unwrap().streaming);
    futures_executor::block_on(async {
        let seen = registry
            .call("who_unary", json!({}), &Ctx, alice())
            .await
            .unwrap();
        assert_eq!(seen, json!({"caller": "alice"}));
        let anon = registry
            .call("who_unary", json!({}), &Ctx, CallOptions::default())
            .await
            .unwrap();
        assert_eq!(anon, json!({"caller": null}));
    });
}

#[test]
fn stream_op_with_sees_the_caller_and_is_streaming() {
    let registry = registry! {
        Ctx = Ctx;
        entries: [{
            noun: "who", verb: "stream", description: "caller",
            access: reads, audience: Agent, params: Empty, output: Who,
            stream_op_with: who_stream,
        }]
    };
    assert!(registry.get("who_stream").unwrap().streaming);
    futures_executor::block_on(async {
        assert_eq!(
            caller_seen_as(&registry, "who_stream", alice()).await,
            vec![json!({"caller": "alice"})]
        );
        assert_eq!(
            caller_seen_as(&registry, "who_stream", CallOptions::default()).await,
            vec![json!({"caller": null})]
        );
    });
}

async fn caller_seen_as(
    registry: &Registry<Ctx>,
    name: &str,
    opts: CallOptions,
) -> Vec<serde_json::Value> {
    registry
        .call_stream(name, json!({}), &Ctx, opts, None)
        .await
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>()
        .await
}

#[test]
fn op_without_options_still_compiles_and_runs() {
    let registry = registry! {
        Ctx = Ctx;
        entries: [{
            noun: "who", verb: "plain", description: "plain",
            access: reads, audience: Agent, params: Empty, output: Who,
            op: plain,
        }]
    };
    assert!(!registry.get("who_plain").unwrap().streaming);
    futures_executor::block_on(async {
        let out = registry
            .call("who_plain", json!({}), &Ctx, alice())
            .await
            .unwrap();
        assert_eq!(out, json!({"caller": null}));
    });
}
