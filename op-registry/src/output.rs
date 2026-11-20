//! Output schema discovery: `Some(schema)` when the `schema` feature is on.

/// Produces the JSON schema of an operation output type, when available.
pub trait OutputSchema {
    fn output_schema() -> Option<serde_json::Value>;
}

#[cfg(feature = "schema")]
impl<T: schemars::JsonSchema> OutputSchema for T {
    fn output_schema() -> Option<serde_json::Value> {
        Some(serde_json::to_value(schemars::schema_for!(T)).expect("schemars output is valid JSON"))
    }
}

#[cfg(not(feature = "schema"))]
impl<T> OutputSchema for T {
    fn output_schema() -> Option<serde_json::Value> {
        None
    }
}
