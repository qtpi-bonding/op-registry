//! op-listen: shared listener for op-registry.

pub mod cursor;
pub mod entry;
pub mod error;
pub mod frame;
pub mod listener;
pub mod run;

pub use cursor::ListenCursor;
pub use entry::listen_entry;
pub use error::{lagged, ListenError};
pub use frame::ListenFrame;
pub use listener::{Grant, Listener, Opened, OverflowPolicy};
pub use op_registry::Caller;
pub use run::{listen_stream, ListenOptions};
