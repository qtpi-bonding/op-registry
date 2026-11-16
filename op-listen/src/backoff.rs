//! Capped exponential backoff for reconnecting listeners.

use std::time::Duration;

/// Capped exponential backoff: each delay doubles until it reaches `cap`.
///
/// A connection that stays up for at least `healthy_after` resets the sequence
/// via [`Backoff::connection_ended`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Backoff {
    /// Delay returned by the first call to [`Backoff::next_delay`] after a reset.
    first: Duration,
    /// Upper bound for any delay.
    cap: Duration,
    /// Minimum connection lifetime that counts as healthy.
    healthy_after: Duration,
    /// Delay returned by the next call to [`Backoff::next_delay`].
    next: Duration,
}

impl Backoff {
    /// Creates a backoff starting at `first` and doubling up to `cap`.
    ///
    /// `first` is clamped to `cap`; `healthy_after` defaults to `cap`.
    pub fn new(first: Duration, cap: Duration) -> Self {
        let first = first.min(cap);
        Self {
            first,
            cap,
            healthy_after: cap,
            next: first,
        }
    }

    /// Backoff for `link` reconnects: 1s doubling to 30s.
    pub fn link() -> Self {
        Self::new(Duration::from_secs(1), Duration::from_secs(30))
    }

    /// Backoff for follower reconnects: 250ms doubling to 30s.
    pub fn follower() -> Self {
        Self::new(Duration::from_millis(250), Duration::from_secs(30))
    }

    /// Sets the minimum connection lifetime that counts as healthy.
    pub fn with_healthy_after(mut self, healthy_after: Duration) -> Self {
        self.healthy_after = healthy_after;
        self
    }

    /// Returns the current delay and advances the sequence, capped at `cap`.
    pub fn next_delay(&mut self) -> Duration {
        let current = self.next;
        self.next = current.saturating_mul(2).min(self.cap);
        current
    }

    /// Restarts the sequence so the next delay is `first` again.
    pub fn reset(&mut self) {
        self.next = self.first;
    }

    /// Records that a connection lived for `lived`; resets if it was healthy.
    pub fn connection_ended(&mut self, lived: Duration) {
        if lived >= self.healthy_after {
            self.reset();
        }
    }
}
