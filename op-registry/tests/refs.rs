//! Refs: the subject flag, scopes and resolution on every call path.

use clap::Command;
use futures_util::future::BoxFuture;
use op_registry::{
    CallOptions, Op, OpParams, OpSpec, ParseCx, Ref, Registry, Resolve, Resolved, Scope,
};
use serde::Serialize;
use serde_json::json;
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
            match kind {
                "build" => {
                    self.seen_scope
                        .lock()
                        .unwrap()
                        .push(scope.get("blueprint").map(str::to_owned));
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
    assert_eq!(code(&err), "fix.build.not_found");
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
    assert_eq!(code(&err), "fix.build.not_found");
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
    assert_eq!(code(&err), "fix.build.not_found");
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
