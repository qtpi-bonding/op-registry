//! Scripted connect and recording sink for the follower tests.

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use op_listen::ListenCursor;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

pub fn item(text: &str, cursor: &str) -> Value {
    json!({"frame": "item", "item": text, "cursor": cursor})
}
pub fn snap(resync: bool, cursor: &str) -> Value {
    json!({"frame": "snapshot", "snapshot": {"k": "v"}, "resync": resync, "cursor": cursor})
}
pub fn stop(code: &str) -> Value {
    json!({"frame": "stop", "code": code, "message": "gone"})
}
pub fn marker(cursor: Value, resync: bool) -> Value {
    json!({"marker": "reconnected", "cursor": cursor, "resync": resync})
}

pub enum Step {
    Frames(Vec<anyhow::Result<Value>>),
    ConnectErr(anyhow::Error),
}

pub fn frames(vs: Vec<Value>) -> Step {
    Step::Frames(vs.into_iter().map(Ok).collect())
}

pub type Connects = Arc<Mutex<Vec<(Option<ListenCursor>, Instant)>>>;
pub type Conn = BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Value>>>>;

pub fn scripted(steps: Vec<Step>) -> (impl FnMut(Option<ListenCursor>) -> Conn, Connects) {
    let steps = Arc::new(Mutex::new(VecDeque::from(steps)));
    let connects: Connects = Arc::new(Mutex::new(Vec::new()));
    let rec = connects.clone();
    let connect = move |cursor: Option<ListenCursor>| -> Conn {
        rec.lock().unwrap().push((cursor, Instant::now()));
        let step = steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("connect called more often than scripted");
        async move {
            match step {
                Step::Frames(items) => Ok(futures_util::stream::iter(items).boxed()),
                Step::ConnectErr(e) => Err(e),
            }
        }
        .boxed()
    };
    (connect, connects)
}

pub type Lines = Arc<Mutex<Vec<String>>>;

pub fn make_sink() -> (impl FnMut(&str) -> std::io::Result<()>, Lines) {
    let lines: Lines = Arc::new(Mutex::new(Vec::new()));
    let rec = lines.clone();
    (
        move |line: &str| {
            rec.lock().unwrap().push(line.to_owned());
            Ok(())
        },
        lines,
    )
}
