//! The `StatusSource` trait implemented by services that report status.
//!
//! A source is read-only. It returns components through [`StatusSource::snapshot`],
//! applying the filter itself, and never more than `max + 1` components so the
//! caller can tell that the result was truncated.

use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;

use crate::types::{Component, ServiceInfo, StatusFilter};

/// The retained sequence window of a source.
///
/// A cursor seq `s` is retained iff `oldest <= s <= newest`. `newest` is the
/// newest seq the source has emitted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeqWindow {
    /// Oldest seq still retained.
    pub oldest: u64,
    /// Newest seq the source has emitted.
    pub newest: u64,
}

/// A read-only provider of component status.
pub trait StatusSource: Send + Sync {
    /// Identity of the service that owns this source.
    fn service(&self) -> ServiceInfo;

    /// Current source epoch. Changes when the source restarts or resets.
    fn epoch(&self) -> u64;

    /// Current time in Unix milliseconds. Returns 0 if the clock is before the Unix epoch.
    fn now_unix_ms(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
            Err(_) => 0,
        }
    }

    /// Takes a snapshot of components matching `filter`.
    ///
    /// Returns at most `max + 1` components so the caller can detect truncation.
    fn snapshot<'a>(
        &'a self,
        filter: &'a StatusFilter,
        max: usize,
    ) -> BoxFuture<'a, anyhow::Result<Vec<Component>>>;

    /// Returns the changes with seq greater than `from_seq`.
    ///
    /// `from_seq` of `None` means only live changes. Otherwise the retained
    /// backlog is replayed first, then live changes follow. Defaults to `None`.
    fn watch(
        &self,
        from_seq: Option<u64>,
    ) -> Option<BoxStream<'_, anyhow::Result<(u64, Component)>>> {
        let _ = from_seq;
        None
    }

    /// The retained sequence window. Defaults to an empty window at zero.
    fn seq_window(&self) -> SeqWindow {
        SeqWindow::default()
    }

    /// Whether this source implements [`StatusSource::watch`].
    fn provides_watch() -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Attribute keys this source may report. Anything else is dropped or rejected.
    fn allowed_attr_keys(&self) -> &'static [&'static str];
}
