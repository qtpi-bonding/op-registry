#![allow(dead_code)]
//! Toy listeners shared by the op-listen integration tests.

use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream};
use futures_util::{FutureExt, StreamExt};
use op_listen::{Grant, ListenCursor, Listener, Opened, OverflowPolicy};
use op_registry::{Caller, OpParams};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub struct Ctx;

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.app")]
pub enum AppError {
    #[error("not yours")]
    #[coded(code = "denied", class = Logical)]
    Denied,
    #[error("try again")]
    #[coded(code = "transient", class = Transient)]
    Transient,
}

pub fn denied() -> anyhow::Error {
    coded_error::into_anyhow(AppError::Denied)
}

pub fn transient() -> anyhow::Error {
    coded_error::into_anyhow(AppError::Transient)
}

pub fn code_of(err: &anyhow::Error) -> &'static str {
    coded_error::facts_anyhow(err).code
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, OpParams)]
pub struct TopicParams {
    #[op(positional)]
    pub topic: String,
}

pub fn params(topic: &str) -> TopicParams {
    TopicParams {
        topic: topic.to_owned(),
    }
}

pub fn caller(name: &str) -> Caller {
    Caller(name.to_owned())
}

pub fn lc(epoch: u64, seq: u64) -> ListenCursor {
    ListenCursor::new(epoch, seq)
}

pub fn grant(not_before: Option<ListenCursor>, recheck_every: Option<Duration>) -> Grant {
    let mut g = Grant::default();
    g.not_before = not_before;
    g.recheck_every = recheck_every;
    g
}

pub type Source = BoxStream<'static, anyhow::Result<(u64, String)>>;

pub fn source(items: Vec<anyhow::Result<(u64, String)>>) -> Source {
    stream::iter(items).boxed()
}

pub fn ok(seq: u64, text: &str) -> anyhow::Result<(u64, String)> {
    Ok((seq, text.to_owned()))
}

pub type OpenCall = (Option<ListenCursor>, Option<ListenCursor>);

/// A replay listener over an in-memory log, epoch 0.
pub struct LogListener {
    pub log: Vec<(u64, String)>,
    pub grant: Grant,
    /// When set, `authorize` fails with the app's coded `Denied` error.
    pub refuse: bool,
    pub allowed: AtomicBool,
    pub allowed_delay: Option<Duration>,
    /// Replay the whole log regardless of the cursor passed to `open`.
    pub replay_all: bool,
    /// After the replay (and the optional live tail) never end.
    pub tail_pending: bool,
    /// Live source chained after the replay, taken by the first `open`.
    pub tail: Mutex<Option<Source>>,
    pub authorizes: Mutex<Vec<(String, String)>>,
    pub still_allowed_calls: AtomicUsize,
    pub still_allowed_args: Mutex<Vec<(String, String)>>,
    pub opens: Mutex<Vec<OpenCall>>,
    /// Order of "authorize" / "open" events.
    pub events: Mutex<Vec<&'static str>>,
}

impl LogListener {
    pub fn new(log: Vec<(u64, &str)>) -> Self {
        Self {
            log: log.into_iter().map(|(s, t)| (s, t.to_owned())).collect(),
            grant: Grant::default(),
            refuse: false,
            allowed: AtomicBool::new(true),
            allowed_delay: None,
            replay_all: false,
            tail_pending: false,
            tail: Mutex::new(None),
            authorizes: Mutex::new(Vec::new()),
            still_allowed_calls: AtomicUsize::new(0),
            still_allowed_args: Mutex::new(Vec::new()),
            opens: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        }
    }

    pub fn with_grant(mut self, grant: Grant) -> Self {
        self.grant = grant;
        self
    }

    pub fn with_tail(self, tail: Source) -> Self {
        *self.tail.lock().unwrap() = Some(tail);
        self
    }

    pub fn authorize_calls(&self) -> Vec<(String, String)> {
        self.authorizes.lock().unwrap().clone()
    }

    pub fn open_calls(&self) -> Vec<OpenCall> {
        self.opens.lock().unwrap().clone()
    }

    pub fn still_allowed_count(&self) -> usize {
        self.still_allowed_calls.load(Ordering::SeqCst)
    }
}

impl Listener<Ctx> for LogListener {
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
        caller: &'a Caller,
        topic: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<Grant>> {
        async move {
            self.authorizes
                .lock()
                .unwrap()
                .push((caller.0.clone(), topic.to_owned()));
            self.events.lock().unwrap().push("authorize");
            if self.refuse {
                return Err(denied());
            }
            Ok(self.grant.clone())
        }
        .boxed()
    }

    fn still_allowed<'a>(
        &'a self,
        _ctx: &'a Ctx,
        caller: &'a Caller,
        topic: &'a str,
    ) -> BoxFuture<'a, bool> {
        async move {
            self.still_allowed_calls.fetch_add(1, Ordering::SeqCst);
            self.still_allowed_args
                .lock()
                .unwrap()
                .push((caller.0.clone(), topic.to_owned()));
            if let Some(delay) = self.allowed_delay {
                tokio::time::sleep(delay).await;
            }
            self.allowed.load(Ordering::SeqCst)
        }
        .boxed()
    }

    fn open<'a>(
        &'a self,
        _ctx: &'a Ctx,
        _params: Self::Params,
        cursor: Option<ListenCursor>,
        floor: Option<ListenCursor>,
    ) -> BoxFuture<'a, anyhow::Result<Opened<'a, Self::Snapshot, Self::Item>>> {
        async move {
            self.opens.lock().unwrap().push((cursor, floor));
            self.events.lock().unwrap().push("open");
            let after = if self.replay_all {
                None
            } else {
                cursor.map(|c| c.seq)
            };
            let replay: Vec<anyhow::Result<(u64, String)>> = self
                .log
                .iter()
                .filter(|(seq, _)| after.is_none_or(|a| *seq > a))
                .cloned()
                .map(Ok)
                .collect();
            let mut out: Source = stream::iter(replay).boxed();
            if let Some(tail) = self.tail.lock().unwrap().take() {
                out = out.chain(tail).boxed();
            }
            if self.tail_pending {
                out = out.chain(stream::pending()).boxed();
            }
            Ok(Opened::replay(out))
        }
        .boxed()
    }
}

pub struct Plan {
    pub state: BTreeMap<String, String>,
    pub seq: u64,
    pub stream: Source,
}

pub fn state(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// A snapshot listener over a state map. Each `open` consumes the next
/// queued `Plan`; with none queued it snapshots `state`/`seq` and ends.
pub struct StateListener {
    pub epoch: u64,
    pub state: BTreeMap<String, String>,
    pub seq: u64,
    pub resync: bool,
    pub policy: OverflowPolicy,
    pub grant: Grant,
    pub plans: Mutex<VecDeque<Plan>>,
    pub opens: Mutex<Vec<OpenCall>>,
}

impl StateListener {
    pub fn new(epoch: u64, state: BTreeMap<String, String>, seq: u64) -> Self {
        Self {
            epoch,
            state,
            seq,
            resync: false,
            policy: OverflowPolicy::End,
            grant: Grant::default(),
            plans: Mutex::new(VecDeque::new()),
            opens: Mutex::new(Vec::new()),
        }
    }

    pub fn with_resync(mut self, resync: bool) -> Self {
        self.resync = resync;
        self
    }

    pub fn with_policy(mut self, policy: OverflowPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Queue the live source (and snapshot) for the next `open`.
    pub fn queue(&self, state: BTreeMap<String, String>, seq: u64, stream: Source) {
        self.plans
            .lock()
            .unwrap()
            .push_back(Plan { state, seq, stream });
    }

    pub fn open_calls(&self) -> Vec<OpenCall> {
        self.opens.lock().unwrap().clone()
    }
}

impl Listener<Ctx> for StateListener {
    type Params = TopicParams;
    type Snapshot = BTreeMap<String, String>;
    type Item = String;

    fn topic(&self, params: &Self::Params) -> String {
        params.topic.clone()
    }

    fn epoch(&self) -> u64 {
        self.epoch
    }

    fn authorize<'a>(
        &'a self,
        _ctx: &'a Ctx,
        _caller: &'a Caller,
        _topic: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<Grant>> {
        async move { Ok(self.grant.clone()) }.boxed()
    }

    fn overflow(&self) -> OverflowPolicy {
        self.policy
    }

    fn open<'a>(
        &'a self,
        _ctx: &'a Ctx,
        _params: Self::Params,
        cursor: Option<ListenCursor>,
        floor: Option<ListenCursor>,
    ) -> BoxFuture<'a, anyhow::Result<Opened<'a, Self::Snapshot, Self::Item>>> {
        async move {
            self.opens.lock().unwrap().push((cursor, floor));
            let plan = self.plans.lock().unwrap().pop_front();
            let (state, seq, stream) = match plan {
                Some(p) => (p.state, p.seq, p.stream),
                None => (self.state.clone(), self.seq, source(vec![])),
            };
            Ok(Opened::snapshot(state, self.resync, seq, stream))
        }
        .boxed()
    }
}

/// Drain a frame stream into JSON values, panicking on stream errors.
pub async fn frames_json<S>(mut s: S) -> Vec<serde_json::Value>
where
    S: futures_util::Stream<Item = anyhow::Result<serde_json::Value>> + Unpin,
{
    let mut out = Vec::new();
    while let Some(v) = s.next().await {
        out.push(v.unwrap());
    }
    out
}

/// Turn a frame stream into a stream of serialised frames.
pub fn jsonify<'a, S, I>(
    s: BoxStream<'a, anyhow::Result<op_listen::ListenFrame<S, I>>>,
) -> BoxStream<'a, anyhow::Result<serde_json::Value>>
where
    S: serde::Serialize + Send + 'a,
    I: serde::Serialize + Send + 'a,
{
    s.map(|r| r.map(|f| serde_json::to_value(f).unwrap()))
        .boxed()
}
