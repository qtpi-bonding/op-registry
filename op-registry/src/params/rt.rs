//! Runtime helpers the `OpParams` derive calls. Not public API.

use std::collections::BTreeMap;

use clap::ValueEnum;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::{usage, Patch, UsageError};
use crate::FieldMeta;

type Result<T> = anyhow::Result<T>;

fn invalid(name: &str, reason: impl ToString) -> anyhow::Error {
    usage(UsageError::InvalidField {
        name: name.to_owned(),
        reason: reason.to_string(),
    })
}

/// The top-level JSON object, with every key checked against `known`.
pub fn object(value: Value, known: &[String]) -> Result<Map<String, Value>> {
    let Value::Object(object) = value else {
        return Err(invalid("params", "expected object"));
    };
    if let Some(key) = object.keys().find(|k| !known.iter().any(|n| n == *k)) {
        return Err(usage(UsageError::UnknownKey {
            param: "params".to_owned(),
            key: key.clone(),
        }));
    }
    Ok(object)
}

/// An optional key. Absent and `null` are both `None`.
pub fn get<T: DeserializeOwned>(object: &Map<String, Value>, key: &str) -> Result<Option<T>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| invalid(key, e)),
    }
}

pub fn missing(key: &str) -> anyhow::Error {
    usage(UsageError::MissingField {
        name: key.to_owned(),
    })
}

pub fn require<T>(value: Option<T>, key: &str) -> Result<T> {
    value.ok_or_else(|| missing(key))
}

/// A declared default, converted to the field type through its JSON form.
pub fn default_of<T: DeserializeOwned>(rust_name: &str, value: impl serde::Serialize) -> T {
    let json = serde_json::to_value(value)
        .unwrap_or_else(|e| panic!("op-registry: default of `{rust_name}` is not serialisable: {e}"));
    serde_json::from_value(json)
        .unwrap_or_else(|e| panic!("op-registry: default of `{rust_name}` has the wrong type: {e}"))
}

pub fn enum_value<T: ValueEnum>(key: &str, name: &str) -> Result<T> {
    T::from_str(name, false).map_err(|_| invalid(key, format!("`{name}` is not a valid value")))
}

pub fn enum_name<T: ValueEnum>(value: &T) -> String {
    value
        .to_possible_value()
        .map(|v| v.get_name().to_owned())
        .unwrap_or_default()
}

pub fn enum_names<T: ValueEnum>() -> Vec<String> {
    T::value_variants()
        .iter()
        .filter_map(|v| v.to_possible_value())
        .map(|v| v.get_name().to_owned())
        .collect()
}

fn check_key(param: &str, key: &str, keys: Option<&[&str]>) -> Result<()> {
    match keys {
        Some(keys) if !keys.contains(&key) => Err(usage(UsageError::UnknownKey {
            param: param.to_owned(),
            key: key.to_owned(),
        })),
        _ => Ok(()),
    }
}

/// A map from repeated `k=v` flag values.
pub fn map_from_pairs<'a>(
    flag: &str,
    pairs: impl Iterator<Item = &'a String>,
    keys: Option<&[&str]>,
) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for pair in pairs {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| invalid(flag, "expected key=value"))?;
        check_key(flag, k, keys)?;
        if map.insert(k.to_owned(), v.to_owned()).is_some() {
            return Err(usage(UsageError::RepeatedKey {
                param: flag.to_owned(),
                key: k.to_owned(),
            }));
        }
    }
    Ok(map)
}

/// A map from a JSON object of strings.
pub fn map_from_json(
    object: &Map<String, Value>,
    key: &str,
    keys: Option<&[&str]>,
) -> Result<BTreeMap<String, String>> {
    let map: BTreeMap<String, String> = get(object, key)?.unwrap_or_default();
    for k in map.keys() {
        check_key(key, k, keys)?;
    }
    Ok(map)
}

/// The part of a JSON object a flattened group owns, as its own object.
pub fn sub_object(object: &Map<String, Value>, fields: &[FieldMeta]) -> Value {
    Value::Object(
        object
            .iter()
            .filter(|(k, _)| fields.iter().any(|f| f.key == **k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    )
}

/// A clearable key: absent is `Keep`, `null` is `Clear`.
pub fn patch<T: DeserializeOwned>(object: &Map<String, Value>, key: &str) -> Result<Patch<T>> {
    match object.get(key) {
        None => Ok(Patch::Keep),
        Some(Value::Null) => Ok(Patch::Clear),
        Some(v) => serde_json::from_value(v.clone())
            .map(Patch::Set)
            .map_err(|e| invalid(key, e)),
    }
}

/// A clearable enum key, decoded through its value names.
pub fn patch_enum<T: ValueEnum>(object: &Map<String, Value>, key: &str) -> Result<Patch<T>> {
    Ok(match patch::<String>(object, key)? {
        Patch::Keep => Patch::Keep,
        Patch::Clear => Patch::Clear,
        Patch::Set(name) => Patch::Set(enum_value(key, &name)?),
    })
}

/// A property schema that also admits `null`.
pub fn nullable(mut schema: Value) -> Value {
    if let Some(object) = schema.as_object_mut() {
        if let Some(Value::String(ty)) = object.get("type") {
            let ty = ty.clone();
            object.insert("type".to_owned(), serde_json::json!([ty, "null"]));
        }
        if let Some(Value::Array(values)) = object.get_mut("enum") {
            values.push(Value::Null);
        }
    }
    schema
}

/// Collects an input schema from fields, relations and flattened groups.
#[derive(Default)]
pub struct SchemaBuilder {
    properties: Map<String, Value>,
    required: Vec<String>,
    /// Each entry is an object holding one `oneOf`.
    constraints: Vec<Value>,
    dependent: Map<String, Value>,
}

impl SchemaBuilder {
    pub fn property(&mut self, key: &str, schema: Value) {
        self.properties.insert(key.to_owned(), schema);
    }

    pub fn require(&mut self, key: &str) {
        self.required.push(key.to_owned());
    }

    /// Exactly one (or, when `optional`, at most one) of `keys` is given.
    pub fn one_of(&mut self, keys: &[&str], optional: bool) {
        let mut alternatives: Vec<Value> = keys
            .iter()
            .map(|k| serde_json::json!({ "required": [k] }))
            .collect();
        if optional {
            alternatives.push(serde_json::json!({
                "not": { "anyOf": alternatives.clone() }
            }));
        }
        self.constraints
            .push(serde_json::json!({ "oneOf": alternatives }));
    }

    /// `key` is required whenever `other` is present.
    pub fn depends(&mut self, other: &str, key: &str) {
        let entry = self
            .dependent
            .entry(other.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(keys) = entry {
            keys.push(Value::String(key.to_owned()));
        }
    }

    /// Merges the schema of a flattened group.
    pub fn absorb(&mut self, schema: Value) {
        let Value::Object(mut schema) = schema else {
            return;
        };
        if let Some(Value::Object(properties)) = schema.remove("properties") {
            self.properties.extend(properties);
        }
        if let Some(Value::Array(required)) = schema.remove("required") {
            self.required
                .extend(required.into_iter().filter_map(|v| v.as_str().map(str::to_owned)));
        }
        if let Some(one_of) = schema.remove("oneOf") {
            self.constraints.push(serde_json::json!({ "oneOf": one_of }));
        }
        if let Some(Value::Array(all)) = schema.remove("allOf") {
            self.constraints.extend(all);
        }
        if let Some(Value::Object(dependent)) = schema.remove("dependentRequired") {
            for (other, keys) in dependent {
                if let Value::Array(keys) = keys {
                    for key in keys.iter().filter_map(Value::as_str) {
                        self.depends(&other, key);
                    }
                }
            }
        }
    }

    pub fn build(self) -> Value {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": self.properties,
            "additionalProperties": false,
        });
        if !self.required.is_empty() {
            schema["required"] = serde_json::json!(self.required);
        }
        if !self.dependent.is_empty() {
            schema["dependentRequired"] = Value::Object(self.dependent);
        }
        let mut constraints = self.constraints;
        match constraints.len() {
            0 => {}
            1 => schema["oneOf"] = constraints.remove(0)["oneOf"].take(),
            _ => schema["allOf"] = Value::Array(constraints),
        }
        schema
    }
}

/// The relation errors the derived `validate()` returns.
pub fn check_one_of(group: &str, given: usize, optional: bool) -> Result<()> {
    if given > 1 || (given == 0 && !optional) {
        return Err(usage(UsageError::OneOf {
            group: group.to_owned(),
        }));
    }
    Ok(())
}

pub fn check_requires_if(present: bool, other_present: bool, field: &str, other: &str) -> Result<()> {
    if other_present && !present {
        return Err(usage(UsageError::RequiresIf {
            field: field.to_owned(),
            other: other.to_owned(),
        }));
    }
    Ok(())
}

/// Two fields of one op that share a flag or key.
pub fn find_clash(fields: &[FieldMeta]) -> Option<(&FieldMeta, &FieldMeta)> {
    for (i, a) in fields.iter().enumerate() {
        for b in &fields[i + 1..] {
            if a.flag == b.flag || a.key == b.key {
                return Some((a, b));
            }
        }
    }
    None
}
