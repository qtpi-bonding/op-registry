//! The CLI command tree and `cli::parse`: tree shape, flags, usage failures.

#![allow(dead_code)]

mod common;

use common::{Echo, Out};
use futures_util::future::BoxFuture;
use op_registry::cli::{self, Parsed, UsageFailure};
use op_registry::{
    CallOptions, CliContext, OpGlobals, OpParams, OpSpec, Page, PageRequest, Paths, Registry,
    Style, Surface,
};
use serde::Serialize;
use std::ffi::OsString;

const HUB_ENV: &str = "OPREG_CLI_TREE_HUB_UNIQUE";

/// Serializes the tests that depend on the process environment.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Process settings of the test binary.
#[derive(Debug, OpGlobals)]
struct G {
    /// The hub address.
    #[op(env = "OPREG_CLI_TREE_HUB_UNIQUE")]
    hub: Option<String>,
    /// Chatty output.
    verbose: bool,
}

struct Ctx;
impl op_registry::Resolve for Ctx {}
impl CliContext for Ctx {
    type Globals = G;
    fn from_globals(_globals: &G) -> anyhow::Result<Self> {
        Ok(Ctx)
    }
}

#[derive(Clone, OpParams)]
struct Get {
    /// The concept id.
    id: String,
}

#[derive(Clone, OpParams)]
struct Filter {
    /// Only concepts with this tag.
    tag: Option<String>,
}

#[derive(Clone, OpParams)]
struct Build {
    /// The blueprint to build.
    blueprint: Option<String>,
}

#[derive(Clone, OpParams)]
struct Tune {
    /// How many.
    count: Option<i32>,
    /// A free note.
    note: Option<String>,
    /// A secret pin.
    #[op(secret)]
    pin: Option<u32>,
    /// A required name.
    name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Item {
    id: String,
}

fn unary<P: OpParams + Send + 'static>(r: &mut Registry<Ctx>, spec: OpSpec<P, Out>) {
    r.push_op(
        spec,
        |_c: &Ctx, _p: P, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async { Ok(Out { ok: true }) })
        },
    );
}

fn stream(r: &mut Registry<Ctx>, spec: OpSpec<Echo, Out>) {
    r.push_stream(spec, |_c: &Ctx, _p: Echo, _cur, _o: CallOptions| {
        Box::pin(async {
            Ok(
                Box::pin(futures_util::stream::iter(vec![Ok(Out { ok: true })]))
                    as futures_util::stream::BoxStream<'_, anyhow::Result<Out>>,
            )
        })
    });
}

fn reg() -> Registry<Ctx> {
    let mut r = Registry::new();
    r.set_pillar("fix");
    r.set_bin("app", "1.2.3", "A test app.");
    r.group("concept", "Concepts in a collection.");
    r.group("build", "Build things.");
    r.group("internal", "Internals.");
    r.group("feed", "Feeds.");
    r.group("report", "Reports.");
    r.group("tune", "Tuning.");

    unary(&mut r, OpSpec::<Echo, Out>::new("ping", "Ping."));
    let mut get = OpSpec::<Get, Out>::new("concept get", "Show one concept.");
    get.long_help = "Longer words about getting a concept.";
    unary(&mut r, get);
    let mut purge = OpSpec::<Echo, Out>::new("concept purge", "Purge concepts.");
    purge.hidden = true;
    unary(&mut r, purge);
    r.push_list(
        OpSpec::<Filter, Item>::new("concept list", "List concepts."),
        |_c: &Ctx, _p: Filter, _r: PageRequest| -> BoxFuture<'_, anyhow::Result<Page<Item>>> {
            Box::pin(async {
                Ok(Page {
                    items: vec![],
                    next_cursor: None,
                })
            })
        },
    );
    let mut run = OpSpec::<Build, Out>::new("build run", "Run a build.");
    run.group_default = true;
    unary(&mut r, run);
    unary(
        &mut r,
        OpSpec::<Echo, Out>::new("build status", "Build status."),
    );
    let mut dump = OpSpec::<Echo, Out>::new("internal dump", "Dump.");
    dump.paths = Paths::of(&[Surface::Typed]);
    unary(&mut r, dump);
    stream(&mut r, OpSpec::new("feed watch", "Watch."));
    let mut tail = OpSpec::<Echo, Out>::new("feed tail", "Tail.");
    tail.cursor = true;
    stream(&mut r, tail);
    let mut show = OpSpec::<Echo, Out>::new("report show", "Show a report.");
    show.render_text = Some(|_o: &Out| "text".to_owned());
    unary(&mut r, show);
    unary(
        &mut r,
        OpSpec::<Echo, Out>::new("report plain", "Plain report."),
    );
    unary(&mut r, OpSpec::<Tune, Out>::new("tune set", "Tune."));
    r
}

fn argv(args: &[&str]) -> Vec<OsString> {
    std::iter::once("app")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect()
}

fn parse(args: &[&str]) -> Result<(Parsed, G), UsageFailure> {
    cli::parse(&reg(), &argv(args)).map(|(p, g)| match (&p, g) {
        (Parsed::Help(_) | Parsed::Version(_), None) => (
            p,
            G {
                hub: None,
                verbose: false,
            },
        ),
        (_, g) => (p, g.expect("globals on a non-help parse")),
    })
}

fn op_name(args: &[&str]) -> String {
    match parse(args) {
        Ok((Parsed::Op { name, .. }, _)) => name,
        Ok(_) => panic!("{args:?}: not an op"),
        Err(e) => panic!("{args:?}: {} {}", e.code, e.message),
    }
}

fn fail(args: &[&str]) -> UsageFailure {
    match parse(args) {
        Err(e) => e,
        Ok(_) => panic!("{args:?} should fail"),
    }
}

fn help(args: &[&str]) -> String {
    match parse(args) {
        Ok((Parsed::Help(text), _)) => text,
        Ok(_) => panic!("{args:?}: not help"),
        Err(e) => panic!("{args:?}: {} {}", e.code, e.message),
    }
}

#[test]
fn noun_groups_nest_and_verbs_are_leaves() {
    let cmd = cli::command(&reg());
    cmd.clone().debug_assert();
    assert_eq!(cmd.get_name(), "app");
    let concept = cmd.find_subcommand("concept").unwrap();
    assert_eq!(
        concept.get_about().unwrap().to_string(),
        "Concepts in a collection."
    );
    assert!(concept.find_subcommand("get").is_some());
    assert!(concept.find_subcommand("list").is_some());
    assert_eq!(op_name(&["concept", "get", "--id", "x"]), "concept_get");
}

#[test]
fn one_word_path_is_top_level() {
    let cmd = cli::command(&reg());
    assert!(cmd.find_subcommand("ping").is_some());
    assert_eq!(op_name(&["ping"]), "ping");
}

#[test]
fn group_default_runs_with_no_verb() {
    assert_eq!(op_name(&["build", "--blueprint", "x"]), "build_run");
    assert_eq!(op_name(&["build"]), "build_run");
    assert_eq!(op_name(&["build", "status"]), "build_status");
    assert_eq!(op_name(&["build", "run", "--blueprint", "x"]), "build_run");
}

#[test]
fn non_verb_after_default_group_is_usage_error() {
    let err = fail(&["build", "idd"]);
    assert_eq!(err.code, "op_registry.cli.unknown_command");
    assert!(!err.message.is_empty());
}

#[test]
fn group_default_group_takes_globals_and_verbs_take_library_flags() {
    let (_, g) = parse(&["--hub", "h", "build", "status"]).unwrap();
    assert_eq!(g.hub.as_deref(), Some("h"));
    let (_, g) = parse(&["build", "status", "--hub", "h"]).unwrap();
    assert_eq!(g.hub.as_deref(), Some("h"));
    let (_, g) = parse(&["build", "--hub", "h", "--blueprint", "x"]).unwrap();
    assert_eq!(g.hub.as_deref(), Some("h"));
    assert_eq!(op_name(&["build", "status", "--pretty"]), "build_status");
    assert_eq!(op_name(&["--verbose", "build"]), "build_run");
}

#[test]
fn stray_word_after_a_leaf_is_invalid_not_unknown_command() {
    assert_eq!(
        fail(&["concept", "get", "--id", "x", "extra"]).code,
        "op_registry.usage.invalid"
    );
}

#[test]
fn unknown_and_missing_commands_are_cli_unknown_command() {
    assert_eq!(fail(&["nope"]).code, "op_registry.cli.unknown_command");
    assert_eq!(fail(&["concept"]).code, "op_registry.cli.unknown_command");
    assert_eq!(fail(&[]).code, "op_registry.cli.unknown_command");
}

#[test]
fn hidden_op_absent_from_help_but_parses() {
    assert_eq!(op_name(&["concept", "purge"]), "concept_purge");
    let text = help(&["concept", "--help"]);
    assert!(text.contains("get"), "{text}");
    assert!(!text.contains("purge"), "{text}");
}

#[test]
fn typed_only_op_absent() {
    let cmd = cli::command(&reg());
    assert!(cmd.find_subcommand("internal").is_none());
    assert_eq!(
        fail(&["internal", "dump"]).code,
        "op_registry.cli.unknown_command"
    );
}

#[test]
fn list_pair_one_command_selects_by_flags() {
    let cmd = cli::command(&reg());
    let concept = cmd.find_subcommand("concept").unwrap();
    assert_eq!(
        concept
            .get_subcommands()
            .filter(|c| c.get_name() == "list")
            .count(),
        1
    );
    assert_eq!(op_name(&["concept", "list"]), "concept_list");
    assert_eq!(
        op_name(&["concept", "list", "--limit", "5"]),
        "concept_list_page"
    );
    let (parsed, _) = parse(&["concept", "list", "--cursor", "c"]).unwrap();
    let Parsed::Op { name, matches, .. } = parsed else {
        panic!("not an op")
    };
    assert_eq!(name, "concept_list_page");
    assert_eq!(matches.get_one::<u32>("limit"), Some(&100));
    assert_eq!(matches.get_one::<String>("cursor").unwrap(), "c");
    let (parsed, _) = parse(&["concept", "list", "--limit", "5", "--tag", "t"]).unwrap();
    let Parsed::Op { matches, .. } = parsed else {
        panic!("not an op")
    };
    assert_eq!(matches.get_one::<u32>("limit"), Some(&5));
}

#[test]
fn stream_cursor_flag_only_when_declared() {
    assert_eq!(
        fail(&["feed", "watch", "--cursor", "c"]).code,
        "op_registry.usage.invalid"
    );
    let (parsed, _) = parse(&["feed", "tail", "--cursor", "c"]).unwrap();
    let Parsed::Op { name, cursor, .. } = parsed else {
        panic!("not an op")
    };
    assert_eq!(name, "feed_tail");
    assert_eq!(cursor.unwrap().as_str(), "c");
    let (parsed, _) = parse(&["feed", "tail"]).unwrap();
    let Parsed::Op { cursor, .. } = parsed else {
        panic!("not an op")
    };
    assert!(cursor.is_none());
}

#[test]
fn text_flag_only_on_text_ops() {
    let (parsed, _) = parse(&["report", "show", "--text"]).unwrap();
    let Parsed::Op { text, .. } = parsed else {
        panic!("not an op")
    };
    assert!(text);
    let (parsed, _) = parse(&["report", "show"]).unwrap();
    let Parsed::Op { text, .. } = parsed else {
        panic!("not an op")
    };
    assert!(!text);
    assert_eq!(
        fail(&["report", "plain", "--text"]).code,
        "op_registry.usage.invalid"
    );
}

#[test]
fn globals_before_or_after_verb() {
    let _env = env_lock();
    let (_, before) = parse(&["--hub", "h1", "--verbose", "ping"]).unwrap();
    assert_eq!(before.hub.as_deref(), Some("h1"));
    assert!(before.verbose);
    let (_, after) = parse(&["concept", "get", "--id", "x", "--hub", "h2", "--verbose"]).unwrap();
    assert_eq!(after.hub.as_deref(), Some("h2"));
    assert!(after.verbose);
    let (_, none) = parse(&["ping"]).unwrap();
    assert!(none.hub.is_none() && !none.verbose);
}

#[test]
fn global_env_fallback() {
    let _env = env_lock();
    std::env::set_var(HUB_ENV, "from-env");
    let (_, g) = parse(&["ping"]).unwrap();
    assert_eq!(g.hub.as_deref(), Some("from-env"));
    let (_, g) = parse(&["ping", "--hub", "flag"]).unwrap();
    assert_eq!(g.hub.as_deref(), Some("flag"));
    std::env::remove_var(HUB_ENV);
}

#[test]
fn pretty_compact_conflict() {
    let (parsed, _) = parse(&["--pretty", "ping"]).unwrap();
    let Parsed::Op { style, .. } = parsed else {
        panic!("not an op")
    };
    assert_eq!(style, Some(Style::Pretty));
    let (parsed, _) = parse(&["ping", "--compact"]).unwrap();
    let Parsed::Op { style, .. } = parsed else {
        panic!("not an op")
    };
    assert_eq!(style, Some(Style::Compact));
    let (parsed, _) = parse(&["ping"]).unwrap();
    let Parsed::Op { style, .. } = parsed else {
        panic!("not an op")
    };
    assert_eq!(style, None);
    assert_eq!(
        fail(&["ping", "--pretty", "--compact"]).code,
        "op_registry.usage.invalid"
    );
}

#[test]
fn help_from_doc_comments_on_leaf_and_flags() {
    let text = help(&["concept", "get", "--help"]);
    assert!(text.contains("Show one concept."), "{text}");
    assert!(
        text.contains("Longer words about getting a concept."),
        "{text}"
    );
    assert!(text.contains("--id"), "{text}");
    assert!(text.contains("The concept id."), "{text}");
    let root = help(&["--help"]);
    assert!(root.contains("A test app."), "{root}");
    assert!(root.contains("--hub"), "{root}");
    assert!(root.contains("The hub address."), "{root}");
    assert!(!root.contains(HUB_ENV) || root.contains("[env:"), "{root}");
    match parse(&["--version"]) {
        Ok((Parsed::Version(text), _)) => assert!(text.contains("1.2.3"), "{text}"),
        _ => panic!("expected version"),
    }
}

#[test]
fn secret_value_not_in_clap_error() {
    let err = fail(&["tune", "set", "--name", "n", "--pin", "hunter2xyz"]);
    assert_eq!(err.code, "op_registry.usage.invalid");
    assert_eq!(err.message, "invalid value for --pin");
    assert!(!err.message.contains("hunter2xyz"));
    let err = fail(&["tune", "set", "--name", "n", "--pin=hunter2xyz"]);
    assert!(!err.message.contains("hunter2xyz"), "{}", err.message);
    // A non-secret flag keeps clap's own wording, with its value.
    let err = fail(&["tune", "set", "--name", "n", "--count", "abc"]);
    assert!(err.message.contains("abc"), "{}", err.message);
}

#[test]
fn negative_and_hyphen_values() {
    let (parsed, _) = parse(&["tune", "set", "--name", "n", "--count", "-3"]).unwrap();
    let Parsed::Op { matches, .. } = parsed else {
        panic!("not an op")
    };
    assert_eq!(matches.get_one::<i32>("count"), Some(&-3));
    let (parsed, _) = parse(&["tune", "set", "--name", "n", "--note=-x"]).unwrap();
    let Parsed::Op { matches, .. } = parsed else {
        panic!("not an op")
    };
    assert_eq!(matches.get_one::<String>("note").unwrap(), "-x");
    let err = fail(&["tune", "set", "--name", "n", "--note", "-x"]);
    assert_eq!(err.code, "op_registry.usage.invalid");
}

#[test]
fn usage_missing_flag_code() {
    let err = fail(&["concept", "get"]);
    assert_eq!(err.code, "op_registry.usage.missing");
    assert!(err.message.contains("--id"), "{}", err.message);
}

#[test]
#[should_panic(expected = "clashes with a library global")]
fn library_global_clash_panics() {
    #[derive(OpGlobals)]
    struct Clash {
        /// Clashing.
        pretty: bool,
    }
    struct C2;
    impl op_registry::Resolve for C2 {}
    impl CliContext for C2 {
        type Globals = Clash;
        fn from_globals(_g: &Clash) -> anyhow::Result<Self> {
            Ok(C2)
        }
    }
    let _ = cli::command(&Registry::<C2>::new());
}

/// A binary with a required global still answers `--help` and `--version`.
#[derive(Debug, OpGlobals)]
struct Strict {
    /// Required.
    hub: String,
}

struct StrictCtx;
impl op_registry::Resolve for StrictCtx {}
impl CliContext for StrictCtx {
    type Globals = Strict;
    fn from_globals(_g: &Strict) -> anyhow::Result<Self> {
        Ok(StrictCtx)
    }
}

#[test]
fn help_and_version_work_with_a_required_global() {
    let mut r = Registry::<StrictCtx>::new();
    r.set_bin("strict", "9.9.9", "Strict.");
    let (parsed, globals) = cli::parse(&r, &argv(&["--help"])).unwrap();
    assert!(matches!(parsed, Parsed::Help(_)) && globals.is_none());
    let (parsed, globals) = cli::parse(&r, &argv(&["--version"])).unwrap();
    assert!(matches!(parsed, Parsed::Version(ref v) if v.contains("9.9.9")) && globals.is_none());
}

#[test]
fn missing_required_global_is_usage_missing() {
    let mut r = Registry::<StrictCtx>::new();
    r.group("a", "A.");
    r.push_op(
        OpSpec::<Echo, Out>::new("a b", "B."),
        |_c: &StrictCtx, _p: Echo, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async { Ok(Out { ok: true }) })
        },
    );
    let err = cli::parse(&r, &argv(&["a", "b"])).unwrap_err();
    assert_eq!(err.code, "op_registry.usage.missing");
    let (_, g) = cli::parse(&r, &argv(&["a", "b", "--hub", "h"])).unwrap();
    assert_eq!(g.unwrap().hub, "h");
}

/// An op with a relation and a needed global, an env param and a prompted secret.
#[derive(Clone, OpParams)]
struct Rules {
    /// The recipients.
    #[op(required_unless = "signed_file")]
    to: Vec<String>,
    /// A signed message file.
    #[op(conflicts_with = "to")]
    signed_file: Option<String>,
    /// The region.
    #[op(env = "OPREG_CLI_TREE_REGION_UNIQUE")]
    region: Option<String>,
    /// The API key.
    #[cfg(feature = "secrets")]
    #[op(prompt = "API key")]
    api_key: op_registry::SecretFile,
}

fn rules_reg() -> Registry<Ctx> {
    let mut r = Registry::new();
    r.set_pillar("fix");
    r.set_bin("app", "1.2.3", "A test app.");
    let mut spec = OpSpec::<Rules, Out>::new("send", "Send it.");
    spec.needs_globals = vec!["hub"];
    unary(&mut r, spec);
    r
}

fn rules_help() -> String {
    match cli::parse(&rules_reg(), &argv(&["send", "--help"])) {
        Ok((Parsed::Help(text), _)) => text,
        Ok(_) => panic!("not help"),
        Err(e) => panic!("{} {}", e.code, e.message),
    }
}

#[test]
fn help_ends_with_rules() {
    let text = rules_help();
    assert!(
        text.trim_end().ends_with(
            "Rules:\n  --to is required unless --signed-file is given\n  --signed-file conflicts with --to"
        ),
        "{text}"
    );
    // Short help (`-h`) has the needs line and no Rules block.
    let short = match cli::parse(&rules_reg(), &argv(&["send", "-h"])) {
        Ok((Parsed::Help(text), _)) => text,
        _ => panic!("not help"),
    };
    assert!(short.contains("Needs: --hub"), "{short}");
    assert!(!short.contains("Rules:"), "{short}");
    // An op without relations has no Rules block.
    assert!(!help(&["concept", "get", "--help"]).contains("Rules:"));
}

#[test]
fn help_holds_needs_env_and_prompt_lines() {
    let text = rules_help();
    assert!(
        text.contains("Needs: --hub (or OPREG_CLI_TREE_HUB_UNIQUE)."),
        "{text}"
    );
    assert!(
        text.contains("[env: OPREG_CLI_TREE_REGION_UNIQUE]"),
        "{text}"
    );
    #[cfg(feature = "secrets")]
    assert!(
        text.contains("Asked for on a terminal when left out."),
        "{text}"
    );
    // The needs line stays above the rules.
    assert!(
        text.find("Needs:").unwrap() < text.find("Rules:").unwrap(),
        "{text}"
    );
}
