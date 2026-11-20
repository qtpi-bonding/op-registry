#![allow(clippy::err_expect, clippy::ok_expect)]
mod common;

use common::*;
use futures_util::StreamExt;
use op_listen::{listen_entry, listen_stream};
use op_registry::{CallOptions, Caller, OpSpec, Registry};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn authorize_refusal_sends_no_frame_and_surfaces_the_apps_coded_error() {
    futures_executor::block_on(async {
        let mut l = LogListener::new(vec![(1, "a"), (2, "b")]);
        l.refuse = true;
        let res = listen_stream(&l, &Ctx, params("t"), None, caller("mallory")).await;
        let err = res.err().expect("refusal must be an error, not a stream");
        assert_eq!(code_of(&err), "test.app.denied");
        assert!(
            l.open_calls().is_empty(),
            "open must never run after a refusal"
        );
        assert_eq!(l.authorize_calls().len(), 1);
    });
}

#[test]
fn authorize_is_called_once_before_the_first_frame_with_caller_and_topic() {
    futures_executor::block_on(async {
        let l = LogListener::new(vec![(1, "a"), (2, "b")]);
        let s = listen_stream(&l, &Ctx, params("room"), None, caller("alice"))
            .await
            .unwrap();
        let mut s = jsonify(s);
        // Already authorised before any frame is pulled.
        assert_eq!(
            l.authorize_calls(),
            vec![("alice".to_owned(), "room".to_owned())]
        );
        let first = s.next().await.unwrap().unwrap();
        assert_eq!(first["frame"], "item");
        while s.next().await.is_some() {}
        assert_eq!(l.authorize_calls().len(), 1);
        let events = l.events.lock().unwrap().clone();
        assert_eq!(events[0], "authorize");
        assert_eq!(events[1], "open");
        assert_eq!(l.still_allowed_count(), 0);
    });
}

#[test]
fn call_options_carry_the_caller_to_the_gate() {
    futures_executor::block_on(async {
        let l = Arc::new(LogListener::new(vec![(1, "a")]));
        let mut registry: Registry<Ctx> = Registry::new();
        registry.group("log", "Log.");
        listen_entry(
            &mut registry,
            l.clone(),
            OpSpec::new("log listen", "Listens to the log"),
        );

        let with = CallOptions::default().with_caller(Caller("bob".to_owned()));
        let frames = registry
            .call_stream("log_listen", json!({"topic": "t"}), &Ctx, with, None)
            .await
            .unwrap();
        let frames = frames_json(frames).await;
        assert_eq!(
            frames,
            vec![json!({"frame": "item", "item": "a", "cursor": "0:1"})]
        );
        assert_eq!(
            l.authorize_calls(),
            vec![("bob".to_owned(), "t".to_owned())]
        );
    });
}

#[test]
fn grant_not_before_floors_replay_and_clamps_an_older_cursor() {
    futures_executor::block_on(async {
        let log = vec![(1, "a"), (2, "b"), (3, "c"), (4, "d"), (5, "e")];
        let floor = lc(0, 3);

        // (A) No cursor; a listener that replays everything is trimmed by the runtime.
        let mut l = LogListener::new(log.clone()).with_grant(grant(Some(floor), None));
        l.replay_all = true;
        let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"))
            .await
            .unwrap();
        let frames = frames_json(jsonify(s)).await;
        assert_eq!(
            frames,
            vec![
                json!({"frame": "item", "item": "d", "cursor": "0:4"}),
                json!({"frame": "item", "item": "e", "cursor": "0:5"}),
            ]
        );
        assert_eq!(l.open_calls(), vec![(Some(floor), Some(floor))]);

        // (B) An older cursor is clamped up to the floor.
        let l = LogListener::new(log.clone()).with_grant(grant(Some(floor), None));
        let s = listen_stream(&l, &Ctx, params("t"), Some(lc(0, 1)), caller("a"))
            .await
            .unwrap();
        let frames = frames_json(jsonify(s)).await;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["cursor"], "0:4");
        assert_eq!(l.open_calls(), vec![(Some(floor), Some(floor))]);

        // (C) A newer cursor wins over the floor; the floor is still reported.
        let l = LogListener::new(log).with_grant(grant(Some(floor), None));
        let s = listen_stream(&l, &Ctx, params("t"), Some(lc(0, 4)), caller("a"))
            .await
            .unwrap();
        let frames = frames_json(jsonify(s)).await;
        assert_eq!(
            frames,
            vec![json!({"frame": "item", "item": "e", "cursor": "0:5"})]
        );
        assert_eq!(l.open_calls(), vec![(Some(lc(0, 4)), Some(floor))]);
    });
}

#[tokio::test(start_paused = true)]
async fn grant_default_never_rechecks() {
    let mut l = LogListener::new(vec![(1, "a"), (2, "b")]);
    l.tail_pending = true;
    let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"))
        .await
        .unwrap();
    let mut s = jsonify(s);
    assert!(s.next().await.is_some());
    assert!(s.next().await.is_some());
    let idle = tokio::time::timeout(Duration::from_secs(3600), s.next()).await;
    assert!(idle.is_err(), "stream must stay open and silent");
    assert_eq!(l.still_allowed_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn recheck_false_sends_stop_then_ends() {
    let mut l =
        LogListener::new(vec![(1, "a")]).with_grant(grant(None, Some(Duration::from_secs(10))));
    l.tail_pending = true;
    l.allowed.store(false, Ordering::SeqCst);
    let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"))
        .await
        .unwrap();
    let frames = tokio::time::timeout(Duration::from_secs(60), frames_json(jsonify(s)))
        .await
        .expect("stream must end after the stop frame");
    assert_eq!(frames.len(), 2);
    assert_eq!(
        frames[0],
        json!({"frame": "item", "item": "a", "cursor": "0:1"})
    );
    assert_eq!(frames[1]["frame"], "stop");
    assert_eq!(frames[1]["code"], "op_listen.revoked");
    assert!(!frames[1]["message"].as_str().unwrap().is_empty());
    assert_eq!(
        l.still_allowed_args.lock().unwrap()[0],
        ("a".to_owned(), "t".to_owned())
    );
}

#[tokio::test(start_paused = true)]
async fn recheck_does_not_hold_back_frames_while_still_allowed_is_pending() {
    // The live item only arrives at t = 20s, after the first check (t = 10s)
    // started and got stuck for 1000s.
    let late = futures_util::stream::once(async {
        tokio::time::sleep(Duration::from_secs(20)).await;
        ok(1, "a")
    })
    .chain(futures_util::stream::pending())
    .boxed();
    let mut l = LogListener::new(vec![])
        .with_grant(grant(None, Some(Duration::from_secs(10))))
        .with_tail(late);
    l.allowed_delay = Some(Duration::from_secs(1000));
    let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"))
        .await
        .unwrap();
    let mut s = jsonify(s);

    let frame = tokio::time::timeout(Duration::from_secs(25), s.next())
        .await
        .expect("frame must not wait for the pending check")
        .unwrap()
        .unwrap();
    assert_eq!(
        frame,
        json!({"frame": "item", "item": "a", "cursor": "0:1"})
    );
    assert_eq!(l.still_allowed_count(), 1);

    // Further ticks do not start a second check while one is pending.
    assert!(tokio::time::timeout(Duration::from_secs(100), s.next())
        .await
        .is_err());
    assert_eq!(l.still_allowed_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn recheck_runs_at_most_every_recheck_every() {
    let mut l = LogListener::new(vec![]).with_grant(grant(None, Some(Duration::from_secs(10))));
    l.tail_pending = true;
    let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"))
        .await
        .unwrap();
    let mut s = jsonify(s);

    assert!(tokio::time::timeout(Duration::from_secs(5), s.next())
        .await
        .is_err());
    assert_eq!(
        l.still_allowed_count(),
        0,
        "first check only after the interval"
    );

    assert!(tokio::time::timeout(Duration::from_secs(20), s.next())
        .await
        .is_err());
    // t = 25s: ticks at 10s and 20s.
    assert_eq!(l.still_allowed_count(), 2);

    assert!(tokio::time::timeout(Duration::from_secs(30), s.next())
        .await
        .is_err());
    // t = 55s: ticks at 10, 20, 30, 40, 50.
    assert_eq!(l.still_allowed_count(), 5);
}

#[test]
fn missing_caller_is_refused_before_authorize_is_called() {
    futures_executor::block_on(async {
        let l = Arc::new(LogListener::new(vec![(1, "a")]));
        let mut registry: Registry<Ctx> = Registry::new();
        registry.group("log", "Log.");
        listen_entry(
            &mut registry,
            l.clone(),
            OpSpec::new("log listen", "Listens to the log"),
        );
        let err = registry
            .call_stream(
                "log_listen",
                json!({"topic": "u"}),
                &Ctx,
                CallOptions::default(),
                None,
            )
            .await
            .err()
            .expect("no caller must be refused");
        let facts = coded_error::facts_anyhow(&err);
        assert_eq!(facts.code, "op_listen.no_caller");
        assert_eq!(facts.class, coded_error::ErrorClass::Logical);
        assert!(l.authorize_calls().is_empty(), "authorize must not run");
        assert!(l.open_calls().is_empty());
    });
}
