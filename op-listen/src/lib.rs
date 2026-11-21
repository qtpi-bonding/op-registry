//! op-listen: shared listener for op-registry.

pub mod backoff;
pub mod cursor;
#[cfg(feature = "detach")]
pub mod detach;
pub mod entry;
pub mod error;
pub mod follow;
pub mod frame;
pub mod listener;
pub mod run;
pub mod stdout;
#[cfg(feature = "testkit")]
pub mod testkit;

pub use backoff::Backoff;
pub use cursor::ListenCursor;
#[cfg(feature = "detach")]
pub use detach::detached_connect;
pub use entry::listen_entry;
pub use error::{lagged, ListenError};
pub use follow::{
    follow, follow_items, follow_typed, supervise, supervise_stream, FollowEnd, FollowEvent,
    FollowItem, FollowOptions, Resumable, StreamStep, Supervised,
};
pub use frame::ListenFrame;
pub use listener::{Grant, Listener, Opened, OverflowPolicy};
pub use op_registry::Caller;
pub use run::listen_stream;
pub use stdout::{follow_to_stdout, stdout_sink};
