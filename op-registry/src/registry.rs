//! The registry: typed operations behind erased entries, with build-time checks.

use crate::{
    derive_name, Access, Audience, CallOptions, Cursor, Descriptor, ErrorCodeEntry, Exit, OpKind,
    OpParams, OutputSchema, ParseCx, Paths,
};
use anyhow::Result;
use futures_util::{future::BoxFuture, stream::BoxStream};
use serde::Serialize;
use serde_json::Value;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Whether the operation is hidden from listings.
    pub hidden: bool,
    /// Whether the command runs when its group is named without a verb.
    pub group_default: bool,
    /// Whether the operation has a text rendering.
    pub text: bool,
    /// Documented exit codes.
    pub exits: Vec<(u8, &'static str)>,
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
    /// Whether the op declares an unpaged form.
    pub unpaged: bool,
    /// Extra `key: value` lines for the text rendering of the parameters.
    pub fields: Option<fn(&P) -> Vec<(&'static str, String)>>,
    /// The exit code for a successful output (acted on from Task 12).
    pub ok_exit: Option<fn(&O) -> u8>,
    /// The text rendering of an output (acted on from Task 12).
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
            hidden: false,
            group_default: false,
            text: false,
            exits: Vec::new(),
            errors: Vec::new(),
            paths: Paths::All,
            cursor: false,
            cap: 10_000,
            page_size: 100,
            unpaged: false,
            fields: None,
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

type UnaryRun<C> = Box<
    dyn for<'a> Fn(&'a C, Box<dyn Any + Send>, CallOptions) -> BoxFuture<'a, Result<Value>>
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

/// The spec fields the registry keeps for later tasks, typed by `P` and `O`.
#[allow(dead_code)]
pub(crate) struct Extras<P, O> {
    pub cap: usize,
    pub page_size: u32,
    pub fields: Option<fn(&P) -> Vec<(&'static str, String)>>,
    pub ok_exit: Option<fn(&O) -> u8>,
    pub render_text: Option<fn(&O) -> String>,
}

pub(crate) struct Entry<C> {
    pub descriptor: Descriptor,
    /// The typed function (an `Arc<dyn Fn>` over `P` and `O`), for typed calls.
    #[allow(dead_code)]
    pub typed: Box<dyn Any + Send + Sync>,
    /// The `Extras<P, O>` of the spec.
    #[allow(dead_code)]
    pub extras: Box<dyn Any + Send + Sync>,
    pub decode_json: fn(Value, &mut ParseCx) -> Result<Box<dyn Any + Send>>,
    pub augment: fn(clap::Command) -> clap::Command,
    pub decode_matches: fn(&clap::ArgMatches, &mut ParseCx) -> Result<Box<dyn Any + Send>>,
    pub run: Run<C>,
}

#[derive(Clone)]
struct Group {
    path: Vec<String>,
    summary: String,
}

/// A declaration-ordered collection of operations.
pub struct Registry<C> {
    pub(crate) entries: Vec<Entry<C>>,
    by_name: HashMap<String, usize>,
    groups: Vec<Group>,
    pillar: &'static str,
}

fn words(path: &str) -> Vec<String> {
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
        }
    }

    /// Names the pillar this registry belongs to.
    pub fn set_pillar(&mut self, pillar: &'static str) {
        self.pillar = pillar;
    }

    /// Returns the pillar name set by [`Registry::set_pillar`].
    pub fn pillar(&self) -> &'static str {
        self.pillar
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
        for group in other.groups {
            if !self.groups.iter().any(|g| g.path == group.path) {
                self.group(&group.path.join(" "), &group.summary);
            }
        }
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
        if !new.group_default {
            if self.groups.iter().any(|g| g.path == new.path) {
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

    /// Runs the entry's unary pipeline on already decoded parameters.
    pub(crate) async fn run_unary(
        &self,
        name: &str,
        params: Box<dyn Any + Send>,
        ctx: &C,
        opts: CallOptions,
    ) -> Result<Value> {
        match &self.entry(name)?.run {
            Run::Unary(run) => run(ctx, params, opts).await,
            Run::Stream(_) => Err(coded_error::into_anyhow(crate::CallError::WrongShape {
                name: name.to_owned(),
            })),
        }
    }

    /// Runs the entry's stream pipeline on already decoded parameters.
    pub(crate) async fn run_stream<'a>(
        &self,
        name: &str,
        params: Box<dyn Any + Send>,
        ctx: &'a C,
        opts: CallOptions,
        cursor: Option<Cursor>,
    ) -> Result<BoxStream<'a, Result<Value>>> {
        match &self.entry(name)?.run {
            Run::Stream(run) => run(ctx, params, opts, cursor).await,
            Run::Unary(_) => Err(coded_error::into_anyhow(crate::CallError::WrongShape {
                name: name.to_owned(),
            })),
        }
    }
}

impl<C: Sync + 'static> Registry<C> {
    fn describe<P, O>(spec: &OpSpec<P, O>, kind: OpKind) -> Descriptor
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
            kind,
            hidden: spec.hidden,
            group_default: spec.group_default,
            text: spec.text,
            exits: spec
                .exits
                .iter()
                .map(|(code, meaning)| Exit {
                    code: *code,
                    meaning: (*meaning).to_owned(),
                })
                .collect(),
            errors: spec.errors.iter().map(ErrorCodeEntry::from).collect(),
            paths: spec.paths,
            cursor: spec.cursor,
            input_schema: P::input_schema(),
            output_schema: <O as OutputSchema>::output_schema(),
            list_pair: None,
            unpaged: spec.unpaged,
        }
    }

    fn push_unary_kind<P, O, F>(&mut self, spec: OpSpec<P, O>, f: F, kind: OpKind)
    where
        P: OpParams + Send + 'static,
        O: Serialize + OutputSchema + Send + 'static,
        F: for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync + 'static,
    {
        let descriptor = Self::describe(&spec, kind);
        check_param_clashes::<P>(&descriptor.name);
        let extras = Extras {
            cap: spec.cap,
            page_size: spec.page_size,
            fields: spec.fields,
            ok_exit: spec.ok_exit,
            render_text: spec.render_text,
        };
        let f: Arc<
            dyn for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync,
        > = Arc::new(f);
        let runner = f.clone();
        let run: UnaryRun<C> = Box::new(move |ctx, params, opts| {
            let fut = runner(ctx, downcast::<P>(params), opts);
            Box::pin(async move { serde_json::to_value(fut.await?).map_err(Into::into) })
        });
        self.insert_entry(Entry {
            descriptor,
            typed: Box::new(f),
            extras: Box::new(extras),
            decode_json: |value, cx| Ok(Box::new(P::from_json(value, cx)?)),
            augment: P::add_args,
            decode_matches: |matches, cx| Ok(Box::new(P::from_matches(matches, cx)?)),
            run: Run::Unary(run),
        });
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
        let mut descriptor = Self::describe(&spec, OpKind::Stream);
        descriptor.cursor = true;
        check_param_clashes::<P>(&descriptor.name);
        let extras = Extras {
            cap: spec.cap,
            page_size: spec.page_size,
            fields: spec.fields,
            ok_exit: spec.ok_exit,
            render_text: spec.render_text,
        };
        type Typed<C, P, O> = Arc<
            dyn for<'a> Fn(
                    &'a C,
                    P,
                    Option<Cursor>,
                    CallOptions,
                ) -> BoxFuture<'a, Result<BoxStream<'a, Result<O>>>>
                + Send
                + Sync,
        >;
        let f: Typed<C, P, O> = Arc::new(f);
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
            extras: Box::new(extras),
            decode_json: |value, cx| Ok(Box::new(P::from_json(value, cx)?)),
            augment: P::add_args,
            decode_matches: |matches, cx| Ok(Box::new(P::from_matches(matches, cx)?)),
            run: Run::Stream(run),
        });
    }
}

/// Flattened groups are opaque to the derive, so a name used by two fields
/// of one op is found here.
fn check_param_clashes<P: OpParams>(op: &str) {
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
