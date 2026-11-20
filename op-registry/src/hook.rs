//! The call hook and the facts it sees about each call.

use crate::{Access, Audience, Caller, TraceParent};
use futures_util::future::BoxFuture;

/// The path a call took to reach the operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallPath {
    /// A typed Rust call.
    Typed,
    /// The command line.
    Cli,
    /// An MCP tool call.
    Mcp,
    /// A named JSON transport.
    Transport(String),
}

/// What a hook is told about one call.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct CallInfo {
    /// The operation name.
    pub name: String,
    /// The access the operation needs.
    pub access: Access,
    /// The operation's audience.
    pub audience: Audience,
    /// The path the call took.
    pub path: CallPath,
    /// Who is calling.
    pub caller: Option<Caller>,
    /// The `key: value` facts the op's `fields` function gave for its parameters.
    pub fields: Vec<(&'static str, String)>,
    /// The trace context, passed through untouched.
    pub trace: TraceParent,
}

/// Wraps every call of a registry.
pub trait CallHook: Send + Sync {
    /// Wraps the call. A hook that never polls `call` makes the call fail
    /// with an uncoded error; it does not hang.
    fn around<'a>(&'a self, info: &CallInfo, call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()> {
        let _ = info;
        call
    }

    /// Called exactly once when the call fails after its `CallInfo` is built.
    fn on_error(&self, info: &CallInfo, err: &anyhow::Error) {
        let _ = (info, err);
    }
}

/// The default hook: does nothing.
pub struct NoHook;

impl CallHook for NoHook {}
