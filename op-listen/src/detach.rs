//! Detached streaming connections.

use crate::{ListenCursor, ListenError};
use anyhow::Result;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{Future, Stream};
use op_registry::{CallOptions, Registry};
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Connects to a streaming operation while detaching its lifetime from the
/// caller's connection future.
pub fn detached_connect<C>(
    registry: Arc<Registry<C>>,
    ctx: Arc<C>,
    name: String,
    args: Value,
    caller: op_registry::Caller,
) -> impl FnMut(
    Option<ListenCursor>,
) -> BoxFuture<'static, Result<BoxStream<'static, Result<Value>>>>
where
    C: Send + Sync + 'static,
{
    move |cursor| {
        let registry = registry.clone();
        let ctx = ctx.clone();
        let name = name.clone();
        let args = args.clone();
        let caller = caller.clone();

        Box::pin(async move {
            let (first_tx, first_rx) = oneshot::channel();
            let (tx, rx) = mpsc::channel(64);
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

                use futures_util::StreamExt;
                futures_util::pin_mut!(stream);
                while let Some(item) = stream.next().await {
                    if tx.send(item).await.is_err() {
                        return;
                    }
                }
            });

            match first_rx.await {
                Ok(Ok(())) => Ok(Box::pin(DetachedStream {
                    receiver: rx,
                    task,
                    panic_reported: false,
                    task_done: false,
                }) as BoxStream<'static, Result<Value>>),
                Ok(Err(error)) => {
                    task.abort();
                    Err(error)
                }
                Err(_) => {
                    // The task can terminate before it gets to report the
                    // connection result (most notably by panicking).
                    drop(task);
                    Err(task_panicked())
                }
            }
        })
    }
}

struct DetachedStream {
    receiver: mpsc::Receiver<Result<Value>>,
    task: JoinHandle<()>,
    panic_reported: bool,
    task_done: bool,
}

impl Stream for DetachedStream {
    type Item = Result<Value>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match this.receiver.poll_recv(cx) {
            Poll::Ready(Some(item)) => Poll::Ready(Some(item)),
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => {
                if this.task_done {
                    return Poll::Ready(None);
                }
                match Pin::new(&mut this.task).poll(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(Ok(())) => {
                        this.task_done = true;
                        Poll::Ready(None)
                    }
                    Poll::Ready(Err(_)) => {
                        this.task_done = true;
                        if !this.panic_reported {
                            this.panic_reported = true;
                            Poll::Ready(Some(Err(task_panicked())))
                        } else {
                            Poll::Ready(None)
                        }
                    }
                }
            }
        }
    }
}

impl Drop for DetachedStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn task_panicked() -> anyhow::Error {
    coded_error::into_anyhow(ListenError::TaskPanicked)
}
