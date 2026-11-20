//! Value types used in parameter structs.

/// A field of an update op that can be kept, set or cleared.
///
/// `#[op(clearable)]` on a `Patch<T>` field adds `--clear-<field>`. On JSON
/// paths `null` is [`Patch::Clear`] and an absent key is [`Patch::Keep`].
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Patch<T> {
    /// Leave the stored value as it is.
    #[default]
    Keep,
    /// Replace the stored value.
    Set(T),
    /// Remove the stored value.
    Clear,
}

/// Any JSON value.
///
/// On the command line: a JSON string, `@path` (the file's JSON) or `@-`
/// (JSON from stdin). On JSON paths: the value itself.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Json(pub serde_json::Value);

/// Raw bytes.
///
/// On the command line: a path, or `-` for stdin. On JSON paths: a base64
/// string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input(pub Vec<u8>);

impl serde::Serialize for Input {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::base64::encode(&self.0))
    }
}

impl<'de> serde::Deserialize<'de> for Input {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        super::base64::decode(&text)
            .map(Input)
            .map_err(serde::de::Error::custom)
    }
}
