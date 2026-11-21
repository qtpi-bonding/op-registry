mod common;

use common::*;
use op_registry::{CallOptions, OpKind, OpSpec, Paths, Registry, Surface};
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
    spec.paths = Paths::of(&[Surface::Typed]);
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

#[test]
#[should_panic(expected = "group \"concept\" is declared twice with different summaries")]
fn merge_with_conflicting_group_summary_panics() {
    let mut registry: Registry<Ctx> = Registry::new();
    registry.group("concept", "Concepts.");
    let mut other: Registry<Ctx> = Registry::new();
    other.group("concept", "Something else.");
    registry.merge(other);
}

#[test]
fn merge_with_equal_group_summary_is_accepted() {
    let mut registry: Registry<Ctx> = Registry::new();
    registry.group("concept", "Concepts.");
    let mut other: Registry<Ctx> = Registry::new();
    other.group("concept", "Concepts.");
    registry.merge(other);
}

fn code(code: &'static str, class: coded_error::ErrorClass) -> coded_error::CodeEntry {
    coded_error::CodeEntry {
        code,
        class: Some(class),
    }
}

#[test]
fn merge_unions_common_errors() {
    #[derive(Default)]
    struct Counter(std::sync::atomic::AtomicUsize);
    impl op_registry::CallHook for Counter {
        fn on_error(&self, _i: &op_registry::CallInfo, _e: &anyhow::Error) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    use coded_error::ErrorClass::{Logical, Transient};
    let mut a = registry_of(&["alpha"]);
    let seen = std::sync::Arc::new(Counter::default());
    a.set_call_hook(seen.clone());
    a.add_common_errors(vec![code("x.denied", Logical), code("x.shared", Logical)]);
    let mut b = registry_of(&["beta"]);
    b.group("g", "G.");
    b.add_common_errors(vec![code("x.shared", Logical), code("x.busy", Transient)]);
    a.merge(b);
    let codes: Vec<&str> = a.common_errors().iter().map(|e| e.code.as_str()).collect();
    assert_eq!(codes, vec!["x.denied", "x.shared", "x.busy"]);
    // The receiver's hook and the other's groups survive too.
    assert_eq!(a.names(), vec!["alpha", "beta"]);
    // The other's group came along: an op under it registers.
    push_ok(&mut a, OpSpec::new("g gamma", "Uses the other's group."));
    // The receiver's hook sees a failure of a merged-in op.
    let err = futures_executor::block_on(a.call(
        "beta",
        json!({"bogus": 1}),
        &Ctx,
        CallOptions::default(),
    ));
    assert!(err.is_err());
    let err = futures_executor::block_on(a.call("alpha", json!({}), &Ctx, CallOptions::default()));
    assert!(err.is_ok());
    assert_eq!(seen.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    a.push_op(
        OpSpec::<Echo, Out>::new("fails", "Fails."),
        |_c: &Ctx,
         _p: Echo,
         _o: CallOptions|
         -> futures_util::future::BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async { Err(anyhow::anyhow!("no")) })
        },
    );
    let _ = futures_executor::block_on(a.call("fails", json!({}), &Ctx, CallOptions::default()));
    assert_eq!(seen.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn merge_class_clash_panics() {
    use coded_error::ErrorClass::{Logical, Transient};
    let mut a = registry_of(&["alpha"]);
    a.add_common_errors(vec![code("x.same", Logical)]);
    let mut b = registry_of(&["beta"]);
    b.add_common_errors(vec![code("x.same", Transient)]);
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| a.merge(b))).unwrap_err();
    let text = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap();
    assert!(text.contains("x.same"), "{text}");
    assert!(
        text.contains("Logical") && text.contains("Transient"),
        "{text}"
    );
}
