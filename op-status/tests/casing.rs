#![cfg(feature = "listen")]
mod common;

use common::Toy;
use op_registry::testing::{assert_casing, Casing};
use op_status::ops::{status_registry, StatusOptions};
use op_status::{Component, State, StatusSnapshot};
use serde_json::json;

#[test]
fn status_registry_is_camel_case() {
    assert_casing(&status_registry::<Toy>(StatusOptions::new()), Casing::Camel);
}

#[test]
fn snapshot_serializes_taken_at_unix_ms() {
    let snapshot = StatusSnapshot {
        service: "toy".into(),
        version: "1.0".into(),
        taken_at_unix_ms: 5,
        epoch: 7,
        components: vec![Component::new("db", "main", State::Ok)],
        truncated: false,
    };
    let value = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(value["takenAtUnixMs"], json!(5));
    assert!(value.get("taken_at_unix_ms").is_none());
    let component = &value["components"][0];
    for key in [
        "lastErrorCode",
        "lastErrorMessage",
        "consecutiveFailures",
        "changedAtUnixMs",
    ] {
        assert!(component.get(key).is_some(), "missing {key}: {component}");
    }
    assert_eq!(component["state"], json!("OK"));
    let back: StatusSnapshot = serde_json::from_value(value).unwrap();
    assert_eq!(back, snapshot);
}
