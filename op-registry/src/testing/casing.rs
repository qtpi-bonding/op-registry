//! `assert_casing`: schema property keys follow one casing.

use super::fail_on;
use crate::Registry;
use serde_json::Value;

/// A casing the keys of every schema must follow.
#[derive(Clone, Copy, Debug)]
pub enum Casing {
    /// `agentId`: a lowercase first letter, then letters and digits only.
    Camel,
}

impl Casing {
    fn accepts(self, key: &str) -> bool {
        match self {
            Casing::Camel => {
                let mut chars = key.chars();
                chars.next().is_some_and(|c| c.is_ascii_lowercase())
                    && chars.all(|c| c.is_ascii_alphanumeric())
            }
        }
    }
}

/// Checks the property keys of every op's input and output schema, at every
/// depth, against `casing`. The keys of a map are data, not schema, so they
/// never appear here; a map's value schema is still checked.
///
/// # Panics
///
/// Panics with every offending key and where it is.
pub fn assert_casing<C: Sync>(registry: &Registry<C>, casing: Casing) {
    let mut problems = Vec::new();
    for descriptor in registry.descriptors() {
        let mut check = |side: &str, schema: &Value| {
            let mut found = Vec::new();
            walk(schema, casing, "", &mut found);
            for at in found {
                problems.push(format!("{} {side} schema: {at}", descriptor.name));
            }
        };
        check("input", &descriptor.input_schema);
        if let Some(output) = &descriptor.output_schema {
            check("output", output);
        }
    }
    fail_on("assert_casing", &problems);
}

fn walk(schema: &Value, casing: Casing, at: &str, found: &mut Vec<String>) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (key, value) in properties {
            let here = format!("{at}/{key}");
            if !casing.accepts(key) {
                found.push(format!("key `{key}` at {here} is not {casing:?} case"));
            }
            walk(value, casing, &here, found);
        }
    }
    for key in ["items", "additionalProperties", "not", "if", "then", "else"] {
        if let Some(value) = object.get(key) {
            walk(value, casing, &format!("{at}/{key}"), found);
        }
    }
    for key in ["oneOf", "anyOf", "allOf", "prefixItems"] {
        for (i, value) in object
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            walk(value, casing, &format!("{at}/{key}/{i}"), found);
        }
    }
    for key in ["$defs", "definitions"] {
        for (name, value) in object
            .get(key)
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            walk(value, casing, &format!("{at}/{key}/{name}"), found);
        }
    }
}
