//! Stdout helpers for following listen streams.

use crate::{follow, FollowEnd, FollowOptions, ListenCursor};
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use serde_json::Value;
use std::io::Write;

/// Returns a sink that writes each line to stdout followed by a newline, and
/// flushes after every line.
///
/// ```no_run
/// let mut sink = op_listen::stdout_sink();
/// sink("{}").unwrap();
/// ```
pub fn stdout_sink() -> impl FnMut(&str) -> std::io::Result<()> {
    |line: &str| {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        writeln!(lock, "{line}")?;
        lock.flush()
    }
}

/// Follows a listen stream and prints its events to stdout as JSON lines.
/// Equivalent to [`follow`] with [`stdout_sink`].
pub async fn follow_to_stdout<F>(connect: F, opts: FollowOptions) -> anyhow::Result<FollowEnd>
where
    F: FnMut(
        Option<ListenCursor>,
    ) -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<Value>>>>,
{
    follow(connect, stdout_sink(), opts).await
}
