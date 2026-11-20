//! MCP core: tool specs, annotations, and calls through the one pipeline.

#[cfg(feature = "secrets")]
use coded_error::ErrorClass;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use op_registry::mcp::{call_tool, tool_error, tool_list};
use op_registry::{
    Access, Audience, CallHook, CallInfo, CallOptions, CallPath, OpKind, OpParams, OpSpec, Page,
    PageRequest, Paths, Registry, Surface,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct Ctx;
impl op_registry::Resolve for Ctx {}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct Plain {
    /// A host machine.
    host_machine: Option<String>,
}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct Group {
    /// The project root.
    project_root: Option<String>,
}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct Wrapped {
    own_name: Option<String>,
    #[op(flatten)]
    group: Group,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Out {
    ok: bool,
}

fn ok_op<P: Send + 'static>(
    _c: &Ctx,
    _p: P,
    _o: CallOptions,
) -> BoxFuture<'_, anyhow::Result<Out>> {
    Box::pin(async { Ok(Out { ok: true }) })
}

fn registry() -> Registry<Ctx> {
    let mut r: Registry<Ctx> = Registry::new();
    let mut read = OpSpec::<Plain, Out>::new("read_it", "Reads it.");
    read.access = Access::reads();
    read.long_help = "More detail.";
    r.push_op(read, ok_op::<Plain>);
    let mut hidden = OpSpec::<Plain, Out>::new("hidden_op", "Hidden.");
    hidden.hidden = true;
    r.push_op(hidden, ok_op::<Plain>);
    let mut typed = OpSpec::<Plain, Out>::new("typed_op", "Typed only.");
    typed.paths = Paths::of(&[Surface::Typed]);
    r.push_op(typed, ok_op::<Plain>);
    let mut owner = OpSpec::<Plain, Out>::new("owner_op", "Owner.");
    owner.audience = Audience::Owner;
    r.push_op(owner, ok_op::<Plain>);
    r.push_local(
        OpSpec::<Plain, Out>::new("local_op", "Local."),
        ok_op::<Plain>,
    );
    r.push_stream(
        OpSpec::<Plain, Out>::new("flow", "Streams."),
        |_c: &Ctx, _p: Plain, _cur, _o: CallOptions| {
            Box::pin(async {
                Ok(
                    Box::pin(futures_util::stream::iter(vec![Ok(Out { ok: true })]))
                        as BoxStream<'_, anyhow::Result<Out>>,
                )
            })
        },
    );
    r
}

fn names(r: &Registry<Ctx>, audience: Audience) -> Vec<String> {
    tool_list(r, audience).into_iter().map(|t| t.name).collect()
}

#[test]
fn tool_list_filters_hidden_stream_typed_and_audience() {
    let r = registry();
    assert_eq!(names(&r, Audience::Agent), vec!["read_it", "local_op"]);
    assert_eq!(names(&r, Audience::Owner), vec!["owner_op"]);
    assert!(names(&r, Audience::Dev).is_empty());
    let tools = tool_list(&r, Audience::Agent);
    assert_eq!(tools[0].description, "Reads it.\n\nMore detail.");
    assert_eq!(tools[1].description, "Local.");
}

#[tokio::test]
async fn hidden_op_still_callable_on_mcp() {
    let r = registry();
    let out = call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "hidden_op",
        json!({}),
        CallOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(out, json!({"ok": true}));
}

#[tokio::test]
async fn call_errors_by_audience_shape_and_path() {
    let r = registry();
    let code = |name: &'static str, audience| {
        let r = &r;
        async move {
            let err = call_tool(r, &Ctx, audience, name, json!({}), CallOptions::default())
                .await
                .unwrap_err();
            tool_error(&err, "t", "fallback").code
        }
    };
    assert_eq!(
        code("owner_op", Audience::Agent).await,
        "op_registry.call.outside_audience"
    );
    assert_eq!(
        code("flow", Audience::Agent).await,
        "op_registry.call.wrong_shape"
    );
    assert_eq!(
        code("typed_op", Audience::Agent).await,
        "op_registry.call.wrong_path"
    );
    assert_eq!(
        code("nope", Audience::Agent).await,
        "op_registry.call.unknown_operation"
    );
}

#[test]
fn annotations_from_access() {
    let mut r: Registry<Ctx> = Registry::new();
    for (name, access) in [
        ("a_read", Access::reads()),
        ("b_write", Access::writes()),
        ("c_destroy", Access::destructive()),
        ("d_both", Access::reads().with_destructive()),
        ("e_none", Access::default()),
    ] {
        let mut spec = OpSpec::<Plain, Out>::new(name, "x");
        spec.access = access;
        r.push_op(spec, ok_op::<Plain>);
    }
    let hints: Vec<(bool, bool)> = tool_list(&r, Audience::Agent)
        .iter()
        .map(|t| (t.annotations.read_only_hint, t.annotations.destructive_hint))
        .collect();
    assert_eq!(
        hints,
        vec![
            (true, false),
            (false, false),
            (false, true),
            (false, true),
            (false, false)
        ]
    );
    let json = serde_json::to_value(&tool_list(&r, Audience::Agent)[0]).unwrap();
    assert_eq!(
        json["annotations"],
        json!({"readOnlyHint": true, "destructiveHint": false})
    );
    assert!(json.get("inputSchema").is_some());
}

fn input_of<P: OpParams + Send + 'static>(name: &'static str) -> Value {
    let mut r: Registry<Ctx> = Registry::new();
    r.push_op(OpSpec::<P, Out>::new(name, "x"), ok_op::<P>);
    tool_list(&r, Audience::Agent).remove(0).input_schema
}

#[test]
fn input_keys_camel_including_flattened() {
    let schema = input_of::<Wrapped>("wrapped");
    let props = schema["properties"].as_object().unwrap();
    assert!(props.contains_key("ownName"), "{schema}");
    assert!(props.contains_key("projectRoot"), "{schema}");
    assert!(!props.contains_key("own_name") && !props.contains_key("project_root"));
}

#[test]
fn globals_are_not_tool_params() {
    let schema = input_of::<Wrapped>("wrapped");
    let keys: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 2, "{schema}");
}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct Lst {
    tag: Option<String>,
}

#[test]
fn list_pair_both_tools() {
    let mut r: Registry<Ctx> = Registry::new();
    r.set_pillar("fix");
    r.group("thing", "Things.");
    r.push_list(
        OpSpec::<Lst, Out>::new("thing list", "List things."),
        |_c: &Ctx, _p: Lst, _req: PageRequest| -> BoxFuture<'_, anyhow::Result<Page<Out>>> {
            Box::pin(async {
                Ok(Page {
                    items: vec![Out { ok: true }],
                    next_cursor: None,
                })
            })
        },
    );
    let kinds: Vec<OpKind> = r.descriptors().iter().map(|d| d.kind).collect();
    assert!(kinds.contains(&OpKind::ListAll) && kinds.contains(&OpKind::ListPage));
    let listed = names(&r, Audience::Agent);
    assert_eq!(listed.len(), 2, "{listed:?}");
    for d in r.descriptors() {
        assert!(listed.contains(&d.name));
    }
}

#[tokio::test]
async fn call_tool_returns_items_object() {
    let mut r: Registry<Ctx> = Registry::new();
    r.set_pillar("fix");
    r.group("thing", "Things.");
    r.push_list(
        OpSpec::<Lst, Out>::new("thing list", "List things."),
        |_c: &Ctx, _p: Lst, _req: PageRequest| -> BoxFuture<'_, anyhow::Result<Page<Out>>> {
            Box::pin(async {
                Ok(Page {
                    items: vec![Out { ok: true }],
                    next_cursor: None,
                })
            })
        },
    );
    let out = call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "thing_list",
        json!({}),
        CallOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(out, json!({"items": [{"ok": true}]}));
    #[cfg(feature = "schema")]
    {
        let specs = tool_list(&r, Audience::Agent);
        let spec = specs.iter().find(|s| s.name == "thing_list").unwrap();
        let schema = spec.output_schema.as_ref().expect("an output schema");
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["items"]));
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<CallPath>>);
impl CallHook for Recorder {
    fn around<'a>(&'a self, info: &CallInfo, call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()> {
        self.0.lock().unwrap().push(info.path.clone());
        call
    }
}

#[tokio::test]
async fn call_tool_path_is_mcp() {
    let mut r = registry();
    let rec = Arc::new(Recorder::default());
    r.set_call_hook(rec.clone());
    call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "read_it",
        json!({}),
        CallOptions::default(),
    )
    .await
    .unwrap();
    // a transport label does not change the path
    call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "read_it",
        json!({}),
        CallOptions::default().with_transport("rest"),
    )
    .await
    .unwrap();
    assert_eq!(*rec.0.lock().unwrap(), vec![CallPath::Mcp, CallPath::Mcp]);
}

#[cfg(feature = "secrets")]
#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.app")]
enum Leaky {
    /// A logical error is shown as is by the class rules, so only the call's
    /// redactor can hide the secret in it.
    #[error("rejected {0}")]
    #[coded(code = "rejected", class = Logical)]
    Rejected(String),
}

#[cfg(feature = "secrets")]
#[derive(Clone, Debug, OpParams)]
struct Keyed {
    #[op(secret)]
    token: String,
}

#[cfg(feature = "secrets")]
#[tokio::test]
async fn tool_error_redacts_secret() {
    let mut r: Registry<Ctx> = Registry::new();
    r.push_op(
        OpSpec::<Keyed, Out>::new("keyed", "x"),
        |_c: &Ctx, p: Keyed, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async move { Err(coded_error::into_anyhow(Leaky::Rejected(p.token))) })
        },
    );
    let err = call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "keyed",
        json!({"token": "hunter2-secret"}),
        CallOptions::default(),
    )
    .await
    .unwrap_err();
    let failure = tool_error(&err, "t", "fallback");
    assert_eq!(failure.class, ErrorClass::Logical);
    assert_eq!(failure.message, "rejected <secret>");
    assert!(!failure.fingerprint.is_empty());
}

#[cfg(not(feature = "schema"))]
#[test]
fn output_schema_none_without_schema_feature() {
    let mut r: Registry<Ctx> = Registry::new();
    r.push_op(OpSpec::<Plain, Out>::new("a", "x"), ok_op::<Plain>);
    let tool = tool_list(&r, Audience::Agent).remove(0);
    assert_eq!(tool.output_schema, None);
    assert!(serde_json::to_value(&tool)
        .unwrap()
        .get("outputSchema")
        .is_none());
}

// DEC-1: one op in several audiences.

struct SeenAudiences(Mutex<Vec<(CallPath, Audience)>>);
impl CallHook for SeenAudiences {
    fn around<'a>(&'a self, info: &CallInfo, call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()> {
        self.0
            .lock()
            .unwrap()
            .push((info.path.clone(), info.audience));
        call
    }
}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct NoArgs {
    tag: Option<String>,
}

fn shared_registry() -> Registry<Ctx> {
    let mut r: Registry<Ctx> = Registry::new();
    let mut shared = OpSpec::<NoArgs, Out>::new("shared_op", "Agent and owner.");
    shared.audience = Audience::Agent;
    shared.more_audiences = vec![Audience::Owner];
    r.push_op(shared, ok_op::<NoArgs>);
    let mut agent = OpSpec::<NoArgs, Out>::new("agent_op", "Agent only.");
    agent.audience = Audience::Agent;
    r.push_op(agent, ok_op::<NoArgs>);
    r
}

#[tokio::test]
async fn multi_audience_op_in_both_tool_lists_and_callable() {
    let r = shared_registry();
    assert!(names(&r, Audience::Agent).contains(&"shared_op".to_owned()));
    assert!(names(&r, Audience::Owner).contains(&"shared_op".to_owned()));
    assert!(!names(&r, Audience::Dev).contains(&"shared_op".to_owned()));
    let descriptor = r.get("shared_op").unwrap();
    assert_eq!(descriptor.audience, Audience::Agent);
    assert_eq!(descriptor.audiences, vec![Audience::Agent, Audience::Owner]);
    for audience in [Audience::Agent, Audience::Owner] {
        let out = call_tool(
            &r,
            &Ctx,
            audience,
            "shared_op",
            json!({}),
            CallOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(out, json!({"ok": true}));
    }
}

#[tokio::test]
async fn agent_only_op_outside_owner_server() {
    let r = shared_registry();
    assert!(!names(&r, Audience::Owner).contains(&"agent_op".to_owned()));
    let err = call_tool(
        &r,
        &Ctx,
        Audience::Owner,
        "agent_op",
        json!({}),
        CallOptions::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        tool_error(&err, "t", "fallback").code,
        "op_registry.call.outside_audience"
    );
}

#[tokio::test]
async fn call_info_audience_is_server_on_mcp_primary_elsewhere() {
    let mut r = shared_registry();
    let seen = Arc::new(SeenAudiences(Mutex::new(Vec::new())));
    r.set_call_hook(seen.clone());
    call_tool(
        &r,
        &Ctx,
        Audience::Owner,
        "shared_op",
        json!({}),
        CallOptions::default(),
    )
    .await
    .unwrap();
    call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "shared_op",
        json!({}),
        CallOptions::default(),
    )
    .await
    .unwrap();
    r.call("shared_op", json!({}), &Ctx, CallOptions::default())
        .await
        .unwrap();
    struct Shared;
    impl op_registry::Op for Shared {
        type Ctx = Ctx;
        type Params = NoArgs;
        type Output = Out;
        const NAME: &'static str = "shared_op";
    }
    r.call_typed::<Shared>(&Ctx, NoArgs { tag: None }, CallOptions::default())
        .await
        .unwrap();
    let seen = seen.0.lock().unwrap();
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[0], (CallPath::Mcp, Audience::Owner));
    assert_eq!(seen[1], (CallPath::Mcp, Audience::Agent));
    assert_eq!(seen[2].1, Audience::Agent);
    assert_eq!(seen[3], (CallPath::Typed, Audience::Agent));
}

#[test]
fn list_halves_share_audiences() {
    let mut r: Registry<Ctx> = Registry::new();
    r.set_pillar("fix");
    r.group("thing", "Things.");
    let mut spec = OpSpec::<Lst, Out>::new("thing list", "List things.");
    spec.audience = Audience::Agent;
    spec.more_audiences = vec![Audience::Owner];
    r.push_list(
        spec,
        |_c: &Ctx, _p: Lst, _req: PageRequest| -> BoxFuture<'_, anyhow::Result<Page<Out>>> {
            Box::pin(async {
                Ok(Page {
                    items: vec![],
                    next_cursor: None,
                })
            })
        },
    );
    let owner = names(&r, Audience::Owner);
    assert_eq!(owner.len(), 2, "{owner:?}");
    for d in r.descriptors() {
        assert_eq!(d.audiences, vec![Audience::Agent, Audience::Owner]);
        assert!(owner.contains(&d.name));
    }
}
