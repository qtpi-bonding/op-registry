//! The generated manual, `render_json` and the `manual` op.
#![cfg(feature = "manual")]
#![allow(dead_code)]

use coded_error::{CodeEntry, ErrorClass};
use futures_util::future::BoxFuture;
use op_registry::cli::{self, Io, NoHooks};
use op_registry::manual::{render, render_json, FrontMatter, Guide, Manual, ManualOp, Section};
use op_registry::mcp::{call_tool, tool_list};
#[cfg(feature = "testing")]
use op_registry::testing::check_exit_table;
use op_registry::{
    Access, Audience, CallOptions, CliContext, OpGlobals, OpParams, OpSpec, Page, PageRequest,
    Paths, Registry, Surface,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::ffi::OsString;

const HUB_ENV: &str = "OPREG_MANUAL_HUB_UNIQUE";

/// Serializes the tests that read or write the process environment.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Process settings of the test binary.
#[derive(Debug, OpGlobals)]
struct G {
    /// The hub address.
    #[op(env = "OPREG_MANUAL_HUB_UNIQUE")]
    hub: Option<String>,
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

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Item {
    id: String,
}

fn get_op(_c: &Ctx, p: Get, _o: CallOptions) -> BoxFuture<'_, anyhow::Result<Item>> {
    Box::pin(async move { Ok(Item { id: p.id }) })
}

fn page_op(_c: &Ctx, _p: Filter, _r: PageRequest) -> BoxFuture<'_, anyhow::Result<Page<Item>>> {
    Box::pin(async {
        Ok(Page {
            items: vec![],
            next_cursor: None,
        })
    })
}

fn nothing<P: Send + 'static>(
    _c: &Ctx,
    _p: P,
    _o: CallOptions,
) -> BoxFuture<'_, anyhow::Result<Item>> {
    Box::pin(async { Ok(Item { id: "x".into() }) })
}

/// Two ops (`concept get`, the `concept list` pair), one group, one declared
/// exit and a code whose class varies.
fn fixture() -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.set_bin("concordd", "1.2.3", "A concordance tool.");
    r.group("concept", "Concepts in a collection.");
    r.add_common_errors(vec![
        CodeEntry {
            code: "fix.common.denied",
            class: Some(ErrorClass::Logical),
        },
        CodeEntry {
            code: "fix.common.busy",
            class: Some(ErrorClass::Transient),
        },
    ]);
    let mut get = OpSpec::<Get, Item>::new("concept get", "Show one concept.");
    get.access = Access::reads().with_network();
    get.exits = vec![(4, "records drifted")];
    get.error_exits = vec![(5, "busy", "fix.common.busy")];
    get.errors = vec![
        CodeEntry {
            code: "fix.concept.not_found",
            class: Some(ErrorClass::Logical),
        },
        CodeEntry {
            code: "fix.concept.drift",
            class: None,
        },
        CodeEntry {
            code: "fix.common.busy",
            class: Some(ErrorClass::Transient),
        },
    ];
    r.push_op(get, get_op);
    let mut list = OpSpec::<Filter, Item>::new("concept list", "List concepts.");
    list.access = Access::reads();
    r.push_list(list, page_op);
    r
}

fn sample_manual() -> Manual {
    Manual {
        front_matter: None,
        sections: vec![Section {
            title: "Getting started".into(),
            guides: vec![Guide {
                title: "Read a concept".into(),
                body: "Fetch one concept by its id.".into(),
                commands: vec!["concept get".into()],
            }],
        }],
    }
}

fn render_fixture() -> String {
    render(&fixture(), &sample_manual()).unwrap()
}

#[test]
fn manual_matches_snapshot() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/manual_snapshot.md");
    let fresh = render_fixture();
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(path, &fresh).unwrap();
    }
    let snapshot = std::fs::read_to_string(path).unwrap();
    assert_eq!(fresh, snapshot, "run with UPDATE_SNAPSHOTS=1 to rewrite");
}

#[test]
fn missing_guide_path_is_error() {
    let mut manual = sample_manual();
    manual.sections[0].guides[0].commands = vec!["concept nope".into()];
    let err = render(&fixture(), &manual).unwrap_err();
    assert!(format!("{err:#}").contains("concept nope"), "{err:#}");
}

#[test]
fn front_matter_rendered() {
    let mut manual = sample_manual();
    manual.front_matter = Some(FrontMatter {
        name: "concordd-manual".into(),
        description: "How to use concordd.".into(),
    });
    let text = render(&fixture(), &manual).unwrap();
    assert!(text.starts_with(
        "---\nname: concordd-manual\ndescription: How to use concordd.\n---\n\n# concordd\n"
    ));
    assert!(!render_fixture().starts_with("---"));
}

#[test]
fn library_code_table_lists_each_code_once() {
    let text = render_fixture();
    let library = text
        .split("## Library codes")
        .nth(1)
        .expect("library codes section");
    let library = library.split("\n## ").next().unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for line in library.lines().filter(|l| l.starts_with("| op_registry.")) {
        let code = line.split('|').nth(1).unwrap().trim();
        assert!(seen.insert(code.to_owned()), "{code} is listed twice");
    }
    // `MissingField` and `MissingUnless` share one code.
    assert!(seen.contains("op_registry.usage.missing"));
    assert!(seen.contains("op_registry.usage.conflicts"));
    assert!(seen.contains("op_registry.usage.at_least_one"));
}

#[test]
fn codes_listed_with_class_and_exit() {
    let text = render_fixture();
    assert!(text.contains("| fix.concept.not_found | logical | 65 |"));
    assert!(text.contains("| fix.concept.drift | varies | varies |"));
    assert!(text.contains("- 4: records drifted"));
    assert!(text.contains("Access: reads, network"));
    assert!(text.contains("| op_registry.usage.missing | logical | 2 |"));
    assert!(text.contains("| op_registry.cli.unknown_command | logical | 2 |"));
    assert!(text.contains("| op_registry.call.unknown_operation | logical | 65 |"));
}

#[test]
fn too_many_listed_for_list_ops() {
    let text = render_fixture();
    assert!(text.contains("| fix.concept.too_many | logical | 65 |"));
    // The list pair is one command.
    assert_eq!(text.matches("### concordd concept list").count(), 1);
    assert!(!text.contains("concept list-page"));
}

#[test]
fn one_word_list_cap_code_in_manual() {
    let mut r = fixture();
    r.push_list(
        OpSpec::<Filter, Item>::new("graph-search", "Search the graph."),
        page_op,
    );
    let text = render(&r, &sample_manual()).unwrap();
    assert!(
        text.contains("| fix.graph_search.too_many | logical | 65 |"),
        "{text}"
    );
}

#[test]
fn manual_shows_paths_line() {
    let mut r = fixture();
    let mut owner = OpSpec::<Filter, Item>::new("concept sign", "Sign it.");
    owner.paths = Paths::of(&[Surface::Cli, Surface::Typed]);
    r.push_op(owner, nothing::<Filter>);
    let text = render(&r, &sample_manual()).unwrap();
    let section = text.split("### concordd concept sign").nth(1).unwrap();
    let section = section.split("\n### ").next().unwrap();
    assert!(section.contains("Paths: cli, typed"), "{section}");
    // An op on every surface has no paths line.
    assert_eq!(text.matches("Paths: ").count(), 1);
}

#[test]
fn manual_lists_ops_off_the_command_line() {
    let mut r = fixture();
    let mut tool = OpSpec::<Filter, Item>::new("concept probe", "Probe it.");
    tool.long_help = "Only agents call this.";
    tool.access = Access::reads();
    tool.exits = vec![(5, "probe failed")];
    tool.ok_exit = Some(|_o: &Item| 5);
    tool.paths = Paths::of(&[Surface::Mcp]);
    r.push_op(tool, nothing::<Filter>);
    let mut inner = OpSpec::<Filter, Item>::new("concept inner", "Inner use.");
    inner.paths = Paths::of(&[Surface::Typed]);
    r.push_op(inner, nothing::<Filter>);
    let mut hidden = OpSpec::<Filter, Item>::new("concept purge", "Purge it.");
    hidden.hidden = true;
    hidden.paths = Paths::of(&[Surface::Typed]);
    r.push_op(hidden, nothing::<Filter>);
    let text = render(&r, &sample_manual()).unwrap();
    let (cli_part, off) = text
        .split_once("## Operations off the command line")
        .expect("the section exists");
    let off = off.split("\n## ").next().unwrap();
    assert!(!cli_part.contains("concept_probe") && !cli_part.contains("concept probe"));
    let probe = off.split("### concept_probe").nth(1).unwrap();
    let probe = probe.split("\n### ").next().unwrap();
    assert!(probe.contains("Probe it."), "{probe}");
    assert!(probe.contains("Only agents call this."), "{probe}");
    assert!(probe.contains("Paths: mcp"), "{probe}");
    assert!(probe.contains("Access: reads"), "{probe}");
    assert!(probe.contains("- 5: probe failed"), "{probe}");
    let inner = off.split("### concept_inner").nth(1).unwrap();
    assert!(inner.contains("Paths: typed"), "{inner}");
    // A hidden op is not listed; the section is absent when nothing is off the CLI.
    assert!(!text.contains("concept_purge"));
    assert!(!render_fixture().contains("Operations off the command line"));
}

#[cfg(feature = "testing")]
#[test]
fn exit_table_checks_off_cli_ops() {
    let mut r = fixture();
    let mut tool = OpSpec::<Filter, Item>::new("concept probe", "Probe it.");
    tool.exits = vec![(5, "probe failed")];
    tool.ok_exit = Some(|_o: &Item| 5);
    tool.paths = Paths::of(&[Surface::Mcp]);
    r.push_op(tool, nothing::<Filter>);
    let rendered = render(&r, &sample_manual()).unwrap();
    assert!(check_exit_table(&r, &rendered).is_empty());
    let broken = rendered.replace("- 5: probe failed", "- 5: something else");
    let problems = check_exit_table(&r, &broken);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("concept_probe") && p.contains("declared exit 5")),
        "{problems:?}"
    );
}

/// One op `concept wait` that declares the error exit
/// `3 => "timed out" on "fix.concept.timeout"`, and a common code the op also
/// lists on its own.
fn exits_fixture() -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.set_bin("concordd", "1.2.3", "A concordance tool.");
    r.group("concept", "Concepts in a collection.");
    r.add_common_errors(vec![
        CodeEntry {
            code: "fix.common.denied",
            class: Some(ErrorClass::Logical),
        },
        CodeEntry {
            code: "fix.common.busy",
            class: Some(ErrorClass::Transient),
        },
    ]);
    let mut wait = OpSpec::<Filter, Item>::new("concept wait", "Wait for a concept.");
    wait.errors = vec![
        CodeEntry {
            code: "fix.concept.timeout",
            class: Some(ErrorClass::Transient),
        },
        CodeEntry {
            code: "fix.common.denied",
            class: Some(ErrorClass::Logical),
        },
    ];
    wait.error_exits = vec![(3, "timed out", "fix.concept.timeout")];
    r.push_op(wait, nothing::<Filter>);
    r
}

#[test]
fn manual_error_exit_line_and_table() {
    let text = render(&exits_fixture(), &sample_manual_empty()).unwrap();
    assert!(
        text.contains("- 3: timed out (on fix.concept.timeout)"),
        "{text}"
    );
    assert!(
        text.contains("| fix.concept.timeout | transient | 3 |"),
        "{text}"
    );
    let json = render_json(&exits_fixture());
    let wait = &json["ops"][0];
    assert_eq!(wait["exits"][0]["on"], "fix.concept.timeout");
    assert_eq!(
        wait["codes"][0],
        json!({"code": "fix.concept.timeout", "class": "transient", "exit": 3})
    );
    #[cfg(feature = "testing")]
    assert!(check_exit_table(&exits_fixture(), &text).is_empty());
}

#[test]
fn manual_on_code_that_is_common_shows_declared_exit() {
    let mut r = exits_fixture();
    let mut spec = OpSpec::<Filter, Item>::new("concept hold", "Hold.");
    spec.errors = vec![CodeEntry {
        code: "fix.common.busy",
        class: Some(ErrorClass::Transient),
    }];
    spec.error_exits = vec![(5, "busy", "fix.common.busy")];
    r.push_op(spec, nothing::<Filter>);
    let text = render(&r, &sample_manual_empty()).unwrap();
    let section = text.split("### concordd concept hold").nth(1).unwrap();
    let section = section.split("\n## ").next().unwrap();
    assert!(
        section.contains("- 5: busy (on fix.common.busy)"),
        "{section}"
    );
    assert!(
        section.contains("| fix.common.busy | transient | 5 |"),
        "{section}"
    );
    // The common table stays class-only.
    assert!(text.contains("| fix.common.busy | transient | 75 |"));
    #[cfg(feature = "testing")]
    assert!(check_exit_table(&r, &text).is_empty());
}

#[test]
fn manual_common_codes_once_and_json_common_codes() {
    let r = exits_fixture();
    let text = render(&r, &sample_manual_empty()).unwrap();
    assert_eq!(text.matches("## Common codes").count(), 1);
    let exit_table = text.find("## Exit codes").unwrap();
    let common = text.find("## Common codes").unwrap();
    assert!(exit_table < common && common < text.find("## Commands").unwrap());
    // The code the op also lists on its own is printed once, in the common table.
    assert_eq!(text.matches("| fix.common.denied |").count(), 1, "{text}");
    assert_eq!(text.matches("| fix.common.busy |").count(), 1, "{text}");
    assert!(text.contains("| fix.common.busy | transient | 75 |"));
    let section = text.split("### concordd concept wait").nth(1).unwrap();
    let section = section.split("\n## ").next().unwrap();
    assert!(section.contains("| fix.concept.timeout |"), "{section}");
    assert!(!section.contains("fix.common.denied"), "{section}");
    assert!(section.contains("Also the common codes."), "{section}");

    let json = render_json(&r);
    let common: Vec<&str> = json["commonCodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap())
        .collect();
    assert_eq!(common, ["fix.common.denied", "fix.common.busy"]);
    let own: Vec<&str> = json["ops"][0]["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap())
        .collect();
    assert_eq!(own, ["fix.concept.timeout"]);
    // No common codes: no section and an empty list.
    let bare = Registry::<Ctx>::new();
    assert!(!render(&bare, &sample_manual_empty())
        .unwrap()
        .contains("## Common codes"));
    assert_eq!(render_json(&bare)["commonCodes"], json!([]));
}

#[test]
fn hidden_op_not_in_manual() {
    let mut r = fixture();
    let mut hidden = OpSpec::<Filter, Item>::new("concept purge", "Purge it.");
    hidden.hidden = true;
    r.push_op(hidden, nothing::<Filter>);
    let text = render(&r, &sample_manual()).unwrap();
    assert!(!text.contains("concept purge"));
    // The JSON lists every op.
    let json = render_json(&r);
    let names: Vec<&str> = json["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"concept_purge"));
}

fn with_manual() -> Registry<Ctx> {
    let mut r = fixture();
    r.with_manual(sample_manual(), Audience::Agent);
    r
}

fn argv(args: &[&str]) -> Vec<OsString> {
    std::iter::once("concordd")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect()
}

#[test]
fn manual_op_out_writes_file() {
    let dir = std::env::temp_dir().join(format!("oprm-{}", std::process::id()));
    let file = dir.join("nested/dir/MANUAL.md");
    let r = with_manual();
    let (io, out) = Io::memory(b"", false, false);
    let code = futures_executor::block_on(cli::run_with_io(
        &r,
        argv(&["manual", "--out", file.to_str().unwrap()]),
        &NoHooks,
        io,
    ));
    assert_eq!(code, 0, "{}", out.stderr());
    let written: Value = serde_json::from_str(out.stdout().trim()).unwrap();
    assert_eq!(written["written"], file.to_str().unwrap());
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("# concordd\n"));
    assert!(text.contains("### concordd manual"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn manual_op_text_prints_markdown_raw() {
    let r = with_manual();
    let (io, out) = Io::memory(b"", false, false);
    let code = futures_executor::block_on(cli::run_with_io(
        &r,
        argv(&["manual", "--text"]),
        &NoHooks,
        io,
    ));
    assert_eq!(code, 0, "{}", out.stderr());
    assert!(out.stdout().starts_with("# concordd\n"));
}

#[test]
fn manual_json_has_ops_and_codes() {
    let r = with_manual();
    let json = render_json(&r);
    assert_eq!(json["bin"], "concordd");
    assert_eq!(json["version"], "1.2.3");
    assert_eq!(json["exits"][0], json!({"code": 0, "meaning": "ok"}));
    let ops = json["ops"].as_array().unwrap();
    let get = ops.iter().find(|o| o["name"] == "concept_get").unwrap();
    assert_eq!(
        get["codes"][0],
        json!({"code": "fix.concept.not_found", "class": "logical", "exit": 65})
    );
    assert_eq!(
        get["codes"][1],
        json!({"code": "fix.concept.drift", "class": null, "exit": null})
    );
    assert_eq!(get["exits"][0]["code"], 4);
    assert!(ops.iter().any(|o| o["name"] == "manual"));
    assert_eq!(get.get("outputSchema").is_some(), cfg!(feature = "schema"));
    for key in ["longHelp", "inputSchema", "listPair", "groupDefault"] {
        assert!(get.get(key).is_some(), "missing {key}");
    }
    assert!(get.get("long_help").is_none());

    let (io, out) = Io::memory(b"", false, false);
    let code = futures_executor::block_on(cli::run_with_io(
        &r,
        argv(&["manual", "--json"]),
        &NoHooks,
        io,
    ));
    assert_eq!(code, 0, "{}", out.stderr());
    let printed: Value = serde_json::from_str(out.stdout().trim()).unwrap();
    assert_eq!(printed, render_json(&r));
}

#[test]
fn manual_is_an_op_on_every_path() {
    let r = with_manual();
    let tools = tool_list(&r, Audience::Agent);
    assert!(tools.iter().any(|t| t.name == "manual"));
    let called = futures_executor::block_on(call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "manual",
        json!({}),
        CallOptions::default(),
    ))
    .unwrap();
    assert!(called["markdown"]
        .as_str()
        .unwrap()
        .starts_with("# concordd\n"));

    let typed = futures_executor::block_on(r.call_typed::<ManualOp<Ctx>>(
        &Ctx,
        op_registry::manual::ManualParams {
            out: None,
            json: false,
        },
        CallOptions::default(),
    ))
    .unwrap();
    assert!(matches!(
        typed,
        op_registry::manual::ManualOutput::Markdown { .. }
    ));
    let d = r.get("manual").unwrap();
    assert_eq!(d.kind, op_registry::OpKind::Local);
    assert!(d.access.reads && d.access.writes);
    assert!(d.text);
    assert_eq!(d.summary, "Show this tool's manual.");
}

#[test]
fn manual_includes_ops_merged_after_with_manual() {
    let mut r = with_manual();
    let mut later = Registry::<Ctx>::new();
    later.group("late", "Late ones.");
    later.push_op(
        OpSpec::<Filter, Item>::new("late arrival", "Arrives late."),
        nothing::<Filter>,
    );
    r.merge(later);
    let called = futures_executor::block_on(call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "manual",
        json!({}),
        CallOptions::default(),
    ))
    .unwrap();
    assert!(called["markdown"]
        .as_str()
        .unwrap()
        .contains("### concordd late arrival"));
}

#[test]
fn help_never_shows_env_value() {
    let _env = env_lock();
    std::env::set_var(HUB_ENV, "https://hub.example.test/SECRETVALUE");
    let r = with_manual();
    let text = render(&r, &sample_manual()).unwrap();
    assert!(!text.contains("SECRETVALUE"));
    assert!(!text.contains(&format!("{HUB_ENV}=")));
    let mut root = cli::command(&r);
    let help = root.render_long_help().to_string();
    assert!(help.contains("--hub"));
    assert!(!help.contains("SECRETVALUE"));
    assert!(!help.contains(&format!("{HUB_ENV}=")));
    std::env::remove_var(HUB_ENV);
}

#[test]
fn out_is_refused_off_the_command_line() {
    let r = with_manual();
    let dir = std::env::temp_dir().join(format!("oprm-off-cli-{}", std::process::id()));
    let file = dir.join("never.md");
    let failure = futures_executor::block_on(call_tool(
        &r,
        &Ctx,
        Audience::Agent,
        "manual",
        json!({"out": file.to_str().unwrap()}),
        CallOptions::default(),
    ))
    .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(failure.error()).code,
        "op_registry.usage.invalid"
    );
    let failure = futures_executor::block_on(r.call(
        "manual",
        json!({"out": file.to_str().unwrap()}),
        &Ctx,
        CallOptions::default(),
    ))
    .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(failure.error()).code,
        "op_registry.usage.invalid"
    );
    let err = futures_executor::block_on(r.call_typed::<ManualOp<Ctx>>(
        &Ctx,
        op_registry::manual::ManualParams {
            out: Some(file.clone()),
            json: false,
        },
        CallOptions::default(),
    ))
    .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "op_registry.usage.invalid"
    );
    assert!(!dir.exists(), "nothing is written off the command line");
}

#[test]
fn manual_json_has_audiences() {
    let mut r = fixture();
    let mut spec = OpSpec::<Filter, Item>::new("concept shared", "Shared.");
    spec.more_audiences = vec![Audience::Owner];
    r.push_op(spec, nothing::<Filter>);
    let json = render_json(&r);
    let op = json["ops"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == "concept_shared")
        .unwrap();
    assert_eq!(op["audiences"], serde_json::json!(["agent", "owner"]));
    assert_eq!(op["audience"], "agent");
}

#[cfg(feature = "schema")]
#[test]
fn manual_op_schema_root_is_object() {
    let r = with_manual();
    let schema = r.get("manual").unwrap().output_schema.as_ref().unwrap();
    assert_eq!(schema["type"], "object", "{schema}");
    let tool = tool_list(&r, Audience::Agent)
        .into_iter()
        .find(|t| t.name == "manual")
        .expect("the manual tool");
    assert!(tool.output_schema.is_some());
}

#[test]
fn manual_prints_needs_line() {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.set_bin("concordd", "1.2.3", "A concordance tool.");
    let mut spec = OpSpec::<Filter, Item>::new("init", "Initialise.");
    spec.needs_globals = vec!["hub"];
    r.push_op(spec, nothing::<Filter>);
    let text = render(&r, &sample_manual_empty()).unwrap();
    assert!(
        text.contains("Needs: --hub (or OPREG_MANUAL_HUB_UNIQUE)."),
        "{text}"
    );
}

fn sample_manual_empty() -> Manual {
    Manual {
        front_matter: None,
        sections: vec![],
    }
}

/// An op with a relation of every kind, for the "Rules:" lines.
#[derive(Clone, OpParams)]
struct Submit {
    /// The recipients.
    #[op(required_unless = "signed_file")]
    to: Vec<String>,
    /// A signed message file.
    #[op(conflicts_with = "to")]
    signed_file: Option<String>,
}

fn submit_registry(paths: Paths) -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.set_bin("concordd", "1.2.3", "A concordance tool.");
    let mut spec = OpSpec::<Submit, Item>::new("submit", "Submit a message.");
    spec.paths = paths;
    r.push_op(spec, nothing::<Submit>);
    r
}

#[test]
fn manual_prints_rules() {
    // A command: the rules end its help block, under "Rules:".
    let text = render(&submit_registry(Paths::ALL), &sample_manual_empty()).unwrap();
    assert!(
        text.contains(
            "Rules:\n  --to is required unless --signed-file is given\n  --signed-file conflicts with --to\n```"
        ),
        "{text}"
    );
    assert_eq!(text.matches("--to is required unless").count(), 1, "{text}");
    // An op off the command line has no help block: the manual prints them.
    let text = render(
        &submit_registry(Paths::of(&[Surface::Mcp])),
        &sample_manual_empty(),
    )
    .unwrap();
    assert!(
        text.contains(
            "Rules:\n- --to is required unless --signed-file is given\n- --signed-file conflicts with --to"
        ),
        "{text}"
    );
    // The JSON form carries them too.
    let json = render_json(&submit_registry(Paths::ALL));
    assert_eq!(
        json["ops"][0]["relations"],
        json!([
            "--to is required unless --signed-file is given",
            "--signed-file conflicts with --to"
        ])
    );
}

fn serve(_g: &G, _m: &clap::ArgMatches) -> BoxFuture<'static, anyhow::Result<u8>> {
    Box::pin(async { Ok(0) })
}

fn with_extra(r: &mut Registry<Ctx>) {
    r.push_extra(
        clap::Command::new("serve")
            .about("Run the server.")
            .long_about("Run the server.\n\nIt listens until stopped.")
            .arg(clap::Arg::new("port").long("port").help("The port.")),
        serve,
    );
}

fn run_fallback(_args: Vec<OsString>) -> BoxFuture<'static, anyhow::Result<u8>> {
    Box::pin(async { Ok(0) })
}

fn free_text(_g: &G, _args: Vec<OsString>) -> BoxFuture<'_, anyhow::Result<u8>> {
    Box::pin(async { Ok(0) })
}

/// A registry with one extra and a fallback of each kind the code has.
fn fallbacks() -> Vec<(&'static str, Registry<Ctx>)> {
    let mut flags_only = fixture();
    flags_only.fallback_flags_only("fix", "Fix things.\nMore fix text.", run_fallback);
    let mut before = fixture();
    before.fallback_flags_only_before_runtime("daemon", "Run the daemon.\nMore daemon text.");
    let mut text = fixture();
    text.fallback_free_text(
        "ask",
        "Ask the agent.\nMore ask text.",
        &[],
        false,
        free_text,
    );
    vec![("fix", flags_only), ("daemon", before), ("ask", text)]
}

#[test]
fn manual_lists_extra_and_free_text_fallback() {
    for (name, mut r) in fallbacks() {
        with_extra(&mut r);
        let text = render(&r, &sample_manual()).unwrap();
        assert_eq!(text.matches("## Other commands").count(), 1, "{text}");
        assert!(!text.contains("## Fallback"), "{text}");
        let section = text
            .split("## Other commands")
            .nth(1)
            .unwrap()
            .split("\n## ")
            .next()
            .unwrap();
        // The extra: name, about, long about and its own flag.
        assert!(section.contains("concordd serve"), "{section}");
        assert!(section.contains("It listens until stopped."), "{section}");
        assert!(section.contains("--port"), "{section}");
        // The fallback: name and full help, once.
        assert!(section.contains(&format!("### {name}")), "{section}");
        assert!(section.contains("More "), "{section}");
        assert_eq!(text.matches("More ").count(), 1, "{text}");
        // Each is marked.
        assert_eq!(
            section.matches("CLI only, not an op.").count(),
            2,
            "{section}"
        );
    }
    // Nothing to list: no section.
    let text = render(&fixture(), &sample_manual()).unwrap();
    assert!(!text.contains("## Other commands"), "{text}");
}

#[test]
fn manual_json_other_commands() {
    let (_, mut r) = fallbacks().remove(2);
    with_extra(&mut r);
    let json = render_json(&r);
    let other = json["otherCommands"].as_array().unwrap();
    assert_eq!(other.len(), 2, "{json}");
    assert_eq!(other[0]["name"], "serve");
    assert_eq!(other[0]["kind"], "extra");
    assert_eq!(other[0]["about"], "Run the server.");
    assert!(other[0]["longAbout"]
        .as_str()
        .unwrap()
        .contains("It listens until stopped."));
    assert_eq!(other[0]["cliOnly"], true);
    assert_eq!(other[1]["name"], "ask");
    assert_eq!(other[1]["kind"], "fallback");
    assert!(other[1]["help"]
        .as_str()
        .unwrap()
        .contains("More ask text."));
    // No other commands: an empty list, not a missing key.
    assert_eq!(render_json(&fixture())["otherCommands"], json!([]));
}
