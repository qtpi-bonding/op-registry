mod common;

use common::follow::*;
use common::*;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use op_listen::{
    follow_typed, supervise_stream, FollowEnd, FollowEvent, FollowOptions, ListenCursor,
    ListenFrame, Resumable, Supervised,
};
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

type Frame = ListenFrame<Value, String>;
type PlainConn = BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Frame>>>>;

fn frame_item(text: &str, cursor: ListenCursor) -> Frame {
    ListenFrame::Item {
        item: text.to_owned(),
        cursor,
    }
}

fn frame_stop(code: &str) -> Frame {
    ListenFrame::Stop {
        code: code.to_owned(),
        message: "gone".to_owned(),
    }
}

#[test]
fn listen_frame_resumable_maps_cursor_stop_and_resync() {
    let snap = Frame::Snapshot {
        snapshot: json!({}),
        resync: true,
        cursor: lc(1, 2),
    };
    assert_eq!(snap.cursor(), Some(lc(1, 2)));
    assert!(Resumable::resync(&snap));
    assert_eq!(Resumable::stop_code(&snap), None);
    let plain = frame_item("a", lc(1, 3));
    assert_eq!(Resumable::cursor(&plain), Some(lc(1, 3)));
    assert!(!Resumable::resync(&plain));
    let stop = frame_stop("x.y");
    assert_eq!(Resumable::cursor(&stop), None);
    assert_eq!(Resumable::stop_code(&stop), Some("x.y"));
}

type Seen = Arc<Mutex<Vec<String>>>;

async fn run_stream(
    script: Vec<Result<Vec<anyhow::Result<Frame>>, anyhow::Error>>,
    opts: FollowOptions,
) -> (anyhow::Result<FollowEnd>, Vec<Option<ListenCursor>>, Seen) {
    let calls: Arc<Mutex<Vec<Option<ListenCursor>>>> = Arc::new(Mutex::new(Vec::new()));
    let rec = calls.clone();
    let mut script = std::collections::VecDeque::from(script);
    let connect = move |cursor: Option<ListenCursor>| -> PlainConn {
        rec.lock().unwrap().push(cursor);
        let step = script.pop_front().expect("scripted");
        async move {
            match step {
                Ok(items) => Ok(futures_util::stream::iter(items).boxed()),
                Err(e) => Err(e),
            }
        }
        .boxed()
    };
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let rec = seen.clone();
    let end = supervise_stream::<Frame, _, _>(connect, opts, move |event| {
        let line = match event {
            Supervised::Opened => "opened".to_owned(),
            Supervised::Item(Frame::Item { item, .. }) => format!("item:{item}"),
            Supervised::Item(_) => "item:other".to_owned(),
            Supervised::Retrying { code } => format!("retrying:{code}"),
            Supervised::Stopped { code } => format!("stopped:{code}"),
        };
        rec.lock().unwrap().push(line);
        Ok(())
    })
    .await;
    let calls = calls.lock().unwrap().clone();
    (end, calls, seen)
}

#[tokio::test(start_paused = true)]
async fn typed_frames_resume_from_last_cursor() {
    let (end, calls, _seen) = run_stream(
        vec![
            Ok(vec![
                Ok(frame_item("a", lc(0, 1))),
                Ok(frame_item("b", lc(0, 2))),
                Err(transient()),
            ]),
            Ok(vec![Ok(frame_stop("done"))]),
        ],
        FollowOptions::default(),
    )
    .await;
    end.expect("ends cleanly");
    assert_eq!(calls, vec![None, Some(lc(0, 2))]);
}

#[tokio::test(start_paused = true)]
async fn typed_stop_frame_ends_with_code() {
    let (end, calls, seen) = run_stream(
        vec![
            Ok(vec![
                Ok(frame_item("a", lc(0, 1))),
                Ok(frame_stop("op_listen.revoked")),
            ]),
            Ok(vec![Ok(frame_item("never", lc(0, 9)))]),
        ],
        FollowOptions::default(),
    )
    .await;
    assert_eq!(
        end.expect("ends cleanly"),
        FollowEnd::Stopped {
            code: "op_listen.revoked".to_owned()
        }
    );
    assert_eq!(calls.len(), 1);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            "opened".to_owned(),
            "item:a".to_owned(),
            "stopped:op_listen.revoked".to_owned()
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn stream_end_reconnects() {
    let (end, calls, seen) = run_stream(
        vec![
            Ok(vec![Ok(frame_item("a", lc(0, 1)))]),
            Ok(vec![Ok(frame_stop("done"))]),
        ],
        FollowOptions::default(),
    )
    .await;
    end.expect("ends cleanly");
    assert_eq!(calls, vec![None, Some(lc(0, 1))]);
    assert!(seen
        .lock()
        .unwrap()
        .contains(&"retrying:op_listen.ended".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn follow_typed_honours_listed_mid_stream_code() {
    let code = code_of(&denied());
    let (connect, connects) = scripted(vec![
        Step::Frames(vec![Ok(item("a", "0:1")), Err(denied())]),
        frames(vec![stop("never")]),
    ]);
    let (sink, events) = event_sink();
    let mut opts = FollowOptions::default();
    opts.stop_codes = vec![code.to_owned()];
    let end = follow_typed::<Value, String, _, _>(connect, sink, opts)
        .await
        .expect("a listed code ends with Ok");
    assert_eq!(
        end,
        FollowEnd::Stopped {
            code: code.to_owned()
        }
    );
    assert_eq!(connects.lock().unwrap().len(), 1);
    assert_eq!(
        events.lock().unwrap().last(),
        Some(&FollowEvent::Stopped {
            code: code.to_owned()
        })
    );
}
