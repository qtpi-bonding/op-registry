//! op-registry: declare operations once, derive the adapters.

mod call;
pub mod cli;
mod descriptor;
pub mod mcp;
mod output;
mod params;
pub mod parity;
mod registry;

pub use call::CallError;
pub use call::{CallOptions, Caller, Cursor};
pub use descriptor::*;
pub use op_registry_derive::OpParams;
pub use output::OutputSchema;
pub use params::*;
pub use registry::{OpSpec, Registry};

#[doc(hidden)]
pub mod __private {
    pub use crate::params::rt;
    pub use crate::params::usage;
    pub use anyhow;
    pub use clap;
    pub use futures_util;
    #[cfg(feature = "schema")]
    pub use schemars;
    pub use serde_json;
}
