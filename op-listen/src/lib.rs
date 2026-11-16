//! op-listen: shared listener for op-registry.

pub mod backoff;
pub mod cursor;
pub mod detach;
pub mod entry;
pub mod error;
pub mod follow;
pub mod frame;
pub mod listener;
pub mod run;
pub mod stdout;

pub use backoff::Backoff;
pub use cursor::ListenCursor;
pub use detach::detached_connect;
pub use entry::listen_entry;
pub use error::{lagged, ListenError};
pub use follow::{follow, follow_typed, FollowEnd, FollowEvent, FollowOptions};
pub use frame::ListenFrame;
pub use listener::{Grant, Listener, Opened, OverflowPolicy};
pub use op_registry::Caller;
pub use run::{listen_stream, ListenOptions};
pub use stdout::{follow_to_stdout, stdout_sink};
