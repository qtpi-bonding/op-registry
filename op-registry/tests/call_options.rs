#![allow(clippy::err_expect, clippy::ok_expect)]
use futures_util::{future::BoxFuture, stream::BoxStream, FutureExt, StreamExt};
use op_registry::{CallOptions, Caller, CliHooks, Cursor, Descriptor, OpParams, Registry};
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
