//! Detached streaming connections.

use crate::{ListenCursor, ListenError};
use anyhow::Result;
use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream};
use futures_util::StreamExt;
use op_registry::{CallOptions, Registry};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

const DETACH_BUFFER: usize = 64;

/// Connects to a streaming operation while detaching its lifetime from the
/// caller's connection future.
pub fn detached_connect<C>(
    registry: Arc<Registry<C>>,
    ctx: Arc<C>,
    name: String,
    args: Value,
    caller: op_registry::Caller,
) -> impl FnMut(Option<ListenCursor>) -> BoxFuture<'static, Result<BoxStream<'static, Result<Value>>>>
where
    C: op_registry::Resolve + 'static,
{
    move |cursor| {
        let registry = registry.clone();
        let ctx = ctx.clone();
        let name = name.clone();
        let args = args.clone();
        let caller = caller.clone();

        Box::pin(async move {
            let (first_tx, first_rx) = oneshot::channel();
            let (tx, rx) = mpsc::channel(DETACH_BUFFER);
            let task = tokio::spawn(async move {
                let stream = match registry
                    .call_stream(
                        &name,
                        args,
                        &*ctx,
                        CallOptions::default().with_caller(caller),
                        cursor.map(op_registry::Cursor::from),
                    )
                    .await
                {
                    Ok(stream) => stream,
                    Err(error) => {
                        let _ = first_tx.send(Err(error));
                        return;
                    }
                };

                if first_tx.send(Ok(())).is_err() {
                    return;
                }

                futures_util::pin_mut!(stream);
                loop {
                    let item = tokio::select! {
                        _ = tx.closed() => return,
                        item = stream.next() => item,
                    };
                    let Some(item) = item else { return };
                    if tx.send(item).await.is_err() {
                        return;
                    }
                }
            });

            // The abort guard must exist before we await first_rx: if this
            // future is dropped before the first outcome (timeout, select),
            // dropping the stream (which owns the guard) aborts the task.
            let state = DetachState {
                receiver: rx,
                task: Some(AbortOnDrop(task)),
            };
            // `fuse` keeps polling after the end safe (unfold would panic).
            let detached = stream::unfold(state, next_item).fuse();
            match first_rx.await {
                Ok(Ok(())) => Ok(Box::pin(detached) as BoxStream<'static, Result<Value>>),
                Ok(Err(error)) => Err(error),
                // The task ended before reporting the connection result.
                Err(_) => Err(task_panicked()),
            }
        })
    }
}

struct DetachState {
    receiver: mpsc::Receiver<Result<Value>>,
    task: Option<AbortOnDrop>,
}

/// Aborts the forwarding task when dropped.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn next_item(mut state: DetachState) -> Option<(Result<Value>, DetachState)> {
    if let Some(item) = state.receiver.recv().await {
        return Some((item, state));
    }
    // Channel closed: take the handle and await it once. Once taken, the
    // state is no longer returned, so the stream ends on later polls.
    let mut guard = state.task.take()?;
    match (&mut guard.0).await {
        Ok(()) => None,
        Err(join_error) if !join_error.is_panic() => None,
        Err(_) => Some((Err(task_panicked()), state)),
    }
}

fn task_panicked() -> anyhow::Error {
    coded_error::into_anyhow(ListenError::TaskPanicked)
}
