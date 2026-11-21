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
        stop_codes: Vec::new(),
        on_retry: None,
        redact: None,
    }
}

/// A follow running on a background task, with the JSON lines it has rendered.
///
/// Dropping a `Followed` aborts the background task. Call [`Followed::abort`]
/// explicitly for the same effect, or [`Followed::join`] to wait for a follow
/// that ends on a stop frame.
pub struct Followed {
    /// Every rendered line parsed as JSON, in arrival order.
    lines: Arc<Mutex<Vec<Value>>>,
    /// The background task running [`follow`].
    task: JoinHandle<anyhow::Result<FollowEnd>>,
}

impl Followed {
    /// Starts following with `connect` and `options` on a spawned task.
    ///
    /// Each line rendered by the follow is parsed as JSON and recorded.
    /// A line that is not valid JSON ends the follow with an `InvalidData` error.
    pub fn spawn<F>(connect: F, options: FollowOptions) -> Self
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
        let task = tokio::spawn(follow(connect, sink, options));
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

    /// Aborts the background follow task.
    ///
    /// Also happens implicitly when the `Followed` is dropped.
    pub fn abort(self) {
        self.task.abort();
    }

    /// Waits for the follow task to finish and returns its result.
    ///
    /// # Errors
    ///
    /// Returns the follow's own error, or an error if the task panicked or was cancelled.
    pub async fn join(self) -> anyhow::Result<FollowEnd> {
        let mut this = self;
        // Aborting the finished task on drop is a no-op.
        (&mut this.task)
            .await
            .map_err(|e| anyhow!("follow task failed to join: {e}"))?
    }

    /// Asserts that the background follow task is still running.
    ///
    /// # Panics
    ///
    /// Panics with a message containing `step`, the follow's result, and the
    /// frames recorded so far if the task has finished.
    pub async fn assert_still_following(&mut self, step: &str) {
        if self.is_finished() {
            let result = (&mut self.task).await;
            ended_panic(step, result, &self.lines());
        }
    }

    /// Counts the recorded lines that carry a `"marker": "reconnected"` marker.
    pub fn reconnect_markers(&self) -> usize {
        self.lines()
            .iter()
            .filter(|l| l["marker"] == "reconnected")
            .count()
    }

    /// Polls every 10ms until `done` returns true for the recorded lines.
    ///
    /// # Panics
    ///
    /// Panics with a message containing `step` and the frames so far if
    /// `timeout` elapses first, or if the follow task finishes before `done`
    /// becomes true (the message also includes the follow's result).
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
                let result = (&mut self.task).await;
                ended_panic(&format!("waiting for {step}"), result, &self.lines());
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for {step}. Frames so far: {:?}",
                    self.lines()
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

/// Aborts the follow task when the `Followed` is dropped.
impl Drop for Followed {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Panics with the shared "follow ended" message: the step, the task's result,
/// and the frames recorded so far.
fn ended_panic(
    step: &str,
    result: Result<anyhow::Result<FollowEnd>, tokio::task::JoinError>,
    lines: &[Value],
) -> ! {
    panic!("follow task ended while {step}: {result:?}. Frames so far: {lines:?}");
}
