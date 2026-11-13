use crate::{CallOptions, Cursor, Descriptor, OpParams};
use anyhow::Result;
use futures_util::{future::BoxFuture, stream::BoxStream};
use serde_json::Value;
use std::sync::Arc;

/// A registered operation's typed contract. Only `registry!` implements it.
pub trait Operation: crate::__private::Sealed {
    type Ctx;
    type Params: OpParams;
    type Output: serde::Serialize + schemars::JsonSchema;
    fn descriptor() -> Descriptor;
}

type UnaryCall<C> =
    Box<dyn for<'a> Fn(&'a C, Value, CallOptions) -> BoxFuture<'a, Result<Value>> + Send + Sync>;
type StreamCall<C> = Box<
    dyn for<'a> Fn(
            &'a C,
            Value,
            CallOptions,
            Option<Cursor>,
        ) -> BoxFuture<'a, Result<BoxStream<'a, Result<Value>>>>
        + Send
        + Sync,
>;

pub(crate) enum EntryCall<C> {
    Unary(UnaryCall<C>),
    Stream(StreamCall<C>),
}

pub(crate) struct Entry<C> {
    pub descriptor: Descriptor,
    pub call: EntryCall<C>,
}

/// A declaration-ordered collection of operations.
pub struct Registry<C> {
    pub(crate) entries: Vec<Entry<C>>,
}

fn unary_dispatch<'a, C, P, O, F>(
    f: Arc<F>,
    ctx: &'a C,
    args: Value,
    opts: CallOptions,
) -> BoxFuture<'a, Result<Value>>
where
    P: OpParams + 'static,
    O: serde::Serialize + 'static,
    F: for<'b> Fn(&'b C, P, CallOptions) -> BoxFuture<'b, Result<O>>,
{
    let parsed = match P::from_json(args) {
        Ok(p) => p,
        Err(e) => return Box::pin(async move { Err(e) }),
    };
    let fut = f(ctx, parsed, opts);
    Box::pin(async move { serde_json::to_value(fut.await?).map_err(Into::into) })
}

fn stream_dispatch<'a, C, P, O, F, S>(
    f: Arc<F>,
    ctx: &'a C,
    args: Value,
    opts: CallOptions,
    cursor: Option<Cursor>,
) -> BoxFuture<'a, Result<BoxStream<'a, Result<Value>>>>
where
    P: OpParams + 'static,
    O: serde::Serialize + 'static,
    S: futures_util::Stream<Item = Result<O>> + Send + 'static,
    F: for<'b> Fn(&'b C, P, Option<Cursor>, CallOptions) -> BoxFuture<'b, Result<S>>,
{
    let parsed = match P::from_json(args) {
        Ok(p) => p,
        Err(e) => return Box::pin(async move { Err(e) }),
    };
    let fut = f(ctx, parsed, cursor, opts);
    Box::pin(async move {
        let stream = fut.await?;
        Ok(Box::pin(futures_util::StreamExt::map(stream, |item| {
            item.and_then(|v| serde_json::to_value(v).map_err(Into::into))
        })) as BoxStream<'a, Result<Value>>)
    })
}

impl<C: Sync> Registry<C> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
    pub fn descriptors(&self) -> Vec<&Descriptor> {
        self.entries.iter().map(|e| &e.descriptor).collect()
    }
    pub fn get(&self, name: &str) -> Option<&Descriptor> {
        self.entries
            .iter()
            .find(|e| e.descriptor.name == name)
            .map(|e| &e.descriptor)
    }
    pub fn names(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|e| e.descriptor.name.as_str())
            .collect()
    }
    pub(crate) fn entry(&self, name: &str) -> Result<&Entry<C>> {
        self.entries
            .iter()
            .find(|e| e.descriptor.name == name)
            .ok_or_else(|| {
                coded_error::into_anyhow(crate::CallError::UnknownOperation {
                    name: name.to_owned(),
                })
            })
    }
    #[doc(hidden)]
    pub fn push_unary<P, O, F>(&mut self, descriptor: Descriptor, f: F)
    where
        P: OpParams + 'static,
        O: serde::Serialize + 'static,
        F: for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync + 'static,
    {
        let f = Arc::new(f);
        let call: UnaryCall<C> = Box::new(move |ctx, args, opts| {
            unary_dispatch::<C, P, O, F>(Arc::clone(&f), ctx, args, opts)
        });
        self.entries.push(Entry {
            descriptor,
            call: EntryCall::Unary(call),
        });
    }
    #[doc(hidden)]
    pub fn push_stream<P, O, F, S>(&mut self, descriptor: Descriptor, f: F)
    where
        P: OpParams + 'static,
        O: serde::Serialize + 'static,
        S: futures_util::Stream<Item = Result<O>> + Send + 'static,
        F: for<'a> Fn(&'a C, P, Option<Cursor>, CallOptions) -> BoxFuture<'a, Result<S>>
            + Send
            + Sync
            + 'static,
    {
        let f = Arc::new(f);
        let call: StreamCall<C> = Box::new(move |ctx, args, opts, cursor| {
            stream_dispatch::<C, P, O, F, S>(Arc::clone(&f), ctx, args, opts, cursor)
        });
        self.entries.push(Entry {
            descriptor,
            call: EntryCall::Stream(call),
        });
    }
}
impl<C: Sync> Default for Registry<C> {
    fn default() -> Self {
        Self::new()
    }
}

/// Declares a registry; each entry lists `params`, `output`, then its `op` or
/// `stream_op`, followed optionally by `output_schema` and the remaining
/// descriptor fields.
#[macro_export]
macro_rules! registry {
    (Ctx = $ctx:ty; entries: [ $( {
        $(name: $name:expr,)? noun: $noun:expr, verb: $verb:expr, description: $desc:expr,
        access: $access:ident, audience: $aud:ident, params: $params:ty, output: $output:ty,
        $kind:ident: $op:path, $(output_schema: $out_schema:expr,)?
        $(renders_text: $renders:expr,)? $(exit_code_on_ok: $exit:expr,)?
        $(requires: [$($req:expr),* $(,)?],)? $(hidden: $hidden:expr,)?
        $(since: $since:expr,)? $(deprecated: $deprecated:expr,)? $(aliases: [$($alias:expr),* $(,)?],)?
    } ),* $(,)? ] ) => {{
        let mut registry = $crate::Registry::<$ctx>::new();
        $({
            struct __Op;
            impl $crate::__private::Sealed for __Op {}
            fn __descriptor() -> $crate::Descriptor {
                $crate::Descriptor::new($noun, $verb)
                    $(.name($name))?
                    .description($desc)
                    .access($crate::access!($access))
                    .audience($crate::Audience::$aud)
                    .streaming(matches!(stringify!($kind), "stream_op"))
                    .input_schema(<$params as $crate::OpParams>::schema())
                    .output_schema($crate::__private::output_schema_of::<$output>())
                    $(.output_schema($out_schema))?
                    $(.renders_text($renders))? $(.exit_code_on_ok($exit))?
                    $(.requires(vec![$($req.to_string()),*]))? $(.hidden($hidden))?
                    $(.since($since))? $(.deprecated($deprecated))?
                    $(.aliases(vec![$($alias.to_string()),*]))?
            }
            impl $crate::Operation for __Op {
                type Ctx = $ctx; type Params = $params; type Output = $output;
                fn descriptor() -> $crate::Descriptor { __descriptor() }
            }
            let descriptor = __descriptor();
            $crate::registry!(@push registry, $kind, $params, $output, $op, $ctx, descriptor);
        })*
        registry
    }};
    (@push $registry:ident, op, $params:ty, $output:ty, $op:path, $ctx:ty, $descriptor:ident) => {
        $registry.push_unary::<$params, $output, _>($descriptor, |ctx, params, opts| {
            Box::pin(async move { let _ = opts; $op(ctx, params).await })
        });
    };
    (@push $registry:ident, stream_op, $params:ty, $output:ty, $op:path, $ctx:ty, $descriptor:ident) => {
        $registry.push_stream::<$params, $output, _, _>($descriptor, |ctx, params, cursor, opts| {
            Box::pin(async move { let _ = opts; $op(ctx, params, cursor).await })
        });
    };
}
