//! Runtime helpers the `OpParams` derive calls. Not public API.

use std::collections::BTreeMap;

use clap::ValueEnum;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::{usage, Input, Json, ParseCx, Patch, UsageError};
use crate::{FieldKind, FieldMeta};

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
    let json = serde_json::to_value(value).unwrap_or_else(|e| {
        panic!("op-registry: default of `{rust_name}` is not serialisable: {e}")
    });
    serde_json::from_value(json)
        .unwrap_or_else(|e| panic!("op-registry: default of `{rust_name}` has the wrong type: {e}"))
}

pub fn enum_value<T: ValueEnum>(key: &str, name: &str, ignore_case: bool) -> Result<T> {
    T::from_str(name, ignore_case)
        .map_err(|_| invalid(key, format!("`{name}` is not a valid value")))
}

/// Gives a param its env fallback. clap takes an empty variable as the value
/// `""`, so an empty one is left off the argument (the flag stays absent) and
/// help still names it. `secret` hides the variable from help.
pub fn env_arg(arg: clap::Arg, var: &'static str, secret: bool) -> clap::Arg {
    if std::env::var_os(var).is_some_and(|v| v.is_empty()) {
        if secret {
            return arg;
        }
        let help = match arg.get_help() {
            Some(h) => format!("{h} [env: {var}]"),
            None => format!("[env: {var}]"),
        };
        return arg.help(help);
    }
    if secret {
        arg.env(var).hide_env(true)
    } else {
        arg.env(var).hide_env_values(true)
    }
}

/// A `from_str` value: the text through `T::from_str`. A `secret` field says
/// only `invalid value`, since a parser's message may quote its input.
pub fn from_str_arg<T>(key: &str, text: &str, hide: bool) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    T::from_str(text).map_err(|e| {
        if hide {
            invalid(key, "invalid value")
        } else {
            invalid(key, e)
        }
    })
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
pub fn patch_enum<T: ValueEnum>(
    object: &Map<String, Value>,
    key: &str,
    ignore_case: bool,
) -> Result<Patch<T>> {
    Ok(match patch::<String>(object, key)? {
        Patch::Keep => Patch::Keep,
        Patch::Clear => Patch::Clear,
        Patch::Set(name) => Patch::Set(enum_value(key, &name, ignore_case)?),
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
            if !values.contains(&Value::Null) {
                values.push(Value::Null);
            }
        }
    }
    schema
}

/// The key and kind of the leaf `rust_name` of a flattened group `G`
/// (a relation's dotted target). The derive checks the path itself, so a
/// missing leaf is a bug in the derive.
pub fn leaf_of<G: crate::OpParams>(rust_name: &str) -> (String, FieldKind) {
    G::fields()
        .into_iter()
        .find(|f| f.rust_name == rust_name)
        .map(|f| (f.key, f.kind))
        .unwrap_or_else(|| panic!("op-registry: no field `{rust_name}` in the flattened group"))
}

/// The clap arg ids of a relation target: a tri-state also has `no_x`, a
/// `Patch` also `clear_x`.
pub fn arg_ids(rust_name: &str, kind: FieldKind) -> Vec<String> {
    let mut ids = vec![rust_name.to_owned()];
    match kind {
        FieldKind::TriState => ids.push(format!("no_{rust_name}")),
        FieldKind::Patch => ids.push(format!("clear_{rust_name}")),
        _ => {}
    }
    ids
}

/// Whether a field is given, in the one sense every surface shares: an
/// absent `Option`, an empty `Vec` or map, a `false` switch and a `Patch::Keep`
/// are all "not present".
pub trait Presence {
    fn present(&self) -> bool;
}

impl Presence for bool {
    fn present(&self) -> bool {
        *self
    }
}

impl<T> Presence for Option<T> {
    fn present(&self) -> bool {
        self.is_some()
    }
}

impl<T> Presence for Vec<T> {
    fn present(&self) -> bool {
        !self.is_empty()
    }
}

impl<K, V> Presence for BTreeMap<K, V> {
    fn present(&self) -> bool {
        !self.is_empty()
    }
}

impl<T> Presence for Patch<T> {
    fn present(&self) -> bool {
        !matches!(self, Patch::Keep)
    }
}

/// The schema fragment "`key` is present", agreeing with [`Presence`] for a
/// field of `kind`. A `Vec` or map fragment also pins the `type`: `minItems`
/// alone is vacuously true for the `null` that `from_json` reads as absent.
pub fn present_schema(key: &str, kind: FieldKind) -> Value {
    use serde_json::json;
    match kind {
        FieldKind::Switch => json!({"required": [key], "properties": {key: {"const": true}}}),
        FieldKind::Repeated => {
            json!({"required": [key], "properties": {key: {"type": "array", "minItems": 1}}})
        }
        FieldKind::Map => {
            json!({"required": [key], "properties": {key: {"type": "object", "minProperties": 1}}})
        }
        FieldKind::Patch => json!({"required": [key]}),
        _ => json!({"required": [key], "properties": {key: {"not": {"type": "null"}}}}),
    }
}

/// The fragment "any of `members` is present".
fn any_present(members: &[(&str, FieldKind)]) -> Vec<Value> {
    members
        .iter()
        .map(|(key, kind)| present_schema(key, *kind))
        .collect()
}

/// Collects an input schema from fields, relations and flattened groups.
#[derive(Default)]
pub struct SchemaBuilder {
    properties: Map<String, Value>,
    required: Vec<String>,
    /// Each entry is a whole object: one `oneOf`, `anyOf`, `not` or `if`/`then`.
    constraints: Vec<Value>,
}

impl SchemaBuilder {
    pub fn property(&mut self, key: &str, schema: Value) {
        self.properties.insert(key.to_owned(), schema);
    }

    pub fn require(&mut self, key: &str) {
        self.required.push(key.to_owned());
    }

    /// One whole constraint object (`oneOf`, `anyOf`, `not`, `if`/`then`).
    pub fn constraint(&mut self, c: Value) {
        self.constraints.push(c);
    }

    /// Exactly one (or, when `optional`, at most one) of `members` is present.
    pub fn one_of(&mut self, members: &[(&str, FieldKind)], optional: bool) {
        let mut alternatives = any_present(members);
        if optional {
            let none = serde_json::json!({ "not": { "anyOf": alternatives.clone() } });
            alternatives.push(none);
        }
        self.constraint(serde_json::json!({ "oneOf": alternatives }));
    }

    /// `field` is present whenever any of `others` is.
    pub fn requires_if(&mut self, field: (&str, FieldKind), others: &[(&str, FieldKind)]) {
        let mut when = any_present(others);
        let when = if when.len() == 1 {
            when.remove(0)
        } else {
            serde_json::json!({ "anyOf": when })
        };
        self.constraint(serde_json::json!({
            "if": when,
            "then": present_schema(field.0, field.1),
        }));
    }

    /// `a` and `b` are not both present.
    pub fn conflicts_with(&mut self, a: (&str, FieldKind), b: (&str, FieldKind)) {
        self.constraint(serde_json::json!({
            "not": { "allOf": [present_schema(a.0, a.1), present_schema(b.0, b.1)] }
        }));
    }

    /// `field` or any of `others` is present.
    pub fn required_unless(&mut self, field: (&str, FieldKind), others: &[(&str, FieldKind)]) {
        let mut alternatives = vec![present_schema(field.0, field.1)];
        alternatives.extend(any_present(others));
        self.constraint(serde_json::json!({ "anyOf": alternatives }));
    }

    /// At least one of `members` is present.
    pub fn at_least_one(&mut self, members: &[(&str, FieldKind)]) {
        self.constraint(serde_json::json!({ "anyOf": any_present(members) }));
    }

    /// Lets an absorbed property admit `null`, which `from_json` reads as
    /// absent: a relation of this struct names a leaf of a flattened group.
    pub fn nullable(&mut self, key: &str) {
        if self.required.iter().any(|r| r == key) {
            return;
        }
        if let Some(property) = self.properties.get_mut(key) {
            *property = nullable(property.take());
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
            self.required.extend(
                required
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned)),
            );
        }
        for key in ["oneOf", "anyOf", "not"] {
            if let Some(value) = schema.remove(key) {
                self.constraint(serde_json::json!({ key: value }));
            }
        }
        if let Some(when) = schema.remove("if") {
            let mut c = Map::new();
            c.insert("if".to_owned(), when);
            for key in ["then", "else"] {
                if let Some(value) = schema.remove(key) {
                    c.insert(key.to_owned(), value);
                }
            }
            self.constraint(Value::Object(c));
        }
        if let Some(Value::Array(all)) = schema.remove("allOf") {
            self.constraints.extend(all);
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
        let mut constraints = self.constraints;
        match constraints.len() {
            0 => {}
            1 => {
                if let (Value::Object(root), Value::Object(only)) =
                    (&mut schema, constraints.remove(0))
                {
                    root.extend(only);
                }
            }
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

/// `field` is required when `other` is given.
pub fn requires_if_error(field: &str, other: String) -> anyhow::Error {
    usage(UsageError::RequiresIf {
        field: field.to_owned(),
        other,
    })
}

/// `field` cannot be given with `other`.
pub fn conflicts_error(field: &str, other: String) -> anyhow::Error {
    usage(UsageError::Conflicts {
        field: field.to_owned(),
        other,
    })
}

/// `name` is required unless one of `unless` is given.
pub fn missing_unless_error(name: &str, unless: Vec<String>) -> anyhow::Error {
    usage(UsageError::MissingUnless(super::MissingUnless {
        name: name.to_owned(),
        unless: unless.join(" or "),
    }))
}

pub fn check_at_least_one(group: &str, given: usize) -> Result<()> {
    if given == 0 {
        return Err(usage(UsageError::AtLeastOne {
            group: group.to_owned(),
        }));
    }
    Ok(())
}

/// A `required` Vec or Option that is absent (or empty).
pub fn check_required(present: bool, key: &str) -> Result<()> {
    if present {
        Ok(())
    } else {
        Err(missing(key))
    }
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

/// Reads `path`; with `hide` the error leaves the (secret) path out.
fn read_path(flag: &str, path: &str, hide: bool) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| {
        let reason = if hide {
            format!("cannot read the file: {e}")
        } else {
            format!("cannot read {path}: {e}")
        };
        invalid(flag, reason)
    })
}

/// The decode result of an `#[op(secret)]` field: any error but a missing
/// value becomes a bare `invalid value`, since serde's messages quote the
/// value they reject.
pub fn hide_value<T>(key: &str, result: Result<T>) -> Result<T> {
    result.map_err(|err| {
        if coded_error::facts_anyhow(&err).code == "op_registry.usage.missing" {
            err
        } else {
            invalid(key, "invalid value")
        }
    })
}

/// A `Json` flag value: JSON text, `@path` or `@-`.
pub fn json_arg(cx: &mut ParseCx, flag: &str, text: &str) -> Result<Json> {
    let bytes = match text.strip_prefix('@') {
        Some("-") => cx.read_stdin(flag)?,
        Some(path) => read_path(flag, path, false)?,
        None => text.as_bytes().to_vec(),
    };
    serde_json::from_slice(&bytes)
        .map(Json)
        .map_err(|e| invalid(flag, format!("not valid JSON: {e}")))
}

/// An `Input` flag value: a path, or `-` for stdin.
pub fn input_arg(cx: &mut ParseCx, flag: &str, text: &str) -> Result<Input> {
    let bytes = if text == "-" {
        cx.read_stdin(flag)?
    } else {
        read_path(flag, text, false)?
    };
    Ok(Input(bytes))
}

/// A field value a subject's scope can name: its resolved id or raw text.
/// `Err` is a usage error (a path that is not UTF-8).
pub trait ScopeValue {
    fn scope_value(&self) -> Result<Option<String>>;
}

impl ScopeValue for String {
    fn scope_value(&self) -> Result<Option<String>> {
        Ok(Some(self.clone()))
    }
}

impl ScopeValue for crate::Ref {
    fn scope_value(&self) -> Result<Option<String>> {
        Ok(Some(match self.resolved_opt() {
            Some(crate::Resolved::Id(s) | crate::Resolved::Raw(s)) => s.clone(),
            _ => self.as_raw().to_owned(),
        }))
    }
}

impl ScopeValue for std::path::PathBuf {
    fn scope_value(&self) -> Result<Option<String>> {
        match self.to_str() {
            Some(s) => Ok(Some(s.to_owned())),
            None => Err(usage(UsageError::InvalidField {
                name: "scope".to_owned(),
                reason: format!("path {} is not valid UTF-8", self.display()),
            })),
        }
    }
}

impl<T: ScopeValue> ScopeValue for Option<T> {
    fn scope_value(&self) -> Result<Option<String>> {
        self.as_ref().map_or(Ok(None), ScopeValue::scope_value)
    }
}

/// The scope of a subject: the named fields that were given.
pub fn scope(pairs: Vec<(&str, Result<Option<String>>)>) -> Result<crate::Scope> {
    let mut out = Vec::new();
    for (name, value) in pairs {
        if let Some(value) = value? {
            out.push((name.to_owned(), value));
        }
    }
    Ok(crate::Scope::from_pairs(out))
}

/// `base` with the entries of `over` added (`over` wins on a clash).
pub fn merge_scope(base: &crate::Scope, over: &crate::Scope) -> crate::Scope {
    base.merged(over)
}

/// The text form of a `#[op(secret)]` field's value, for redaction.
pub trait SecretValue {
    fn push_secrets(&self, out: &mut Vec<Vec<u8>>);
}

macro_rules! secret_by_to_string {
    ($($t:ty),*) => {$(
        impl SecretValue for $t {
            fn push_secrets(&self, out: &mut Vec<Vec<u8>>) {
                out.push(self.to_string().into_bytes());
            }
        }
    )*};
}
secret_by_to_string!(String, i32, i64, u8, u16, u32, u64, usize, f64);

impl SecretValue for bool {
    fn push_secrets(&self, _out: &mut Vec<Vec<u8>>) {}
}

impl SecretValue for std::path::PathBuf {
    fn push_secrets(&self, out: &mut Vec<Vec<u8>>) {
        out.push(self.to_string_lossy().into_owned().into_bytes());
    }
}

impl SecretValue for crate::Ref {
    fn push_secrets(&self, out: &mut Vec<Vec<u8>>) {
        out.push(self.as_raw().as_bytes().to_vec());
    }
}

impl SecretValue for Json {
    fn push_secrets(&self, out: &mut Vec<Vec<u8>>) {
        out.push(self.0.to_string().into_bytes());
    }
}

impl SecretValue for Input {
    fn push_secrets(&self, out: &mut Vec<Vec<u8>>) {
        out.push(self.0.clone());
    }
}

impl SecretValue for BTreeMap<String, String> {
    fn push_secrets(&self, out: &mut Vec<Vec<u8>>) {
        out.extend(self.values().map(|v| v.clone().into_bytes()));
    }
}

impl<T: SecretValue> SecretValue for Option<T> {
    fn push_secrets(&self, out: &mut Vec<Vec<u8>>) {
        if let Some(v) = self {
            v.push_secrets(out);
        }
    }
}

impl<T: SecretValue> SecretValue for Vec<T> {
    fn push_secrets(&self, out: &mut Vec<Vec<u8>>) {
        self.iter().for_each(|v| v.push_secrets(out));
    }
}

impl<T: SecretValue> SecretValue for Patch<T> {
    fn push_secrets(&self, out: &mut Vec<Vec<u8>>) {
        if let Patch::Set(v) = self {
            v.push_secrets(out);
        }
    }
}

#[cfg(feature = "secrets")]
impl SecretValue for crate::SecretFile {
    fn push_secrets(&self, out: &mut Vec<Vec<u8>>) {
        out.push(self.bytes().to_vec());
        if self.path_is_secret() {
            if let Some(path) = self.path() {
                out.push(path.to_string_lossy().into_owned().into_bytes());
            }
        }
    }
}

/// A `SecretFile` flag value: a path, or `-` for stdin.
#[cfg(feature = "secrets")]
pub fn secret_file_arg(
    cx: &mut ParseCx,
    flag: &str,
    text: &str,
    hide_path: bool,
) -> Result<crate::SecretFile> {
    let file = if text == "-" {
        crate::SecretFile::new(None, cx.read_stdin(flag)?)
    } else {
        crate::SecretFile::new(Some(text.into()), read_path(flag, text, hide_path)?)
    };
    Ok(if hide_path { file.hide_path() } else { file })
}

/// A `SecretFile` JSON value: a path string, or with `inline` also
/// `{"inline": "<base64>"}`. Absent and `null` are `None`.
#[cfg(feature = "secrets")]
pub fn secret_file_json(
    object: &Map<String, Value>,
    key: &str,
    inline: bool,
    hide_path: bool,
) -> Result<Option<crate::SecretFile>> {
    let file = match object.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(path)) => {
            crate::SecretFile::new(Some(path.into()), read_path(key, path, hide_path)?)
        }
        Some(Value::Object(o)) if inline => {
            let text = match (o.len(), o.get("inline")) {
                (1, Some(Value::String(text))) => text,
                _ => return Err(invalid(key, "expected {\"inline\": \"<base64>\"}")),
            };
            let bytes = super::base64::decode(text).map_err(|e| invalid(key, e))?;
            crate::SecretFile::new(None, bytes)
        }
        Some(_) => return Err(invalid(key, "expected a path string")),
    };
    Ok(Some(if hide_path { file.hide_path() } else { file }))
}
