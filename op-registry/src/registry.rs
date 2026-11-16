use crate::{CallOptions, Cursor, Descriptor, OpParams};
use anyhow::Result;
use futures_util::{future::BoxFuture, stream::BoxStream};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Command-line adapters for an operation's parameter type.
#[derive(Clone, Copy)]
pub struct CliHooks {
    pub add_args: fn(clap::Command) -> clap::Command,
    pub parse: fn(&clap::ArgMatches) -> anyhow::Result<Value>,
}

impl CliHooks {
    /// Creates command-line adapters for `P`.
    pub fn for_params<P>() -> Self
    where
        P: OpParams + serde::Serialize + 'static,
    {
        fn parse<P>(matches: &clap::ArgMatches) -> anyhow::Result<Value>
        where
            P: OpParams + serde::Serialize + 'static,
        {
            let params = P::from_matches(matches)?;
            serde_json::to_value(params).map_err(anyhow::Error::from)
        }

        Self {
            add_args: P::clap_args,
            parse: parse::<P>,
        }
    }
}

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
    pub hooks: CliHooks,
}

/// A declaration-ordered collection of operations.
pub struct Registry<C> {
    pub(crate) entries: Vec<Entry<C>>,
    by_name: HashMap<String, usize>,
    by_cli: HashMap<String, usize>,
}

fn map_stream_values<'a, O>(stream: BoxStream<'a, Result<O>>) -> BoxStream<'a, Result<Value>>
where
    O: serde::Serialize + 'static,
{
    Box::pin(futures_util::StreamExt::map(stream, |item| {
        item.and_then(|v| serde_json::to_value(v).map_err(Into::into))
    }))
}

/// The unary dispatch: parses the JSON arguments as `P`, calls `f`, and
/// serialises the output.
fn unary_dispatch<C, P, O, F>(f: F) -> UnaryCall<C>
where
    C: Sync,
    P: OpParams + 'static,
    O: serde::Serialize + 'static,
    F: for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync + 'static,
{
    let f = Arc::new(f);
    Box::new(move |ctx, args, opts| {
        let parsed = match P::from_json(args) {
            Ok(p) => p,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let fut = f(ctx, parsed, opts);
        Box::pin(async move { serde_json::to_value(fut.await?).map_err(Into::into) })
    })
}

/// The single stream dispatch: parses the JSON arguments as `P`, calls `f`
/// with the cursor, and serialises each stream item.
fn stream_dispatch<C, P, O, F>(f: F) -> StreamCall<C>
where
    C: Sync,
    P: OpParams + 'static,
    O: serde::Serialize + 'static,
    F: for<'a> Fn(
            &'a C,
            P,
            Option<Cursor>,
            CallOptions,
        ) -> BoxFuture<'a, Result<BoxStream<'a, Result<O>>>>
        + Send
        + Sync
        + 'static,
{
    let f = Arc::new(f);
    Box::new(move |ctx, args, opts, cursor| {
        let parsed = match P::from_json(args) {
            Ok(p) => p,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let fut = f(ctx, parsed, cursor, opts);
        Box::pin(async move { Ok(map_stream_values(fut.await?)) })
    })
}

/// The words a descriptor occupies in the CLI tree: the noun words and verb for
/// a derived name, or the explicit name as one word.
fn cli_words(descriptor: &Descriptor) -> Vec<String> {
    if descriptor.is_path() {
        let mut words: Vec<String> = descriptor
            .noun
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        words.push(descriptor.verb.clone());
        words
    } else {
        vec![descriptor.name.clone()]
    }
}

impl<C: Sync> Registry<C> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            by_name: HashMap::new(),
            by_cli: HashMap::new(),
        }
    }
    pub fn descriptors(&self) -> Vec<&Descriptor> {
        self.entries.iter().map(|e| &e.descriptor).collect()
    }
    pub fn get(&self, name: &str) -> Option<&Descriptor> {
        self.by_name.get(name).map(|&i| &self.entries[i].descriptor)
    }
    /// Finds the descriptor whose CLI name (`noun verb` or explicit name) matches.
    pub fn find_cli(&self, cli_name: &str) -> Option<&Descriptor> {
        self.by_cli
            .get(cli_name)
            .map(|&i| &self.entries[i].descriptor)
    }
    pub fn names(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|e| e.descriptor.name.as_str())
            .collect()
    }
    pub fn cli_hooks(&self, name: &str) -> Option<&CliHooks> {
        self.by_name.get(name).map(|&i| &self.entries[i].hooks)
    }
    pub fn merge(&mut self, other: Registry<C>) {
        for entry in other.entries {
            self.insert_entry(entry);
        }
    }
    pub(crate) fn entry(&self, name: &str) -> Result<&Entry<C>> {
        self.by_name
            .get(name)
            .map(|&i| &self.entries[i])
            .ok_or_else(|| {
                coded_error::into_anyhow(crate::CallError::UnknownOperation {
                    name: name.to_owned(),
                })
            })
    }

    /// Checks an entry's name and CLI words against every registered entry,
    /// then appends it. Panics on a duplicate name or a CLI collision.
    fn insert_entry(&mut self, entry: Entry<C>) {
        let name = entry.descriptor.name.clone();
        if self.by_name.contains_key(&name) {
            panic!("duplicate operation name {:?}", name);
        }
        self.assert_unique_cli(&entry.descriptor, &name);
        let index = self.entries.len();
        let cli = entry.descriptor.cli_name();
        self.by_name.insert(name, index);
        self.by_cli.insert(cli, index);
        self.entries.push(entry);
    }

    /// Panics when the new descriptor's CLI name or CLI words equal, or are a
    /// strict prefix of or extend, the words of an existing operation.
    fn assert_unique_cli(&self, new: &Descriptor, new_name: &str) {
        let new_words = cli_words(new);
        let new_cli = new.cli_name();
        for existing in &self.entries {
            let existing_words = cli_words(&existing.descriptor);
            if new_cli == existing.descriptor.cli_name() || new_words == existing_words {
                panic!(
                    "{:?} and {:?} both define the command {:?}",
                    existing.descriptor.name, new_name, new_cli
                );
            }
            let shared = new_words.len().min(existing_words.len());
            if new_words[..shared] == existing_words[..shared] {
                panic!(
                    "{:?} collides with {:?}: a word cannot be both a leaf and a group",
                    existing.descriptor.name, new_name
                );
            }
        }
    }

    /// Adds a unary operation to the registry.
    ///
    /// # Panics
    ///
    /// Panics if the descriptor name is already registered, with the
    /// duplicate name quoted in the panic message, or if its CLI words collide
    /// with another operation's.
    pub fn push_unary<P, O, F>(&mut self, descriptor: Descriptor, f: F, hooks: CliHooks)
    where
        P: OpParams + 'static,
        O: serde::Serialize + 'static,
        F: for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync + 'static,
    {
        self.insert_entry(Entry {
            descriptor,
            call: EntryCall::Unary(unary_dispatch::<C, P, O, F>(f)),
            hooks,
        });
    }

    /// Adds a streaming operation to the registry.
    ///
    /// # Panics
    ///
    /// Panics if the descriptor name is already registered, with the
    /// duplicate name quoted in the panic message, or if its CLI words collide
    /// with another operation's.
    pub fn push_stream<P, O, F, S>(&mut self, descriptor: Descriptor, f: F, hooks: CliHooks)
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
        self.push_stream_borrowed::<P, O, _>(
            descriptor,
            move |ctx, params, cursor, opts| {
                let fut = f(ctx, params, cursor, opts);
                Box::pin(async move {
                    let stream = fut.await?;
                    Ok(Box::pin(stream) as BoxStream<'_, Result<O>>)
                })
            },
            hooks,
        );
    }

    /// Adds a streaming operation whose stream may borrow the registry context.
    ///
    /// # Panics
    ///
    /// Panics if the descriptor name is already registered, with the
    /// duplicate name quoted in the panic message, or if its CLI words collide
    /// with another operation's.
    pub fn push_stream_borrowed<P, O, F>(&mut self, descriptor: Descriptor, f: F, hooks: CliHooks)
    where
        P: OpParams + 'static,
        O: serde::Serialize + 'static,
        F: for<'a> Fn(
                &'a C,
                P,
                Option<Cursor>,
                CallOptions,
            ) -> BoxFuture<'a, Result<BoxStream<'a, Result<O>>>>
            + Send
            + Sync
            + 'static,
    {
        self.insert_entry(Entry {
            descriptor,
            call: EntryCall::Stream(stream_dispatch::<C, P, O, F>(f)),
            hooks,
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
/// descriptor fields. Parameter types must implement `serde::Serialize`.
/// A duplicate operation name panics.
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
        }, $crate::CliHooks::for_params::<$params>());
    };
    (@push $registry:ident, stream_op, $params:ty, $output:ty, $op:path, $ctx:ty, $descriptor:ident) => {
        $registry.push_stream::<$params, $output, _, _>($descriptor, |ctx, params, cursor, opts| {
            Box::pin(async move { let _ = opts; $op(ctx, params, cursor).await })
        }, $crate::CliHooks::for_params::<$params>());
    };
}
