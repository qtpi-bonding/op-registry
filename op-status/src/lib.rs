//! op-status: shared status types and a `status get` operation.

pub mod source;
pub mod types;

pub mod check;
pub use source::StatusSource;
pub use types::*;
pub use check::*;
