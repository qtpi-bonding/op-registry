use crate::Registry;
use anyhow::Result;
use futures_util::stream::BoxStream;
use serde_json::Value;
use std::future::Future;
use std::task::{Context, Poll};

#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct CallOptions {}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cursor(pub String);

pub(crate) fn run<F: Future>(future: F) -> F::Output {
    let waker = futures_util::task::noop_waker_ref();
    let mut context = Context::from_waker(waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// Errors raised by the registry dispatch layer.
#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "op_registry.call")]
#[non_exhaustive]
pub enum CallError {
    #[error("unknown operation {name}")]
    #[coded(code = "unknown_operation", class = Logical)]
    UnknownOperation { name: String },
    #[error("operation {name} has the wrong call shape")]
    #[coded(code = "wrong_shape", class = Logical)]
    WrongShape { name: String },
}

impl<C: Sync> Registry<C> {
    /// Calls a unary operation by name.
    pub fn call(&self, name: &str, args: Value, ctx: &C, opts: CallOptions) -> Result<Value> {
        let entry = self.entry(name)?;
        match &entry.call {
            crate::registry::EntryCall::Unary(call) => run(call(ctx, args, opts)),
            crate::registry::EntryCall::Stream(_) => {
                Err(coded_error::into_anyhow(CallError::WrongShape {
                    name: name.to_owned(),
                }))
            }
        }
    }

    /// Calls a streaming operation by name.
    pub fn call_stream<'a>(
        &self,
        name: &str,
        args: Value,
        ctx: &'a C,
        opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> Result<BoxStream<'a, Result<Value>>> {
        let entry = self.entry(name)?;
        match &entry.call {
            crate::registry::EntryCall::Stream(call) => run(call(ctx, args, opts, cursor)),
            crate::registry::EntryCall::Unary(_) => {
                Err(coded_error::into_anyhow(CallError::WrongShape {
                    name: name.to_owned(),
                }))
            }
        }
    }
}

/// Dispatches a unary operation.
pub fn call<C: Sync>(
    registry: &Registry<C>,
    name: &str,
    args: Value,
    ctx: &C,
    opts: CallOptions,
) -> Result<Value> {
    registry.call(name, args, ctx, opts)
}

/// Dispatches a streaming operation.
pub fn call_stream<'a, C: Sync>(
    registry: &'a Registry<C>,
    name: &str,
    args: Value,
    ctx: &'a C,
    opts: CallOptions,
    cursor: Option<Cursor>,
) -> Result<BoxStream<'a, Result<Value>>> {
    registry.call_stream(name, args, ctx, opts, cursor)
}
