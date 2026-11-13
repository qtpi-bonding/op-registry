//! op-registry: declare operations once, derive the adapters.

mod call;
mod descriptor;
mod params;
mod registry;

pub use call::{call, call_stream, CallError};
pub use call::{CallOptions, Cursor};
pub use descriptor::*;
pub use op_registry_derive::OpParams;
pub use params::*;
pub use registry::{Operation, Registry};

#[doc(hidden)]
pub mod __private {
    /// This module is not API. Implementing this convention-level seal yourself
    /// voids the stability guarantee for `Operation` implementations.
    pub trait Sealed {}
    pub use crate::params::usage;
    pub use anyhow;
    pub use clap;
    pub use futures_util;
    pub use serde_json;
    #[macro_export]
    macro_rules! access {
        (reads) => {
            $crate::Access::reads()
        };
        (writes) => {
            $crate::Access::writes()
        };
        (reads_writes) => {
            $crate::Access {
                reads: true,
                writes: true,
                ..$crate::Access::default()
            }
        };
        (none) => {
            $crate::Access::default()
        };
    }
}
