use futures_util::{future::BoxFuture, stream::BoxStream};
use op_registry::{
    Audience, CallOptions, Cursor, Op, OpKind, OpParams, Page, PageParams, PageRequest, Paths,
};
use serde::Serialize;
use serde_json::json;

pub struct Ctx;
impl op_registry::Resolve for Ctx {}

#[derive(op_registry::OpGlobals)]
pub struct Globals {}

impl op_registry::CliContext for Ctx {
    type Globals = Globals;
    fn from_globals(_: &Globals) -> anyhow::Result<Self> {
        Ok(Ctx)
    }
}

#[derive(Clone, Debug, PartialEq, OpParams)]
pub struct GetConcept {
    #[op(subject = "concept")]
    pub id: op_registry::Ref,
}

#[derive(Clone, Debug, PartialEq, OpParams)]
pub struct Keygen {
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, OpParams)]
pub struct ListConcepts {
    pub tag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, OpParams)]
pub struct Watch {
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Concept {
    pub id: String,
}

impl op_registry::OkExit for Concept {}
impl op_registry::RenderText for Concept {
    fn render_text(&self) -> String {
        self.id.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct KeyPath {
    pub path: String,
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "fix.inner")]
pub enum InnerError {
    #[error("bad")]
    #[coded(code = "bad", class = Logical)]
    Bad,
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "fix.concept")]
pub enum ConceptError {
    #[error("not found")]
    #[coded(code = "not_found", class = Logical)]
    NotFound,
    #[error(transparent)]
    #[coded(transparent, catalog)]
    Inner(InnerError),
}

fn concept_fields(p: &GetConcept) -> Vec<(&'static str, String)> {
    vec![("concept_id", p.id.as_raw().to_owned())]
}

async fn concept_get(_c: &Ctx, p: GetConcept, _o: CallOptions) -> anyhow::Result<Concept> {
    Ok(Concept {
        id: p.id.id().to_owned(),
    })
}

async fn keygen(_c: &Ctx, _p: Keygen, _o: CallOptions) -> anyhow::Result<KeyPath> {
    Ok(KeyPath { path: "k".into() })
}

async fn concept_page(
    _c: &Ctx,
    _p: ListConcepts,
    req: PageRequest,
) -> anyhow::Result<Page<Concept>> {
    let items = match req.cursor.as_deref() {
        None => vec![Concept { id: "a".into() }],
        Some(_) => vec![Concept { id: "b".into() }],
    };
    let next_cursor = req.cursor.is_none().then(|| "n1".to_owned());
    Ok(Page { items, next_cursor })
}

fn concept_watch(
    _c: &Ctx,
    _p: Watch,
    _cur: Option<Cursor>,
    _o: CallOptions,
) -> BoxFuture<'_, anyhow::Result<BoxStream<'_, anyhow::Result<Concept>>>> {
    Box::pin(async {
        Ok(Box::pin(futures_util::stream::iter(vec![Ok(Concept {
            id: "w".into(),
        })])) as BoxStream<'_, anyhow::Result<Concept>>)
    })
}

#[derive(Clone, Debug, PartialEq, OpParams)]
pub struct PlanParams {
    pub goal: Option<String>,
}

#[derive(Clone, Debug, PartialEq, OpParams)]
pub struct Index {
    pub path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Plan {
    pub goal: String,
}

async fn plan(_c: &Ctx, p: PlanParams, _o: CallOptions) -> anyhow::Result<Plan> {
    Ok(Plan {
        goal: p.goal.unwrap_or_default(),
    })
}

async fn index(_c: &Ctx, _p: Index, _o: CallOptions) -> anyhow::Result<Plan> {
    Ok(Plan {
        goal: "indexed".into(),
    })
}

mod fixture {
    use super::*;

    op_registry::ops! {
        Ctx = Ctx;
        globals = Globals;
        pillar = "fix";
        bin = "fixd"; version = "1.2.3"; about = "A fixture.";
        resolve_server = ["concept", "tag"];
        groups = {
            "concept" => "Concepts in a collection.",
            "key" => "Keys.",
        };
        /// Show one concept.
        ///
        /// Longer help, shown by `--help`.
        /// A second line.
        "concept get" => { params: GetConcept, output: Concept,
                           access: reads, audience: agent,
                           op: concept_get,
                           errors: [ConceptError], fields: concept_fields,
                           exits: { 3 => "not found" }, text: true },
        /// Make a new key pair on this machine.
        "key keygen" => { params: Keygen, output: KeyPath,
                          access: [writes, spawns], audience: owner,
                          local: keygen, hidden: true },
        /// List concepts.
        "concept list" => { params: ListConcepts, item: Concept,
                            access: reads, audience: agent, list: concept_page,
                            cap: 50, page_size: 10, text: true },
        /// Make a plan. Bare verb whose output type is named like its marker.
        "plan" => { params: PlanParams, output: Plan,
                    access: reads, audience: agent, op: plan },
        /// Index. Bare verb whose params type is named like its marker.
        "index" => { params: Index, output: Plan,
                     access: reads, audience: agent, op: index },
        /// Watch a concept.
        "concept watch" => { params: Watch, output: Concept,
                             access: reads, audience: dev,
                             stream: concept_watch, cursor: true },
    }
}

use fixture::{ops, registry};

#[test]
fn doc_comment_is_summary_and_long_help() {
    let r = registry();
    let d = r.get("concept_get").unwrap();
    assert_eq!(d.summary, "Show one concept.");
    assert_eq!(
        d.long_help,
        "Longer help, shown by `--help`.\nA second line."
    );
    assert_eq!(r.get("key_keygen").unwrap().long_help, "");
    assert_eq!(r.pillar(), "fix");
    assert_eq!(d.exits.len(), 1);
    assert_eq!(d.exits[0].code, 3);
    assert!(d.text);
    assert_eq!(d.paths, Paths::ALL);
}

#[test]
fn derived_names_and_marker_types() {
    assert_eq!(<ops::ConceptGet as Op>::NAME, "concept_get");
    assert_eq!(<ops::KeyKeygen as Op>::NAME, "key_keygen");
    assert_eq!(<ops::ConceptWatch as Op>::NAME, "concept_watch");
    assert_eq!(<ops::ConceptList as Op>::NAME, "concept_list");
    assert_eq!(<ops::ConceptListPage as Op>::NAME, "concept_list_page");
    let r = registry();
    assert_eq!(
        r.names(),
        vec![
            "concept_get",
            "key_keygen",
            "concept_list",
            "concept_list_page",
            "plan",
            "index",
            "concept_watch"
        ]
    );
}

#[test]
fn call_typed_through_marker_returns_typed_output() {
    let r = registry();
    let out = futures_executor::block_on(r.call_typed::<ops::ConceptGet>(
        &Ctx,
        GetConcept {
            id: op_registry::Ref::new("c1"),
        },
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(out, Concept { id: "c1".into() });
    let out = futures_executor::block_on(r.call_typed::<ops::KeyKeygen>(
        &Ctx,
        Keygen { label: None },
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(out.path, "k");
}

#[test]
fn list_entry_generates_pair_and_markers() {
    let r = registry();
    let all = r.get("concept_list").unwrap();
    let page = r.get("concept_list_page").unwrap();
    assert_eq!(all.kind, OpKind::ListAll);
    assert_eq!(page.kind, OpKind::ListPage);
    assert!(all.text && all.exits.is_empty());
    assert!(page.text && page.exits.is_empty());

    let items = futures_executor::block_on(r.call_typed::<ops::ConceptList>(
        &Ctx,
        ListConcepts { tag: None },
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(
        items,
        vec![Concept { id: "a".into() }, Concept { id: "b".into() }]
    );

    let page = futures_executor::block_on(r.call_typed::<ops::ConceptListPage>(
        &Ctx,
        PageParams {
            inner: ListConcepts { tag: None },
            limit: 5,
            cursor: None,
        },
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.next_cursor.as_deref(), Some("n1"));
}

#[test]
fn errors_codes_flattened_from_catalog() {
    let r = registry();
    let codes: Vec<&str> = r
        .get("concept_get")
        .unwrap()
        .errors
        .iter()
        .map(|e| e.code.as_str())
        .collect();
    assert_eq!(codes, vec!["fix.concept.not_found", "fix.inner.bad"]);
    assert!(r.get("key_keygen").unwrap().errors.is_empty());
}

#[test]
fn access_list_sets_flags() {
    let r = registry();
    let a = r.get("key_keygen").unwrap();
    assert!(a.access.writes && a.access.spawns);
    assert!(!a.access.reads && !a.access.network && !a.access.destructive);
    assert_eq!(a.audience, Audience::Owner);
    assert!(a.hidden);
    let g = r.get("concept_get").unwrap();
    assert!(g.access.reads && !g.access.writes);
    assert_eq!(g.audience, Audience::Agent);
}

#[test]
fn local_op_kind_local() {
    let r = registry();
    assert_eq!(r.get("key_keygen").unwrap().kind, OpKind::Local);
    assert_eq!(r.get("concept_get").unwrap().kind, OpKind::Unary);
    assert_eq!(r.get("concept_watch").unwrap().kind, OpKind::Stream);
    assert_eq!(r.get("concept_watch").unwrap().audience, Audience::Dev);
}

#[test]
fn groups_registered_with_summary() {
    // A noun path without a declared group panics in `push`, so the
    // registry building at all proves both groups were registered first.
    let r = registry();
    assert_eq!(r.names().len(), 7);
}

#[test]
fn resolve_server_kinds_applied() {
    let r = registry();
    // `Ctx` has the default resolver, which errors: the call only succeeds
    // because the registry passes `concept` refs through raw.
    let out = futures_executor::block_on(r.call_typed::<ops::ConceptGet>(
        &Ctx,
        GetConcept {
            id: op_registry::Ref::new("raw-id"),
        },
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(out.id, "raw-id");
}

#[test]
fn bin_info_is_stored() {
    let r = registry();
    assert_eq!(r.bin(), "fixd");
    assert_eq!(r.version(), "1.2.3");
    assert_eq!(r.about(), "A fixture.");
}

#[tokio::test]
async fn json_call_runs_the_wrapped_functions() {
    let r = registry();
    let out = r
        .call(
            "concept_get",
            json!({"id": "z"}),
            &Ctx,
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "z"}));
    let out = r
        .call(
            "concept_list_page",
            json!({"limit": 5}),
            &Ctx,
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(out["nextCursor"], "n1");
}

#[test]
fn marker_named_like_a_user_type_does_not_shadow_it() {
    // `ops::Plan` and `ops::Index` share their names with the user's `Plan`
    // output and `Index` params; the markers still resolve to the user types.
    let r = registry();
    let out: Plan = futures_executor::block_on(r.call_typed::<ops::Plan>(
        &Ctx,
        PlanParams {
            goal: Some("g".into()),
        },
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(out, Plan { goal: "g".into() });
    let out = futures_executor::block_on(r.call_typed::<ops::Index>(
        &Ctx,
        Index { path: None },
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(out.goal, "indexed");
}

mod cfg_fixture {
    use super::*;

    // `dup` is declared twice. If the cfg-off entry still produced a marker
    // (or a push block), this module would not compile or `registry()` would
    // panic on a duplicate name.
    op_registry::ops! {
        Ctx = Ctx;
        /// Off.
        #[cfg(any())]
        "dup" => { params: PlanParams, output: Plan,
                   access: reads, audience: agent, op: missing_function },
        /// On.
        #[cfg(all())]
        "dup" => { params: PlanParams, output: Plan,
                   access: reads, audience: agent, op: plan },
        /// Gone.
        #[cfg(any())]
        "gone" => { params: PlanParams, item: Plan,
                    access: reads, audience: agent, list: missing_function },
        /// Always.
        "always" => { params: Index, output: Plan,
                      access: reads, audience: agent, op: index,
                      globals: [gait_dir, other] },
    }
}

#[test]
fn cfg_off_entry_absent_and_no_marker() {
    let r = cfg_fixture::registry();
    assert_eq!(r.names(), vec!["dup", "always"]);
    assert!(r.get("gone").is_none() && r.get("gone_page").is_none());
    assert_eq!(r.get("dup").unwrap().summary, "On.");
}

#[test]
fn cfg_on_entry_present() {
    assert_eq!(<cfg_fixture::ops::Dup as Op>::NAME, "dup");
    assert_eq!(<cfg_fixture::ops::Always as Op>::NAME, "always");
}

#[test]
fn entry_globals_become_needs_flags() {
    let r = cfg_fixture::registry();
    assert_eq!(
        r.get("always").unwrap().needs_globals,
        vec!["gait-dir".to_owned(), "other".to_owned()]
    );
    assert!(r.get("dup").unwrap().needs_globals.is_empty());
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "fix.common")]
pub enum CommonError {
    #[error("denied")]
    #[coded(code = "denied", class = Logical)]
    Denied,
    #[error("busy")]
    #[coded(code = "busy", class = Transient)]
    Busy,
}

mod common_fixture {
    use super::*;

    async fn denied(_c: &Ctx, _p: Index, _o: CallOptions) -> anyhow::Result<Plan> {
        Err(coded_error::into_anyhow(CommonError::Busy))
    }

    op_registry::ops! {
        Ctx = Ctx;
        pillar = "fix";
        bin = "fixd"; version = "1.0.0"; about = "A fixture.";
        errors = [CommonError];
        groups = { "concept" => "Concepts." };
        /// Fails with a header code the entry does not list.
        "plan" => { params: Index, output: Plan,
                    access: reads, audience: agent, op: denied },
        /// Lists a header code on its own too, and has an error exit.
        "concept get" => { params: GetConcept, output: Concept,
                           access: reads, audience: agent, op: concept_get,
                           errors: [ConceptError, CommonError],
                           exits: { 3 => "busy" on "fix.common.busy" } },
        /// A list whose error exit reaches both halves.
        "concept list" => { params: ListConcepts, item: Concept,
                            access: reads, audience: agent, list: concept_page,
                            exits: { 4 => "busy" on "fix.common.busy" } },
    }
}

#[test]
fn header_errors_are_common_not_own() {
    let r = common_fixture::registry();
    let common: Vec<&str> = r.common_errors().iter().map(|e| e.code.as_str()).collect();
    assert_eq!(common, vec!["fix.common.denied", "fix.common.busy"]);
    assert!(r.get("plan").unwrap().errors.is_empty());
}

#[cfg(feature = "testing")]
#[test]
fn header_code_accepted_by_strict_for_every_op() {
    let mut r = common_fixture::registry();
    op_registry::testing::strict_codes(&mut r);
    // `plan` declares no errors of its own: the header code is accepted.
    let err = futures_executor::block_on(r.call("plan", json!({}), &Ctx, CallOptions::default()))
        .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(err.error()).code,
        "fix.common.busy"
    );
}

#[test]
fn code_in_header_and_entry_listed_once() {
    let r = common_fixture::registry();
    let common = r
        .common_errors()
        .iter()
        .filter(|e| e.code == "fix.common.busy")
        .count();
    assert_eq!(common, 1);
    #[cfg(feature = "manual")]
    {
        let text = op_registry::manual::render(&r, &Default::default()).unwrap();
        assert_eq!(text.matches("| fix.common.busy |").count(), 1, "{text}");
    }
}

#[test]
fn list_error_exit_applies_to_both_halves() {
    let r = common_fixture::registry();
    for name in ["concept_list", "concept_list_page"] {
        let d = r.get(name).unwrap();
        assert_eq!(d.exits.len(), 1, "{name}");
        assert_eq!(d.exits[0].code, 4);
        assert_eq!(d.exits[0].on.as_deref(), Some("fix.common.busy"));
    }
    let get = r.get("concept_get").unwrap();
    assert_eq!(get.exits[0].on.as_deref(), Some("fix.common.busy"));
}
