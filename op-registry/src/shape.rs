//! The `{"items": [...]}` wrap of array outputs (value and schema), and the
//! class of an output schema's root.

use serde_json::{json, Map, Value};

/// The pointer from the wrapped root to the array it holds.
const ITEMS: &str = "#/properties/items";

/// An array becomes `{"items": <array>}`; any other value is unchanged.
pub(crate) fn wrap_items(value: Value) -> Value {
    match value {
        Value::Array(items) => json!({ "items": items }),
        other => other,
    }
}

/// Wraps a schema whose root is `"type": "array"` (a string, not a list)
/// into the object that [`wrap_items`] produces. `$defs` and `definitions`
/// stay at the root, so `#/$defs/T` and `#/definitions/T` still resolve; a
/// reference into the old root (`#`, `#/items`) is pointed at the array's
/// new place. Any other schema is returned unchanged.
pub(crate) fn wrap_array_schema(schema: Value) -> Value {
    let Value::Object(mut root) = schema else {
        return schema;
    };
    if root.get("type").and_then(Value::as_str) != Some("array") {
        return Value::Object(root);
    }
    let schema_uri = root.remove("$schema");
    let title = root.remove("title");
    let defs = root.remove("$defs");
    let definitions = root.remove("definitions");

    let mut wrapped = Map::new();
    wrapped.extend(schema_uri.map(|v| ("$schema".to_owned(), v)));
    wrapped.extend(title.map(|v| ("title".to_owned(), v)));
    wrapped.insert("type".to_owned(), json!("object"));
    wrapped.insert(
        "properties".to_owned(),
        json!({ "items": Value::Object(root) }),
    );
    wrapped.insert("required".to_owned(), json!(["items"]));
    wrapped.insert("additionalProperties".to_owned(), json!(false));
    wrapped.extend(defs.map(|v| ("$defs".to_owned(), v)));
    wrapped.extend(definitions.map(|v| ("definitions".to_owned(), v)));

    let mut wrapped = Value::Object(wrapped);
    repoint_refs(&mut wrapped);
    wrapped
}

/// Rewrites every `"$ref"` that points into the old array root: `#` becomes
/// the array's new place and `#/<key>...` moves under it. References into
/// `$defs` and `definitions` stay as they are.
fn repoint_refs(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                match child {
                    Value::String(reference) if key == "$ref" => {
                        if let Some(moved) = repointed(reference) {
                            *reference = moved;
                        }
                    }
                    other => repoint_refs(other),
                }
            }
        }
        Value::Array(list) => list.iter_mut().for_each(repoint_refs),
        _ => {}
    }
}

fn repointed(reference: &str) -> Option<String> {
    if reference == "#" {
        return Some(ITEMS.to_owned());
    }
    let rest = reference.strip_prefix("#/")?;
    let first = rest.split('/').next().unwrap_or_default();
    if matches!(first, "$defs" | "%24defs" | "definitions") {
        return None;
    }
    Some(format!("{ITEMS}/{rest}"))
}

/// What an output schema's root says about the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootClass {
    /// Every value is a JSON object.
    Object,
    /// The schema says nothing about the type (what a `Value` output gives).
    Any,
    /// Not an object: `"scalar"`, `"nullable"` or `"mixed"`.
    NonObject(&'static str),
}

/// Follows a local `$ref` (`#` or `#/...`) in `root`.
fn lookup<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let pointer = reference.strip_prefix('#')?;
    root.pointer(pointer)
}

/// Classifies the root of an output schema. A root `$ref` is classed by its
/// target; a target that cannot be found, or a chain of references that
/// loops, is `NonObject("mixed")`.
pub(crate) fn root_class(schema: &Value) -> RootClass {
    let mut current = schema;
    let mut seen: Vec<&str> = Vec::new();
    while let Some(reference) = current.get("$ref").and_then(Value::as_str) {
        if seen.contains(&reference) {
            return RootClass::NonObject("mixed");
        }
        seen.push(reference);
        match lookup(schema, reference) {
            Some(target) => current = target,
            None => return RootClass::NonObject("mixed"),
        }
    }
    classify(current, schema)
}

fn classify(node: &Value, root: &Value) -> RootClass {
    if is_object(node, root, &mut Vec::new()) {
        return RootClass::Object;
    }
    match node.get("type") {
        Some(Value::String(kind)) => return RootClass::NonObject(type_class(kind)),
        Some(Value::Array(kinds)) => {
            let names: Vec<&str> = kinds.iter().filter_map(Value::as_str).collect();
            return RootClass::NonObject(if names.contains(&"null") {
                "nullable"
            } else {
                "mixed"
            });
        }
        _ => {}
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(Value::Array(branches)) = node.get(key) {
            let nullable = branches.iter().any(|b| {
                matches!(b.get("type"), Some(Value::String(t)) if t == "null")
                    || matches!(b.get("type"), Some(Value::Array(ts)) if ts.iter().any(|t| t == "null"))
            });
            return RootClass::NonObject(if nullable { "nullable" } else { "mixed" });
        }
    }
    if node.get("allOf").is_some() {
        return RootClass::NonObject("mixed");
    }
    if node.get("enum").is_some() || node.get("const").is_some() {
        return RootClass::NonObject("scalar");
    }
    RootClass::Any
}

fn type_class(kind: &str) -> &'static str {
    if kind == "null" {
        "nullable"
    } else {
        "scalar"
    }
}

/// Whether a schema node describes only objects.
fn is_object<'a>(node: &'a Value, root: &'a Value, seen: &mut Vec<&'a str>) -> bool {
    if node.get("type").and_then(Value::as_str) == Some("object")
        || node.get("properties").is_some()
    {
        return true;
    }
    if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        if seen.contains(&reference) {
            return false;
        }
        seen.push(reference);
        let found = lookup(root, reference).is_some_and(|t| is_object(t, root, seen));
        seen.pop();
        return found;
    }
    if let Some(Value::Array(members)) = node.get("allOf") {
        if members.iter().any(|m| is_object(m, root, seen)) {
            return true;
        }
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(Value::Array(branches)) = node.get(key) {
            if !branches.is_empty() && branches.iter().all(|b| is_object(b, root, seen)) {
                return true;
            }
        }
    }
    false
}

/// A root `oneOf`/`anyOf` whose every branch is an object gets `"type":
/// "object"`. Any other schema is unchanged.
pub(crate) fn mark_object_root(schema: &mut Value) {
    let Value::Object(map) = schema else { return };
    if map.contains_key("type") || !(map.contains_key("oneOf") || map.contains_key("anyOf")) {
        return;
    }
    if is_object(&*schema, &schema.clone(), &mut Vec::new()) {
        if let Value::Object(map) = schema {
            map.insert("type".to_owned(), json!("object"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_only_arrays() {
        assert_eq!(wrap_items(json!([1])), json!({"items": [1]}));
        assert_eq!(wrap_items(json!({"a": 1})), json!({"a": 1}));
        assert_eq!(wrap_items(json!(null)), json!(null));
    }

    #[test]
    fn repoints_root_refs_only() {
        assert_eq!(repointed("#").as_deref(), Some("#/properties/items"));
        assert_eq!(
            repointed("#/items/0").as_deref(),
            Some("#/properties/items/items/0")
        );
        assert_eq!(repointed("#/$defs/T"), None);
        assert_eq!(repointed("#/definitions/T"), None);
        assert_eq!(repointed("other.json#/items"), None);
    }

    #[test]
    fn non_array_roots_are_unchanged() {
        let nullable = json!({"type": ["array", "null"], "items": {}});
        assert_eq!(wrap_array_schema(nullable.clone()), nullable);
        let object = json!({"type": "object"});
        assert_eq!(wrap_array_schema(object.clone()), object);
    }

    #[test]
    fn root_class_names_scalar_nullable_mixed() {
        use RootClass::*;
        assert_eq!(root_class(&json!({"type": "integer"})), NonObject("scalar"));
        assert_eq!(root_class(&json!({"type": "string"})), NonObject("scalar"));
        assert_eq!(
            root_class(&json!({"type": ["integer", "null"]})),
            NonObject("nullable")
        );
        assert_eq!(
            root_class(&json!({"anyOf": [{"$ref": "#/$defs/A"}, {"type": "null"}],
                "$defs": {"A": {"type": "object"}}})),
            NonObject("nullable")
        );
        assert_eq!(
            root_class(&json!({"oneOf": [{"type": "object"}, {"type": "string"}]})),
            NonObject("mixed")
        );
        assert_eq!(
            root_class(&json!({"type": ["string", "integer"]})),
            NonObject("mixed")
        );
        assert_eq!(root_class(&json!({"type": "object"})), Object);
        assert_eq!(root_class(&json!({"title": "Value"})), Any);
        assert_eq!(root_class(&json!(true)), Any);
    }

    #[test]
    fn root_class_follows_refs_safely() {
        use RootClass::*;
        let defs = json!({"$ref": "#/$defs/A", "$defs": {"A": {"properties": {}}}});
        assert_eq!(root_class(&defs), Object);
        assert_eq!(
            root_class(&json!({"$ref": "#/$defs/Gone"})),
            NonObject("mixed")
        );
        assert_eq!(root_class(&json!({"$ref": "#"})), NonObject("mixed"));
        let loop_ab = json!({"$ref": "#/$defs/A", "$defs": {
            "A": {"$ref": "#/$defs/B"}, "B": {"$ref": "#/$defs/A"}}});
        assert_eq!(root_class(&loop_ab), NonObject("mixed"));
        // A recursive object is still an object.
        let tree = json!({"$ref": "#/$defs/N", "$defs": {"N": {"type": "object",
            "properties": {"c": {"$ref": "#/$defs/N"}}}}});
        assert_eq!(root_class(&tree), Object);
    }

    #[test]
    fn mark_object_root_marks_only_all_object_unions() {
        let mut tagged = json!({"oneOf": [{"type": "object"}, {"properties": {}}]});
        mark_object_root(&mut tagged);
        assert_eq!(tagged["type"], "object");
        let mut mixed = json!({"oneOf": [{"type": "object"}, {"type": "string"}]});
        mark_object_root(&mut mixed);
        assert!(mixed.get("type").is_none());
        let mut looped = json!({"anyOf": [{"$ref": "#"}]});
        mark_object_root(&mut looped);
        assert!(looped.get("type").is_none());
    }
}
