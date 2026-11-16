use coded_error::{ErrorClass, ErrorInfo, Facts, Overrides};
use futures_util::{stream, StreamExt};
use op_registry::{registry, CallOptions, Cursor, OpParams};
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Debug, Deserialize, serde::Serialize, OpParams)]
#[allow(dead_code)]
struct Empty {
    value: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, OpParams)]
struct Input {
    value: i32,
}

#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
struct Output {
    result: i32,
}

#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
struct TextOutput {
    text: String,
}

struct FirstCtx {
    value: i32,
}
struct SecondCtx {
    value: i32,
}

async fn get_action(ctx: &FirstCtx, _: Empty) -> anyhow::Result<Output> {
    Ok(Output { result: ctx.value })
}

async fn explicit_action(ctx: &FirstCtx, _: Empty) -> anyhow::Result<Output> {
    Ok(Output {
        result: ctx.value + 1,
    })
}

async fn first_value(ctx: &FirstCtx, _: Empty) -> anyhow::Result<Output> {
    Ok(Output { result: ctx.value })
}

async fn second_value(ctx: &SecondCtx, _: Empty) -> anyhow::Result<Output> {
    Ok(Output { result: ctx.value })
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.operation")]
enum MyCoded {
    #[error("original failure")]
    #[coded(code = "failed", class = Logical)]
    Failed,
}

async fn coded_failure(_: &FirstCtx, _: Input) -> anyhow::Result<Output> {
    Err(coded_error::into_anyhow(MyCoded::Failed).context("while doing x"))
}

async fn stream_values(
    _: &FirstCtx,
    input: Input,
    _: Option<Cursor>,
) -> anyhow::Result<impl futures_util::Stream<Item = anyhow::Result<TextOutput>>> {
    Ok(stream::iter(vec![
        Ok(TextOutput {
            text: format!("{}-one", input.value),
        }),
        Ok(TextOutput {
            text: format!("{}-two", input.value),
        }),
    ]))
}

fn registry_for_calls() -> op_registry::Registry<FirstCtx> {
    registry! {
        Ctx = FirstCtx;
        entries: [
            {
                name: "failing", noun: "operation", verb: "fail", description: "fails",
                access: none, audience: Agent, params: Input, output: Output,
                op: coded_failure,
            },
            {
                noun: "value", verb: "get", description: "gets a value",
                access: reads, audience: Agent, params: Empty, output: Output,
                op: get_action,
            },
        ]
    }
}

fn stream_registry() -> op_registry::Registry<FirstCtx> {
    registry! {
        Ctx = FirstCtx;
        entries: [{
            noun: "value", verb: "stream", description: "streams values",
            access: reads, audience: Agent, params: Input, output: TextOutput,
            stream_op: stream_values,
        }]
    }
}

fn mixed_registry() -> op_registry::Registry<FirstCtx> {
    registry! {
        Ctx = FirstCtx;
        entries: [
            {
                noun: "value", verb: "get", description: "gets a value",
                access: reads, audience: Agent, params: Empty, output: Output,
                op: get_action,
            },
            {
                noun: "value", verb: "stream", description: "streams values",
                access: reads, audience: Agent, params: Input, output: TextOutput,
                stream_op: stream_values,
            },
        ]
    }
}

#[test]
fn registry_names_derive_noun_verb_and_honour_explicit_name() {
    let registry = registry! {
        Ctx = FirstCtx;
        entries: [
            {
                noun: "noun", verb: "verb", description: "derived",
                access: reads, audience: Agent, params: Empty, output: Output,
                op: get_action,
            },
            {
                name: "chosen", noun: "other", verb: "action", description: "explicit",
                access: writes, audience: Owner, params: Empty, output: Output,
                op: explicit_action,
            },
        ]
    };
    let descriptors = registry.descriptors();
    assert_eq!(descriptors.len(), 2);
    assert_eq!(descriptors[0].name, "noun_verb");
    assert_eq!(descriptors[0].cli_name(), "noun verb");
    assert_eq!(descriptors[1].name, "chosen");
    assert_eq!(registry.get("chosen").unwrap().noun, "other");
    assert_eq!(registry.names(), vec!["noun_verb", "chosen"]);
}

#[test]
fn two_registries_in_one_crate_do_not_collide() {
    futures_executor::block_on(async {
        let first = registry! {
            Ctx = FirstCtx;
            entries: [{
                noun: "same", verb: "thing", description: "first",
                access: reads, audience: Agent, params: Empty, output: Output,
                op: first_value,
            }]
        };
        let second = registry! {
            Ctx = SecondCtx;
            entries: [{
                noun: "same", verb: "thing", description: "second",
                access: reads, audience: Agent, params: Empty, output: Output,
                op: second_value,
            }]
        };
        let first_value = first
            .call(
                "same_thing",
                json!({}),
                &FirstCtx { value: 11 },
                CallOptions::default(),
            )
            .await
            .unwrap();
        let second_value = second
            .call(
                "same_thing",
                json!({}),
                &SecondCtx { value: 22 },
                CallOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(first_value["result"], 11);
        assert_eq!(second_value["result"], 22);
    });
}

#[test]
fn mixed_registry_preserves_order_shapes_and_calls() {
    futures_executor::block_on(async {
        let registry = mixed_registry();
        let descriptors = registry.descriptors();
        assert_eq!(descriptors[0].name, "value_get");
        assert!(!descriptors[0].streaming);
        assert_eq!(descriptors[0].input_schema, Empty::schema());
        assert_eq!(descriptors[1].name, "value_stream");
        assert!(descriptors[1].streaming);
        assert_eq!(descriptors[1].input_schema, Input::schema());

        let value = registry
            .call(
                "value_get",
                json!({}),
                &FirstCtx { value: 5 },
                CallOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(value, json!({"result": 5}));
        let stream = registry
            .call_stream(
                "value_stream",
                json!({"value": 8}),
                &FirstCtx { value: 0 },
                CallOptions::default(),
                None,
            )
            .await
            .unwrap();
        let values = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(values.len(), 2);
    });
}

// The macro's operation type is intentionally local to the expression, so it
// cannot be named from this integration test. The registry descriptor above
// is built by the same local helper used by that operation's implementation.

#[test]
fn call_returns_coded_error_chain_intact() {
    futures_executor::block_on(async {
        let registry = registry_for_calls();
        let streams = stream_registry();
        let error = registry
            .call(
                "failing",
                json!({"value": 1}),
                &FirstCtx { value: 0 },
                CallOptions::default(),
            )
            .await
            .unwrap_err();
        let facts = coded_error::facts_anyhow(&error);
        assert_eq!(facts.code, "test.operation.failed");
        assert_eq!(facts.class, ErrorClass::Logical);
        assert!(format!("{:#}", error).contains("while doing x"));
        assert!(format!("{:#}", error).contains("original failure"));

        let unknown = registry
            .call(
                "missing",
                json!({}),
                &FirstCtx { value: 0 },
                CallOptions::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            coded_error::facts_anyhow(&unknown).code,
            "op_registry.call.unknown_operation"
        );

        let bad_args = registry
            .call(
                "failing",
                json!({"unknown": true}),
                &FirstCtx { value: 0 },
                CallOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(coded_error::facts_anyhow(&bad_args)
            .code
            .starts_with("op_registry.usage."));

        let output = registry
            .call(
                "value_get",
                json!({}),
                &FirstCtx { value: 7 },
                CallOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(output, json!({"result": 7}));

        let stream_error = match streams
            .call(
                "value_stream",
                json!({"value": 3}),
                &FirstCtx { value: 0 },
                CallOptions::default(),
            )
            .await
        {
            Ok(_) => panic!("stream operation unexpectedly accepted by call"),
            Err(error) => error,
        };
        assert_eq!(
            coded_error::facts_anyhow(&stream_error).code,
            "op_registry.call.wrong_shape"
        );
        let unary_error = match registry
            .call_stream(
                "value_get",
                json!({}),
                &FirstCtx { value: 0 },
                CallOptions::default(),
                None,
            )
            .await
        {
            Ok(_) => panic!("unary operation unexpectedly accepted by call_stream"),
            Err(error) => error,
        };
        assert_eq!(
            coded_error::facts_anyhow(&unary_error).code,
            "op_registry.call.wrong_shape"
        );

        let stream = streams
            .call_stream(
                "value_stream",
                json!({"value": 3}),
                &FirstCtx { value: 0 },
                CallOptions::default(),
                None,
            )
            .await
            .unwrap();
        let values = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            values,
            vec![json!({"text": "3-one"}), json!({"text": "3-two"})]
        );
    });
}

#[test]
fn coded_error_round_trips_through_json() {
    futures_executor::block_on(async {
        let facts = Facts {
            code: "test.transport.failed",
            class: ErrorClass::Logical,
            overrides: Overrides::NONE,
        };
        let info = ErrorInfo::from_facts(&facts, "registry test");
        let encoded = serde_json::to_string(&info).unwrap();
        let decoded: ErrorInfo = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.code, facts.code);
        assert_eq!(decoded.class, facts.class);

        let registry = registry_for_calls();
        let error = registry
            .call(
                "does_not_exist",
                json!({}),
                &FirstCtx { value: 0 },
                CallOptions::default(),
            )
            .await
            .unwrap_err();
        let error_facts = coded_error::facts_anyhow(&error);
        let error_info = ErrorInfo::from_facts(&error_facts, "registry call");
        let error_json = serde_json::to_string(&error_info).unwrap();
        let error_back: ErrorInfo = serde_json::from_str(&error_json).unwrap();
        assert_eq!(error_back.code, "op_registry.call.unknown_operation");
        assert_eq!(error_back.class, ErrorClass::Logical);
    });
}

async fn slow_value(ctx: &FirstCtx, _: Empty) -> anyhow::Result<Output> {
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    Ok(Output { result: ctx.value })
}

#[tokio::test(flavor = "current_thread")]
async fn call_does_not_block_a_current_thread_runtime() {
    let registry = registry! {
        Ctx = FirstCtx;
        entries: [{
            noun: "slow", verb: "get", description: "waits on a tokio timer",
            access: reads, audience: Agent, params: Empty, output: Output,
            op: slow_value,
        }]
    };
    let ctx = FirstCtx { value: 9 };
    let value = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        registry.call("slow_get", json!({}), &ctx, CallOptions::default()),
    )
    .await
    .expect("call must yield to the runtime instead of blocking it")
    .unwrap();
    assert_eq!(value, json!({"result": 9}));
}

#[test]
fn output_schema_is_derived_from_the_output_type() {
    let registry = mixed_registry();
    let unary = registry.get("value_get").unwrap();
    assert_eq!(
        unary.output_schema,
        serde_json::to_value(schemars::schema_for!(Output)).unwrap()
    );
    assert_eq!(
        unary.output_schema["properties"]["result"]["type"],
        "integer"
    );
    let streamed = registry.get("value_stream").unwrap();
    assert_eq!(
        streamed.output_schema,
        serde_json::to_value(schemars::schema_for!(TextOutput)).unwrap()
    );
    assert_eq!(
        streamed.output_schema["properties"]["text"]["type"],
        "string"
    );
}

#[test]
fn explicit_output_schema_overrides_the_derived_one() {
    let registry = registry! {
        Ctx = FirstCtx;
        entries: [{
            noun: "value", verb: "get", description: "override",
            access: reads, audience: Agent, params: Empty, output: Output,
            op: get_action,
            output_schema: json!({"type": "string"}),
        }]
    };
    assert_eq!(
        registry.get("value_get").unwrap().output_schema,
        json!({"type": "string"})
    );
}

#[test]
#[should_panic(expected = "duplicate operation name")]
fn duplicate_operation_name_is_refused_when_the_registry_is_built() {
    let _ = registry! {
        Ctx = FirstCtx;
        entries: [
            {
                noun: "value", verb: "get", description: "first",
                access: reads, audience: Agent, params: Empty, output: Output,
                op: get_action,
            },
            {
                name: "value_get", noun: "other", verb: "thing", description: "same name",
                access: reads, audience: Agent, params: Empty, output: Output,
                op: explicit_action,
            },
        ]
    };
}

#[test]
#[should_panic(expected = "duplicate operation name")]
fn duplicate_name_between_unary_and_stream_is_refused() {
    let _ = registry! {
        Ctx = FirstCtx;
        entries: [
            {
                noun: "value", verb: "stream", description: "unary",
                access: reads, audience: Agent, params: Empty, output: Output,
                op: get_action,
            },
            {
                noun: "value", verb: "stream", description: "stream",
                access: reads, audience: Agent, params: Input, output: TextOutput,
                stream_op: stream_values,
            },
        ]
    };
}

#[test]
fn library_crate_can_contribute_entries_without_the_macro() {
    futures_executor::block_on(async {
        let mut contributed = op_registry::Registry::<FirstCtx>::new();
        contributed.push_unary::<Input, Output, _>(
            op_registry::Descriptor::new("lib", "double")
                .description("doubles")
                .audience(op_registry::Audience::Agent)
                .input_schema(Input::schema()),
            |_ctx, params, _opts| {
                Box::pin(async move {
                    Ok(Output {
                        result: params.value * 2,
                    })
                })
            },
            op_registry::CliHooks::for_params::<Input>(),
        );
        let mut registry = registry_for_calls();
        registry.merge(contributed);
        assert_eq!(registry.names(), vec!["failing", "value_get", "lib_double"]);
        let out = registry
            .call(
                "lib_double",
                json!({"value": 21}),
                &FirstCtx { value: 0 },
                CallOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(out["result"], 42);
        let hooks = registry
            .cli_hooks("lib_double")
            .expect("hooks for lib_double");
        let cmd = (hooks.add_args)(clap::Command::new("double"));
        let matches = cmd
            .try_get_matches_from(["double", "--value", "5"])
            .unwrap();
        assert_eq!((hooks.parse)(&matches).unwrap(), json!({"value": 5}));
        assert!(registry.cli_hooks("value_get").is_some());
        assert!(registry.cli_hooks("missing").is_none());
    });
}

#[test]
#[should_panic(expected = "duplicate operation name")]
fn merge_refuses_a_duplicate_name() {
    let mut first = registry_for_calls();
    let second = registry_for_calls();
    first.merge(second);
}

fn bare(descriptor: op_registry::Descriptor) -> op_registry::Registry<FirstCtx> {
    let mut registry = op_registry::Registry::<FirstCtx>::new();
    push_bare(&mut registry, descriptor);
    registry
}

fn push_bare(registry: &mut op_registry::Registry<FirstCtx>, descriptor: op_registry::Descriptor) {
    registry.push_unary::<Input, Output, _>(
        descriptor,
        |_ctx, params, _opts| {
            Box::pin(async move {
                Ok(Output {
                    result: params.value,
                })
            })
        },
        op_registry::CliHooks::for_params::<Input>(),
    );
}

fn panic_message(f: impl FnOnce()) -> String {
    let payload =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).expect_err("must panic");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

#[test]
#[should_panic(expected = "both define the command")]
fn push_rejects_an_equal_cli_command_at_construction() {
    let mut registry = bare(op_registry::Descriptor::new("widget", "list"));
    push_bare(
        &mut registry,
        op_registry::Descriptor::new("x", "y").name("widget list"),
    );
}

#[test]
#[should_panic(expected = "a word cannot be both a leaf and a group")]
fn push_rejects_leaf_and_group_at_construction() {
    let mut registry = bare(op_registry::Descriptor::new("widget", "list"));
    push_bare(
        &mut registry,
        op_registry::Descriptor::new("widget list", "show"),
    );
}

#[test]
fn merge_rejects_the_same_collisions() {
    let equal = panic_message(|| {
        let mut first = bare(op_registry::Descriptor::new("widget", "list"));
        first.merge(bare(
            op_registry::Descriptor::new("x", "y").name("widget list"),
        ));
    });
    assert!(equal.contains("both define the command"), "{equal}");
    let group = panic_message(|| {
        let mut first = bare(op_registry::Descriptor::new("widget", "list"));
        first.merge(bare(op_registry::Descriptor::new("widget list", "show")));
    });
    assert!(
        group.contains("a word cannot be both a leaf and a group"),
        "{group}"
    );
    let reverse = panic_message(|| {
        let mut first = bare(op_registry::Descriptor::new("widget list", "show"));
        first.merge(bare(op_registry::Descriptor::new("widget", "list")));
    });
    assert!(
        reverse.contains("a word cannot be both a leaf and a group"),
        "{reverse}"
    );
}

#[test]
fn lookups_after_merge_return_the_right_descriptor_in_declaration_order() {
    let mut first = op_registry::Registry::<FirstCtx>::new();
    let mut second = op_registry::Registry::<FirstCtx>::new();
    for i in 0..1000 {
        let target = if i < 500 { &mut first } else { &mut second };
        push_bare(
            target,
            op_registry::Descriptor::new(format!("noun{i}"), "get"),
        );
    }
    first.merge(second);
    assert_eq!(first.descriptors().len(), 1000);
    for (i, descriptor) in first.descriptors().into_iter().enumerate() {
        assert_eq!(descriptor.name, format!("noun{i}_get"));
    }
    for i in [0, 499, 500, 999] {
        let name = format!("noun{i}_get");
        assert_eq!(first.get(&name).unwrap().noun, format!("noun{i}"));
        assert!(first.cli_hooks(&name).is_some());
    }
    assert!(first.get("noun1000_get").is_none());
    assert!(first.cli_hooks("noun1000_get").is_none());
}

#[test]
fn find_cli_matches_cli_name() {
    let mut registry = bare(op_registry::Descriptor::new("widget", "list"));
    push_bare(
        &mut registry,
        op_registry::Descriptor::new("x", "y").name("ping"),
    );
    assert_eq!(
        registry.find_cli("widget list").unwrap().name,
        "widget_list"
    );
    assert_eq!(registry.find_cli("ping").unwrap().name, "ping");
    assert!(registry.find_cli("widget_list").is_none());
    assert!(registry.find_cli("x y").is_none());
}
