use futures_util::stream;
use op_registry::cli;
use op_registry::mcp::{self, ToolSpec};
use op_registry::parity::{assert_parity, cli_surface, Surface};
use op_registry::{registry, Audience, Cursor, OpParams, Registry};

struct Ctx;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, OpParams)]
struct Empty {
    #[allow(dead_code)]
    value: Option<i32>,
}

#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
struct Done {
    ok: bool,
}

async fn op(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    Ok(Done { ok: true })
}

async fn watch(
    _: &Ctx,
    _: Empty,
    _: Option<Cursor>,
) -> anyhow::Result<impl futures_util::Stream<Item = anyhow::Result<Done>>> {
    Ok(stream::iter(vec![Ok(Done { ok: true })]))
}

fn toy() -> Registry<Ctx> {
    registry! {
        Ctx = Ctx;
        entries: [
            {
                noun: "item", verb: "get", description: "gets",
                access: reads, audience: Agent, params: Empty, output: Done,
                op: op,
            },
            {
                name: "ping", noun: "system", verb: "ping", description: "pings",
                access: none, audience: Agent, params: Empty, output: Done,
                op: op,
            },
            {
                noun: "item", verb: "watch", description: "watches",
                access: reads, audience: Agent, params: Empty, output: Done,
                stream_op: watch,
            },
            {
                noun: "admin", verb: "reset", description: "owner only",
                access: writes, audience: Owner, params: Empty, output: Done,
                op: op,
            },
        ]
    }
}

fn surfaces(registry: &Registry<Ctx>) -> (Surface, Vec<ToolSpec>) {
    let cmd = cli::command(registry, "app", "toy");
    (cli_surface(&cmd), mcp::tool_list(registry, Audience::Agent))
}

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_owned(),
        description: String::new(),
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
    }
}

fn problems(result: Result<(), op_registry::parity::ParityReport>) -> String {
    result.err().expect("parity must fail").to_string()
}

#[test]
fn parity_helper_fails_on_unlisted_cli_or_mcp_item() {
    let registry = toy();
    let (cli, mcp) = surfaces(&registry);
    let expected: Vec<&str> = vec!["admin_reset", "item_get", "item_watch", "ping"];
    assert_eq!(
        cli.names.iter().map(String::as_str).collect::<Vec<_>>(),
        expected
    );

    // Correct surfaces pass: the streaming op and the owner op are CLI-only
    // with no allow-list entry.
    assert_parity(&registry, &cli, &mcp, &[], &[], Audience::Agent).unwrap();

    // An unlisted CLI-only command fails, naming the item; allow-listing fixes it.
    let mut cli_extra = cli.clone();
    cli_extra.names.insert("doctor".to_owned());
    let msg = problems(assert_parity(
        &registry,
        &cli_extra,
        &mcp,
        &[],
        &[],
        Audience::Agent,
    ));
    assert!(msg.contains("doctor"), "{msg}");
    assert_parity(
        &registry,
        &cli_extra,
        &mcp,
        &["doctor"],
        &[],
        Audience::Agent,
    )
    .unwrap();

    // An unlisted MCP-only tool fails, naming the item; allow-listing fixes it.
    let mut mcp_extra = mcp.clone();
    mcp_extra.push(spec("health"));
    let msg = problems(assert_parity(
        &registry,
        &cli,
        &mcp_extra,
        &[],
        &[],
        Audience::Agent,
    ));
    assert!(msg.contains("health"), "{msg}");
    assert_parity(
        &registry,
        &cli,
        &mcp_extra,
        &[],
        &["health"],
        Audience::Agent,
    )
    .unwrap();

    // A streaming op wrongly exposed as an MCP tool fails.
    let mut mcp_stream = mcp.clone();
    mcp_stream.push(spec("item_watch"));
    let msg = problems(assert_parity(
        &registry,
        &cli,
        &mcp_stream,
        &[],
        &[],
        Audience::Agent,
    ));
    assert!(msg.contains("item_watch"), "{msg}");

    // An op missing from the CLI fails.
    let mut cli_missing = cli.clone();
    cli_missing.names.remove("item_get");
    let msg = problems(assert_parity(
        &registry,
        &cli_missing,
        &mcp,
        &[],
        &[],
        Audience::Agent,
    ));
    assert!(msg.contains("item_get"), "{msg}");

    // An op missing from MCP fails.
    let mcp_missing: Vec<ToolSpec> = mcp.iter().filter(|t| t.name != "ping").cloned().collect();
    let msg = problems(assert_parity(
        &registry,
        &cli,
        &mcp_missing,
        &[],
        &[],
        Audience::Agent,
    ));
    assert!(msg.contains("ping"), "{msg}");

    // Every problem is collected, not just the first.
    let msg = problems(assert_parity(
        &registry,
        &cli_extra,
        &mcp_extra,
        &[],
        &[],
        Audience::Agent,
    ));
    assert!(msg.contains("doctor") && msg.contains("health"), "{msg}");

    // The Owner audience sees the owner op as the MCP tool instead.
    let owner_mcp = mcp::tool_list(&registry, Audience::Owner);
    assert_parity(&registry, &cli, &owner_mcp, &[], &[], Audience::Owner).unwrap();
}

#[test]
fn parity_helper_fails_on_a_stale_allow_list_entry() {
    let registry = toy();
    let (cli, mcp) = surfaces(&registry);

    // Allow-list entry that matches nothing.
    let msg = problems(assert_parity(
        &registry,
        &cli,
        &mcp,
        &["ghost"],
        &[],
        Audience::Agent,
    ));
    assert!(msg.contains("ghost"), "{msg}");
    let msg = problems(assert_parity(
        &registry,
        &cli,
        &mcp,
        &[],
        &["phantom"],
        Audience::Agent,
    ));
    assert!(msg.contains("phantom"), "{msg}");

    // An allow-list entry naming a real op (present on both sides) is stale too.
    let msg = problems(assert_parity(
        &registry,
        &cli,
        &mcp,
        &["item_get"],
        &[],
        Audience::Agent,
    ));
    assert!(msg.contains("item_get"), "{msg}");

    // A legitimate entry stops being legitimate once the extra goes away.
    let mut cli_extra = cli.clone();
    cli_extra.names.insert("doctor".to_owned());
    assert_parity(
        &registry,
        &cli_extra,
        &mcp,
        &["doctor"],
        &[],
        Audience::Agent,
    )
    .unwrap();
    let msg = problems(assert_parity(
        &registry,
        &cli,
        &mcp,
        &["doctor"],
        &[],
        Audience::Agent,
    ));
    assert!(msg.contains("doctor"), "{msg}");
}
