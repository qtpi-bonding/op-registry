//! The CLI runner: output, exits, hook order, streams, secrets and pipes.

mod common;

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use op_registry::cli::{self, Io, NoHooks, Outcome, RunHooks, Selected, UsageFailure};
use op_registry::{
    CallHook, CallInfo, CallOptions, Caller, CliContext, OpGlobals, OpParams, OpSpec, Page,
    PageRequest, Registry, SecretFile,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::sync::{Arc, Mutex};

/// Process settings of the test binary.
#[derive(Debug, OpGlobals)]
struct G {
    /// Makes `from_globals` fail.
    fail_context: bool,
    /// The data directory.
    #[op(env = "OPREG_T23_GAIT_DIR")]
    #[allow(dead_code)]
    gait_dir: Option<String>,
}

struct Ctx;
impl op_registry::Resolve for Ctx {}

impl Ctx {
    #[allow(dead_code)]
    fn connect(&self) {
        panic!("a local op must not connect");
    }
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "runner.test")]
enum Down {
    #[error("the backend is down")]
    #[coded(code = "down", class = Infrastructure)]
    Backend,
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "runner.test")]
enum Slow {
    #[error("the wait timed out")]
    #[coded(code = "wait_timeout", class = Transient)]
    Timeout,
}

impl CliContext for Ctx {
    type Globals = G;
    fn from_globals(globals: &G) -> anyhow::Result<Self> {
        if globals.fail_context {
            return Err(coded_error::into_anyhow(Down::Backend));
        }
        Ok(Ctx)
    }
    fn caller(&self) -> Option<Caller> {
        Some(Caller("runner-test".to_owned()))
    }
}

#[derive(Clone, OpParams)]
struct Nothing {}

#[derive(Clone, OpParams)]
#[allow(dead_code)]
struct Filter {
    /// Only this tag.
    tag: Option<String>,
}

#[derive(Clone, OpParams)]
struct Vault {
    /// The token.
    #[op(secret)]
    token: Option<String>,
    /// The key file.
    #[op(secret)]
    key: Option<SecretFile>,
    /// Make the op fail with the token in its message.
    leak: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
struct Out {
    ok: bool,
    who: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Item {
    n: u32,
}

fn unary<P: OpParams + Send + 'static>(
    r: &mut Registry<Ctx>,
    spec: OpSpec<P, Out>,
    f: fn(&CallOptions) -> anyhow::Result<Out>,
) {
    r.push_op(
        spec,
        move |_c: &Ctx, _p: P, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async move { f(&o) })
        },
    );
}

fn ok_out(o: &CallOptions) -> anyhow::Result<Out> {
    Ok(Out {
        ok: true,
        who: o.caller.as_ref().map(|c| c.0.clone()),
    })
}

fn items(r: &mut Registry<Ctx>, spec: OpSpec<Nothing, Item>, fail_at: Option<u32>) {
    r.push_stream(
        spec,
        move |_c: &Ctx,
              _p: Nothing,
              _cur,
              _o: CallOptions|
              -> BoxFuture<'_, anyhow::Result<BoxStream<'_, anyhow::Result<Item>>>> {
            Box::pin(async move {
                let list: Vec<anyhow::Result<Item>> = (1..=3)
                    .map(|n| {
                        if Some(n) == fail_at {
                            Err(coded_error::into_anyhow(Down::Backend))
                        } else {
                            Ok(Item { n })
                        }
                    })
                    .take_while({
                        let mut stop = false;
                        move |r| {
                            let go = !stop;
                            stop = r.is_err();
                            go
                        }
                    })
                    .collect();
                Ok(Box::pin(futures_util::stream::iter(list)) as BoxStream<'_, _>)
            })
        },
    );
}

fn reg() -> Registry<Ctx> {
    let mut r = Registry::new();
    r.set_pillar("fix");
    r.set_bin("app", "1.2.3", "A test app.");
    for (g, s) in [
        ("concept", "Concepts."),
        ("report", "Reports."),
        ("feed", "Feeds."),
        ("vault", "Vault."),
        ("task", "Tasks."),
    ] {
        r.group(g, s);
    }
    unary(&mut r, OpSpec::<Nothing, Out>::new("ping", "Ping."), ok_out);
    let mut init = OpSpec::<Nothing, Out>::new("init", "Initialises.");
    init.needs_globals = vec!["gait_dir"];
    unary(&mut r, init, ok_out);
    unary(
        &mut r,
        OpSpec::<Nothing, Out>::new("task fail", "Fails."),
        |_| {
            Err(coded_error::into_anyhow(
                op_registry::CallError::WrongShape {
                    name: "x".to_owned(),
                },
            ))
        },
    );
    let mut four = OpSpec::<Nothing, Out>::new("task four", "Exits four.");
    four.exits = vec![(4, "four")];
    four.ok_exit = Some(|_| 4);
    unary(&mut r, four, ok_out);
    let mut show = OpSpec::<Nothing, Out>::new("report show", "Shows.");
    show.render_text = Some(|o| format!("ok={}", o.ok));
    unary(&mut r, show, ok_out);
    r.push_local(
        OpSpec::<Nothing, Out>::new("task local", "Local."),
        |_c: &Ctx, _p: Nothing, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async move { ok_out(&o) })
        },
    );
    r.push_op(
        OpSpec::<Vault, Out>::new("vault put", "Puts."),
        |_c: &Ctx, p: Vault, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async move {
                if p.leak {
                    anyhow::bail!("rejected token {}", p.token.unwrap_or_default());
                }
                Ok(Out {
                    ok: true,
                    who: None,
                })
            })
        },
    );
    unary(
        &mut r,
        OpSpec::<Nothing, Out>::new("task usage", "Returns a usage error."),
        |_| {
            Err(op_registry::usage(op_registry::UsageError::InvalidField {
                name: "x".to_owned(),
                reason: "the op says so".to_owned(),
            }))
        },
    );
    items(&mut r, OpSpec::new("feed watch", "Watch."), None);
    items(&mut r, OpSpec::new("feed broken", "Breaks."), Some(2));
    r.push_list(
        OpSpec::<Filter, Item>::new("concept list", "List."),
        |_c: &Ctx, _p: Filter, req: PageRequest| -> BoxFuture<'_, anyhow::Result<Page<Item>>> {
            Box::pin(async move {
                let start: u32 = req.cursor.as_deref().map_or(0, |c| c.parse().unwrap());
                let end = (start + req.limit).min(5);
                Ok(Page {
                    items: (start..end).map(|n| Item { n }).collect(),
                    next_cursor: (end < 5).then(|| end.to_string()),
                })
            })
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

struct Ran {
    code: u8,
    out: String,
    err: String,
}

impl Ran {
    fn out_json(&self) -> Value {
        serde_json::from_str(&self.out).unwrap_or_else(|e| panic!("{e}: {:?}", self.out))
    }
    fn last_err(&self) -> Value {
        let line = self.err.lines().last().expect("stderr has a line");
        serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
    }
}

fn run_with(
    args: &[&str],
    stdin: &[u8],
    out_tty: bool,
    err_tty: bool,
    hooks: &dyn RunHooks,
) -> Ran {
    let (io, cap) = Io::memory(stdin, out_tty, err_tty);
    let code = futures_executor::block_on(cli::run_with_io(&reg(), argv(args), hooks, io));
    Ran {
        code,
        out: cap.stdout(),
        err: cap.stderr(),
    }
}

fn run(args: &[&str]) -> Ran {
    run_with(args, b"", false, false, &NoHooks)
}

#[test]
fn prints_one_json_value_compact_when_piped() {
    let r = run(&["ping"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.out, "{\"ok\":true,\"who\":\"runner-test\"}\n");
    assert_eq!(r.err, "");
}

#[test]
fn prints_pretty_when_stdout_tty() {
    let r = run_with(&["ping"], b"", true, false, &NoHooks);
    assert!(r.out.starts_with("{\n  \"ok\": true"), "{}", r.out);
    let r = run_with(&["ping"], b"", false, true, &NoHooks);
    assert!(r.out.starts_with("{\n  \"ok\""), "{}", r.out);
}

#[test]
fn pretty_flag_forces() {
    let r = run(&["ping", "--pretty"]);
    assert!(r.out.starts_with("{\n  \"ok\": true"), "{}", r.out);
    let r = run_with(&["ping", "--compact"], b"", true, true, &NoHooks);
    assert_eq!(r.out.lines().count(), 1, "{}", r.out);
}

#[test]
fn text_flag_prints_render_text() {
    let r = run(&["report", "show", "--text"]);
    assert_eq!(r.out, "ok=true\n");
    assert_eq!(r.code, 0);
    assert_eq!(run(&["report", "show"]).out_json()["ok"], json!(true));
}

#[derive(Default)]
struct Rec {
    exits: Mutex<Vec<u8>>,
    log: Mutex<Vec<&'static str>>,
    stderr: Mutex<Option<op_registry::cli::SharedWriter>>,
}

impl Rec {
    fn log(&self) -> Vec<&'static str> {
        self.log.lock().unwrap().clone()
    }
}

impl RunHooks for Rec {
    fn pre_parse(&self, _argv: &[OsString]) {
        self.log.lock().unwrap().push("pre_parse");
    }
    fn on_usage_error(&self, _f: &UsageFailure) {
        self.log.lock().unwrap().push("on_usage_error");
    }
    fn post_run(&self, outcome: &Outcome) {
        self.exits.lock().unwrap().push(outcome.exit_code());
        let tag = match outcome {
            Outcome::Ok { .. } => "post_run_ok",
            Outcome::Err { .. } => "post_run_err",
            _ => "post_run_other",
        };
        self.log.lock().unwrap().push(tag);
        if let Some(w) = self.stderr.lock().unwrap().as_mut() {
            use std::io::Write;
            writeln!(w, "log text").unwrap();
        }
    }
}

#[test]
fn usage_error_exits_2_and_calls_on_usage_error() {
    let rec = Rec::default();
    let r = run_with(&["nonesuch"], b"", false, false, &rec);
    assert_eq!(r.code, 2);
    assert_eq!(rec.log(), ["pre_parse", "on_usage_error", "post_run_err"]);
    assert_eq!(r.out, "");
    let e = r.last_err();
    assert_eq!(e["error"]["code"], "op_registry.cli.unknown_command");
    assert_eq!(e["error"]["class"], "logical");
}

#[test]
fn op_usage_error_reaches_hook_once_and_exits_2() {
    let rec = Rec::default();
    let r = run_with(&["task", "usage"], b"", false, false, &rec);
    assert_eq!(r.code, 2);
    assert_eq!(rec.log(), ["pre_parse", "on_usage_error", "post_run_err"]);
    assert_eq!(r.last_err()["error"]["code"], "op_registry.usage.invalid");
    // A failure that is not exit 2 does not reach the hook.
    let rec = Rec::default();
    let r = run_with(&["task", "fail"], b"", false, false, &rec);
    assert_ne!(r.code, 2);
    assert_eq!(rec.log(), ["pre_parse", "post_run_err"]);
}

#[test]
fn hook_order_pre_parse_post_run_then_error_line() {
    let rec = Rec::default();
    let (io, cap) = Io::memory(b"", false, false);
    *rec.stderr.lock().unwrap() = Some(io.stderr.clone());
    let code =
        futures_executor::block_on(cli::run_with_io(&reg(), argv(&["task", "fail"]), &rec, io));
    assert_eq!(code, 65);
    assert_eq!(rec.log(), ["pre_parse", "post_run_err"]);
    let err = cap.stderr();
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines[0], "log text");
    let last: Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    assert!(last["error"]["code"].is_string());
    assert_eq!(lines.len(), 2);
}

#[test]
fn post_run_precedes_the_printed_value() {
    let rec = Rec::default();
    let (io, cap) = Io::memory(b"", false, false);
    *rec.stderr.lock().unwrap() = Some(io.stderr.clone());
    let code = futures_executor::block_on(cli::run_with_io(&reg(), argv(&["ping"]), &rec, io));
    assert_eq!(code, 0);
    assert_eq!(rec.log(), ["pre_parse", "post_run_ok"]);
    assert_eq!(cap.stderr(), "log text\n");
}

#[test]
fn logical_error_exits_65_with_error_last_line() {
    let r = run(&["task", "fail"]);
    assert_eq!(r.code, 65);
    assert_eq!(r.out, "");
    let e = r.last_err();
    assert_eq!(e["error"]["code"], "op_registry.call.wrong_shape");
    assert_eq!(e["error"]["class"], "logical");
}

#[test]
fn declared_ok_exit_4() {
    let r = run(&["task", "four"]);
    assert_eq!(r.code, 4);
    assert_eq!(r.out_json()["ok"], json!(true));
    assert_eq!(r.err, "");
}

#[test]
fn stream_prints_ndjson_lines() {
    let r = run(&["feed", "watch"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.out, "{\"n\":1}\n{\"n\":2}\n{\"n\":3}\n");
    // A terminal still gets one compact line per item.
    let r = run_with(&["feed", "watch"], b"", true, true, &NoHooks);
    assert_eq!(r.out, "{\"n\":1}\n{\"n\":2}\n{\"n\":3}\n");
}

#[test]
fn stream_mid_error_ends_with_error_line_and_class_exit() {
    let r = run(&["feed", "broken"]);
    assert_eq!(r.code, 69);
    assert_eq!(r.out, "{\"n\":1}\n");
    let e = r.last_err();
    assert_eq!(e["error"]["code"], "runner.test.down");
    assert_eq!(e["error"]["class"], "infrastructure");
}

#[test]
fn follow_all_list_prints_items_object() {
    let r = run(&["concept", "list"]);
    assert_eq!(r.code, 0);
    assert_eq!(
        r.out_json(),
        json!({"items": [{"n": 0}, {"n": 1}, {"n": 2}, {"n": 3}, {"n": 4}]})
    );
}

#[test]
fn limit_prints_items_object() {
    let r = run(&["concept", "list", "--limit", "2"]);
    assert_eq!(
        r.out_json(),
        json!({"items": [{"n": 0}, {"n": 1}], "nextCursor": "2"})
    );
    let r = run(&["concept", "list", "--cursor", "4"]);
    assert_eq!(
        r.out_json(),
        json!({"items": [{"n": 4}], "nextCursor": null})
    );
}

struct TraceHook(Mutex<Vec<op_registry::TraceParent>>);
impl CallHook for TraceHook {
    fn around<'a>(&'a self, info: &CallInfo, call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()> {
        self.0.lock().unwrap().push(info.trace.clone());
        call
    }
}

#[test]
fn traceparent_env_reaches_call_info() {
    let hook = Arc::new(TraceHook(Mutex::new(Vec::new())));
    let mut registry = reg();
    registry.set_call_hook(hook.clone());
    let keys = ["TRACEPARENT", "TRACESTATE", "BAGGAGE"];
    let saved: Vec<Option<String>> = keys.iter().map(|k| std::env::var(k).ok()).collect();
    std::env::set_var("TRACEPARENT", "00-aaaa-bbbb-01");
    std::env::set_var("TRACESTATE", "k=v");
    std::env::remove_var("BAGGAGE");
    let (io, _cap) = Io::memory(b"", false, false);
    let code =
        futures_executor::block_on(cli::run_with_io(&registry, argv(&["ping"]), &NoHooks, io));
    for (k, v) in keys.iter().zip(saved) {
        match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        }
    }
    assert_eq!(code, 0);
    let seen = hook.0.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].traceparent.as_deref(), Some("00-aaaa-bbbb-01"));
    assert_eq!(seen[0].tracestate.as_deref(), Some("k=v"));
    assert_eq!(seen[0].baggage, None);
}

fn notice_registry() -> Registry<Ctx> {
    let mut r = reg();
    r.push_op(
        OpSpec::<Nothing, Out>::new("task note", "Notes."),
        |_c: &Ctx, _p: Nothing, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async move {
                o.notices.notice("hello");
                o.notices.progress(1, Some(2), "files");
                ok_out(&o)
            })
        },
    );
    r
}

#[test]
fn notice_lines_on_piped_stderr_are_json() {
    let (io, cap) = Io::memory(b"", false, false);
    let code = futures_executor::block_on(cli::run_with_io(
        &notice_registry(),
        argv(&["task", "note"]),
        &NoHooks,
        io,
    ));
    assert_eq!(code, 0);
    let lines: Vec<Value> = cap
        .stderr()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        lines,
        vec![
            json!({"notice": "hello"}),
            json!({"progress": {"done": 1, "total": 2, "label": "files"}})
        ]
    );
}

#[test]
fn notice_lines_on_tty_stderr_are_text() {
    let (io, cap) = Io::memory(b"", false, true);
    futures_executor::block_on(cli::run_with_io(
        &notice_registry(),
        argv(&["task", "note"]),
        &NoHooks,
        io,
    ));
    assert_eq!(cap.stderr(), "notice: hello\nprogress: 1/2 files\n");
}

#[test]
fn context_from_globals_error_exits_by_class() {
    let rec = Rec::default();
    let r = run_with(&["ping", "--fail-context"], b"", false, false, &rec);
    assert_eq!(r.code, 69);
    assert_eq!(rec.log(), ["pre_parse", "post_run_err"]);
    assert_eq!(r.last_err()["error"]["code"], "runner.test.down");
}

#[test]
fn local_op_never_calls_connect() {
    let r = run(&["task", "local"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.out_json()["ok"], json!(true));
}

#[test]
fn secret_never_in_error_line() {
    // An op error that echoes the token.
    let r = run(&["vault", "put", "--token", "s3cretvalue", "--leak"]);
    assert_eq!(r.code, 70);
    assert!(!r.err.contains("s3cretvalue"), "{}", r.err);
    assert!(r.last_err()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("<secret>"));
    // A decode error that echoes the secret file's path.
    let r = run(&[
        "vault",
        "put",
        "--key",
        "/nonexistent/hidden-secret-path.key",
    ]);
    assert_eq!(r.code, 2);
    assert!(!r.err.contains("hidden-secret-path"), "{}", r.err);
    assert!(r.last_err()["error"]["message"].is_string());
    // The same with the equals form, and a clap-level error.
    let r = run(&["vault", "put", "--key=/nonexistent/hidden-secret-path.key"]);
    assert!(!r.err.contains("hidden-secret-path"), "{}", r.err);
    let r = run(&[
        "vault",
        "put",
        "--token",
        "s3cretvalue",
        "--bogus",
        "s3cretvalue",
    ]);
    assert_eq!(r.code, 2);
    assert!(!r.err.contains("s3cretvalue"), "{}", r.err);
}

#[test]
fn stdin_reaches_dash_inputs() {
    let r = run_with(
        &["vault", "put", "--key", "-"],
        b"from-stdin-key",
        false,
        false,
        &NoHooks,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out_json()["ok"], json!(true));
}

/// A writer that is a closed pipe.
struct Closed;
impl std::io::Write for Closed {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

#[test]
fn broken_pipe_on_stdout_exits_0_quietly() {
    for args in [&["ping"][..], &["feed", "watch"], &["concept", "list"]] {
        let (mut io, cap) = Io::memory(b"", false, false);
        io.stdout = Box::new(Closed);
        let code = futures_executor::block_on(cli::run_with_io(&reg(), argv(args), &NoHooks, io));
        assert_eq!(code, 0, "{args:?}");
        assert_eq!(cap.stderr(), "", "{args:?}");
    }
}

#[test]
fn help_exits_0() {
    let rec = Rec::default();
    let r = run_with(&["--help"], b"", false, false, &rec);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("A test app."), "{}", r.out);
    assert_eq!(r.err, "");
    let r = run(&["--version"]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("1.2.3"), "{}", r.out);
    // Help does not build the context, so a failing global does not matter.
    assert_eq!(run(&["ping", "--help", "--fail-context"]).code, 0);
}

#[test]
fn caller_from_context_reaches_op() {
    assert_eq!(run(&["ping"]).out_json()["who"], "runner-test");
}

/// A writer that keeps each `write` call apart, to see whether a line was
/// written in one piece.
#[derive(Clone, Default)]
struct Writes(Arc<Mutex<Vec<Vec<u8>>>>);

impl std::io::Write for Writes {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().push(buf.to_vec());
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn each_stderr_and_stdout_line_is_one_write() {
    for (args, out_tty, err_tty) in [
        (&["task", "note"][..], false, false),
        (&["task", "fail"][..], false, false),
        (&["task", "fail"][..], false, true),
        (&["feed", "broken"][..], false, false),
    ] {
        let (stdout, stderr) = (Writes::default(), Writes::default());
        let (mut io, _) = Io::memory(b"", out_tty, err_tty);
        io.stdout = Box::new(stdout.clone());
        io.stderr = cli::SharedWriter::new(Box::new(stderr.clone()));
        io.stdin = None;
        futures_executor::block_on(cli::run_with_io(
            &notice_registry(),
            argv(args),
            &NoHooks,
            io,
        ));
        for (stream, writes) in [("stdout", &stdout), ("stderr", &stderr)] {
            let writes = writes.0.lock().unwrap();
            assert!(
                !writes.is_empty() || stream == "stdout",
                "{args:?}: no {stream}"
            );
            for write in writes.iter() {
                let text = String::from_utf8_lossy(write);
                assert!(
                    text.ends_with('\n') && text.len() > 1,
                    "{args:?}: {stream} got a partial line {text:?}"
                );
            }
        }
    }
}

fn coded_err_extra(_: &G, _: &clap::ArgMatches) -> BoxFuture<'static, anyhow::Result<u8>> {
    Box::pin(async { Err(coded_error::into_anyhow(Down::Backend)) })
}

/// `reg()` plus extras and a flags-only fallback.
fn reg_ext() -> Registry<Ctx> {
    let mut r = reg();
    r.push_extra(clap::Command::new("boom"), coded_err_extra);
    r.push_extra(
        clap::Command::new("five"),
        |_: &G, _: &clap::ArgMatches| -> BoxFuture<'_, anyhow::Result<u8>> {
            Box::pin(async { Ok(5) })
        },
    );
    r.push_extra(
        clap::Command::new("misuse"),
        |_: &G, _: &clap::ArgMatches| -> BoxFuture<'_, anyhow::Result<u8>> {
            Box::pin(async {
                Err(op_registry::usage(op_registry::UsageError::InvalidField {
                    name: "x".to_owned(),
                    reason: "the extra says so".to_owned(),
                }))
            })
        },
    );
    r.fallback_flags_only(
        "fix",
        "Fix things.",
        |_: Vec<OsString>| -> BoxFuture<'static, anyhow::Result<u8>> { Box::pin(async { Ok(6) }) },
    );
    r
}

fn run_ext(args: &[&str], hooks: &dyn RunHooks) -> Ran {
    let (io, cap) = Io::memory(b"", false, false);
    let code = futures_executor::block_on(cli::run_with_io(&reg_ext(), argv(args), hooks, io));
    Ran {
        code,
        out: cap.stdout(),
        err: cap.stderr(),
    }
}

#[test]
fn extra_err_writes_error_line_and_exits_by_class() {
    let rec = Rec::default();
    let r = run_ext(&["boom"], &rec);
    assert_eq!(r.code, 69);
    assert_eq!(r.out, "");
    assert_eq!(r.last_err()["error"]["code"], "runner.test.down");
    assert_eq!(rec.log(), ["pre_parse", "post_run_err"]);
    // A usage code exits 2 and reaches on_usage_error once.
    let rec = Rec::default();
    let r = run_ext(&["misuse"], &rec);
    assert_eq!(r.code, 2);
    assert_eq!(rec.log(), ["pre_parse", "on_usage_error", "post_run_err"]);
    assert_eq!(r.last_err()["error"]["code"], "op_registry.usage.invalid");
}

#[test]
fn extra_ok_exits_n() {
    let rec = Rec::default();
    let r = run_ext(&["five"], &rec);
    assert_eq!(r.code, 5);
    assert_eq!(r.err, "");
    assert_eq!(rec.log(), ["pre_parse", "post_run_ok"]);
}

fn panic_message(f: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let payload = std::panic::catch_unwind(f).expect_err("expected a panic");
    match payload.downcast::<String>() {
        Ok(s) => *s,
        Err(p) => p.downcast::<&str>().map(|s| s.to_string()).unwrap(),
    }
}

fn noop(_: &G, _: &clap::ArgMatches) -> BoxFuture<'static, anyhow::Result<u8>> {
    Box::pin(async { Ok(0) })
}

#[test]
fn extra_with_pretty_flag_panics() {
    for flag in [
        "pretty",
        "compact",
        "text",
        "help",
        "version",
        "fail-context",
    ] {
        let message = panic_message(|| {
            let mut r = reg();
            r.push_extra(
                clap::Command::new("serve").arg(clap::Arg::new("x").long(flag)),
                noop,
            );
        });
        assert!(
            message.contains(&format!("extra `serve`: --{flag} is already a global flag")),
            "{message}"
        );
    }
    // A subcommand's flag counts too.
    let message = panic_message(|| {
        let mut r = reg();
        r.push_extra(
            clap::Command::new("serve")
                .subcommand(clap::Command::new("now").arg(clap::Arg::new("x").long("pretty"))),
            noop,
        );
    });
    assert!(
        message.contains("--pretty is already a global flag"),
        "{message}"
    );
}

#[test]
fn extra_with_name_flag_ok() {
    let mut r = reg();
    r.push_extra(
        clap::Command::new("serve").arg(clap::Arg::new("name").long("name")),
        noop,
    );
    cli::command(&r);
}

#[derive(Default)]
struct Sel(Mutex<Vec<Selected>>);

impl RunHooks for Sel {
    fn on_selected(&self, selected: &Selected) {
        self.0.lock().unwrap().push(selected.clone());
    }
}

#[test]
fn on_selected_once_per_kind() {
    let cases: [(&[&str], Selected); 5] = [
        (
            &["ping"],
            Selected::Op {
                name: "ping".to_owned(),
            },
        ),
        (
            &["five"],
            Selected::Extra {
                name: "five".to_owned(),
            },
        ),
        (
            &["--weird", "x"],
            Selected::Fallback {
                name: "fix",
                args: vec![OsString::from("--weird"), OsString::from("x")],
            },
        ),
        (&["--help"], Selected::Help),
        (&["--version"], Selected::Version),
    ];
    for (args, want) in cases {
        let sel = Sel::default();
        run_ext(args, &sel);
        assert_eq!(*sel.0.lock().unwrap(), [want], "{args:?}");
    }
}

#[test]
fn usage_failure_does_not_call_on_selected() {
    let sel = Sel::default();
    let r = run_ext(&["nonesuch"], &sel);
    assert_eq!(r.code, 2);
    assert!(sel.0.lock().unwrap().is_empty());
}

const NEED_ENV: &str = "OPREG_T23_GAIT_DIR";

static NEED_ENV_LOCK: Mutex<()> = Mutex::new(());

/// Holds the env lock, sets (or clears) the need's variable, and clears it on
/// drop.
struct NeedEnv(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

fn need_env(value: Option<&str>) -> NeedEnv {
    let lock = NEED_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match value {
        Some(v) => std::env::set_var(NEED_ENV, v),
        None => std::env::remove_var(NEED_ENV),
    }
    NeedEnv(lock)
}

impl Drop for NeedEnv {
    fn drop(&mut self) {
        std::env::remove_var(NEED_ENV);
    }
}

#[test]
fn missing_need_exits_2_with_message() {
    let _env = need_env(None);
    let r = run(&["init"]);
    assert_eq!(r.code, 2);
    assert_eq!(r.out, "");
    let e = r.last_err();
    assert_eq!(e["error"]["code"], "op_registry.usage.missing");
    assert_eq!(
        e["error"]["message"],
        "`init` needs --gait-dir (or OPREG_T23_GAIT_DIR)"
    );
}

#[test]
fn env_meets_need() {
    let _env = need_env(Some("/x"));
    let r = run(&["init"]);
    assert_eq!(r.code, 0, "{}", r.err);
}

#[test]
fn empty_env_does_not_meet_need() {
    let _env = need_env(Some(""));
    // With an empty value the arg has no env attached: the source is neither
    // the command line nor the environment.
    let mut matches = cli::command(&reg())
        .try_get_matches_from(argv(&["init"]))
        .unwrap();
    let (_, leaf) = matches.remove_subcommand().unwrap();
    let source = leaf.value_source("gait_dir");
    assert!(
        !matches!(
            source,
            Some(clap::parser::ValueSource::CommandLine)
                | Some(clap::parser::ValueSource::EnvVariable)
        ),
        "{source:?}"
    );
    let r = run(&["init"]);
    assert_eq!(r.code, 2);
    assert_eq!(r.last_err()["error"]["code"], "op_registry.usage.missing");
}

#[test]
fn flag_before_or_after_verb_meets_need() {
    let _env = need_env(None);
    assert_eq!(run(&["--gait-dir", "/x", "init"]).code, 0);
    assert_eq!(run(&["init", "--gait-dir", "/x"]).code, 0);
}

#[test]
fn parse_alone_returns_need_failure() {
    let _env = need_env(None);
    let failure = cli::parse(&reg(), &argv(&["init"])).unwrap_err();
    assert_eq!(failure.code, "op_registry.usage.missing");
    assert_eq!(
        failure.message,
        "`init` needs --gait-dir (or OPREG_T23_GAIT_DIR)"
    );
    assert!(cli::parse(&reg(), &argv(&["init", "--gait-dir", "/x"])).is_ok());
    // `select` goes through `parse`.
    assert!(cli::select(&reg(), &argv(&["init"])).is_err());
}

#[test]
fn help_works_without_need() {
    let _env = need_env(None);
    let r = run(&["init", "--help"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out
            .trim_end()
            .ends_with("Needs: --gait-dir (or OPREG_T23_GAIT_DIR)."),
        "{}",
        r.out
    );
    assert_eq!(run(&["--version"]).code, 0);
}

fn reg_needing(name: &'static str) -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    let mut spec = OpSpec::<Nothing, Out>::new("init", "Initialises.");
    spec.needs_globals = vec![name];
    unary(&mut r, spec, ok_out);
    r
}

#[test]
fn unknown_need_panics_at_command() {
    let r = reg_needing("nope");
    let message = panic_message(std::panic::AssertUnwindSafe(|| {
        cli::command(&r);
    }));
    assert!(message.contains("nope"), "{message}");
    assert!(message.contains("not a global"), "{message}");
}

#[test]
fn switch_need_panics_at_command() {
    let r = reg_needing("fail_context");
    let message = panic_message(std::panic::AssertUnwindSafe(|| {
        cli::command(&r);
    }));
    assert!(
        message.contains("a needed global must take a value"),
        "{message}"
    );
}

#[test]
fn need_reaches_usage_hook() {
    let _env = need_env(None);
    let rec = Rec::default();
    let r = run_with(&["init"], b"", false, false, &rec);
    assert_eq!(r.code, 2);
    assert_eq!(rec.log(), ["pre_parse", "on_usage_error", "post_run_err"]);
}

#[test]
fn list_halves_share_needs() {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    let mut spec = OpSpec::<Filter, Item>::new("things", "Lists.");
    spec.needs_globals = vec!["gait_dir"];
    r.push_list(
        spec,
        |_c: &Ctx, _p: Filter, _r: PageRequest| -> BoxFuture<'_, anyhow::Result<Page<Item>>> {
            Box::pin(async {
                Ok(Page {
                    items: vec![],
                    next_cursor: None,
                })
            })
        },
    );
    for name in ["things", "things_page"] {
        assert_eq!(r.get(name).unwrap().needs_globals, ["gait-dir"], "{name}");
    }
}

/// A registry whose two ops declare `3 => "timed out" on "runner.test.wait_timeout"`.
/// `task wait` returns that code; `task other` returns an infrastructure code.
fn exits_reg() -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    r.set_bin("app", "1.2.3", "A test app.");
    r.group("task", "Tasks.");
    for (path, fail) in [("task wait", true), ("task other", false)] {
        let mut spec = OpSpec::<Nothing, Out>::new(path, "Waits.");
        spec.errors = <Slow as coded_error::CodeCatalog>::codes();
        spec.error_exits = vec![(3, "timed out", "runner.test.wait_timeout")];
        r.push_op(
            spec,
            move |_c: &Ctx, _p: Nothing, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
                Box::pin(async move {
                    Err(if fail {
                        coded_error::into_anyhow(Slow::Timeout)
                    } else {
                        coded_error::into_anyhow(Down::Backend)
                    })
                })
            },
        );
    }
    let mut usage = OpSpec::<Nothing, Out>::new("task usage", "Usage error with an exit.");
    usage.error_exits = vec![(6, "bad usage", "op_registry.usage.invalid")];
    r.push_op(
        usage,
        |_c: &Ctx, _p: Nothing, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async {
                Err(op_registry::usage(op_registry::UsageError::InvalidField {
                    name: "x".to_owned(),
                    reason: "the op says so".to_owned(),
                }))
            })
        },
    );
    r
}

fn run_exits(args: &[&str], hooks: &dyn RunHooks) -> Ran {
    let (io, cap) = Io::memory(b"", false, false);
    let code = futures_executor::block_on(cli::run_with_io(&exits_reg(), argv(args), hooks, io));
    Ran {
        code,
        out: cap.stdout(),
        err: cap.stderr(),
    }
}

#[test]
fn error_exit_code_exits_3_with_error_line() {
    let r = run_exits(&["task", "wait"], &NoHooks);
    assert_eq!(r.code, 3);
    assert_eq!(r.out, "");
    let e = r.last_err();
    assert_eq!(e["error"]["code"], "runner.test.wait_timeout");
    assert_eq!(e["error"]["class"], "transient");
}

#[test]
fn post_run_sees_exit_3() {
    let rec = Rec::default();
    let r = run_exits(&["task", "wait"], &rec);
    assert_eq!(r.code, 3);
    assert_eq!(*rec.exits.lock().unwrap(), vec![3]);
    // Not a usage failure, so the usage hook stays quiet.
    assert_eq!(rec.log(), ["pre_parse", "post_run_err"]);
}

#[test]
fn other_code_exits_by_class() {
    let rec = Rec::default();
    let r = run_exits(&["task", "other"], &rec);
    assert_eq!(r.code, 69);
    assert_eq!(*rec.exits.lock().unwrap(), vec![69]);
    assert_eq!(r.last_err()["error"]["code"], "runner.test.down");
}

#[test]
fn usage_code_with_error_exit_still_fires_usage_hook() {
    let rec = Rec::default();
    let r = run_exits(&["task", "usage"], &rec);
    assert_eq!(r.code, 6);
    assert_eq!(*rec.exits.lock().unwrap(), vec![6]);
    assert_eq!(rec.log(), ["pre_parse", "on_usage_error", "post_run_err"]);
}

#[test]
fn run_code_returns_the_exit_code_as_u8() {
    let code: u8 =
        futures_executor::block_on(cli::run_code(&reg(), argv(&["--version"]), &NoHooks));
    assert_eq!(code, 0);
    let code: u8 = futures_executor::block_on(cli::run_code(&reg(), argv(&["nope"]), &NoHooks));
    assert_eq!(code, 2);
}
