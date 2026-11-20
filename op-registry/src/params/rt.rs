//! Runtime helpers the `OpParams` derive calls. Not public API.

use std::collections::BTreeMap;

use clap::ValueEnum;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::{usage, UsageError};

type Result<T> = anyhow::Result<T>;

fn invalid(name: &str, reason: impl ToString) -> anyhow::Error {
    usage(UsageError::InvalidField {
        name: name.to_owned(),
        reason: reason.to_string(),
    })
}

/// The top-level JSON object, with every key checked against `known`.
pub fn object(value: Value, known: &[&str]) -> Result<Map<String, Value>> {
    let Value::Object(object) = value else {
        return Err(invalid("params", "expected object"));
    };
    if let Some(key) = object.keys().find(|k| !known.contains(&k.as_str())) {
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
