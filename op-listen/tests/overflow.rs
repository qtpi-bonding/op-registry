#![allow(clippy::err_expect, clippy::ok_expect)]
mod common;

use common::*;
use futures_util::StreamExt;
use op_listen::{lagged, listen_stream, ListenOptions, OverflowPolicy};
use serde_json::json;

fn opts() -> ListenOptions {
    ListenOptions::default()
}

#[test]
fn lagged_item_with_resnapshot_policy_reopens_and_emits_a_resync_snapshot() {
    futures_executor::block_on(async {
        let l = StateListener::new(1, state(&[("a", "1")]), 2)
            .with_policy(OverflowPolicy::Resnapshot)
            .with_resync(false);
        l.queue(
            state(&[("a", "1")]),
            2,
            source(vec![ok(3, "x"), Err(lagged(3))]),
        );
        l.queue(state(&[("a", "2")]), 10, source(vec![ok(11, "z")]));

        let s = listen_stream(&l, &Ctx, params("t"), Some(lc(1, 1)), caller("a"), opts())
            .await
            .unwrap();
        let frames = frames_json(jsonify(s)).await;
        assert_eq!(
            frames,
            vec![
                json!({"frame": "snapshot", "snapshot": {"a": "1"}, "resync": false, "cursor": "1:2"}),
                json!({"frame": "item", "item": "x", "cursor": "1:3"}),
                json!({"frame": "snapshot", "snapshot": {"a": "2"}, "resync": true, "cursor": "1:10"}),
                json!({"frame": "item", "item": "z", "cursor": "1:11"}),
            ]
        );
        assert_eq!(
            l.open_calls(),
            vec![(Some(lc(1, 1)), None), (None, None)],
            "second open must be a fresh snapshot (cursor None)"
        );
    });
}

#[test]
fn lagged_item_with_end_policy_ends_the_stream_with_the_coded_lagged_error() {
    futures_executor::block_on(async {
        let l = StateListener::new(1, state(&[]), 2).with_policy(OverflowPolicy::End);
        l.queue(
            state(&[]),
            2,
            source(vec![ok(3, "x"), Err(lagged(3)), ok(4, "never")]),
        );
        let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"), opts())
            .await
            .unwrap();
        let mut s = jsonify(s);
        assert_eq!(s.next().await.unwrap().unwrap()["frame"], "snapshot");
        assert_eq!(s.next().await.unwrap().unwrap()["frame"], "item");
        let err = s.next().await.unwrap().err().expect("lagged error");
        assert_eq!(code_of(&err), "op_listen.lagged");
        assert!(s.next().await.is_none(), "stream ends after the lag error");
        assert_eq!(l.open_calls().len(), 1, "End policy never reopens");
    });
}

#[test]
fn resnapshot_at_an_equal_seq_continues_but_a_smaller_one_is_out_of_order() {
    futures_executor::block_on(async {
        // A lag that dropped no items: the fresh snapshot is at the last seq.
        let l = StateListener::new(1, state(&[]), 2).with_policy(OverflowPolicy::Resnapshot);
        l.queue(state(&[]), 2, source(vec![ok(3, "x"), Err(lagged(0))]));
        l.queue(state(&[("a", "2")]), 3, source(vec![ok(4, "z")]));
        let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"), opts())
            .await
            .unwrap();
        let frames = frames_json(jsonify(s)).await;
        assert_eq!(
            frames,
            vec![
                json!({"frame": "snapshot", "snapshot": {}, "resync": false, "cursor": "1:2"}),
                json!({"frame": "item", "item": "x", "cursor": "1:3"}),
                json!({"frame": "snapshot", "snapshot": {"a": "2"}, "resync": true, "cursor": "1:3"}),
                json!({"frame": "item", "item": "z", "cursor": "1:4"}),
            ]
        );

        // A snapshot older than what was already sent is still out of order.
        let l = StateListener::new(1, state(&[]), 2).with_policy(OverflowPolicy::Resnapshot);
        l.queue(state(&[]), 2, source(vec![ok(5, "x"), Err(lagged(0))]));
        l.queue(state(&[]), 4, source(vec![]));
        let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"), opts())
            .await
            .unwrap();
        let mut s = jsonify(s);
        s.next().await.unwrap().unwrap();
        s.next().await.unwrap().unwrap();
        let err = s.next().await.unwrap().err().expect("must fail");
        assert_eq!(
            coded_error::facts_anyhow(&err).code,
            "op_listen.out_of_order"
        );
        assert!(s.next().await.is_none());
    });
}
