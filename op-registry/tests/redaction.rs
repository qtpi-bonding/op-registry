//! Redaction on every path: secret globals, `Registry::call` failures, the
//! hooks' views of an error, JSON decode errors and `Parsed`'s `Debug`.
#![cfg(feature = "secrets")]

use futures_util::future::BoxFuture;
use op_registry::cli::{self, Io, Outcome, RunHooks};
use op_registry::{
    CallHook, CallInfo, CallOptions, CliContext, OpGlobals, OpParams, OpSpec, ParseCx, Registry,
    SecretFile,
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
}

impl RunHooks for Seen {
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
    let mut r = registry();
    r.set_call_hook(seen.clone());
    let (io, cap) = Io::memory(b"", false, false);
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
