//! The registry: typed operations behind erased entries, with build-time checks.

use crate::{
    derive_name, Access, Audience, CallHook, CallOptions, Cursor, Descriptor, ErrorCodeEntry, Exit,
    NoHook, OpKind, OpParams, OutputSchema, ParseCx, Paths, Resolve,
};
use anyhow::Result;
use futures_util::{future::BoxFuture, stream::BoxStream};
use serde::Serialize;
use serde_json::Value;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A function pulling span fields out of typed params.
type FieldsFn<P> = fn(&P) -> Vec<(&'static str, String)>;

/// The declaration of one operation: its path, help text and options.
///
/// `P` is the parameter type. `O` is the op's output: the item type for
/// streams, and `T` for list pairs.
pub struct OpSpec<P, O> {
    /// Space-separated command words, outermost group first and the verb last.
    pub path: &'static str,
    /// A one-line summary.
    pub summary: &'static str,
    /// Longer help text.
    pub long_help: &'static str,
    /// The access the operation needs.
    pub access: Access,
    /// The intended audience.
    pub audience: Audience,
    /// Further audiences the operation also belongs to.
    pub more_audiences: Vec<Audience>,
    /// Whether the operation is hidden from listings.
    pub hidden: bool,
    /// Whether the command runs when its group is named without a verb.
    pub group_default: bool,
    /// Documented exit codes of a successful output.
    pub exits: Vec<(u8, &'static str)>,
    /// Exits taken by an error code on the command line: `(exit, meaning, code)`.
    pub error_exits: Vec<(u8, &'static str, &'static str)>,
    /// Error codes the operation may return.
    pub errors: Vec<coded_error::CodeEntry>,
    /// Which call paths reach the operation.
    pub paths: Paths,
    /// Whether the operation accepts a cursor.
    pub cursor: bool,
    /// The most items a list returns.
    pub cap: usize,
    /// The default page size of a paged list.
    pub page_size: u32,
    /// The page size the follow-all half asks for; `None` is `page_size`.
    pub follow_page: Option<u32>,
    /// Whether the op declares an unpaged form.
    pub unpaged: bool,
    /// Field names (Rust) of the globals struct that the op needs on the command line.
    pub needs_globals: Vec<&'static str>,
    /// Extra `key: value` lines for the text rendering of the parameters.
    pub fields: Option<FieldsFn<P>>,
    /// The exit code for a successful output; it must be one of `exits`.
    pub ok_exit: Option<fn(&O) -> u8>,
    /// The text rendering of an output; its presence is `Descriptor::text`.
    pub render_text: Option<fn(&O) -> String>,
}

impl<P, O> OpSpec<P, O> {
    /// Creates a spec with the given path and summary; every other field has
    /// its default.
    pub fn new(path: &'static str, summary: &'static str) -> Self {
        Self {
            path,
            summary,
            long_help: "",
            access: Access::default(),
            audience: Audience::default(),
            more_audiences: Vec::new(),
            hidden: false,
            group_default: false,
            exits: Vec::new(),
            error_exits: Vec::new(),
            errors: Vec::new(),
            paths: Paths::ALL,
            cursor: false,
            cap: 10_000,
            page_size: 100,
            follow_page: None,
            unpaged: false,
            needs_globals: Vec::new(),
            fields: None,
            ok_exit: None,
            render_text: None,
        }
    }
}

impl<P, O> OpSpec<P, O> {
    /// The same metadata over other parameter and output types: what a list
    /// pair's two ops share. The output hooks (`ok_exit`, `render_text`) are
    /// typed by `O` and do not carry over (a list pair sets `render_text` on
    /// each half itself), and a list pair has no ok exits, so `exits` is
    /// cleared; `error_exits` are copied to both halves.
    pub(crate) fn respec<P2, O2>(&self, fields: Option<FieldsFn<P2>>) -> OpSpec<P2, O2> {
        OpSpec {
            path: self.path,
            summary: self.summary,
            long_help: self.long_help,
            access: self.access,
            audience: self.audience,
            more_audiences: self.more_audiences.clone(),
            hidden: self.hidden,
            group_default: self.group_default,
            exits: Vec::new(),
            error_exits: self.error_exits.clone(),
            errors: self.errors.clone(),
            paths: self.paths,
            cursor: self.cursor,
            cap: self.cap,
            page_size: self.page_size,
            follow_page: self.follow_page,
            unpaged: self.unpaged,
            needs_globals: self.needs_globals.clone(),
            fields,
            ok_exit: None,
            render_text: None,
        }
    }
}

impl<P, O> Default for OpSpec<P, O> {
    fn default() -> Self {
        Self::new("", "")
    }
}

/// A unary op's typed function. It also receives the registry it runs in;
/// user ops ignore it, and only the `manual` op reads it.
pub(crate) type TypedUnary<C, P, O> = Arc<
    dyn for<'a> Fn(&'a Registry<C>, &'a C, P, CallOptions) -> BoxFuture<'a, Result<O>>
        + Send
        + Sync,
>;

/// Wraps a user's unary function, which does not see the registry.
pub(crate) fn ignoring_registry<C, P, O>(
    f: impl for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync + 'static,
) -> TypedUnary<C, P, O> {
    Arc::new(move |_registry, ctx, params, opts| f(ctx, params, opts))
}

pub(crate) type TypedStream<C, P, O> = Arc<
    dyn for<'a> Fn(
            &'a C,
            P,
            Option<Cursor>,
            CallOptions,
        ) -> BoxFuture<'a, Result<BoxStream<'a, Result<O>>>>
        + Send
        + Sync,
>;
pub(crate) type ResolveRefs<C> =
    for<'a> fn(&'a Registry<C>, &'a C, &'a mut (dyn Any + Send)) -> BoxFuture<'a, Result<()>>;
pub(crate) type ErasedFields =
    Box<dyn Fn(&(dyn Any + Send)) -> Vec<(&'static str, String)> + Send + Sync>;

/// What a unary run produces: the output as JSON, its raw ok exit code
/// (checked against the declared exits by the pipeline) and its text.
pub(crate) struct Ran {
    pub value: Value,
    pub exit: u8,
    pub text: Option<String>,
}

/// Runs a unary op; the flag asks for the text rendering.
type UnaryRun<C> = Box<
    dyn for<'a> Fn(
            &'a Registry<C>,
            &'a C,
            Box<dyn Any + Send>,
            CallOptions,
            bool,
        ) -> BoxFuture<'a, Result<Ran>>
        + Send
        + Sync,
>;
type StreamRun<C> = Box<
    dyn for<'a> Fn(
            &'a C,
            Box<dyn Any + Send>,
            CallOptions,
            Option<Cursor>,
        ) -> BoxFuture<'a, Result<BoxStream<'a, Result<Value>>>>
        + Send
        + Sync,
>;

pub(crate) enum Run<C> {
    Unary(UnaryRun<C>),
    Stream(StreamRun<C>),
}

pub(crate) struct Entry<C> {
    pub descriptor: Descriptor,
    /// The typed function (an `Arc<dyn Fn>` over `P` and `O`), for typed calls.
    pub typed: Box<dyn Any + Send + Sync>,
    /// The spec's `fields` function over erased parameters.
    pub fields: Option<ErasedFields>,
    pub decode_json: fn(Value, &mut ParseCx) -> Result<Box<dyn Any + Send>>,
    pub augment: fn(clap::Command) -> clap::Command,
    pub decode_matches: fn(&clap::ArgMatches, &mut ParseCx) -> Result<Box<dyn Any + Send>>,
    /// Resolves the refs of erased parameters in place.
    pub resolve_refs: ResolveRefs<C>,
    /// The default page size of a list op (the spec's `page_size`).
    pub page_size: u32,
    /// The long flags of `#[op(secret)]` fields, for secret-safe CLI errors.
    pub secret_flags: Vec<String>,
    /// The clap argument ids of those fields, to read their raw values.
    pub secret_ids: Vec<String>,
    /// The parameter fields, for the testing kit's argv and subject checks.
    #[cfg(feature = "testing")]
    pub param_fields: Vec<crate::FieldMeta>,
    /// The secrets of erased parameters, for redaction.
    pub secrets: fn(&(dyn Any + Send)) -> Vec<Vec<u8>>,
    pub run: Run<C>,
}

#[derive(Clone)]
pub(crate) struct Group {
    pub path: Vec<String>,
    pub summary: String,
}

/// Rewrites an error before any surface sees it (see
/// [`Registry::set_error_mapper`]).
pub(crate) type ErrorMapper = fn(anyhow::Error) -> anyhow::Error;

/// A declaration-ordered collection of operations.
pub struct Registry<C> {
    pub(crate) entries: Vec<Entry<C>>,
    by_name: HashMap<String, usize>,
    pub(crate) groups: Vec<Group>,
    pillar: &'static str,
    bin: &'static str,
    version: &'static str,
    about: &'static str,
    /// Kinds the server resolves: the library passes their refs through raw.
    pub(crate) server_kinds: HashSet<&'static str>,
    pub(crate) hook: Arc<dyn CallHook>,
    /// Error codes every op may return (the `errors = [..]` header key).
    pub(crate) common_errors: Vec<ErrorCodeEntry>,
    /// Rewrites the error of resolution or an op, once, in the pipeline.
    pub(crate) error_mapper: Option<ErrorMapper>,
    /// Extra (non-op) commands, in declaration order.
    pub(crate) extras: Vec<crate::cli::Extra>,
    /// The root fallback, when there is one.
    pub(crate) fallback: Option<crate::cli::Fallback>,
    /// Set by `testing::strict_codes`: a call that returns a code outside
    /// the op's declared list panics.
    #[cfg(feature = "testing")]
    pub(crate) strict_codes: bool,
}

pub(crate) fn words(path: &str) -> Vec<String> {
    path.split_whitespace().map(str::to_owned).collect()
}

fn is_prefix(prefix: &[String], of: &[String]) -> bool {
    of.len() >= prefix.len() && of[..prefix.len()] == *prefix
}

fn downcast<P: 'static>(boxed: Box<dyn Any + Send>) -> P {
    *boxed
        .downcast::<P>()
        .expect("the entry's decoder and runner share one parameter type")
}

impl<C: Sync> Registry<C> {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            by_name: HashMap::new(),
            groups: Vec::new(),
            pillar: "",
            bin: "",
            version: "",
            about: "",
            server_kinds: HashSet::new(),
            hook: Arc::new(NoHook),
            common_errors: Vec::new(),
            error_mapper: None,
            extras: Vec::new(),
            fallback: None,
            #[cfg(feature = "testing")]
            strict_codes: false,
        }
    }

    /// Sets the hook that wraps every call. A registry starts with [`NoHook`];
    /// `merge` keeps the receiver's hook.
    pub fn set_call_hook(&mut self, hook: Arc<dyn CallHook>) {
        self.hook = hook;
    }

    /// Declares error codes every op of the registry may return: strict codes
    /// and the testing kit accept them on any op, and the manual prints them
    /// once. A code given twice is kept once.
    ///
    /// # Panics
    ///
    /// Panics when a code is already declared with another class.
    pub fn add_common_errors(&mut self, codes: Vec<coded_error::CodeEntry>) {
        for entry in &codes {
            self.add_common(ErrorCodeEntry::from(entry));
        }
    }

    /// The common error codes, in declaration order.
    pub fn common_errors(&self) -> &[ErrorCodeEntry] {
        &self.common_errors
    }

    fn add_common(&mut self, entry: ErrorCodeEntry) {
        match self.common_errors.iter().find(|e| e.code == entry.code) {
            Some(have) if have.class == entry.class => {}
            Some(have) => panic!(
                "common error code {:?} has class {:?} in one registry and {:?} in the other",
                entry.code, have.class, entry.class
            ),
            None => self.common_errors.push(entry),
        }
    }

    /// Sets the error mapper. It runs once per error of resolution or of an
    /// op (and per stream item error), before the hook's `on_error`, strict
    /// codes, the CLI exit and every surface's output. It never sees decode,
    /// validate or other `op_registry.usage.*` errors. `merge` keeps the
    /// receiver's mapper.
    pub fn set_error_mapper(&mut self, mapper: fn(anyhow::Error) -> anyhow::Error) {
        self.error_mapper = Some(mapper);
    }

    /// Names the pillar this registry belongs to.
    pub fn set_pillar(&mut self, pillar: &'static str) {
        self.pillar = pillar;
    }

    /// Returns the pillar name set by [`Registry::set_pillar`].
    pub fn pillar(&self) -> &'static str {
        self.pillar
    }

    /// Names the binary: its command name, version and one-line description.
    pub fn set_bin(&mut self, bin: &'static str, version: &'static str, about: &'static str) {
        self.bin = bin;
        self.version = version;
        self.about = about;
    }

    /// The binary's command name set by [`Registry::set_bin`].
    pub fn bin(&self) -> &'static str {
        self.bin
    }

    /// The binary's version set by [`Registry::set_bin`].
    pub fn version(&self) -> &'static str {
        self.version
    }

    /// The binary's description set by [`Registry::set_bin`].
    pub fn about(&self) -> &'static str {
        self.about
    }

    /// Declares that the server resolves refs of `kind`: the library skips
    /// the context's resolver for it and hands the op `Resolved::Raw`.
    pub fn resolve_server(&mut self, kind: &'static str) {
        self.server_kinds.insert(kind);
    }

    /// Declares a command group. Declare a group before the ops under it.
    ///
    /// # Panics
    ///
    /// Panics when an op already has the group's path and is not its
    /// `group_default`.
    pub fn group(&mut self, path: &str, summary: &str) {
        let path = words(path);
        if let Some(existing) = self.groups.iter().find(|g| g.path == path) {
            if existing.summary == summary {
                return;
            }
            panic!(
                "group {:?} is declared twice with different summaries",
                path.join(" ")
            );
        }
        if let Some(op) = self
            .entries
            .iter()
            .map(|e| &e.descriptor)
            .find(|d| d.path == path && !d.group_default)
        {
            panic!(
                "{:?} is both an operation ({:?}) and a group",
                path.join(" "),
                op.name
            );
        }
        self.groups.push(Group {
            path,
            summary: summary.to_owned(),
        });
    }

    /// The descriptors in declaration order.
    pub fn descriptors(&self) -> Vec<&Descriptor> {
        self.entries.iter().map(|e| &e.descriptor).collect()
    }

    /// The descriptor of the named operation.
    pub fn get(&self, name: &str) -> Option<&Descriptor> {
        self.by_name.get(name).map(|&i| &self.entries[i].descriptor)
    }

    /// The operation names in declaration order.
    pub fn names(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|e| e.descriptor.name.as_str())
            .collect()
    }

    /// Appends every group and operation of `other`, applying the build-time
    /// checks to each.
    ///
    /// # Panics
    ///
    /// Panics on any collision the checks reject.
    pub fn merge(&mut self, other: Registry<C>) {
        #[cfg(feature = "testing")]
        {
            self.strict_codes |= other.strict_codes;
        }
        for group in other.groups {
            self.group(&group.path.join(" "), &group.summary);
        }
        self.server_kinds.extend(other.server_kinds);
        for entry in other.common_errors {
            self.add_common(entry);
        }
        for entry in other.entries {
            self.insert_entry(entry);
        }
        for extra in other.extras {
            let name = extra.command.get_name();
            assert!(
                !self.extras.iter().any(|e| e.command.get_name() == name),
                "extra `{name}` is declared twice"
            );
            self.extras.push(extra);
        }
        if let Some(fallback) = other.fallback {
            assert!(
                self.fallback.is_none(),
                "both registries have a fallback (`{}`)",
                fallback.name()
            );
            self.fallback = Some(fallback);
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

    /// The group an op belongs to: its own path when that is a declared group
    /// (a `group_default` sharing the group word), else its noun path.
    fn group_of(&self, descriptor: &Descriptor) -> Vec<String> {
        if self.groups.iter().any(|g| g.path == descriptor.path) {
            descriptor.path.clone()
        } else {
            descriptor.noun_path().to_vec()
        }
    }

    fn insert_entry(&mut self, entry: Entry<C>) {
        let new = &entry.descriptor;
        if let Some(&i) = self.by_name.get(&new.name) {
            panic!(
                "duplicate operation name {:?} (paths {:?} and {:?})",
                new.name,
                self.entries[i].descriptor.path.join(" "),
                new.path.join(" ")
            );
        }
        // A word that is both an operation and a group.
        if !new.group_default && self.groups.iter().any(|g| g.path == new.path) {
            let child = self
                .entries
                .iter()
                .map(|e| &e.descriptor)
                .find(|d| d.path.len() > new.path.len() && is_prefix(&new.path, &d.path));
            panic!(
                "{:?} is both an operation ({:?}) and a group{}",
                new.path.join(" "),
                new.name,
                child
                    .map(|d| format!(" (ops under it: {:?})", d.name))
                    .unwrap_or_default()
            );
        }
        for existing in self.entries.iter().map(|e| &e.descriptor) {
            if !existing.group_default
                && new.path.len() > existing.path.len()
                && is_prefix(&existing.path, &new.path)
            {
                panic!(
                    "{:?} is both an operation ({:?}) and a group (ops under it: {:?})",
                    existing.path.join(" "),
                    existing.name,
                    new.name
                );
            }
        }
        // Every noun path needs a group with a summary.
        for end in 1..new.path.len() {
            let noun = &new.path[..end];
            if !self.groups.iter().any(|g| g.path == noun) {
                panic!(
                    "group {:?} has no summary (op {:?}); declare it with `group` first",
                    noun.join(" "),
                    new.name
                );
            }
        }
        // At most one group_default per group.
        if new.group_default {
            let group = self.group_of(new);
            if let Some(other) = self
                .entries
                .iter()
                .map(|e| &e.descriptor)
                .find(|d| d.group_default && self.group_of(d) == group)
            {
                panic!(
                    "group {:?} has two group_default ops: {:?} and {:?}",
                    group.join(" "),
                    other.name,
                    new.name
                );
            }
        }
        let index = self.entries.len();
        self.by_name.insert(new.name.clone(), index);
        self.entries.push(entry);
    }
}

impl<C: Resolve + 'static> Registry<C> {
    pub(crate) fn describe<P, O>(spec: &OpSpec<P, O>, kind: OpKind) -> Descriptor
    where
        P: OpParams,
        O: OutputSchema,
    {
        let path = words(spec.path);
        assert!(!path.is_empty(), "an operation needs a path");
        Descriptor {
            name: derive_name(&path),
            path,
            summary: spec.summary.to_owned(),
            long_help: spec.long_help.to_owned(),
            access: spec.access,
            audience: spec.audience,
            audiences: std::iter::once(spec.audience)
                .chain(spec.more_audiences.iter().copied())
                .fold(Vec::new(), |mut all, a| {
                    if !all.contains(&a) {
                        all.push(a);
                    }
                    all
                }),
            kind,
            hidden: spec.hidden,
            group_default: spec.group_default,
            text: spec.render_text.is_some(),
            exits: spec
                .exits
                .iter()
                .map(|(code, meaning)| Exit {
                    code: *code,
                    meaning: (*meaning).to_owned(),
                    on: None,
                })
                .chain(spec.error_exits.iter().map(|(code, meaning, on)| Exit {
                    code: *code,
                    meaning: (*meaning).to_owned(),
                    on: Some((*on).to_owned()),
                }))
                .collect(),
            errors: spec.errors.iter().map(ErrorCodeEntry::from).collect(),
            paths: spec.paths,
            cursor: spec.cursor,
            input_schema: P::input_schema(),
            output_schema: <O as OutputSchema>::output_schema().map(|schema| {
                let mut schema = if kind == OpKind::Stream {
                    schema
                } else {
                    crate::shape::wrap_array_schema(schema)
                };
                crate::shape::mark_object_root(&mut schema);
                schema
            }),
            list_pair: None,
            unpaged: spec.unpaged,
            needs_globals: spec
                .needs_globals
                .iter()
                .map(|name| name.replace('_', "-"))
                .collect(),
        }
    }

    fn push_unary_kind<P, O, F>(&mut self, spec: OpSpec<P, O>, f: F, kind: OpKind)
    where
        P: OpParams + Send + 'static,
        O: Serialize + OutputSchema + Send + 'static,
        F: for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync + 'static,
    {
        let entry = Self::unary_entry(&spec, kind, ignoring_registry(f));
        self.insert_entry(entry);
    }

    /// The erased entry of a unary-shaped operation over a typed function.
    pub(crate) fn unary_entry<P, O>(
        spec: &OpSpec<P, O>,
        kind: OpKind,
        f: TypedUnary<C, P, O>,
    ) -> Entry<C>
    where
        P: OpParams + Send + 'static,
        O: Serialize + OutputSchema + Send + 'static,
    {
        let descriptor = Self::describe(spec, kind);
        check_param_clashes::<P>(&descriptor.name);
        let runner = f.clone();
        let (ok_exit, render_text) = (spec.ok_exit, spec.render_text);
        let run: UnaryRun<C> = Box::new(move |registry, ctx, params, opts, want_text| {
            let fut = runner(registry, ctx, downcast::<P>(params), opts);
            Box::pin(async move {
                let out = fut.await?;
                Ok(Ran {
                    exit: ok_exit.map_or(0, |f| f(&out)),
                    text: render_text.filter(|_| want_text).map(|f| f(&out)),
                    value: crate::shape::wrap_items(serde_json::to_value(out)?),
                })
            })
        });
        Entry {
            descriptor,
            typed: Box::new(f),
            fields: erase_fields(spec.fields),
            decode_json: |value, cx| Ok(Box::new(P::from_json(value, cx)?)),
            augment: P::add_args,
            decode_matches: |matches, cx| Ok(Box::new(P::from_matches(matches, cx)?)),
            page_size: spec.page_size,
            secret_flags: secret_flags::<P>(),
            secret_ids: secret_ids::<P>(),
            #[cfg(feature = "testing")]
            param_fields: P::fields(),
            resolve_refs: |registry, ctx, params| {
                let params = params
                    .downcast_mut::<P>()
                    .expect("the entry's decoder and resolver share one parameter type");
                Box::pin(registry.resolve_refs(ctx, params))
            },
            secrets: |params| {
                params
                    .downcast_ref::<P>()
                    .expect("the entry's decoder and secrets share one parameter type")
                    .secrets()
            },
            run: Run::Unary(run),
        }
    }

    /// Inserts an entry built by [`crate::list`].
    pub(crate) fn push_entry(&mut self, entry: Entry<C>) {
        self.insert_entry(entry);
    }

    /// Adds a unary operation.
    ///
    /// # Panics
    ///
    /// Panics on a duplicate name, a word that is both an op and a group, a
    /// noun path without a group summary, or two `group_default` ops in one group.
    pub fn push_op<P, O, F>(&mut self, spec: OpSpec<P, O>, f: F)
    where
        P: OpParams + Send + 'static,
        O: Serialize + OutputSchema + Send + 'static,
        F: for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync + 'static,
    {
        self.push_unary_kind(spec, f, OpKind::Unary);
    }

    /// Adds an operation that runs in the calling process (kind `Local`).
    ///
    /// # Panics
    ///
    /// Panics on the same build-time checks as [`Registry::push_op`].
    pub fn push_local<P, O, F>(&mut self, spec: OpSpec<P, O>, f: F)
    where
        P: OpParams + Send + 'static,
        O: Serialize + OutputSchema + Send + 'static,
        F: for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync + 'static,
    {
        self.push_unary_kind(spec, f, OpKind::Local);
    }

    /// Adds a streaming operation whose stream may borrow the registry context.
    ///
    /// # Panics
    ///
    /// Panics on the same build-time checks as [`Registry::push_op`].
    pub fn push_stream<P, O, F>(&mut self, spec: OpSpec<P, O>, f: F)
    where
        P: OpParams + Send + 'static,
        O: Serialize + OutputSchema + Send + 'static,
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
        let descriptor = Self::describe(&spec, OpKind::Stream);
        check_param_clashes::<P>(&descriptor.name);
        let f: TypedStream<C, P, O> = Arc::new(f);
        let runner = f.clone();
        let run: StreamRun<C> = Box::new(move |ctx, params, opts, cursor| {
            let fut = runner(ctx, downcast::<P>(params), cursor, opts);
            Box::pin(async move {
                let stream = fut.await?;
                Ok(futures_util::StreamExt::boxed(
                    futures_util::StreamExt::map(stream, |item| {
                        item.and_then(|v| serde_json::to_value(v).map_err(Into::into))
                    }),
                ))
            })
        });
        self.insert_entry(Entry {
            descriptor,
            typed: Box::new(f),
            fields: erase_fields(spec.fields),
            decode_json: |value, cx| Ok(Box::new(P::from_json(value, cx)?)),
            augment: P::add_args,
            decode_matches: |matches, cx| Ok(Box::new(P::from_matches(matches, cx)?)),
            page_size: spec.page_size,
            secret_flags: secret_flags::<P>(),
            secret_ids: secret_ids::<P>(),
            #[cfg(feature = "testing")]
            param_fields: P::fields(),
            resolve_refs: |registry, ctx, params| {
                let params = params
                    .downcast_mut::<P>()
                    .expect("the entry's decoder and resolver share one parameter type");
                Box::pin(registry.resolve_refs(ctx, params))
            },
            secrets: |params| {
                params
                    .downcast_ref::<P>()
                    .expect("the entry's decoder and secrets share one parameter type")
                    .secrets()
            },
            run: Run::Stream(run),
        });
    }
}

pub(crate) fn erase_fields<P: 'static>(fields: Option<FieldsFn<P>>) -> Option<ErasedFields> {
    fields.map(|f| {
        Box::new(move |params: &(dyn Any + Send)| {
            f(params
                .downcast_ref::<P>()
                .expect("the entry's fields function and parameters share one type"))
        }) as ErasedFields
    })
}

/// The long flags of a parameter type's `#[op(secret)]` fields.
fn secret_ids<P: OpParams>() -> Vec<String> {
    P::fields()
        .into_iter()
        .filter(|f| f.secret)
        .map(|f| f.rust_name.to_owned())
        .collect()
}

fn secret_flags<P: OpParams>() -> Vec<String> {
    P::fields()
        .into_iter()
        .filter(|f| f.secret)
        .map(|f| f.flag)
        .collect()
}

/// Flattened groups are opaque to the derive, so a name used by two fields
/// of one op is found here.
pub(crate) fn check_param_clashes<P: OpParams>(op: &str) {
    let fields = P::fields();
    if let Some((a, b)) = crate::__private::rt::find_clash(&fields) {
        let origin = |f: &crate::FieldMeta| match f.from_group {
            Some(group) => format!("`{}` (from group {group})", f.rust_name),
            None => format!("`{}`", f.rust_name),
        };
        panic!(
            "operation {op:?}: field {} clashes with field {} (flag --{} / key {})",
            origin(a),
            origin(b),
            a.flag,
            a.key
        );
    }
}

impl<C: Sync> Default for Registry<C> {
    fn default() -> Self {
        Self::new()
    }
}
