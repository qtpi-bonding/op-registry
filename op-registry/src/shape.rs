//! The `{"items": [...]}` wrap of array outputs (value and schema).

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
}
