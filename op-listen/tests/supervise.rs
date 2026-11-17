mod common;

use common::{code_of, denied, transient};
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use op_listen::{supervise, FollowEnd, FollowOptions, Resumable, StreamStep, Supervised};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
struct Tick {
    text: String,
    cursor: Option<String>,
    resync: bool,
}

impl Resumable for Tick {
    type Cursor = String;
    fn cursor(&self) -> Option<String> {
        self.cursor.clone()
    }
    fn resync(&self) -> bool {
        self.resync
    }
}

fn tick(text: &str, cursor: &str) -> StreamStep<Tick> {
    StreamStep::Item(Tick {
        text: text.to_owned(),
        cursor: Some(cursor.to_owned()),
        resync: false,
    })
}

fn snapshot(text: &str, cursor: &str) -> StreamStep<Tick> {
    StreamStep::Item(Tick {
        text: text.to_owned(),
        cursor: Some(cursor.to_owned()),
        resync: true,
    })
}

enum Step {
    Steps(Vec<StreamStep<Tick>>),
    ConnectErr(anyhow::Error),
}

type Conn =
    BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<StreamStep<Tick>>>>>;
type Cursors = Arc<Mutex<Vec<Option<String>>>>;

fn scripted(steps: Vec<Step>) -> (impl FnMut(Option<String>) -> Conn, Cursors) {
    let steps = Arc::new(Mutex::new(VecDeque::from(steps)));
    let cursors: Cursors = Arc::new(Mutex::new(Vec::new()));
    let rec = cursors.clone();
    let connect = move |cursor: Option<String>| -> Conn {
        rec.lock().unwrap().push(cursor);
        let step = steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("connect called more often than scripted");
        async move {
            match step {
                Step::Steps(v) => Ok(futures_util::stream::iter(v.into_iter().map(Ok)).boxed()),
                Step::ConnectErr(e) => Err(e),
            }
        }
        .boxed()
    };
    (connect, cursors)
}

type Log = Arc<Mutex<Vec<String>>>;

fn log_sink() -> (impl FnMut(Supervised<Tick>) -> std::io::Result<()>, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let rec = log.clone();
    (
        move |event: Supervised<Tick>| {
            let line = match event {
                Supervised::Opened { resync } => format!("opened:{resync}"),
                Supervised::Item(t) => format!("item:{}", t.text),
                Supervised::Retrying { code } => format!("retrying:{code}"),
                Supervised::Stopped { code } => format!("stopped:{code}"),
            };
            rec.lock().unwrap().push(line);
            Ok(())
        },
        log,
    )
}

#[tokio::test(start_paused = true)]
async fn session_expired_reopens_from_the_cursor() {
    let (connect, cursors) = scripted(vec![
        Step::Steps(vec![
            tick("a", "c1"),
            tick("b", "c2"),
            StreamStep::SessionExpired,
        ]),
        Step::Steps(vec![tick("c", "c3"), StreamStep::Ended]),
        Step::ConnectErr(denied()),
    ]);
    let (sink, log) = log_sink();
    let stop = code_of(&denied());
    let end = supervise::<Tick, _, _>(connect, FollowOptions::new(), &[stop], sink)
        .await
        .expect("ends cleanly");
    assert!(matches!(end, FollowEnd::Stopped { ref code } if code == stop));
    assert_eq!(
        *cursors.lock().unwrap(),
        vec![None, Some("c2".to_owned()), Some("c3".to_owned())]
    );
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            "opened:false".to_owned(),
            "item:a".to_owned(),
            "item:b".to_owned(),
            "retrying:op_listen.session_expired".to_owned(),
            "opened:false".to_owned(),
            "item:c".to_owned(),
            "retrying:op_listen.ended".to_owned(),
            format!("stopped:{stop}"),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn an_open_refused_with_a_stop_code_stops() {
    let (connect, cursors) = scripted(vec![Step::ConnectErr(denied())]);
    let (sink, log) = log_sink();
    let stop = code_of(&denied());
    let end = supervise::<Tick, _, _>(connect, FollowOptions::new(), &["other.code", stop], sink)
        .await
        .expect("a stop code ends with Ok");
    assert_eq!(
        end,
        FollowEnd::Stopped {
            code: stop.to_owned()
        }
    );
    assert_eq!(cursors.lock().unwrap().len(), 1, "no reopen after a stop");
    assert_eq!(*log.lock().unwrap(), vec![format!("stopped:{stop}")]);
}

#[tokio::test(start_paused = true)]
async fn a_logical_open_error_outside_the_stop_codes_is_returned() {
    let (connect, _cursors) = scripted(vec![Step::ConnectErr(denied())]);
    let (sink, log) = log_sink();
    let err = supervise::<Tick, _, _>(connect, FollowOptions::new(), &["other.code"], sink)
        .await
        .expect_err("not a stop code");
    assert_eq!(code_of(&err), code_of(&denied()));
    assert!(log.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_transient_open_error_retries_with_its_code() {
    let (connect, cursors) = scripted(vec![
        Step::ConnectErr(transient()),
        Step::Steps(vec![tick("a", "c1")]),
        Step::ConnectErr(denied()),
    ]);
    let (sink, log) = log_sink();
    let stop = code_of(&denied());
    supervise::<Tick, _, _>(connect, FollowOptions::new(), &[stop], sink)
        .await
        .expect("ends with the stop");
    assert_eq!(cursors.lock().unwrap().len(), 3);
    let transient_code = code_of(&transient());
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            format!("retrying:{transient_code}"),
            "opened:false".to_owned(),
            "item:a".to_owned(),
            "retrying:op_listen.ended".to_owned(),
            format!("stopped:{stop}"),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn opened_reports_resync() {
    let (connect, _cursors) = scripted(vec![
        Step::Steps(vec![tick("a", "c1"), StreamStep::Ended]),
        Step::Steps(vec![snapshot("s", "c9"), tick("b", "c10")]),
        Step::ConnectErr(denied()),
    ]);
    let (sink, log) = log_sink();
    let stop = code_of(&denied());
    supervise::<Tick, _, _>(connect, FollowOptions::new(), &[stop], sink)
        .await
        .expect("ends with the stop");
    let log = log.lock().unwrap();
    let opened: Vec<&String> = log.iter().filter(|l| l.starts_with("opened")).collect();
    assert_eq!(opened, ["opened:false", "opened:true"]);
    // Opened comes before the first item of its connection.
    assert_eq!(log[0], "opened:false");
    let second = log.iter().position(|l| l == "opened:true").unwrap();
    assert_eq!(log[second + 1], "item:s");
}
