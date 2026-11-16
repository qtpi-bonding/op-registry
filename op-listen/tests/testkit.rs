#![cfg(feature = "testkit")]

use futures_util::stream;
use futures_util::{FutureExt, StreamExt};
use op_listen::testkit::{short_follow_options, Followed};
use op_listen::FollowEnd;
use op_listen::ListenCursor;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

fn stop_connect() -> Followed {
    Followed::spawn(
        |_cursor| {
            async {
                let frames: Vec<anyhow::Result<Value>> = vec![
                    Ok(json!({"cursor":"0:1","frame":"item","item":"a"})),
                    Ok(json!({"code":"op_listen.revoked","frame":"stop","message":"gone"})),
                ];
                Ok(stream::iter(frames).boxed())
            }
            .boxed()
        },
        short_follow_options(),
    )
}

#[test]
fn short_follow_options_have_no_retry_cap() {
    assert_eq!(short_follow_options().max_retries, None);
}

#[tokio::test]
async fn followed_collects_json_lines_and_finishes_on_stop() {
    let mut followed = stop_connect();
    followed
        .wait_until("stop marker", Duration::from_secs(5), |lines| {
            lines.iter().any(|l| l["marker"] == "stopped")
        })
        .await;
    let lines = followed.lines();
    assert_eq!(lines[0]["item"], "a");
    assert_eq!(lines.last().unwrap()["marker"], "stopped");
    let end = followed.join().await.expect("follow ends cleanly");
    assert!(matches!(end, FollowEnd::Stopped { ref code } if code == "op_listen.revoked"));
}

#[tokio::test]
#[should_panic(expected = "waiting for step-x")]
async fn wait_until_panics_with_the_step_name_on_timeout() {
    let mut followed = Followed::spawn(
        |_cursor| async { Ok(stream::pending().boxed()) }.boxed(),
        short_follow_options(),
    );
    followed
        .wait_until("step-x", Duration::from_millis(50), |lines| {
            !lines.is_empty()
        })
        .await;
}

/// A forever-follow whose connect closure owns a clone of `probe`: the clone is dropped exactly
/// when the spawned task (which owns the closure) is gone.
fn forever_follow(probe: &Arc<()>) -> Followed {
    let guard = Arc::clone(probe);
    Followed::spawn(
        move |_cursor| {
            let _guard = &guard;
            async { Ok(stream::pending().boxed()) }.boxed()
        },
        short_follow_options(),
    )
}

async fn assert_task_gone(probe: &Arc<()>) {
    for _ in 0..200 {
        if Arc::strong_count(probe) == 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the follow task is still alive");
}

#[tokio::test]
async fn abort_stops_a_forever_follow() {
    let probe = Arc::new(());
    let mut followed = forever_follow(&probe);
    followed.assert_still_following("following").await;
    assert_eq!(Arc::strong_count(&probe), 2, "the task holds the closure");
    followed.abort();
    assert_task_gone(&probe).await;
}

#[tokio::test]
async fn dropping_followed_aborts_the_task() {
    let probe = Arc::new(());
    let followed = forever_follow(&probe);
    assert_eq!(Arc::strong_count(&probe), 2, "the task holds the closure");
    drop(followed);
    assert_task_gone(&probe).await;
}

#[tokio::test]
async fn assert_still_following_passes_while_following() {
    let probe = Arc::new(());
    let mut followed = forever_follow(&probe);
    followed.assert_still_following("step-x").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    followed.assert_still_following("step-x").await;
}

#[tokio::test]
#[should_panic(expected = "step-y")]
async fn assert_still_following_panics_after_the_follow_ended() {
    let mut followed = stop_connect();
    followed
        .wait_until("stop marker", Duration::from_secs(5), |lines| {
            lines.iter().any(|l| l["marker"] == "stopped")
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    followed.assert_still_following("step-y").await;
}

#[tokio::test]
async fn assert_still_following_message_has_the_result_and_frames() {
    let mut followed = stop_connect();
    followed
        .wait_until("stop marker", Duration::from_secs(5), |lines| {
            lines.iter().any(|l| l["marker"] == "stopped")
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let panic = std::panic::AssertUnwindSafe(followed.assert_still_following("step-y"));
    let err = futures_util::FutureExt::catch_unwind(panic)
        .await
        .expect_err("the follow ended");
    let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains("step-y"), "{msg}");
    assert!(msg.contains("Stopped"), "{msg}");
    assert!(msg.contains("Frames so far"), "{msg}");
    assert!(
        msg.contains("\"item\":\"a\"") || msg.contains("item"),
        "{msg}"
    );
}

#[tokio::test]
#[should_panic(expected = "Frames so far")]
async fn wait_until_panics_with_frames_when_the_follow_ended_first() {
    let mut followed = stop_connect();
    followed
        .wait_until("step-z", Duration::from_secs(5), |lines| {
            lines.iter().any(|l| l["marker"] == "never")
        })
        .await;
}

#[tokio::test]
#[should_panic(expected = "Frames so far")]
async fn wait_until_timeout_message_has_the_frames() {
    let probe = Arc::new(());
    let mut followed = forever_follow(&probe);
    followed
        .wait_until("step-x", Duration::from_millis(50), |lines| {
            !lines.is_empty()
        })
        .await;
}

#[tokio::test]
async fn reconnect_markers_counts_reconnects() {
    let mut calls = 0u32;
    let mut followed = Followed::spawn(
        move |_cursor| {
            calls += 1;
            let n = calls;
            async move {
                if n == 1 {
                    // A retryable stream end: the follower reconnects.
                    let frames: Vec<anyhow::Result<Value>> =
                        vec![Ok(json!({"cursor":"0:1","frame":"item","item":"a"}))];
                    Ok(stream::iter(frames).boxed())
                } else {
                    let frames: Vec<anyhow::Result<Value>> =
                        vec![Ok(json!({"cursor":"0:2","frame":"item","item":"b"}))];
                    Ok(stream::iter(frames).chain(stream::pending()).boxed())
                }
            }
            .boxed()
        },
        short_follow_options(),
    );
    assert_eq!(followed.reconnect_markers(), 0);
    followed
        .wait_until(
            "an item after the reconnect",
            Duration::from_secs(5),
            |lines| lines.iter().any(|l| l["item"] == "b"),
        )
        .await;
    assert_eq!(followed.reconnect_markers(), 1);
}

#[tokio::test]
async fn spawn_uses_the_options_it_is_given() {
    let seen: Arc<std::sync::Mutex<Vec<Option<ListenCursor>>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let mut options = short_follow_options();
    options.initial_cursor = Some(ListenCursor::new(3, 7));
    let mut followed = Followed::spawn(
        move |cursor| {
            sink.lock().unwrap().push(cursor);
            async { Ok(stream::pending().boxed()) }.boxed()
        },
        options,
    );
    followed.assert_still_following("connecting").await;
    for _ in 0..100 {
        if !seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(*seen.lock().unwrap(), vec![Some(ListenCursor::new(3, 7))]);
}
