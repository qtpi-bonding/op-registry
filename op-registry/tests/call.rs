//! The one call pipeline: hook, call info, notices and the typed path.

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use op_registry::cli::{self, Io, NoHooks};
use op_registry::{
    Access, Audience, CallFailure, CallHook, CallInfo, CallOptions, CallPath, Caller, CliContext,
    Notice, NoticeSink, Notices, Op, OpGlobals, OpParams, OpSpec, Paths, Registry, Surface,
    TraceParent,
};
use serde::Serialize;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Ctx;
impl op_registry::Resolve for Ctx {}

#[derive(Debug, OpGlobals)]
struct NoGlobals {}

impl CliContext for Ctx {
    type Globals = NoGlobals;
    fn from_globals(_globals: &NoGlobals) -> anyhow::Result<Self> {
        Ok(Ctx)
    }
}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct Pick {
    #[op(one_of = "target")]
    id: Option<String>,
    #[op(one_of = "target")]
    name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Out {
    ok: bool,
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.app")]
enum AppError {
    #[error("the app failed")]
    #[coded(code = "failed", class = Logical)]
    Failed,
    #[error("mapped")]
    #[coded(code = "mapped", class = Logical)]
    Mapped,
}

/// Maps an `io::Error` to the app's coded error; everything else passes.
fn map_io(err: anyhow::Error) -> anyhow::Error {
    if err.downcast_ref::<std::io::Error>().is_some() {
        coded_error::into_anyhow(AppError::Mapped)
    } else {
        err
    }
}

/// Maps every error it sees.
fn map_all(_err: anyhow::Error) -> anyhow::Error {
    coded_error::into_anyhow(AppError::Mapped)
}

/// Maps every error to a different code than `map_io` does.
fn map_other(_err: anyhow::Error) -> anyhow::Error {
    coded_error::into_anyhow(AppError::Failed)
}

#[derive(Clone, OpParams)]
struct Leaky {
    #[op(secret)]
    token: Option<String>,
}

struct IoFail;
impl Op for IoFail {
    type Ctx = Ctx;
    type Params = Pick;
    type Output = Out;
    const NAME: &'static str = "io_fail";
}
struct Leak;
impl Op for Leak {
    type Ctx = Ctx;
    type Params = Leaky;
    type Output = Out;
    const NAME: &'static str = "leak";
}
struct OpenFail;
impl Op for OpenFail {
    type Ctx = Ctx;
    type Params = Pick;
    type Output = Out;
    const NAME: &'static str = "open_fail";
}
struct ItemFail;
impl Op for ItemFail {
    type Ctx = Ctx;
    type Params = Leaky;
    type Output = Out;
    const NAME: &'static str = "item_fail";
}

struct Probe;
impl Op for Probe {
    type Ctx = Ctx;
    type Params = Pick;
    type Output = Out;
    const NAME: &'static str = "probe";
}
struct Boom;
impl Op for Boom {
    type Ctx = Ctx;
    type Params = Pick;
    type Output = Out;
    const NAME: &'static str = "boom";
}
struct Hidden;
impl Op for Hidden {
    type Ctx = Ctx;
    type Params = Pick;
    type Output = Out;
    const NAME: &'static str = "hidden";
}
struct Flow;
impl Op for Flow {
    type Ctx = Ctx;
    type Params = Pick;
    type Output = Out;
    const NAME: &'static str = "flow";
}
struct Mismatch;
impl Op for Mismatch {
    type Ctx = Ctx;
    type Params = Pick;
    type Output = String;
    const NAME: &'static str = "probe";
}

fn ok_op(_c: &Ctx, _p: Pick, _o: CallOptions) -> BoxFuture<'_, anyhow::Result<Out>> {
    Box::pin(async { Ok(Out { ok: true }) })
}

fn registry() -> Registry<Ctx> {
    let mut r: Registry<Ctx> = Registry::new();
    let mut probe = OpSpec::<Pick, Out>::new("probe", "Probe.");
    probe.access = Access::writes();
    probe.audience = Audience::Owner;
    probe.fields = Some(|p| vec![("collection_id", p.id.clone().unwrap_or_default())]);
    r.push_op(probe, ok_op);
    r.push_op(
        OpSpec::<Pick, Out>::new("boom", "Fails."),
        |_c: &Ctx, _p: Pick, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async { Err(coded_error::into_anyhow(AppError::Failed)) })
        },
    );
    let mut hidden = OpSpec::<Pick, Out>::new("hidden", "Typed only.");
    hidden.paths = Paths::of(&[Surface::Typed]);
    r.push_op(hidden, ok_op);
    r.push_op(
        OpSpec::<Pick, Out>::new("notify", "Notifies."),
        |_c: &Ctx, _p: Pick, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async move {
                o.notices.notice("x");
                o.notices.progress(1, Some(2), "l");
                Ok(Out { ok: true })
            })
        },
    );
    r.push_op(
        OpSpec::<Pick, Out>::new("io_fail", "Fails with io."),
        |_c: &Ctx, _p: Pick, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async { Err(std::io::Error::other("disk gone").into()) })
        },
    );
    r.push_op(
        OpSpec::<Leaky, Out>::new("leak", "Leaks its secret."),
        |_c: &Ctx, p: Leaky, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(
                async move { Err(anyhow::anyhow!("rejected {}", p.token.unwrap_or_default())) },
            )
        },
    );
    r.push_stream(
        OpSpec::<Pick, Out>::new("open_fail", "Fails to open."),
        |_c: &Ctx, _p: Pick, _cur, _o: CallOptions| {
            Box::pin(async { Err(std::io::Error::other("no socket").into()) })
        },
    );
    r.push_stream(
        OpSpec::<Leaky, Out>::new("item_fail", "Fails on an item."),
        |_c: &Ctx, p: Leaky, _cur, _o: CallOptions| {
            let token = p.token.unwrap_or_default();
            Box::pin(async move {
                Ok(Box::pin(futures_util::stream::iter(vec![
                    Ok(Out { ok: true }),
                    Err(anyhow::Error::from(std::io::Error::other(format!(
                        "dropped {token}"
                    )))),
                ])) as BoxStream<'_, anyhow::Result<Out>>)
            })
        },
    );
    r.push_stream(
        OpSpec::<Pick, Out>::new("flow", "Streams."),
        |_c: &Ctx, _p: Pick, _cur, _o: CallOptions| {
            Box::pin(async {
                Ok(
                    Box::pin(futures_util::stream::iter(vec![Ok(Out { ok: true })]))
                        as BoxStream<'_, anyhow::Result<Out>>,
                )
            })
        },
    );
    r
}

fn pick() -> Pick {
    Pick {
        id: Some("a".into()),
        name: None,
    }
}

#[derive(Default)]
struct Recorder {
    infos: Mutex<Vec<CallInfo>>,
    errors: Mutex<Vec<String>>,
}

impl CallHook for Recorder {
    fn around<'a>(&'a self, info: &CallInfo, call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()> {
        self.infos.lock().unwrap().push(info.clone());
        call
    }
    fn on_error(&self, _info: &CallInfo, err: &anyhow::Error) {
        self.errors.lock().unwrap().push(err.to_string());
    }
}

fn hooked() -> (Registry<Ctx>, Arc<Recorder>) {
    let mut r = registry();
    let rec = Arc::new(Recorder::default());
    r.set_call_hook(rec.clone());
    (r, rec)
}

#[tokio::test]
async fn hook_sees_every_path_with_path_label() {
    let (r, rec) = hooked();
    let opts = CallOptions::default().with_caller(Caller("agent-1".into()));
    r.call_typed::<Probe>(&Ctx, pick(), opts.clone())
        .await
        .unwrap();
    r.call(
        "probe",
        json!({"id": "a"}),
        &Ctx,
        opts.clone().with_transport("rest"),
    )
    .await
    .unwrap();
    r.call("probe", json!({"id": "a"}), &Ctx, opts)
        .await
        .unwrap();
    let infos = rec.infos.lock().unwrap();
    assert_eq!(infos.len(), 3);
    assert_eq!(infos[0].path, CallPath::Typed);
    assert_eq!(infos[1].path, CallPath::Transport("rest".into()));
    assert_eq!(infos[2].path, CallPath::Transport("json".into()));
    for info in infos.iter() {
        assert_eq!(info.name, "probe");
        assert_eq!(info.access, Access::writes());
        assert_eq!(info.audience, Audience::Owner);
        assert_eq!(info.caller, Some(Caller("agent-1".into())));
    }
}

#[tokio::test]
async fn fields_fn_feeds_call_info() {
    let (r, rec) = hooked();
    r.call_typed::<Probe>(&Ctx, pick(), CallOptions::default())
        .await
        .unwrap();
    r.call("probe", json!({"id": "b"}), &Ctx, CallOptions::default())
        .await
        .unwrap();
    let infos = rec.infos.lock().unwrap();
    assert_eq!(infos[0].fields, vec![("collection_id", "a".to_owned())]);
    assert_eq!(infos[1].fields, vec![("collection_id", "b".to_owned())]);
}

#[tokio::test]
async fn trace_parent_passed_through_untouched() {
    let (r, rec) = hooked();
    let trace = TraceParent {
        traceparent: Some("00-abc-def-01".into()),
        tracestate: Some("k=v".into()),
        baggage: Some("b=1".into()),
    };
    r.call_typed::<Probe>(
        &Ctx,
        pick(),
        CallOptions::default().with_trace(trace.clone()),
    )
    .await
    .unwrap();
    assert_eq!(rec.infos.lock().unwrap()[0].trace, trace);
}

#[tokio::test]
async fn on_error_called_once_on_failure() {
    let (r, rec) = hooked();
    assert!(r
        .call("boom", json!({"id": "a"}), &Ctx, CallOptions::default())
        .await
        .is_err());
    assert!(r
        .call_typed::<Boom>(&Ctx, pick(), CallOptions::default())
        .await
        .is_err());
    assert_eq!(rec.errors.lock().unwrap().len(), 2);
    r.call_typed::<Probe>(&Ctx, pick(), CallOptions::default())
        .await
        .unwrap();
    assert_eq!(rec.errors.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn typed_error_keeps_concrete_type() {
    let r = registry();
    let err = r
        .call_typed::<Boom>(&Ctx, pick(), CallOptions::default())
        .await
        .unwrap_err();
    assert!(coded_error::downcast_ref::<AppError>(err.error()).is_some());
}

#[tokio::test]
async fn typed_path_runs_validate() {
    let r = registry();
    let err = r
        .call_typed::<Probe>(
            &Ctx,
            Pick {
                id: None,
                name: None,
            },
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "op_registry.usage.one_of"
    );
}

#[tokio::test]
async fn typed_only_op_callable_typed() {
    let r = registry();
    assert!(r
        .call_typed::<Hidden>(&Ctx, pick(), CallOptions::default())
        .await
        .is_ok());
    let err = r
        .call("hidden", json!({"id": "a"}), &Ctx, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "op_registry.call.wrong_path"
    );
}

#[tokio::test]
async fn typed_type_mismatch_is_wrong_shape() {
    let r = registry();
    let err = r
        .call_typed::<Mismatch>(&Ctx, pick(), CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "op_registry.call.wrong_shape"
    );
    let err = r
        .call_stream_typed::<Probe>(&Ctx, pick(), CallOptions::default(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "op_registry.call.wrong_shape"
    );
}

struct Skip;
impl CallHook for Skip {
    fn around<'a>(&'a self, _info: &CallInfo, _call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn on_error(&self, _info: &CallInfo, _err: &anyhow::Error) {
        ERRORS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
static ERRORS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[tokio::test]
async fn hook_dropping_call_is_error_not_hang() {
    let mut r = registry();
    r.set_call_hook(Arc::new(Skip));
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        r.call_typed::<Probe>(&Ctx, pick(), CallOptions::default()),
    )
    .await
    .expect("no hang");
    let err = result.unwrap_err();
    assert!(err.to_string().contains("call hook did not run the call"));
    assert_eq!(coded_error::facts_anyhow(err.error()).code, "unknown");
    assert_eq!(ERRORS.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[derive(Default)]
struct VecSink(Mutex<Vec<Notice>>);
impl NoticeSink for VecSink {
    fn send(&self, notice: Notice) {
        self.0.lock().unwrap().push(notice);
    }
}

#[tokio::test]
async fn notices_reach_sink() {
    let r = registry();
    let sink = Arc::new(VecSink::default());
    let opts = CallOptions::default().with_notices(Notices::new(sink.clone()));
    r.call("notify", json!({"id": "a"}), &Ctx, opts)
        .await
        .unwrap();
    assert_eq!(
        *sink.0.lock().unwrap(),
        vec![
            Notice::Text("x".into()),
            Notice::Progress {
                done: 1,
                total: Some(2),
                label: "l".into()
            }
        ]
    );
    // No sink: dropped without error.
    r.call("notify", json!({"id": "a"}), &Ctx, CallOptions::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn stream_open_goes_through_hook() {
    let (r, rec) = hooked();
    let items: Vec<_> = r
        .call_stream(
            "flow",
            json!({"id": "a"}),
            &Ctx,
            CallOptions::default(),
            None,
        )
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(items.len(), 1);
    let typed: Vec<_> = r
        .call_stream_typed::<Flow>(&Ctx, pick(), CallOptions::default(), None)
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(typed[0].as_ref().unwrap(), &Out { ok: true });
    let infos = rec.infos.lock().unwrap();
    assert_eq!(infos.len(), 2);
    assert_eq!(infos[0].path, CallPath::Transport("json".into()));
    assert_eq!(infos[1].path, CallPath::Typed);
}

#[tokio::test]
async fn wrong_shape_call_on_stream() {
    let r = registry();
    let err = r
        .call("flow", json!({"id": "a"}), &Ctx, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "op_registry.call.wrong_shape"
    );
}

#[tokio::test]
async fn wrong_shape_is_checked_before_decoding() {
    let r = registry();
    // `flow` is a stream and `probe` is unary; the malformed args (both
    // `id` and `name` break the one_of group) must not hide the shape error.
    let bad = json!({"id": "a", "name": "b"});
    let err = r
        .call("flow", bad.clone(), &Ctx, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "op_registry.call.wrong_shape"
    );
    let err = r
        .call_stream("probe", bad, &Ctx, CallOptions::default(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "op_registry.call.wrong_shape"
    );
}

fn code_of(failure: &CallFailure) -> &'static str {
    coded_error::facts_anyhow(failure.error()).code
}

fn run_cli(r: &Registry<Ctx>, args: &[&str]) -> (u8, String) {
    let argv: Vec<std::ffi::OsString> = std::iter::once("app")
        .chain(args.iter().copied())
        .map(Into::into)
        .collect();
    let (io, cap) = Io::memory(b"", false, false);
    let code = futures_executor::block_on(cli::run_with_io(r, argv, &NoHooks, io));
    (code, cap.stderr())
}

#[tokio::test]
async fn mapper_rewrites_io_error_on_every_surface() {
    let mut r = registry();
    r.set_error_mapper(map_io);
    let (_, line) = run_cli(&r, &["io_fail", "--id", "a"]);
    assert!(line.contains("test.app.mapped"), "{line}");
    let err = r
        .call("io_fail", json!({"id": "a"}), &Ctx, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), "test.app.mapped");
    let err = op_registry::mcp::call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "io_fail",
        json!({"id": "a"}),
        CallOptions::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(code_of(&err), "test.app.mapped");
    let err = r
        .call_typed::<IoFail>(&Ctx, pick(), CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), "test.app.mapped");
}

#[tokio::test]
async fn hook_on_error_sees_mapped_error() {
    let (mut r, rec) = hooked();
    r.set_error_mapper(map_io);
    r.call_typed::<IoFail>(&Ctx, pick(), CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(rec.errors.lock().unwrap().as_slice(), ["mapped"]);
}

#[tokio::test]
async fn usage_error_not_mapped() {
    let (mut r, rec) = hooked();
    r.set_error_mapper(map_all);
    let err = r
        .call(
            "probe",
            json!({"id": "a", "name": "b"}),
            &Ctx,
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), "op_registry.usage.one_of");
    let err = r
        .call_typed::<Probe>(
            &Ctx,
            Pick {
                id: None,
                name: None,
            },
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), "op_registry.usage.one_of");
    let (code, line) = run_cli(&r, &["probe", "--id", "a", "--name", "b"]);
    assert_eq!(code, 2, "{line}");
    assert!(rec.errors.lock().unwrap().is_empty());
    // An op error is mapped.
    let err = r
        .call_typed::<Boom>(&Ctx, pick(), CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), "test.app.mapped");
}

#[tokio::test]
async fn merge_keeps_receiver_mapper() {
    let mut receiver: Registry<Ctx> = Registry::new();
    receiver.set_error_mapper(map_io);
    let mut other = registry();
    other.set_error_mapper(map_other);
    receiver.merge(other);
    let err = receiver
        .call_typed::<IoFail>(&Ctx, pick(), CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), "test.app.mapped");
}

#[tokio::test]
async fn typed_error_redacts_secret_param() {
    let r = registry();
    let leaky = Leaky {
        token: Some("hunter2".into()),
    };
    let err = r
        .call_typed::<Leak>(&Ctx, leaky, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "rejected <secret>");
    assert!(!format!("{err:?}").contains("hunter2"));
}

#[tokio::test]
async fn typed_failure_downcasts_to_op_error() {
    let r = registry();
    let err = r
        .call_typed::<Boom>(&Ctx, pick(), CallOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(
        coded_error::downcast_ref::<AppError>(err.error()),
        Some(AppError::Failed)
    ));
}

#[tokio::test]
async fn into_coded_keeps_code() {
    let r = registry();
    let err = r
        .call_typed::<Boom>(&Ctx, pick(), CallOptions::default())
        .await
        .unwrap_err();
    let coded = err.into_coded();
    assert_eq!(coded_error::facts_anyhow(&coded).code, "test.app.failed");
    let leaky = Leaky {
        token: Some("hunter2".into()),
    };
    let err = r
        .call_typed::<Leak>(&Ctx, leaky, CallOptions::default())
        .await
        .unwrap_err();
    assert!(!format!("{:#}", err.into_coded()).contains("hunter2"));
}

#[tokio::test]
async fn stream_open_and_item_errors_are_call_failures() {
    let r = registry();
    let open: Result<BoxStream<'_, Result<serde_json::Value, CallFailure>>, CallFailure> = r
        .call_stream(
            "open_fail",
            json!({"id": "a"}),
            &Ctx,
            CallOptions::default(),
            None,
        )
        .await;
    assert!(open.is_err());
    let open = r
        .call_stream_typed::<OpenFail>(&Ctx, pick(), CallOptions::default(), None)
        .await;
    assert!(open.is_err());
    let leaky = || Leaky {
        token: Some("hunter2".into()),
    };
    let items: Vec<_> = r
        .call_stream_typed::<ItemFail>(&Ctx, leaky(), CallOptions::default(), None)
        .await
        .unwrap()
        .collect()
        .await;
    let err = items[1].as_ref().unwrap_err();
    assert!(err.to_string().contains("dropped <secret>"), "{err}");
    let items: Vec<_> = r
        .call_stream(
            "item_fail",
            json!({"token": "hunter2"}),
            &Ctx,
            CallOptions::default(),
            None,
        )
        .await
        .unwrap()
        .collect()
        .await;
    let err = items[1].as_ref().unwrap_err();
    assert!(err.to_string().contains("dropped <secret>"), "{err}");
}

#[tokio::test]
async fn stream_item_errors_are_mapped() {
    let mut r = registry();
    r.set_error_mapper(map_io);
    let items: Vec<_> = r
        .call_stream_typed::<ItemFail>(&Ctx, Leaky { token: None }, CallOptions::default(), None)
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(code_of(items[1].as_ref().unwrap_err()), "test.app.mapped");
    let err = r
        .call_stream_typed::<OpenFail>(&Ctx, pick(), CallOptions::default(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(code_of(&err), "test.app.mapped");
}
