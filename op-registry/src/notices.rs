//! Notices: out-of-band messages an operation sends while it runs.

use crate::Redactor;
use std::sync::Arc;

/// A message an operation sends to its caller while it runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    /// A line of text.
    Text(String),
    /// A progress report.
    Progress {
        /// Units done so far.
        done: u64,
        /// Total units, when known.
        total: Option<u64>,
        /// What the units are.
        label: String,
    },
}

/// Receives the notices of a call.
pub trait NoticeSink: Send + Sync {
    /// Takes one notice.
    fn send(&self, notice: Notice);
}

/// A handle ops use to send notices. Without a sink, notices are dropped.
#[derive(Clone, Default)]
pub struct Notices(Option<Arc<dyn NoticeSink>>);

impl std::fmt::Debug for Notices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("Notices")
            .field(&self.0.is_some())
            .finish()
    }
}

struct RedactingSink {
    inner: Arc<dyn NoticeSink>,
    redactor: Redactor,
}

impl NoticeSink for RedactingSink {
    fn send(&self, notice: Notice) {
        self.inner.send(match notice {
            Notice::Text(text) => Notice::Text(self.redactor.redact(&text)),
            Notice::Progress { done, total, label } => Notice::Progress {
                done,
                total,
                label: self.redactor.redact(&label),
            },
        });
    }
}

impl Notices {
    /// A handle whose text and labels are redacted before they reach the sink.
    pub(crate) fn redacting(self, redactor: &Redactor) -> Self {
        match self.0 {
            Some(inner) if !redactor.is_empty() => Self(Some(Arc::new(RedactingSink {
                inner,
                redactor: redactor.clone(),
            }))),
            other => Self(other),
        }
    }

    /// Creates a handle that sends to `sink`.
    pub fn new(sink: Arc<dyn NoticeSink>) -> Self {
        Self(Some(sink))
    }

    /// Sends a text notice.
    pub fn notice(&self, text: impl Into<String>) {
        if let Some(sink) = &self.0 {
            sink.send(Notice::Text(text.into()));
        }
    }

    /// Sends a progress notice.
    pub fn progress(&self, done: u64, total: Option<u64>, label: &str) {
        if let Some(sink) = &self.0 {
            sink.send(Notice::Progress {
                done,
                total,
                label: label.to_owned(),
            });
        }
    }
}
