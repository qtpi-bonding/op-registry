mod common;

use common::{code_of, denied, transient};
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use op_listen::{follow_items, FollowEnd, FollowItem, FollowOptions, Resumable};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

#[derive(Clone, Debug, PartialEq)]
enum Tick {
    Msg {
        text: String,
        cursor: Option<String>,
        resync: bool,
    },
    Halt {
        code: String,
    },
}

impl Resumable for Tick {
    type Cursor = String;
    fn cursor(&self) -> Option<String> {
        match self {
            Tick::Msg { cursor, .. } => cursor.clone(),
            Tick::Halt { .. } => None,
        }
    }
    fn stop_code(&self) -> Option<&str> {
        match self {
            Tick::Halt { code } => Some(code),
            _ => None,
        }
    }
    fn resync(&self) -> bool {
        matches!(self, Tick::Msg { resync: true, .. })
    }
}

fn msg(text: &str, cursor: &str) -> Tick {
    Tick::Msg {
        text: text.to_owned(),
        cursor: Some(cursor.to_owned()),
        resync: false,
    }
}

fn halt(code: &str) -> Tick {
    Tick::Halt {
        code: code.to_owned(),
    }
}

enum Step {
    Items(Vec<anyhow::Result<Tick>>),
    ConnectErr(anyhow::Error),
}

fn items(v: Vec<Tick>) -> Step {
    Step::Items(v.into_iter().map(Ok).collect())
}

type Connects = Arc<Mutex<Vec<(Option<String>, Instant)>>>;
type Conn = BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Tick>>>>;

fn scripted_items(steps: Vec<Step>) -> (impl FnMut(Option<String>) -> Conn, Connects) {
    let steps = Arc::new(Mutex::new(VecDeque::from(steps)));
    let connects: Connects = Arc::new(Mutex::new(Vec::new()));
    let rec = connects.clone();
    let connect = move |cursor: Option<String>| -> Conn {
        rec.lock().unwrap().push((cursor, Instant::now()));
        let step = steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("connect called more often than scripted");
        async move {
            match step {
                Step::Items(v) => Ok(futures_util::stream::iter(v).boxed()),
                Step::ConnectErr(e) => Err(e),
            }
        }
        .boxed()
    };
    (connect, connects)
}

/// Sink events flattened to strings so the tests do not depend on derives.
type Log = Arc<Mutex<Vec<String>>>;

fn log_sink() -> (impl FnMut(FollowItem<Tick>) -> std::io::Result<()>, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let rec = log.clone();
    (
        move |event: FollowItem<Tick>| {
            let line = match event {
                FollowItem::Item(Tick::Msg { text, .. }) => format!("item:{text}"),
                FollowItem::Item(Tick::Halt { code }) => format!("item-halt:{code}"),
                FollowItem::Reconnected { cursor, resync } => {
                    format!("reconnected:{}:{resync}", cursor.unwrap_or_default())
                }
                FollowItem::Stopped { code } => format!("stopped:{code}"),
            };
            rec.lock().unwrap().push(line);
            Ok(())
        },
        log,
    )
}

fn cursors(connects: &Connects) -> Vec<Option<String>> {
    connects
        .lock()
        .unwrap()
        .iter()
        .map(|(c, _)| c.clone())
        .collect()
}

#[tokio::test(start_paused = true)]
async fn follow_items_resumes_after_the_last_cursor() {
    let (connect, connects) = scripted_items(vec![
        Step::Items(vec![
            Ok(msg("a", "c1")),
            Ok(msg("b", "c2")),
            Err(transient()),
        ]),
        items(vec![msg("c", "c3"), halt("done")]),
    ]);
    let (sink, _log) = log_sink();
    follow_items::<Tick, _, _>(connect, sink, FollowOptions::new())
        .await
        .expect("ends cleanly");
    assert_eq!(cursors(&connects), vec![None, Some("c2".to_owned())]);
}

#[tokio::test(start_paused = true)]
async fn a_clean_end_reconnects_from_the_cursor() {
    let (connect, connects) = scripted_items(vec![
        items(vec![msg("a", "c1")]),
        items(vec![msg("b", "c2"), halt("done")]),
    ]);
    let (sink, _log) = log_sink();
    follow_items::<Tick, _, _>(connect, sink, FollowOptions::new())
        .await
        .expect("ends cleanly");
    assert_eq!(cursors(&connects), vec![None, Some("c1".to_owned())]);
}

#[tokio::test(start_paused = true)]
async fn a_stop_item_ends_without_reconnecting() {
    let (connect, connects) = scripted_items(vec![
        items(vec![msg("a", "c1"), halt("op_listen.revoked")]),
        items(vec![msg("never", "c9")]),
    ]);
    let (sink, log) = log_sink();
    let end = follow_items::<Tick, _, _>(connect, sink, FollowOptions::new())
        .await
        .expect("ends cleanly");
    assert!(matches!(end, FollowEnd::Stopped { ref code } if code == "op_listen.revoked"));
    assert_eq!(connects.lock().unwrap().len(), 1);
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            "item:a".to_owned(),
            "item-halt:op_listen.revoked".to_owned(),
            "stopped:op_listen.revoked".to_owned(),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_transient_error_backs_off_and_reconnects() {
    let (connect, connects) = scripted_items(vec![
        Step::Items(vec![Ok(msg("a", "c1")), Err(transient())]),
        items(vec![msg("b", "c2"), halt("done")]),
    ]);
    let (sink, _log) = log_sink();
    follow_items::<Tick, _, _>(connect, sink, FollowOptions::new())
        .await
        .expect("ends cleanly");
    let calls = connects.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, Some("c1".to_owned()));
    assert!(
        calls[1].1 > calls[0].1,
        "a backoff delay passed between connects"
    );
}

#[tokio::test(start_paused = true)]
async fn a_logical_error_ends_the_follow() {
    let (connect, connects) = scripted_items(vec![
        Step::Items(vec![Ok(msg("a", "c1")), Err(denied())]),
        items(vec![halt("done")]),
    ]);
    let (sink, _log) = log_sink();
    let err = follow_items::<Tick, _, _>(connect, sink, FollowOptions::new())
        .await
        .expect_err("logical error ends the follow");
    assert_eq!(code_of(&err), "test.app.denied");
    assert_eq!(connects.lock().unwrap().len(), 1);

    let (connect, connects) = scripted_items(vec![Step::ConnectErr(denied()), items(vec![])]);
    let (sink, _log) = log_sink();
    let err = follow_items::<Tick, _, _>(connect, sink, FollowOptions::new())
        .await
        .expect_err("logical connect error ends the follow");
    assert_eq!(code_of(&err), "test.app.denied");
    assert_eq!(connects.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn items_reach_the_sink_in_order_with_no_gap_or_duplicate() {
    let mut resynced = msg("c", "c3");
    if let Tick::Msg { resync, .. } = &mut resynced {
        *resync = true;
    }
    let (connect, connects) = scripted_items(vec![
        Step::Items(vec![
            Ok(msg("a", "c1")),
            Ok(msg("b", "c2")),
            Err(transient()),
        ]),
        items(vec![resynced, msg("d", "c4"), halt("done")]),
    ]);
    let (sink, log) = log_sink();
    let mut opts = FollowOptions::new();
    opts.initial_cursor = None;
    follow_items::<Tick, _, _>(connect, sink, opts)
        .await
        .expect("ends cleanly");
    assert_eq!(cursors(&connects), vec![None, Some("c2".to_owned())]);
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            "item:a".to_owned(),
            "item:b".to_owned(),
            "reconnected:c2:true".to_owned(),
            "item:c".to_owned(),
            "item:d".to_owned(),
            "item-halt:done".to_owned(),
            "stopped:done".to_owned(),
        ]
    );
}
