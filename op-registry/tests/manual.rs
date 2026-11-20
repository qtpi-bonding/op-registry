//! The generated manual, `render_json` and the `manual` op.
#![cfg(feature = "manual")]
#![allow(dead_code)]

use coded_error::{CodeEntry, ErrorClass};
use futures_util::future::BoxFuture;
use op_registry::cli::{self, Io, NoHooks};
use op_registry::manual::{render, render_json, FrontMatter, Guide, Manual, ManualOp, Section};
use op_registry::mcp::{call_tool, tool_list};
use op_registry::{
    Access, Audience, CallOptions, CliContext, OpGlobals, OpParams, OpSpec, Page, PageRequest,
    Paths, Registry,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::ffi::OsString;

const HUB_ENV: &str = "OPREG_MANUAL_HUB_UNIQUE";

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
    let mut get = OpSpec::<Get, Item>::new("concept get", "Show one concept.");
    get.access = Access::reads().with_network();
    get.exits = vec![(4, "records drifted")];
    get.errors = vec![
        CodeEntry {
            code: "fix.concept.not_found",
            class: Some(ErrorClass::Logical),
        },
        CodeEntry {
            code: "fix.concept.drift",
            class: None,
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
fn hidden_and_typed_only_not_in_manual() {
    let mut r = fixture();
    let mut hidden = OpSpec::<Filter, Item>::new("concept purge", "Purge it.");
    hidden.hidden = true;
    r.push_op(hidden, nothing::<Filter>);
    let mut typed = OpSpec::<Filter, Item>::new("concept inner", "Inner use.");
    typed.paths = Paths::TypedOnly;
    r.push_op(typed, nothing::<Filter>);
    let text = render(&r, &sample_manual()).unwrap();
    assert!(!text.contains("concept purge"));
    assert!(!text.contains("concept inner"));
    // The JSON lists every op.
    let json = render_json(&r);
    let names: Vec<&str> = json["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"concept_purge") && names.contains(&"concept_inner"));
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
    assert_eq!(get.get("output_schema").is_some(), cfg!(feature = "schema"));

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
