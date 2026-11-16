//! `status watch`: a listen stream over component status.
//!
//! A watch stream opens with a snapshot frame, then emits one item frame per
//! status change, each with a cursor `epoch:seq`.
//!
//! Resync semantics: the first snapshot frame carries `resync: true` when the
//! client sent no cursor, when the cursor's epoch differs from the source's
//! epoch, or when the cursor's seq is no longer retained. In that case the
//! snapshot is a full reset: the client must discard its state and rebuild it
//! from the snapshot. With `resync: false` the snapshot is a refresh: the
//! cursor's epoch matches and its seq is still retained, so the client keeps
//! its state and applies the items that follow.
//!
//! Watch is not an exact resume. A reconnect may repeat changes the client
//! already saw, and a lagged stream is replaced by a fresh snapshot with
//! `resync: true` after the source overflows.

use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::future::BoxFuture;
use futures_util::StreamExt;
use op_listen::{Grant, ListenCursor, Listener, Opened, OverflowPolicy};
use op_registry::Caller;

use crate::open::{open_status, StatusError};
use crate::ops::{clamp_limit, GetParams};
use crate::source::StatusSource;
use crate::types::{Component, StatusFilter, StatusSnapshot};

/// Listener that streams component status changes after a snapshot.
pub struct StatusListener {
    max_components: usize,
    /// Source epoch recorded by the most recent `open`. Zero before any open.
    epoch: AtomicU64,
}

impl StatusListener {
    /// Creates a listener that returns at most `max_components` components per snapshot.
    pub fn new(max_components: usize) -> Self {
        Self {
            max_components,
            epoch: AtomicU64::new(0),
        }
    }
}

impl<C> Listener<C> for StatusListener
where
    C: StatusSource + Send + Sync + 'static,
{
    type Params = GetParams;
    type Snapshot = StatusSnapshot;
    type Item = Component;

    fn topic(&self, _params: &Self::Params) -> String {
        "status".to_owned()
    }

    /// Returns the epoch stored by the most recent `open`.
    ///
    /// op-listen's `Listener::epoch` has no ctx, so the listener records the
    /// source epoch at open; the cursor epoch of frames is the epoch of the
    /// newest open.
    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn authorize<'a>(
        &'a self,
        _ctx: &'a C,
        _caller: &'a Caller,
        _topic: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<Grant>> {
        Box::pin(std::future::ready(Ok(Grant::default())))
    }

    fn overflow(&self) -> OverflowPolicy {
        OverflowPolicy::Resnapshot
    }

    fn open<'a>(
        &'a self,
        ctx: &'a C,
        params: Self::Params,
        cursor: Option<ListenCursor>,
        _floor: Option<ListenCursor>,
    ) -> BoxFuture<'a, anyhow::Result<Opened<'a, Self::Snapshot, Self::Item>>> {
        Box::pin(async move {
            let limit = clamp_limit(params.limit, self.max_components);
            let filter = StatusFilter {
                kind: params.kind,
                id: params.id,
            };
            self.epoch.store(ctx.epoch(), Ordering::SeqCst);
            let opened = open_status(ctx, &filter, limit, cursor).await?;
            let changes = opened.changes.map(|item| item.map_err(map_lagged)).boxed();
            Ok(Opened::snapshot(
                opened.snapshot,
                opened.resync,
                opened.seq,
                changes,
            ))
        })
    }
}

/// Maps the status lag error to the op-listen lag error; other errors pass unchanged.
///
/// The skipped count is read from the `StatusError` when it can be downcast.
/// `coded_error::into_anyhow` wraps the typed error inside a `Coded` value, and
/// `coded_error::downcast_ref` sees through that wrapper.
fn map_lagged(error: anyhow::Error) -> anyhow::Error {
    if coded_error::facts_anyhow(&error).code != "op_status.lagged" {
        return error;
    }
    let skipped = match coded_error::downcast_ref::<StatusError>(&error) {
        Some(StatusError::Lagged { skipped }) => *skipped,
        _ => 0,
    };
    op_listen::lagged(skipped)
}
