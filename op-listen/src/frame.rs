//! Wire frames emitted by a listen stream.

use crate::ListenCursor;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
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
