mod common;

use common::follow::*;
use common::*;
use op_listen::{follow, follow_typed, FollowEnd, FollowEvent, FollowOptions};
use serde_json::Value;
use std::io::{Error, ErrorKind};

/// A sink that records lines and fails with `kind` on the `fail_at`-th call (1-based).
fn failing_sink(
    fail_at: usize,
    kind: ErrorKind,
) -> (impl FnMut(&str) -> std::io::Result<()>, Lines) {
    let lines: Lines = Default::default();
    let rec = lines.clone();
    let mut calls = 0;
    (
        move |line: &str| {
            calls += 1;
            if calls == fail_at {
                return Err(Error::from(kind));
            }
            rec.lock().unwrap().push(line.to_owned());
            Ok(())
        },
        lines,
    )
}

#[tokio::test(start_paused = true)]
async fn broken_pipe_sink_ends_sink_closed_without_reconnect() {
    let (connect, connects) = scripted(vec![frames(vec![snap(false, "0:1"), item("a", "0:2")])]);
    let (sink, lines) = failing_sink(2, ErrorKind::BrokenPipe);
    let end = follow(connect, sink, FollowOptions::default())
        .await
        .expect("a closed sink is a clean end");
    assert_eq!(end, FollowEnd::SinkClosed);
    assert_eq!(connects.lock().unwrap().len(), 1);
    assert_eq!(lines.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn broken_pipe_from_the_marker_write_also_closes() {
    let (connect, connects) = scripted(vec![
        Step::Frames(vec![Ok(snap(false, "0:1")), Err(transient())]),
        frames(vec![snap(true, "0:9"), stop("op_listen.revoked")]),
    ]);
    // line 1: first snapshot; line 2: the reconnected marker.
    let (sink, lines) = failing_sink(2, ErrorKind::BrokenPipe);
    let end = follow(connect, sink, FollowOptions::default())
        .await
        .expect("a closed sink is a clean end");
    assert_eq!(end, FollowEnd::SinkClosed);
    assert_eq!(connects.lock().unwrap().len(), 2);
    assert_eq!(lines.lock().unwrap().len(), 1, "nothing after the marker");
}

#[tokio::test(start_paused = true)]
async fn other_sink_error_propagates() {
    let (connect, connects) = scripted(vec![frames(vec![snap(false, "0:1"), item("a", "0:2")])]);
    let (sink, _lines) = failing_sink(2, ErrorKind::Other);
    let err = follow(connect, sink, FollowOptions::default())
        .await
        .expect_err("other sink errors are not a clean end");
    let io = err.downcast_ref::<Error>().expect("carries the io error");
    assert_eq!(io.kind(), ErrorKind::Other);
    assert_eq!(connects.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn broken_pipe_in_a_stream_error_is_retried() {
    let (connect, connects) = scripted(vec![
        Step::Frames(vec![Err(anyhow::Error::from(Error::from(
            ErrorKind::BrokenPipe,
        )))]),
        frames(vec![stop("op_listen.revoked")]),
    ]);
    let (sink, _lines) = make_sink();
    let end = follow(connect, sink, FollowOptions::default())
        .await
        .expect("retried to a stop");
    assert_eq!(
        end,
        FollowEnd::Stopped {
            code: "op_listen.revoked".to_owned()
        }
    );
    assert_eq!(connects.lock().unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn typed_follow_also_closes_on_broken_pipe() {
    let (connect, connects) = scripted(vec![frames(vec![snap(false, "0:1")])]);
    let end = follow_typed::<Value, String, _, _>(
        connect,
        |_: FollowEvent<Value, String>| Err(Error::from(ErrorKind::BrokenPipe)),
        FollowOptions::default(),
    )
    .await
    .expect("a closed sink is a clean end");
    assert_eq!(end, FollowEnd::SinkClosed);
    assert_eq!(connects.lock().unwrap().len(), 1);
}
