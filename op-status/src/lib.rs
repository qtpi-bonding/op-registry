//! op-status: shared status types and a `status get` operation.

pub mod source;
pub mod types;

pub mod check;
pub use check::*;
pub mod open;
pub use open::*;
pub use source::{SeqWindow, StatusSource};
pub use types::*;

#[cfg(feature = "registry")]
pub mod ops;

#[cfg(feature = "listen")]
pub mod listen;
#[cfg(feature = "listen")]
pub use listen::StatusListener;
