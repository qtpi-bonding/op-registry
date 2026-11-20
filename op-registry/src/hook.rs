//! The call hook and the facts it sees about each call.

use crate::{Access, Audience, Caller, Redactor, Surface, TraceParent};
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

impl CallPath {
    /// The surface this path is a call on.
    pub fn surface(&self) -> Surface {
        match self {
            CallPath::Typed => Surface::Typed,
            CallPath::Cli => Surface::Cli,
            CallPath::Mcp => Surface::Mcp,
            CallPath::Transport(_) => Surface::Json,
        }
    }
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
    /// The `key: value` facts the op's `fields` function gave for its
    /// parameters, before their refs are resolved (a ref shows its raw text).
    pub fields: Vec<(&'static str, String)>,
    /// The trace context, passed through untouched.
    pub trace: TraceParent,
    /// The call's secrets, for [`CallInfo::redact`].
    pub(crate) redactor: Redactor,
}

impl CallInfo {
    /// `text` with every secret of this call replaced by `<secret>`: the
    /// values of its `#[op(secret)]` parameters, the contents of its
    /// `SecretFile`s and, on the command line, its secret globals.
    pub fn redact(&self, text: &str) -> String {
        self.redactor.redact(text)
    }
}

/// Wraps every call of a registry.
pub trait CallHook: Send + Sync {
    /// Wraps the call. A hook that never polls `call` makes the call fail
    /// with an uncoded error; it does not hang.
    fn around<'a>(&'a self, info: &CallInfo, call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()> {
        let _ = info;
        call
    }

    /// Called exactly once when the call fails after its `CallInfo` is built:
    /// a ref that does not resolve or a failure of the op. Decode and
    /// validation errors come before the info and do not reach it.
    ///
    /// `err` is the raw error and may carry secret values: pass any text
    /// taken from it through [`CallInfo::redact`] before it is logged.
    fn on_error(&self, info: &CallInfo, err: &anyhow::Error) {
        let _ = (info, err);
    }
}

/// The default hook: does nothing.
pub struct NoHook;

impl CallHook for NoHook {}
