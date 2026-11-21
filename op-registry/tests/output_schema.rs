#![cfg(feature = "schema")]

use op_registry::OutputSchema;
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Serialize, JsonSchema)]
struct Echo {
    id: String,
}

#[test]
fn output_schema_is_some_with_schema_feature() {
    let schema = <Echo as OutputSchema>::output_schema().unwrap();
    assert_eq!(schema["properties"]["id"]["type"], "string");
}

mod common;

use common::{Ctx, Echo as Args};
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use op_registry::{CallOptions, OpSpec, Registry};
use serde_json::{json, Value};

#[derive(Clone, Serialize, JsonSchema)]
struct Named {
    name: String,
}

#[derive(Serialize, JsonSchema)]
struct Node {
    label: String,
    children: Vec<Node>,
}

#[derive(Serialize, JsonSchema)]
struct Tree(Vec<Tree>);

/// A draft-07 style schema keyed `definitions`, written by hand.
#[derive(Serialize)]
struct Legacy(Vec<u8>);

impl OutputSchema for Legacy {
    fn output_schema() -> Option<Value> {
        Some(json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "title": "Legacy",
            "type": "array",
            "items": {"$ref": "#/definitions/Row"},
            "definitions": {
                "Row": {"type": "object", "properties": {"next": {"$ref": "#/definitions/Row"}}}
            }
        }))
    }
}

/// The schema `describe` computed for a unary op returning `O`.
fn schema_of<O>(out: fn() -> O) -> Value
where
    O: Serialize + OutputSchema + Send + 'static,
{
    let mut r: Registry<Ctx> = Registry::new();
    r.push_op(
        OpSpec::<Args, O>::new("probe", "Probe."),
        move |_c: &Ctx, _p: Args, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<O>> {
            Box::pin(async move { Ok(out()) })
        },
    );
    r.descriptors()[0].output_schema.clone().expect("a schema")
}

/// Whether every `$ref` in `schema` is a pointer that resolves from the root.
fn all_refs_resolve(schema: &Value) -> bool {
    fn unescape(token: &str) -> Option<String> {
        let mut bytes = Vec::new();
        let raw = token.as_bytes();
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == b'%' {
                let hex = std::str::from_utf8(raw.get(i + 1..i + 3)?).ok()?;
                bytes.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            } else {
                bytes.push(raw[i]);
                i += 1;
            }
        }
        Some(
            String::from_utf8(bytes)
                .ok()?
                .replace("~1", "/")
                .replace("~0", "~"),
        )
    }
    fn resolves(root: &Value, reference: &str) -> bool {
        let Some(pointer) = reference.strip_prefix('#') else {
            return false;
        };
        let mut at = root;
        if pointer.is_empty() {
            return true;
        }
        let Some(rest) = pointer.strip_prefix('/') else {
            return false;
        };
        for token in rest.split('/') {
            let Some(key) = unescape(token) else {
                return false;
            };
            at = match at {
                Value::Object(map) => match map.get(&key) {
                    Some(next) => next,
                    None => return false,
                },
                Value::Array(list) => match key.parse::<usize>().ok().and_then(|i| list.get(i)) {
                    Some(next) => next,
                    None => return false,
                },
                _ => return false,
            };
        }
        true
    }
    fn walk(root: &Value, at: &Value, seen: &mut usize) -> bool {
        match at {
            Value::Object(map) => map.iter().all(|(key, value)| {
                if key == "$ref" {
                    *seen += 1;
                    return value.as_str().is_some_and(|r| resolves(root, r));
                }
                walk(root, value, seen)
            }),
            Value::Array(list) => list.iter().all(|v| walk(root, v, seen)),
            _ => true,
        }
    }
    let mut seen = 0;
    walk(schema, schema, &mut seen)
}

fn refs_in(schema: &Value) -> usize {
    match schema {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| if k == "$ref" { 1 } else { refs_in(v) })
            .sum(),
        Value::Array(list) => list.iter().map(refs_in).sum(),
        _ => 0,
    }
}

#[test]
fn all_refs_resolve_rejects_dangling_and_handles_escapes() {
    assert!(!all_refs_resolve(
        &json!({"a": {"$ref": "#/$defs/Missing"}})
    ));
    assert!(all_refs_resolve(
        &json!({"$defs": {"T": {}}, "a": {"$ref": "#/$defs/T"}})
    ));
    assert!(all_refs_resolve(
        &json!({"$defs": {"a/b": {}}, "a": {"$ref": "#/$defs/a~1b"}})
    ));
    assert!(all_refs_resolve(
        &json!({"$defs": {"a~b": {}}, "a": {"$ref": "#/$defs/a~0b"}})
    ));
    assert!(all_refs_resolve(
        &json!({"$defs": {"a b": {}}, "a": {"$ref": "#/$defs/a%20b"}})
    ));
    assert!(!all_refs_resolve(
        &json!({"a": {"$ref": "#/$defs/a~1b"}, "$defs": {"a~1b": {}}})
    ));
    assert!(!all_refs_resolve(&json!({"a": {"$ref": "other.json"}})));
}

/// The wrapped schema's keys, in the order `wrap_array_schema` writes them.
fn assert_wrapped_key_order(schema: &Value, defs_key: &str) {
    let keys: Vec<&str> = schema
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let want = [
        "$schema",
        "title",
        "type",
        "properties",
        "required",
        "additionalProperties",
        defs_key,
    ];
    assert_eq!(keys, want, "{schema}");
}

#[test]
fn vec_output_schema_wrapped_with_root_defs() {
    let schema = schema_of(|| vec![Named { name: "a".into() }]);
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["items"]));
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(schema["properties"]["items"]["type"], "array");
    assert!(schema["$defs"]["Named"].is_object(), "{schema}");
    assert!(schema["properties"]["items"].get("$defs").is_none());
    assert!(schema["properties"]["items"].get("$schema").is_none());
    assert!(refs_in(&schema) > 0, "{schema}");
    assert!(all_refs_resolve(&schema), "{schema}");
    assert_wrapped_key_order(&schema, "$defs");
}

#[test]
fn recursive_item_refs_resolve() {
    let schema = schema_of(|| {
        vec![Node {
            label: "x".into(),
            children: vec![],
        }]
    });
    assert_eq!(schema["type"], "object");
    assert!(refs_in(&schema) >= 2, "{schema}");
    assert!(all_refs_resolve(&schema), "{schema}");
}

#[test]
fn nested_vec_refs_resolve() {
    let schema = schema_of(|| vec![vec![Named { name: "a".into() }]]);
    assert_eq!(schema["properties"]["items"]["items"]["type"], "array");
    assert!(refs_in(&schema) > 0, "{schema}");
    assert!(all_refs_resolve(&schema), "{schema}");
}

#[test]
fn definitions_keyed_schema_hoisted() {
    let schema = schema_of(|| Legacy(vec![]));
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["items"]));
    assert!(schema["definitions"]["Row"].is_object(), "{schema}");
    assert!(schema["properties"]["items"].get("definitions").is_none());
    assert_eq!(
        schema["properties"]["items"]["items"]["$ref"],
        "#/definitions/Row"
    );
    assert!(all_refs_resolve(&schema), "{schema}");
    assert_wrapped_key_order(&schema, "definitions");
}

#[test]
fn root_self_ref_rewritten() {
    let schema = schema_of(|| Tree(vec![]));
    assert_eq!(schema["type"], "object");
    assert!(refs_in(&schema) > 0, "{schema}");
    assert!(all_refs_resolve(&schema), "{schema}");
    let target = schema
        .pointer("/properties/items/items/$ref")
        .and_then(Value::as_str)
        .expect("a self reference");
    assert_eq!(
        schema.pointer(&target[1..]).unwrap()["type"],
        "array",
        "{schema}"
    );
}

#[test]
fn optional_vec_root_not_wrapped() {
    let schema = schema_of(|| Some(vec![Named { name: "a".into() }]));
    assert_eq!(schema["type"], json!(["array", "null"]), "{schema}");
    assert!(schema.get("properties").is_none());
    assert!(all_refs_resolve(&schema), "{schema}");
}

#[test]
fn stream_of_vec_items_not_wrapped() {
    let mut r: Registry<Ctx> = Registry::new();
    r.push_stream(
        OpSpec::<Args, Vec<Named>>::new("flow", "Flow."),
        |_c: &Ctx, _p: Args, _cur, _o: CallOptions| {
            Box::pin(async {
                Ok(Box::pin(futures_util::stream::empty())
                    as BoxStream<'_, anyhow::Result<Vec<Named>>>)
            })
        },
    );
    let schema = r.descriptors()[0].output_schema.clone().unwrap();
    assert_eq!(schema["type"], "array", "{schema}");
    assert!(all_refs_resolve(&schema), "{schema}");
}
