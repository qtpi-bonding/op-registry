mod common;

use common::{code_of, denied, transient};
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use op_listen::{
    supervise, supervise_stream, FollowEnd, FollowOptions, ListenCursor, ListenFrame, Resumable,
    StreamStep, Supervised,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
struct Tick {
    text: String,
    cursor: Option<String>,
    resync: bool,
    stop: Option<String>,
}

impl Resumable for Tick {
    type Cursor = String;
    fn cursor(&self) -> Option<String> {
        self.cursor.clone()
    }
    fn stop_code(&self) -> Option<&str> {
        self.stop.as_deref()
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
        stop: None,
    })
}

fn snapshot(text: &str, cursor: &str) -> StreamStep<Tick> {
    StreamStep::Item(Tick {
        text: text.to_owned(),
        cursor: Some(cursor.to_owned()),
        resync: true,
        stop: None,
    })
}

fn halt(code: &str) -> StreamStep<Tick> {
    StreamStep::Item(Tick {
        text: "halt".to_owned(),
        cursor: None,
        resync: false,
        stop: Some(code.to_owned()),
    })
}

enum Step {
    Steps(Vec<StreamStep<Tick>>),
    Mixed(Vec<anyhow::Result<StreamStep<Tick>>>),
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
                Step::Mixed(v) => Ok(futures_util::stream::iter(v).boxed()),
                Step::ConnectErr(e) => Err(e),
            }
        }
        .boxed()
    };
    (connect, cursors)
}

fn stopping(codes: &[&str]) -> FollowOptions<String> {
    let mut opts = FollowOptions::new();
    opts.stop_codes = codes.iter().map(|c| (*c).to_owned()).collect();
    opts
}

type Log = Arc<Mutex<Vec<String>>>;

fn log_sink() -> (impl FnMut(Supervised<Tick>) -> std::io::Result<()>, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let rec = log.clone();
    (
        move |event: Supervised<Tick>| {
            let line = match event {
                Supervised::Opened => "opened".to_owned(),
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
    let end = supervise::<Tick, _, _>(connect, stopping(&[stop]), sink)
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
            "opened".to_owned(),
            "item:a".to_owned(),
            "item:b".to_owned(),
            "retrying:op_listen.session_expired".to_owned(),
            "opened".to_owned(),
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
    let end = supervise::<Tick, _, _>(connect, stopping(&["other.code", stop]), sink)
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
    let err = supervise::<Tick, _, _>(connect, stopping(&["other.code"]), sink)
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
    supervise::<Tick, _, _>(connect, stopping(&[stop]), sink)
        .await
        .expect("ends with the stop");
    assert_eq!(cursors.lock().unwrap().len(), 3);
    let transient_code = code_of(&transient());
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            format!("retrying:{transient_code}"),
            "opened".to_owned(),
            "item:a".to_owned(),
            "retrying:op_listen.ended".to_owned(),
            format!("stopped:{stop}"),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_quiet_connection_reports_opened() {
    let (connect, _cursors) = scripted(vec![
        Step::Steps(vec![StreamStep::Ended]),
        Step::ConnectErr(denied()),
    ]);
    let (sink, log) = log_sink();
    let stop = code_of(&denied());
    supervise::<Tick, _, _>(connect, stopping(&[stop]), sink)
        .await
        .expect("ends with the stop");
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            "opened".to_owned(),
            "retrying:op_listen.ended".to_owned(),
            format!("stopped:{stop}"),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn opened_comes_before_the_first_item_of_each_connection() {
    let (connect, _cursors) = scripted(vec![
        Step::Steps(vec![tick("a", "c1"), StreamStep::Ended]),
        Step::Steps(vec![snapshot("s", "c9"), tick("b", "c10")]),
        Step::ConnectErr(denied()),
    ]);
    let (sink, log) = log_sink();
    let stop = code_of(&denied());
    supervise::<Tick, _, _>(connect, stopping(&[stop]), sink)
        .await
        .expect("ends with the stop");
    let log = log.lock().unwrap();
    assert_eq!(log[0], "opened");
    assert_eq!(log[1], "item:a");
    let second = log.iter().rposition(|l| l == "opened").unwrap();
    assert_eq!(log[second + 1], "item:s");
}

#[tokio::test(start_paused = true)]
async fn stop_item_delivered_once_as_stopped() {
    let (connect, cursors) = scripted(vec![
        Step::Steps(vec![tick("a", "c1"), halt("op_listen.revoked")]),
        Step::Steps(vec![tick("never", "c9")]),
    ]);
    let (sink, log) = log_sink();
    let end = supervise::<Tick, _, _>(connect, FollowOptions::new(), sink)
        .await
        .expect("ends cleanly");
    assert_eq!(
        end,
        FollowEnd::Stopped {
            code: "op_listen.revoked".to_owned()
        }
    );
    assert_eq!(cursors.lock().unwrap().len(), 1);
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            "opened".to_owned(),
            "item:a".to_owned(),
            "stopped:op_listen.revoked".to_owned(),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn listed_mid_stream_code_ends_stopped() {
    let stop = code_of(&denied());
    let (connect, cursors) = scripted(vec![
        Step::Mixed(vec![Ok(tick("a", "c1")), Err(denied())]),
        Step::Steps(vec![tick("never", "c9")]),
    ]);
    let (sink, log) = log_sink();
    let end = supervise::<Tick, _, _>(connect, stopping(&[stop]), sink)
        .await
        .expect("a listed code ends with Ok");
    assert_eq!(
        end,
        FollowEnd::Stopped {
            code: stop.to_owned()
        }
    );
    assert_eq!(cursors.lock().unwrap().len(), 1);
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            "opened".to_owned(),
            "item:a".to_owned(),
            format!("stopped:{stop}"),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn unlisted_logical_ends_err() {
    let (connect, cursors) = scripted(vec![
        Step::Mixed(vec![Ok(tick("a", "c1")), Err(denied())]),
        Step::Steps(vec![tick("never", "c9")]),
    ]);
    let (sink, log) = log_sink();
    let err = supervise::<Tick, _, _>(connect, stopping(&["other.code"]), sink)
        .await
        .expect_err("unlisted logical error ends the follow");
    assert_eq!(code_of(&err), code_of(&denied()));
    assert_eq!(cursors.lock().unwrap().len(), 1);
    assert_eq!(
        *log.lock().unwrap(),
        vec!["opened".to_owned(), "item:a".to_owned()]
    );
}

#[tokio::test(start_paused = true)]
async fn unlisted_transient_retries() {
    let (connect, cursors) = scripted(vec![
        Step::Mixed(vec![Ok(tick("a", "c1")), Err(transient())]),
        Step::Steps(vec![tick("b", "c2"), halt("done")]),
    ]);
    let (sink, log) = log_sink();
    let end = supervise::<Tick, _, _>(connect, stopping(&["other.code"]), sink)
        .await
        .expect("retries then stops");
    assert_eq!(
        end,
        FollowEnd::Stopped {
            code: "done".to_owned()
        }
    );
    assert_eq!(*cursors.lock().unwrap(), vec![None, Some("c1".to_owned())]);
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            "opened".to_owned(),
            "item:a".to_owned(),
            format!("retrying:{}", code_of(&transient())),
            "opened".to_owned(),
            "item:b".to_owned(),
            "stopped:done".to_owned(),
        ]
    );
}

type Frame = ListenFrame<String, String>;
type PlainConn = BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Frame>>>>;

fn lc(epoch: u64, seq: u64) -> ListenCursor {
    format!("{epoch}:{seq}").parse().expect("cursor")
}

#[tokio::test(start_paused = true)]
async fn supervise_stream_delivers_opened_and_retrying() {
    let calls: Arc<Mutex<Vec<Option<ListenCursor>>>> = Arc::new(Mutex::new(Vec::new()));
    let rec = calls.clone();
    let mut script: VecDeque<Vec<anyhow::Result<Frame>>> = VecDeque::from(vec![
        vec![Ok(Frame::Item {
            item: "a".to_owned(),
            cursor: lc(0, 1),
        })],
        vec![Ok(Frame::Stop {
            code: "op_listen.revoked".to_owned(),
            message: "gone".to_owned(),
        })],
    ]);
    let connect = move |cursor: Option<ListenCursor>| -> PlainConn {
        rec.lock().unwrap().push(cursor);
        let items = script.pop_front().expect("scripted");
        async move { Ok(futures_util::stream::iter(items).boxed()) }.boxed()
    };
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let rec = log.clone();
    let end = supervise_stream::<Frame, _, _>(connect, FollowOptions::default(), move |event| {
        let line = match event {
            Supervised::Opened => "opened".to_owned(),
            Supervised::Item(Frame::Item { item, .. }) => format!("item:{item}"),
            Supervised::Item(_) => "item:other".to_owned(),
            Supervised::Retrying { code } => format!("retrying:{code}"),
            Supervised::Stopped { code } => format!("stopped:{code}"),
        };
        rec.lock().unwrap().push(line);
        Ok(())
    })
    .await
    .expect("ends cleanly");
    assert_eq!(
        end,
        FollowEnd::Stopped {
            code: "op_listen.revoked".to_owned()
        }
    );
    assert_eq!(*calls.lock().unwrap(), vec![None, Some(lc(0, 1))]);
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            "opened".to_owned(),
            "item:a".to_owned(),
            "retrying:op_listen.ended".to_owned(),
            "opened".to_owned(),
            "stopped:op_listen.revoked".to_owned(),
        ]
    );
}
