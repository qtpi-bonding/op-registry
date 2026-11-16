use op_listen::{ListenCursor, ListenFrame};
use serde_json::json;

#[test]
fn listen_frame_schema_and_wire_are_pinned() {
    let item = ListenFrame::<String, String>::Item {
        item: "a".into(),
        cursor: ListenCursor::new(1, 2),
    };
    assert_eq!(
        serde_json::to_string(&item).unwrap(),
        r#"{"frame":"item","item":"a","cursor":"1:2"}"#
    );
    let snapshot = ListenFrame::<String, String>::Snapshot {
        snapshot: "s".into(),
        resync: true,
        cursor: ListenCursor::new(1, 2),
    };
    assert_eq!(
        serde_json::to_string(&snapshot).unwrap(),
        r#"{"frame":"snapshot","snapshot":"s","resync":true,"cursor":"1:2"}"#
    );
    let stop = ListenFrame::<String, String>::Stop {
        code: "c".into(),
        message: "m".into(),
    };
    assert_eq!(
        serde_json::to_string(&stop).unwrap(),
        r#"{"frame":"stop","code":"c","message":"m"}"#
    );

    let schema = serde_json::to_value(schemars::schema_for!(ListenFrame<String, String>)).unwrap();
    assert_eq!(
        schema,
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "ListenFrame",
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "cursor": {"type": "string"},
                        "frame": {"const": "snapshot", "type": "string"},
                        "resync": {"type": "boolean"},
                        "snapshot": {"type": "string"}
                    },
                    "required": ["frame", "snapshot", "resync", "cursor"]
                },
                {
                    "type": "object",
                    "properties": {
                        "cursor": {"type": "string"},
                        "frame": {"const": "item", "type": "string"},
                        "item": {"type": "string"}
                    },
                    "required": ["frame", "item", "cursor"]
                },
                {
                    "type": "object",
                    "properties": {
                        "code": {"type": "string"},
                        "frame": {"const": "stop", "type": "string"},
                        "message": {"type": "string"}
                    },
                    "required": ["frame", "code", "message"]
                }
            ]
        })
    );
}
