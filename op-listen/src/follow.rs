//! Reconnecting follower for listen streams.
//!
//! [`follow`] drives a connect closure, prints every frame as one compact JSON
//! line through a sink, and reconnects after transient failures with capped
//! exponential backoff. Each reconnection is announced with a marker line.

use crate::Backoff;
use crate::ListenCursor;
use crate::ListenError;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::time::Instant;

/// Options for [`follow`].
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
pub async fn follow<F, K>(
    mut connect: F,
    mut sink: K,
    opts: FollowOptions,
) -> anyhow::Result<FollowEnd>
where
    F: FnMut(
        Option<ListenCursor>,
    ) -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Value>>>>,
    K: FnMut(&str) -> std::io::Result<()>,
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
                run_connection(stream, connect_cursor, reconnected, &mut state, &mut sink).await?
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
                    let was_reset = state.backoff.is_reset();
                    state.backoff.connection_ended(started.elapsed());
                    if !was_reset && state.backoff.is_reset() {
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

/// Drains one connection's stream, printing its frames.
async fn run_connection<K>(
    mut stream: BoxStream<'static, anyhow::Result<Value>>,
    connect_cursor: Option<ListenCursor>,
    reconnected: bool,
    state: &mut State,
    sink: &mut K,
) -> anyhow::Result<Attempt>
where
    K: FnMut(&str) -> std::io::Result<()>,
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
            let marker = json!({
                "marker": "reconnected",
                "cursor": connect_cursor.map(|c| c.to_string()),
                "resync": resync,
            });
            sink(&compact(&marker)?)?;
            marker_pending = false;
        }
        if let Some(cursor) = cursor_of(&frame) {
            state.cursor = Some(cursor);
        }
        sink(&compact(&frame)?)?;
        delivered = true;
        if frame.get("frame").and_then(Value::as_str) == Some("stop") {
            let code = frame
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            sink(&compact(&json!({"marker": "stopped", "code": code}))?)?;
            return Ok(Attempt::Stopped { code });
        }
    }
    Ok(Attempt::Failed {
        error: coded_error::into_anyhow(ListenError::Ended),
        delivered,
    })
}
