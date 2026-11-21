//! The one call pipeline every path runs through.
//!
//! Steps: look up the op, check the path and shape, decode, validate, build
//! the `CallInfo` from the unresolved parameters, then inside the hook resolve
//! refs (non-subject refs first, then the subject) and run, and report a
//! failure to the hook. Errors before the `CallInfo` exists (unknown op, wrong
//! path or shape, decode, validate) are returned without the hook: there is
//! no info to hand it. A resolution failure reaches the hook.

use crate::registry::{Entry, Run, TypedStream, TypedUnary};
use crate::{
    CallError, CallFailure, CallInfo, CallOptions, CallPath, Cursor, Op, OpParams, ParseCx,
    Redactor, RefSlot, Registry, Resolve, Resolved, Surface,
};
use anyhow::Result;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use serde_json::Value;
use std::any::Any;
use std::sync::{Arc, Mutex};

/// The outcome of a call together with the redactor built from its
/// parameters, so an adapter can redact the error message it renders.
pub(crate) struct Invocation<T> {
    pub result: Result<T>,
    pub redactor: Redactor,
    /// The process exit code of a successful call (0 unless the op declares
    /// one); meaningless for a failure.
    pub exit: u8,
    /// The text rendering, when asked for and the op has one.
    pub text: Option<String>,
}

impl<T> Invocation<T> {
    /// A failure before any parameters exist: only what the caller's options
    /// already hold is redacted.
    fn failed(err: anyhow::Error, redactor: Redactor) -> Self {
        Self {
            result: Err(err),
            redactor,
            exit: 0,
            text: None,
        }
    }
}

/// The one redactor of a call: the handle the caller's options already
/// carry, extended with every secret the parameters hold. The handle is never
/// replaced, so an op that passes a clone of its options to a nested call
/// shares it with that call. Every output site of the call gets a clone.
fn call_redactor(opts: &CallOptions, secrets: Vec<Vec<u8>>) -> Redactor {
    let redactor = opts.redactor.clone();
    for secret in secrets {
        redactor.add(&secret);
    }
    redactor
}

/// Applies the registry's error mapper to an error, except a usage error
/// (decode, validate or `check_id`), which is never mapped.
fn map_error(
    mapper: Option<fn(anyhow::Error) -> anyhow::Error>,
    err: anyhow::Error,
) -> anyhow::Error {
    match mapper {
        Some(mapper)
            if !coded_error::facts_anyhow(&err)
                .code
                .starts_with("op_registry.usage.") =>
        {
            mapper(err)
        }
        _ => err,
    }
}

/// Maps each error item of a stream with the registry's mapper.
fn map_items<'a, T: Send + 'a>(
    mapper: Option<fn(anyhow::Error) -> anyhow::Error>,
    stream: BoxStream<'a, Result<T>>,
) -> BoxStream<'a, Result<T>> {
    match mapper {
        Some(_) => Box::pin(stream.map(move |item| item.map_err(|err| map_error(mapper, err)))),
        None => stream,
    }
}

/// Wraps each error item of a stream in a [`CallFailure`] with `redactor`.
fn failures<'a, T: Send + 'a>(
    stream: BoxStream<'a, Result<T>>,
    redactor: Redactor,
) -> BoxStream<'a, std::result::Result<T, CallFailure>> {
    Box::pin(stream.map(move |item| item.map_err(|err| CallFailure::new(err, redactor.clone()))))
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
    /// A failure carries the call's redactor: render it with
    /// [`CallFailure::redacted_message`] or its `Display`.
    pub async fn call(
        &self,
        name: &str,
        args: Value,
        ctx: &C,
        opts: CallOptions,
    ) -> std::result::Result<Value, CallFailure> {
        let path = json_path(&opts);
        let invocation = self.call_json(name, args, ctx, opts, path, false).await;
        invocation
            .result
            .map_err(|error| CallFailure::new(error, invocation.redactor))
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
        want_text: bool,
    ) -> Invocation<Value> {
        match self.decode_json(name, args, &path, false) {
            Ok(params) => {
                self.call_decoded(name, params, ctx, opts, path, want_text)
                    .await
            }
            Err(err) => Invocation::failed(err, opts.redactor),
        }
    }

    /// Calls a streaming operation by name with JSON arguments. The hook
    /// wraps the opening of the stream only. Every failure, the open's and
    /// each item's, is a [`CallFailure`] carrying the call's redactor.
    pub async fn call_stream<'a>(
        &'a self,
        name: &str,
        args: Value,
        ctx: &'a C,
        opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> std::result::Result<BoxStream<'a, std::result::Result<Value, CallFailure>>, CallFailure>
    {
        let path = json_path(&opts);
        let params = self
            .decode_json(name, args, &path, true)
            .map_err(|error| CallFailure::new(error, opts.redactor.clone()))?;
        let invocation = self
            .call_stream_decoded(name, params, ctx, opts, cursor, path)
            .await;
        let redactor = invocation.redactor;
        match invocation.result {
            Ok(stream) => Ok(failures(stream, redactor)),
            Err(error) => Err(CallFailure::new(error, redactor)),
        }
    }

    /// Decodes JSON arguments and opens a stream op on `path`. The kit's
    /// stream calls use it; the redactor holds only the call's own secrets.
    #[cfg(feature = "testing")]
    pub(crate) async fn call_stream_json<'a>(
        &'a self,
        name: &str,
        args: Value,
        ctx: &'a C,
        opts: CallOptions,
        cursor: Option<Cursor>,
        path: CallPath,
    ) -> Invocation<BoxStream<'a, Result<Value>>> {
        match self.decode_json(name, args, &path, true) {
            Ok(params) => {
                self.call_stream_decoded(name, params, ctx, opts, cursor, path)
                    .await
            }
            Err(err) => Invocation::failed(err, opts.redactor),
        }
    }

    fn decode_json(
        &self,
        name: &str,
        args: Value,
        path: &CallPath,
        stream: bool,
    ) -> Result<Box<dyn Any + Send>> {
        let entry = self.entry(name)?;
        check_path(entry, name, path.surface())?;
        if matches!(entry.run, Run::Stream(_)) != stream {
            return Err(wrong_shape(name));
        }
        (entry.decode_json)(args, &mut ParseCx::default())
    }

    /// Runs a unary op on decoded parameters (the decoder has validated them).
    /// What the caller knows to be secret besides the parameters (the command
    /// line's secret globals) is already in `opts`' redactor.
    pub(crate) async fn call_decoded(
        &self,
        name: &str,
        mut params: Box<dyn Any + Send>,
        ctx: &C,
        mut opts: CallOptions,
        path: CallPath,
        want_text: bool,
    ) -> Invocation<Value> {
        let entry = match self.entry(name) {
            Ok(entry) => entry,
            Err(err) => return Invocation::failed(err, opts.redactor),
        };
        if let Err(err) = check_path(entry, name, path.surface()) {
            return Invocation::failed(err, opts.redactor);
        }
        let Run::Unary(run) = &entry.run else {
            return Invocation::failed(wrong_shape(name), opts.redactor);
        };
        let redactor = call_redactor(&opts, (entry.secrets)(&*params));
        opts.notices = opts.notices.redacting(&redactor);
        opts.path = Some(path.clone());
        let info = self.info(entry, &*params, path, &opts, &redactor);
        let result = self
            .pipeline(info, move || {
                Box::pin(async move {
                    (entry.resolve_refs)(self, ctx, &mut *params).await?;
                    run(self, ctx, params, opts, want_text).await
                })
            })
            .await;
        match result {
            Ok(ran) => Invocation {
                result: Ok(ran.value),
                redactor,
                exit: crate::exit::ok_exit(&entry.descriptor.exits, ran.exit),
                text: ran.text,
            },
            Err(err) => {
                self.strict(name, &err);
                Invocation {
                    result: Err(err),
                    redactor,
                    exit: 0,
                    text: None,
                }
            }
        }
    }

    /// Runs a streaming op on decoded parameters; the redactor as for
    /// [`Registry::call_decoded`].
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
            Err(err) => return Invocation::failed(err, opts.redactor),
        };
        if let Err(err) = check_path(entry, name, path.surface()) {
            return Invocation::failed(err, opts.redactor);
        }
        let Run::Stream(run) = &entry.run else {
            return Invocation::failed(wrong_shape(name), opts.redactor);
        };
        let redactor = call_redactor(&opts, (entry.secrets)(&*params));
        opts.notices = opts.notices.redacting(&redactor);
        opts.path = Some(path.clone());
        let info = self.info(entry, &*params, path, &opts, &redactor);
        let result = self
            .pipeline(info, move || {
                Box::pin(async move {
                    (entry.resolve_refs)(self, ctx, &mut *params).await?;
                    run(ctx, params, opts, cursor).await
                })
            })
            .await;
        if let Err(err) = &result {
            self.strict(name, err);
        }
        let result = result.map(|stream| map_items(self.error_mapper, stream));
        Invocation {
            result,
            redactor,
            exit: 0,
            text: None,
        }
    }

    /// Under `testing::strict_codes`, panics when `err` is not one of the
    /// op's declared codes.
    #[cfg(feature = "testing")]
    fn strict(&self, name: &str, err: &anyhow::Error) {
        if self.strict_codes {
            crate::testing::check_strict(self, name, err);
        }
    }

    #[cfg(not(feature = "testing"))]
    fn strict(&self, _name: &str, _err: &anyhow::Error) {}

    /// Resolves the refs of `params` in place: non-subject refs first, then
    /// the subject (whose scope reads the siblings' resolved ids).
    pub(crate) async fn resolve_refs<P: OpParams>(&self, ctx: &C, params: &mut P) -> Result<()> {
        for subject in [false, true] {
            for slot in params.ref_slots()? {
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
            absent_ok,
            scope,
            slot,
            ..
        } = slot;
        let raw = slot.as_raw().to_owned();
        let resolved = if id_only || absent_ok {
            if !self.server_kinds.contains(kind) {
                ctx.check_id(kind, &raw).map_err(crate::usage)?;
            }
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
        redactor: &Redactor,
    ) -> CallInfo {
        CallInfo {
            name: entry.descriptor.name.clone(),
            access: entry.descriptor.access,
            audience: opts.serving.unwrap_or(entry.descriptor.audience),
            path,
            caller: opts.caller.clone(),
            fields: entry.fields.as_ref().map(|f| f(params)).unwrap_or_default(),
            trace: opts.trace.clone(),
            redactor: redactor.clone(),
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
        let outcome = taken
            .unwrap_or_else(|| Err(anyhow::anyhow!("call hook did not run the call")))
            .map_err(|err| map_error(self.error_mapper, err));
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
    ) -> std::result::Result<O::Output, CallFailure> {
        let plain = |error: anyhow::Error| CallFailure::new(error, opts.redactor.clone());
        let entry = self.entry(O::NAME).map_err(plain)?;
        check_path(entry, O::NAME, Surface::Typed).map_err(plain)?;
        let f = entry
            .typed
            .downcast_ref::<TypedUnary<C, O::Params, O::Output>>()
            .ok_or_else(|| plain(wrong_shape(O::NAME)))?;
        let redactor = call_redactor(&opts, params.secrets());
        crate::OpParams::validate(&params)
            .map_err(|error| CallFailure::new(error, redactor.clone()))?;
        opts.notices = opts.notices.redacting(&redactor);
        opts.path = Some(CallPath::Typed);
        let info = self.info(entry, &params, CallPath::Typed, &opts, &redactor);
        let result = self
            .pipeline(info, move || {
                Box::pin(async move {
                    self.resolve_refs(ctx, &mut params).await?;
                    f(self, ctx, params, opts).await
                })
            })
            .await;
        if let Err(err) = &result {
            self.strict(O::NAME, err);
        }
        result.map_err(|error| CallFailure::new(error, redactor))
    }

    /// Opens the streaming operation `O::NAME` with typed parameters. The
    /// hook wraps the opening of the stream only.
    pub async fn call_stream_typed<'a, O: Op<Ctx = C>>(
        &'a self,
        ctx: &'a C,
        params: O::Params,
        opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> std::result::Result<BoxStream<'a, std::result::Result<O::Output, CallFailure>>, CallFailure>
    {
        self.open_typed_stream::<O>(ctx, params, opts, cursor).await
    }

    /// The one pipeline of the typed stream calls: path check, validate, the
    /// hook around the open, the error mapper and the failure wrapping.
    async fn open_typed_stream<'a, O: Op<Ctx = C>>(
        &'a self,
        ctx: &'a C,
        mut params: O::Params,
        mut opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> std::result::Result<BoxStream<'a, std::result::Result<O::Output, CallFailure>>, CallFailure>
    {
        let plain = |error: anyhow::Error| CallFailure::new(error, opts.redactor.clone());
        let entry = self.entry(O::NAME).map_err(plain)?;
        check_path(entry, O::NAME, Surface::Typed).map_err(plain)?;
        let f = entry
            .typed
            .downcast_ref::<TypedStream<C, O::Params, O::Output>>()
            .ok_or_else(|| plain(wrong_shape(O::NAME)))?;
        let redactor = call_redactor(&opts, params.secrets());
        crate::OpParams::validate(&params)
            .map_err(|error| CallFailure::new(error, redactor.clone()))?;
        opts.notices = opts.notices.redacting(&redactor);
        opts.path = Some(CallPath::Typed);
        let info = self.info(entry, &params, CallPath::Typed, &opts, &redactor);
        let result = self
            .pipeline(info, move || {
                Box::pin(async move {
                    self.resolve_refs(ctx, &mut params).await?;
                    f(ctx, params, cursor, opts).await
                })
            })
            .await;
        if let Err(err) = &result {
            self.strict(O::NAME, err);
        }
        match result {
            Ok(stream) => Ok(failures(map_items(self.error_mapper, stream), redactor)),
            Err(error) => Err(CallFailure::new(error, redactor)),
        }
    }
}

impl<C: Resolve + Send + Sync + 'static> Registry<C> {
    /// Like [`call_stream_typed`](Self::call_stream_typed), but the stream
    /// holds the registry and the context itself, so it can be moved into a
    /// spawned task after the caller drops its handles.
    pub async fn call_stream_typed_owned<O: Op<Ctx = C>>(
        self: Arc<Self>,
        ctx: Arc<C>,
        params: O::Params,
        opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> std::result::Result<
        BoxStream<'static, std::result::Result<O::Output, CallFailure>>,
        CallFailure,
    >
    where
        O::Output: 'static,
        O::Params: 'static,
    {
        // SAFETY: the references are extended to `'static` only to open the
        // stream; the stream is stored in `Owned` before the `Arc`s it
        // borrows from, so it is dropped first, and an `Arc` never moves its
        // pointee. Nothing else can reach the extended references.
        let (registry_ref, ctx_ref): (&'static Self, &'static C) =
            unsafe { (&*Arc::as_ptr(&self), &*Arc::as_ptr(&ctx)) };
        let stream = registry_ref
            .open_typed_stream::<O>(ctx_ref, params, opts, cursor)
            .await?;
        Ok(Box::pin(Owned {
            stream,
            _ctx: ctx,
            _registry: self,
        }))
    }
}

/// A stream together with the owners of what it borrows. The stream field is
/// declared first so it is dropped before them.
struct Owned<T, C, R> {
    stream: BoxStream<'static, T>,
    _ctx: Arc<C>,
    _registry: Arc<R>,
}

impl<T, C, R> futures_util::Stream for Owned<T, C, R> {
    type Item = T;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<T>> {
        self.stream.poll_next_unpin(cx)
    }
}

fn json_path(opts: &CallOptions) -> CallPath {
    CallPath::Transport(opts.transport.clone().unwrap_or_else(|| "json".to_owned()))
}

/// Rejects a call on a surface the op does not declare in its `paths`.
pub(crate) fn check_path<C>(entry: &Entry<C>, name: &str, surface: Surface) -> Result<()> {
    if !entry.descriptor.paths.contains(surface) {
        return Err(coded_error::into_anyhow(CallError::WrongPath {
            name: name.to_owned(),
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpSpec, Paths};
    use futures_util::future::BoxFuture;
    use serde::Serialize;
    use serde_json::json;

    struct Ctx;
    impl Resolve for Ctx {}

    #[derive(OpParams)]
    struct P {
        code: Option<u8>,
    }

    #[derive(Serialize)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    struct Out {
        code: u8,
    }

    fn registry() -> Registry<Ctx> {
        let mut r = Registry::<Ctx>::new();
        let mut spec = OpSpec::<P, Out>::new("probe run", "Probe.");
        spec.paths = Paths::ALL;
        spec.exits = vec![(4, "four")];
        spec.ok_exit = Some(|o| o.code);
        spec.render_text = Some(|o| format!("code {}", o.code));
        r.group("probe", "Probes.");
        r.push_op(
            spec,
            |_c: &Ctx, p: P, _o: CallOptions| -> BoxFuture<'_, Result<Out>> {
                Box::pin(async move {
                    Ok(Out {
                        code: p.code.unwrap_or(0),
                    })
                })
            },
        );
        r.push_op(
            OpSpec::<P, Out>::new("probe plain", "Plain."),
            |_c: &Ctx, p: P, _o: CallOptions| -> BoxFuture<'_, Result<Out>> {
                Box::pin(async move {
                    Ok(Out {
                        code: p.code.unwrap_or(0),
                    })
                })
            },
        );
        r
    }

    fn run(name: &str, code: u8, want_text: bool) -> Invocation<Value> {
        futures_executor::block_on(registry().call_json(
            name,
            json!({"code": code}),
            &Ctx,
            CallOptions::default(),
            CallPath::Cli,
            want_text,
        ))
    }

    #[test]
    fn declared_ok_exit_returned() {
        let inv = run("probe_run", 4, false);
        assert_eq!(inv.result.unwrap(), json!({"code": 4}));
        assert_eq!(inv.exit, 4);
        assert_eq!(run("probe_run", 0, false).exit, 0);
    }

    #[test]
    fn undeclared_ok_exit_is_70() {
        assert_eq!(run("probe_run", 9, false).exit, 70);
        assert_eq!(run("probe_run", 64, false).exit, 70);
    }

    #[test]
    fn op_without_ok_exit_exits_0() {
        assert_eq!(run("probe_plain", 4, true).exit, 0);
    }

    #[test]
    fn text_rendered_only_when_asked() {
        assert_eq!(run("probe_run", 4, true).text.as_deref(), Some("code 4"));
        assert_eq!(run("probe_run", 4, false).text, None);
        assert_eq!(run("probe_plain", 4, true).text, None);
        assert!(registry().get("probe_run").unwrap().text);
        assert!(!registry().get("probe_plain").unwrap().text);
    }
}
