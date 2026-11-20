//! Minimal valid JSON for a derived input schema.

use serde_json::{json, Map, Value};

/// Builds the smallest JSON value that satisfies `schema`: every required
/// property is filled (string `"x"`, integer 1, number 1.0, boolean false,
/// the first enum value, `[]`, `{}`) and the first branch of a `oneOf` is taken.
pub fn sample_params(schema: &Value) -> Value {
    sample(schema, schema, 0)
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
        Some("array") => json!([]),
        Some("object") => sample_object(object, root, depth),
        _ => {
            if object.contains_key("properties") {
                return sample_object(object, root, depth);
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

fn sample_object(object: &Map<String, Value>, root: &Value, depth: usize) -> Value {
    let properties = object.get("properties").and_then(Value::as_object);
    let mut keys: Vec<String> = string_list(object.get("required"));
    // Object-level constraints: `oneOf` alternatives name required keys, and
    // `allOf` holds several such constraints.
    let mut constraints: Vec<&Value> = Vec::new();
    if let Some(one_of) = object.get("oneOf") {
        constraints.push(one_of);
    }
    for entry in object
        .get("allOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(one_of) = entry.get("oneOf") {
            constraints.push(one_of);
        }
    }
    for one_of in constraints {
        if let Some(first) = one_of.as_array().and_then(|a| a.first()) {
            keys.extend(string_list(first.get("required")));
        }
    }
    let mut out = Map::new();
    for key in keys {
        let schema = properties.and_then(|p| p.get(&key)).unwrap_or(&Value::Null);
        out.insert(key, sample(schema, root, depth + 1));
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
