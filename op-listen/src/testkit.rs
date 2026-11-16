//! Test helpers for driving [`follow`] against scripted listen streams.
//!
//! [`Followed`] runs a reconnecting follow on a background task and records
//! every JSON line it renders, so tests can wait for a condition on the output
//! and then inspect the final result.

use crate::{follow, FollowEnd, FollowOptions};
use crate::{Backoff, ListenCursor};
use anyhow::anyhow;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;

/// Follow options for tests: a fast 20ms-to-200ms backoff with no retry cap.
pub fn short_follow_options() -> FollowOptions {
    FollowOptions {
        initial_cursor: None,
        backoff: Backoff::new(Duration::from_millis(20), Duration::from_millis(200)),
        max_retries: None,
    }
}

/// A follow running on a background task, with the JSON lines it has rendered.
pub struct Followed {
    /// Every rendered line parsed as JSON, in arrival order.
    lines: Arc<Mutex<Vec<Value>>>,
    /// The background task running [`follow`].
    task: JoinHandle<anyhow::Result<FollowEnd>>,
}

impl Followed {
    /// Starts following with `connect` on a spawned task.
    ///
    /// Each line rendered by the follow is parsed as JSON and recorded.
    /// A line that is not valid JSON ends the follow with an `InvalidData` error.
    pub fn spawn<F>(connect: F) -> Self
    where
        F: FnMut(
                Option<ListenCursor>,
            )
                -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Value>>>>
            + Send
            + 'static,
    {
        let lines: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_lines = Arc::clone(&lines);
        let sink = move |line: &str| -> std::io::Result<()> {
            let value: Value = serde_json::from_str(line)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            sink_lines.lock().expect("lines mutex poisoned").push(value);
            Ok(())
        };
        let task = tokio::spawn(follow(connect, sink, short_follow_options()));
        Self { lines, task }
    }

    /// Returns a snapshot of the lines recorded so far.
    pub fn lines(&self) -> Vec<Value> {
        self.lines.lock().expect("lines mutex poisoned").clone()
    }

    /// Returns true once the background follow task has finished.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Polls every 10ms until `done` returns true for the recorded lines.
    ///
    /// # Panics
    ///
    /// Panics with a message naming `step` if `timeout` elapses first, or if
    /// the follow task finishes before `done` becomes true.
    pub async fn wait_until(
        &mut self,
        step: &str,
        timeout: Duration,
        done: impl Fn(&[Value]) -> bool,
    ) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if done(&self.lines()) {
                return;
            }
            if self.is_finished() {
                // Check once more: the task may have recorded the final line
                // just before finishing.
                if done(&self.lines()) {
                    return;
                }
                panic!("follow task ended while waiting for {step}");
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("timed out waiting for {step}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits for the follow task to finish and returns its result.
    ///
    /// # Errors
    ///
    /// Returns the follow's own error, or an error if the task panicked or was cancelled.
    pub async fn finish(self) -> anyhow::Result<FollowEnd> {
        self.task
            .await
            .map_err(|e| anyhow!("follow task failed to join: {e}"))?
    }
}
