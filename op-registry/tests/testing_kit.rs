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
    PageRequest, Paths, Registry,
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

/// A registry with a get, a list pair, a stream, a hidden op, a typed-only op
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
    typed.paths = Paths::TypedOnly;
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

const AGENT_OPS: [&str; 6] = [
    "concept_get",
    "concept_list",
    "concept_list_page",
    "status_watch",
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
        text.contains("status_watch on rest: missing from mount"),
        "{text}"
    );
    assert!(
        text.contains("concept_rich on rest: missing from mount"),
        "{text}"
    );
}

#[test]
fn parity_flags_typed_only_on_mcp() {
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
        text.contains("typed-only op concept_inner is in mount mcp-mount"),
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
fn help_warns_item_differs_from_get() {
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
    let warnings = assert_help_complete(&r, &rules());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("thing_list items differ from the output of thing_get"));
}

#[test]
fn help_matching_item_and_get_has_no_warning() {
    // The fixture's `concept get` and `concept list` share `Item`.
    assert!(assert_help_complete(&fixture(), &rules()).is_empty());
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
        let params = sample_params(&d.input_schema);
        let out = futures_executor::block_on(r.call(
            &d.name,
            params.clone(),
            &ctx,
            CallOptions::default(),
        ));
        // `TypedOnly` ops refuse the JSON path; every other op decodes.
        if d.paths == Paths::TypedOnly {
            continue;
        }
        if d.kind == op_registry::OpKind::Stream {
            continue;
        }
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
