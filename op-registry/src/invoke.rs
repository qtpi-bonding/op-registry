//! The one call pipeline every path runs through.
//!
//! Steps: look up the op, check the path and shape, decode, validate, resolve
//! refs (non-subject refs first, then the subject), build the `CallInfo`, run inside the hook, and
//! report a failure to the hook. Errors before the `CallInfo` exists (unknown
//! op, wrong path or shape, decode, validate) are returned without the hook:
//! there is no info to hand it.

use crate::registry::{Entry, Run, TypedStream, TypedUnary};
use crate::{
    CallError, CallInfo, CallOptions, CallPath, Cursor, Op, OpParams, ParseCx, Paths, Redactor,
    RefSlot, Registry, Resolve, Resolved,
};
use anyhow::Result;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use serde_json::Value;
use std::any::Any;
use std::sync::{Arc, Mutex};

/// The outcome of a call together with the redactor built from its
/// parameters, so an adapter can redact the error message it renders.
// The MCP adapter reads `redactor`; the CLI adapter will too (Task 14).
#[allow(dead_code)]
pub(crate) struct Invocation<T> {
    pub result: Result<T>,
    pub redactor: Redactor,
}

impl<T> Invocation<T> {
    /// A failure before any parameters exist: nothing to redact.
    fn failed(err: anyhow::Error) -> Self {
        Self {
            result: Err(err),
            redactor: Redactor::default(),
        }
    }
}

/// The redactor for a call: every secret its parameters hold.
fn redactor_of(secrets: Vec<Vec<u8>>) -> Redactor {
    let mut redactor = Redactor::default();
    for secret in secrets {
        redactor.add(&secret);
    }
    redactor
}

fn wrong_shape(name: &str) -> anyhow::Error {
    coded_error::into_anyhow(CallError::WrongShape {
        name: name.to_owned(),
    })
}

impl<C: Resolve> Registry<C> {
    /// Calls a unary operation by name with JSON arguments.
    ///
    /// The path label is `Transport(opts.transport)`, or `Transport("json")`.
    pub async fn call(&self, name: &str, args: Value, ctx: &C, opts: CallOptions) -> Result<Value> {
        let path = json_path(&opts);
        self.call_json(name, args, ctx, opts, path).await.result
    }

    /// Decodes JSON arguments and runs a unary op on `path`, keeping the
    /// call's redactor for the adapter that renders its error.
    pub(crate) async fn call_json(
        &self,
        name: &str,
        args: Value,
        ctx: &C,
        opts: CallOptions,
        path: CallPath,
    ) -> Invocation<Value> {
        match self.decode_json(name, args, &path, false) {
            Ok(params) => self.call_decoded(name, params, ctx, opts, path).await,
            Err(err) => Invocation::failed(err),
        }
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
        let params = self.decode_json(name, args, &path, true)?;
        self.call_stream_decoded(name, params, ctx, opts, cursor, path)
            .await
            .result
    }

    fn decode_json(
        &self,
        name: &str,
        args: Value,
        path: &CallPath,
        stream: bool,
    ) -> Result<Box<dyn Any + Send>> {
        let entry = self.entry(name)?;
        check_path(entry, name, path)?;
        if matches!(entry.run, Run::Stream(_)) != stream {
            return Err(wrong_shape(name));
        }
        (entry.decode_json)(args, &mut ParseCx::default())
    }

    /// Runs a unary op on decoded parameters (the decoder has validated them).
    pub(crate) async fn call_decoded(
        &self,
        name: &str,
        mut params: Box<dyn Any + Send>,
        ctx: &C,
        mut opts: CallOptions,
        path: CallPath,
    ) -> Invocation<Value> {
        let entry = match self.entry(name) {
            Ok(entry) => entry,
            Err(err) => return Invocation::failed(err),
        };
        if let Err(err) = check_path(entry, name, &path) {
            return Invocation::failed(err);
        }
        let Run::Unary(run) = &entry.run else {
            return Invocation::failed(wrong_shape(name));
        };
        let redactor = redactor_of((entry.secrets)(&*params));
        opts.notices = opts.notices.redacting(&redactor);
        let result = async {
            (entry.resolve_refs)(self, ctx, &mut *params).await?;
            let info = self.info(entry, &*params, path, &opts);
            self.pipeline(info, move || run(ctx, params, opts)).await
        }
        .await;
        Invocation { result, redactor }
    }

    /// Runs a streaming op on decoded parameters.
    pub(crate) async fn call_stream_decoded<'a>(
        &'a self,
        name: &str,
        mut params: Box<dyn Any + Send>,
        ctx: &'a C,
        mut opts: CallOptions,
        cursor: Option<Cursor>,
        path: CallPath,
    ) -> Invocation<BoxStream<'a, Result<Value>>> {
        let entry = match self.entry(name) {
            Ok(entry) => entry,
            Err(err) => return Invocation::failed(err),
        };
        if let Err(err) = check_path(entry, name, &path) {
            return Invocation::failed(err);
        }
        let Run::Stream(run) = &entry.run else {
            return Invocation::failed(wrong_shape(name));
        };
        let redactor = redactor_of((entry.secrets)(&*params));
        opts.notices = opts.notices.redacting(&redactor);
        let result = async {
            (entry.resolve_refs)(self, ctx, &mut *params).await?;
            let info = self.info(entry, &*params, path, &opts);
            self.pipeline(info, move || run(ctx, params, opts, cursor))
                .await
        }
        .await;
        Invocation { result, redactor }
    }

    /// Resolves the refs of `params` in place: non-subject refs first, then
    /// the subject (whose scope reads the siblings' resolved ids).
    pub(crate) async fn resolve_refs<P: OpParams>(&self, ctx: &C, params: &mut P) -> Result<()> {
        for subject in [false, true] {
            for slot in params.ref_slots() {
                if slot.subject == subject {
                    self.resolve_slot(ctx, slot).await?;
                }
            }
        }
        Ok(())
    }

    async fn resolve_slot(&self, ctx: &C, slot: RefSlot<'_>) -> Result<()> {
        let RefSlot {
            kind,
            id_only,
            scope,
            slot,
            ..
        } = slot;
        let raw = slot.as_raw().to_owned();
        let resolved = if id_only {
            Resolved::Id(raw)
        } else if self.server_kinds.contains(kind) {
            Resolved::Raw(raw)
        } else {
            ctx.resolve(kind, &*slot, &scope).await?
        };
        slot.set_resolved(resolved);
        Ok(())
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

impl<C: Resolve + 'static> Registry<C> {
    /// Calls the operation `O::NAME` with typed parameters and output.
    pub async fn call_typed<O: Op<Ctx = C>>(
        &self,
        ctx: &C,
        mut params: O::Params,
        mut opts: CallOptions,
    ) -> Result<O::Output> {
        let entry = self.entry(O::NAME)?;
        let f = entry
            .typed
            .downcast_ref::<TypedUnary<C, O::Params, O::Output>>()
            .ok_or_else(|| wrong_shape(O::NAME))?;
        crate::OpParams::validate(&params)?;
        opts.notices = opts.notices.redacting(&redactor_of(params.secrets()));
        self.resolve_refs(ctx, &mut params).await?;
        let info = self.info(entry, &params, CallPath::Typed, &opts);
        self.pipeline(info, move || f(ctx, params, opts)).await
    }

    /// Opens the streaming operation `O::NAME` with typed parameters. The
    /// hook wraps the opening of the stream only.
    pub async fn call_stream_typed<'a, O: Op<Ctx = C>>(
        &'a self,
        ctx: &'a C,
        mut params: O::Params,
        mut opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> Result<BoxStream<'a, Result<O::Output>>> {
        let entry = self.entry(O::NAME)?;
        let f = entry
            .typed
            .downcast_ref::<TypedStream<C, O::Params, O::Output>>()
            .ok_or_else(|| wrong_shape(O::NAME))?;
        crate::OpParams::validate(&params)?;
        opts.notices = opts.notices.redacting(&redactor_of(params.secrets()));
        self.resolve_refs(ctx, &mut params).await?;
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
