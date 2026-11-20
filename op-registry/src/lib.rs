//! op-registry: declare operations once, derive the adapters.

mod call;
pub mod cli;
mod descriptor;
pub mod mcp;
mod output;
mod params;
pub mod parity;
mod registry;

pub use call::{call, call_stream, CallError};
pub use call::{CallOptions, Caller, Cursor};
pub use descriptor::*;
pub use op_registry_derive::OpParams;
pub use output::OutputSchema;
pub use params::*;
pub use registry::{CliHooks, Operation, Registry};

#[doc(hidden)]
pub mod __private {
    /// This module is not API. Implementing this convention-level seal yourself
    /// voids the stability guarantee for `Operation` implementations.
    pub trait Sealed {}
    pub use crate::params::usage;
    pub use anyhow;
    pub use clap;
    pub use futures_util;
    #[cfg(feature = "schema")]
    pub use schemars;
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
        (destructive) => {
            $crate::Access::destructive()
        };
        (none) => {
            $crate::Access::default()
        };
    }
}
