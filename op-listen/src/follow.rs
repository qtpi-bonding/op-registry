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
use futures_util::{FutureExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::task::Poll;
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

/// A value or lifecycle marker from a supervised stream.
pub enum StreamStep<T> {
    /// An item from the stream.
    Item(T),
    /// The connection ended and may be reopened.
    Ended,
    /// The listen session expired and must be reopened.
    SessionExpired,
}

/// An event delivered by [`supervise`].
pub enum Supervised<T: Resumable> {
    /// A connection opened, reported after `connect` succeeds and before any item.
    /// Read `item.resync()` on the first item itself.
    Opened,
    /// An item from the connection.
    Item(T),
    /// A failed attempt will be retried.
    Retrying { code: String },
    /// The follow stopped with this code.
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
    /// Error codes that end the follow as `Stopped { code }` when they arrive as a
    /// failed open or a mid-stream error, before the Logical check. A stop that
    /// arrives as a frame is decided by [`Resumable::stop_code`], not by this list.
    pub stop_codes: Vec<String>,
}

impl<C> FollowOptions<C> {
    /// Options with no initial cursor, the follower backoff and no retry limit.
    pub fn new() -> Self {
        Self {
            initial_cursor: None,
            backoff: Backoff::follower(),
            max_retries: None,
            stop_codes: Vec::new(),
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
    Opened,
    Reconnected {
        cursor: Option<T::Cursor>,
        resync: bool,
    },
    Frame(T),
    Stopped {
        code: String,
    },
    Retrying {
        code: String,
    },
}

struct DriveConfig {
    /// Report `Opened` after each successful connect.
    opened: bool,
    /// Deliver a stop item only as `Stopped`, never as a `Frame` first.
    stop_as_event_only: bool,
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
    /// Connect attempts so far, failed opens included.
    attempts: u64,
    backoff: Backoff,
    failures: u32,
}

fn is_logical(error: &anyhow::Error) -> bool {
    coded_error::facts_anyhow(error).class == coded_error::ErrorClass::Logical
}

fn cursor_of(frame: &Value) -> Option<ListenCursor> {
    frame.get("cursor")?.as_str()?.parse().ok()
}

/// Raw JSON listen frames, decided by shape alone. The cursor is the frame's
/// `cursor` string when it parses as a [`ListenCursor`], else `None`. A frame
/// whose `frame` is `"stop"` is a stop, and its `code` (empty when missing) is
/// the stop code, so every stop frame ends a follow. A resync is a `frame` of
/// `"snapshot"` with `resync: true`. Any other value is an item that is neither
/// a stop nor a resync. A consumer frame type with per-topic stops implements
/// [`Resumable`] itself.
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
    drive(
        connect,
        |event| render(&mut sink, event),
        opts,
        DriveConfig {
            opened: false,
            stop_as_event_only: false,
        },
    )
    .await
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
                Event::Opened | Event::Retrying { .. } => return Ok(()),
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
        DriveConfig {
            opened: false,
            stop_as_event_only: false,
        },
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
                Event::Opened | Event::Retrying { .. } => return Ok(()),
                Event::Reconnected { cursor, resync } => FollowItem::Reconnected { cursor, resync },
                Event::Stopped { code } => FollowItem::Stopped { code },
            };
            sink(item).map_err(sink_error)
        },
        opts,
        DriveConfig {
            opened: false,
            stop_as_event_only: true,
        },
    )
    .await
}

/// Follows explicit stream steps and reports connection lifecycle events.
/// A stop item is delivered only as `Stopped { code }`.
pub async fn supervise<T, F, K>(
    mut connect: F,
    opts: FollowOptions<T::Cursor>,
    sink: K,
) -> anyhow::Result<FollowEnd>
where
    T: Resumable + Send + 'static,
    F: FnMut(
        Option<T::Cursor>,
    ) -> BoxFuture<
        'static,
        anyhow::Result<BoxStream<'static, anyhow::Result<StreamStep<T>>>>,
    >,
    K: FnMut(Supervised<T>) -> std::io::Result<()>,
{
    let wrapped = move |cursor| {
        let future = connect(cursor);
        Box::pin(async move {
            let stream = future.await?;
            let stream = stream.scan(false, |done, next| {
                let result = if *done {
                    None
                } else {
                    match next {
                        Ok(StreamStep::Item(item)) => Some(Some(Ok(item))),
                        Ok(StreamStep::Ended) => {
                            *done = true;
                            Some(Some(Err(coded_error::into_anyhow(ListenError::Ended))))
                        }
                        Ok(StreamStep::SessionExpired) => {
                            *done = true;
                            Some(Some(Err(coded_error::into_anyhow(
                                ListenError::SessionExpired,
                            ))))
                        }
                        Err(error) => Some(Some(Err(error))),
                    }
                };
                async move { result }
            });
            Ok(stream.filter_map(|item| async move { item }).boxed())
        }) as BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<T>>>>
    };
    supervise_stream(wrapped, opts, sink).await
}

/// Like [`supervise`], over a plain stream of items. The end of a stream is
/// `op_listen.ended` (Transient, retried). A stop item is delivered only as
/// `Stopped { code }`.
pub async fn supervise_stream<T, F, K>(
    connect: F,
    opts: FollowOptions<T::Cursor>,
    mut sink: K,
) -> anyhow::Result<FollowEnd>
where
    T: Resumable + Send + 'static,
    F: FnMut(
        Option<T::Cursor>,
    ) -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<T>>>>,
    K: FnMut(Supervised<T>) -> std::io::Result<()>,
{
    drive(
        connect,
        |event| {
            let event = match event {
                Event::Opened => Supervised::Opened,
                Event::Frame(item) => Supervised::Item(item),
                Event::Retrying { code } => Supervised::Retrying { code },
                Event::Stopped { code } => Supervised::Stopped { code },
                Event::Reconnected { .. } => return Ok(()),
            };
            sink(event).map_err(sink_error)
        },
        opts,
        DriveConfig {
            opened: true,
            stop_as_event_only: true,
        },
    )
    .await
}

/// One value of a [`follow_stream`]: a frame of the followed stream, or the
/// marker that precedes the first frame of a reconnection.
#[derive(Serialize)]
#[cfg_attr(feature = "registry", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Followed<F> {
    /// An item of the followed stream.
    Frame(F),
    /// The reconnect marker, as shaped by the caller's marker function.
    Marker(Map<String, Value>),
}

/// A reconnecting follower as a stream, so it can be the body of a stream op.
///
/// It runs the same reconnect loop as [`follow_items`]: it connects with the
/// last cursor (or the initial one), reconnects with the backoff, and ends
/// with the last error after `max_retries` consecutive failures. A failed open
/// is a failure like a connection that ends, so it is retried while the error
/// is not Logical; a connection that ends without an item counts as one too,
/// except that an empty connection which lasted at least
/// `backoff.healthy_after` first resets the failure count (it is the one
/// failure counted afterwards), so such connections never add up toward
/// `max_retries`. The marker is yielded before the first item that follows a
/// failed first open or any earlier connection.
///
/// The source must yield. The follower is polled as one future whose items go
/// into an unbounded queue, so a source that is always ready (a stream that
/// never returns `Pending`) is drained into that queue in a single poll and
/// never lets the consumer see an item.
///
/// Every item is yielded as [`Followed::Frame`]. Before the first item of a
/// reconnection it yields `Followed::Marker(marker(cursor, resync))`; a
/// connection that yields nothing gets no marker. A stop item is yielded once
/// and then the stream ends with `op_listen.stopped` (Logical). An error whose
/// code is in `stop_codes` ends it with `op_listen.stopped` naming that code
/// and no frame. Any other Logical error ends it as itself.
pub fn follow_stream<'a, T, F>(
    connect: F,
    opts: FollowOptions<T::Cursor>,
    marker: impl Fn(Option<&T::Cursor>, bool) -> Map<String, Value> + Send + 'a,
) -> BoxStream<'a, anyhow::Result<Followed<T>>>
where
    T: Resumable + Send + 'a,
    F: FnMut(Option<T::Cursor>) -> BoxFuture<'a, anyhow::Result<BoxStream<'a, anyhow::Result<T>>>>
        + Send
        + 'a,
{
    type Queue<T> = Arc<Mutex<VecDeque<anyhow::Result<Followed<T>>>>>;
    let queue: Queue<T> = Arc::default();
    let pushed = Arc::clone(&queue);
    let handler = move |event: Event<T>| -> Result<(), Handled> {
        let out = match event {
            Event::Frame(item) => Followed::Frame(item),
            Event::Reconnected { cursor, resync } => {
                Followed::Marker(marker(cursor.as_ref(), resync))
            }
            // The end of the stream carries the stop; there is nothing to print.
            Event::Opened | Event::Retrying { .. } | Event::Stopped { .. } => return Ok(()),
        };
        pushed.lock().expect("queue lock").push_back(Ok(out));
        Ok(())
    };
    let mut driving: Option<BoxFuture<'a, anyhow::Result<FollowEnd>>> = Some(
        drive(
            connect,
            handler,
            opts,
            DriveConfig {
                opened: false,
                stop_as_event_only: false,
            },
        )
        .boxed(),
    );
    futures_util::stream::poll_fn(move |cx| loop {
        if let Some(next) = queue.lock().expect("queue lock").pop_front() {
            return Poll::Ready(Some(next));
        }
        let Some(future) = driving.as_mut() else {
            return Poll::Ready(None);
        };
        match future.as_mut().poll(cx) {
            // The loop may have queued items before it suspended.
            Poll::Pending if queue.lock().expect("queue lock").is_empty() => return Poll::Pending,
            Poll::Pending => {}
            Poll::Ready(end) => {
                driving = None;
                let error = match end {
                    Ok(FollowEnd::Stopped { code }) => {
                        coded_error::into_anyhow(ListenError::Stopped { code })
                    }
                    // The handler never fails, so the sink cannot close.
                    Ok(FollowEnd::SinkClosed) => continue,
                    Err(error) => error,
                };
                queue.lock().expect("queue lock").push_back(Err(error));
            }
        }
    })
    .boxed()
}

/// Renders one event as a compact JSON line for [`follow`]. Only errors from
/// the sink call are classified as [`Handled`] sink errors.
fn render<K>(sink: &mut K, event: Event<Value>) -> Result<(), Handled>
where
    K: FnMut(&str) -> std::io::Result<()>,
{
    let line = match event {
        Event::Opened | Event::Retrying { .. } => return Ok(()),
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
async fn drive<'a, T, F, H>(
    mut connect: F,
    mut handle: H,
    opts: FollowOptions<T::Cursor>,
    config: DriveConfig,
) -> anyhow::Result<FollowEnd>
where
    T: Resumable + Send + 'a,
    F: FnMut(Option<T::Cursor>) -> BoxFuture<'a, anyhow::Result<BoxStream<'a, anyhow::Result<T>>>>,
    H: FnMut(Event<T>) -> Result<(), Handled>,
{
    let mut state = State {
        cursor: opts.initial_cursor.clone(),
        attempts: 0,
        backoff: opts.backoff.clone(),
        failures: 0,
    };
    loop {
        let connect_cursor = state.cursor.clone().or_else(|| opts.initial_cursor.clone());
        let mut connected_at: Option<Instant> = None;
        // Any attempt after the first makes the next delivered item the first
        // of a reconnection, whether the earlier attempt failed to open, ended
        // empty or served.
        let reconnected = state.attempts > 0;
        state.attempts += 1;
        let attempt = match connect(connect_cursor.clone()).await {
            Ok(stream) => {
                connected_at = Some(Instant::now());
                if config.opened {
                    match emit(&mut handle, Event::Opened) {
                        Ok(()) => {}
                        Err(Handled::SinkClosed) => return Ok(FollowEnd::SinkClosed),
                        Err(Handled::Failed(error)) => return Err(error),
                    }
                }
                match run_connection(
                    stream,
                    connect_cursor,
                    reconnected,
                    &mut state,
                    &mut handle,
                    &opts.stop_codes,
                    config.stop_as_event_only,
                )
                .await
                {
                    Ok(attempt) => attempt,
                    Err(Handled::SinkClosed) => return Ok(FollowEnd::SinkClosed),
                    Err(Handled::Failed(error)) => return Err(error),
                }
            }
            Err(error) => {
                let code = coded_error::facts_anyhow(&error).code.to_owned();
                if opts.stop_codes.iter().any(|stop| *stop == code) {
                    match emit(&mut handle, Event::Stopped { code: code.clone() }) {
                        Ok(()) => {}
                        Err(Handled::SinkClosed) => return Ok(FollowEnd::SinkClosed),
                        Err(Handled::Failed(error)) => return Err(error),
                    }
                    return Ok(FollowEnd::Stopped { code });
                }
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
                let code = coded_error::facts_anyhow(&error).code.to_owned();
                match emit(&mut handle, Event::Retrying { code }) {
                    Ok(()) => {}
                    Err(Handled::SinkClosed) => return Ok(FollowEnd::SinkClosed),
                    Err(Handled::Failed(error)) => return Err(error),
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
async fn run_connection<'a, T, H>(
    mut stream: BoxStream<'a, anyhow::Result<T>>,
    connect_cursor: Option<T::Cursor>,
    reconnected: bool,
    state: &mut State<T::Cursor>,
    handle: &mut H,
    stop_codes: &[String],
    stop_as_event_only: bool,
) -> Result<Attempt, Handled>
where
    T: Resumable + Send + 'a,
    H: FnMut(Event<T>) -> Result<(), Handled>,
{
    let mut delivered = false;
    let mut marker_pending = reconnected;
    while let Some(next) = stream.next().await {
        let item = match next {
            Ok(item) => item,
            Err(error) => {
                let code = coded_error::facts_anyhow(&error).code;
                if stop_codes.iter().any(|stop| stop == code) {
                    let code = code.to_owned();
                    emit(handle, Event::Stopped { code: code.clone() })?;
                    return Ok(Attempt::Stopped { code });
                }
                return Ok(Attempt::Failed { error, delivered });
            }
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
        if stop_code.is_none() || !stop_as_event_only {
            emit(handle, Event::Frame(item))?;
        }
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
