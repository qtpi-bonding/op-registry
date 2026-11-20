//! The one call pipeline: hook, call info, notices and the typed path.

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use op_registry::{
    Access, Audience, CallHook, CallInfo, CallOptions, CallPath, Caller, Notice, NoticeSink,
    Notices, Op, OpParams, OpSpec, Paths, Registry, Surface, TraceParent,
};
use serde::Serialize;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Ctx;
impl op_registry::Resolve for Ctx {}

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
    assert!(coded_error::downcast_ref::<AppError>(&err).is_some());
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
        coded_error::facts_anyhow(&err).code,
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
        coded_error::facts_anyhow(&err).code,
        "op_registry.call.wrong_shape"
    );
    let err = r
        .call_stream_typed::<Probe>(&Ctx, pick(), CallOptions::default(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
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
    assert_eq!(coded_error::facts_anyhow(&err).code, "unknown");
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
        coded_error::facts_anyhow(&err).code,
        "op_registry.call.wrong_shape"
    );
}
