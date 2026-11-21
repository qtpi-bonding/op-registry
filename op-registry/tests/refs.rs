//! Refs: the subject flag, scopes and resolution on every call path.

use clap::Command;
use futures_util::future::BoxFuture;
use op_registry::{
    CallOptions, Op, OpParams, OpSpec, ParseCx, Ref, Registry, Resolve, Resolved, Scope, UsageError,
};
use serde::Serialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "fix.build")]
enum BuildError {
    #[error("no build {0}")]
    #[coded(code = "not_found", class = Logical)]
    NotFound(String),
    #[error("{0} is ambiguous")]
    #[coded(code = "ambiguous", class = Logical)]
    Ambiguous(String),
}

#[derive(Default)]
struct Ctx {
    panic_on_resolve: bool,
    seen_scope: Mutex<Vec<Option<String>>>,
    /// The `root` and `gcodeStore` scope keys each build lookup saw.
    seen_paths: Mutex<Vec<(Option<String>, Option<String>)>>,
    /// Every lookup: the kind and the `root` scope key it saw.
    seen_roots: Mutex<Vec<(String, Option<String>)>>,
}

impl Resolve for Ctx {
    fn resolve<'a>(
        &'a self,
        kind: &'a str,
        r: &'a Ref,
        scope: &'a Scope,
    ) -> BoxFuture<'a, anyhow::Result<Resolved>> {
        Box::pin(async move {
            assert!(!self.panic_on_resolve, "the resolver must not be called");
            let raw = r.as_raw();
            self.seen_roots
                .lock()
                .unwrap()
                .push((kind.to_owned(), scope.get("root").map(str::to_owned)));
            match kind {
                "build" => {
                    self.seen_scope
                        .lock()
                        .unwrap()
                        .push(scope.get("blueprint").map(str::to_owned));
                    self.seen_paths.lock().unwrap().push((
                        scope.get("root").map(str::to_owned),
                        scope.get("gcodeStore").map(str::to_owned),
                    ));
                    if raw == "dup" {
                        return Err(coded_error::into_anyhow(BuildError::Ambiguous(raw.into())));
                    }
                    let in_bp = scope.get("blueprint").is_none_or(|b| b == "bp");
                    if (raw == "b1" || raw == "nightly") && in_bp {
                        Ok(Resolved::Id("b1".into()))
                    } else {
                        Err(coded_error::into_anyhow(BuildError::NotFound(raw.into())))
                    }
                }
                "blueprint" if raw == "bp" || raw == "Blue" => Ok(Resolved::Id("bp".into())),
                "collection" if raw == "c1" => Ok(Resolved::Id("c1".into())),
                "place" if raw == "Home" => Ok(Resolved::Id("p1".into())),
                _ => Err(coded_error::into_anyhow(BuildError::NotFound(raw.into()))),
            }
        })
    }
}

struct Plain;
impl Resolve for Plain {}

#[derive(Debug, OpParams)]
struct Get {
    #[op(subject = "build")]
    build: Ref,
}

#[derive(Debug, OpParams)]
struct Scoped {
    #[op(subject = "build", scope = ["blueprint"])]
    id: Ref,
    #[op(kind = "blueprint")]
    blueprint: Ref,
}

#[derive(Debug, OpParams)]
struct ProjectGroup {
    root: PathBuf,
}

#[derive(Debug, OpParams)]
struct StoreGroup {
    gcode_store: Option<PathBuf>,
}

#[derive(Debug, OpParams)]
struct Dotted {
    #[op(subject = "build", scope = ["project.root", "store.gcode_store"])]
    id: Ref,
    #[op(flatten)]
    project: ProjectGroup,
    #[op(flatten)]
    store: StoreGroup,
}

#[derive(Debug, OpParams)]
struct SubjectGroup {
    #[op(subject = "build")]
    id: Ref,
}

#[derive(Debug, OpParams)]
struct Grouped {
    #[op(flatten, scope = ["blueprint", "project.root"])]
    subject: SubjectGroup,
    #[op(kind = "blueprint")]
    blueprint: Ref,
    #[op(flatten)]
    project: ProjectGroup,
}

#[derive(Debug, OpParams)]
struct IdOnly {
    #[op(subject = "agent", id_only)]
    id: Ref,
}

#[derive(Debug, OpParams)]
struct Waits {
    #[op(subject = "build")]
    id: Ref,
    #[op(kind = "build")]
    waits_for: Option<Ref>,
    #[op(kind = "collection")]
    more: Vec<Ref>,
}

#[derive(Debug, OpParams)]
struct Group {
    #[op(kind = "collection")]
    collection: Ref,
}

#[derive(Debug, OpParams)]
struct Outer {
    #[op(subject = "build")]
    id: Ref,
    #[op(flatten)]
    group: Group,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Seen {
    id: String,
    raw: bool,
}

fn seen(r: &Ref) -> Seen {
    Seen {
        id: r.id().to_owned(),
        raw: matches!(r.resolved(), Resolved::Raw(_)),
    }
}

macro_rules! op {
    ($ty:ident, $params:ident, $name:literal, $field:ident) => {
        struct $ty;
        impl Op for $ty {
            type Ctx = Ctx;
            type Params = $params;
            type Output = Seen;
            const NAME: &'static str = $name;
        }
    };
}
op!(GetOp, Get, "get", build);
op!(WaitsOp, Waits, "waits", id);
op!(DottedOp, Dotted, "dotted", id);
op!(GroupedOp, Grouped, "grouped", id);

fn registry(server: Option<&'static str>) -> Registry<Ctx> {
    let mut r: Registry<Ctx> = Registry::new();
    if let Some(kind) = server {
        r.resolve_server(kind);
    }
    r.push_op(
        OpSpec::<Get, Seen>::new("get", "Gets."),
        |_c: &Ctx, p: Get, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.build)) })
        },
    );
    r.push_op(
        OpSpec::<Scoped, Seen>::new("scoped", "Scoped."),
        |_c: &Ctx, p: Scoped, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.id)) })
        },
    );
    r.push_op(
        OpSpec::<Dotted, Seen>::new("dotted", "Dotted."),
        |_c: &Ctx, p: Dotted, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.id)) })
        },
    );
    r.push_op(
        OpSpec::<Grouped, Seen>::new("grouped", "Grouped."),
        |_c: &Ctx, p: Grouped, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.subject.id)) })
        },
    );
    r.push_op(
        OpSpec::<IdOnly, Seen>::new("idonly", "Id only."),
        |_c: &Ctx, p: IdOnly, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.id)) })
        },
    );
    r.push_op(
        OpSpec::<Waits, Seen>::new("waits", "Waits."),
        |_c: &Ctx, p: Waits, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.id)) })
        },
    );
    r.push_op(
        OpSpec::<Outer, Seen>::new("outer", "Outer."),
        |_c: &Ctx, p: Outer, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.group.collection)) })
        },
    );
    r
}

fn code(err: &anyhow::Error) -> String {
    coded_error::facts_anyhow(err).code.to_string()
}

#[test]
fn subject_is_id_flag_and_key() {
    let fields = Get::fields();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].flag, "id");
    assert_eq!(fields[0].key, "id");
    assert_eq!(fields[0].rust_name, "build");
    assert_eq!(fields[0].kind, op_registry::FieldKind::Ref);
    assert!(fields[0].required);
    let cmd = Get::add_args(Command::new("t"));
    let m = cmd.try_get_matches_from(["t", "--id", "x"]).unwrap();
    let p = Get::from_matches(&m, &mut ParseCx::default()).unwrap();
    assert_eq!(p.build.as_raw(), "x");
    let p = Get::from_json(json!({"id": "y"}), &mut ParseCx::default()).unwrap();
    assert_eq!(p.build.as_raw(), "y");
}

#[tokio::test]
async fn name_resolves_to_id_before_op() {
    let r = registry(None);
    let out = r
        .call(
            "get",
            json!({"id": "nightly"}),
            &Ctx::default(),
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "b1", "raw": false}));
}

#[tokio::test]
async fn unknown_ref_is_apps_not_found_on_every_path() {
    let r = registry(None);
    let ctx = Ctx::default();
    // JSON
    let err = r
        .call("get", json!({"id": "nope"}), &ctx, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.not_found");
    // CLI
    let m = Get::add_args(Command::new("t"))
        .try_get_matches_from(["t", "--id", "nope"])
        .unwrap();
    let p = Get::from_matches(&m, &mut ParseCx::default()).unwrap();
    let err = r
        .call_typed::<GetOp>(&ctx, p, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.not_found");
    // typed
    let err = r
        .call_typed::<GetOp>(
            &ctx,
            Get {
                build: Ref::new("nope"),
            },
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.not_found");
    // ambiguity propagates unchanged
    let err = r
        .call("get", json!({"id": "dup"}), &ctx, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.ambiguous");
}

#[tokio::test]
async fn resolve_default_is_error() {
    let mut r: Registry<Plain> = Registry::new();
    r.push_op(
        OpSpec::<Get, Seen>::new("get", "Gets."),
        |_c: &Plain, p: Get, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.build)) })
        },
    );
    let err = r
        .call("get", json!({"id": "x"}), &Plain, CallOptions::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no resolver for kind"), "{err}");
}

#[tokio::test]
async fn scope_passes_resolved_sibling() {
    let r = registry(None);
    let ctx = Ctx::default();
    // "Blue" resolves to "bp", and the build is looked up inside "bp".
    let out = r
        .call(
            "scoped",
            json!({"id": "nightly", "blueprint": "Blue"}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out["id"], "b1");
    assert_eq!(
        ctx.seen_scope.lock().unwrap().as_slice(),
        [Some("bp".to_owned())]
    );
}

#[tokio::test]
async fn server_kind_skips_resolver() {
    let r = registry(Some("build"));
    let ctx = Ctx {
        panic_on_resolve: true,
        ..Ctx::default()
    };
    let out = r
        .call(
            "get",
            json!({"id": "nightly"}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "nightly", "raw": true}));
}

#[tokio::test]
async fn id_only_skips_resolver() {
    let r = registry(None);
    let ctx = Ctx {
        panic_on_resolve: true,
        ..Ctx::default()
    };
    let out = r
        .call("idonly", json!({"id": "a-9"}), &ctx, CallOptions::default())
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "a-9", "raw": false}));
}

#[tokio::test]
async fn kind_ref_flag_resolves() {
    let r = registry(None);
    let ctx = Ctx::default();
    let m = Waits::add_args(Command::new("t"))
        .try_get_matches_from(["t", "--id", "b1", "--waits-for", "x", "--more", "c1"])
        .unwrap();
    let p = Waits::from_matches(&m, &mut ParseCx::default()).unwrap();
    let err = r
        .call_typed::<WaitsOp>(&ctx, p, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.not_found");
    // Known values resolve, including the Vec.
    let out = r
        .call(
            "waits",
            json!({"id": "b1", "waitsFor": "nightly", "more": ["c1"]}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out["id"], "b1");
    let err = r
        .call(
            "waits",
            json!({"id": "b1", "more": ["zzz"]}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.not_found");
}

#[tokio::test]
async fn flattened_group_refs_resolve() {
    let r = registry(None);
    let ctx = Ctx::default();
    let out = r
        .call(
            "outer",
            json!({"id": "b1", "collection": "c1"}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out["id"], "c1");
    let err = r
        .call(
            "outer",
            json!({"id": "b1", "collection": "no"}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.not_found");
}

#[tokio::test]
async fn non_subject_refs_resolve_before_the_subject() {
    let r = registry(None);
    let ctx = Ctx::default();
    // The blueprint "nope" fails first, so the build resolver is never asked.
    let err = r
        .call(
            "scoped",
            json!({"id": "nightly", "blueprint": "nope"}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.not_found");
    assert!(ctx.seen_scope.lock().unwrap().is_empty());
}

#[test]
fn ref_schema_is_string() {
    let schema = Scoped::input_schema();
    assert_eq!(schema["properties"]["id"]["type"], "string");
    assert_eq!(schema["properties"]["blueprint"]["type"], "string");
    let waits = Waits::input_schema();
    assert_eq!(waits["properties"]["more"]["items"]["type"], "string");
    assert_eq!(schema["required"], json!(["id", "blueprint"]));
}

/// Records what the hook sees: each `around` and each `on_error` code.
#[derive(Default)]
struct Recorder {
    arounds: Mutex<Vec<String>>,
    errors: Mutex<Vec<String>>,
}

impl op_registry::CallHook for Recorder {
    fn around<'a>(
        &'a self,
        info: &op_registry::CallInfo,
        call: BoxFuture<'a, ()>,
    ) -> BoxFuture<'a, ()> {
        self.arounds.lock().unwrap().push(info.name.clone());
        call
    }
    fn on_error(&self, info: &op_registry::CallInfo, err: &anyhow::Error) {
        self.errors
            .lock()
            .unwrap()
            .push(format!("{} {}", info.name, code(err)));
    }
}

#[tokio::test]
async fn resolution_failure_reaches_the_hook_on_every_path() {
    let hook = std::sync::Arc::new(Recorder::default());
    let mut r = registry(None);
    r.set_call_hook(hook.clone());
    let ctx = Ctx::default();
    // JSON
    let err = r
        .call("get", json!({"id": "nope"}), &ctx, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.not_found");
    // typed
    let err = r
        .call_typed::<GetOp>(
            &ctx,
            Get {
                build: Ref::new("nope"),
            },
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.not_found");
    assert_eq!(hook.arounds.lock().unwrap().as_slice(), ["get", "get"]);
    assert_eq!(
        hook.errors.lock().unwrap().as_slice(),
        ["get fix.build.not_found", "get fix.build.not_found"]
    );
}

fn dotted_typed(root: PathBuf, store: Option<PathBuf>) -> Dotted {
    Dotted {
        id: Ref::new("nightly"),
        project: ProjectGroup { root },
        store: StoreGroup { gcode_store: store },
    }
}

#[tokio::test]
async fn dotted_scope_reaches_the_resolver_on_every_path() {
    let r = registry(None);
    let want = vec![(Some("/srv/x".to_owned()), Some("/srv/g".to_owned()))];
    // JSON (`call` / `call_tool` share this decode).
    let ctx = Ctx::default();
    r.call(
        "dotted",
        json!({"id": "nightly", "root": "/srv/x", "gcodeStore": "/srv/g"}),
        &ctx,
        CallOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(*ctx.seen_paths.lock().unwrap(), want);
    // MCP `call_tool`
    let ctx = Ctx::default();
    op_registry::mcp::call_tool(
        &r,
        &ctx,
        op_registry::Audience::Agent,
        "dotted",
        json!({"id": "nightly", "root": "/srv/x", "gcodeStore": "/srv/g"}),
        CallOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(*ctx.seen_paths.lock().unwrap(), want);
    // CLI
    let ctx = Ctx::default();
    let m = Dotted::add_args(Command::new("t"))
        .try_get_matches_from([
            "t",
            "--id",
            "nightly",
            "--root",
            "/srv/x",
            "--gcode-store",
            "/srv/g",
        ])
        .unwrap();
    let p = Dotted::from_matches(&m, &mut ParseCx::default()).unwrap();
    r.call_typed::<DottedOp>(&ctx, p, CallOptions::default())
        .await
        .unwrap();
    assert_eq!(*ctx.seen_paths.lock().unwrap(), want);
    // typed
    let ctx = Ctx::default();
    let p = dotted_typed("/srv/x".into(), Some("/srv/g".into()));
    r.call_typed::<DottedOp>(&ctx, p, CallOptions::default())
        .await
        .unwrap();
    assert_eq!(*ctx.seen_paths.lock().unwrap(), want);
}

#[tokio::test]
async fn dotted_scope_skips_an_absent_optional() {
    let r = registry(None);
    let ctx = Ctx::default();
    let p = dotted_typed("/srv/x".into(), None);
    r.call_typed::<DottedOp>(&ctx, p, CallOptions::default())
        .await
        .unwrap();
    assert_eq!(
        *ctx.seen_paths.lock().unwrap(),
        [(Some("/srv/x".to_owned()), None)]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn non_utf8_path_in_scope_is_a_usage_error() {
    use std::os::unix::ffi::OsStringExt;
    let r = registry(None);
    let ctx = Ctx::default();
    let bad = PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', 0xff, b'x']));
    let err = r
        .call_typed::<DottedOp>(&ctx, dotted_typed(bad, None), CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "op_registry.usage.invalid");
    assert!(ctx.seen_paths.lock().unwrap().is_empty());
}

#[tokio::test]
async fn flattened_subject_takes_its_groups_scope() {
    let r = registry(None);
    let ctx = Ctx::default();
    let out = r
        .call(
            "grouped",
            json!({"id": "nightly", "blueprint": "Blue", "root": "/srv/x"}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out["id"], "b1");
    // The blueprint reaches the resolver as its resolved id, and the path too.
    assert_eq!(*ctx.seen_scope.lock().unwrap(), [Some("bp".to_owned())]);
    assert_eq!(
        *ctx.seen_paths.lock().unwrap(),
        [(Some("/srv/x".to_owned()), None)]
    );
    // The typed path sees the same scope.
    let ctx = Ctx::default();
    let m = Grouped::add_args(Command::new("t"))
        .try_get_matches_from([
            "t",
            "--id",
            "nightly",
            "--blueprint",
            "Blue",
            "--root",
            "/r",
        ])
        .unwrap();
    let p = Grouped::from_matches(&m, &mut ParseCx::default()).unwrap();
    r.call_typed::<GroupedOp>(&ctx, p, CallOptions::default())
        .await
        .unwrap();
    assert_eq!(
        *ctx.seen_paths.lock().unwrap(),
        [(Some("/r".to_owned()), None)]
    );
}

#[derive(Debug, OpParams)]
struct Many {
    #[op(subject = "agent", id_only)]
    id: Ref,
    #[op(kind = "build")]
    builds: Vec<Ref>,
}

#[test]
fn vec_ref_field_reports_repeated_kind() {
    let fields = Many::fields();
    let builds = fields.iter().find(|f| f.rust_name == "builds").unwrap();
    assert_eq!(builds.kind, op_registry::FieldKind::Repeated);
    assert!(builds.subject.is_none());
}

#[test]
fn subject_field_meta_names_kind() {
    let fields = IdOnly::fields();
    assert_eq!(
        fields[0].subject,
        Some(op_registry::SubjectMeta::new("agent").id_only(true))
    );
    let s = fields[0].subject.as_ref().unwrap();
    assert_eq!(
        (s.kind.as_str(), s.id_only, s.absent_ok),
        ("agent", true, false)
    );
    assert_eq!(fields[0].kind, op_registry::FieldKind::Ref);
    let plain = Get::fields();
    assert_eq!(plain[0].subject.as_ref().map(|s| s.id_only), Some(false));
    let waits = Waits::fields();
    assert!(waits
        .iter()
        .find(|f| f.rust_name == "waits_for")
        .unwrap()
        .subject
        .is_none());
}

// ---- Optional subjects, `absent_ok` subjects, `Resolve::check_id` ----

#[derive(Debug, OpParams)]
struct OptSubject {
    #[op(subject = "build", scope = ["blueprint"])]
    id: Option<Ref>,
    #[op(kind = "blueprint")]
    blueprint: Option<Ref>,
}

#[derive(Debug, OpParams)]
struct IdOrAction {
    #[op(subject = "build", one_of = "target")]
    id: Option<Ref>,
    #[op(one_of = "target")]
    action: Option<String>,
}

#[derive(Debug, OpParams)]
struct AbsentOk {
    #[op(subject = "agent", absent_ok)]
    id: Ref,
}

#[derive(Debug, OpParams)]
struct AbsentOkOpt {
    #[op(subject = "agent", absent_ok)]
    id: Option<Ref>,
}

#[derive(Debug, OpParams)]
struct AbsentOkIdOnly {
    #[op(subject = "agent", absent_ok, id_only)]
    id: Ref,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct OptSeen {
    id: Option<String>,
    raw: bool,
    action: Option<String>,
}

fn opt_seen(id: &Option<Ref>, action: Option<String>) -> OptSeen {
    OptSeen {
        id: id.as_ref().map(|r| r.id().to_owned()),
        raw: id
            .as_ref()
            .is_some_and(|r| matches!(r.resolved(), Resolved::Raw(_))),
        action,
    }
}

/// A context whose `check_id` rejects ids with a space and records every call.
#[derive(Default)]
struct CheckCtx {
    checked: Mutex<Vec<(String, String)>>,
}

impl Resolve for CheckCtx {
    fn resolve<'a>(
        &'a self,
        _kind: &'a str,
        _r: &'a Ref,
        _scope: &'a Scope,
    ) -> BoxFuture<'a, anyhow::Result<Resolved>> {
        Box::pin(async { panic!("the resolver must not be called") })
    }

    fn check_id(&self, kind: &str, raw: &str) -> Result<(), UsageError> {
        self.checked
            .lock()
            .unwrap()
            .push((kind.to_owned(), raw.to_owned()));
        if raw.contains(' ') {
            return Err(UsageError::InvalidField {
                name: "id".to_owned(),
                reason: format!("`{raw}` is not a {kind} id"),
            });
        }
        Ok(())
    }
}

#[derive(op_registry::OpGlobals)]
struct NoGlobals {}

impl op_registry::CliContext for CheckCtx {
    type Globals = NoGlobals;
    fn from_globals(_: &NoGlobals) -> anyhow::Result<Self> {
        Ok(CheckCtx::default())
    }
}

struct AbsentOkOp;
impl Op for AbsentOkOp {
    type Ctx = CheckCtx;
    type Params = AbsentOk;
    type Output = Seen;
    const NAME: &'static str = "absent";
}

fn opt_registry() -> Registry<Ctx> {
    let mut r: Registry<Ctx> = Registry::new();
    r.push_op(
        OpSpec::<OptSubject, OptSeen>::new("opt", "Optional subject."),
        |_c: &Ctx, p: OptSubject, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<OptSeen>> {
            Box::pin(async move { Ok(opt_seen(&p.id, None)) })
        },
    );
    r.push_op(
        OpSpec::<IdOrAction, OptSeen>::new("pick", "Id or action."),
        |_c: &Ctx, p: IdOrAction, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<OptSeen>> {
            Box::pin(async move { Ok(opt_seen(&p.id, p.action)) })
        },
    );
    r.push_op(
        OpSpec::<AbsentOk, Seen>::new("absent", "Absent ok."),
        |_c: &Ctx, p: AbsentOk, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.id)) })
        },
    );
    r.push_op(
        OpSpec::<AbsentOkOpt, OptSeen>::new("absent_opt", "Absent ok, optional."),
        |_c: &Ctx, p: AbsentOkOpt, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<OptSeen>> {
            Box::pin(async move { Ok(opt_seen(&p.id, None)) })
        },
    );
    r.push_op(
        OpSpec::<AbsentOkIdOnly, Seen>::new("absent_id_only", "Absent ok and id only."),
        |_c: &Ctx, p: AbsentOkIdOnly, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.id)) })
        },
    );
    r
}

fn panicking() -> Ctx {
    Ctx {
        panic_on_resolve: true,
        ..Ctx::default()
    }
}

#[tokio::test]
async fn optional_subject_absent_is_none_without_resolver() {
    let fields = OptSubject::fields();
    let id = fields.iter().find(|f| f.key == "id").unwrap();
    assert_eq!(id.flag, "id");
    assert!(!id.required);
    assert!(id.subject.is_some());
    let r = opt_registry();
    // JSON
    let out = r
        .call("opt", json!({}), &panicking(), CallOptions::default())
        .await
        .unwrap();
    assert_eq!(out, json!({"id": null, "raw": false, "action": null}));
    // CLI parse, then typed
    let m = OptSubject::add_args(Command::new("t"))
        .try_get_matches_from(["t"])
        .unwrap();
    let p = OptSubject::from_matches(&m, &mut ParseCx::default()).unwrap();
    assert!(p.id.is_none());
    // The schema does not require `id`.
    let schema = r.get("opt").unwrap().input_schema.clone();
    assert!(
        !schema["required"]
            .as_array()
            .is_some_and(|a| a.contains(&json!("id"))),
        "{schema}"
    );
}

#[tokio::test]
async fn optional_subject_present_resolved_with_scope() {
    let r = opt_registry();
    let ctx = Ctx::default();
    let out = r
        .call(
            "opt",
            json!({"id": "nightly", "blueprint": "Blue"}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "b1", "raw": false, "action": null}));
    assert_eq!(*ctx.seen_scope.lock().unwrap(), [Some("bp".to_owned())]);
    // `--id` on the CLI
    let m = OptSubject::add_args(Command::new("t"))
        .try_get_matches_from(["t", "--id", "x"])
        .unwrap();
    let p = OptSubject::from_matches(&m, &mut ParseCx::default()).unwrap();
    assert_eq!(p.id.as_ref().unwrap().as_raw(), "x");
    // The subject without its optional scope member: no `blueprint` key.
    let ctx = Ctx::default();
    r.call("opt", json!({"id": "b1"}), &ctx, CallOptions::default())
        .await
        .unwrap();
    assert_eq!(*ctx.seen_scope.lock().unwrap(), [None]);
}

#[tokio::test]
async fn one_of_over_id_and_action() {
    let r = opt_registry();
    let ctx = Ctx::default();
    let go = |args| r.call("pick", args, &ctx, CallOptions::default());
    let out = go(json!({"id": "b1"})).await.unwrap();
    assert_eq!(out["id"], "b1");
    let out = go(json!({"action": "stop"})).await.unwrap();
    assert_eq!(out, json!({"id": null, "raw": false, "action": "stop"}));
    for bad in [json!({}), json!({"id": "b1", "action": "stop"})] {
        let err = go(bad).await.unwrap_err();
        assert_eq!(code(err.error()), "op_registry.usage.one_of");
    }
    let fields = IdOrAction::fields();
    assert!(fields.iter().all(|f| !f.required));
    let cmd = IdOrAction::add_args(Command::new("t"));
    assert!(cmd
        .clone()
        .try_get_matches_from(["t", "--id", "b1"])
        .is_ok());
    assert!(cmd
        .clone()
        .try_get_matches_from(["t", "--id", "b1", "--action", "x"])
        .is_err());
}

#[tokio::test]
async fn absent_ok_subject_never_resolved() {
    let r = opt_registry();
    let fields = AbsentOk::fields();
    assert_eq!(
        fields[0].subject,
        Some(op_registry::SubjectMeta::new("agent").absent_ok(true))
    );
    // JSON
    let out = r
        .call(
            "absent",
            json!({"id": "brand-new"}),
            &panicking(),
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "brand-new", "raw": false}));
    // typed
    let out = r
        .call_typed::<AbsentOkOpAny>(
            &panicking(),
            AbsentOk {
                id: Ref::new("brand-new"),
            },
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out.id, "brand-new");
    // the optional form: absent is None, present passes through
    let out = r
        .call(
            "absent_opt",
            json!({}),
            &panicking(),
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out["id"], json!(null));
    let out = r
        .call(
            "absent_opt",
            json!({"id": "x1"}),
            &panicking(),
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out["id"], "x1");
}

struct AbsentOkOpAny;
impl Op for AbsentOkOpAny {
    type Ctx = Ctx;
    type Params = AbsentOk;
    type Output = Seen;
    const NAME: &'static str = "absent";
}

#[tokio::test]
async fn absent_ok_with_id_only_allowed() {
    let fields = AbsentOkIdOnly::fields();
    let s = fields[0].subject.as_ref().unwrap();
    assert_eq!(
        (s.kind.as_str(), s.id_only, s.absent_ok),
        ("agent", true, true)
    );
    let r = opt_registry();
    let out = r
        .call(
            "absent_id_only",
            json!({"id": "z-1"}),
            &panicking(),
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "z-1", "raw": false}));
}

fn check_registry() -> Registry<CheckCtx> {
    let mut r: Registry<CheckCtx> = Registry::new();
    r.set_pillar("fix");
    r.set_bin("app", "1.0.0", "A test app.");
    r.push_op(
        OpSpec::<AbsentOk, Seen>::new("absent", "Absent ok."),
        |_c: &CheckCtx, p: AbsentOk, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.id)) })
        },
    );
    r.push_op(
        OpSpec::<IdOnly, Seen>::new("idonly", "Id only."),
        |_c: &CheckCtx, p: IdOnly, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.id)) })
        },
    );
    r.push_op(
        OpSpec::<AbsentOkIdOnly, Seen>::new("both", "Both."),
        |_c: &CheckCtx,
         p: AbsentOkIdOnly,
         _o: CallOptions|
         -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.id)) })
        },
    );
    r
}

#[tokio::test]
async fn check_id_runs_for_absent_ok_and_id_only() {
    let r = check_registry();
    let ctx = CheckCtx::default();
    for op in ["absent", "idonly", "both"] {
        r.call(op, json!({"id": "a-1"}), &ctx, CallOptions::default())
            .await
            .unwrap();
    }
    assert_eq!(
        *ctx.checked.lock().unwrap(),
        [
            ("agent".to_owned(), "a-1".to_owned()),
            ("agent".to_owned(), "a-1".to_owned()),
            ("agent".to_owned(), "a-1".to_owned()),
        ]
    );
    // A resolved subject is not checked, and a server kind is not checked.
    let mut server = check_registry();
    server.resolve_server("agent");
    let ctx = CheckCtx::default();
    server
        .call("idonly", json!({"id": "s 1"}), &ctx, CallOptions::default())
        .await
        .unwrap();
    assert!(ctx.checked.lock().unwrap().is_empty());
    // The default `check_id` accepts anything.
    assert!(Plain.check_id("agent", "anything at all").is_ok());
}

#[tokio::test]
async fn malformed_id_is_usage_invalid_every_surface() {
    use op_registry::cli::{self, Io, NoHooks};
    use op_registry::{mcp, Audience};
    let r = check_registry();
    let ctx = CheckCtx::default();
    // JSON
    let err = r
        .call(
            "absent",
            json!({"id": "bad id"}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "op_registry.usage.invalid");
    // typed
    let err = r
        .call_typed::<AbsentOkOp>(
            &ctx,
            AbsentOk {
                id: Ref::new("bad id"),
            },
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "op_registry.usage.invalid");
    // MCP
    let err = mcp::call_tool(
        &r,
        &ctx,
        Audience::Agent,
        "idonly",
        json!({"id": "bad id"}),
        CallOptions::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(code(err.error()), "op_registry.usage.invalid");
    // CLI: exit 2 for a malformed id, 0 for a well-formed one
    let run = |id: &str| {
        let argv: Vec<std::ffi::OsString> = ["app", "absent", "--id", id]
            .iter()
            .map(std::ffi::OsString::from)
            .collect();
        let (io, _cap) = Io::memory(b"", false, false);
        futures_executor::block_on(cli::run_with_io(&check_registry(), argv, &NoHooks, io))
    };
    assert_eq!(run("bad id"), 2);
    assert_eq!(run("good-id"), 0);
}

// ---- Non-subject refs: scope and `id_only` (PAR-2), `Patch<Ref>` (PAR-3) ----

#[derive(Debug, OpParams)]
struct NonSubjectScope {
    #[op(subject = "build")]
    id: Ref,
    root: Option<PathBuf>,
    #[op(kind = "blueprint", scope = ["root"])]
    blueprint: Option<Ref>,
    #[op(kind = "collection", scope = ["root"])]
    more: Vec<Ref>,
}

#[derive(Debug, OpParams)]
struct NonSubjectIdOnly {
    #[op(subject = "build")]
    id: Ref,
    #[op(kind = "action", id_only)]
    waits_for: Option<Ref>,
    #[op(kind = "action", id_only)]
    after: Vec<Ref>,
}

#[derive(Debug, OpParams)]
struct Homed {
    #[op(subject = "build")]
    id: Ref,
    #[op(kind = "place", clearable)]
    home_place: op_registry::Patch<Ref>,
}

struct NonSubjectScopeOp;
impl Op for NonSubjectScopeOp {
    type Ctx = Ctx;
    type Params = NonSubjectScope;
    type Output = OptSeen;
    const NAME: &'static str = "nonsubject_scope";
}

fn par_registry() -> Registry<Ctx> {
    let mut r: Registry<Ctx> = Registry::new();
    r.push_op(
        OpSpec::<NonSubjectScope, OptSeen>::new("nonsubject_scope", "Scoped non-subjects."),
        |_c: &Ctx, p: NonSubjectScope, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<OptSeen>> {
            Box::pin(async move { Ok(opt_seen(&p.blueprint, None)) })
        },
    );
    r.push_op(
        OpSpec::<NonSubjectIdOnly, OptSeen>::new("waits_id_only", "Id-only non-subjects."),
        |_c: &Ctx,
         p: NonSubjectIdOnly,
         _o: CallOptions|
         -> BoxFuture<'_, anyhow::Result<OptSeen>> {
            Box::pin(async move {
                let after: Vec<&str> = p.after.iter().map(Ref::id).collect();
                Ok(opt_seen(&p.waits_for, Some(after.join(","))))
            })
        },
    );
    r.push_op(
        OpSpec::<Homed, OptSeen>::new("homed", "Patch ref."),
        |_c: &Ctx, p: Homed, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<OptSeen>> {
            Box::pin(async move {
                Ok(match &p.home_place {
                    op_registry::Patch::Set(r) => opt_seen(&Some(r.clone()), Some("set".into())),
                    op_registry::Patch::Keep => opt_seen(&None, Some("keep".into())),
                    op_registry::Patch::Clear => opt_seen(&None, Some("clear".into())),
                })
            })
        },
    );
    r
}

fn roots(ctx: &Ctx, kind: &str) -> Vec<Option<String>> {
    ctx.seen_roots
        .lock()
        .unwrap()
        .iter()
        .filter(|(k, _)| k == kind)
        .map(|(_, root)| root.clone())
        .collect()
}

#[tokio::test]
async fn non_subject_ref_scope_reaches_resolver() {
    let r = par_registry();
    // JSON: both the Option<Ref> and each Vec<Ref> member see the scope.
    let ctx = Ctx::default();
    r.call(
        "nonsubject_scope",
        json!({"id": "nightly", "root": "/srv/x", "blueprint": "Blue", "more": ["c1", "c1"]}),
        &ctx,
        CallOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(roots(&ctx, "blueprint"), [Some("/srv/x".to_owned())]);
    assert_eq!(
        roots(&ctx, "collection"),
        [Some("/srv/x".to_owned()), Some("/srv/x".to_owned())]
    );
    // An absent scope member is left out of the scope.
    let ctx = Ctx::default();
    r.call(
        "nonsubject_scope",
        json!({"id": "nightly", "blueprint": "Blue"}),
        &ctx,
        CallOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(roots(&ctx, "blueprint"), [None]);
    // CLI-decoded typed params.
    let ctx = Ctx::default();
    let m = NonSubjectScope::add_args(Command::new("t"))
        .try_get_matches_from([
            "t",
            "--id",
            "nightly",
            "--root",
            "/r",
            "--blueprint",
            "Blue",
        ])
        .unwrap();
    let p = NonSubjectScope::from_matches(&m, &mut ParseCx::default()).unwrap();
    r.call_typed::<NonSubjectScopeOp>(&ctx, p, CallOptions::default())
        .await
        .unwrap();
    assert_eq!(roots(&ctx, "blueprint"), [Some("/r".to_owned())]);
}

#[tokio::test]
async fn non_subject_id_only_skips_resolver() {
    let mut r = par_registry();
    // The subject is server-resolved, so only the id_only refs could reach
    // the resolver, and it panics if called.
    r.resolve_server("build");
    let ctx = Ctx {
        panic_on_resolve: true,
        ..Ctx::default()
    };
    let out = r
        .call(
            "waits_id_only",
            json!({"id": "nightly", "waitsFor": "w-1", "after": ["a-1", "a-2"]}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "w-1", "raw": false, "action": "a-1,a-2"}));
}

#[tokio::test]
async fn id_without_blueprint_has_no_blueprint_scope_key() {
    // archegraph's case: `--id` alone reaches the build resolver without a
    // `blueprint` key in its scope.
    let r = opt_registry();
    let ctx = Ctx::default();
    r.call(
        "opt",
        json!({"id": "nightly"}),
        &ctx,
        CallOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(*ctx.seen_scope.lock().unwrap(), [None]);
    let ctx = Ctx::default();
    r.call(
        "opt",
        json!({"id": "nightly", "blueprint": "Blue"}),
        &ctx,
        CallOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(*ctx.seen_scope.lock().unwrap(), [Some("bp".to_owned())]);
}

#[tokio::test]
async fn patch_ref_set_resolves_keep_clear_do_not() {
    let r = par_registry();
    let ctx = Ctx::default();
    let out = r
        .call(
            "homed",
            json!({"id": "nightly", "homePlace": "Home"}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "p1", "raw": false, "action": "set"}));
    assert_eq!(roots(&ctx, "place").len(), 1);
    // An unknown name fails like any non-subject ref.
    let err = r
        .call(
            "homed",
            json!({"id": "nightly", "homePlace": "nowhere"}),
            &ctx,
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(err.error()), "fix.build.not_found");
    // Keep and Clear call no resolver for the place.
    for (input, action) in [
        (json!({"id": "nightly"}), "keep"),
        (json!({"id": "nightly", "homePlace": null}), "clear"),
    ] {
        let ctx = Ctx::default();
        let out = r
            .call("homed", input, &ctx, CallOptions::default())
            .await
            .unwrap();
        assert_eq!(out["action"], action);
        assert!(roots(&ctx, "place").is_empty());
    }
}

#[test]
fn patch_ref_clear_flag_and_json_null() {
    use op_registry::Patch;
    let cli = |args: &[&str]| {
        let mut argv = vec!["t", "--id", "b1"];
        argv.extend_from_slice(args);
        Homed::add_args(Command::new("t")).try_get_matches_from(argv)
    };
    let decode = |m: &clap::ArgMatches| Homed::from_matches(m, &mut ParseCx::default()).unwrap();
    assert!(matches!(decode(&cli(&[]).unwrap()).home_place, Patch::Keep));
    assert!(matches!(
        decode(&cli(&["--clear-home-place"]).unwrap()).home_place,
        Patch::Clear
    ));
    match decode(&cli(&["--home-place", "Home"]).unwrap()).home_place {
        Patch::Set(r) => assert_eq!(r.as_raw(), "Home"),
        _ => panic!("expected Set"),
    }
    assert!(cli(&["--home-place", "Home", "--clear-home-place"]).is_err());
    let json = |v| {
        Homed::from_json(v, &mut ParseCx::default())
            .unwrap()
            .home_place
    };
    assert!(matches!(json(json!({"id": "b1"})), Patch::Keep));
    assert!(matches!(
        json(json!({"id": "b1", "homePlace": null})),
        Patch::Clear
    ));
    match json(json!({"id": "b1", "homePlace": "Home"})) {
        Patch::Set(r) => assert_eq!(r.as_raw(), "Home"),
        _ => panic!("expected Set"),
    }
}

#[derive(Debug, OpParams)]
struct CheckNonSubject {
    #[op(subject = "agent", id_only)]
    id: Ref,
    #[op(kind = "place", id_only)]
    home: Option<Ref>,
    #[op(kind = "place", id_only)]
    more: Vec<Ref>,
    #[op(kind = "place", id_only, clearable)]
    moved_to: op_registry::Patch<Ref>,
}

#[tokio::test]
async fn check_id_runs_for_non_subject_id_only() {
    let mut r: Registry<CheckCtx> = Registry::new();
    r.push_op(
        OpSpec::<CheckNonSubject, Seen>::new("nonsubj", "Non-subject id_only."),
        |_c: &CheckCtx,
         p: CheckNonSubject,
         _o: CallOptions|
         -> BoxFuture<'_, anyhow::Result<Seen>> {
            Box::pin(async move { Ok(seen(&p.id)) })
        },
    );
    let ctx = CheckCtx::default();
    r.call(
        "nonsubj",
        json!({"id": "a-1", "home": "h-1", "more": ["m-1"], "movedTo": "t-1"}),
        &ctx,
        CallOptions::default(),
    )
    .await
    .unwrap();
    let mut checked = ctx.checked.lock().unwrap().clone();
    checked.sort();
    assert_eq!(
        checked,
        [
            ("agent".to_owned(), "a-1".to_owned()),
            ("place".to_owned(), "h-1".to_owned()),
            ("place".to_owned(), "m-1".to_owned()),
            ("place".to_owned(), "t-1".to_owned()),
        ]
    );
    // Keep and Clear are not checked.
    for moved in [json!(null), json!("absent")] {
        let ctx = CheckCtx::default();
        let mut input = json!({"id": "a-1"});
        if moved.is_null() {
            input["movedTo"] = moved;
        }
        r.call("nonsubj", input, &ctx, CallOptions::default())
            .await
            .unwrap();
        assert_eq!(ctx.checked.lock().unwrap().len(), 1);
    }
    // A malformed id is a usage error for each non-subject shape.
    for bad in [
        json!({"id": "a-1", "home": "h 1"}),
        json!({"id": "a-1", "more": ["m 1"]}),
        json!({"id": "a-1", "movedTo": "t 1"}),
    ] {
        let err = r
            .call("nonsubj", bad, &CheckCtx::default(), CallOptions::default())
            .await
            .unwrap_err();
        assert_eq!(code(err.error()), "op_registry.usage.invalid");
    }
}
