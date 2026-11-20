//! The one call pipeline every path runs through.
//!
//! Steps: look up the op, check the path and shape, decode, validate, (resolve
//! refs, from a later task), build the `CallInfo`, run inside the hook, and
//! report a failure to the hook. Errors before the `CallInfo` exists (unknown
//! op, wrong path or shape, decode, validate) are returned without the hook:
//! there is no info to hand it.

use crate::registry::{Entry, Run, TypedStream, TypedUnary};
use crate::{CallError, CallInfo, CallOptions, CallPath, Cursor, Op, ParseCx, Paths, Registry};
use anyhow::Result;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use serde_json::Value;
use std::any::Any;
use std::sync::{Arc, Mutex};

fn wrong_shape(name: &str) -> anyhow::Error {
    coded_error::into_anyhow(CallError::WrongShape {
        name: name.to_owned(),
    })
}

impl<C: Sync> Registry<C> {
    /// Calls a unary operation by name with JSON arguments.
    ///
    /// The path label is `Transport(opts.transport)`, or `Transport("json")`.
    pub async fn call(&self, name: &str, args: Value, ctx: &C, opts: CallOptions) -> Result<Value> {
        let path = json_path(&opts);
        let params = self.decode_json(name, args, &path)?;
        self.call_decoded(name, params, ctx, opts, path).await
    }

    /// Calls a streaming operation by name with JSON arguments. The hook
    /// wraps the opening of the stream only.
    pub async fn call_stream<'a>(
        &'a self,
        name: &str,
        args: Value,
        ctx: &'a C,
        opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> Result<BoxStream<'a, Result<Value>>> {
        let path = json_path(&opts);
        let params = self.decode_json(name, args, &path)?;
        self.call_stream_decoded(name, params, ctx, opts, cursor, path)
            .await
    }

    fn decode_json(
        &self,
        name: &str,
        args: Value,
        path: &CallPath,
    ) -> Result<Box<dyn Any + Send>> {
        let entry = self.entry(name)?;
        check_path(entry, name, path)?;
        (entry.decode_json)(args, &mut ParseCx::default())
    }

    /// Runs a unary op on decoded parameters (the decoder has validated them).
    pub(crate) async fn call_decoded(
        &self,
        name: &str,
        params: Box<dyn Any + Send>,
        ctx: &C,
        opts: CallOptions,
        path: CallPath,
    ) -> Result<Value> {
        let entry = self.entry(name)?;
        check_path(entry, name, &path)?;
        let Run::Unary(run) = &entry.run else {
            return Err(wrong_shape(name));
        };
        // Step 4, resolving refs, joins here.
        let info = self.info(entry, &*params, path, &opts);
        self.pipeline(info, move || run(ctx, params, opts)).await
    }

    /// Runs a streaming op on decoded parameters.
    pub(crate) async fn call_stream_decoded<'a>(
        &'a self,
        name: &str,
        params: Box<dyn Any + Send>,
        ctx: &'a C,
        opts: CallOptions,
        cursor: Option<Cursor>,
        path: CallPath,
    ) -> Result<BoxStream<'a, Result<Value>>> {
        let entry = self.entry(name)?;
        check_path(entry, name, &path)?;
        let Run::Stream(run) = &entry.run else {
            return Err(wrong_shape(name));
        };
        let info = self.info(entry, &*params, path, &opts);
        self.pipeline(info, move || run(ctx, params, opts, cursor))
            .await
    }

    fn info(
        &self,
        entry: &Entry<C>,
        params: &(dyn Any + Send),
        path: CallPath,
        opts: &CallOptions,
    ) -> CallInfo {
        CallInfo {
            name: entry.descriptor.name.clone(),
            access: entry.descriptor.access,
            audience: entry.descriptor.audience,
            path,
            caller: opts.caller.clone(),
            fields: entry.fields.as_ref().map(|f| f(params)).unwrap_or_default(),
            trace: opts.trace.clone(),
        }
    }

    /// Runs `body` inside the hook, keeping the result in a slot the library
    /// owns, and reports a failure to the hook once.
    async fn pipeline<'a, T: Send + 'a>(
        &self,
        info: CallInfo,
        body: impl FnOnce() -> BoxFuture<'a, Result<T>> + Send + 'a,
    ) -> Result<T> {
        let slot: Arc<Mutex<Option<Result<T>>>> = Arc::new(Mutex::new(None));
        let writer = slot.clone();
        let call: BoxFuture<'a, ()> = Box::pin(async move {
            let outcome = body().await;
            *writer.lock().expect("the slot lock is never poisoned") = Some(outcome);
        });
        self.hook.around(&info, call).await;
        let taken = slot.lock().expect("the slot lock is never poisoned").take();
        let outcome =
            taken.unwrap_or_else(|| Err(anyhow::anyhow!("call hook did not run the call")));
        if let Err(err) = &outcome {
            self.hook.on_error(&info, err);
        }
        outcome
    }
}

impl<C: Sync + 'static> Registry<C> {
    /// Calls the operation `O::NAME` with typed parameters and output.
    pub async fn call_typed<O: Op<Ctx = C>>(
        &self,
        ctx: &C,
        params: O::Params,
        opts: CallOptions,
    ) -> Result<O::Output> {
        let entry = self.entry(O::NAME)?;
        let f = entry
            .typed
            .downcast_ref::<TypedUnary<C, O::Params, O::Output>>()
            .ok_or_else(|| wrong_shape(O::NAME))?;
        crate::OpParams::validate(&params)?;
        let info = self.info(entry, &params, CallPath::Typed, &opts);
        self.pipeline(info, move || f(ctx, params, opts)).await
    }

    /// Opens the streaming operation `O::NAME` with typed parameters. The
    /// hook wraps the opening of the stream only.
    pub async fn call_stream_typed<'a, O: Op<Ctx = C>>(
        &'a self,
        ctx: &'a C,
        params: O::Params,
        opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> Result<BoxStream<'a, Result<O::Output>>> {
        let entry = self.entry(O::NAME)?;
        let f = entry
            .typed
            .downcast_ref::<TypedStream<C, O::Params, O::Output>>()
            .ok_or_else(|| wrong_shape(O::NAME))?;
        crate::OpParams::validate(&params)?;
        let info = self.info(entry, &params, CallPath::Typed, &opts);
        self.pipeline(info, move || f(ctx, params, cursor, opts))
            .await
    }
}

fn json_path(opts: &CallOptions) -> CallPath {
    CallPath::Transport(opts.transport.clone().unwrap_or_else(|| "json".to_owned()))
}

/// Rejects a `TypedOnly` op on any path but the typed one.
fn check_path<C>(entry: &Entry<C>, name: &str, path: &CallPath) -> Result<()> {
    if entry.descriptor.paths == Paths::TypedOnly && *path != CallPath::Typed {
        return Err(coded_error::into_anyhow(CallError::WrongPath {
            name: name.to_owned(),
        }));
    }
    Ok(())
}
