//! Extra commands and the FlagsOnly / FreeText root fallbacks.

use futures_util::future::BoxFuture;
use op_registry::cli::{self, edit_distance, Io, NoHooks};
use op_registry::{CallOptions, CliContext, OpGlobals, OpParams, OpSpec, Registry};
use serde::Serialize;
use std::cell::RefCell;
use std::ffi::OsString;

/// Process settings of the test binary.
#[derive(Debug, OpGlobals)]
struct G {
    /// The endpoint.
    #[op(env = "OPREG_FALLBACK_TEST_ENDPOINT")]
    endpoint: Option<String>,
    /// The region, read from the environment only by `extra_env_fallback`.
    #[op(env = "OPREG_FALLBACK_TEST_REGION")]
    region: Option<String>,
}

struct Ctx;
impl op_registry::Resolve for Ctx {}
impl CliContext for Ctx {
    type Globals = G;
    fn from_globals(_: &G) -> anyhow::Result<Self> {
        Ok(Ctx)
    }
}

#[derive(Clone, OpParams)]
struct Nothing {}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Out {
    ok: bool,
}

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(line: String) {
    LOG.with(|l| l.borrow_mut().push(line));
}

fn logged() -> Vec<String> {
    LOG.with(|l| l.borrow().clone())
}

fn base() -> Registry<Ctx> {
    let mut r = Registry::new();
    r.set_pillar("fix");
    r.set_bin("app", "1.2.3", "A test app.");
    r.group("build", "Builds.");
    r.group("task", "Tasks.");
    for path in ["ping", "build run", "task log", "log"] {
        r.push_op(
            OpSpec::<Nothing, Out>::new(path, "Does it."),
            |_c: &Ctx, _p: Nothing, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
                Box::pin(async { Ok(Out { ok: true }) })
            },
        );
    }
    r
}

fn flags_only(r: &mut Registry<Ctx>) {
    r.fallback_flags_only(
        "fix",
        "Fix things.\nMore text.",
        |args: Vec<OsString>| -> BoxFuture<'static, u8> {
            Box::pin(async move {
                log(format!("flags_only {:?}", strings(&args)));
                7
            })
        },
    );
}

fn free_text(r: &mut Registry<Ctx>) {
    r.fallback_free_text(
        "ask",
        "Ask the agent.\nMore text.",
        &["codebase"],
        |g: &G, args: Vec<OsString>| -> BoxFuture<'_, u8> {
            Box::pin(async move {
                log(format!("free_text {:?} {:?}", g.endpoint, strings(&args)));
                9
            })
        },
    );
}

fn strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

struct Ran {
    code: u8,
    out: String,
    err: String,
}

fn run(r: &Registry<Ctx>, args: &[&str]) -> Ran {
    let argv: Vec<OsString> = std::iter::once("app")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect();
    let (io, cap) = Io::memory(b"", false, false);
    let code = futures_executor::block_on(cli::run_with_io(r, argv, &NoHooks, io));
    Ran {
        code,
        out: cap.stdout(),
        err: cap.stderr(),
    }
}

fn last_err(r: &Ran) -> serde_json::Value {
    let line: serde_json::Value =
        serde_json::from_str(r.err.lines().last().expect("stderr has a line")).unwrap();
    line["error"].clone()
}

fn serve(r: &mut Registry<Ctx>) {
    r.push_extra(
        clap::Command::new("serve").arg(clap::Arg::new("port").long("port")),
        |g: &G, m: &clap::ArgMatches| -> BoxFuture<'_, u8> {
            Box::pin(async move {
                log(format!(
                    "serve {:?} {:?}",
                    g.endpoint,
                    m.get_one::<String>("port")
                ));
                3
            })
        },
    );
}

#[test]
fn extra_gets_globals_and_matches() {
    let mut r = base();
    serve(&mut r);
    let ran = run(&r, &["serve", "--port", "80", "--endpoint", "e1"]);
    assert_eq!(ran.code, 3);
    assert_eq!(logged(), vec![r#"serve Some("e1") Some("80")"#]);
    // A global before the extra works too.
    let ran = run(&r, &["--endpoint", "e2", "serve"]);
    assert_eq!(ran.code, 3);
    assert_eq!(logged()[1], r#"serve Some("e2") None"#);
}

#[test]
fn extra_env_fallback() {
    let mut r = base();
    r.push_extra(
        clap::Command::new("where"),
        |g: &G, _: &clap::ArgMatches| -> BoxFuture<'_, u8> {
            Box::pin(async move {
                log(format!("where {:?}", g.region));
                4
            })
        },
    );
    std::env::set_var("OPREG_FALLBACK_TEST_REGION", "from-env");
    let ran = run(&r, &["where"]);
    std::env::remove_var("OPREG_FALLBACK_TEST_REGION");
    assert_eq!(ran.code, 4);
    assert_eq!(logged(), vec![r#"where Some("from-env")"#]);
}

#[test]
fn extra_clashing_with_op_word_panics() {
    let result = std::panic::catch_unwind(|| {
        let mut r = base();
        r.push_extra(
            clap::Command::new("build"),
            |_: &G, _: &clap::ArgMatches| -> BoxFuture<'_, u8> { Box::pin(async { 0 }) },
        );
    });
    let message = *result.unwrap_err().downcast::<String>().unwrap();
    assert!(message.contains("clashes with op word"), "{message}");
}

#[test]
fn flags_only_empty_and_leading_flag_go_to_fallback() {
    let mut r = base();
    flags_only(&mut r);
    assert_eq!(run(&r, &[]).code, 7);
    assert_eq!(run(&r, &["--weird", "x"]).code, 7);
    assert_eq!(
        logged(),
        vec!["flags_only []", r#"flags_only ["--weird", "x"]"#]
    );
}

#[test]
fn flags_only_unknown_bare_word_is_usage_2() {
    let mut r = base();
    flags_only(&mut r);
    let ran = run(&r, &["idd"]);
    assert_eq!(ran.code, 2);
    assert_eq!(last_err(&ran)["code"], "op_registry.cli.unknown_command");
    assert!(logged().is_empty());
    // A command still runs.
    assert_eq!(run(&r, &["ping"]).code, 0);
}

#[test]
fn help_and_version_never_reach_fallback() {
    for free in [false, true] {
        let mut r = base();
        if free {
            free_text(&mut r);
        } else {
            flags_only(&mut r);
        }
        for flag in ["-h", "--help"] {
            let ran = run(&r, &[flag]);
            assert_eq!(ran.code, 0, "{flag}");
            assert!(ran.out.contains("A test app."), "{}", ran.out);
        }
        let ran = run(&r, &["--version"]);
        assert_eq!(ran.code, 0);
        assert!(ran.out.contains("1.2.3"), "{}", ran.out);
        assert!(logged().is_empty());
    }
}

#[test]
fn help_fallback_name_prints_its_help() {
    let mut r = base();
    flags_only(&mut r);
    let ran = run(&r, &["help", "fix"]);
    assert_eq!(ran.code, 0);
    assert!(ran.out.contains("Fix things.\nMore text."), "{}", ran.out);
    assert!(logged().is_empty());
    let mut r = base();
    free_text(&mut r);
    let ran = run(&r, &["help", "ask"]);
    assert_eq!(ran.code, 0);
    assert!(ran.out.contains("Ask the agent."), "{}", ran.out);
}

#[test]
fn root_help_lists_fallback() {
    let mut r = base();
    flags_only(&mut r);
    let ran = run(&r, &["--help"]);
    assert!(ran.out.contains("fix: Fix things."), "{}", ran.out);
    assert!(!ran.out.contains("More text."), "{}", ran.out);
}

#[test]
fn free_text_double_dash_escape() {
    let mut r = base();
    free_text(&mut r);
    assert_eq!(run(&r, &["--", "log"]).code, 9);
    assert_eq!(logged(), vec![r#"free_text None ["log"]"#]);
}

#[test]
fn free_text_unknown_dash_flag_goes_to_fallback() {
    let mut r = base();
    free_text(&mut r);
    assert_eq!(run(&r, &["--codebase", "X", "fix", "it"]).code, 9);
    assert_eq!(
        logged(),
        vec![r#"free_text None ["--codebase", "X", "fix", "it"]"#]
    );
}

#[test]
fn free_text_globals_parsed_then_fallback() {
    let mut r = base();
    free_text(&mut r);
    assert_eq!(
        run(&r, &["--endpoint", "e", "--pretty", "fix", "it"]).code,
        9
    );
    assert_eq!(run(&r, &["--endpoint=e3", "-- x"]).code, 9);
    assert_eq!(
        logged(),
        vec![
            r#"free_text Some("e") ["fix", "it"]"#,
            r#"free_text Some("e3") ["-- x"]"#
        ]
    );
}

#[test]
fn free_text_command_word_runs_command() {
    let mut r = base();
    free_text(&mut r);
    let ran = run(&r, &["--endpoint", "e", "ping"]);
    assert_eq!(ran.code, 0);
    assert!(ran.out.contains("\"ok\":true"), "{}", ran.out);
    assert_eq!(run(&r, &["build", "run"]).code, 0);
    assert!(logged().is_empty());
}

#[test]
fn free_text_typo_is_usage_error() {
    let mut r = base();
    free_text(&mut r);
    let ran = run(&r, &["bulid"]);
    assert_eq!(ran.code, 2);
    let err = last_err(&ran);
    assert_eq!(err["code"], "op_registry.cli.unknown_command");
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .contains("unknown command `bulid`; did you mean `build`?"),
        "{err}"
    );
    assert!(logged().is_empty());
}

#[test]
fn free_text_far_word_goes_to_fallback() {
    let mut r = base();
    free_text(&mut r);
    assert_eq!(run(&r, &["refactor"]).code, 9);
}

#[test]
fn free_text_sentence_never_matches() {
    let mut r = base();
    free_text(&mut r);
    assert_eq!(run(&r, &["buld it"]).code, 9);
    assert_eq!(logged(), vec![r#"free_text None ["buld it"]"#]);
}

#[test]
fn free_text_flag_clash_panics() {
    let result = std::panic::catch_unwind(|| {
        let mut r = base();
        r.fallback_free_text(
            "ask",
            "Ask.",
            &["endpoint"],
            |_: &G, _: Vec<OsString>| -> BoxFuture<'_, u8> { Box::pin(async { 0 }) },
        );
        cli::command(&r);
    });
    assert!(result.is_err());
}

#[test]
fn second_fallback_panics() {
    let result = std::panic::catch_unwind(|| {
        let mut r = base();
        flags_only(&mut r);
        free_text(&mut r);
    });
    assert!(result.is_err());
}

#[test]
fn edit_distance_cases() {
    assert_eq!(edit_distance("", ""), 0);
    assert_eq!(edit_distance("abc", ""), 3);
    assert_eq!(edit_distance("", "abc"), 3);
    assert_eq!(edit_distance("build", "build"), 0);
    assert_eq!(edit_distance("bulid", "build"), 2);
    assert_eq!(edit_distance("kitten", "sitting"), 3);
    assert_eq!(edit_distance("refactor", "build"), 8);
}
