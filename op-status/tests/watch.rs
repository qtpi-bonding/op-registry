#![allow(clippy::err_expect, clippy::ok_expect)]
mod common;

use coded_error::ErrorClass;
use common::*;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use op_listen::{
    follow, listen_stream, Caller, FollowOptions, ListenCursor, ListenOptions, Listener, Opened,
    OverflowPolicy,
};
use op_registry::cli::{self};
use op_registry::parity::{assert_parity, cli_surface};
use op_registry::{mcp, Audience};
use op_status::ops::{status_registry, GetParams, StatusOptions};
use op_status::{open_status, Component, StatusFilter, StatusListener, WATCH_BUFFER};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

fn gp() -> GetParams {
    GetParams {
        kind: None,
        id: None,
        limit: None,
    }
}

fn caller() -> Caller {
    Caller("tester".to_owned())
}

fn lc(epoch: u64, seq: u64) -> ListenCursor {
    ListenCursor::new(epoch, seq)
}

type Frames = Vec<anyhow::Result<Value>>;

async fn frames(src: &Toy, listener: &StatusListener, cursor: Option<ListenCursor>) -> Frames {
    let stream = listen_stream(
        listener,
        src,
        gp(),
        cursor,
        caller(),
        ListenOptions::default(),
    )
    .await
    .ok()
    .expect("listen_stream");
    stream
        .map(|f| f.and_then(|f| Ok(serde_json::to_value(f)?)))
        .collect()
        .await
}

fn ok(frames: &Frames, i: usize) -> &Value {
    frames[i].as_ref().ok().expect("frame is ok")
}

/// Pushes `seqs` onto the log and pins newest to `newest`.
fn log_items(src: &Toy, seqs: &[u64], newest: u64) {
    for seq in seqs {
        src.push(*seq, comp(&format!("c{seq}")));
    }
    src.set_window(1, newest);
}

#[tokio::test]
async fn watch_first_frame_is_a_full_snapshot_with_resync_true_without_a_cursor() {
    let src = Toy::new().with_rows(vec![comp("a"), comp("b")]);
    src.set_window(1, 3);
    let listener = StatusListener::new(100);
    let out = frames(&src, &listener, None).await;
    assert_eq!(out.len(), 1);
    let first = ok(&out, 0);
    assert_eq!(first["frame"], "snapshot");
    assert_eq!(first["resync"], true);
    assert_eq!(first["cursor"], "7:3");
    assert_eq!(first["snapshot"]["service"], "toy");
    assert_eq!(first["snapshot"]["epoch"], 7);
    assert_eq!(first["snapshot"]["components"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn watch_with_matching_epoch_and_retained_seq_is_resync_false() {
    let src = Toy::new().with_rows(vec![comp("a")]);
    src.set_window(2, 5);
    let listener = StatusListener::new(100);
    let out = frames(&src, &listener, Some(lc(7, 2))).await;
    let first = ok(&out, 0);
    assert_eq!(first["frame"], "snapshot");
    assert_eq!(first["resync"], false);
    assert_eq!(first["cursor"], "7:5");
    assert_eq!(first["snapshot"]["components"][0]["id"], "a");
}

#[tokio::test]
async fn watch_with_other_epoch_or_too_old_seq_is_resync_true_not_an_error() {
    let src = Toy::new().with_rows(vec![comp("a")]);
    src.set_window(2, 5);
    let listener = StatusListener::new(100);
    for cursor in [lc(8, 3), lc(7, 1), lc(7, 6)] {
        let out = frames(&src, &listener, Some(cursor)).await;
        assert_eq!(out.len(), 1, "{cursor}");
        let first = ok(&out, 0);
        assert_eq!(first["frame"], "snapshot", "{cursor}");
        assert_eq!(first["resync"], true, "{cursor}");
        assert_eq!(first["cursor"], "7:5", "{cursor}");
    }
}

#[tokio::test]
async fn watch_frames_carry_an_increasing_cursor() {
    let src = Toy::new().with_rows(vec![comp("a")]);
    log_items(&src, &[4, 5, 9], 3);
    let listener = StatusListener::new(100);
    let out = frames(&src, &listener, None).await;
    assert_eq!(out.len(), 4);
    let mut cursors = Vec::new();
    for (i, expected) in ["snapshot", "item", "item", "item"].iter().enumerate() {
        let f = ok(&out, i);
        assert_eq!(f["frame"], *expected);
        cursors.push(
            f["cursor"]
                .as_str()
                .unwrap()
                .parse::<ListenCursor>()
                .unwrap(),
        );
    }
    assert_eq!(cursors[0].to_string(), "7:3");
    assert!(cursors.iter().all(|c| c.epoch == 7));
    let seqs: Vec<u64> = cursors.iter().map(|c| c.seq).collect();
    assert_eq!(seqs, vec![3, 4, 5, 9]);
    assert_eq!(ok(&out, 1)["item"]["id"], "c4");
    assert_eq!(ok(&out, 3)["cursor"], "7:9");
}

#[tokio::test]
async fn watch_overflow_emits_a_fresh_snapshot_and_continues() {
    let src = Toy::new().with_rows(vec![comp("a")]);
    let seqs: Vec<u64> = (1001..=1300).collect();
    log_items(&src, &seqs, 1000);
    let listener = StatusListener::new(100);
    assert_eq!(
        <StatusListener as Listener<Toy>>::overflow(&listener),
        OverflowPolicy::Resnapshot
    );
    let mut stream = listen_stream(
        &listener,
        &src,
        gp(),
        None,
        caller(),
        ListenOptions::default(),
    )
    .await
    .ok()
    .expect("listen_stream");
    let first = serde_json::to_value(stream.next().await.unwrap().ok().expect("snapshot")).unwrap();
    assert_eq!(first["cursor"], "7:1000");

    // After the first open, the source moves on: the re-snapshot sees newest 1300
    // and one later change.
    *src.log.lock().unwrap() = vec![(1301, comp("after"))];
    src.set_window(1, 1300);

    let rest: Frames = stream
        .map(|f| f.and_then(|f| Ok(serde_json::to_value(f)?)))
        .collect()
        .await;
    assert!(
        rest.iter().all(|f| f.is_ok()),
        "lag must not surface as an error"
    );
    let kinds: Vec<&str> = rest
        .iter()
        .map(|f| f.as_ref().unwrap()["frame"].as_str().unwrap())
        .collect();
    let snap_at = kinds
        .iter()
        .position(|k| *k == "snapshot")
        .expect("a fresh snapshot");
    assert!(kinds[..snap_at].iter().all(|k| *k == "item"));
    assert!(snap_at <= WATCH_BUFFER);
    assert_eq!(kinds.iter().filter(|k| **k == "snapshot").count(), 1);
    let snap = ok(&rest, snap_at);
    assert_eq!(snap["resync"], true);
    assert_eq!(snap["cursor"], "7:1300");
    assert_eq!(rest.len(), snap_at + 2);
    let last = ok(&rest, snap_at + 1);
    assert_eq!(last["frame"], "item");
    assert_eq!(last["cursor"], "7:1301");
    assert_eq!(last["item"]["id"], "after");
}

#[test]
fn status_watch_is_absent_when_the_source_has_no_watch_or_opts_say_no() {
    let with = status_registry::<Toy>(StatusOptions::new());
    assert!(with.get("status_watch").is_some());
    assert!(with.get("status_get").is_some());

    let off = status_registry::<Toy>(StatusOptions::new().watch(false));
    assert!(off.get("status_watch").is_none());
    assert!(off.get("status_get").is_some());

    let none = status_registry::<NoWatch>(StatusOptions::new());
    assert!(none.get("status_watch").is_none());
    assert!(none.get("status_get").is_some());
    assert_eq!(none.names(), vec!["status_get"]);
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn watch_change_for_an_unlisted_attr_key_is_refused_in_debug_and_dropped_in_release() {
    let src = Toy::new();
    src.push(
        4,
        comp("bad")
            .attr("count", 1i64)
            .attr("leaked_path", "/home/x"),
    );
    src.set_window(1, 3);
    let listener = StatusListener::new(100);
    let out = frames(&src, &listener, None).await;
    assert_eq!(out.len(), 2);
    assert_eq!(ok(&out, 0)["frame"], "snapshot");
    let err = out[1].as_ref().err().expect("debug refuses the change");
    assert_eq!(code_of(err), "op_status.attr_not_allowed");
    assert_eq!(class_of(err), ErrorClass::Logical);
    assert!(format!("{err:#}").contains("leaked_path"));
}

#[cfg(not(debug_assertions))]
#[tokio::test]
async fn watch_change_for_an_unlisted_attr_key_is_refused_in_debug_and_dropped_in_release() {
    let src = Toy::new();
    src.push(
        4,
        comp("bad")
            .attr("count", 1i64)
            .attr("leaked_path", "/home/x"),
    );
    src.set_window(1, 3);
    let listener = StatusListener::new(100);
    let out = frames(&src, &listener, None).await;
    assert_eq!(out.len(), 2);
    let item = ok(&out, 1);
    assert_eq!(item["frame"], "item");
    assert_eq!(item["item"]["attrs"], json!({"count": 1}));
    assert!(!item.to_string().contains("leaked_path"));
}

#[tokio::test]
async fn error_message_cannot_carry_a_long_payload_through_status_watch() {
    let src = Toy::new();
    src.push(4, comp("bad").last_error("e.code", "m".repeat(10_000)));
    src.set_window(1, 3);
    let listener = StatusListener::new(100);
    let out = frames(&src, &listener, None).await;
    assert_eq!(out.len(), 2);
    if cfg!(debug_assertions) {
        let err = out[1].as_ref().err().expect("debug refuses");
        assert_eq!(code_of(err), "op_status.field_too_long");
    } else {
        let msg = ok(&out, 1)["item"]["last_error_message"].as_str().unwrap();
        assert!(msg.len() <= 256, "{}", msg.len());
    }
}

#[test]
fn status_watch_is_a_cli_only_stream_entry_and_parity_holds() {
    let reg = status_registry::<Toy>(StatusOptions::new());
    assert_eq!(reg.names(), vec!["status_get", "status_watch"]);
    let d = reg.get("status_watch").unwrap();
    assert_eq!((d.noun.as_str(), d.verb.as_str()), ("status", "watch"));
    assert!(d.streaming);
    assert!(d.access.reads && !d.access.writes && !d.access.network);
    assert_eq!(d.audience, Audience::Agent);
    assert!(!d.output_schema.is_null());
    assert!(!reg.get("status_get").unwrap().streaming);

    let tools = mcp::tool_list(&reg, Audience::Agent);
    assert!(tools.iter().all(|t| t.name != "status_watch"));
    assert!(tools.iter().any(|t| t.name == "status_get"));

    let cmd = cli::command(&reg, "app", "toy app");
    cmd.clone().debug_assert();
    let surface = cli_surface(&cmd);
    assert!(surface.names.contains("status_watch"));
    assert_parity(&reg, &surface, &tools, &[], &[], Audience::Agent).unwrap();
    cmd.try_get_matches_from(["app", "status", "watch", "--kind", "room", "--limit", "2"])
        .unwrap();

    let owner = status_registry::<Toy>(StatusOptions::new().audience(Audience::Owner));
    assert_eq!(owner.get("status_watch").unwrap().audience, Audience::Owner);
}

fn rows3() -> Vec<Component> {
    vec![comp("a"), comp("b"), comp("c")]
}

#[tokio::test]
async fn status_listener_open_matches_open_status() {
    let src = Toy::new().with_rows(rows3());
    log_items(&src, &[4, 5], 3);
    let listener = StatusListener::new(100);
    assert_eq!(<StatusListener as Listener<Toy>>::epoch(&listener), 0);
    assert_eq!(
        <StatusListener as Listener<Toy>>::topic(&listener, &gp()),
        "status"
    );
    let grant = <StatusListener as Listener<Toy>>::authorize(&listener, &src, &caller(), "status")
        .await
        .ok()
        .expect("authorize");
    assert!(grant.not_before.is_none() && grant.recheck_every.is_none());

    let cases = [
        (None, None),
        (Some(lc(7, 3)), Some(lc(7, 3))),
        (Some(lc(8, 3)), Some(lc(8, 3))),
        (Some(lc(7, 0)), Some(lc(7, 0))),
    ];
    src.set_window(1, 3);
    for (listen_cursor, status_cursor) in cases {
        let direct = open_status(&src, &StatusFilter::default(), 100, status_cursor)
            .await
            .ok()
            .expect("open_status");
        let direct_changes: Vec<(u64, Component)> = direct
            .changes
            .map(|r| r.ok().expect("change"))
            .collect()
            .await;

        let opened = listener
            .open(&src, gp(), listen_cursor, None)
            .await
            .ok()
            .expect("listener open");
        let Opened::Snapshot {
            snapshot,
            resync,
            seq,
            stream,
            ..
        } = opened
        else {
            panic!("expected a snapshot open");
        };
        assert_eq!(snapshot, direct.snapshot, "{listen_cursor:?}");
        assert_eq!(resync, direct.resync, "{listen_cursor:?}");
        assert_eq!(seq, direct.seq);
        let changes: Vec<(u64, Component)> =
            stream.map(|r| r.ok().expect("change")).collect().await;
        assert_eq!(changes, direct_changes);
        assert_eq!(changes.len(), 2);
    }
    // The cursor mapping gives the expected resync values.
    let (listener_ref, src_ref) = (&listener, &src);
    let r = |c| async move {
        match listener_ref
            .open(src_ref, gp(), c, None)
            .await
            .ok()
            .expect("open")
        {
            Opened::Snapshot { resync, .. } => resync,
            _ => panic!("snapshot"),
        }
    };
    assert!(r(None).await);
    assert!(!r(Some(lc(7, 3))).await);
    assert!(r(Some(lc(8, 3))).await);

    // epoch() follows the source seen by the most recent open.
    assert_eq!(<StatusListener as Listener<Toy>>::epoch(&listener), 7);
    src.set_epoch(9);
    let _ = listener
        .open(&src, gp(), None, None)
        .await
        .ok()
        .expect("open");
    assert_eq!(<StatusListener as Listener<Toy>>::epoch(&listener), 9);
}

#[tokio::test]
async fn status_listener_maps_lag_to_the_op_listen_lagged_error() {
    let src = Toy::new();
    let seqs: Vec<u64> = (1001..=1300).collect();
    log_items(&src, &seqs, 1000);
    let listener = StatusListener::new(100);
    let Opened::Snapshot { mut stream, .. } = listener
        .open(&src, gp(), None, None)
        .await
        .ok()
        .expect("open")
    else {
        panic!("snapshot");
    };
    let err = loop {
        match stream.next().await {
            Some(Ok(_)) => {}
            Some(Err(e)) => break e,
            None => panic!("no lag error"),
        }
    };
    assert_eq!(code_of(&err), "op_listen.lagged");
    assert_eq!(class_of(&err), ErrorClass::Transient);
}

#[tokio::test]
async fn status_listener_clamps_limit_and_reports_truncated() {
    let src = Toy::new().with_rows((0..5).map(|i| comp(&format!("r{i}"))).collect());
    let listener = StatusListener::new(3);
    let snap = |params: GetParams| {
        let src = &src;
        let listener = &listener;
        async move {
            match listener
                .open(src, params, None, None)
                .await
                .ok()
                .expect("open")
            {
                Opened::Snapshot { snapshot, .. } => snapshot,
                _ => panic!("snapshot"),
            }
        }
    };
    let s = snap(gp()).await;
    assert_eq!(ids(&s.components), vec!["r0", "r1", "r2"]);
    assert!(s.truncated);
    let s = snap(GetParams {
        limit: Some(2),
        ..gp()
    })
    .await;
    assert_eq!(s.components.len(), 2);
    assert!(s.truncated);
    let s = snap(GetParams {
        limit: Some(100),
        ..gp()
    })
    .await;
    assert_eq!(s.components.len(), 3);
    assert!(s.truncated);
    let s = snap(GetParams {
        id: Some("r4".into()),
        ..gp()
    })
    .await;
    assert_eq!(ids(&s.components), vec!["r4"]);
    assert!(!s.truncated);
}

#[tokio::test(start_paused = true)]
async fn follow_over_status_watch_prints_marker_and_resnapshots_after_an_epoch_change() {
    let src: &'static Toy = Box::leak(Box::new(Toy::new().with_rows(vec![comp("a")])));
    let listener: &'static StatusListener = Box::leak(Box::new(StatusListener::new(100)));
    log_items(src, &[4, 5], 3);
    // First connection ends with a transient error after its items.
    src.push_tail(Err(transient()));

    let connects: Arc<Mutex<Vec<Option<ListenCursor>>>> = Arc::new(Mutex::new(Vec::new()));
    let rec = connects.clone();
    let connect = move |cursor: Option<ListenCursor>| -> BoxFuture<
        'static,
        anyhow::Result<BoxStream<'static, anyhow::Result<Value>>>,
    > {
        let n = {
            let mut c = rec.lock().unwrap();
            c.push(cursor);
            c.len()
        };
        async move {
            match n {
                1 => {}
                2 => {
                    // The source restarted: new epoch, fresh sequence space.
                    src.set_epoch(8);
                    *src.log.lock().unwrap() = Vec::new();
                    src.set_window(1, 2);
                }
                _ => return Err(denied()),
            }
            let frames = listen_stream(
                listener,
                src,
                gp(),
                cursor,
                Caller("follower".to_owned()),
                ListenOptions::default(),
            )
            .await?;
            Ok(frames
                .map(|f| f.and_then(|f| Ok(serde_json::to_value(f)?)))
                .boxed())
        }
        .boxed()
    };

    let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_lines = lines.clone();
    let sink = move |line: &str| {
        sink_lines.lock().unwrap().push(line.to_owned());
        Ok(())
    };
    let end = follow(connect, sink, FollowOptions::default()).await;
    let err = end.err().expect("third connect is refused logically");
    assert_eq!(code_of(&err), "test.app.denied");

    assert_eq!(
        *connects.lock().unwrap(),
        vec![None, Some(lc(7, 5)), Some(lc(8, 2))],
        "reconnect resumes from the last cursor"
    );
    let parsed: Vec<Value> = lines
        .lock()
        .unwrap()
        .iter()
        .map(|l| serde_json::from_str(l).expect("JSON line"))
        .collect();
    assert_eq!(parsed.len(), 5, "{parsed:#?}");
    assert_eq!(parsed[0]["frame"], "snapshot");
    assert_eq!(parsed[0]["resync"], true);
    assert_eq!(parsed[0]["cursor"], "7:3");
    assert_eq!(parsed[1]["frame"], "item");
    assert_eq!(parsed[1]["cursor"], "7:4");
    assert_eq!(parsed[2]["cursor"], "7:5");
    assert_eq!(
        parsed[3],
        json!({"marker": "reconnected", "cursor": "7:5", "resync": true})
    );
    assert_eq!(parsed[4]["frame"], "snapshot");
    assert_eq!(parsed[4]["resync"], true);
    assert_eq!(parsed[4]["cursor"], "8:2");
    assert_eq!(parsed[4]["snapshot"]["epoch"], 8);
}
