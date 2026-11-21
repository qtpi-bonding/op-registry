//! op-listen: shared listener for op-registry.

pub mod backoff;
pub mod cursor;
#[cfg(feature = "detach")]
pub mod detach;
#[cfg(feature = "registry")]
pub mod entry;
pub mod error;
pub mod follow;
pub mod frame;
#[cfg(feature = "registry")]
pub mod listener;
#[cfg(feature = "registry")]
pub mod run;
pub mod stdout;
#[cfg(feature = "testkit")]
pub mod testkit;

pub use backoff::Backoff;
pub use cursor::ListenCursor;
#[cfg(feature = "detach")]
pub use detach::detached_connect;
#[cfg(feature = "registry")]
pub use entry::{listen_entry, listen_entry_following};
pub use error::{lagged, ListenError};
pub use follow::{
    follow, follow_items, follow_stream, follow_typed, supervise, supervise_stream, FollowEnd,
    FollowEvent, FollowItem, FollowOptions, Followed, RedactFn, Resumable, RetryHook, StreamStep,
    Supervised,
};
pub use frame::ListenFrame;
#[cfg(feature = "registry")]
pub use listener::{Grant, Listener, Opened, OverflowPolicy};
#[cfg(feature = "registry")]
pub use op_registry::Caller;
#[cfg(feature = "registry")]
pub use run::listen_stream;
pub use stdout::{follow_to_stdout, stdout_sink};
