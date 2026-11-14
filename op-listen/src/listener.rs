//! Traits and types for opening and authorizing listen streams.

use crate::{ListenCursor, ListenError};
use futures_util::{future::BoxFuture, stream::BoxStream};
use op_registry::Caller;
use serde::Serialize;
use std::time::Duration;

/// Grants permission to open a listener stream and optionally controls its starting point.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Grant {
    /// The earliest cursor from which the caller may receive items.
    pub not_before: Option<ListenCursor>,
    /// How often authorization should be checked while the stream is running.
    pub recheck_every: Option<Duration>,
}

/// The source selected after a listener has authorized a request.
#[non_exhaustive]
pub enum Opened<'a, S, I> {
    /// A stream of items to replay from the requested cursor.
    Replay(BoxStream<'a, anyhow::Result<(u64, I)>>),
    /// A snapshot followed by items newer than the snapshot sequence.
    Snapshot {
        /// The state represented by the snapshot.
        snapshot: S,
        /// Whether the snapshot requires the consumer to resynchronize.
        resync: bool,
        /// The sequence represented by the snapshot.
        seq: u64,
        /// Items following the snapshot.
        stream: BoxStream<'a, anyhow::Result<(u64, I)>>,
    },
}

impl<'a, S, I> Opened<'a, S, I> {
    /// Creates a replay result from an item stream.
    pub fn replay(stream: BoxStream<'a, anyhow::Result<(u64, I)>>) -> Self {
        Self::Replay(stream)
    }

    /// Creates a snapshot result and its following item stream.
    pub fn snapshot(
        snapshot: S,
        resync: bool,
        seq: u64,
        stream: BoxStream<'a, anyhow::Result<(u64, I)>>,
    ) -> Self {
        Self::Snapshot {
            snapshot,
            resync,
            seq,
            stream,
        }
    }
}

/// Specifies what a listener does when its source has overflowed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverflowPolicy {
    /// Request a fresh snapshot and continue when the source overflows.
    Resnapshot,
    /// Return the overflow error and end the stream.
    #[default]
    End,
}

/// Authorizes and opens a typed stream of listener snapshots and items.
pub trait Listener<C>: Send + Sync + 'static {
    /// Parameters used to identify and open a topic.
    type Params: op_registry::OpParams + Serialize + Clone + Send + 'static;
    /// Snapshot value emitted when opening or resynchronizing a stream.
    type Snapshot: Serialize + schemars::JsonSchema + Send + 'static;
    /// Item value emitted by the listener stream.
    type Item: Serialize + schemars::JsonSchema + Send + 'static;

    /// Returns the topic identified by the supplied parameters.
    fn topic(&self, params: &Self::Params) -> String;

    /// Returns the current epoch for this listener.
    fn epoch(&self) -> u64;

    /// Authorizes a caller for a topic and returns its stream grant.
    fn authorize<'a>(
        &'a self,
        ctx: &'a C,
        caller: &'a Caller,
        topic: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<Grant>>;

    /// Rechecks whether a caller may continue receiving a topic.
    fn still_allowed<'a>(
        &'a self,
        _ctx: &'a C,
        _caller: &'a Caller,
        _topic: &'a str,
    ) -> BoxFuture<'a, bool> {
        Box::pin(std::future::ready(true))
    }

    /// Creates the coded error reported when access to a topic is revoked.
    fn revoked_error(&self, topic: &str) -> anyhow::Error {
        coded_error::into_anyhow(ListenError::Revoked {
            topic: topic.to_owned(),
        })
    }

    /// Selects the response to a source overflow.
    fn overflow(&self) -> OverflowPolicy {
        OverflowPolicy::End
    }

    /// Opens a replay or snapshot stream at the requested cursor.
    fn open<'a>(
        &'a self,
        ctx: &'a C,
        params: Self::Params,
        cursor: Option<ListenCursor>,
        floor: Option<ListenCursor>,
    ) -> BoxFuture<'a, anyhow::Result<Opened<'a, Self::Snapshot, Self::Item>>>;
}
