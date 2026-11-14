//! Wire frames emitted by a listen stream.

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "frame", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ListenFrame<S, I> {
    Snapshot {
        snapshot: S,
        resync: bool,
        cursor: String,
    },
    Item {
        item: I,
        cursor: String,
    },
    Stop {
        code: String,
        message: String,
    },
}
