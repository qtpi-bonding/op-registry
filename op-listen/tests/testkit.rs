#![cfg(feature = "testkit")]

use futures_util::stream;
use futures_util::{FutureExt, StreamExt};
use op_listen::testkit::{short_follow_options, Followed};
use op_listen::FollowEnd;
use serde_json::{json, Value};
use std::time::Duration;

fn stop_connect() -> Followed {
    Followed::spawn(|_cursor| {
        async {
            let frames: Vec<anyhow::Result<Value>> = vec![
                Ok(json!({"cursor":"0:1","frame":"item","item":"a"})),
                Ok(json!({"code":"op_listen.revoked","frame":"stop","message":"gone"})),
            ];
            Ok(stream::iter(frames).boxed())
        }
        .boxed()
    })
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
    let end = followed.finish().await.expect("follow ends cleanly");
    assert!(matches!(end, FollowEnd::Stopped { ref code } if code == "op_listen.revoked"));
}

#[tokio::test]
#[should_panic(expected = "waiting for step-x")]
async fn wait_until_panics_with_the_step_name_on_timeout() {
    let mut followed = Followed::spawn(|_cursor| async { Ok(stream::pending().boxed()) }.boxed());
    followed
        .wait_until("step-x", Duration::from_millis(50), |lines| {
            !lines.is_empty()
        })
        .await;
}
