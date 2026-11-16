mod common;

use common::follow::*;
use common::*;
use op_listen::{follow_typed, FollowEnd, FollowEvent, FollowOptions, ListenFrame};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

type Ev = FollowEvent<Value, String>;

fn event_sink() -> (impl FnMut(Ev) -> std::io::Result<()>, Arc<Mutex<Vec<Ev>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let rec = events.clone();
    (
        move |event: Ev| {
            rec.lock().unwrap().push(event);
            Ok(())
        },
        events,
    )
}

fn script() -> Vec<Step> {
    vec![
        Step::Frames(vec![
            Ok(snap(false, "0:1")),
            Ok(item("a", "0:2")),
            Err(transient()),
        ]),
        frames(vec![snap(true, "0:9"), stop("op_listen.revoked")]),
    ]
}

#[tokio::test(start_paused = true)]
async fn typed_events_mirror_the_json_lines() {
    let (connect, _connects) = scripted(script());
    let (sink, events) = event_sink();
    let end = follow_typed::<Value, String, _, _>(connect, sink, FollowOptions::default())
        .await
        .expect("ends cleanly");
    assert!(matches!(end, FollowEnd::Stopped { ref code } if code == "op_listen.revoked"));
    let want = vec![
        FollowEvent::Frame(ListenFrame::Snapshot {
            snapshot: json!({"k": "v"}),
            resync: false,
            cursor: lc(0, 1),
        }),
        FollowEvent::Frame(ListenFrame::Item {
            item: "a".to_owned(),
            cursor: lc(0, 2),
        }),
        FollowEvent::Reconnected {
            cursor: Some(lc(0, 2)),
            resync: true,
        },
        FollowEvent::Frame(ListenFrame::Snapshot {
            snapshot: json!({"k": "v"}),
            resync: true,
            cursor: lc(0, 9),
        }),
        FollowEvent::Frame(ListenFrame::Stop {
            code: "op_listen.revoked".to_owned(),
            message: "gone".to_owned(),
        }),
        FollowEvent::Stopped {
            code: "op_listen.revoked".to_owned(),
        },
    ];
    assert_eq!(*events.lock().unwrap(), want);
}

#[tokio::test(start_paused = true)]
async fn typed_decode_failure_is_bad_frame_and_not_retried() {
    let (connect, connects) = scripted(vec![
        frames(vec![json!({"frame": "item", "item": 5, "cursor": "0:1"})]),
        frames(vec![stop("op_listen.revoked")]),
    ]);
    let (sink, events) = event_sink();
    let err = follow_typed::<Value, String, _, _>(connect, sink, FollowOptions::default())
        .await
        .expect_err("decode failure ends the follow");
    assert_eq!(code_of(&err), "op_listen.bad_frame");
    assert_eq!(connects.lock().unwrap().len(), 1);
    assert!(events.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn unknown_frame_tag_is_bad_frame() {
    let (connect, connects) = scripted(vec![frames(vec![json!({"frame": "bogus"})])]);
    let (sink, _events) = event_sink();
    let err = follow_typed::<Value, String, _, _>(connect, sink, FollowOptions::default())
        .await
        .expect_err("unknown tag ends the follow");
    assert_eq!(code_of(&err), "op_listen.bad_frame");
    assert_eq!(connects.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn typed_reconnect_resumes_from_the_last_typed_cursor() {
    let (connect, connects) = scripted(script());
    let (sink, _events) = event_sink();
    follow_typed::<Value, String, _, _>(connect, sink, FollowOptions::default())
        .await
        .expect("ends cleanly");
    let cursors: Vec<_> = connects.lock().unwrap().iter().map(|(c, _)| *c).collect();
    assert_eq!(cursors, vec![None, Some(lc(0, 2))]);
}
