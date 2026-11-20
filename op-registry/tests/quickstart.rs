//! The README quick start, compiled and run. Keep the two in step.

use op_registry::cli::{self, Io, NoHooks};
use op_registry::{Audience, CallOptions, OpGlobals, OpParams};
use serde::Serialize;

/// Process settings: the `--hub` flag and `APP_HUB` variable of the binary.
#[derive(OpGlobals)]
pub struct Globals {
    /// Hub to talk to.
    pub hub: Option<String>,
}

/// The context every op receives. Built once per run from the globals.
pub struct Context {
    hub: Option<String>,
}

impl op_registry::Resolve for Context {}

impl op_registry::CliContext for Context {
    type Globals = Globals;
    fn from_globals(globals: &Globals) -> anyhow::Result<Self> {
        Ok(Context {
            hub: globals.hub.clone(),
        })
    }
}

#[derive(OpParams)]
pub struct ListItems {
    /// Largest number of items to return.
    #[op(default = 10)]
    pub limit: u32,
}

#[derive(Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Items {
    pub item_names: Vec<String>,
}

async fn item_list(
    ctx: &Context,
    params: ListItems,
    _opts: op_registry::CallOptions,
) -> anyhow::Result<Items> {
    let hub = ctx.hub.as_deref().unwrap_or("local");
    let item_names = (1..=params.limit).map(|n| format!("{hub}-{n}")).collect();
    Ok(Items { item_names })
}

mod app {
    use super::*;

    op_registry::ops! {
        Ctx = Context;
        globals = Globals;
        pillar = "app";
        bin = "app"; version = "0.1.0"; about = "Example app.";
        groups = {
            "item" => "Items.",
        };
        /// List items.
        "item list" => { params: ListItems, output: Items,
                         access: reads, audience: agent, op: item_list },
    }
}

#[test]
fn quick_start_runs_on_every_path() {
    let registry = app::registry();

    // The CLI runner: one JSON value on stdout, exit 0.
    let (io, captured) = Io::memory(b"", false, false);
    let argv = ["app", "--hub", "local", "item", "list", "--limit", "2"]
        .map(Into::into)
        .to_vec();
    let code = futures_executor::block_on(cli::run_with_io(&registry, argv, &NoHooks, io));
    assert_eq!(code, 0);
    assert_eq!(
        captured.stdout(),
        "{\"itemNames\":[\"local-1\",\"local-2\"]}\n"
    );

    // The tool list of one audience, and a call as a tool (camelCase keys).
    let tools = op_registry::mcp::tool_list(&registry, Audience::Agent);
    assert_eq!(tools[0].name, "item_list");
    let value = futures_executor::block_on(op_registry::mcp::call_tool(
        &registry,
        &Context { hub: None },
        Audience::Agent,
        "item_list",
        serde_json::json!({"limit": 1}),
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(value, serde_json::json!({"itemNames": ["local-1"]}));

    // The typed path: no JSON, the marker type names the op.
    let list = futures_executor::block_on(registry.call_typed::<app::ops::ItemList>(
        &Context { hub: None },
        ListItems { limit: 3 },
        CallOptions::default(),
    ))
    .unwrap();
    assert_eq!(list.item_names.len(), 3);
}

#[cfg(feature = "testing")]
#[test]
fn quick_start_passes_the_testing_kit() {
    use op_registry::testing::{assert_casing, assert_parity, Casing, Mount};
    let tools = ["item_list"];
    let registry = app::registry();
    assert_casing(&registry, Casing::Camel);
    assert_parity(
        &registry,
        &[Mount {
            name: "mcp",
            audience: Audience::Agent,
            ops: &tools,
        }],
    );
}
