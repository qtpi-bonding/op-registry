//! `paths` is a set of surfaces: an op works on the surfaces it declares and
//! is `wrong_path` on every other.

use futures_util::future::BoxFuture;
use op_registry::cli::{self, Io, NoHooks};
use op_registry::mcp::{call_tool, tool_list};
use op_registry::{
    Access, Audience, CallOptions, CliContext, Op, OpGlobals, OpParams, OpSpec, Page, PageRequest,
    Paths, Registry, Surface,
};
use serde::Serialize;
use serde_json::json;
use std::ffi::OsString;

#[derive(OpGlobals)]
pub struct NoGlobals {}

pub struct Ctx;
impl op_registry::Resolve for Ctx {}
impl CliContext for Ctx {
    type Globals = NoGlobals;
    fn from_globals(_: &NoGlobals) -> anyhow::Result<Self> {
        Ok(Ctx)
    }
}

#[derive(Clone, OpParams)]
pub struct Nothing {}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Out {
    pub done: bool,
}

struct Run;
impl Op for Run {
    type Ctx = Ctx;
    type Params = Nothing;
    type Output = Out;
    const NAME: &'static str = "t_run";
}

fn run_op(_c: &Ctx, _p: Nothing, _o: CallOptions) -> BoxFuture<'_, anyhow::Result<Out>> {
    Box::pin(async { Ok(Out { done: true }) })
}

fn page_op(_c: &Ctx, _p: Nothing, _r: PageRequest) -> BoxFuture<'_, anyhow::Result<Page<Out>>> {
    Box::pin(async {
        Ok(Page {
            items: vec![],
            next_cursor: None,
        })
    })
}

fn registry_with(surfaces: &[Surface]) -> Registry<Ctx> {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.set_bin("tool", "1.0.0", "A fixture.");
    r.group("t", "Things.");
    let mut spec = OpSpec::<Nothing, Out>::new("t run", "Run it.");
    spec.access = Access::reads();
    spec.paths = Paths::of(surfaces);
    r.push_op(spec, run_op);
    r
}

fn argv(args: &[&str]) -> Vec<OsString> {
    std::iter::once("tool")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect()
}

fn is_wrong_path(err: &anyhow::Error) -> bool {
    coded_error::facts_anyhow(err).code == "op_registry.call.wrong_path"
}

const SETS: [&[Surface]; 6] = [
    &[Surface::Cli],
    &[Surface::Typed],
    &[Surface::Mcp],
    &[Surface::Json],
    &[Surface::Cli, Surface::Typed],
    &[Surface::Mcp, Surface::Json],
];

#[test]
fn each_paths_set_reaches_exactly_its_surfaces() {
    for set in SETS {
        let r = registry_with(set);
        let declared = |s: Surface| set.contains(&s);

        let (io, cap) = Io::memory(b"", false, false);
        let code =
            futures_executor::block_on(cli::run_with_io(&r, argv(&["t", "run"]), &NoHooks, io));
        if declared(Surface::Cli) {
            assert_eq!(code, 0, "{set:?}: {}", cap.stderr());
            assert_eq!(cap.stdout(), "{\"done\":true}\n", "{set:?}");
        } else {
            assert_eq!(code, 2, "{set:?}: {}", cap.stderr());
            assert_eq!(cap.stdout(), "", "{set:?}");
        }

        let typed = futures_executor::block_on(r.call_typed::<Run>(
            &Ctx,
            Nothing {},
            CallOptions::default(),
        ));
        match typed {
            Ok(out) => assert!(declared(Surface::Typed) && out.done, "{set:?}"),
            Err(err) => assert!(
                !declared(Surface::Typed) && is_wrong_path(err.error()),
                "{set:?}"
            ),
        }

        let tool = futures_executor::block_on(call_tool(
            &r,
            &Ctx,
            Audience::Agent,
            "t_run",
            json!({}),
            CallOptions::default(),
        ));
        match tool {
            Ok(v) => assert!(
                declared(Surface::Mcp) && v == json!({"done": true}),
                "{set:?}"
            ),
            Err(f) => assert!(
                !declared(Surface::Mcp) && is_wrong_path(f.error()),
                "{set:?}"
            ),
        }

        let json_call =
            futures_executor::block_on(r.call("t_run", json!({}), &Ctx, CallOptions::default()));
        match json_call {
            Ok(v) => assert!(
                declared(Surface::Json) && v == json!({"done": true}),
                "{set:?}"
            ),
            Err(f) => assert!(
                !declared(Surface::Json) && is_wrong_path(f.error()),
                "{set:?}"
            ),
        }
    }
}

#[test]
fn cli_tree_and_tool_list_follow_paths() {
    for set in SETS {
        let r = registry_with(set);
        let root = cli::command(&r);
        let on_cli = root
            .find_subcommand("t")
            .is_some_and(|t| t.find_subcommand("run").is_some());
        assert_eq!(on_cli, set.contains(&Surface::Cli), "{set:?}");
        let listed = tool_list(&r, Audience::Agent)
            .iter()
            .any(|t| t.name == "t_run");
        assert_eq!(listed, set.contains(&Surface::Mcp), "{set:?}");
    }
}

#[test]
fn paths_serialize_as_words() {
    let p = Paths::of(&[Surface::Typed, Surface::Cli]);
    assert_eq!(serde_json::to_value(p).unwrap(), json!(["cli", "typed"]));
    assert_eq!(
        serde_json::to_value(Paths::ALL).unwrap(),
        json!(["cli", "typed", "mcp", "json"])
    );
    let back: Paths = serde_json::from_value(json!(["json", "cli"])).unwrap();
    assert_eq!(back, Paths::of(&[Surface::Cli, Surface::Json]));
    assert_eq!(Paths::default(), Paths::ALL);
    assert_eq!(
        Paths::ALL.iter().collect::<Vec<_>>(),
        vec![Surface::Cli, Surface::Typed, Surface::Mcp, Surface::Json]
    );
}

#[test]
fn list_halves_share_paths() {
    let mut r = Registry::<Ctx>::new();
    r.set_pillar("fix");
    r.set_bin("tool", "1.0.0", "A fixture.");
    r.group("t", "Things.");
    let mut spec = OpSpec::<Nothing, Out>::new("t list", "List them.");
    spec.access = Access::reads();
    spec.paths = Paths::of(&[Surface::Cli, Surface::Mcp]);
    r.push_list(spec, page_op);
    let all = r.get("t_list").unwrap().paths;
    let page = r.get("t_list_page").unwrap().paths;
    assert_eq!(all, Paths::of(&[Surface::Cli, Surface::Mcp]));
    assert_eq!(page, all);
    let names: Vec<String> = tool_list(&r, Audience::Agent)
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, vec!["t_list", "t_list_page"]);
}
