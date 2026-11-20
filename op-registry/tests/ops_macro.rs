use futures_util::{future::BoxFuture, stream::BoxStream};
use op_registry::{
    Audience, CallOptions, Cursor, Op, OpKind, OpParams, Page, PageParams, PageRequest, Paths,
};
use serde::Serialize;
use serde_json::json;

pub struct Ctx;
impl op_registry::Resolve for Ctx {}

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

mod fixture {
    use super::*;

    op_registry::ops! {
        Ctx = Ctx;
        globals = ();
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
                            cap: 50, page_size: 10 },
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
    assert_eq!(d.long_help, "Longer help, shown by `--help`.\nA second line.");
    assert_eq!(r.get("key_keygen").unwrap().long_help, "");
    assert_eq!(r.pillar(), "fix");
    assert_eq!(d.exits.len(), 1);
    assert_eq!(d.exits[0].code, 3);
    assert!(d.text);
    assert_eq!(d.paths, Paths::All);
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
    assert!(!all.text && all.exits.is_empty());
    assert!(!page.text && page.exits.is_empty());

    let items = futures_executor::block_on(r.call_typed::<ops::ConceptList>(
        &Ctx,
        ListConcepts { tag: None },
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(items, vec![Concept { id: "a".into() }, Concept { id: "b".into() }]);

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
    assert_eq!(r.names().len(), 5);
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
        .call("concept_get", json!({"id": "z"}), &Ctx, CallOptions::default())
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "z"}));
    let out = r
        .call("concept_list_page", json!({"limit": 5}), &Ctx, CallOptions::default())
        .await
        .unwrap();
    assert_eq!(out["nextCursor"], "n1");
}
