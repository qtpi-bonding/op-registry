//! Extra commands and the FlagsOnly / FreeText root fallbacks.

use futures_util::future::BoxFuture;
use op_registry::cli::{self, edit_distance, FallbackFlag, Io, NoHooks, Outcome, RunHooks};
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
    /// A secret the error line must not show.
    #[op(secret)]
    token: Option<String>,
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
    for path in ["ping", "build run", "task log", "log", "stats"] {
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
        |args: Vec<OsString>| -> BoxFuture<'static, anyhow::Result<u8>> {
            Box::pin(async move {
                log(format!("flags_only {:?}", strings(&args)));
                Ok(7)
            })
        },
    );
}

const FLAGS: &[FallbackFlag] = &[
    FallbackFlag {
        long: "codebase",
        short: Some('c'),
        takes_value: true,
    },
    FallbackFlag {
        long: "allow",
        short: None,
        takes_value: true,
    },
    FallbackFlag {
        long: "dry",
        short: Some('d'),
        takes_value: false,
    },
];

fn free_text_with(r: &mut Registry<Ctx>, near_miss: bool) {
    r.fallback_free_text(
        "ask",
        "Ask the agent.\nMore text.",
        FLAGS,
        near_miss,
        |g: &G, args: Vec<OsString>| -> BoxFuture<'_, anyhow::Result<u8>> {
            Box::pin(async move {
                log(format!("free_text {:?} {:?}", g.endpoint, strings(&args)));
                Ok(9)
            })
        },
    );
}

fn free_text(r: &mut Registry<Ctx>) {
    free_text_with(r, true);
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
        |g: &G, m: &clap::ArgMatches| -> BoxFuture<'_, anyhow::Result<u8>> {
            Box::pin(async move {
                log(format!(
                    "serve {:?} {:?}",
                    g.endpoint,
                    m.get_one::<String>("port")
                ));
                Ok(3)
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
        |g: &G, _: &clap::ArgMatches| -> BoxFuture<'_, anyhow::Result<u8>> {
            Box::pin(async move {
                log(format!("where {:?}", g.region));
                Ok(4)
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
            |_: &G, _: &clap::ArgMatches| -> BoxFuture<'_, anyhow::Result<u8>> {
                Box::pin(async { Ok(0) })
            },
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
    assert!(
        ran.out.contains("fix: Fix things. (more: app help fix)"),
        "{}",
        ran.out
    );
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
            &[FallbackFlag {
                long: "endpoint",
                short: None,
                takes_value: true,
            }],
            true,
            |_: &G, _: Vec<OsString>| -> BoxFuture<'_, anyhow::Result<u8>> {
                Box::pin(async { Ok(0) })
            },
        );
        cli::command(&r);
    });
    assert!(result.is_err());
}

#[test]
fn short_flag_clash_panics() {
    // `-h` belongs to the library.
    let result = std::panic::catch_unwind(|| {
        let mut r = base();
        r.fallback_free_text(
            "ask",
            "Ask.",
            &[FallbackFlag {
                long: "hello",
                short: Some('h'),
                takes_value: false,
            }],
            true,
            |_: &G, _: Vec<OsString>| -> BoxFuture<'_, anyhow::Result<u8>> {
                Box::pin(async { Ok(0) })
            },
        );
        cli::command(&r);
    });
    let message = *result.unwrap_err().downcast::<String>().unwrap();
    assert!(message.contains("-h"), "{message}");
}

#[test]
fn empty_argv_reaches_fallback_with_empty_vec() {
    let mut r = base();
    free_text(&mut r);
    assert_eq!(run(&r, &[]).code, 9);
    assert_eq!(logged(), vec!["free_text None []"]);
}

#[test]
fn globals_only_reaches_fallback_with_empty_vec() {
    let mut r = base();
    free_text(&mut r);
    assert_eq!(run(&r, &["--endpoint", "e", "--pretty"]).code, 9);
    assert_eq!(logged(), vec![r#"free_text Some("e") []"#]);
}

#[test]
fn trailing_globals_taken_from_tail() {
    let mut r = base();
    free_text(&mut r);
    assert_eq!(run(&r, &["fix it", "--endpoint", "e", "--pretty"]).code, 9);
    assert_eq!(run(&r, &["-x", "word", "--endpoint=e2"]).code, 9);
    assert_eq!(
        logged(),
        vec![
            r#"free_text Some("e") ["fix it"]"#,
            r#"free_text Some("e2") ["-x", "word"]"#
        ]
    );
}

#[test]
fn value_flag_value_spelling_a_global_kept() {
    let mut r = base();
    free_text(&mut r);
    assert_eq!(
        run(
            &r,
            &["go", "--allow", "--pretty", "-c", "--endpoint", "--dry"]
        )
        .code,
        9
    );
    assert_eq!(
        logged(),
        vec![r#"free_text None ["go", "--allow", "--pretty", "-c", "--endpoint", "--dry"]"#]
    );
}

#[test]
fn double_dash_stops_scan() {
    let mut r = base();
    free_text(&mut r);
    assert_eq!(
        run(&r, &["go", "--endpoint", "e", "--", "--endpoint", "x"]).code,
        9
    );
    assert_eq!(
        logged(),
        vec![r#"free_text Some("e") ["go", "--", "--endpoint", "x"]"#]
    );
}

#[test]
fn near_miss_false_sends_stat_to_fallback() {
    let mut r = base();
    free_text_with(&mut r, false);
    // `stat` is within distance 2 of `stats`.
    assert_eq!(run(&r, &["stat"]).code, 9);
    assert_eq!(logged(), vec![r#"free_text None ["stat"]"#]);
}

#[test]
fn near_miss_true_refuses_stat() {
    let mut r = base();
    free_text_with(&mut r, true);
    let ran = run(&r, &["stat"]);
    assert_eq!(ran.code, 2);
    assert_eq!(last_err(&ran)["code"], "op_registry.cli.unknown_command");
    assert!(logged().is_empty());
}

#[test]
fn free_text_err_goes_through_the_error_path() {
    let mut r = base();
    r.fallback_free_text(
        "ask",
        "Ask.",
        &[],
        true,
        |g: &G, _: Vec<OsString>| -> BoxFuture<'_, anyhow::Result<u8>> {
            Box::pin(
                async move { anyhow::bail!("rejected {}", g.token.clone().unwrap_or_default()) },
            )
        },
    );
    let rec = Rec::default();
    let ran = run_hooked(&r, &["hello", "--token", "s3cretvalue"], &rec);
    assert_eq!(ran.code, 70);
    assert_eq!(*rec.0.lock().unwrap(), ["post_run_err"]);
    assert!(!ran.err.contains("s3cretvalue"), "{}", ran.err);
}

#[test]
fn root_help_has_fallback_pointer() {
    for free in [false, true] {
        let mut r = base();
        if free {
            free_text(&mut r);
        } else {
            flags_only(&mut r);
        }
        let name = if free { "ask" } else { "fix" };
        let ran = run(&r, &["--help"]);
        assert!(
            ran.out.contains(&format!("(more: app help {name})")),
            "{}",
            ran.out
        );
    }
}

#[test]
fn fallback_help_replaces_bin() {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.set_bin("gait-min", "1.0.0", "Min.");
    r.fallback_flags_only(
        "turn",
        "Run a turn.\nUsage: {bin} \"task\"",
        |_: Vec<OsString>| -> BoxFuture<'static, anyhow::Result<u8>> { Box::pin(async { Ok(0) }) },
    );
    let ran = run(&r, &["help", "turn"]);
    assert_eq!(ran.code, 0);
    assert!(ran.out.contains("Usage: gait-min \"task\""), "{}", ran.out);
    assert!(!ran.out.contains("{bin}"), "{}", ran.out);
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

/// Records the hooks that fire, in order, and the exit the outcome carries.
#[derive(Default)]
struct Rec(std::sync::Mutex<Vec<String>>);

impl RunHooks for Rec {
    fn on_usage_error(&self, _: &cli::UsageFailure) {
        self.0.lock().unwrap().push("on_usage_error".to_owned());
    }
    fn post_run(&self, outcome: &Outcome) {
        let tag = match outcome {
            Outcome::Ok { exit, .. } => format!("post_run_ok {exit}"),
            Outcome::Err { .. } => "post_run_err".to_owned(),
            _ => "post_run_other".to_owned(),
        };
        self.0.lock().unwrap().push(tag);
    }
}

fn run_hooked(r: &Registry<Ctx>, args: &[&str], hooks: &dyn RunHooks) -> Ran {
    let argv: Vec<OsString> = std::iter::once("app")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect();
    let (io, cap) = Io::memory(b"", false, false);
    let code = futures_executor::block_on(cli::run_with_io(r, argv, hooks, io));
    Ran {
        code,
        out: cap.stdout(),
        err: cap.stderr(),
    }
}

#[test]
fn flags_only_fallback_err_post_run_sees_err() {
    let mut r = base();
    r.fallback_flags_only(
        "fix",
        "Fix things.",
        |_args: Vec<OsString>| -> BoxFuture<'static, anyhow::Result<u8>> {
            Box::pin(async {
                Err(op_registry::usage(op_registry::UsageError::InvalidField {
                    name: "x".to_owned(),
                    reason: "bad".to_owned(),
                }))
            })
        },
    );
    let rec = Rec::default();
    let ran = run_hooked(&r, &["--weird"], &rec);
    assert_eq!(ran.code, 2);
    assert_eq!(*rec.0.lock().unwrap(), ["on_usage_error", "post_run_err"]);
    assert_eq!(last_err(&ran)["code"], "op_registry.usage.invalid");
    assert_eq!(ran.out, "");
}

#[test]
fn extra_err_redacts_secret_globals() {
    let mut r = base();
    r.push_extra(
        clap::Command::new("leak"),
        |g: &G, _: &clap::ArgMatches| -> BoxFuture<'_, anyhow::Result<u8>> {
            Box::pin(
                async move { anyhow::bail!("rejected {}", g.token.clone().unwrap_or_default()) },
            )
        },
    );
    let ran = run(&r, &["leak", "--token", "s3cretvalue"]);
    assert_eq!(ran.code, 70);
    assert!(!ran.err.contains("s3cretvalue"), "{}", ran.err);
    assert!(last_err(&ran)["message"]
        .as_str()
        .unwrap()
        .contains("<secret>"));
}

fn before_runtime(r: &mut Registry<Ctx>) {
    r.fallback_flags_only_before_runtime("daemon", "Run the daemon.\nMore text.");
}

fn argv(args: &[&str]) -> Vec<OsString> {
    std::iter::once("app")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect()
}

#[test]
fn select_empty_argv_is_fallback() {
    let mut r = base();
    before_runtime(&mut r);
    let selected = cli::select(&r, &argv(&[])).unwrap();
    assert_eq!(
        selected,
        cli::Selected::Fallback {
            name: "daemon",
            args: vec![]
        }
    );
}

#[test]
fn select_flag_led_argv_is_fallback() {
    let mut r = base();
    flags_only(&mut r);
    let selected = cli::select(&r, &argv(&["--agent-dir", "/x", "-v"])).unwrap();
    assert_eq!(
        selected,
        cli::Selected::Fallback {
            name: "fix",
            args: vec!["--agent-dir".into(), "/x".into(), "-v".into()]
        }
    );
}

#[test]
fn select_command_is_op() {
    let mut r = base();
    before_runtime(&mut r);
    assert_eq!(
        cli::select(&r, &argv(&["build", "run"])).unwrap(),
        cli::Selected::Op {
            name: "build_run".into()
        }
    );
    assert_eq!(
        cli::select(&r, &argv(&["--help"])).unwrap(),
        cli::Selected::Help
    );
}

#[test]
fn select_bare_unknown_word_is_usage_failure() {
    let mut r = base();
    before_runtime(&mut r);
    let failure = cli::select(&r, &argv(&["bogus"])).unwrap_err();
    assert_eq!(failure.code, "op_registry.cli.unknown_command");
}

#[test]
fn run_reaching_before_runtime_fallback_exits_70() {
    let mut r = base();
    before_runtime(&mut r);
    let ran = run(&r, &["--endpoint", "x"]);
    assert_eq!(ran.code, 70, "{}", ran.err);
    let error = last_err(&ran);
    assert_eq!(
        error["message"],
        "fallback `daemon` runs before the runtime; dispatch it after cli::select"
    );
    assert_eq!(error["code"], "unknown");
}

#[test]
fn before_runtime_fallback_in_help_and_manual() {
    let mut r = base();
    before_runtime(&mut r);
    let ran = run(&r, &["--help"]);
    assert!(
        ran.out
            .contains("daemon: Run the daemon. (more: app help daemon)"),
        "{}",
        ran.out
    );
    let ran = run(&r, &["help", "daemon"]);
    assert_eq!(ran.code, 0);
    assert!(
        ran.out.contains("Run the daemon.\nMore text."),
        "{}",
        ran.out
    );
    let manual = op_registry::manual::render(&r, &op_registry::manual::Manual::default()).unwrap();
    assert!(manual.contains("daemon"), "{manual}");
    assert!(manual.contains("Run the daemon.\nMore text."), "{manual}");
}
