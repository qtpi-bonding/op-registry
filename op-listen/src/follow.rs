//! Reconnecting follower for listen streams.
//!
//! [`follow`] and [`follow_typed`] share one reconnect loop, [`drive`]. It
//! connects, reconnects after transient failures with capped exponential
//! backoff, and hands every event to a handler. [`follow`] renders events as
//! compact JSON lines; [`follow_typed`] decodes frames into [`ListenFrame`]s.
//! Each reconnection is announced with a marker.

use crate::Backoff;
use crate::ListenCursor;
use crate::ListenError;
use crate::ListenFrame;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::time::Instant;

/// Options for [`follow`] and [`follow_typed`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FollowOptions {
    /// Cursor used for the first connection when no frame has been seen yet.
    pub initial_cursor: Option<ListenCursor>,
    /// Delay sequence between reconnection attempts.
    pub backoff: Backoff,
    /// Number of consecutive failed attempts before giving up (`None` = forever).
    pub max_retries: Option<u32>,
}

impl Default for FollowOptions {
    fn default() -> Self {
        Self {
            initial_cursor: None,
            backoff: Backoff::follower(),
            max_retries: None,
        }
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
enum Event<'a> {
    Reconnected {
        cursor: Option<ListenCursor>,
        resync: bool,
    },
    Frame(&'a Value),
    Stopped {
        code: String,
    },
}

/// Outcome of one connection attempt's stream, as seen by the follower.
enum Attempt {
    /// The stream delivered a stop frame.
    Stopped { code: String },
    /// The stream failed or ended; the error explains why.
    Failed {
        error: anyhow::Error,
        delivered: bool,
    },
}

/// Mutable state carried across connections.
struct State {
    cursor: Option<ListenCursor>,
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
                    let frame =
                        serde_json::from_value::<ListenFrame<S, I>>(raw.clone()).map_err(|e| {
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

/// Renders one event as a compact JSON line for [`follow`]. Only errors from
/// the sink call are classified as [`Handled`] sink errors.
fn render<K>(sink: &mut K, event: Event<'_>) -> Result<(), Handled>
where
    K: FnMut(&str) -> std::io::Result<()>,
{
    let line = match event {
        Event::Reconnected { cursor, resync } => compact(&json!({
            "marker": "reconnected",
            "cursor": cursor.map(|c| c.to_string()),
            "resync": resync,
        })),
        Event::Frame(raw) => compact(raw),
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
async fn drive<F, H>(
    mut connect: F,
    mut handle: H,
    opts: FollowOptions,
) -> anyhow::Result<FollowEnd>
where
    F: FnMut(
        Option<ListenCursor>,
    ) -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Value>>>>,
    H: FnMut(Event<'_>) -> Result<(), Handled>,
{
    let mut state = State {
        cursor: opts.initial_cursor,
        connections: 0,
        backoff: opts.backoff.clone(),
        failures: 0,
    };
    loop {
        let connect_cursor = state.cursor.or(opts.initial_cursor);
        let mut connected_at: Option<Instant> = None;
        let attempt = match connect(connect_cursor).await {
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
fn emit<H>(handle: &mut H, event: Event<'_>) -> Result<(), Handled>
where
    H: FnMut(Event<'_>) -> Result<(), Handled>,
{
    handle(event)
}

/// Drains one connection's stream, passing its events to `handle`.
async fn run_connection<H>(
    mut stream: BoxStream<'static, anyhow::Result<Value>>,
    connect_cursor: Option<ListenCursor>,
    reconnected: bool,
    state: &mut State,
    handle: &mut H,
) -> Result<Attempt, Handled>
where
    H: FnMut(Event<'_>) -> Result<(), Handled>,
{
    let mut delivered = false;
    let mut marker_pending = reconnected;
    while let Some(next) = stream.next().await {
        let frame = match next {
            Ok(frame) => frame,
            Err(error) => return Ok(Attempt::Failed { error, delivered }),
        };
        if marker_pending {
            let resync = frame.get("frame").and_then(Value::as_str) == Some("snapshot")
                && frame.get("resync").and_then(Value::as_bool) == Some(true);
            emit(
                handle,
                Event::Reconnected {
                    cursor: connect_cursor,
                    resync,
                },
            )?;
            marker_pending = false;
        }
        if let Some(cursor) = cursor_of(&frame) {
            state.cursor = Some(cursor);
        }
        emit(handle, Event::Frame(&frame))?;
        delivered = true;
        if frame.get("frame").and_then(Value::as_str) == Some("stop") {
            let code = frame
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            emit(handle, Event::Stopped { code: code.clone() })?;
            return Ok(Attempt::Stopped { code });
        }
    }
    Ok(Attempt::Failed {
        error: coded_error::into_anyhow(ListenError::Ended),
        delivered,
    })
}
