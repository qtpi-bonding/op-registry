mod common;

use common::*;
use op_listen::{listen_entry, ListenCursor, ListenFrame};
use op_registry::testing::{assert_casing, Casing};
use op_registry::{OpSpec, Registry};
use serde_json::json;
use std::sync::Arc;

#[test]
fn listen_registry_is_camel_case() {
    let mut registry = Registry::<Ctx>::new();
    registry.group("log", "Log.");
    listen_entry(
        &mut registry,
        Arc::new(LogListener::new(vec![(1, "a")])),
        OpSpec::new("log listen", "listens to the log"),
    );
    assert_casing(&registry, Casing::Camel);
}

#[test]
fn listen_frame_is_camel_case() {
    let schema = serde_json::to_value(schemars::schema_for!(ListenFrame<String, String>)).unwrap();
    let text = schema.to_string();
    assert!(
        !text.contains('_'),
        "snake_case key in frame schema: {text}"
    );
    let frame = ListenFrame::<String, String>::Snapshot {
        snapshot: "s".into(),
        resync: false,
        cursor: ListenCursor::new(1, 2),
    };
    let value = serde_json::to_value(&frame).unwrap();
    assert_eq!(value["frame"], json!("snapshot"));
}
