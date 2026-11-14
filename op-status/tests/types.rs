#![allow(clippy::err_expect, clippy::ok_expect)]
use op_status::{AttrValue, Component, State, StatusSnapshot};
use serde_json::{json, Value};

#[test]
fn state_serialises_as_screaming_snake_strings() {
    let table = [
        (State::Unspecified, "UNSPECIFIED"),
        (State::Unknown, "UNKNOWN"),
        (State::Starting, "STARTING"),
        (State::Ok, "OK"),
        (State::Degraded, "DEGRADED"),
        (State::Paused, "PAUSED"),
        (State::Down, "DOWN"),
        (State::Stopped, "STOPPED"),
    ];
    for (state, text) in table {
        assert_eq!(serde_json::to_value(state).unwrap(), json!(text));
        let back: State = serde_json::from_value(json!(text)).unwrap();
        assert_eq!(back, state);
    }
    assert!(serde_json::from_value::<State>(json!("ok")).is_err());
}

#[test]
fn attr_value_numbers_stay_numbers() {
    assert_eq!(serde_json::to_string(&AttrValue::Int(3)).unwrap(), "3");
    assert_eq!(serde_json::to_string(&AttrValue::Float(1.5)).unwrap(), "1.5");
    assert_eq!(serde_json::to_string(&AttrValue::Bool(true)).unwrap(), "true");
    assert_eq!(
        serde_json::to_string(&AttrValue::Str("x".into())).unwrap(),
        "\"x\""
    );
    assert_eq!(AttrValue::from(3i64), AttrValue::Int(3));
    assert_eq!(AttrValue::from(1.5f64), AttrValue::Float(1.5));
    assert_eq!(AttrValue::from(false), AttrValue::Bool(false));
    assert_eq!(AttrValue::from("s"), AttrValue::Str("s".into()));
    assert_eq!(AttrValue::from(String::from("s")), AttrValue::Str("s".into()));

    for v in [
        AttrValue::Int(-7),
        AttrValue::Float(2.25),
        AttrValue::Bool(false),
        AttrValue::Str("hello".into()),
    ] {
        let text = serde_json::to_string(&v).unwrap();
        let back: AttrValue = serde_json::from_str(&text).unwrap();
        assert_eq!(back, v, "{text}");
    }
    let n: AttrValue = serde_json::from_str("3").unwrap();
    assert_eq!(n, AttrValue::Int(3));
    let f: AttrValue = serde_json::from_str("1.5").unwrap();
    assert_eq!(f, AttrValue::Float(1.5));
}

#[test]
fn component_has_no_failing_for_secs_and_serialises_stable_field_order() {
    let c = Component::new("room", "r1", State::Degraded)
        .parent("service", "main")
        .ready(true)
        .terminal(false)
        .last_error("net.timeout", "timed out")
        .consecutive_failures(4)
        .changed_at_unix_ms(1_700_000_000_000)
        .attr("count", 3i64)
        .attr("name", "alpha");

    let expected = json!({
        "kind": "room",
        "id": "r1",
        "parent": {"kind": "service", "id": "main"},
        "state": "DEGRADED",
        "ready": true,
        "terminal": false,
        "last_error_code": "net.timeout",
        "last_error_message": "timed out",
        "consecutive_failures": 4,
        "changed_at_unix_ms": 1_700_000_000_000i64,
        "attrs": {"count": 3, "name": "alpha"}
    });
    assert_eq!(serde_json::to_value(&c).unwrap(), expected);

    let raw = serde_json::to_string(&c).unwrap();
    assert!(!raw.contains("failing_for_secs"), "{raw}");
    let order = [
        "\"kind\"",
        "\"id\"",
        "\"parent\"",
        "\"state\"",
        "\"ready\"",
        "\"terminal\"",
        "\"last_error_code\"",
        "\"last_error_message\"",
        "\"consecutive_failures\"",
        "\"changed_at_unix_ms\"",
        "\"attrs\"",
    ];
    let mut last = 0usize;
    for key in order {
        let pos = raw[last..]
            .find(key)
            .unwrap_or_else(|| panic!("{key} missing or out of order in {raw}"));
        last += pos + key.len();
    }

    // Defaults: all fields still serialised, None as null.
    let d = serde_json::to_value(Component::new("k", "i", State::Unknown)).unwrap();
    assert_eq!(
        d,
        json!({
            "kind": "k", "id": "i", "parent": null, "state": "UNKNOWN",
            "ready": false, "terminal": false,
            "last_error_code": "", "last_error_message": "",
            "consecutive_failures": null, "changed_at_unix_ms": 0, "attrs": {}
        })
    );

    // Round trip.
    let back: Component = serde_json::from_str(&raw).unwrap();
    assert_eq!(back, c);

    // Snapshot field order.
    let snap = StatusSnapshot {
        service: "s".into(),
        version: "1".into(),
        taken_at_unix_ms: 5,
        epoch: 2,
        components: vec![],
        truncated: false,
    };
    let raw = serde_json::to_string(&snap).unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        v,
        json!({"service":"s","version":"1","taken_at_unix_ms":5,"epoch":2,
               "components":[],"truncated":false})
    );
    let mut last = 0usize;
    for key in [
        "\"service\"",
        "\"version\"",
        "\"taken_at_unix_ms\"",
        "\"epoch\"",
        "\"components\"",
        "\"truncated\"",
    ] {
        let pos = raw[last..].find(key).unwrap_or_else(|| panic!("{key} in {raw}"));
        last += pos + key.len();
    }
}
