//! op-registry: declare operations once, derive the adapters.

mod call;
pub mod cli;
mod descriptor;
pub mod mcp;
mod params;
pub mod parity;
mod registry;

pub use call::{call, call_stream, CallError};
pub use call::{CallOptions, Caller, Cursor};
pub use descriptor::*;
pub use op_registry_derive::OpParams;
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
    pub use schemars;
    pub use serde_json;

    /// Returns the JSON schema generated for an operation output type.
    pub fn output_schema_of<O: schemars::JsonSchema>() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(O)).unwrap()
    }
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
            $crate::Access::default()
        };
        (none) => {
            $crate::Access::default()
        };
    }
}
