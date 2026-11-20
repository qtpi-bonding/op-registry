//! The testing kit: parity, canonical, casing, help and `sample_params`.
#![cfg(feature = "testing")]
#![allow(dead_code)]

use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream, StreamExt};
use op_registry::testing::{
    assert_canonical, assert_casing, assert_help_complete, assert_parity, parity_report,
    sample_params, Casing, HelpRules, Mount, Reach,
};
use op_registry::{
    Access, Audience, CallOptions, CliContext, Cursor, OpGlobals, OpParams, OpSpec, Page,
    PageRequest, Paths, Registry, Surface,
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(OpGlobals)]
struct NoGlobals {}

struct Ctx;
impl op_registry::Resolve for Ctx {}
impl CliContext for Ctx {
    type Globals = NoGlobals;
    fn from_globals(_: &NoGlobals) -> anyhow::Result<Self> {
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
struct Rich {
    /// The name.
    name: String,
    /// How many.
    count: u32,
    /// A ratio.
    ratio: f64,
    /// A switch.
    on: bool,
    /// Tags.
    tags: Vec<String>,
}

#[derive(Clone, OpParams)]
struct Undocumented {
    flag_without_docs: Option<String>,
}

#[derive(Clone, OpParams)]
struct GetOther {
    /// An identifier.
    id: String,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
struct Item {
    item_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
struct Other {
    other_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Snake {
    item_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
struct WithMap {
    by_name: BTreeMap<String, String>,
}

fn item() -> Item {
    Item {
        item_id: "x".into(),
    }
}

fn get_op(_c: &Ctx, _p: Get, _o: CallOptions) -> BoxFuture<'_, anyhow::Result<Item>> {
    Box::pin(async { Ok(item()) })
}

fn page_op(_c: &Ctx, _p: Filter, _r: PageRequest) -> BoxFuture<'_, anyhow::Result<Page<Item>>> {
    Box::pin(async {
        Ok(Page {
            items: vec![],
            next_cursor: None,
        })
    })
}

fn page_other(_c: &Ctx, _p: Filter, _r: PageRequest) -> BoxFuture<'_, anyhow::Result<Page<Other>>> {
    Box::pin(async {
        Ok(Page {
            items: vec![],
            next_cursor: None,
        })
    })
}

fn rich_op(_c: &Ctx, _p: Rich, _o: CallOptions) -> BoxFuture<'_, anyhow::Result<Item>> {
    Box::pin(async { Ok(item()) })
}

fn filter_op<O: Serialize + op_registry::OutputSchema + Default + Send + 'static>(
    _c: &Ctx,
    _p: Filter,
    _o: CallOptions,
) -> BoxFuture<'_, anyhow::Result<O>> {
    Box::pin(async { Ok(O::default()) })
}

fn watch_op(
    _c: &Ctx,
    _p: Filter,
    _cursor: Option<Cursor>,
    _o: CallOptions,
) -> BoxFuture<'_, anyhow::Result<BoxStream<'_, anyhow::Result<Item>>>> {
    Box::pin(async { Ok(stream::iter(vec![Ok(item())]).boxed()) })
}

fn doctor(_g: &NoGlobals, _m: &clap::ArgMatches) -> BoxFuture<'static, u8> {
    Box::pin(async { 0 })
}

/// A registry with a get, a list pair, a stream, a hidden op, an op declared on typed only
/// and one extra.
fn fixture() -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.set_bin("fixd", "1.0.0", "A fixture.");
    r.group("concept", "Concepts in a collection.");
    r.group("status", "Status of things.");
    let mut get = OpSpec::<Get, Item>::new("concept get", "Show one concept.");
    get.access = Access::reads();
    r.push_op(get, get_op);
    let mut list = OpSpec::<Filter, Item>::new("concept list", "List concepts.");
    list.access = Access::reads();
    r.push_list(list, page_op);
    r.push_stream(
        OpSpec::<Filter, Item>::new("status watch", "Follow the status."),
        watch_op,
    );
    let mut hidden = OpSpec::<Get, Item>::new("concept purge", "Purge a concept.");
    hidden.hidden = true;
    r.push_op(hidden, get_op);
    let mut typed = OpSpec::<Get, Item>::new("concept inner", "Inner use only.");
    typed.paths = Paths::of(&[Surface::Typed]);
    r.push_op(typed, get_op);
    r.push_op(
        OpSpec::<Rich, Item>::new("concept rich", "Take every kind of field."),
        rich_op,
    );
    r.push_extra(
        clap::Command::new("doctor").about("Check the setup."),
        doctor,
    );
    r
}

const AGENT_OPS: [&str; 5] = [
    "concept_get",
    "concept_list",
    "concept_list_page",
    "concept_purge",
    "concept_rich",
];

fn panic_text(f: impl FnOnce()) -> String {
    let err =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).expect_err("expected a panic");
    err.downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

fn rules() -> HelpRules<'static> {
    HelpRules {
        banned_words: &["daemon"],
    }
}

#[test]
fn parity_passes_for_fixture() {
    let r = fixture();
    let mounts = [Mount {
        name: "rest",
        audience: Audience::Agent,
        ops: &AGENT_OPS,
    }];
    assert_parity(&r, &mounts);
}

#[test]
fn parity_reports_missing_mount_op() {
    let r = fixture();
    let ops = ["concept_get", "concept_list"];
    let mounts = [Mount {
        name: "rest",
        audience: Audience::Agent,
        ops: &ops,
    }];
    let text = panic_text(|| assert_parity(&r, &mounts));
    // Every missing op is named, not only the first.
    assert!(
        text.contains("concept_list_page on rest: missing from mount"),
        "{text}"
    );
    assert!(
        text.contains("concept_rich on rest: missing from mount"),
        "{text}"
    );
}

#[test]
fn parity_flags_undeclared_op_in_a_mount() {
    let r = fixture();
    let mut ops = AGENT_OPS.to_vec();
    ops.push("concept_inner");
    ops.push("concept_ghost");
    let mounts = [Mount {
        name: "mcp-mount",
        audience: Audience::Agent,
        ops: &ops,
    }];
    let text = panic_text(|| assert_parity(&r, &mounts));
    assert!(
        text.contains("op concept_inner is in mount mcp-mount but does not declare json"),
        "{text}"
    );
    assert!(
        text.contains("`concept_ghost`, which is not in the registry"),
        "{text}"
    );
}

#[test]
fn parity_lists_stream_reason_and_extra_reason() {
    let r = fixture();
    let mounts = [Mount {
        name: "rest",
        audience: Audience::Agent,
        ops: &AGENT_OPS,
    }];
    let lines = parity_report(&r, &mounts);
    let find = |name: &str, path: &str| {
        lines
            .iter()
            .find(|l| l.name == name && l.path == path)
            .unwrap_or_else(|| panic!("no line for {name} on {path}"))
            .reach
            .clone()
    };
    assert_eq!(
        find("status_watch", "mcp"),
        Reach::Absent {
            reason: "stream".into()
        }
    );
    assert_eq!(find("status_watch", "cli"), Reach::Reachable);
    assert_eq!(find("doctor", "cli"), Reach::Reachable);
    assert_eq!(
        find("doctor", "mcp"),
        Reach::Absent {
            reason: "not an op".into()
        }
    );
    // A hidden op is checked structurally and counts as reachable.
    assert_eq!(find("concept_purge", "mcp"), Reach::Reachable);
    assert_eq!(find("concept_list_page", "cli"), Reach::Reachable);
    assert_eq!(find("concept_get", "rest"), Reach::Reachable);
}

#[test]
fn canonical_reports_missing_and_extra() {
    let r = fixture();
    let mut names = r.names();
    assert_canonical(&r, &names);
    names.retain(|n| *n != "concept_get");
    names.push("concept_vanished");
    let text = panic_text(|| assert_canonical(&r, &names));
    assert!(
        text.contains("`concept_vanished` is canonical but not in the registry"),
        "{text}"
    );
    assert!(
        text.contains("`concept_get` is in the registry but not canonical"),
        "{text}"
    );
}

#[test]
fn casing_passes_for_fixture() {
    assert_casing(&fixture(), Casing::Camel);
}

#[test]
fn casing_flags_snake_output_key() {
    let mut r = fixture();
    r.push_op(
        OpSpec::<Get, Snake>::new("concept snake", "Returns snake keys."),
        |_c: &Ctx, _p: Get, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Snake>> {
            Box::pin(async {
                Ok(Snake {
                    item_id: "x".into(),
                })
            })
        },
    );
    let text = panic_text(|| assert_casing(&r, Casing::Camel));
    assert!(text.contains("concept_snake output schema"), "{text}");
    assert!(text.contains("`item_id`"), "{text}");
}

#[test]
fn casing_ignores_map_keys() {
    let mut r = fixture();
    r.push_op(
        OpSpec::<Get, WithMap>::new("concept map", "Returns a map."),
        |_c: &Ctx, _p: Get, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<WithMap>> {
            Box::pin(async {
                Ok(WithMap {
                    by_name: BTreeMap::from([("snake_key".to_owned(), "v".to_owned())]),
                })
            })
        },
    );
    assert_casing(&r, Casing::Camel);
}

#[test]
fn help_passes_for_fixture() {
    let warnings = assert_help_complete(&fixture(), &rules());
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn help_flags_missing_doc() {
    let mut r = fixture();
    r.push_op(
        OpSpec::<Undocumented, Item>::new("concept bare", "A bare op."),
        |_c: &Ctx, _p: Undocumented, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
            Box::pin(async { Ok(item()) })
        },
    );
    r.push_op(OpSpec::<Get, Item>::new("concept empty", ""), get_op);
    let text = panic_text(|| {
        assert_help_complete(&r, &rules());
    });
    assert!(
        text.contains("--flag-without-docs on `concept bare` has no help"),
        "{text}"
    );
    assert!(text.contains("`concept empty` has no summary"), "{text}");
}

#[test]
fn help_flags_two_wordings_for_one_flag() {
    let mut r = fixture();
    r.push_op(
        OpSpec::<GetOther, Item>::new("concept other", "Another get."),
        |_c: &Ctx, _p: GetOther, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
            Box::pin(async { Ok(item()) })
        },
    );
    let text = panic_text(|| {
        assert_help_complete(&r, &rules());
    });
    assert!(text.contains("flag --id has 2 wordings"), "{text}");
    assert!(text.contains("\"The concept id.\""), "{text}");
    assert!(text.contains("\"An identifier.\""), "{text}");
}

#[test]
fn help_flags_home_path_and_banned_word() {
    let mut r = fixture();
    r.push_op(
        OpSpec::<Get, Item>::new("concept path", "Reads /Users/gp/notes from the Daemon."),
        get_op,
    );
    let text = panic_text(|| {
        assert_help_complete(&r, &rules());
    });
    assert!(text.contains("contains `/Users/`"), "{text}");
    assert!(text.contains("the banned word `daemon`"), "{text}");
}

#[test]
fn help_warns_list_verb_not_list() {
    let mut r = fixture();
    r.group("agent", "Agents.");
    r.push_op(
        OpSpec::<Filter, Item2>::new("agent list", "List agents."),
        filter_op::<Item2>,
    );
    let warnings = assert_help_complete(&r, &rules());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("agent_list"), "{warnings:?}");
    assert!(warnings[0].contains("not a list op"), "{warnings:?}");
}

#[test]
fn help_unpaged_silences_warning() {
    let mut r = fixture();
    r.group("agent", "Agents.");
    let mut spec = OpSpec::<Filter, Item2>::new("agent list", "List agents.");
    spec.unpaged = true;
    r.push_op(spec, filter_op::<Item2>);
    assert!(assert_help_complete(&r, &rules()).is_empty());
}

#[test]
fn list_get_warning_fires_when_items_differ() {
    let mut r = fixture();
    r.group("thing", "Things.");
    r.push_op(
        OpSpec::<Get, Item>::new("thing get", "Show a thing."),
        get_op,
    );
    r.set_pillar("fix");
    r.push_list(
        OpSpec::<Filter, Other>::new("thing list", "List things."),
        page_other,
    );
    let list = r.get("thing_list").unwrap();
    let schema = list.output_schema.as_ref().unwrap();
    assert_eq!(schema["type"], "object", "{schema}");
    assert!(
        schema.pointer("/properties/items/items").is_some(),
        "{schema}"
    );
    let warnings = assert_help_complete(&r, &rules());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("thing_list items differ from the output of thing_get"));
}

#[test]
fn list_get_warning_quiet_when_items_match() {
    // The fixture's `concept get` and `concept list` share `Item`.
    let r = fixture();
    let schema = r
        .get("concept_list")
        .unwrap()
        .output_schema
        .clone()
        .unwrap();
    assert!(
        schema.pointer("/properties/items/items").is_some(),
        "{schema}"
    );
    assert!(assert_help_complete(&r, &rules()).is_empty());
}

/// An output type for ops that need a second shape.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Item2 {
    ok: bool,
}

#[test]
fn sample_params_satisfies_from_json() {
    let r = fixture();
    let ctx = Ctx;
    for d in r.descriptors() {
        // Ops that do not declare `json` refuse the JSON path and streams are not unary.
        if !d.paths.contains(Surface::Json) || d.kind == op_registry::OpKind::Stream {
            continue;
        }
        let params = sample_params(&d.input_schema);
        let out = futures_executor::block_on(r.call(
            &d.name,
            params.clone(),
            &ctx,
            CallOptions::default(),
        ));
        assert!(
            out.is_ok(),
            "{}: {params} did not decode: {:?}",
            d.name,
            out.err()
        );
    }
}

#[test]
fn sample_params_fills_required_by_type() {
    let schema = Rich::input_schema();
    let sample = sample_params(&schema);
    assert_eq!(sample["name"], "x");
    assert_eq!(sample["count"], 1);
    assert_eq!(sample["ratio"], 1.0);
    // A switch and a repeated flag are optional, so they are left out.
    assert!(sample.get("on").is_none());
    assert!(sample.get("tags").is_none());
}

#[test]
fn sample_params_handles_enum_one_of_and_dependents() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "mode": {"type": "string", "enum": ["fast", "slow"]},
            "a": {"type": "string"},
            "b": {"type": "string"},
            "c": {"type": "integer"},
            "d": {"type": "boolean"},
            "m": {"type": "object", "additionalProperties": {"type": "string"}},
            "list": {"type": "array", "items": {"type": "string"}},
            "n": {"type": "number"},
        },
        "required": ["mode", "m", "list", "n"],
        "oneOf": [{"required": ["a"]}, {"required": ["b"]}],
        "dependentRequired": {"a": ["c"], "c": ["d"]},
    });
    let sample = sample_params(&schema);
    assert_eq!(
        sample,
        serde_json::json!({"mode": "fast", "m": {}, "list": [], "n": 1.0, "a": "x", "c": 1, "d": false})
    );
}

#[test]
fn parity_mount_without_stream_passes_and_reports_stream() {
    let r = fixture();
    assert!(!AGENT_OPS.contains(&"status_watch"));
    let mounts = [Mount {
        name: "rest",
        audience: Audience::Agent,
        ops: &AGENT_OPS,
    }];
    assert_parity(&r, &mounts);
    let line = parity_report(&r, &mounts)
        .into_iter()
        .find(|l| l.name == "status_watch" && l.path == "rest")
        .unwrap();
    assert_eq!(
        line.reach,
        Reach::Absent {
            reason: "stream".into()
        }
    );
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Inner {
    snake_key: String,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
struct MapOfInner {
    by_name: BTreeMap<String, Inner>,
}

#[test]
fn casing_flags_snake_key_inside_map_value() {
    let mut r = fixture();
    r.push_op(
        OpSpec::<Get, MapOfInner>::new("concept nested", "Returns a map of structs."),
        |_c: &Ctx, _p: Get, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<MapOfInner>> {
            Box::pin(async {
                Ok(MapOfInner {
                    by_name: BTreeMap::new(),
                })
            })
        },
    );
    let text = panic_text(|| assert_casing(&r, Casing::Camel));
    assert!(text.contains("concept_nested output schema"), "{text}");
    assert!(text.contains("`snake_key`"), "{text}");
}

// ---- Task 19: snapshot, exit table, codes, secrets, refs, offline ----

use coded_error::{CodeEntry, ErrorClass};
use op_registry::manual::Manual;
use op_registry::testing::{
    assert_error_codes, assert_exit_table, assert_local_ops_offline, assert_manual_snapshot,
    assert_unknown_ref_not_found, check_exit_table, check_manual_snapshot, strict_codes, Fixture,
};
use op_registry::{Ref, Resolved, Scope};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("op-registry-t19-{}-{name}", std::process::id()))
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "fix.concept")]
enum ConceptError {
    #[error("no concept")]
    #[coded(code = "not_found", class = Logical)]
    NotFound,
    #[error("rogue")]
    #[coded(code = "rogue", class = Logical)]
    Rogue,
}

fn failing(
    err: fn() -> anyhow::Error,
) -> impl for<'a> Fn(&'a Ctx, Get, CallOptions) -> BoxFuture<'a, anyhow::Result<Item>> {
    move |_c, _p, _o| Box::pin(async move { Err(err()) })
}

/// An op that fails with a declared code, one with an undeclared code, and
/// one with an uncoded error; `concept get` succeeds and declares exit 4.
fn codes_fixture() -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.set_bin("fixd", "1.0.0", "A fixture.");
    r.group("concept", "Concepts.");
    let mut get = OpSpec::<Get, Item>::new("concept get", "Show one concept.");
    get.exits = vec![(4, "drifted")];
    get.errors = vec![CodeEntry {
        code: "fix.concept.not_found",
        class: Some(ErrorClass::Logical),
    }];
    r.push_op(get, get_op);
    let mut declared = OpSpec::<Get, Item>::new("concept missing", "Fails declared.");
    declared.errors = get_errors();
    r.push_op(
        declared,
        failing(|| coded_error::into_anyhow(ConceptError::NotFound)),
    );
    r.push_op(
        OpSpec::<Get, Item>::new("concept rogue", "Fails undeclared."),
        failing(|| coded_error::into_anyhow(ConceptError::Rogue)),
    );
    r.push_op(
        OpSpec::<Get, Item>::new("concept plain", "Fails uncoded."),
        failing(|| anyhow::anyhow!("plain")),
    );
    r
}

fn get_errors() -> Vec<CodeEntry> {
    vec![CodeEntry {
        code: "fix.concept.not_found",
        class: Some(ErrorClass::Logical),
    }]
}

fn call_json(r: &Registry<Ctx>, op: &str) -> anyhow::Result<serde_json::Value> {
    futures_executor::block_on(r.call(op, json!({"id": "x"}), &Ctx, CallOptions::default()))
        .map_err(op_registry::CallFailure::into_inner)
}

#[test]
fn snapshot_mismatch_reports_and_update_rewrites() {
    let r = fixture();
    let manual = Manual::default();
    let path = temp_path("snapshot.md");
    let _ = std::fs::remove_file(&path);

    let missing = check_manual_snapshot(&r, &manual, &path, false).unwrap_err();
    assert!(missing.contains("UPDATE_SNAPSHOTS=1"), "{missing}");
    assert!(!path.exists());

    check_manual_snapshot(&r, &manual, &path, true).unwrap();
    assert!(path.exists());
    check_manual_snapshot(&r, &manual, &path, false).unwrap();

    let fresh = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        fresh.replacen("Show one concept.", "Show a concept.", 1),
    )
    .unwrap();
    let err = check_manual_snapshot(&r, &manual, &path, false).unwrap_err();
    assert!(err.contains("UPDATE_SNAPSHOTS=1"), "{err}");
    assert!(err.contains("Show a concept."), "{err}");
    assert!(err.contains("Show one concept."), "{err}");
    // The panicking wrapper agrees.
    let text = panic_text(|| assert_manual_snapshot(&r, &manual, &path));
    assert!(text.contains("UPDATE_SNAPSHOTS=1"), "{text}");

    check_manual_snapshot(&r, &manual, &path, true).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), fresh);
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn snapshot_line_ending_only_diff_terminates() {
    let r = fixture();
    let manual = Manual::default();
    let path = temp_path("snapshot-eol.md");
    check_manual_snapshot(&r, &manual, &path, true).unwrap();
    let fresh = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, fresh.trim_end_matches('\n')).unwrap();
    let err = check_manual_snapshot(&r, &manual, &path, false).unwrap_err();
    assert!(
        err.contains("line endings or the trailing newline"),
        "{err}"
    );
    std::fs::write(&path, fresh.replace('\n', "\r\n")).unwrap();
    let err = check_manual_snapshot(&r, &manual, &path, false).unwrap_err();
    assert!(
        err.contains("line endings or the trailing newline"),
        "{err}"
    );
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn exit_table_check_passes_and_flags_missing_code() {
    let r = codes_fixture();
    assert_exit_table(&r, &Manual::default());

    let rendered = op_registry::manual::render(&r, &Manual::default()).unwrap();
    assert!(check_exit_table(&r, &rendered).is_empty());

    let no_code = rendered.replace("fix.concept.not_found", "gone");
    let problems = check_exit_table(&r, &no_code);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("concept_get") && p.contains("fix.concept.not_found")),
        "{problems:?}"
    );
    let no_exit = rendered.replace("- 4: drifted", "- 4: x");
    let problems = check_exit_table(&r, &no_exit);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("concept_get") && p.contains("exit 4")),
        "{problems:?}"
    );
    let no_row = rendered.replace("| 75 | transient |", "");
    let problems = check_exit_table(&r, &no_row);
    assert!(problems.iter().any(|p| p.contains("75")), "{problems:?}");
}

#[test]
fn exit_table_flags_declared_exit_out_of_range() {
    let mut r = codes_fixture();
    let mut spec = OpSpec::<Get, Item>::new("concept wide", "Declares exit 99.");
    spec.exits = vec![(99, "too big"), (2, "taken")];
    r.push_op(spec, get_op);
    let text = panic_text(|| assert_exit_table(&r, &Manual::default()));
    assert!(text.contains("concept_wide"), "{text}");
    assert!(text.contains("99") && text.contains("3..=63"), "{text}");
    assert!(text.contains("exit 2"), "{text}");
}

#[test]
fn strict_codes_panics_on_undeclared_code() {
    let mut r = codes_fixture();
    strict_codes(&mut r);
    // Declared codes, success and library codes pass through untouched.
    call_json(&r, "concept_get").unwrap();
    let err = call_json(&r, "concept_missing").unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "fix.concept.not_found"
    );
    let err = futures_executor::block_on(r.call(
        "concept_get",
        json!({"id": "x", "bogus": 1}),
        &Ctx,
        CallOptions::default(),
    ))
    .unwrap_err();
    assert!(coded_error::facts_anyhow(err.error())
        .code
        .starts_with("op_registry."));
    // An undeclared code names the op and the code.
    let text = panic_text(|| {
        let _ = call_json(&r, "concept_rogue");
    });
    assert!(text.contains("concept_rogue"), "{text}");
    assert!(text.contains("fix.concept.rogue"), "{text}");
    // An uncoded error panics too.
    let text = panic_text(|| {
        let _ = call_json(&r, "concept_plain");
    });
    assert!(text.contains("concept_plain"), "{text}");
    assert!(text.contains("uncoded"), "{text}");
}

#[test]
fn strict_codes_off_by_default() {
    let r = codes_fixture();
    assert!(call_json(&r, "concept_rogue").is_err());
    assert!(call_json(&r, "concept_plain").is_err());
}

#[test]
fn error_codes_report_lists_offender() {
    let r = codes_fixture();
    let args = json!({"id": "x"});
    let good = [
        Fixture {
            op: "concept_get",
            args: args.clone(),
        },
        Fixture {
            op: "concept_missing",
            args: args.clone(),
        },
    ];
    futures_executor::block_on(assert_error_codes(&r, &Ctx, &good));

    let fixtures = [
        Fixture {
            op: "concept_missing",
            args: args.clone(),
        },
        Fixture {
            op: "concept_rogue",
            args: args.clone(),
        },
        Fixture {
            op: "concept_plain",
            args,
        },
    ];
    let text = panic_text(|| futures_executor::block_on(assert_error_codes(&r, &Ctx, &fixtures)));
    // Every offender is reported, not only the first.
    assert!(
        text.contains("concept_rogue") && text.contains("fix.concept.rogue"),
        "{text}"
    );
    assert!(
        text.contains("concept_plain") && text.contains("uncoded"),
        "{text}"
    );
    assert!(!text.contains("concept_missing"), "{text}");
}

#[tokio::test]
async fn error_codes_check_stream_items() {
    let mut r = codes_fixture();
    r.push_stream(
        OpSpec::<Filter, Item>::new("concept follow", "Follow."),
        |_c: &Ctx,
         _p: Filter,
         _cur: Option<Cursor>,
         _o: CallOptions|
         -> BoxFuture<'_, anyhow::Result<BoxStream<'_, anyhow::Result<Item>>>> {
            Box::pin(async {
                Ok(stream::iter(vec![
                    Err(coded_error::into_anyhow(ConceptError::Rogue)),
                    Ok(item()),
                ])
                .boxed())
            })
        },
    );
    let fixtures = [Fixture {
        op: "concept_follow",
        args: json!({}),
    }];
    let text = async_panic_text(assert_error_codes(&r, &Ctx, &fixtures)).await;
    assert!(
        text.contains("concept_follow") && text.contains("fix.concept.rogue"),
        "{text}"
    );
}

fn stream_registry<S>(make: fn() -> S) -> Registry<Ctx>
where
    S: futures_util::Stream<Item = anyhow::Result<Item>> + Send + 'static,
{
    let mut r = codes_fixture();
    r.push_stream(
        OpSpec::<Filter, Item>::new("concept follow", "Follow."),
        move |_c: &Ctx,
              _p: Filter,
              _cur: Option<Cursor>,
              _o: CallOptions|
              -> BoxFuture<'_, anyhow::Result<BoxStream<'_, anyhow::Result<Item>>>> {
            Box::pin(async move { Ok(make().boxed()) })
        },
    );
    r
}

fn follow() -> [Fixture<'static>; 1] {
    [Fixture {
        op: "concept_follow",
        args: json!({}),
    }]
}

async fn async_panic_text(f: impl std::future::Future<Output = ()>) -> String {
    use futures_util::FutureExt;
    let err = std::panic::AssertUnwindSafe(f)
        .catch_unwind()
        .await
        .expect_err("expected a panic");
    err.downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

#[tokio::test(start_paused = true)]
async fn kit_stream_that_never_yields_is_reported_after_timeout() {
    let r = stream_registry(stream::pending);
    let text = async_panic_text(assert_error_codes(&r, &Ctx, &follow())).await;
    assert!(text.contains("did not end within 10 s"), "{text}");
    assert!(text.contains("concept_follow"), "{text}");
}

#[tokio::test]
async fn kit_endless_stream_is_cut_after_first_item() {
    let r = stream_registry(|| {
        stream::iter(0..).map(|i| {
            assert!(i < 1000, "kit drained the stream");
            Ok(item())
        })
    });
    assert_error_codes(&r, &Ctx, &follow()).await;
}

#[test]
fn kit_stream_without_runtime_reports_problem() {
    let r = stream_registry(|| stream::iter(vec![Ok(item())]));
    let text = panic_text(|| futures_executor::block_on(assert_error_codes(&r, &Ctx, &follow())));
    assert!(text.contains("need a tokio runtime with time"), "{text}");
}

#[tokio::test]
async fn kit_calls_carry_max_retries_zero() {
    use std::sync::{Arc, Mutex};
    let seen: Arc<Mutex<Vec<Option<u32>>>> = Arc::default();
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.group("tool", "Tools.");
    let sink = seen.clone();
    r.push_local(
        OpSpec::<Get, Item>::new("tool peek", "Records its options."),
        move |_c: &Ctx, _p: Get, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
            sink.lock().unwrap().push(o.max_retries);
            Box::pin(async { Ok(item()) })
        },
    );
    op_registry::testing::assert_local_ops_offline(&r, &Ctx, &[]).await;
    assert_eq!(*seen.lock().unwrap(), vec![Some(0)]);
}

#[test]
fn with_max_retries_sets_field() {
    assert_eq!(CallOptions::default().max_retries, None);
    assert_eq!(
        CallOptions::default().with_max_retries(3).max_retries,
        Some(3)
    );
}

// ---- secrets ----

#[cfg(feature = "secrets")]
mod secrets_kit {
    use super::*;
    use op_registry::testing::{assert_secrets_not_echoed, SecretSentinel};
    use op_registry::SecretFile;

    #[derive(OpParams)]
    struct Vault {
        /// The key file.
        key: SecretFile,
        /// An API token.
        #[op(secret)]
        token: Option<String>,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Leak {
        Nothing,
        Token,
        KeyFile,
    }

    #[derive(Serialize)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    struct Said {
        said: String,
    }

    fn vault_registry(leak: Leak) -> Registry<Ctx> {
        let mut r = Registry::<Ctx>::new();
        r.set_pillar("fix");
        r.set_bin("fixd", "1.0.0", "A fixture.");
        r.group("vault", "Vaults.");
        r.push_op(
            OpSpec::<Vault, Said>::new("vault open", "Open the vault."),
            move |_c: &Ctx, p: Vault, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Said>> {
                Box::pin(async move {
                    let said = match leak {
                        Leak::Nothing => "opened".to_owned(),
                        Leak::Token => p.token.unwrap_or_default(),
                        Leak::KeyFile => String::from_utf8_lossy(p.key.bytes()).into_owned(),
                    };
                    Ok(Said { said })
                })
            },
        );
        r.push_op(OpSpec::<Get, Item>::new("vault look", "Look."), get_op);
        r.push_op(
            OpSpec::<Pinned, Said>::new("vault pin", "Pin the vault."),
            |_c: &Ctx, _p: Pinned, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Said>> {
                Box::pin(async move {
                    Ok(Said {
                        said: "pinned".to_owned(),
                    })
                })
            },
        );
        r
    }

    /// A numeric secret: the sentinel text is a bad value for it on both
    /// the CLI and the JSON path.
    #[derive(OpParams)]
    struct Pinned {
        /// A PIN.
        #[op(secret)]
        pin: Option<u64>,
    }

    const SENTINEL: &str = "SENTINEL-token-4471";
    const KEY_LINES: &str = "KEYLINE-alpha-9921\nKEYLINE-beta-9922\n";

    fn sentinel_file(name: &str) -> PathBuf {
        let path = temp_path(name);
        std::fs::write(&path, KEY_LINES).unwrap();
        path
    }

    #[test]
    fn secrets_sentinel_never_echoed_passes() {
        let key_file = sentinel_file("key-passes.pem");
        let sentinel = SecretSentinel {
            value: SENTINEL,
            key_file: &key_file,
        };
        let r = vault_registry(Leak::Nothing);
        futures_executor::block_on(assert_secrets_not_echoed(&r, None, &sentinel));
        futures_executor::block_on(assert_secrets_not_echoed(
            &r,
            Some(&Manual::default()),
            &sentinel,
        ));
        std::fs::remove_file(&key_file).unwrap();
    }

    #[test]
    fn secrets_check_catches_leaky_op() {
        let key_file = sentinel_file("key-leaky.pem");
        let sentinel = SecretSentinel {
            value: SENTINEL,
            key_file: &key_file,
        };
        let token = vault_registry(Leak::Token);
        let text = panic_text(|| {
            futures_executor::block_on(assert_secrets_not_echoed(&token, None, &sentinel))
        });
        assert!(text.contains("vault_open"), "{text}");
        assert!(text.contains("stdout"), "{text}");
        assert!(text.contains("sentinel value"), "{text}");

        let file = vault_registry(Leak::KeyFile);
        let text = panic_text(|| {
            futures_executor::block_on(assert_secrets_not_echoed(&file, None, &sentinel))
        });
        assert!(text.contains("vault_open"), "{text}");
        assert!(text.contains("KEYLINE-beta-9922"), "{text}");
        std::fs::remove_file(&key_file).unwrap();
    }

    #[test]
    fn secrets_check_catches_secret_in_help() {
        let key_file = sentinel_file("key-help.pem");
        let r = vault_registry(Leak::Nothing);
        // "Open the vault." is help text: a sentinel found there is reported for
        // the help, the manual and the schemas' descriptions.
        let sentinel = SecretSentinel {
            value: "Open the vault.",
            key_file: &key_file,
        };
        let text = panic_text(|| {
            futures_executor::block_on(assert_secrets_not_echoed(&r, None, &sentinel))
        });
        assert!(text.contains("--help of `vault open`"), "{text}");
        assert!(text.contains("the manual"), "{text}");
        std::fs::remove_file(&key_file).unwrap();
    }
}

// ---- unknown refs ----

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "fix.build")]
enum BuildError {
    #[error("no build {0}")]
    #[coded(code = "not_found", class = Logical)]
    NotFound(String),
}

#[derive(OpParams)]
struct GetBuild {
    /// The build.
    #[op(subject = "build")]
    id: Ref,
}

#[derive(OpParams)]
struct Scoped {
    /// The build.
    #[op(subject = "build", scope = ["blueprint"])]
    id: Ref,
    /// The blueprint.
    #[op(kind = "blueprint")]
    blueprint: Ref,
}

struct RefCtx {
    strict: bool,
    seen: Mutex<Vec<(String, String)>>,
}

impl RefCtx {
    fn new(strict: bool) -> Self {
        Self {
            strict,
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl op_registry::Resolve for RefCtx {
    fn resolve<'a>(
        &'a self,
        kind: &'a str,
        r: &'a Ref,
        _scope: &'a Scope,
    ) -> BoxFuture<'a, anyhow::Result<Resolved>> {
        Box::pin(async move {
            let raw = r.as_raw().to_owned();
            self.seen
                .lock()
                .unwrap()
                .push((kind.to_owned(), raw.clone()));
            if kind == "blueprint" && raw == "bp" {
                return Ok(Resolved::Id("bp".into()));
            }
            if self.strict {
                Err(coded_error::into_anyhow(BuildError::NotFound(raw)))
            } else {
                Ok(Resolved::Id(raw))
            }
        })
    }
}

fn ref_registry() -> Registry<RefCtx> {
    let mut r = Registry::<RefCtx>::new();
    r.set_pillar("fix");
    r.group("build", "Builds.");
    r.push_op(
        OpSpec::<GetBuild, Item>::new("build get", "Show a build."),
        |_c: &RefCtx, _p: GetBuild, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
            // A lookup that finds nothing and returns a value anyway.
            Box::pin(async { Ok(item()) })
        },
    );
    r.push_op(
        OpSpec::<Scoped, Item>::new("build scoped", "Show a scoped build."),
        |_c: &RefCtx, _p: Scoped, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
            Box::pin(async { Ok(item()) })
        },
    );
    // Not a subject: a plain string called `id` is skipped.
    r.push_op(
        OpSpec::<Get, Item>::new("build plain", "Plain id."),
        |_c: &RefCtx, _p: Get, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
            Box::pin(async { Ok(item()) })
        },
    );
    r
}

#[test]
fn unknown_ref_check_passes_with_resolver_and_fails_with_null_returning_op() {
    let r = ref_registry();
    let strict = RefCtx::new(true);
    let overrides = [Fixture {
        op: "build_scoped",
        args: json!({"blueprint": "bp"}),
    }];
    futures_executor::block_on(assert_unknown_ref_not_found(&r, &strict, &overrides));
    let seen = strict.seen.lock().unwrap().clone();
    // The override's value reached the resolver, and the subject got a
    // random 16-hex ref.
    assert!(
        seen.contains(&("blueprint".to_owned(), "bp".to_owned())),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|(k, raw)| k == "build"
            && raw.len() == 16
            && raw.chars().all(|c| c.is_ascii_hexdigit())),
        "{seen:?}"
    );
    assert!(!seen.iter().any(|(_, raw)| raw == "x"), "{seen:?}");

    // A context that resolves anything lets the op return a value: reported.
    let lax = RefCtx::new(false);
    let text = panic_text(|| {
        futures_executor::block_on(assert_unknown_ref_not_found(&r, &lax, &overrides))
    });
    assert!(
        text.contains("build_get") && text.contains("build_scoped"),
        "{text}"
    );
    assert!(!text.contains("build_plain"), "{text}");
}

#[test]
fn unknown_ref_check_flags_wrong_code_and_class() {
    struct Wrong;
    impl op_registry::Resolve for Wrong {
        fn resolve<'a>(
            &'a self,
            _k: &'a str,
            _r: &'a Ref,
            _s: &'a Scope,
        ) -> BoxFuture<'a, anyhow::Result<Resolved>> {
            Box::pin(async { Err(anyhow::anyhow!("no resolver")) })
        }
    }
    let mut r = Registry::<Wrong>::new();
    r.group("build", "Builds.");
    r.push_op(
        OpSpec::<GetBuild, Item>::new("build get", "Show a build."),
        |_c: &Wrong, _p: GetBuild, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
            Box::pin(async { Ok(item()) })
        },
    );
    let text =
        panic_text(|| futures_executor::block_on(assert_unknown_ref_not_found(&r, &Wrong, &[])));
    assert!(text.contains("build_get"), "{text}");
    assert!(text.contains("not_found"), "{text}");
}

// ---- offline ----

struct Conn;
impl op_registry::Resolve for Conn {}
impl Conn {
    fn connect(&self) -> ! {
        panic!("connector used");
    }
}

fn offline_registry(touch: bool) -> Registry<Conn> {
    let mut r = Registry::<Conn>::new();
    r.group("tool", "Tools.");
    r.push_local(
        OpSpec::<Get, Item>::new("tool clean", "Runs without a connection."),
        |_c: &Conn, _p: Get, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
            Box::pin(async { Ok(item()) })
        },
    );
    r.push_local(
        OpSpec::<Get, Item>::new("tool fails", "Fails without a connection."),
        |_c: &Conn, _p: Get, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
            Box::pin(async { Err(anyhow::anyhow!("nope")) })
        },
    );
    if touch {
        r.push_local(
            OpSpec::<Get, Item>::new("tool connect", "Opens a connection."),
            |c: &Conn, _p: Get, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
                Box::pin(async move { c.connect() })
            },
        );
        // A unary op may connect; it is not run.
        r.push_op(
            OpSpec::<Get, Item>::new("tool remote", "Needs a connection."),
            |c: &Conn, _p: Get, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
                Box::pin(async move { c.connect() })
            },
        );
    }
    r
}

#[test]
fn local_offline_catches_connect() {
    futures_executor::block_on(assert_local_ops_offline(
        &offline_registry(false),
        &Conn,
        &[],
    ));
    let text = panic_text(|| {
        futures_executor::block_on(assert_local_ops_offline(
            &offline_registry(true),
            &Conn,
            &[],
        ))
    });
    assert!(text.contains("tool_connect"), "{text}");
    assert!(text.contains("connector used"), "{text}");
    assert!(!text.contains("tool_remote"), "{text}");
    assert!(!text.contains("tool_clean"), "{text}");
}

// ---- KIT-1: the kit runs every op on a surface it declares ----

mod kit_surfaces {
    use super::*;
    use op_registry::testing::{assert_unknown_ref_not_found, Fixture};
    use op_registry::{CallHook, CallInfo, CallPath};
    use std::sync::{Arc, Mutex};

    /// Records the path and caller of every call.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<(String, CallPath, Option<op_registry::Caller>)>>);

    impl CallHook for Recorder {
        fn around<'a>(&'a self, info: &CallInfo, call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()> {
            self.0.lock().unwrap().push((
                info.name.clone(),
                info.path.clone(),
                info.caller.clone(),
            ));
            call
        }
    }

    fn ran(hook: &Recorder) -> Vec<(String, CallPath, Option<op_registry::Caller>)> {
        hook.0.lock().unwrap().clone()
    }

    fn only(surface: Surface) -> Paths {
        Paths::of(&[surface])
    }

    fn running(
        body: fn() -> anyhow::Result<Item>,
    ) -> impl for<'a> Fn(&'a Ctx, Get, CallOptions) -> BoxFuture<'a, anyhow::Result<Item>> {
        move |_c, _p, _o| Box::pin(std::future::ready(body()))
    }

    /// One op named `tool op` on `paths` whose body runs `body`.
    fn one_op(
        paths: Paths,
        local: bool,
        body: fn() -> anyhow::Result<Item>,
        hook: Arc<Recorder>,
    ) -> Registry<Ctx> {
        let mut r = Registry::<Ctx>::new();
        r.set_pillar("fix");
        r.group("tool", "Tools.");
        r.set_call_hook(hook);
        let mut spec = OpSpec::<Get, Item>::new("tool op", "Runs.");
        spec.paths = paths;
        if local {
            r.push_local(spec, running(body));
        } else {
            r.push_op(spec, running(body));
        }
        r
    }

    fn rogue() -> anyhow::Result<Item> {
        Err(coded_error::into_anyhow(super::ConceptError::Rogue))
    }

    fn not_a_value() -> anyhow::Result<Item> {
        Ok(item())
    }

    fn args() -> [Fixture<'static>; 1] {
        [Fixture {
            op: "tool_op",
            args: json!({"id": "x"}),
        }]
    }

    #[test]
    fn kit_runs_cli_only_op_in_error_codes() {
        let hook = Arc::<Recorder>::default();
        let r = one_op(only(Surface::Cli), false, rogue, hook.clone());
        let text = panic_text(|| {
            futures_executor::block_on(assert_error_codes(&r, &Ctx, &args()));
        });
        assert!(
            text.contains("tool_op") && text.contains("fix.concept.rogue"),
            "{text}"
        );
        assert!(!text.contains("wrong_path"), "{text}");
        assert_eq!(ran(&hook)[0].1, CallPath::Cli);
    }

    #[test]
    fn kit_runs_cli_only_op_offline() {
        let hook = Arc::<Recorder>::default();
        let r = one_op(only(Surface::Cli), true, not_a_value, hook.clone());
        futures_executor::block_on(assert_local_ops_offline(&r, &Ctx, &args()));
        let calls = ran(&hook);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, CallPath::Cli);
    }

    #[test]
    fn kit_runs_cli_only_op_for_unknown_ref() {
        let hook = Arc::<Recorder>::default();
        let mut r = Registry::<RefCtx>::new();
        r.set_pillar("fix");
        r.group("tool", "Tools.");
        let hook2 = hook.clone();
        r.set_call_hook(hook2);
        let mut spec = OpSpec::<GetBuild, Item>::new("tool op", "Runs.");
        spec.paths = only(Surface::Cli);
        r.push_op(
            spec,
            |_c: &RefCtx, _p: GetBuild, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Item>> {
                Box::pin(async { Ok(item()) })
            },
        );
        // The op answers a value, so the check fails; what matters here is
        // that it ran, on the CLI.
        let text = panic_text(|| {
            futures_executor::block_on(assert_unknown_ref_not_found(&r, &RefCtx::new(false), &[]))
        });
        assert!(!text.contains("wrong_path"), "{text}");
        let calls = ran(&hook);
        assert!(!calls.is_empty(), "{text}");
        assert!(calls.iter().all(|c| c.1 == CallPath::Cli));
    }

    #[test]
    fn kit_runs_mcp_only_op_with_a_caller() {
        let hook = Arc::<Recorder>::default();
        let r = one_op(only(Surface::Mcp), false, not_a_value, hook.clone());
        futures_executor::block_on(assert_error_codes(&r, &Ctx, &args()));
        let calls = ran(&hook);
        assert_eq!(calls[0].1, CallPath::Mcp);
        assert_eq!(
            calls[0].2,
            Some(op_registry::Caller("op-registry-kit".to_owned()))
        );
    }

    #[test]
    fn kit_prefers_json_when_declared() {
        let hook = Arc::<Recorder>::default();
        let r = one_op(Paths::ALL, false, not_a_value, hook.clone());
        futures_executor::block_on(assert_error_codes(&r, &Ctx, &args()));
        assert_eq!(
            ran(&hook)[0].1,
            CallPath::Transport("op-registry-kit".to_owned())
        );
    }

    #[test]
    fn kit_skips_typed_only_op_and_names_it() {
        let hook = Arc::<Recorder>::default();
        let mut r = one_op(
            only(Surface::Typed),
            false,
            || panic!("body ran"),
            hook.clone(),
        );
        // A failing op beside it makes the runner panic, so its report shows.
        r.push_op(
            OpSpec::<Get, Item>::new("tool bad", "Fails undeclared."),
            running(rogue),
        );
        let both = [
            Fixture {
                op: "tool_op",
                args: json!({"id": "x"}),
            },
            Fixture {
                op: "tool_bad",
                args: json!({"id": "x"}),
            },
        ];
        let text = panic_text(|| futures_executor::block_on(assert_error_codes(&r, &Ctx, &both)));
        assert!(
            text.contains("assert_error_codes: tool_op: typed only: not run"),
            "{text}"
        );
        assert!(text.contains("tool_bad"), "{text}");
        futures_executor::block_on(assert_local_ops_offline(&r, &Ctx, &args()));
        futures_executor::block_on(assert_unknown_ref_not_found(&r, &Ctx, &[]));
        assert!(ran(&hook).iter().all(|c| c.0 != "tool_op"));
    }
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Cat {
    lives: u32,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Dog {
    bones: u32,
}

/// An untagged either-or output whose branches are `$ref`s.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
enum Pet {
    Cat(Cat),
    Dog(Dog),
}

fn pet_get(_c: &Ctx, _p: Get, _o: CallOptions) -> BoxFuture<'_, anyhow::Result<Pet>> {
    Box::pin(async { Ok(Pet::Cat(Cat { lives: 9 })) })
}

fn pet_page(_c: &Ctx, _p: Filter, _r: PageRequest) -> BoxFuture<'_, anyhow::Result<Page<Pet>>> {
    Box::pin(async {
        Ok(Page {
            items: vec![],
            next_cursor: None,
        })
    })
}

#[test]
fn tagged_enum_list_get_no_spurious_warning() {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.group("pet", "Pets.");
    r.push_op(OpSpec::<Get, Pet>::new("pet get", "Show a pet."), pet_get);
    r.push_list(
        OpSpec::<Filter, Pet>::new("pet list", "List pets."),
        pet_page,
    );
    let warnings = assert_help_complete(&r, &rules());
    assert!(warnings.is_empty(), "{warnings:?}");
}
