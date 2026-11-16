#![allow(clippy::err_expect, clippy::ok_expect)]
mod common;

use common::follow::*;
use op_listen::{follow, Backoff, FollowEnd, FollowOptions, ListenCursor};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.app")]
enum AppError {
    #[error("not yours")]
    #[coded(code = "denied", class = Logical)]
    Denied,
    #[error("try again")]
    #[coded(code = "transient", class = Transient)]
    Transient,
}

fn denied() -> anyhow::Error {
    coded_error::into_anyhow(AppError::Denied)
}
fn transient() -> anyhow::Error {
    coded_error::into_anyhow(AppError::Transient)
}
fn code_of(err: &anyhow::Error) -> &'static str {
    coded_error::facts_anyhow(err).code
}

fn lc(epoch: u64, seq: u64) -> ListenCursor {
    ListenCursor::new(epoch, seq)
}

fn parsed(lines: &Lines) -> Vec<Value> {
    lines
        .lock()
        .unwrap()
        .iter()
        .map(|l| {
            assert!(!l.contains('\n'), "no newline inside a line: {l:?}");
            serde_json::from_str(l).expect("each line is JSON")
        })
        .collect()
}

fn cursors(c: &Connects) -> Vec<Option<ListenCursor>> {
    c.lock().unwrap().iter().map(|(c, _)| *c).collect()
}

fn opts() -> FollowOptions {
    FollowOptions::default()
}

#[tokio::test(start_paused = true)]
async fn follow_prints_one_json_line_per_frame() {
    let (connect, connects) = scripted(vec![Step::Frames(vec![
        Ok(snap(false, "0:1")),
        Ok(item("a", "0:2")),
        Ok(item("b", "0:3")),
        Err(denied()),
    ])]);
    let (sink, lines) = make_sink();
    let mut o = opts();
    o.initial_cursor = Some(lc(0, 1));
    let res = follow(connect, sink, o).await;
    assert_eq!(
        code_of(&res.err().expect("logical error ends it")),
        "test.app.denied"
    );
    assert_eq!(
        parsed(&lines),
        vec![snap(false, "0:1"), item("a", "0:2"), item("b", "0:3")]
    );
    assert_eq!(cursors(&connects), vec![Some(lc(0, 1))]);
}

#[tokio::test(start_paused = true)]
async fn follow_reconnects_with_the_last_cursor_and_prints_the_marker() {
    let (connect, connects) = scripted(vec![
        frames(vec![snap(false, "0:1"), item("a", "0:2")]),
        frames(vec![item("b", "0:3"), stop("op_listen.revoked")]),
    ]);
    let (sink, lines) = make_sink();
    let res = follow(connect, sink, opts()).await.unwrap();
    assert_eq!(
        res,
        FollowEnd::Stopped {
            code: "op_listen.revoked".to_owned()
        }
    );
    assert_eq!(cursors(&connects), vec![None, Some(lc(0, 2))]);
    assert_eq!(
        parsed(&lines),
        vec![
            snap(false, "0:1"),
            item("a", "0:2"),
            marker(json!("0:2"), false),
            item("b", "0:3"),
            stop("op_listen.revoked"),
            json!({"marker": "stopped", "code": "op_listen.revoked"}),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn follow_marker_resync_is_true_only_for_a_resync_snapshot() {
    let (connect, connects) = scripted(vec![
        Step::Frames(vec![]),
        frames(vec![snap(true, "0:5")]),
        frames(vec![snap(false, "0:9"), stop("op_listen.revoked")]),
    ]);
    let (sink, lines) = make_sink();
    follow(connect, sink, opts()).await.unwrap();
    assert_eq!(cursors(&connects), vec![None, None, Some(lc(0, 5))]);
    assert_eq!(
        parsed(&lines),
        vec![
            marker(Value::Null, true),
            snap(true, "0:5"),
            marker(json!("0:5"), false),
            snap(false, "0:9"),
            stop("op_listen.revoked"),
            json!({"marker": "stopped", "code": "op_listen.revoked"}),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn follow_does_not_reconnect_on_a_logical_error() {
    // From the stream.
    let (connect, connects) = scripted(vec![Step::Frames(vec![
        Ok(item("a", "0:1")),
        Err(denied()),
    ])]);
    let (sink, lines) = make_sink();
    let err = follow(connect, sink, opts())
        .await
        .err()
        .expect("must fail");
    assert_eq!(code_of(&err), "test.app.denied");
    assert_eq!(connects.lock().unwrap().len(), 1);
    assert_eq!(parsed(&lines), vec![item("a", "0:1")], "no marker");

    // From connect itself.
    let (connect, connects) = scripted(vec![Step::ConnectErr(denied())]);
    let (sink, lines) = make_sink();
    let err = follow(connect, sink, opts())
        .await
        .err()
        .expect("must fail");
    assert_eq!(code_of(&err), "test.app.denied");
    assert_eq!(connects.lock().unwrap().len(), 1);
    assert!(lines.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn follow_stop_prints_the_stopped_marker_and_does_not_reconnect() {
    let (connect, connects) = scripted(vec![frames(vec![
        item("a", "0:1"),
        stop("test.app.denied"),
        item("never", "0:2"),
    ])]);
    let (sink, lines) = make_sink();
    let res = follow(connect, sink, opts()).await;
    assert_eq!(
        res.unwrap(),
        FollowEnd::Stopped {
            code: "test.app.denied".to_owned()
        }
    );
    assert_eq!(connects.lock().unwrap().len(), 1);
    assert_eq!(
        parsed(&lines),
        vec![
            item("a", "0:1"),
            stop("test.app.denied"),
            json!({"marker": "stopped", "code": "test.app.denied"}),
        ]
    );
}

fn gaps(c: &Connects) -> Vec<Duration> {
    let c = c.lock().unwrap();
    c.windows(2).map(|w| w[1].1 - w[0].1).collect()
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[tokio::test(start_paused = true)]
async fn follow_backoff_is_capped_and_max_retries_is_honoured() {
    // Failures only: 250, 500, 1000, 2000, then capped at 2000.
    let steps = (0..6).map(|_| Step::ConnectErr(transient())).collect();
    let (connect, connects) = scripted(steps);
    let (sink, lines) = make_sink();
    let mut o = opts();
    o.backoff = Backoff::new(ms(250), ms(2000));
    o.max_retries = Some(5);
    let err = follow(connect, sink, o)
        .await
        .err()
        .expect("retries exhausted");
    assert_eq!(code_of(&err), "test.app.transient");
    assert_eq!(connects.lock().unwrap().len(), 6);
    assert_eq!(
        gaps(&connects),
        vec![ms(250), ms(500), ms(1000), ms(2000), ms(2000)]
    );
    assert!(lines.lock().unwrap().is_empty());

    // Defaults.
    let d = FollowOptions::default();
    assert_eq!(d.backoff, Backoff::follower());
    assert_eq!(d.max_retries, None);
    assert_eq!(d.initial_cursor, None);

    // max_retries = Some(0): one attempt, then the error.
    let (connect, connects) = scripted(vec![Step::ConnectErr(transient())]);
    let (sink, _) = make_sink();
    let mut o = opts();
    o.max_retries = Some(0);
    let err = follow(connect, sink, o).await.err().expect("no retries");
    assert_eq!(code_of(&err), "test.app.transient");
    assert_eq!(connects.lock().unwrap().len(), 1);

    // A clean end counts as a failure; the last error is `ended`.
    let (connect, connects) = scripted(vec![Step::Frames(vec![]), Step::Frames(vec![])]);
    let (sink, _) = make_sink();
    let mut o = opts();
    o.max_retries = Some(1);
    let err = follow(connect, sink, o)
        .await
        .err()
        .expect("retries exhausted");
    assert_eq!(code_of(&err), "op_listen.ended");
    assert_eq!(connects.lock().unwrap().len(), 2);

    // A delivered frame resets the backoff.
    let (connect, connects) = scripted(vec![
        Step::ConnectErr(transient()),
        Step::ConnectErr(transient()),
        frames(vec![item("a", "0:1")]),
        Step::ConnectErr(denied()),
    ]);
    let (sink, _) = make_sink();
    let err = follow(connect, sink, opts())
        .await
        .err()
        .expect("logical ends it");
    assert_eq!(code_of(&err), "test.app.denied");
    assert_eq!(gaps(&connects), vec![ms(250), ms(500), ms(250)]);
}

/// Runs three idle connections of `life` each, then a logical connect error;
/// returns the follow result and the connect-to-connect gaps.
async fn idle_run(
    life: Duration,
    max_retries: Option<u32>,
) -> (anyhow::Result<FollowEnd>, Vec<Duration>) {
    let (connect, connects) = scripted(vec![
        Step::Idle(life),
        Step::Idle(life),
        Step::Idle(life),
        Step::ConnectErr(denied()),
    ]);
    let (sink, _) = make_sink();
    let mut o = opts();
    o.backoff = Backoff::new(ms(20), ms(200)).with_healthy_after(ms(200));
    o.max_retries = max_retries;
    let res = follow(connect, sink, o).await;
    (res, gaps(&connects))
}

#[tokio::test(start_paused = true)]
async fn idle_connection_that_lived_past_the_healthy_period_resets_backoff() {
    let (res, gaps) = idle_run(ms(300), None).await;
    assert_eq!(
        code_of(&res.err().expect("logical ends it")),
        "test.app.denied"
    );
    // Each gap is the connection lifetime plus a delay that stays at 20ms.
    assert_eq!(gaps, vec![ms(320), ms(320), ms(320)]);
}

#[tokio::test(start_paused = true)]
async fn short_lived_connection_does_not_reset_backoff() {
    let (res, gaps) = idle_run(ms(10), None).await;
    assert_eq!(
        code_of(&res.err().expect("logical ends it")),
        "test.app.denied"
    );
    assert_eq!(gaps, vec![ms(30), ms(50), ms(90)]);
}

#[tokio::test(start_paused = true)]
async fn healthy_idle_connection_also_clears_the_retry_budget() {
    let (res, _) = idle_run(ms(300), Some(1)).await;
    assert_eq!(
        code_of(&res.err().expect("logical ends it")),
        "test.app.denied"
    );
    // Control: short-lived idle endings do exhaust the same budget.
    let (res, _) = idle_run(ms(10), Some(1)).await;
    assert_eq!(
        code_of(&res.err().expect("budget exhausted")),
        "op_listen.ended"
    );
}

#[tokio::test(start_paused = true)]
async fn healthy_idle_connection_clears_the_budget_when_first_equals_cap() {
    let (connect, _) = scripted(vec![
        Step::Idle(ms(300)),
        Step::Idle(ms(300)),
        Step::ConnectErr(denied()),
    ]);
    let (sink, _) = make_sink();
    let mut o = opts();
    o.backoff = Backoff::new(ms(20), ms(20)).with_healthy_after(ms(200));
    o.max_retries = Some(1);
    let res = follow(connect, sink, o).await;
    assert_eq!(
        code_of(&res.err().expect("logical ends it")),
        "test.app.denied"
    );
}
