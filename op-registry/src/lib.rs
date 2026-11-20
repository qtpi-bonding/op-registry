//! op-registry: declare operations once, derive the adapters.

mod call;
pub mod cli;
mod descriptor;
mod hook;
mod invoke;
mod list;
pub mod mcp;
mod notices;
mod output;
mod params;
pub mod parity;
mod refs;
mod registry;

pub use call::CallError;
pub use call::{CallOptions, Caller, Cursor, Op, TraceParent};
pub use descriptor::*;
pub use hook::{CallHook, CallInfo, CallPath, NoHook};
pub use list::{ListItem, Page, PageParams, PageRequest, TooMany};
pub use notices::{Notice, NoticeSink, Notices};
pub use op_registry_derive::OpParams;
pub use output::{write_private, OutputSchema};
pub use params::*;
pub use refs::{Ref, RefSlot, Resolve, Resolved, Scope};
pub use registry::{OpSpec, Registry};

/// Fails the build of a `SecretFile` field when the `secrets` feature is off.
#[cfg(not(feature = "secrets"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __require_secrets {
    () => {
        compile_error!("a `SecretFile` field needs op-registry's `secrets` feature: enable it");
    };
}

/// Nothing to check when the `secrets` feature is on.
#[cfg(feature = "secrets")]
#[doc(hidden)]
#[macro_export]
macro_rules! __require_secrets {
    () => {};
}

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
