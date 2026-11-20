#![cfg(feature = "schema")]

use op_registry::OutputSchema;
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Serialize, JsonSchema)]
struct Echo {
    id: String,
}

#[test]
fn output_schema_is_some_with_schema_feature() {
    let schema = <Echo as OutputSchema>::output_schema().unwrap();
    assert_eq!(schema["properties"]["id"]["type"], "string");
}
