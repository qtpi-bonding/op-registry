//! List pairs: follow-all with a cap, the paged op, `PageParams`.

use coded_error::ErrorClass;
use futures_util::future::BoxFuture;
use op_registry::{
    CallOptions, OpKind, OpParams, OpSpec, Page, PageParams, PageRequest, ParseCx, Registry,
};
use serde::Serialize;
use serde_json::json;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Concept {
    id: String,
}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct ListConcepts {
    /// Only concepts with this tag.
    tag: Option<String>,
}

/// Serves `pages` by cursor: none is page 0, `cN` is page N. With `stuck`
/// every page names `c1` as the next cursor.
#[derive(Default)]
struct Ctx {
    pages: Vec<Vec<&'static str>>,
    stuck: bool,
    /// Every page is empty and names a fresh next cursor.
    endless_empty: bool,
    seen: Mutex<Vec<PageRequest>>,
}
impl op_registry::Resolve for Ctx {}

fn serve(
    ctx: &Ctx,
    _p: ListConcepts,
    req: PageRequest,
) -> BoxFuture<'_, anyhow::Result<Page<Concept>>> {
    ctx.seen.lock().unwrap().push(req.clone());
    let index = match req.cursor.as_deref() {
        None => 0,
        Some(c) => c[1..].parse::<usize>().unwrap(),
    };
    Box::pin(async move {
        let items = ctx
            .pages
            .get(index)
            .map(|p| {
                p.iter()
                    .map(|id| Concept {
                        id: (*id).to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let next_cursor = if ctx.endless_empty {
            Some(format!("c{}", index + 1))
        } else if ctx.stuck {
            Some("c1".to_owned())
        } else if index + 1 < ctx.pages.len() {
            Some(format!("c{}", index + 1))
        } else {
            None
        };
        Ok(Page { items, next_cursor })
    })
}

fn registry(configure: impl FnOnce(&mut OpSpec<ListConcepts, Concept>)) -> Registry<Ctx> {
    let mut r: Registry<Ctx> = Registry::new();
    r.set_pillar("fix");
    r.group("concept", "Concepts.");
    let mut spec = OpSpec::<ListConcepts, Concept>::new("concept list", "List concepts.");
    configure(&mut spec);
    r.push_list(spec, serve);
    r
}

fn ctx(pages: &[&[&'static str]]) -> Ctx {
    Ctx {
        pages: pages.iter().map(|p| p.to_vec()).collect(),
        ..Ctx::default()
    }
}

async fn call(
    r: &Registry<Ctx>,
    c: &Ctx,
    name: &str,
    args: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    r.call(name, args, c, CallOptions::default())
        .await
        .map_err(op_registry::CallFailure::into_inner)
}

#[tokio::test]
async fn follow_all_returns_items_object_across_pages() {
    let r = registry(|s| s.page_size = 2);
    let c = ctx(&[&["a", "b"], &["c", "d"], &["e"]]);
    let out = call(&r, &c, "concept_list", json!({})).await.unwrap();
    assert_eq!(
        out,
        json!({"items": [{"id":"a"},{"id":"b"},{"id":"c"},{"id":"d"},{"id":"e"}]})
    );
    let seen = c.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen.iter().all(|req| req.limit == 2));
    assert_eq!(seen[0].cursor, None);
    assert_eq!(seen[2].cursor.as_deref(), Some("c2"));
}

#[tokio::test]
async fn follow_all_over_cap_is_too_many() {
    let r = registry(|s| s.cap = 5);
    let c = ctx(&[&["a", "b", "c"], &["d", "e", "f"], &["g", "h", "i"]]);
    let err = call(&r, &c, "concept_list", json!({})).await.unwrap_err();
    let facts = coded_error::facts_anyhow(&err);
    assert_eq!(facts.code, "fix.concept.too_many");
    assert_eq!(facts.class, ErrorClass::Logical);
    assert!(err.to_string().contains("--limit"), "{err}");
    assert!(err.to_string().contains("concept_list"), "{err}");
}

#[tokio::test]
async fn follow_all_never_truncates() {
    // Exactly the cap is fine and returns every item; one more fails.
    let r = registry(|s| s.cap = 6);
    let c = ctx(&[&["a", "b", "c"], &["d", "e", "f"]]);
    let out = call(&r, &c, "concept_list", json!({})).await.unwrap();
    assert_eq!(out["items"].as_array().unwrap().len(), 6);
    let c = ctx(&[&["a", "b", "c"], &["d", "e", "f"], &["g"]]);
    assert!(call(&r, &c, "concept_list", json!({})).await.is_err());
}

#[tokio::test]
async fn page_op_returns_items_and_next_cursor() {
    let r = registry(|_| {});
    let c = ctx(&[&["a"], &["b", "c"], &["d"]]);
    let out = call(
        &r,
        &c,
        "concept_list_page",
        json!({"limit": 2, "cursor": "c1"}),
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        json!({"items":[{"id":"b"},{"id":"c"}],"nextCursor":"c2"})
    );
    assert_eq!(
        c.seen.lock().unwrap()[0],
        PageRequest {
            limit: 2,
            cursor: Some("c1".into())
        }
    );
}

#[tokio::test]
async fn empty_list_is_empty_array_and_empty_page() {
    let r = registry(|_| {});
    let c = ctx(&[]);
    assert_eq!(
        call(&r, &c, "concept_list", json!({})).await.unwrap(),
        json!({"items": []})
    );
    assert_eq!(
        call(&r, &c, "concept_list_page", json!({"limit": 5}))
            .await
            .unwrap(),
        json!({"items":[],"nextCursor":null})
    );
}

#[tokio::test]
async fn repeated_cursor_is_coded() {
    let r = registry(|_| {});
    let c = Ctx {
        pages: vec![vec!["a"], vec!["b"]],
        stuck: true,
        ..Ctx::default()
    };
    let err = call(&r, &c, "concept_list", json!({})).await.unwrap_err();
    assert_eq!(err.to_string(), "page cursor c1 repeated");
    let facts = coded_error::facts_anyhow(&err);
    assert_eq!(facts.code, "op_registry.list.cursor_repeated");
    assert_eq!(facts.class, ErrorClass::Logical);
}

#[tokio::test]
async fn empty_pages_forever_end_with_too_many() {
    let r = registry(|s| s.cap = 3);
    let c = Ctx {
        endless_empty: true,
        ..Ctx::default()
    };
    let err = call(&r, &c, "concept_list", json!({})).await.unwrap_err();
    assert_eq!(coded_error::facts_anyhow(&err).code, "fix.concept.too_many");
    // Cap 3: the fourth empty page is the one that fails.
    assert_eq!(c.seen.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn follow_all_uses_follow_page() {
    let r = registry(|s| {
        s.page_size = 2;
        s.follow_page = Some(500);
    });
    let c = ctx(&[&["a"], &["b"]]);
    call(&r, &c, "concept_list", json!({})).await.unwrap();
    assert!(c.seen.lock().unwrap().iter().all(|req| req.limit == 500));
    // The page half takes its limit as given.
    let c = ctx(&[&["a"]]);
    call(&r, &c, "concept_list_page", json!({"limit": 2}))
        .await
        .unwrap();
    assert_eq!(c.seen.lock().unwrap()[0].limit, 2);
}

fn one_word_registry() -> Registry<Ctx> {
    let mut r: Registry<Ctx> = Registry::new();
    r.set_pillar("fix");
    let mut spec = OpSpec::<ListConcepts, Concept>::new("graph-search", "Search the graph.");
    spec.cap = 2;
    r.push_list(spec, serve);
    r
}

#[tokio::test]
async fn one_word_list_registers_and_caps() {
    let r = one_word_registry();
    assert!(r.get("graph_search").is_some());
    assert!(r.get("graph_search_page").is_some());
    let c = ctx(&[&["a", "b"], &["c"]]);
    let err = call(&r, &c, "graph_search", json!({})).await.unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "fix.graph_search.too_many"
    );
}

#[cfg(feature = "testing")]
#[test]
fn one_word_list_cap_code_is_accepted_by_assert_error_codes() {
    use op_registry::testing::{assert_error_codes, Fixture};
    let r = one_word_registry();
    let c = ctx(&[&["a", "b"], &["c"]]);
    futures_executor::block_on(assert_error_codes(
        &r,
        &c,
        &[Fixture {
            op: "graph_search",
            args: json!({}),
        }],
    ));
}

#[test]
fn pair_names_and_list_pair_links() {
    let r = registry(|_| {});
    assert_eq!(r.names(), vec!["concept_list", "concept_list_page"]);
    let all = r.get("concept_list").unwrap();
    let page = r.get("concept_list_page").unwrap();
    assert_eq!(all.kind, OpKind::ListAll);
    assert_eq!(page.kind, OpKind::ListPage);
    assert_eq!(all.list_pair.as_deref(), Some("concept_list_page"));
    assert_eq!(page.list_pair.as_deref(), Some("concept_list"));
    assert_eq!(all.summary, page.summary);
    assert_eq!(all.path, page.path);
    #[cfg(feature = "schema")]
    {
        let all_schema = all.output_schema.as_ref().unwrap();
        assert_eq!(all_schema["type"], "object");
        assert_eq!(all_schema["properties"]["items"]["type"], "array");
        let page_schema = page.output_schema.as_ref().unwrap();
        assert!(page_schema["properties"]["nextCursor"].is_object());
    }
}

#[test]
fn page_schema_requires_limit() {
    let r = registry(|_| {});
    let page = &r.get("concept_list_page").unwrap().input_schema;
    assert_eq!(page["properties"]["limit"]["type"], "integer");
    assert_eq!(page["properties"]["cursor"]["type"], "string");
    assert!(page["properties"]["tag"].is_object());
    assert_eq!(page["required"], json!(["limit"]));
    let all = &r.get("concept_list").unwrap().input_schema;
    assert!(all["properties"].get("limit").is_none());
}

#[test]
fn page_params_roundtrip_json_and_cli() {
    let from_json = PageParams::<ListConcepts>::from_json(
        json!({"tag": "x", "limit": 7, "cursor": "c9"}),
        &mut ParseCx::default(),
    )
    .unwrap();
    assert_eq!(from_json.inner.tag.as_deref(), Some("x"));
    assert_eq!(from_json.limit, 7);
    assert_eq!(from_json.cursor.as_deref(), Some("c9"));

    let cmd = PageParams::<ListConcepts>::add_args(clap::Command::new("t"));
    let matches = cmd
        .clone()
        .try_get_matches_from(["t", "--tag", "x", "--limit", "7", "--cursor", "c9"])
        .unwrap();
    let from_cli =
        PageParams::<ListConcepts>::from_matches(&matches, &mut ParseCx::default()).unwrap();
    assert_eq!(from_cli.inner, from_json.inner);
    assert_eq!((from_cli.limit, from_cli.cursor), (7, Some("c9".into())));

    let no_cursor = cmd.try_get_matches_from(["t", "--limit", "3"]).unwrap();
    let p = PageParams::<ListConcepts>::from_matches(&no_cursor, &mut ParseCx::default()).unwrap();
    assert_eq!((p.limit, p.cursor, p.inner.tag), (3, None, None));

    let fields: Vec<_> = PageParams::<ListConcepts>::fields()
        .iter()
        .map(|f| f.rust_name)
        .collect();
    assert_eq!(fields, ["tag", "limit", "cursor"]);

    let err = PageParams::<ListConcepts>::from_json(json!({"tag": "x"}), &mut ParseCx::default())
        .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "op_registry.usage.missing"
    );
}

#[test]
#[should_panic(expected = "set_pillar before push_list")]
fn list_without_pillar_panics() {
    let mut r: Registry<Ctx> = Registry::new();
    r.group("concept", "Concepts.");
    r.push_list(
        OpSpec::<ListConcepts, Concept>::new("concept list", "List."),
        serve,
    );
}

#[test]
#[should_panic(expected = "page_size must be > 0")]
fn list_with_zero_page_size_panics() {
    let _ = registry(|s| s.page_size = 0);
}

#[test]
fn list_pair_halves_carry_no_text_or_exits() {
    let r = registry(|s| {
        s.render_text = Some(|_| String::new());
        s.ok_exit = Some(|_| 3);
        s.exits = vec![(3, "x")];
    });
    for name in ["concept_list", "concept_list_page"] {
        let d = r.get(name).unwrap();
        assert!(!d.text, "{name}");
        assert!(d.exits.is_empty(), "{name}");
    }
}

// --- OUT-1: arrays are `{"items": [...]}` on every JSON surface ---

#[derive(op_registry::OpGlobals)]
struct NoGlobals {}

impl op_registry::CliContext for Ctx {
    type Globals = NoGlobals;
    fn from_globals(_globals: &NoGlobals) -> anyhow::Result<Self> {
        Ok(Ctx {
            pages: vec![vec!["a", "b"], vec!["c"]],
            ..Ctx::default()
        })
    }
}

fn run_cli(args: &[&str]) -> serde_json::Value {
    let argv = std::iter::once("app")
        .chain(args.iter().copied())
        .map(std::ffi::OsString::from)
        .collect();
    let (io, cap) = op_registry::cli::Io::memory(b"", false, false);
    let code = futures_executor::block_on(op_registry::cli::run_with_io(
        &registry(|s| s.page_size = 2),
        argv,
        &op_registry::cli::NoHooks,
        io,
    ));
    assert_eq!(code, 0, "{}", cap.stderr());
    serde_json::from_str(&cap.stdout()).unwrap()
}

#[test]
fn cli_follow_all_prints_items_object() {
    assert_eq!(
        run_cli(&["concept", "list"]),
        json!({"items": [{"id":"a"},{"id":"b"},{"id":"c"}]})
    );
}

#[test]
fn page_half_prints_items_and_next_cursor() {
    assert_eq!(
        run_cli(&["concept", "list", "--limit", "2"]),
        json!({"items": [{"id":"a"},{"id":"b"}], "nextCursor": "c1"})
    );
}

#[tokio::test]
async fn call_returns_items_object() {
    let r = registry(|_| {});
    let c = ctx(&[&["a"]]);
    let out = call(&r, &c, "concept_list", json!({})).await.unwrap();
    assert_eq!(out, json!({"items": [{"id":"a"}]}));
}

struct ListAll;
impl op_registry::Op for ListAll {
    type Ctx = Ctx;
    type Params = ListConcepts;
    type Output = Vec<Concept>;
    const NAME: &'static str = "concept_list";
}

#[tokio::test]
async fn call_typed_returns_vec() {
    let r = registry(|_| {});
    let c = ctx(&[&["a"], &["b"]]);
    let out: Vec<Concept> = r
        .call_typed::<ListAll>(&c, ListConcepts { tag: None }, CallOptions::default())
        .await
        .unwrap();
    assert_eq!(
        out,
        vec![Concept { id: "a".into() }, Concept { id: "b".into() }]
    );
}

#[tokio::test]
async fn value_output_array_is_wrapped() {
    let mut r: Registry<Ctx> = Registry::new();
    r.push_op(
        OpSpec::<ListConcepts, serde_json::Value>::new("dump", "Dump."),
        |_c: &Ctx,
         _p: ListConcepts,
         _o: CallOptions|
         -> BoxFuture<'_, anyhow::Result<serde_json::Value>> {
            Box::pin(async { Ok(json!([1, 2])) })
        },
    );
    let out = call(&r, &Ctx::default(), "dump", json!({})).await.unwrap();
    assert_eq!(out, json!({"items": [1, 2]}));
}

// --- OUT-3: `--text` on list pairs ---

impl op_registry::RenderText for Concept {
    fn render_text(&self) -> String {
        format!("concept {}", self.id)
    }
}

fn text_registry() -> Registry<Ctx> {
    let mut r: Registry<Ctx> = Registry::new();
    r.set_pillar("fix");
    r.group("concept", "Concepts.");
    let mut spec = OpSpec::<ListConcepts, Concept>::new("concept list", "List concepts.");
    spec.page_size = 2;
    r.push_list_text(spec, serve);
    r
}

/// Runs the CLI over the fixed pages `[a, b]` and `[c]` (see `from_globals`).
fn text_of(registry: &Registry<Ctx>, args: &[&str]) -> String {
    let argv = ["app"]
        .into_iter()
        .chain(args.iter().copied())
        .map(std::ffi::OsString::from)
        .collect();
    let (io, cap) = op_registry::cli::Io::memory(b"", false, false);
    let code = futures_executor::block_on(op_registry::cli::run_with_io(
        registry,
        argv,
        &op_registry::cli::NoHooks,
        io,
    ));
    assert_eq!(code, 0, "{}", cap.stderr());
    cap.stdout()
}

#[test]
fn list_text_follow_all_one_row_per_line() {
    let out = text_of(&text_registry(), &["concept", "list", "--text"]);
    assert_eq!(out, "concept a\nconcept b\nconcept c\n");
}

#[test]
fn list_text_page_ends_with_next_cursor() {
    let out = text_of(
        &text_registry(),
        &["concept", "list", "--limit", "2", "--text"],
    );
    assert_eq!(out, "concept a\nconcept b\nnext cursor: c1\n");
}

#[test]
fn list_text_page_without_cursor_has_no_cursor_line() {
    let out = text_of(
        &text_registry(),
        &[
            "concept", "list", "--limit", "2", "--cursor", "c1", "--text",
        ],
    );
    assert_eq!(out, "concept c\n");
}
