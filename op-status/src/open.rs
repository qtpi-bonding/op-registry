//! Opening a status snapshot together with a bounded stream of changes.
//!
//! [`open_status`] reads the sequence window first, then takes the snapshot,
//! then subscribes to changes after the window's newest seq. Changes at or
//! below that seq are skipped, so the snapshot and the change stream join
//! without a gap or a duplicate.
//!
//! Every component and service identity passes through the same attribute and
//! free-text rules as `status get`: debug builds refuse a violation, release
//! builds truncate or drop the offending part.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Result;
use futures_util::stream::{BoxStream, Stream};
use futures_util::StreamExt;

#[cfg(debug_assertions)]
use crate::check::{check_component, check_service_info};
#[cfg(not(debug_assertions))]
use crate::check::{
    truncate_at_char_boundary, MAX_ATTRS, MAX_ATTR_KEY_BYTES, MAX_ATTR_VALUE_BYTES,
};
use crate::source::StatusSource;
#[cfg(not(debug_assertions))]
use crate::types::AttrValue;
use crate::types::{Component, ServiceInfo, StatusFilter, StatusSnapshot};

/// Number of change items buffered before a consumer is reported as lagged.
pub const WATCH_BUFFER: usize = 256;

/// An opened status: a snapshot, the resync flag and the change stream.
pub struct OpenedStatus<'a> {
    /// Snapshot taken after the seq was read.
    pub snapshot: StatusSnapshot,
    /// True when the cursor cannot be resumed and the consumer must resync.
    pub resync: bool,
    /// The newest seq read before the snapshot. Changes after it follow in `changes`.
    pub seq: u64,
    /// Changes with seq greater than `seq`.
    pub changes: BoxStream<'a, anyhow::Result<(u64, Component)>>,
}

/// Errors raised while opening or following a status stream.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StatusError {
    /// The consumer fell behind by more than `WATCH_BUFFER` items.
    #[error("status watch lagged and skipped {skipped} changes")]
    Lagged {
        /// Number of changes that were dropped.
        skipped: u64,
    },
    /// The source does not implement `watch`.
    #[error("the status source does not support watch")]
    WatchUnsupported,
}

impl coded_error::CodedError for StatusError {
    fn code(&self) -> &'static str {
        match self {
            Self::Lagged { .. } => "op_status.lagged",
            Self::WatchUnsupported => "op_status.watch_unsupported",
        }
    }

    fn class(&self) -> coded_error::ErrorClass {
        match self {
            Self::Lagged { .. } => coded_error::ErrorClass::Transient,
            Self::WatchUnsupported => coded_error::ErrorClass::Logical,
        }
    }
}

/// Checks or bounds one component in place, using the rules of `status get`.
#[cfg(debug_assertions)]
pub(crate) fn filter_component(component: &mut Component, allowed: &[&str]) -> Result<()> {
    check_component(component, allowed).map_err(coded_error::into_anyhow)
}

/// Checks or bounds one component in place, using the rules of `status get`.
#[cfg(not(debug_assertions))]
pub(crate) fn filter_component(component: &mut Component, allowed: &[&str]) -> Result<()> {
    truncate_at_char_boundary(&mut component.kind, MAX_ATTR_VALUE_BYTES);
    truncate_at_char_boundary(&mut component.id, MAX_ATTR_VALUE_BYTES);
    truncate_at_char_boundary(&mut component.last_error_code, MAX_ATTR_VALUE_BYTES);
    truncate_at_char_boundary(&mut component.last_error_message, MAX_ATTR_VALUE_BYTES);

    component.attrs.retain(|key, value| {
        if !allowed.contains(&key.as_str()) || key.len() > MAX_ATTR_KEY_BYTES {
            return false;
        }
        match value {
            AttrValue::Str(text) => text.len() <= MAX_ATTR_VALUE_BYTES,
            _ => true,
        }
    });
    while component.attrs.len() > MAX_ATTRS {
        let last = component.attrs.keys().next_back().cloned();
        match last {
            Some(key) => {
                component.attrs.remove(&key);
            }
            None => break,
        }
    }
    Ok(())
}

/// Checks or bounds the service identity in place.
#[cfg(debug_assertions)]
pub(crate) fn filter_service(info: &mut ServiceInfo) -> Result<()> {
    check_service_info(info).map_err(coded_error::into_anyhow)
}

/// Checks or bounds the service identity in place.
#[cfg(not(debug_assertions))]
pub(crate) fn filter_service(info: &mut ServiceInfo) -> Result<()> {
    truncate_at_char_boundary(&mut info.name, MAX_ATTR_VALUE_BYTES);
    truncate_at_char_boundary(&mut info.version, MAX_ATTR_VALUE_BYTES);
    Ok(())
}

/// Takes a filtered snapshot, truncated to `limit` components.
pub(crate) async fn take_snapshot<S: StatusSource + ?Sized>(
    source: &S,
    filter: &StatusFilter,
    limit: usize,
) -> Result<StatusSnapshot> {
    let mut rows = source.snapshot(filter, limit).await?;
    let truncated = rows.len() > limit;
    rows.truncate(limit);

    let allowed = source.allowed_attr_keys();
    let mut components = Vec::with_capacity(rows.len());
    for mut component in rows {
        filter_component(&mut component, allowed)?;
        components.push(component);
    }

    let mut info = source.service();
    filter_service(&mut info)?;
    Ok(StatusSnapshot {
        service: info.name,
        version: info.version,
        taken_at_unix_ms: source.now_unix_ms(),
        epoch: source.epoch(),
        components,
        truncated,
    })
}

/// Opens a status snapshot and the changes that follow it.
pub async fn open_status<'a, S: StatusSource + ?Sized>(
    source: &'a S,
    filter: &StatusFilter,
    limit: usize,
    cursor: Option<op_listen::ListenCursor>,
) -> Result<OpenedStatus<'a>> {
    let window = source.seq_window();
    let seq = window.newest;
    let epoch = source.epoch();
    let snapshot = take_snapshot(source, filter, limit).await?;

    let inner = source
        .watch(Some(seq))
        .ok_or_else(|| coded_error::into_anyhow(StatusError::WatchUnsupported))?;

    let resync = match cursor {
        None => true,
        Some(c) => c.epoch != epoch || !(window.oldest <= c.seq && c.seq <= window.newest),
    };

    let allowed = source.allowed_attr_keys();
    let filtered = inner.filter_map(move |item| async move {
        match item {
            Ok((s, mut c)) => {
                if s <= seq {
                    return None;
                }
                Some(filter_component(&mut c, allowed).map(|()| (s, c)))
            }
            Err(e) => Some(Err(e)),
        }
    });

    Ok(OpenedStatus {
        snapshot,
        resync,
        seq,
        changes: Box::pin(BoundedChanges::new(Box::pin(filtered))),
    })
}

type Item = Result<(u64, Component)>;

/// Buffers up to `WATCH_BUFFER` ready items and reports lag once the buffer overflows.
struct BoundedChanges<'a> {
    inner: BoxStream<'a, Item>,
    buffer: VecDeque<Item>,
    lagged: Option<u64>,
    inner_done: bool,
    finished: bool,
}

impl<'a> BoundedChanges<'a> {
    fn new(inner: BoxStream<'a, Item>) -> Self {
        Self {
            inner,
            buffer: VecDeque::new(),
            lagged: None,
            inner_done: false,
            finished: false,
        }
    }

    /// Pulls ready items into the buffer until the inner stream is pending, done, or lagged.
    fn fill(&mut self, cx: &mut Context<'_>) {
        while self.lagged.is_none() && !self.inner_done {
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(item)) => {
                    if self.buffer.len() < WATCH_BUFFER {
                        self.buffer.push_back(item);
                    } else {
                        self.lagged = Some(1);
                    }
                }
                Poll::Ready(None) => self.inner_done = true,
                Poll::Pending => break,
            }
        }
    }
}

impl Stream for BoundedChanges<'_> {
    type Item = Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }
        self.fill(cx);
        if let Some(item) = self.buffer.pop_front() {
            return Poll::Ready(Some(item));
        }
        if let Some(skipped) = self.lagged.take() {
            self.finished = true;
            return Poll::Ready(Some(Err(coded_error::into_anyhow(StatusError::Lagged {
                skipped,
            }))));
        }
        if self.inner_done {
            self.finished = true;
            return Poll::Ready(None);
        }
        Poll::Pending
    }
}
