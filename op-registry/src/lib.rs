//! op-registry: declare operations once, derive the adapters.

mod descriptor;
mod params;

pub use descriptor::*;
pub use op_registry_derive::OpParams;
pub use params::*;

#[doc(hidden)]
pub mod __private {
    pub use crate::params::usage;
    pub use anyhow;
    pub use clap;
    pub use serde_json;
}
