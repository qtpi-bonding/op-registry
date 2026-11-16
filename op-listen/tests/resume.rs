#![allow(clippy::err_expect, clippy::ok_expect)]
mod common;

use common::*;
use futures_util::StreamExt;
use op_listen::{listen_stream, ListenCursor};
use serde_json::json;

#[test]
fn replay_listener_resumes_exactly_after_the_cursor_without_a_snapshot() {
    futures_executor::block_on(async {
        let log = vec![(1, "a"), (2, "b"), (3, "c"), (4, "d"), (5, "e")];
        let l = LogListener::new(log.clone());
        let s = listen_stream(&l, &Ctx, params("t"), Some(lc(0, 3)), caller("a"))
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
        assert_eq!(l.open_calls(), vec![(Some(lc(0, 3)), None)]);

        let l = LogListener::new(log);
        let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"))
            .await
            .unwrap();
        let frames = frames_json(jsonify(s)).await;
        assert_eq!(frames.len(), 5);
        assert!(frames.iter().all(|f| f["frame"] == "item"));
        assert_eq!(l.open_calls(), vec![(None, None)]);
    });
}

#[test]
fn snapshot_listener_passes_the_resync_flag_through() {
    futures_executor::block_on(async {
        for flag in [true, false] {
            let l = StateListener::new(7, state(&[("k", "v")]), 3).with_resync(flag);
            let s = listen_stream(&l, &Ctx, params("t"), Some(lc(7, 2)), caller("a"))
                .await
                .unwrap();
            let frames = frames_json(jsonify(s)).await;
            assert_eq!(
                frames,
                vec![json!({
                    "frame": "snapshot",
                    "snapshot": {"k": "v"},
                    "resync": flag,
                    "cursor": "7:3"
                })]
            );
            assert_eq!(l.open_calls(), vec![(Some(lc(7, 2)), None)]);
        }
    });
}

#[test]
fn frames_carry_an_increasing_cursor() {
    futures_executor::block_on(async {
        let l = StateListener::new(4, state(&[]), 3);
        l.queue(
            state(&[]),
            3,
            source(vec![ok(4, "a"), ok(5, "b"), ok(9, "c")]),
        );
        let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"))
            .await
            .unwrap();
        let frames = frames_json(jsonify(s)).await;
        let cursors: Vec<String> = frames
            .iter()
            .map(|f| f["cursor"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(cursors, vec!["4:3", "4:4", "4:5", "4:9"]);
        let parsed: Vec<ListenCursor> = cursors.iter().map(|c| c.parse().unwrap()).collect();
        assert!(parsed.windows(2).all(|w| w[0] < w[1]));
    });
}

#[test]
fn a_non_increasing_source_seq_ends_the_stream_with_out_of_order() {
    futures_executor::block_on(async {
        // A repeated seq within the replay.
        let l = LogListener::new(vec![(1, "a"), (2, "b"), (2, "c"), (3, "d")]);
        let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"))
            .await
            .unwrap();
        let mut s = jsonify(s);
        assert_eq!(s.next().await.unwrap().unwrap()["cursor"], "0:1");
        assert_eq!(s.next().await.unwrap().unwrap()["cursor"], "0:2");
        let err = s.next().await.unwrap().err().expect("must be an error");
        assert_eq!(code_of(&err), "op_listen.out_of_order");
        assert!(s.next().await.is_none(), "stream must end after the error");

        // An item that does not advance past the snapshot's seq.
        let l = StateListener::new(0, state(&[]), 5);
        l.queue(state(&[]), 5, source(vec![ok(5, "dup"), ok(6, "later")]));
        let s = listen_stream(&l, &Ctx, params("t"), None, caller("a"))
            .await
            .unwrap();
        let mut s = jsonify(s);
        assert_eq!(s.next().await.unwrap().unwrap()["frame"], "snapshot");
        let err = s.next().await.unwrap().err().expect("must be an error");
        assert_eq!(code_of(&err), "op_listen.out_of_order");
        assert!(s.next().await.is_none());
    });
}
