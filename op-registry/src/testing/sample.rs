//! Minimal valid JSON for a derived input schema.

use serde_json::{json, Map, Value};

/// Builds the smallest JSON value that satisfies `schema`: every required
/// property is filled (string `"x"` or its `examples[0]`, integer 1, number
/// 1.0, boolean false, the first enum value, `[]`, `{}`), one member of each
/// `oneOf`/`anyOf` is taken with its fragment satisfied (`const: true`,
/// `minItems`, `minProperties`, a non-null value), and `if`/`then` rules whose
/// `if` holds add their `then`.
pub fn sample_params(schema: &Value) -> Value {
    sample_params_seeded(schema, Map::new())
}

/// Like [`sample_params`], with the `seed` keys placed first and kept: the
/// rest is sampled around them, so a rule keyed on a seeded key is honoured.
pub fn sample_params_seeded(schema: &Value, seed: Map<String, Value>) -> Value {
    match schema.as_object() {
        Some(object) if !seed.is_empty() => sample_object(object, schema, 0, seed),
        _ => sample(schema, schema, 0),
    }
}

fn sample(schema: &Value, root: &Value, depth: usize) -> Value {
    let Some(object) = schema.as_object() else {
        return json!({});
    };
    if depth > 16 {
        return Value::Null;
    }
    if let Some(Value::String(reference)) = object.get("$ref") {
        let target = reference
            .strip_prefix('#')
            .and_then(|pointer| root.pointer(pointer));
        return target.map_or(Value::Null, |t| sample(t, root, depth + 1));
    }
    if let Some(first) = object
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
    {
        return first.clone();
    }
    if let Some(value) = object.get("const") {
        return value.clone();
    }
    if let Some(example) = object
        .get("examples")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
    {
        return example.clone();
    }
    match type_of(object) {
        Some("string") => {
            if object.get("contentEncoding").and_then(Value::as_str) == Some("base64") {
                json!("eA==")
            } else {
                json!("x")
            }
        }
        Some("integer") => json!(object
            .get("minimum")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .max(1)),
        Some("number") => json!(1.0),
        Some("boolean") => json!(false),
        Some("array") => {
            let count = object.get("minItems").and_then(Value::as_u64).unwrap_or(0);
            sample_items(object.get("items"), count, root, depth)
        }
        Some("object") => sample_object(object, root, depth, Map::new()),
        _ => {
            if object.contains_key("properties") {
                return sample_object(object, root, depth, Map::new());
            }
            // A property-level choice: take the first non-null branch.
            for key in ["oneOf", "anyOf"] {
                if let Some(branch) = object
                    .get(key)
                    .and_then(Value::as_array)
                    .and_then(|b| b.iter().find(|b| b.get("type") != Some(&json!("null"))))
                {
                    return sample(branch, root, depth + 1);
                }
            }
            json!({})
        }
    }
}

/// The schema's type, the first non-null one when it is a list.
fn type_of(object: &Map<String, Value>) -> Option<&str> {
    match object.get("type")? {
        Value::String(ty) => Some(ty),
        Value::Array(types) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null"),
        _ => None,
    }
}

/// `count` items sampled from the `items` schema.
fn sample_items(items: Option<&Value>, count: u64, root: &Value, depth: usize) -> Value {
    let item = items.map_or_else(|| json!({}), |i| sample(i, root, depth + 1));
    Value::Array((0..count).map(|_| item.clone()).collect())
}

/// Every object that holds constraints of `object`: itself and, recursively,
/// the entries of its `allOf`.
fn constraint_objects<'a>(object: &'a Map<String, Value>, out: &mut Vec<&'a Map<String, Value>>) {
    out.push(object);
    for entry in object
        .get("allOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(entry) = entry.as_object() {
            constraint_objects(entry, out);
        }
    }
}

/// Whether `value` meets the property constraints of a presence fragment.
fn property_holds(constraint: &Value, value: &Value) -> bool {
    if let Some(expected) = constraint.get("const") {
        return value == expected;
    }
    if let Some(n) = constraint.get("minItems").and_then(Value::as_u64) {
        return value.as_array().is_some_and(|a| a.len() as u64 >= n);
    }
    if let Some(n) = constraint.get("minProperties").and_then(Value::as_u64) {
        return value.as_object().is_some_and(|o| o.len() as u64 >= n);
    }
    if constraint.get("not").is_some() {
        return !value.is_null();
    }
    true
}

/// Whether `out` satisfies a presence fragment: its required keys are there
/// with values its property constraints accept, or (an `anyOf` of fragments)
/// any of them is. A fragment with neither is not a presence fragment.
fn satisfied(out: &Map<String, Value>, fragment: &Value) -> bool {
    if let Some(any) = fragment.get("anyOf").and_then(Value::as_array) {
        return any.iter().any(|f| satisfied(out, f));
    }
    let Some(required) = fragment.get("required").and_then(Value::as_array) else {
        return false;
    };
    let properties = fragment.get("properties").and_then(Value::as_object);
    required.iter().filter_map(Value::as_str).all(|key| {
        out.get(key).is_some_and(|value| {
            properties
                .and_then(|p| p.get(key))
                .map_or(!value.is_null(), |c| property_holds(c, value))
        })
    })
}

/// Makes `out` satisfy a presence fragment, sampling each key it lacks from
/// the property schema in `properties` and bending the value to the
/// fragment's property constraint.
fn apply(
    out: &mut Map<String, Value>,
    fragment: &Value,
    properties: Option<&Map<String, Value>>,
    root: &Value,
    depth: usize,
) {
    let constraints = fragment.get("properties").and_then(Value::as_object);
    for key in string_list(fragment.get("required")) {
        let constraint = constraints.and_then(|c| c.get(&key));
        if out
            .get(&key)
            .is_some_and(|v| constraint.map_or(!v.is_null(), |c| property_holds(c, v)))
        {
            continue;
        }
        let schema = properties.and_then(|p| p.get(&key)).unwrap_or(&Value::Null);
        let mut value = sample(schema, root, depth + 1);
        if let Some(c) = constraint {
            if let Some(expected) = c.get("const") {
                value = expected.clone();
            } else if let Some(n) = c.get("minItems").and_then(Value::as_u64) {
                if !value.as_array().is_some_and(|a| a.len() as u64 >= n) {
                    value = sample_items(schema.get("items"), n, root, depth);
                }
            } else if let Some(n) = c.get("minProperties").and_then(Value::as_u64) {
                if !value.as_object().is_some_and(|o| o.len() as u64 >= n) {
                    value = Value::Object(
                        (0..n)
                            .map(|i| {
                                let name = if i == 0 {
                                    "k".to_owned()
                                } else {
                                    format!("k{i}")
                                };
                                (name, json!("x"))
                            })
                            .collect(),
                    );
                }
            }
        }
        out.insert(key, value);
    }
}

fn sample_object(
    object: &Map<String, Value>,
    root: &Value,
    depth: usize,
    seed: Map<String, Value>,
) -> Value {
    let properties = object.get("properties").and_then(Value::as_object);
    let mut out = seed;
    for key in string_list(object.get("required")) {
        if !out.contains_key(&key) {
            let schema = properties.and_then(|p| p.get(&key)).unwrap_or(&Value::Null);
            out.insert(key, sample(schema, root, depth + 1));
        }
    }
    let mut holders = Vec::new();
    constraint_objects(object, &mut holders);
    // `not: {allOf: [..]}`: sets of keys that must not all be present.
    let forbidden: Vec<&Vec<Value>> = holders
        .iter()
        .filter_map(|h| h.get("not")?.get("allOf")?.as_array())
        .collect();
    let completes = |out: &Map<String, Value>| {
        forbidden
            .iter()
            .any(|all| all.iter().all(|f| satisfied(out, f)))
    };
    // One member of each oneOf/anyOf that is not already met.
    for holder in &holders {
        for key in ["oneOf", "anyOf"] {
            let Some(branches) = holder.get(key).and_then(Value::as_array) else {
                continue;
            };
            let members: Vec<&Value> = branches
                .iter()
                .filter(|b| b.get("required").is_some())
                .collect();
            if members.is_empty() || members.iter().any(|b| satisfied(&out, b)) {
                continue;
            }
            let mut chosen = None;
            for member in &members {
                let mut trial = out.clone();
                apply(&mut trial, member, properties, root, depth);
                if !completes(&trial) {
                    chosen = Some(trial);
                    break;
                }
            }
            out = chosen.unwrap_or_else(|| {
                apply(&mut out, members[0], properties, root, depth);
                out
            });
        }
    }
    // `if`/`then`, repeated because a `then` can meet another `if`.
    for _ in 0..4 {
        let before = out.clone();
        for holder in &holders {
            if let (Some(when), Some(then)) = (holder.get("if"), holder.get("then")) {
                if satisfied(&out, when) {
                    apply(&mut out, then, properties, root, depth);
                }
            }
        }
        if out == before {
            break;
        }
    }
    Value::Object(out)
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}
