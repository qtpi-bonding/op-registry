mod common;

use common::*;
use op_registry::{CallOptions, OpKind, OpSpec, Paths, Registry};
use serde_json::json;

#[test]
fn push_op_registers_derived_name() {
    let registry = registry_of(&["dispatch-target list", "index"]);
    assert_eq!(registry.names(), vec!["dispatch_target_list", "index"]);
    let d = registry.get("dispatch_target_list").unwrap();
    assert_eq!(d.path, ["dispatch-target", "list"]);
    assert_eq!(d.verb(), "list");
    assert_eq!(d.noun_path(), ["dispatch-target"]);
    assert_eq!(d.kind, OpKind::Unary);
    assert_eq!(d.summary, "An op.");
    assert!(d.input_schema.is_object());
    #[cfg(feature = "schema")]
    assert!(d.output_schema.is_some());
}

#[test]
fn push_local_and_stream_set_their_kind() {
    let mut registry: Registry<Ctx> = Registry::new();
    registry.push_local(
        OpSpec::<Echo, Out>::new("here", "Local."),
        |_c: &Ctx, _p: Echo, _o: CallOptions| Box::pin(async { Ok(Out { ok: true }) }),
    );
    registry.push_stream(
        OpSpec::<Echo, Out>::new("flow", "Stream."),
        |_c: &Ctx, _p: Echo, _cur, _o: CallOptions| {
            Box::pin(async {
                Ok(
                    Box::pin(futures_util::stream::iter(vec![Ok(Out { ok: true })]))
                        as futures_util::stream::BoxStream<'_, anyhow::Result<Out>>,
                )
            })
        },
    );
    assert_eq!(registry.get("here").unwrap().kind, OpKind::Local);
    assert_eq!(registry.get("flow").unwrap().kind, OpKind::Stream);
}

#[test]
#[should_panic(expected = "concept_get")]
fn duplicate_name_panics_naming_both() {
    let mut registry = registry_of(&["concept get"]);
    push_ok(&mut registry, OpSpec::new("concept-get", "Again."));
    // `concept get` and `concept-get` both derive `concept_get`.
}

#[test]
#[should_panic(expected = "build")]
fn word_both_op_and_group_panics() {
    let mut registry: Registry<Ctx> = Registry::new();
    push_ok(&mut registry, OpSpec::new("build", "Build."));
    registry.group("build", "Builds.");
    push_ok(&mut registry, OpSpec::new("build status", "Status."));
}

#[test]
#[should_panic(expected = "build_status")]
fn word_both_op_and_group_panics_naming_the_child() {
    let mut registry: Registry<Ctx> = Registry::new();
    registry.group("build", "Builds.");
    push_ok(&mut registry, OpSpec::new("build status", "Status."));
    push_ok(&mut registry, OpSpec::new("build", "Build."));
}

#[test]
fn group_default_may_share_its_group_word() {
    let mut registry: Registry<Ctx> = Registry::new();
    registry.group("build", "Builds.");
    let mut run = OpSpec::new("build run", "Run.");
    run.group_default = true;
    push_ok(&mut registry, run);
    push_ok(&mut registry, OpSpec::new("build status", "Status."));
    assert_eq!(registry.names(), vec!["build_run", "build_status"]);
    assert!(registry.get("build_run").unwrap().group_default);
}

#[test]
fn group_default_op_with_the_group_word_itself_is_allowed() {
    let mut registry: Registry<Ctx> = Registry::new();
    registry.group("build", "Builds.");
    let mut build = OpSpec::new("build", "Build.");
    build.group_default = true;
    push_ok(&mut registry, build);
    push_ok(&mut registry, OpSpec::new("build status", "Status."));
    assert_eq!(registry.names(), vec!["build", "build_status"]);
}

#[test]
#[should_panic(expected = "build_run")]
fn two_defaults_in_group_panic() {
    let mut registry: Registry<Ctx> = Registry::new();
    registry.group("build", "Builds.");
    for path in ["build run", "build status"] {
        let mut spec = OpSpec::new(path, "Op.");
        spec.group_default = true;
        push_ok(&mut registry, spec);
    }
}

#[test]
#[should_panic(expected = "group \"concept\" has no summary")]
fn missing_group_summary_panics() {
    let mut registry: Registry<Ctx> = Registry::new();
    push_ok(&mut registry, OpSpec::new("concept get", "Get."));
}

#[test]
fn merge_keeps_order_and_checks_collisions() {
    let mut a = registry_of(&["alpha one", "beta"]);
    let b = registry_of(&["gamma two", "alpha two"]);
    a.merge(b);
    assert_eq!(
        a.names(),
        vec!["alpha_one", "beta", "gamma_two", "alpha_two"]
    );
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        a.merge(registry_of(&["beta"]));
    }));
    assert!(result.is_err(), "a duplicate name across merge must panic");
}

#[test]
fn typed_only_op_called_by_json_is_wrong_path() {
    let mut registry: Registry<Ctx> = Registry::new();
    let mut spec = OpSpec::new("x", "Typed only.");
    spec.paths = Paths::TypedOnly;
    push_ok(&mut registry, spec);
    let err =
        futures_executor::block_on(registry.call("x", json!({}), &Ctx, CallOptions::default()))
            .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "op_registry.call.wrong_path"
    );
}

#[test]
fn call_runs_a_json_op() {
    let registry = registry_of(&["x"]);
    let out = futures_executor::block_on(registry.call(
        "x",
        json!({"value": "v"}),
        &Ctx,
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(out, json!({"ok": true}));
    let err =
        futures_executor::block_on(registry.call("nope", json!({}), &Ctx, CallOptions::default()))
            .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "op_registry.call.unknown_operation"
    );
}
