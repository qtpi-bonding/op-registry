//! Errors produced by the listener protocol.

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ListenError {
    #[error("invalid listen cursor {text}")]
    BadCursor { text: String },
    #[error("listen topic {topic} was revoked")]
    Revoked { topic: String },
    #[error("listen stream lagged by {skipped} items")]
    Lagged { skipped: u64 },
    #[error("listen sequence went out of order: {previous} followed by {next}")]
    OutOfOrder { previous: u64, next: u64 },
    #[error("listen resnapshot failed")]
    ResnapshotFailed,
    #[error("listen stream ended")]
    Ended,
    #[error("listen session expired")]
    SessionExpired,
    #[error("listen request has no caller")]
    NoCaller,
    #[error("listen task panicked")]
    TaskPanicked,
    #[error("invalid listen frame: {reason}")]
    BadFrame { reason: String },
    #[error("listen follow stopped: {code}")]
    Stopped { code: String },
}

impl coded_error::CodedError for ListenError {
    fn code(&self) -> &'static str {
        match self {
            Self::BadCursor { .. } => "op_listen.bad_cursor",
            Self::Revoked { .. } => "op_listen.revoked",
            Self::Lagged { .. } => "op_listen.lagged",
            Self::OutOfOrder { .. } => "op_listen.out_of_order",
            Self::ResnapshotFailed => "op_listen.resnapshot_failed",
            Self::Ended => "op_listen.ended",
            Self::SessionExpired => "op_listen.session_expired",
            Self::NoCaller => "op_listen.no_caller",
            Self::TaskPanicked => "op_listen.task_panicked",
            Self::BadFrame { .. } => "op_listen.bad_frame",
            Self::Stopped { .. } => "op_listen.stopped",
        }
    }

    fn class(&self) -> coded_error::ErrorClass {
        match self {
            Self::BadCursor { .. }
            | Self::Revoked { .. }
            | Self::NoCaller
            | Self::BadFrame { .. }
            | Self::Stopped { .. } => coded_error::ErrorClass::Logical,
            Self::Lagged { .. } | Self::Ended | Self::SessionExpired => {
                coded_error::ErrorClass::Transient
            }
            Self::OutOfOrder { .. } | Self::ResnapshotFailed | Self::TaskPanicked => {
                coded_error::ErrorClass::Infrastructure
            }
        }
    }
}

impl coded_error::CodeCatalog for ListenError {
    fn codes() -> Vec<coded_error::CodeEntry> {
        use coded_error::ErrorClass::{Infrastructure, Logical, Transient};
        let entry = |code, class| coded_error::CodeEntry {
            code,
            class: Some(class),
        };
        vec![
            entry("op_listen.bad_cursor", Logical),
            entry("op_listen.revoked", Logical),
            entry("op_listen.lagged", Transient),
            entry("op_listen.out_of_order", Infrastructure),
            entry("op_listen.resnapshot_failed", Infrastructure),
            entry("op_listen.ended", Transient),
            entry("op_listen.session_expired", Transient),
            entry("op_listen.no_caller", Logical),
            entry("op_listen.task_panicked", Infrastructure),
            entry("op_listen.bad_frame", Logical),
            entry("op_listen.stopped", Logical),
        ]
    }
}

pub fn lagged(skipped: u64) -> anyhow::Error {
    coded_error::into_anyhow(ListenError::Lagged { skipped })
}

/// Reports whether `error` is a typed [`ListenError::Lagged`].
///
/// Matches on the error type, not on the code string, so an error from another
/// source that merely carries the same code is not treated as lag.
#[cfg(feature = "registry")]
pub(crate) fn is_lagged(error: &anyhow::Error) -> bool {
    matches!(
        coded_error::downcast_ref::<ListenError>(error),
        Some(ListenError::Lagged { .. })
    )
}
