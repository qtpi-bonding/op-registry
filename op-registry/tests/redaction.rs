//! Redaction on every path: secret globals, `Registry::call` failures, the
//! hooks' views of an error, JSON decode errors and `Parsed`'s `Debug`.
#![cfg(feature = "secrets")]

use futures_util::future::BoxFuture;
use op_registry::cli::UsageFailure;
use op_registry::cli::{self, Io, Outcome, RunHooks};
use op_registry::{
    usage, Audience, CallHook, CallInfo, CallOptions, CliContext, Notice, NoticeSink, Notices, Op,
    OpGlobals, OpParams, OpSpec, ParseCx, Registry, SecretFile, UsageError,
};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::sync::{Arc, Mutex};

const TOKEN_ENV: &str = "OPREG_REDACTION_TOKEN_UNIQUE";

/// Serialises the tests that set or read `TOKEN_ENV`.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Process settings of the test binary.
#[derive(Debug, OpGlobals)]
struct G {
    /// The API token.
    #[op(secret)]
    #[op(env = "OPREG_REDACTION_TOKEN_UNIQUE")]
    token: Option<String>,
    /// Makes `from_globals` fail with the token in its message.
    fail_context: bool,
}

struct Ctx {
    token: String,
}

impl op_registry::Resolve for Ctx {}

impl CliContext for Ctx {
    type Globals = G;
    fn from_globals(globals: &G) -> anyhow::Result<Self> {
        let token = globals.token.clone().unwrap_or_default();
        if globals.fail_context {
            anyhow::bail!("cannot log in with {token}");
        }
        Ok(Ctx { token })
    }
}

#[derive(Clone, OpParams)]
struct Nothing {}

#[derive(Clone, OpParams)]
struct Vault {
    /// The token.
    #[op(secret)]
    token: Option<String>,
}

fn registry() -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    r.push_op(
        OpSpec::<Nothing, Value>::new("echo", "Echoes the global token."),
        |c: &Ctx, _p: Nothing, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
            Box::pin(async move {
                o.notices.notice(format!("using {}", c.token));
                o.notices.progress(1, None, &c.token);
                anyhow::bail!("rejected {}", c.token)
            })
        },
    );
    r.push_op(
        OpSpec::<Vault, Value>::new("vault", "Echoes its own token."),
        |_c: &Ctx, p: Vault, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
            Box::pin(async move { anyhow::bail!("rejected {}", p.token.unwrap_or_default()) })
        },
    );
    r
}

fn argv(args: &[&str]) -> Vec<OsString> {
    std::iter::once("app")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect()
}

#[derive(Default)]
struct Seen {
    post_run: Mutex<Vec<String>>,
    on_error: Mutex<Vec<String>>,
    usage: Mutex<Vec<String>>,
}

impl RunHooks for Seen {
    fn on_usage_error(&self, failure: &UsageFailure) {
        self.usage.lock().unwrap().push(failure.message.clone());
    }

    fn post_run(&self, outcome: &Outcome) {
        let mut seen = self.post_run.lock().unwrap();
        seen.push(format!("{outcome:?}"));
        if let Some(message) = outcome.message() {
            seen.push(message.to_owned());
        }
    }
}

impl CallHook for Seen {
    fn on_error(&self, info: &CallInfo, err: &anyhow::Error) {
        self.on_error
            .lock()
            .unwrap()
            .push(info.redact(&format!("{err:#}")));
    }
}

struct Ran {
    code: u8,
    err: String,
}

fn run(args: &[&str], seen: &Arc<Seen>) -> Ran {
    run_on(registry(), args, b"", seen)
}

fn run_on(mut r: Registry<Ctx>, args: &[&str], stdin: &[u8], seen: &Arc<Seen>) -> Ran {
    r.set_call_hook(seen.clone());
    let (io, cap) = Io::memory(stdin, false, false);
    let code = futures_executor::block_on(cli::run_with_io(&r, argv(args), &**seen, io));
    Ran {
        code,
        err: cap.stderr(),
    }
}

fn assert_hidden(ran: &Ran, seen: &Seen, secret: &str) {
    assert!(!ran.err.contains(secret), "{}", ran.err);
    for line in seen.post_run.lock().unwrap().iter() {
        assert!(!line.contains(secret), "post_run saw {line}");
    }
    for line in seen.on_error.lock().unwrap().iter() {
        assert!(!line.contains(secret), "on_error saw {line}");
    }
}

#[test]
fn argv_secret_global_is_redacted_from_error_and_notices() {
    let _env = env_lock();
    std::env::remove_var(TOKEN_ENV);
    let seen = Arc::new(Seen::default());
    let ran = run(&["--token", "argv-s3cret-global", "echo"], &seen);
    assert_eq!(ran.code, 70, "{}", ran.err);
    assert_hidden(&ran, &seen, "argv-s3cret-global");
    let lines: Vec<&str> = ran.err.lines().collect();
    assert_eq!(lines.len(), 3, "{}", ran.err);
    assert_eq!(lines[0], r#"{"notice":"using <secret>"}"#);
    assert!(lines[1].contains(r#""label":"<secret>""#), "{}", lines[1]);
    assert!(lines[2].contains("rejected <secret>"), "{}", lines[2]);
    assert_eq!(seen.on_error.lock().unwrap().len(), 1);
}

#[test]
fn env_secret_global_is_redacted_from_error_and_notices() {
    let _env = env_lock();
    std::env::set_var(TOKEN_ENV, "env-s3cret-global");
    let seen = Arc::new(Seen::default());
    let ran = run(&["echo"], &seen);
    std::env::remove_var(TOKEN_ENV);
    assert_eq!(ran.code, 70, "{}", ran.err);
    assert_hidden(&ran, &seen, "env-s3cret-global");
    assert!(ran.err.contains("rejected <secret>"), "{}", ran.err);
    assert!(ran.err.contains("using <secret>"), "{}", ran.err);
}

#[test]
fn env_secret_global_is_redacted_from_context_error() {
    let _env = env_lock();
    std::env::set_var(TOKEN_ENV, "env-s3cret-context");
    let seen = Arc::new(Seen::default());
    let ran = run(&["--fail-context", "echo"], &seen);
    std::env::remove_var(TOKEN_ENV);
    assert_eq!(ran.code, 70, "{}", ran.err);
    assert_hidden(&ran, &seen, "env-s3cret-context");
    assert!(
        ran.err.contains("cannot log in with <secret>"),
        "{}",
        ran.err
    );
}

#[test]
fn op_secret_is_redacted_from_post_run_and_on_error() {
    let _env = env_lock();
    std::env::remove_var(TOKEN_ENV);
    let seen = Arc::new(Seen::default());
    let ran = run(&["vault", "--token", "op-s3cret-token"], &seen);
    assert_eq!(ran.code, 70, "{}", ran.err);
    assert_hidden(&ran, &seen, "op-s3cret-token");
    let on_error = seen.on_error.lock().unwrap();
    assert_eq!(on_error.as_slice(), ["rejected <secret>"]);
}

#[test]
fn parsed_debug_leaves_out_secret_values() {
    let _env = env_lock();
    std::env::remove_var(TOKEN_ENV);
    let r = registry();
    let (parsed, _) = cli::parse(
        &r,
        &argv(&[
            "--token",
            "debug-s3cret-global",
            "vault",
            "--token",
            "debug-s3cret-op",
        ]),
    )
    .unwrap();
    let shown = format!("{parsed:?}");
    assert!(shown.contains("vault"), "{shown}");
    assert!(!shown.contains("debug-s3cret"), "{shown}");
}

#[test]
fn registry_call_failure_is_redacted() {
    let r = registry();
    let failure = futures_executor::block_on(r.call(
        "vault",
        json!({"token": "call-s3cret-token"}),
        &Ctx {
            token: String::new(),
        },
        CallOptions::default(),
    ))
    .unwrap_err();
    assert_eq!(failure.redacted_message(), "rejected <secret>");
    assert_eq!(failure.to_string(), "rejected <secret>");
    let debug = format!("{failure:?}");
    assert!(!debug.contains("call-s3cret-token"), "{debug}");
    // The raw error stays reachable for the caller that wants its type.
    assert!(failure.error().to_string().contains("call-s3cret-token"));
}

#[derive(Debug, OpParams)]
struct Pin {
    /// A numeric secret.
    #[op(secret)]
    pin: Option<u64>,
}

#[derive(Debug, OpParams)]
struct HiddenKey {
    /// A key whose path is secret too.
    #[op(secret)]
    key: Option<SecretFile>,
}

#[test]
fn json_decode_errors_do_not_echo_secret_values() {
    let err =
        Pin::from_json(json!({"pin": "json-s3cret-pin"}), &mut ParseCx::default()).unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "op_registry.usage.invalid"
    );
    let shown = format!("{err:#}");
    assert!(!shown.contains("json-s3cret-pin"), "{shown}");
    assert!(shown.contains("pin"), "{shown}");

    let err = HiddenKey::from_json(
        json!({"key": "/nonexistent/json-s3cret-path.key"}),
        &mut ParseCx::default(),
    )
    .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "op_registry.usage.invalid"
    );
    let shown = format!("{err:#}");
    assert!(!shown.contains("json-s3cret-path"), "{shown}");
}

// SEC-1: one shared redactor per call. Every op below declares no secrets and
// registers one with `opts.redact` while it runs.

const S: &str = "s3cr3t-value";

#[derive(Clone, OpParams)]
struct KeyFile {
    /// The key file.
    key: SecretFile,
}

struct LeakOp;
impl Op for LeakOp {
    type Ctx = Ctx;
    type Params = Nothing;
    type Output = Value;
    const NAME: &'static str = "leak";
}
struct StreamLeakOp;
impl Op for StreamLeakOp {
    type Ctx = Ctx;
    type Params = Nothing;
    type Output = Value;
    const NAME: &'static str = "stream_leak";
}
struct InnerOp;
impl Op for InnerOp {
    type Ctx = Ctx;
    type Params = Nothing;
    type Output = Value;
    const NAME: &'static str = "inner";
}

/// Registers `S`, then sends a notice and fails, both holding it.
fn leak(_c: &Ctx, _p: Nothing, o: CallOptions) -> BoxFuture<'_, anyhow::Result<Value>> {
    Box::pin(async move {
        o.redact(S.as_bytes());
        o.notices.notice(format!("noticed {S}"));
        anyhow::bail!("failed {S}")
    })
}

/// A registry inside the outer op, which calls it with a clone of its options.
static INNER: std::sync::LazyLock<Registry<Ctx>> = std::sync::LazyLock::new(|| {
    let mut r = Registry::<Ctx>::new();
    r.push_op(
        OpSpec::<Nothing, Value>::new("inner", "Registers and fails."),
        |_c: &Ctx, _p: Nothing, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
            Box::pin(async move {
                o.redact(S.as_bytes());
                anyhow::bail!("inner failed {S}")
            })
        },
    );
    r
});

fn leak_registry() -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    r.push_op(OpSpec::<Nothing, Value>::new("leak", "Leaks."), leak);
    r.push_stream(
        OpSpec::<Nothing, Value>::new("stream_leak", "Leaks in an item."),
        |_c: &Ctx, _p: Nothing, _cur, o: CallOptions| {
            Box::pin(async move {
                Ok(Box::pin(futures_util::stream::once(async move {
                    o.redact(S.as_bytes());
                    Err(anyhow::anyhow!("item failed {S}"))
                }))
                    as futures_util::stream::BoxStream<
                        '_,
                        anyhow::Result<Value>,
                    >)
            })
        },
    );
    r.push_op(
        OpSpec::<Nothing, Value>::new("clone_first", "Clones its options, then registers."),
        |_c: &Ctx, _p: Nothing, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
            Box::pin(async move {
                let helper = o.clone();
                o.redact(S.as_bytes());
                helper.notices.notice(format!("helper noticed {S}"));
                anyhow::bail!("failed {S}")
            })
        },
    );
    r.push_op(
        OpSpec::<Nothing, Value>::new("usage_leak", "Fails with a usage error."),
        |_c: &Ctx, _p: Nothing, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
            Box::pin(async move {
                o.redact(S.as_bytes());
                Err(usage(UsageError::InvalidField {
                    name: "x".into(),
                    reason: format!("bad {S}"),
                }))
            })
        },
    );
    r.push_op(
        OpSpec::<Nothing, Value>::new("outer", "Calls the inner op."),
        |c: &Ctx, _p: Nothing, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
            Box::pin(async move {
                let inner = INNER
                    .call_typed::<InnerOp>(c, Nothing {}, o.clone())
                    .await
                    .unwrap_err();
                o.notices.notice(format!("outer noticed {S}"));
                Err(anyhow::anyhow!("outer wrapped: {}", inner.error()))
            })
        },
    );
    r.push_op(
        OpSpec::<KeyFile, Value>::new("key_echo", "Fails with its key."),
        |_c: &Ctx, p: KeyFile, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
            Box::pin(
                async move { anyhow::bail!("bad key {}", String::from_utf8_lossy(p.key.bytes())) },
            )
        },
    );
    r
}

fn ctx() -> Ctx {
    Ctx {
        token: String::new(),
    }
}

/// A notice sink that keeps the text of what it is sent.
#[derive(Default)]
struct Sink(Mutex<Vec<String>>);

impl NoticeSink for Sink {
    fn send(&self, notice: Notice) {
        let text = match notice {
            Notice::Text(text) => text,
            Notice::Progress { label, .. } => label,
        };
        self.0.lock().unwrap().push(text);
    }
}

fn with_sink() -> (CallOptions, Arc<Sink>) {
    let sink = Arc::new(Sink::default());
    (
        CallOptions::default().with_notices(Notices::new(sink.clone())),
        sink,
    )
}

/// The CLI ran `args`: no stderr line and no hook saw `S`.
fn assert_cli_clean(ran: &Ran, seen: &Seen) {
    assert_hidden(ran, seen, S);
    for line in seen.usage.lock().unwrap().iter() {
        assert!(!line.contains(S), "on_usage_error saw {line}");
    }
}

#[test]
fn cli_error_line_notice_and_outcome_lack_secret() {
    let seen = Arc::new(Seen::default());
    let ran = run_on(leak_registry(), &["leak"], b"", &seen);
    assert_eq!(ran.code, 70, "{}", ran.err);
    assert_cli_clean(&ran, &seen);
    let lines: Vec<&str> = ran.err.lines().collect();
    assert_eq!(lines.len(), 2, "{}", ran.err);
    assert_eq!(lines[0], r#"{"notice":"noticed <secret>"}"#);
    assert!(lines[1].contains("failed <secret>"), "{}", lines[1]);
    let post_run = seen.post_run.lock().unwrap();
    assert!(
        post_run.iter().any(|m| m == "failed <secret>"),
        "{post_run:?}"
    );
}

#[test]
fn hook_on_error_redact_lacks_secret() {
    let seen = Arc::new(Seen::default());
    let ran = run_on(leak_registry(), &["leak"], b"", &seen);
    assert_cli_clean(&ran, &seen);
    assert_eq!(
        seen.on_error.lock().unwrap().as_slice(),
        ["failed <secret>"]
    );
}

#[tokio::test]
async fn call_failure_display_and_redacted_message_lack_secret() {
    let r = leak_registry();
    let (opts, sink) = with_sink();
    let failure = r.call("leak", json!({}), &ctx(), opts).await.unwrap_err();
    assert_eq!(failure.redacted_message(), "failed <secret>");
    assert_eq!(failure.to_string(), "failed <secret>");
    assert!(!format!("{failure:?}").contains(S));
    assert!(!format!("{:#}", failure.into_coded()).contains(S));
    assert_eq!(sink.0.lock().unwrap().as_slice(), ["noticed <secret>"]);
}

#[tokio::test]
async fn call_tool_and_tool_error_lack_secret() {
    let r = leak_registry();
    let failure = op_registry::mcp::call_tool(
        &r,
        &ctx(),
        Audience::Agent,
        "leak",
        json!({}),
        CallOptions::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.to_string(), "failed <secret>");
    let tool = op_registry::mcp::tool_error(&failure, "test", "fallback");
    assert!(!tool.message.contains(S), "{}", tool.message);
}

#[tokio::test]
async fn call_typed_failure_lacks_secret() {
    let r = leak_registry();
    let failure = r
        .call_typed::<LeakOp>(&ctx(), Nothing {}, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(failure.to_string(), "failed <secret>");
    assert_eq!(failure.redacted_message(), "failed <secret>");
}

#[tokio::test]
async fn stream_item_error_lacks_secret() {
    use futures_util::StreamExt;
    let r = leak_registry();
    let items: Vec<_> = r
        .call_stream(
            "stream_leak",
            json!({}),
            &ctx(),
            CallOptions::default(),
            None,
        )
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(
        items[0].as_ref().unwrap_err().to_string(),
        "item failed <secret>"
    );
    let items: Vec<_> = r
        .call_stream_typed::<StreamLeakOp>(&ctx(), Nothing {}, CallOptions::default(), None)
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(
        items[0].as_ref().unwrap_err().to_string(),
        "item failed <secret>"
    );
}

fn mapper(err: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!("mapped {S}: {err}")
}

#[test]
fn mapped_error_is_redacted() {
    let seen = Arc::new(Seen::default());
    let mut r = leak_registry();
    r.set_error_mapper(mapper);
    let ran = run_on(r, &["leak"], b"", &seen);
    assert_cli_clean(&ran, &seen);
    assert!(
        ran.err.contains("mapped <secret>: failed <secret>"),
        "{}",
        ran.err
    );
    assert_eq!(
        seen.on_error.lock().unwrap().as_slice(),
        ["mapped <secret>: failed <secret>"]
    );
}

#[test]
fn stream_item_error_on_cli_is_redacted() {
    let seen = Arc::new(Seen::default());
    let ran = run_on(leak_registry(), &["stream_leak"], b"", &seen);
    assert_eq!(ran.code, 70, "{}", ran.err);
    assert_cli_clean(&ran, &seen);
    assert!(ran.err.contains("item failed <secret>"), "{}", ran.err);
    let mut r = leak_registry();
    r.set_error_mapper(mapper);
    let ran = run_on(r, &["stream_leak"], b"", &seen);
    assert_cli_clean(&ran, &seen);
    assert!(
        ran.err.contains("mapped <secret>: item failed <secret>"),
        "{}",
        ran.err
    );
}

#[tokio::test]
async fn typed_caller_sink_notice_is_redacted() {
    let r = leak_registry();
    let (opts, sink) = with_sink();
    r.call_typed::<LeakOp>(&ctx(), Nothing {}, opts)
        .await
        .unwrap_err();
    assert_eq!(sink.0.lock().unwrap().as_slice(), ["noticed <secret>"]);
}

#[test]
fn options_clone_shares_redactor() {
    let seen = Arc::new(Seen::default());
    let ran = run_on(leak_registry(), &["clone_first"], b"", &seen);
    assert_cli_clean(&ran, &seen);
    assert!(ran.err.contains("helper noticed <secret>"), "{}", ran.err);
}

#[tokio::test]
async fn options_cloned_before_registration_share_it() {
    let r = leak_registry();
    let base = CallOptions::default();
    let copy = base.clone();
    base.redact(S.as_bytes());
    let sink2 = Arc::new(Sink::default());
    let copy = copy.with_notices(Notices::new(sink2.clone()));
    let failure = r
        .call_typed::<LeakOp>(&ctx(), Nothing {}, copy)
        .await
        .unwrap_err();
    assert_eq!(failure.to_string(), "failed <secret>");
    assert_eq!(sink2.0.lock().unwrap().as_slice(), ["noticed <secret>"]);
}

#[test]
fn op_usage_error_hook_message_redacted() {
    let seen = Arc::new(Seen::default());
    let ran = run_on(leak_registry(), &["usage_leak"], b"", &seen);
    assert_eq!(ran.code, 2, "{}", ran.err);
    assert_cli_clean(&ran, &seen);
    assert_eq!(
        seen.usage.lock().unwrap().as_slice(),
        ["invalid field x: bad <secret>"]
    );
}

#[test]
fn nested_call_secret_redacted_in_outer_error() {
    let seen = Arc::new(Seen::default());
    let ran = run_on(leak_registry(), &["outer"], b"", &seen);
    assert_eq!(ran.code, 70, "{}", ran.err);
    assert_cli_clean(&ran, &seen);
    let lines: Vec<&str> = ran.err.lines().collect();
    assert_eq!(lines[0], r#"{"notice":"outer noticed <secret>"}"#);
    assert!(
        lines[1].contains("outer wrapped: inner failed <secret>"),
        "{}",
        lines[1]
    );
    let post_run = seen.post_run.lock().unwrap();
    assert!(
        post_run
            .iter()
            .any(|m| m == "outer wrapped: inner failed <secret>"),
        "{post_run:?}"
    );
}

#[test]
fn clone_before_add_sees_it_detached_does_not() {
    let r = op_registry::Redactor::new();
    let shared = r.clone();
    let detached = r.detached();
    r.add(S.as_bytes());
    assert_eq!(shared.redact(S), "<secret>");
    assert_eq!(detached.redact(S), S);
    assert!(detached.is_empty());
    // A detached copy keeps what was there and then goes its own way.
    let copy = r.detached();
    assert_eq!(copy.redact(S), "<secret>");
    r.add(b"later-value");
    assert_eq!(copy.redact("later-value"), "later-value");
    assert_eq!(shared.redact("later-value"), "<secret>");
}

// SEC-3: regression tests, green before the shared handle.

#[test]
fn secret_file_with_trailing_newline_redacted() {
    let seen = Arc::new(Seen::default());
    let ran = run_on(
        leak_registry(),
        &["key_echo", "--key", "-"],
        b"sk-abc123\n",
        &seen,
    );
    assert_eq!(ran.code, 70, "{}", ran.err);
    assert_hidden(&ran, &seen, "sk-abc123");
    assert!(ran.err.contains("bad key <secret>"), "{}", ran.err);
}

#[test]
fn secret_with_blank_lines_redacted() {
    let seen = Arc::new(Seen::default());
    let ran = run_on(
        leak_registry(),
        &["key_echo", "--key", "-"],
        b"\n\nsk-abc123\n\n",
        &seen,
    );
    assert_eq!(ran.code, 70, "{}", ran.err);
    assert_hidden(&ran, &seen, "sk-abc123");
    assert!(ran.err.contains("<secret>"), "{}", ran.err);
}

#[tokio::test]
async fn call_tool_pre_call_failure_uses_options_handle() {
    let r = leak_registry();
    let opts = CallOptions::default();
    opts.redact(b"no_such_op_s3cr3t");
    let failure = op_registry::mcp::call_tool(
        &r,
        &ctx(),
        Audience::Agent,
        "no_such_op_s3cr3t",
        json!({}),
        opts,
    )
    .await
    .unwrap_err();
    assert!(
        !failure.to_string().contains("no_such_op_s3cr3t"),
        "{failure}"
    );
    assert!(!failure.redacted_message().contains("no_such_op_s3cr3t"));
}
