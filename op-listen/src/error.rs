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
        }
    }

    fn class(&self) -> coded_error::ErrorClass {
        match self {
            Self::BadCursor { .. } | Self::Revoked { .. } => coded_error::ErrorClass::Logical,
            Self::Lagged { .. } | Self::Ended => coded_error::ErrorClass::Transient,
            Self::OutOfOrder { .. } | Self::ResnapshotFailed => {
                coded_error::ErrorClass::Infrastructure
            }
        }
    }
}

pub fn lagged(skipped: u64) -> anyhow::Error {
    coded_error::into_anyhow(ListenError::Lagged { skipped })
}
