#![allow(dead_code)]
//! A toy status source with a mutable change log, shared by the open and watch tests.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream};
use futures_util::StreamExt;
use op_status::{Component, SeqWindow, ServiceInfo, State, StatusFilter, StatusSource};

pub type Item = anyhow::Result<(u64, Component)>;

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.app")]
pub enum AppError {
    #[error("try again")]
    #[coded(code = "transient", class = Transient)]
    Transient,
    #[error("nope")]
    #[coded(code = "denied", class = Logical)]
    Denied,
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

pub fn class_of(err: &anyhow::Error) -> coded_error::ErrorClass {
    coded_error::facts_anyhow(err).class
}

/// A source whose `provides_watch` is the const parameter `W`.
pub struct ToyW<const W: bool> {
    pub service: Mutex<ServiceInfo>,
    pub epoch: AtomicU64,
    pub window: Mutex<SeqWindow>,
    pub rows: Mutex<Vec<Component>>,
    /// Changes the source has emitted, in seq order.
    pub log: Mutex<Vec<(u64, Component)>>,
    /// Changes pushed into the log as a side effect of `snapshot()`.
    pub inject_on_snapshot: Mutex<Vec<(u64, Component)>>,
    /// Scripted items yielded after the log items, before the stream ends.
    pub tail: Mutex<Vec<Item>>,
    /// Every `from_seq` passed to `watch`.
    pub watch_calls: Mutex<Vec<Option<u64>>>,
    pub allowed: &'static [&'static str],
}

pub type Toy = ToyW<true>;
pub type NoWatch = ToyW<false>;

impl<const W: bool> ToyW<W> {
    pub fn new() -> Self {
        Self {
            service: Mutex::new(ServiceInfo {
                name: "toy".into(),
                version: "1.0".into(),
            }),
            epoch: AtomicU64::new(7),
            window: Mutex::new(SeqWindow::default()),
            rows: Mutex::new(Vec::new()),
            log: Mutex::new(Vec::new()),
            inject_on_snapshot: Mutex::new(Vec::new()),
            tail: Mutex::new(Vec::new()),
            watch_calls: Mutex::new(Vec::new()),
            allowed: &["count", "name"],
        }
    }

    pub fn with_rows(self, rows: Vec<Component>) -> Self {
        *self.rows.lock().unwrap() = rows;
        self
    }

    pub fn set_epoch(&self, epoch: u64) {
        self.epoch.store(epoch, Ordering::SeqCst);
    }

    pub fn set_window(&self, oldest: u64, newest: u64) {
        *self.window.lock().unwrap() = SeqWindow { oldest, newest };
    }

    /// Emits a live change: appends it to the log and advances `newest`.
    pub fn push(&self, seq: u64, component: Component) {
        self.log.lock().unwrap().push((seq, component));
        let mut w = self.window.lock().unwrap();
        w.newest = w.newest.max(seq);
        if w.oldest == 0 {
            w.oldest = 1;
        }
    }

    pub fn inject_during_snapshot(&self, seq: u64, component: Component) {
        self.inject_on_snapshot.lock().unwrap().push((seq, component));
    }

    pub fn push_tail(&self, item: Item) {
        self.tail.lock().unwrap().push(item);
    }

    pub fn set_service(&self, name: &str, version: &str) {
        *self.service.lock().unwrap() = ServiceInfo {
            name: name.into(),
            version: version.into(),
        };
    }

    pub fn watch_froms(&self) -> Vec<Option<u64>> {
        self.watch_calls.lock().unwrap().clone()
    }
}

impl<const W: bool> Default for ToyW<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const W: bool> StatusSource for ToyW<W> {
    fn service(&self) -> ServiceInfo {
        self.service.lock().unwrap().clone()
    }

    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn now_unix_ms(&self) -> i64 {
        1_700_000_000_123
    }

    fn seq_window(&self) -> SeqWindow {
        *self.window.lock().unwrap()
    }

    fn snapshot<'a>(
        &'a self,
        filter: &'a StatusFilter,
        max: usize,
    ) -> BoxFuture<'a, anyhow::Result<Vec<Component>>> {
        let injected: Vec<_> = self.inject_on_snapshot.lock().unwrap().drain(..).collect();
        for (seq, component) in injected {
            self.push(seq, component);
        }
        let rows: Vec<Component> = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|c| filter.kind.as_ref().is_none_or(|k| &c.kind == k))
            .filter(|c| filter.id.as_ref().is_none_or(|i| &c.id == i))
            .take(max + 1)
            .cloned()
            .collect();
        Box::pin(async move { Ok(rows) })
    }

    fn provides_watch() -> bool {
        W
    }

    fn watch(&self, from_seq: Option<u64>) -> Option<BoxStream<'_, Item>> {
        if !W {
            return None;
        }
        self.watch_calls.lock().unwrap().push(from_seq);
        let mut items: Vec<Item> = self
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|(seq, _)| from_seq.is_none_or(|from| *seq > from))
            .map(|(seq, c)| Ok((*seq, c.clone())))
            .collect();
        items.extend(self.tail.lock().unwrap().drain(..));
        Some(stream::iter(items).boxed())
    }

    fn allowed_attr_keys(&self) -> &'static [&'static str] {
        self.allowed
    }
}

pub fn comp(id: &str) -> Component {
    Component::new("room", id, State::Ok)
}

pub fn ids(components: &[Component]) -> Vec<String> {
    components.iter().map(|c| c.id.clone()).collect()
}

/// 255 ASCII bytes, then a two byte char straddling byte 256, then more text.
pub fn straddling() -> String {
    format!("{}\u{e9}tail", "a".repeat(255))
}

/// The expected truncation of [`straddling`]: cut back to the char boundary.
pub fn straddling_cut() -> String {
    "a".repeat(255)
}

pub fn too_long() -> String {
    "x".repeat(257)
}
