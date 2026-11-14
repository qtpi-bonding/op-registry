#![allow(clippy::err_expect, clippy::ok_expect)]
mod common;

use common::*;
use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream};
use futures_util::{FutureExt, StreamExt};
use op_listen::{
    detached_connect, follow, listen_entry, FollowEnd, FollowOptions, Grant, ListenCursor,
    Listener, Opened,
};
use op_registry::{Caller, Descriptor, Registry};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

fn args() -> Value {
    json!({"topic": "t"})
}

fn setup(listener: LogListener) -> (Arc<Registry<Ctx>>, Arc<Ctx>, Arc<LogListener>) {
    let l = Arc::new(listener);
    let mut registry: Registry<Ctx> = Registry::new();
    listen_entry(
        &mut registry,
        l.clone(),
        Descriptor::new("log", "listen").description("listens to the log"),
    );
    (Arc::new(registry), Arc::new(Ctx), l)
}

fn connector(
    registry: Arc<Registry<Ctx>>,
    ctx: Arc<Ctx>,
) -> impl FnMut(
    Option<ListenCursor>,
) -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Value>>>> {
    detached_connect(registry, ctx, "log_listen".to_owned(), args(), caller("alice"))
}

fn sink_to(lines: &Arc<Mutex<Vec<String>>>) -> impl FnMut(&str) -> std::io::Result<()> {
    let lines = lines.clone();
    move |line: &str| {
        lines.lock().unwrap().push(line.to_owned());
        Ok(())
    }
}

fn small_backoff() -> FollowOptions {
    let mut o = FollowOptions::default();
    o.backoff_first = Duration::from_millis(1);
    o.backoff_max = Duration::from_millis(2);
    o
}

/// Fires its oneshot when dropped.
struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);
impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

/// A source that never yields and owns `guard` until the stream is dropped.
fn pending_with_guard(guard: DropSignal) -> Source {
    stream::poll_fn(move |_cx| {
        let _keep = &guard;
        Poll::<Option<anyhow::Result<(u64, String)>>>::Pending
    })
    .boxed()
}

/// Listener whose every `open` consumes the next scripted source (an empty
/// source when none is left) and records the cursor it was opened with.
struct Flaky {
    plans: Mutex<VecDeque<Source>>,
    opens: Mutex<Vec<Option<ListenCursor>>>,
}

impl Listener<Ctx> for Flaky {
    type Params = TopicParams;
    type Snapshot = ();
    type Item = String;

    fn topic(&self, params: &Self::Params) -> String {
        params.topic.clone()
    }
    fn epoch(&self) -> u64 {
        0
    }
    fn authorize<'a>(
        &'a self,
        _ctx: &'a Ctx,
        _caller: &'a Caller,
        _topic: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<Grant>> {
        async { Ok(Grant::default()) }.boxed()
    }
    fn open<'a>(
        &'a self,
        _ctx: &'a Ctx,
        _params: Self::Params,
        cursor: Option<ListenCursor>,
        _floor: Option<ListenCursor>,
    ) -> BoxFuture<'a, anyhow::Result<Opened<'a, Self::Snapshot, Self::Item>>> {
        async move {
            self.opens.lock().unwrap().push(cursor);
            let next = self.plans.lock().unwrap().pop_front();
            Ok(Opened::replay(next.unwrap_or_else(|| source(vec![]))))
        }
        .boxed()
    }
}

#[tokio::test]
async fn detached_stream_delivers_frames_in_order_and_ends_with_the_source() {
    let (registry, ctx, l) = setup(LogListener::new(vec![(1, "a"), (2, "b"), (3, "c")]));
    let mut connect = connector(registry, ctx);

    let s = connect(None).await.ok().expect("connect");
    let got = frames_json(s).await;
    assert_eq!(
        got,
        vec![
            json!({"frame": "item", "item": "a", "cursor": "0:1"}),
            json!({"frame": "item", "item": "b", "cursor": "0:2"}),
            json!({"frame": "item", "item": "c", "cursor": "0:3"}),
        ]
    );

    // The closure is reusable and passes the cursor through.
    let s = connect(Some(lc(0, 2))).await.ok().expect("reconnect");
    let got = frames_json(s).await;
    assert_eq!(
        got,
        vec![json!({"frame": "item", "item": "c", "cursor": "0:3"})]
    );
    assert_eq!(l.open_calls(), vec![(None, None), (Some(lc(0, 2)), None)]);
    assert_eq!(l.authorize_calls()[0], ("alice".to_owned(), "t".to_owned()));
}

#[tokio::test]
async fn detached_connect_returns_the_authorization_refusal_as_a_logical_error_not_a_stream_item() {
    // Direct: the connect future itself is the error.
    let mut refusing = LogListener::new(vec![(1, "a")]);
    refusing.refuse = true;
    let (registry, ctx, l) = setup(refusing);
    let mut connect = connector(registry, ctx);
    let err = connect(None)
        .await
        .err()
        .expect("refusal must be an Err, not a stream");
    let facts = coded_error::facts_anyhow(&err);
    assert_eq!(facts.code, "test.app.denied");
    assert_eq!(facts.class, coded_error::ErrorClass::Logical);
    assert_eq!(l.authorize_calls().len(), 1);
    assert!(l.open_calls().is_empty());

    // Through follow: returned at once, no retry.
    let mut refusing = LogListener::new(vec![(1, "a")]);
    refusing.refuse = true;
    let (registry, ctx, l) = setup(refusing);
    let mut inner = connector(registry, ctx);
    let connects = Arc::new(AtomicUsize::new(0));
    let counter = connects.clone();
    let connect = move |cursor: Option<ListenCursor>| {
        counter.fetch_add(1, Ordering::SeqCst);
        inner(cursor)
    };
    let lines = Arc::new(Mutex::new(Vec::new()));
    let err = follow(connect, sink_to(&lines), small_backoff())
        .await
        .err()
        .expect("logical error ends follow");
    assert_eq!(code_of(&err), "test.app.denied");
    assert_eq!(connects.load(Ordering::SeqCst), 1);
    assert_eq!(l.authorize_calls().len(), 1);
    assert!(lines.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn follow_over_detached_connect_reconnects_with_the_last_cursor_after_a_transient_failure() {
    let flaky = Arc::new(Flaky {
        plans: Mutex::new(VecDeque::from([
            source(vec![ok(1, "a"), ok(2, "b"), Err(transient())]),
            source(vec![ok(3, "c")]),
        ])),
        opens: Mutex::new(Vec::new()),
    });
    let mut registry: Registry<Ctx> = Registry::new();
    listen_entry(&mut registry, flaky.clone(), Descriptor::new("log", "listen"));
    let connect = connector(Arc::new(registry), Arc::new(Ctx));

    let lines = Arc::new(Mutex::new(Vec::new()));
    let mut opts = small_backoff();
    // Connection 3 (a scripted-out, empty source) exhausts the budget and ends the run.
    opts.max_retries = Some(1);
    let res = follow(connect, sink_to(&lines), opts).await;
    assert!(res.is_err(), "run ends by exhausting max_retries");

    let lines: Vec<Value> = lines
        .lock()
        .unwrap()
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        lines,
        vec![
            json!({"frame": "item", "item": "a", "cursor": "0:1"}),
            json!({"frame": "item", "item": "b", "cursor": "0:2"}),
            json!({"marker": "reconnected", "cursor": "0:2", "resync": false}),
            json!({"frame": "item", "item": "c", "cursor": "0:3"}),
        ]
    );
    let opens = flaky.opens.lock().unwrap().clone();
    assert_eq!(opens[0], None);
    assert_eq!(opens[1], Some(lc(0, 2)), "second open gets the last cursor");
    assert_eq!(opens[2], Some(lc(0, 3)));
    assert_eq!(opens.len(), 3);
}

#[tokio::test]
async fn dropping_the_detached_stream_aborts_the_task_and_closes_the_source() {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tail = pending_with_guard(DropSignal(Some(tx)));
    let (registry, ctx, _l) = setup(LogListener::new(vec![]).with_tail(tail));
    let mut connect = connector(registry, ctx);
    let mut rx = rx;

    let s = connect(None).await.ok().expect("connect");
    assert!(
        rx.try_recv().is_err(),
        "source must stay open while the stream lives"
    );
    drop(s);
    tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .expect("source must be dropped after the detached stream is dropped")
        .expect("guard sends exactly once");
}

#[tokio::test]
async fn a_slow_reader_applies_backpressure_not_unbounded_memory() {
    const TOTAL: u64 = 10_000;
    const CAPACITY: usize = 64;
    // Channel capacity plus the one item the forwarding task has pulled from the
    // source and is blocked sending. The toy path through listen_stream is a lazy
    // unfold that pulls nothing ahead, so no further slack is needed.
    const BOUND: usize = CAPACITY + 1;

    let produced = Arc::new(AtomicUsize::new(0));
    let p = produced.clone();
    let tail = stream::iter((1..=TOTAL).map(move |i| {
        p.fetch_add(1, Ordering::SeqCst);
        ok(i, "x")
    }))
    .boxed();
    let (registry, ctx, _l) = setup(LogListener::new(vec![]).with_tail(tail));
    let mut connect = connector(registry, ctx);
    let mut s = connect(None).await.ok().expect("connect");

    let settle = || async {
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    };

    // Let the producer run as far as it can before anything is read.
    settle().await;
    let before = produced.load(Ordering::SeqCst);
    assert!(
        before >= CAPACITY,
        "producer should fill the buffer (got {before}); test would be vacuous otherwise"
    );
    assert!(before <= BOUND, "unread backlog {before} exceeds {BOUND}");

    let mut consumed = 0usize;
    while consumed < 300 {
        let item = s.next().await.expect("more items").expect("ok item");
        consumed += 1;
        assert_eq!(item["cursor"], format!("0:{consumed}"));
        let now = produced.load(Ordering::SeqCst);
        assert!(now - consumed <= BOUND, "backlog {} > {BOUND}", now - consumed);
        settle().await;
        let now = produced.load(Ordering::SeqCst);
        assert!(now - consumed <= BOUND, "backlog {} > {BOUND}", now - consumed);
    }
    assert!(produced.load(Ordering::SeqCst) < TOTAL as usize);
}

#[tokio::test(start_paused = true)]
async fn a_stop_frame_from_the_gate_ends_follow_without_reconnect() {
    let mut l = LogListener::new(vec![(1, "a")])
        .with_grant(grant(None, Some(Duration::from_secs(10))));
    l.tail_pending = true;
    l.allowed.store(false, Ordering::SeqCst);
    let (registry, ctx, l) = setup(l);
    let connect = connector(registry, ctx);

    let lines = Arc::new(Mutex::new(Vec::new()));
    let end = tokio::time::timeout(
        Duration::from_secs(60),
        follow(connect, sink_to(&lines), small_backoff()),
    )
    .await
    .expect("follow must end on the stop frame")
    .expect("stop is a clean end");
    assert_eq!(
        end,
        FollowEnd::Stopped {
            code: "op_listen.revoked".to_owned()
        }
    );
    assert_eq!(l.open_calls().len(), 1, "no reconnect after a stop frame");
    let lines = lines.lock().unwrap().clone();
    assert!(lines.iter().any(|l| l.contains("\"stop\"")));
    assert!(!lines.iter().any(|l| l.contains("reconnected")));
}

#[tokio::test]
async fn a_panic_in_the_forwarding_task_surfaces_as_an_infrastructure_error_item() {
    let tail = stream::poll_fn(|_cx| -> Poll<Option<anyhow::Result<(u64, String)>>> {
        panic!("toy source exploded")
    })
    .boxed();
    let (registry, ctx, _l) = setup(LogListener::new(vec![]).with_tail(tail));
    let mut connect = connector(registry, ctx);
    let mut s = connect(None).await.ok().expect("connect succeeds before the panic");

    let first = tokio::time::timeout(Duration::from_secs(5), s.next())
        .await
        .expect("must not hang")
        .expect("an error item, not silent end");
    let err = first.err().expect("panic is an Err item");
    let facts = coded_error::facts_anyhow(&err);
    assert_eq!(facts.code, "op_listen.task_panicked");
    assert_eq!(facts.class, coded_error::ErrorClass::Infrastructure);

    let end = tokio::time::timeout(Duration::from_secs(5), s.next())
        .await
        .expect("must not hang");
    assert!(end.is_none(), "stream ends after the panic item");
}
