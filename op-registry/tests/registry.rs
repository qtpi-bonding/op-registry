use coded_error::{ErrorClass, ErrorInfo, Facts, Overrides};
use futures_util::{stream, StreamExt};
use op_registry::{registry, CallOptions, Cursor, OpParams};
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Debug, Deserialize, OpParams)]
#[allow(dead_code)]
struct Empty {
    value: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, OpParams)]
struct Input {
    value: i32,
}

#[derive(Clone, Debug, serde::Serialize)]
struct Output {
    result: i32,
}

#[derive(Clone, Debug, serde::Serialize)]
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
        .unwrap();
    let second_value = second
        .call(
            "same_thing",
            json!({}),
            &SecondCtx { value: 22 },
            CallOptions::default(),
        )
        .unwrap();
    assert_eq!(first_value["result"], 11);
    assert_eq!(second_value["result"], 22);
}

#[test]
fn mixed_registry_preserves_order_shapes_and_calls() {
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
        .unwrap();
    let values = futures_executor::block_on(stream.collect::<Vec<_>>())
        .into_iter()
        .collect::<anyhow::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(values.len(), 2);
}

// The macro's operation type is intentionally local to the expression, so it
// cannot be named from this integration test. The registry descriptor above
// is built by the same local helper used by that operation's implementation.

#[test]
fn call_returns_coded_error_chain_intact() {
    let registry = registry_for_calls();
    let streams = stream_registry();
    let error = registry
        .call(
            "failing",
            json!({"value": 1}),
            &FirstCtx { value: 0 },
            CallOptions::default(),
        )
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
        .unwrap();
    assert_eq!(output, json!({"result": 7}));

    let stream_error = match streams.call(
        "value_stream",
        json!({"value": 3}),
        &FirstCtx { value: 0 },
        CallOptions::default(),
    ) {
        Ok(_) => panic!("stream operation unexpectedly accepted by call"),
        Err(error) => error,
    };
    assert_eq!(
        coded_error::facts_anyhow(&stream_error).code,
        "op_registry.call.wrong_shape"
    );
    let unary_error = match registry.call_stream(
        "value_get",
        json!({}),
        &FirstCtx { value: 0 },
        CallOptions::default(),
        None,
    ) {
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
        .unwrap();
    let values = futures_executor::block_on(stream.collect::<Vec<_>>())
        .into_iter()
        .collect::<anyhow::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        values,
        vec![json!({"text": "3-one"}), json!({"text": "3-two"})]
    );
}

#[test]
fn coded_error_round_trips_through_json() {
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
        .unwrap_err();
    let error_facts = coded_error::facts_anyhow(&error);
    let error_info = ErrorInfo::from_facts(&error_facts, "registry call");
    let error_json = serde_json::to_string(&error_info).unwrap();
    let error_back: ErrorInfo = serde_json::from_str(&error_json).unwrap();
    assert_eq!(error_back.code, "op_registry.call.unknown_operation");
    assert_eq!(error_back.class, ErrorClass::Logical);
}
