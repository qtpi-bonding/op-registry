//! Wire frames emitted by a listen stream.

use crate::{ListenCursor, Resumable};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "registry", derive(schemars::JsonSchema))]
#[serde(
    tag = "frame",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum ListenFrame<S, I> {
    Snapshot {
        snapshot: S,
        resync: bool,
        cursor: ListenCursor,
    },
    Item {
        item: I,
        cursor: ListenCursor,
    },
    Stop {
        code: String,
        message: String,
    },
}

impl<S, I> ListenFrame<S, I> {
    /// Returns the cursor carried by this frame, or `None` for a stop frame.
    pub fn cursor(&self) -> Option<ListenCursor> {
        match self {
            Self::Snapshot { cursor, .. } | Self::Item { cursor, .. } => Some(*cursor),
            Self::Stop { .. } => None,
        }
    }
}

/// A listen frame is resumable by its cursor. Only a `Stop` frame is terminal
/// and only a resync `Snapshot` is a resync.
impl<S, I> Resumable for ListenFrame<S, I> {
    type Cursor = ListenCursor;

    fn cursor(&self) -> Option<ListenCursor> {
        ListenFrame::cursor(self)
    }

    fn stop_code(&self) -> Option<&str> {
        match self {
            Self::Stop { code, .. } => Some(code),
            _ => None,
        }
    }

    fn resync(&self) -> bool {
        matches!(self, Self::Snapshot { resync: true, .. })
    }
}
