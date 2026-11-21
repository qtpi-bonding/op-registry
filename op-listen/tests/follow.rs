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

// ---- follow_stream: a reconnecting follower as a stream ----

mod stream_follow {
    use super::*;
    use crate::common::{caller, jsonify, ok, params, source, state, Ctx, StateListener};
    use futures_util::stream::BoxStream;
    use futures_util::{FutureExt, StreamExt};
    use op_listen::{follow_stream, Followed, ListenError, ListenFrame};
    use serde_json::Map;

    fn default_marker(cursor: Option<&ListenCursor>, resync: bool) -> Map<String, Value> {
        let Value::Object(map) = json!({
            "marker": "reconnected",
            "cursor": cursor.map(|c| c.to_string()),
            "resync": resync,
        }) else {
            unreachable!()
        };
        map
    }

    /// Runs the follower to its end: every yielded value as JSON, then the
    /// final error (a follower never ends with `Ok`).
    async fn run(
        mut s: BoxStream<'_, anyhow::Result<Followed<Value>>>,
    ) -> (Vec<Value>, anyhow::Error) {
        let mut out = Vec::new();
        while let Some(next) = s.next().await {
            match next {
                Ok(v) => out.push(serde_json::to_value(v).unwrap()),
                Err(e) => {
                    assert!(s.next().await.is_none(), "the stream ends after its error");
                    return (out, e);
                }
            }
        }
        panic!("a follower ends with an error, never cleanly: {out:?}");
    }

    fn stopped_code(err: &anyhow::Error) -> Option<String> {
        match coded_error::downcast_ref::<ListenError>(err) {
            Some(ListenError::Stopped { code }) => Some(code.clone()),
            _ => None,
        }
    }

    fn limited(max: Option<u32>) -> FollowOptions {
        let mut o = FollowOptions::default();
        o.max_retries = max;
        o
    }

    #[tokio::test(start_paused = true)]
    async fn marker_before_first_frame_of_reconnection() {
        let (connect, connects) = scripted(vec![
            Step::ConnectErr(transient()),
            Step::ConnectErr(transient()),
            frames(vec![
                snap(true, "0:9"),
                item("b", "0:10"),
                stop("op_listen.revoked"),
            ]),
        ]);
        let (out, err) = run(follow_stream(connect, opts(), default_marker)).await;
        assert_eq!(
            out,
            vec![
                marker(Value::Null, true),
                snap(true, "0:9"),
                item("b", "0:10"),
                stop("op_listen.revoked"),
            ]
        );
        assert_eq!(stopped_code(&err).as_deref(), Some("op_listen.revoked"));
        assert_eq!(cursors(&connects), vec![None, None, None]);
    }

    #[tokio::test(start_paused = true)]
    async fn marker_follows_a_served_connection_and_failed_opens() {
        let (connect, connects) = scripted(vec![
            Step::Frames(vec![Ok(item("a", "0:2")), Err(transient())]),
            Step::ConnectErr(transient()),
            Step::ConnectErr(transient()),
            frames(vec![snap(true, "0:9"), stop("op_listen.revoked")]),
        ]);
        let (out, _) = run(follow_stream(connect, opts(), default_marker)).await;
        assert_eq!(
            out,
            vec![
                item("a", "0:2"),
                marker(json!("0:2"), true),
                snap(true, "0:9"),
                stop("op_listen.revoked"),
            ]
        );
        assert_eq!(
            cursors(&connects),
            vec![None, Some(lc(0, 2)), Some(lc(0, 2)), Some(lc(0, 2))]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn max_retries_zero_ends_after_first_failure() {
        let (connect, connects) = scripted(vec![Step::Frames(vec![
            Ok(item("a", "0:1")),
            Err(transient()),
        ])]);
        let (out, err) = run(follow_stream(connect, limited(Some(0)), default_marker)).await;
        assert_eq!(out, vec![item("a", "0:1")]);
        assert_eq!(code_of(&err), "test.app.transient");
        assert_eq!(connects.lock().unwrap().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn stop_item_yields_frame_once_then_stopped_error() {
        let (connect, connects) = scripted(vec![frames(vec![
            item("a", "0:1"),
            stop("op_listen.revoked"),
        ])]);
        let (out, err) = run(follow_stream(connect, opts(), default_marker)).await;
        assert_eq!(out, vec![item("a", "0:1"), stop("op_listen.revoked")]);
        assert_eq!(code_of(&err), "op_listen.stopped");
        assert_eq!(
            coded_error::facts_anyhow(&err).class,
            coded_error::ErrorClass::Logical
        );
        assert_eq!(stopped_code(&err).as_deref(), Some("op_listen.revoked"));
        assert_eq!(
            connects.lock().unwrap().len(),
            1,
            "a stop does not reconnect"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn listed_mid_stream_code_ends_with_stopped() {
        let (connect, _) = scripted(vec![Step::Frames(vec![
            Ok(item("a", "0:1")),
            Err(denied()),
        ])]);
        let mut o = opts();
        o.stop_codes = vec!["test.app.denied".to_owned()];
        let (out, err) = run(follow_stream(connect, o, default_marker)).await;
        assert_eq!(out, vec![item("a", "0:1")], "no frame for the error");
        assert_eq!(code_of(&err), "op_listen.stopped");
        assert_eq!(stopped_code(&err).as_deref(), Some("test.app.denied"));
    }

    #[tokio::test(start_paused = true)]
    async fn other_logical_error_ends_with_that_error() {
        let (connect, _) = scripted(vec![Step::Frames(vec![Err(denied())])]);
        let (out, err) = run(follow_stream(connect, opts(), default_marker)).await;
        assert!(out.is_empty());
        assert_eq!(code_of(&err), "test.app.denied");
    }

    #[tokio::test(start_paused = true)]
    async fn grorg_marker_override_printed_verbatim() {
        let (connect, _) = scripted(vec![
            Step::Frames(vec![Ok(item("a", "0:7")), Err(transient())]),
            frames(vec![item("b", "0:8"), stop("op_listen.revoked")]),
        ]);
        let grorg = |cursor: Option<&ListenCursor>, _resync: bool| {
            let Value::Object(map) = json!({
                "marker": "reconnected",
                "afterSeq": cursor.map(|c| c.seq),
            }) else {
                unreachable!()
            };
            map
        };
        let (out, _) = run(follow_stream(connect, opts(), grorg)).await;
        assert_eq!(out[1], json!({"marker": "reconnected", "afterSeq": 7}));
        assert_eq!(out.len(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn resnapshot_overflow_under_follow_stream() {
        let l = StateListener::new(1, state(&[("a", "1")]), 2)
            .with_policy(op_listen::OverflowPolicy::Resnapshot)
            .with_resync(false);
        l.queue(
            state(&[("a", "1")]),
            2,
            source(vec![ok(3, "x"), Err(op_listen::lagged(3))]),
        );
        l.queue(state(&[("a", "2")]), 10, source(vec![ok(11, "z")]));
        let connect = |cursor: Option<ListenCursor>| {
            let l = &l;
            async move {
                let s = op_listen::listen_stream(l, &Ctx, params("t"), cursor, caller("a")).await?;
                Ok::<_, anyhow::Error>(jsonify_frames(s))
            }
            .boxed()
        };
        let (out, err) = run(follow_stream(connect, limited(Some(0)), default_marker)).await;
        let kinds: Vec<&str> = out.iter().map(|v| v["frame"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["snapshot", "item", "snapshot", "item"]);
        assert_eq!(
            out[2]["resync"], true,
            "the overflow resnapshot is a resync"
        );
        assert_eq!(out[3]["item"], "z");
        assert_eq!(l.open_calls().len(), 2, "one reopen inside listen_stream");
        assert_eq!(code_of(&err), "op_listen.ended");
    }

    fn jsonify_frames<'a>(
        s: BoxStream<
            'a,
            anyhow::Result<ListenFrame<std::collections::BTreeMap<String, String>, String>>,
        >,
    ) -> BoxStream<'a, anyhow::Result<Value>> {
        jsonify(s)
    }

    #[tokio::test(start_paused = true)]
    async fn failed_open_is_retried() {
        let script = || {
            vec![
                Step::ConnectErr(transient()),
                Step::ConnectErr(transient()),
                frames(vec![item("a", "0:1"), stop("op_listen.revoked")]),
            ]
        };
        let (connect, connects) = scripted(script());
        let (out, err) = run(follow_stream(connect, limited(Some(2)), default_marker)).await;
        assert_eq!(
            out,
            vec![
                marker(Value::Null, false),
                item("a", "0:1"),
                stop("op_listen.revoked")
            ]
        );
        assert_eq!(stopped_code(&err).as_deref(), Some("op_listen.revoked"));
        assert_eq!(connects.lock().unwrap().len(), 3);

        // One retry is not enough for two failed opens.
        let (connect, connects) = scripted(script());
        let (out, err) = run(follow_stream(connect, limited(Some(1)), default_marker)).await;
        assert!(out.is_empty());
        assert_eq!(code_of(&err), "test.app.transient");
        assert_eq!(connects.lock().unwrap().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn empty_connections_print_no_marker() {
        let (connect, connects) = scripted(vec![
            Step::Frames(vec![Ok(item("a", "0:1")), Err(transient())]),
            Step::Frames(vec![]),
            Step::Frames(vec![]),
            frames(vec![item("b", "0:2"), stop("op_listen.revoked")]),
        ]);
        let (out, _) = run(follow_stream(connect, opts(), default_marker)).await;
        assert_eq!(
            out,
            vec![
                item("a", "0:1"),
                marker(json!("0:1"), false),
                item("b", "0:2"),
                stop("op_listen.revoked"),
            ],
            "exactly one marker, before the first item of the connection that had one"
        );
        assert_eq!(connects.lock().unwrap().len(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn empty_connections_count_toward_max_retries() {
        let (connect, connects) = scripted(vec![
            Step::Frames(vec![]),
            Step::Frames(vec![]),
            Step::Frames(vec![]),
        ]);
        let (out, err) = run(follow_stream(connect, limited(Some(2)), default_marker)).await;
        assert!(out.is_empty(), "no marker, no frame: {out:?}");
        assert_eq!(code_of(&err), "op_listen.ended");
        assert_eq!(connects.lock().unwrap().len(), 3);

        // The kit's `max_retries = Some(0)` ends the follow after one.
        let (connect, connects) = scripted(vec![Step::Frames(vec![])]);
        let (out, err) = run(follow_stream(connect, limited(Some(0)), default_marker)).await;
        assert!(out.is_empty());
        assert_eq!(code_of(&err), "op_listen.ended");
        assert_eq!(connects.lock().unwrap().len(), 1);
    }
}
