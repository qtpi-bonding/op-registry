use op_listen::{ListenCursor, ListenFrame};
use op_registry::Cursor;
use serde_json::{json, Value};

fn code_of(err: &anyhow::Error) -> &'static str {
    coded_error::facts_anyhow(err).code
}

#[test]
fn listen_cursor_round_trips_and_rejects_garbage() {
    let c = ListenCursor::new(3, 17);
    assert_eq!((c.epoch, c.seq), (3, 17));
    assert_eq!(c.to_string(), "3:17");
    assert_eq!("3:17".parse::<ListenCursor>().unwrap(), c);
    let max = ListenCursor::new(u64::MAX, u64::MAX);
    assert_eq!(max.to_string().parse::<ListenCursor>().unwrap(), max);
    assert_eq!(
        "0:0".parse::<ListenCursor>().unwrap(),
        ListenCursor::new(0, 0)
    );

    // Ord: epoch first, then seq.
    assert!(ListenCursor::new(0, 9) < ListenCursor::new(1, 0));
    assert!(ListenCursor::new(1, 2) < ListenCursor::new(1, 3));
    assert!(ListenCursor::new(2, 0) > ListenCursor::new(1, u64::MAX));
    assert_eq!(ListenCursor::new(1, 1), ListenCursor::new(1, 1));
    assert_eq!(
        ListenCursor::new(1, 2).max(ListenCursor::new(0, 99)),
        ListenCursor::new(1, 2)
    );

    // Conversions with the registry's opaque cursor.
    let wire: Cursor = c.into();
    assert_eq!(wire, Cursor("3:17".to_owned()));
    assert_eq!(ListenCursor::try_from(&wire).unwrap(), c);

    for bad in [
        "",
        "1",
        "a:b",
        "1:2:3",
        "-1:2",
        " 1:2",
        "1:2 ",
        "1: 2",
        "+1:2",
        "1::2",
        ":",
        "1:",
        ":2",
        "18446744073709551616:0",
    ] {
        let err = bad.parse::<ListenCursor>().err().expect(bad);
        assert_eq!(code_of(&err), "op_listen.bad_cursor", "parse {bad:?}");
        let err = ListenCursor::try_from(&Cursor(bad.to_owned()))
            .err()
            .expect(bad);
        assert_eq!(code_of(&err), "op_listen.bad_cursor", "try_from {bad:?}");
    }
}

#[test]
fn frame_json_shape_is_pinned() {
    let snapshot: ListenFrame<Value, Value> = ListenFrame::Snapshot {
        snapshot: json!({"a": 1}),
        resync: false,
        cursor: "0:3".to_owned(),
    };
    assert_eq!(
        serde_json::to_value(&snapshot).unwrap(),
        json!({"frame": "snapshot", "snapshot": {"a": 1}, "resync": false, "cursor": "0:3"})
    );

    let item: ListenFrame<Value, Value> = ListenFrame::Item {
        item: json!("x"),
        cursor: "0:4".to_owned(),
    };
    assert_eq!(
        serde_json::to_value(&item).unwrap(),
        json!({"frame": "item", "item": "x", "cursor": "0:4"})
    );

    let stop: ListenFrame<Value, Value> = ListenFrame::Stop {
        code: "op_listen.revoked".to_owned(),
        message: "revoked".to_owned(),
    };
    assert_eq!(
        serde_json::to_value(&stop).unwrap(),
        json!({"frame": "stop", "code": "op_listen.revoked", "message": "revoked"})
    );

    for frame in [snapshot, item, stop] {
        let back: ListenFrame<Value, Value> =
            serde_json::from_value(serde_json::to_value(&frame).unwrap()).unwrap();
        assert_eq!(back, frame);
    }
}
