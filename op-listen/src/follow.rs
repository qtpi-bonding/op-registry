//! Generic reconnecting followers for listen streams.
//!
//! [`follow`] and [`follow_typed`] use the JSON implementation of [`Resumable`],
//! while [`follow_items`] can follow any item type that supplies a cursor and
//! stop information. All followers share the same reconnect loop, including
//! capped backoff, retry limits, and sink handling.

use crate::Backoff;
use crate::ListenCursor;
use crate::ListenError;
use crate::ListenFrame;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::time::Instant;

/// Supplies the resume and lifecycle information needed by a follower.
pub trait Resumable {
    /// The position a follow resumes from.
    type Cursor: Clone + Send + 'static;

    /// The position to resume after this item, if it carries one.
    fn cursor(&self) -> Option<Self::Cursor>;

    /// The code when this item is a stop (the stream ended for good).
    fn stop_code(&self) -> Option<&str> {
        None
    }

    /// Whether this item is a resync snapshot; reported on a reconnection marker.
    fn resync(&self) -> bool {
        false
    }
}

/// An item or lifecycle event delivered by [`follow_items`].
pub enum FollowItem<T: Resumable> {
    /// An item from the stream.
    Item(T),
    /// A new connection started; `resync` is true when its first item is a resync snapshot.
    Reconnected {
        cursor: Option<T::Cursor>,
        resync: bool,
    },
    /// A stop item was delivered and the follow is ending.
    Stopped { code: String },
}

/// Options for the follower functions.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FollowOptions<C = ListenCursor> {
    /// Cursor used for the first connection when no frame has been seen yet.
    pub initial_cursor: Option<C>,
    /// Delay sequence between reconnection attempts.
    pub backoff: Backoff,
    /// Number of consecutive failed attempts before giving up (`None` = forever).
    pub max_retries: Option<u32>,
}

impl<C> FollowOptions<C> {
    /// Options with no initial cursor, the follower backoff and no retry limit.
    pub fn new() -> Self {
        Self {
            initial_cursor: None,
            backoff: Backoff::follower(),
            max_retries: None,
        }
    }
}

impl Default for FollowOptions<ListenCursor> {
    fn default() -> Self {
        Self::new()
    }
}

/// How a followed stream finished when it returned `Ok`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FollowEnd {
    /// The listener sent a stop frame with this code.
    Stopped { code: String },
    /// The sink reported a broken pipe; the follow stopped without reconnecting.
    SinkClosed,
}

/// One event delivered to the sink of [`follow_typed`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FollowEvent<S, I> {
    /// A decoded frame from the listener.
    Frame(ListenFrame<S, I>),
    /// A new connection started; `resync` is true when its first frame is a resync snapshot.
    Reconnected {
        cursor: Option<ListenCursor>,
        resync: bool,
    },
    /// A stop frame was delivered and the follow is ending.
    Stopped { code: String },
}

/// An event as seen by the handler inside [`drive`].
enum Event<T: Resumable> {
    Reconnected {
        cursor: Option<T::Cursor>,
        resync: bool,
    },
    Frame(T),
    Stopped {
        code: String,
    },
}

/// Outcome of one connection attempt's stream, as seen by the follower.
enum Attempt {
    /// The stream delivered a stop item.
    Stopped { code: String },
    /// The stream failed or ended; the error explains why.
    Failed {
        error: anyhow::Error,
        delivered: bool,
    },
}

/// Mutable state carried across connections.
struct State<C> {
    cursor: Option<C>,
    connections: u64,
    backoff: Backoff,
    failures: u32,
}

fn is_logical(error: &anyhow::Error) -> bool {
    coded_error::facts_anyhow(error).class == coded_error::ErrorClass::Logical
}

fn cursor_of(frame: &Value) -> Option<ListenCursor> {
    frame.get("cursor")?.as_str()?.parse().ok()
}

/// Raw JSON listen frames: the cursor is the frame's `cursor`, a stop is a
/// `stop` frame and a resync is a `snapshot` frame with `resync: true`.
impl Resumable for Value {
    type Cursor = ListenCursor;

    fn cursor(&self) -> Option<Self::Cursor> {
        cursor_of(self)
    }

    fn stop_code(&self) -> Option<&str> {
        if self.get("frame").and_then(Value::as_str) == Some("stop") {
            Some(self.get("code").and_then(Value::as_str).unwrap_or_default())
        } else {
            None
        }
    }

    fn resync(&self) -> bool {
        self.get("frame").and_then(Value::as_str) == Some("snapshot")
            && self.get("resync").and_then(Value::as_bool) == Some(true)
    }
}

fn compact(value: &Value) -> anyhow::Result<String> {
    Ok(serde_json::to_string(value)?)
}

/// Reconnects with `connect` and prints frames through `sink` until a stop
/// frame arrives, a logical error occurs, or the retry budget is exhausted.
pub async fn follow<F, K>(connect: F, mut sink: K, opts: FollowOptions) -> anyhow::Result<FollowEnd>
where
    F: FnMut(
        Option<ListenCursor>,
    ) -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Value>>>>,
    K: FnMut(&str) -> std::io::Result<()>,
{
    drive(connect, |event| render(&mut sink, event), opts).await
}

/// Like [`follow`], but delivers typed [`FollowEvent`]s to `sink`. A frame
/// that does not decode as a [`ListenFrame`] ends the follow with a
/// `op_listen.bad_frame` error, before the sink sees it.
pub async fn follow_typed<S, I, F, K>(
    connect: F,
    mut sink: K,
    opts: FollowOptions,
) -> anyhow::Result<FollowEnd>
where
    S: serde::de::DeserializeOwned,
    I: serde::de::DeserializeOwned,
    F: FnMut(
        Option<ListenCursor>,
    ) -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Value>>>>,
    K: FnMut(FollowEvent<S, I>) -> std::io::Result<()>,
{
    drive(
        connect,
        |event| {
            let typed = match event {
                Event::Reconnected { cursor, resync } => {
                    FollowEvent::Reconnected { cursor, resync }
                }
                Event::Frame(raw) => {
                    let frame = serde_json::from_value::<ListenFrame<S, I>>(raw).map_err(|e| {
                        Handled::Failed(coded_error::into_anyhow(ListenError::BadFrame {
                            reason: e.to_string(),
                        }))
                    })?;
                    FollowEvent::Frame(frame)
                }
                Event::Stopped { code } => FollowEvent::Stopped { code },
            };
            sink(typed).map_err(sink_error)
        },
        opts,
    )
    .await
}

/// Follows arbitrary resumable items and sends lifecycle events to `sink`.
pub async fn follow_items<T, F, K>(
    connect: F,
    mut sink: K,
    opts: FollowOptions<T::Cursor>,
) -> anyhow::Result<FollowEnd>
where
    T: Resumable + Send + 'static,
    F: FnMut(
        Option<T::Cursor>,
    ) -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<T>>>>,
    K: FnMut(FollowItem<T>) -> std::io::Result<()>,
{
    drive(
        connect,
        |event| {
            let item = match event {
                Event::Frame(value) => FollowItem::Item(value),
                Event::Reconnected { cursor, resync } => FollowItem::Reconnected { cursor, resync },
                Event::Stopped { code } => FollowItem::Stopped { code },
            };
            sink(item).map_err(sink_error)
        },
        opts,
    )
    .await
}

/// Renders one event as a compact JSON line for [`follow`]. Only errors from
/// the sink call are classified as [`Handled`] sink errors.
fn render<K>(sink: &mut K, event: Event<Value>) -> Result<(), Handled>
where
    K: FnMut(&str) -> std::io::Result<()>,
{
    let line = match event {
        Event::Reconnected { cursor, resync } => compact(&json!({
            "marker": "reconnected",
            "cursor": cursor.map(|c| c.to_string()),
            "resync": resync,
        })),
        Event::Frame(raw) => compact(&raw),
        Event::Stopped { code } => compact(&json!({"marker": "stopped", "code": code})),
    }
    .map_err(Handled::Failed)?;
    sink(&line).map_err(sink_error)
}

/// Classifies an io error from the sink: a broken pipe is a closed sink.
fn sink_error(error: std::io::Error) -> Handled {
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        Handled::SinkClosed
    } else {
        Handled::Failed(anyhow::Error::from(error))
    }
}

/// The shared reconnect loop. Calls `handle` for every event; a handler error
/// is returned immediately, without retry, and a closed sink ends the follow
/// with [`FollowEnd::SinkClosed`].
async fn drive<T, F, H>(
    mut connect: F,
    mut handle: H,
    opts: FollowOptions<T::Cursor>,
) -> anyhow::Result<FollowEnd>
where
    T: Resumable + Send + 'static,
    F: FnMut(
        Option<T::Cursor>,
    ) -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<T>>>>,
    H: FnMut(Event<T>) -> Result<(), Handled>,
{
    let mut state = State {
        cursor: opts.initial_cursor.clone(),
        connections: 0,
        backoff: opts.backoff.clone(),
        failures: 0,
    };
    loop {
        let connect_cursor = state.cursor.clone().or_else(|| opts.initial_cursor.clone());
        let mut connected_at: Option<Instant> = None;
        let attempt = match connect(connect_cursor.clone()).await {
            Ok(stream) => {
                let reconnected = state.connections > 0;
                state.connections += 1;
                connected_at = Some(Instant::now());
                match run_connection(stream, connect_cursor, reconnected, &mut state, &mut handle)
                    .await
                {
                    Ok(attempt) => attempt,
                    Err(Handled::SinkClosed) => return Ok(FollowEnd::SinkClosed),
                    Err(Handled::Failed(error)) => return Err(error),
                }
            }
            Err(error) => {
                if is_logical(&error) {
                    return Err(error);
                }
                Attempt::Failed {
                    error,
                    delivered: false,
                }
            }
        };

        match attempt {
            Attempt::Stopped { code } => return Ok(FollowEnd::Stopped { code }),
            Attempt::Failed { error, delivered } => {
                if is_logical(&error) {
                    return Err(error);
                }
                if delivered {
                    state.failures = 0;
                    state.backoff.reset();
                } else if let Some(started) = connected_at {
                    if state.backoff.connection_ended(started.elapsed()) {
                        state.failures = 0;
                    }
                }
                state.failures += 1;
                if let Some(max) = opts.max_retries {
                    if state.failures > max {
                        return Err(error);
                    }
                }
                tokio::time::sleep(state.backoff.next_delay()).await;
            }
        }
    }
}

/// Error returned by a handler. A closed sink is kept apart from other
/// handler errors so that `drive` can end the follow cleanly.
enum Handled {
    /// The sink reported a broken pipe.
    SinkClosed,
    /// Any other handler error; returned to the caller as-is.
    Failed(anyhow::Error),
}

/// Calls the handler for one event. The single place where handler errors
/// pass back into the loop.
fn emit<T, H>(handle: &mut H, event: Event<T>) -> Result<(), Handled>
where
    T: Resumable,
    H: FnMut(Event<T>) -> Result<(), Handled>,
{
    handle(event)
}

/// Drains one connection's stream, passing its events to `handle`.
async fn run_connection<T, H>(
    mut stream: BoxStream<'static, anyhow::Result<T>>,
    connect_cursor: Option<T::Cursor>,
    reconnected: bool,
    state: &mut State<T::Cursor>,
    handle: &mut H,
) -> Result<Attempt, Handled>
where
    T: Resumable + Send + 'static,
    H: FnMut(Event<T>) -> Result<(), Handled>,
{
    let mut delivered = false;
    let mut marker_pending = reconnected;
    while let Some(next) = stream.next().await {
        let item = match next {
            Ok(item) => item,
            Err(error) => return Ok(Attempt::Failed { error, delivered }),
        };
        let cursor = item.cursor();
        let stop_code = item.stop_code().map(str::to_owned);
        let resync = marker_pending && item.resync();
        if marker_pending {
            emit(
                handle,
                Event::Reconnected {
                    cursor: connect_cursor.clone(),
                    resync,
                },
            )?;
            marker_pending = false;
        }
        if let Some(cursor) = cursor {
            state.cursor = Some(cursor);
        }
        emit(handle, Event::Frame(item))?;
        delivered = true;
        if let Some(code) = stop_code {
            emit(handle, Event::Stopped { code: code.clone() })?;
            return Ok(Attempt::Stopped { code });
        }
    }
    Ok(Attempt::Failed {
        error: coded_error::into_anyhow(ListenError::Ended),
        delivered,
    })
}
