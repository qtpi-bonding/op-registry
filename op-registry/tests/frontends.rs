#![allow(clippy::err_expect, clippy::ok_expect)]
use clap::ValueEnum;
use coded_error::cli::ExitTable;
use coded_error::ErrorClass;
use futures_util::{stream, StreamExt};
use op_registry::cli::{self, CliOutput};
use op_registry::mcp::{self, ToolSpec};
use op_registry::{registry, Audience, CallOptions, Cursor, OpParams, Registry};
use serde_json::{json, Value};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

struct Ctx;

#[derive(Clone, Debug, PartialEq, ValueEnum, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
enum Speed {
    Fast,
    Slow,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, OpParams)]
struct GetParams {
    #[op(positional)]
    id: String,
    #[op(long = "limit", default = 10)]
    limit: u32,
    tags: Vec<String>,
    #[op(value_enum)]
    speed: Option<Speed>,
    note: Option<String>,
    dry_run: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, OpParams)]
struct Empty {
    #[allow(dead_code)]
    value: Option<i32>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, OpParams)]
struct WatchParams {
    #[op(positional)]
    prefix: String,
}

#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
struct Echo {
    id: String,
    limit: u32,
    tags: Vec<String>,
    speed: Option<String>,
    note: Option<String>,
    dry_run: bool,
}

#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
struct Done {
    ok: bool,
}

#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
struct Event {
    text: String,
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.fail")]
enum Fail {
    #[error("bad request detail")]
    #[coded(code = "logical", class = Logical)]
    Logical,
    #[error("disk exploded")]
    #[coded(code = "infra", class = Infrastructure)]
    Infra,
    #[error("try again later")]
    #[coded(code = "transient", class = Transient)]
    Transient,
}

async fn item_get(_: &Ctx, p: GetParams) -> anyhow::Result<Echo> {
    Ok(Echo {
        id: p.id,
        limit: p.limit,
        tags: p.tags,
        speed: p.speed.map(|s| format!("{s:?}")),
        note: p.note,
        dry_run: p.dry_run,
    })
}

async fn ping(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    Ok(Done { ok: true })
}

async fn reset(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    Ok(Done { ok: true })
}

async fn legacy(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    Ok(Done { ok: true })
}

async fn watch(
    _: &Ctx,
    p: WatchParams,
    cursor: Option<Cursor>,
) -> anyhow::Result<impl futures_util::Stream<Item = anyhow::Result<Event>>> {
    let c = cursor
        .map(|c| c.as_str().to_owned())
        .unwrap_or_else(|| "none".to_owned());
    Ok(stream::iter(vec![
        Ok(Event {
            text: format!("{}/{}/0", p.prefix, c),
        }),
        Ok(Event {
            text: format!("{}/{}/1", p.prefix, c),
        }),
    ]))
}

async fn fail_logical(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    tracing::error!("fail_logical op failed");
    Err(coded_error::into_anyhow(Fail::Logical))
}

async fn fail_infra(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    tracing::error!("fail_infra op failed");
    Err(coded_error::into_anyhow(Fail::Infra).context("while storing"))
}

async fn fail_transient(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    tracing::error!("fail_transient op failed");
    Err(coded_error::into_anyhow(Fail::Transient).context("while polling"))
}

async fn fail_unknown(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    tracing::error!("fail_unknown op failed");
    Err(anyhow::anyhow!("mystery failure").context("while guessing"))
}

fn toy() -> Registry<Ctx> {
    registry! {
        Ctx = Ctx;
        entries: [
            {
                noun: "item", verb: "get", description: "gets an item",
                access: reads, audience: Agent, params: GetParams, output: Echo,
                op: item_get,
            },
            {
                name: "ping", noun: "system", verb: "ping", description: "pings",
                access: none, audience: Agent, params: Empty, output: Done,
                op: ping,
            },
            {
                noun: "item", verb: "watch", description: "watches items",
                access: reads, audience: Agent, params: WatchParams, output: Event,
                stream_op: watch,
            },
            {
                noun: "admin", verb: "reset", description: "owner only reset",
                access: writes, audience: Owner, params: Empty, output: Done,
                op: reset,
            },
            {
                noun: "legacy", verb: "run", description: "old thing",
                access: none, audience: Agent, params: Empty, output: Done,
                op: legacy, hidden: true, deprecated: "use item get",
            },
            {
                noun: "err", verb: "logical", description: "fails logically",
                access: none, audience: Agent, params: Empty, output: Done,
                op: fail_logical,
            },
            {
                noun: "err", verb: "infra", description: "fails infra",
                access: none, audience: Agent, params: Empty, output: Done,
                op: fail_infra,
            },
            {
                noun: "err", verb: "transient", description: "fails transiently",
                access: none, audience: Agent, params: Empty, output: Done,
                op: fail_transient,
            },
            {
                noun: "err", verb: "unknown", description: "fails unclassified",
                access: none, audience: Agent, params: Empty, output: Done,
                op: fail_unknown,
            },
        ]
    }
}

fn command(registry: &Registry<Ctx>) -> clap::Command {
    cli::command(registry, "app", "toy app")
}

fn matches_for(registry: &Registry<Ctx>, argv: &[&str]) -> clap::ArgMatches {
    let mut full = vec!["app"];
    full.extend_from_slice(argv);
    command(registry).try_get_matches_from(full).unwrap()
}

async fn cli_value(registry: &Registry<Ctx>, argv: &[&str]) -> anyhow::Result<Value> {
    let matches = matches_for(registry, argv);
    match cli::dispatch(registry, &Ctx, &matches, CallOptions::default()).await? {
        CliOutput::Value(v) => Ok(v),
        CliOutput::Stream(_) => panic!("expected a value, got a stream"),
    }
}

async fn cli_error(registry: &Registry<Ctx>, argv: &[&str]) -> anyhow::Error {
    cli_value(registry, argv).await.err().expect("must fail")
}

async fn tool_error_of(registry: &Registry<Ctx>, name: &str) -> mcp::ToolError {
    mcp::call_tool(
        registry,
        &Ctx,
        Audience::Agent,
        name,
        json!({}),
        CallOptions::default(),
    )
    .await
    .err()
    .expect("must fail")
}

fn code_of(err: &anyhow::Error) -> &'static str {
    coded_error::facts_anyhow(err).code
}

#[test]
fn cli_command_is_built_from_descriptors_and_passes_debug_assert() {
    let registry = toy();
    let cmd = command(&registry);
    command(&registry).debug_assert();
    assert_eq!(cmd.get_name(), "app");
    let item = cmd.find_subcommand("item").expect("noun group item");
    assert!(item.find_subcommand("get").is_some());
    assert!(item.find_subcommand("watch").is_some());
    let err = cmd.find_subcommand("err").expect("noun group err");
    for verb in ["logical", "infra", "transient", "unknown"] {
        assert!(err.find_subcommand(verb).is_some(), "verb {verb}");
    }
    assert!(cmd
        .find_subcommand("admin")
        .unwrap()
        .find_subcommand("reset")
        .is_some());
    // The noun group itself is not an op.
    assert!(cmd.find_subcommand("item_get").is_none());
    // Missing subcommand is refused.
    assert!(command(&registry).try_get_matches_from(["app"]).is_err());
}

#[test]
fn cli_explicit_name_is_a_top_level_command() {
    let registry = toy();
    let cmd = command(&registry);
    let ping = cmd.find_subcommand("ping").expect("top-level ping");
    assert!(ping.get_subcommands().next().is_none());
    assert!(cmd.find_subcommand("system").is_none());
    futures_executor::block_on(async {
        let v = cli_value(&registry, &["ping"]).await.unwrap();
        assert_eq!(v, json!({"ok": true}));
    });
}

#[test]
fn cli_and_mcp_parse_same_args_to_same_json() {
    let registry = toy();
    let table: Vec<(&str, Vec<&str>, Value)> = vec![
        // defaults only
        ("item_get", vec!["item", "get", "abc"], json!({"id": "abc"})),
        // every field: repeatable Vec, enum, Option, flag, override of default
        (
            "item_get",
            vec![
                "item",
                "get",
                "abc",
                "--limit",
                "3",
                "--tags",
                "a",
                "--tags",
                "b",
                "--speed",
                "slow",
                "--note",
                "n",
                "--dry-run",
            ],
            json!({"id": "abc", "limit": 3, "tags": ["a", "b"], "speed": "slow",
                   "note": "n", "dry_run": true}),
        ),
        // Option and enum absent, Vec single
        (
            "item_get",
            vec!["item", "get", "z", "--tags", "only"],
            json!({"id": "z", "tags": ["only"]}),
        ),
        ("ping", vec!["ping"], json!({})),
    ];
    futures_executor::block_on(async {
        for (name, argv, args) in table {
            let from_cli = cli_value(&registry, &argv).await.unwrap();
            let from_mcp = mcp::call_tool(
                &registry,
                &Ctx,
                Audience::Agent,
                name,
                args.clone(),
                CallOptions::default(),
            )
            .await
            .unwrap();
            assert_eq!(from_cli, from_mcp, "{name} {argv:?}");
        }
        // sanity: the defaults really were applied
        let v = cli_value(&registry, &["item", "get", "abc"]).await.unwrap();
        assert_eq!(v["limit"], 10);
        assert_eq!(v["tags"], json!([]));
        assert_eq!(v["speed"], Value::Null);
        assert_eq!(v["dry_run"], false);
    });
}

#[test]
fn unknown_enum_param_is_refused_by_both() {
    let registry = toy();
    // CLI: the value-enum arg is validated by clap against the ValueEnum
    // variants, so the unknown value never reaches the op.
    let err = command(&registry)
        .try_get_matches_from(["app", "item", "get", "abc", "--speed", "warp"])
        .err()
        .expect("clap must refuse");
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
    // MCP: the same refusal comes back as an op_registry.usage.* code.
    futures_executor::block_on(async {
        let err = mcp::call_tool(
            &registry,
            &Ctx,
            Audience::Agent,
            "item_get",
            json!({"id": "abc", "speed": "warp"}),
            CallOptions::default(),
        )
        .await
        .err()
        .expect("mcp must refuse")
        .into_inner();
        assert!(
            code_of(&err).starts_with("op_registry.usage."),
            "code was {}",
            code_of(&err)
        );
    });
}

#[test]
fn mcp_tool_list_equals_descriptor_list() {
    let registry = toy();
    let tools = mcp::tool_list(&registry, Audience::Agent);
    let expected: Vec<ToolSpec> = registry
        .descriptors()
        .into_iter()
        .filter(|d| !d.streaming && d.audience == Audience::Agent)
        .map(|d| ToolSpec {
            name: d.name.clone(),
            description: d.description.clone(),
            input_schema: d.input_schema.clone(),
            output_schema: d.output_schema.clone(),
        })
        .collect();
    assert_eq!(tools, expected);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "item_get",
            "ping",
            "legacy_run",
            "err_logical",
            "err_infra",
            "err_transient",
            "err_unknown"
        ]
    );
    let owner = mcp::tool_list(&registry, Audience::Owner);
    assert_eq!(owner.len(), 1);
    assert_eq!(owner[0].name, "admin_reset");
    assert!(mcp::tool_list(&registry, Audience::Dev).is_empty());
}

#[test]
fn mcp_tools_have_output_schema() {
    let registry = toy();
    for audience in [Audience::Agent, Audience::Owner, Audience::Dev] {
        for tool in mcp::tool_list(&registry, audience) {
            let obj = tool
                .output_schema
                .as_object()
                .unwrap_or_else(|| panic!("{} output_schema is not an object", tool.name));
            assert!(
                obj.contains_key("type")
                    || obj.contains_key("properties")
                    || obj.contains_key("$schema"),
                "{} output_schema has no type/properties/$schema",
                tool.name
            );
            assert_ne!(tool.output_schema, Value::Null);
        }
    }
}

#[test]
fn streaming_ops_are_not_mcp_tools() {
    let registry = toy();
    for audience in [Audience::Agent, Audience::Owner, Audience::Dev] {
        assert!(mcp::tool_list(&registry, audience)
            .iter()
            .all(|t| t.name != "item_watch"));
    }
    assert!(registry.get("item_watch").unwrap().streaming);
}

#[test]
fn streaming_op_has_cursor_flag_and_unary_does_not() {
    let registry = toy();
    let cmd = command(&registry);
    let item = cmd.find_subcommand("item").unwrap();
    let watch = item.find_subcommand("watch").unwrap();
    let cursor = watch
        .get_arguments()
        .find(|a| a.get_id() == "cursor")
        .expect("streaming op has --cursor");
    assert_eq!(cursor.get_long(), Some("cursor"));
    let get = item.find_subcommand("get").unwrap();
    assert!(get.get_arguments().all(|a| a.get_id() != "cursor"));
    assert!(cmd
        .find_subcommand("ping")
        .unwrap()
        .get_arguments()
        .all(|a| a.get_id() != "cursor"));

    // The flag value reaches the op as Option<Cursor>.
    futures_executor::block_on(async {
        for (argv, expected) in [
            (vec!["item", "watch", "p", "--cursor", "c9"], "p/c9/0"),
            (vec!["item", "watch", "p"], "p/none/0"),
        ] {
            let matches = matches_for(&registry, &argv);
            let out = cli::dispatch(&registry, &Ctx, &matches, CallOptions::default())
                .await
                .ok()
                .expect("dispatch");
            let CliOutput::Stream(s) = out else {
                panic!("expected stream");
            };
            let items: Vec<_> = s.collect().await;
            assert_eq!(items[0].as_ref().unwrap()["text"], expected);
        }
    });
}

#[test]
fn owner_audience_op_is_absent_from_agent_tool_list() {
    let registry = toy();
    let agent = mcp::tool_list(&registry, Audience::Agent);
    assert!(agent.iter().all(|t| t.name != "admin_reset"));
    assert!(mcp::tool_list(&registry, Audience::Owner)
        .iter()
        .any(|t| t.name == "admin_reset"));
    // Still a CLI command.
    assert!(command(&registry)
        .find_subcommand("admin")
        .unwrap()
        .find_subcommand("reset")
        .is_some());
}

#[test]
fn hidden_and_deprecated_ops_still_appear_in_cli_and_tool_list() {
    let registry = toy();
    let d = registry.get("legacy_run").unwrap();
    assert!(d.hidden);
    assert_eq!(d.deprecated.as_deref(), Some("use item get"));
    assert!(command(&registry)
        .find_subcommand("legacy")
        .and_then(|c| c.find_subcommand("run"))
        .is_some());
    assert!(mcp::tool_list(&registry, Audience::Agent)
        .iter()
        .any(|t| t.name == "legacy_run"));
    futures_executor::block_on(async {
        assert_eq!(
            cli_value(&registry, &["legacy", "run"]).await.unwrap(),
            json!({"ok": true})
        );
    });
}

#[test]
fn stream_op_prints_one_json_line_per_event_and_takes_a_cursor() {
    let registry = toy();
    futures_executor::block_on(async {
        let matches = matches_for(&registry, &["item", "watch", "p", "--cursor", "abc"]);
        let out = cli::dispatch(&registry, &Ctx, &matches, CallOptions::default())
            .await
            .ok()
            .expect("dispatch");
        let CliOutput::Stream(s) = out else {
            panic!("expected CliOutput::Stream");
        };
        let items: Vec<anyhow::Result<Value>> = s.collect().await;
        let lines: Vec<String> = items
            .iter()
            .map(|i| cli::render(i.as_ref().unwrap(), false))
            .collect();
        assert_eq!(
            lines,
            vec![
                r#"{"text":"p/abc/0"}"#.to_owned(),
                r#"{"text":"p/abc/1"}"#.to_owned()
            ]
        );
        assert!(lines.iter().all(|l| !l.contains('\n')));
    });
}

#[test]
fn output_contract_stdout_one_json_value_stderr_code_message_exit_from_class_table() {
    let value = json!({"a": 1, "b": [1, 2]});
    assert_eq!(cli::render(&value, false), r#"{"a":1,"b":[1,2]}"#);
    let pretty = cli::render(&value, true);
    assert!(pretty.contains('\n'));
    assert!(!pretty.ends_with('\n'));
    assert_eq!(serde_json::from_str::<Value>(&pretty).unwrap(), value);

    let registry = toy();
    futures_executor::block_on(async {
        let logical = cli_error(&registry, &["err", "logical"]).await;
        let infra = cli_error(&registry, &["err", "infra"]).await;
        let transient = cli_error(&registry, &["err", "transient"]).await;
        let unknown = cli_error(&registry, &["err", "unknown"]).await;

        assert_eq!(
            cli::error_line(&logical),
            format!("{}: {:#}", code_of(&logical), logical)
        );
        assert_eq!(
            cli::error_line(&logical),
            "test.fail.logical: bad request detail"
        );
        assert_eq!(
            cli::error_line(&infra),
            "test.fail.infra: while storing: disk exploded"
        );

        // coded_error's SYSEXITS: logical 65, infrastructure 69, transient 75, unknown 70.
        assert_eq!(logical_class(&logical), ErrorClass::Logical);
        assert_eq!(cli::exit_code_for(&logical, &ExitTable::SYSEXITS), 65);
        assert_eq!(cli::exit_code_for(&infra, &ExitTable::SYSEXITS), 69);
        assert_eq!(cli::exit_code_for(&transient, &ExitTable::SYSEXITS), 75);
        assert_eq!(cli::exit_code_for(&unknown, &ExitTable::SYSEXITS), 70);

        let custom = ExitTable {
            logical: 2,
            infrastructure: 3,
            transient: 4,
            unknown: 5,
        };
        assert_eq!(cli::exit_code_for(&logical, &custom), 2);
        assert_eq!(cli::exit_code_for(&infra, &custom), 3);
        assert_eq!(cli::exit_code_for(&transient, &custom), 4);
        assert_eq!(cli::exit_code_for(&unknown, &custom), 5);
    });
}

fn logical_class(err: &anyhow::Error) -> ErrorClass {
    coded_error::facts_anyhow(err).class
}

#[test]
fn cli_shows_full_message_mcp_redacts_non_logical() {
    let registry = toy();
    futures_executor::block_on(async {
        // Logical: both show the message.
        let cli_logical = cli_error(&registry, &["err", "logical"]).await;
        assert!(cli::error_line(&cli_logical).ends_with("bad request detail"));
        let tool_logical = tool_error_of(&registry, "err_logical").await;
        let f = mcp::tool_error(&tool_logical, "test");
        assert_eq!(f.message, "bad request detail");
        assert_eq!(f.class, ErrorClass::Logical);
        assert_eq!(f.code, "test.fail.logical");
        assert!(!f.fingerprint.is_empty());

        // Non-Logical: CLI shows everything, MCP redacts.
        for (op, argv, class, full) in [
            (
                "err_infra",
                ["err", "infra"],
                ErrorClass::Infrastructure,
                "while storing: disk exploded",
            ),
            (
                "err_transient",
                ["err", "transient"],
                ErrorClass::Transient,
                "while polling: try again later",
            ),
            (
                "err_unknown",
                ["err", "unknown"],
                ErrorClass::Unknown,
                "while guessing: mystery failure",
            ),
        ] {
            let cli_err = cli_error(&registry, &argv).await;
            assert!(
                cli::error_line(&cli_err).ends_with(full),
                "{}",
                cli::error_line(&cli_err)
            );
            let tool = tool_error_of(&registry, op).await;
            let f = mcp::tool_error(&tool, "test");
            assert_eq!(f.message, "request failed", "{op}");
            assert_eq!(f.class, class, "{op}");
            assert!(!f.code.is_empty(), "{op}");
            assert!(!f.fingerprint.is_empty(), "{op}");
            assert!(!f.message.contains("exploded"));
        }
    });
}

thread_local! {
    static ERRORS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct CountErrors;

impl<S: tracing::Subscriber> Layer<S> for CountErrors {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() == tracing::Level::ERROR {
            ERRORS.with(|n| n.set(n.get() + 1));
        }
    }
}

/// Counts ERROR events emitted on this thread while `run` executes. A single
/// global subscriber keeps tracing's callsite interest cache stable when other
/// tests run concurrently; the count is per thread.
fn errors_during(run: impl FnOnce()) -> usize {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(CountErrors);
        tracing::subscriber::set_global_default(subscriber).expect("install subscriber");
    });
    ERRORS.with(|n| n.set(0));
    run();
    ERRORS.with(|n| n.get())
}

#[test]
fn one_failed_call_produces_one_error_event() {
    let registry = toy();
    let count = errors_during(|| {
        futures_executor::block_on(async {
            let _ = cli_error(&registry, &["err", "logical"]).await;
        });
    });
    assert_eq!(count, 1, "dispatch");

    let count = errors_during(|| {
        futures_executor::block_on(async {
            let err = tool_error_of(&registry, "err_infra").await;
            let _ = mcp::tool_error(&err, "test");
        });
    });
    assert_eq!(count, 1, "call_tool");
}

#[test]
fn call_tool_refuses_streaming_and_out_of_audience_ops() {
    let registry = toy();
    futures_executor::block_on(async {
        let call = |aud: Audience, name: &'static str| {
            let registry = &registry;
            async move {
                mcp::call_tool(
                    registry,
                    &Ctx,
                    aud,
                    name,
                    json!({"prefix": "p"}),
                    CallOptions::default(),
                )
                .await
                .err()
                .expect("must be refused")
                .into_inner()
            }
        };
        let e = call(Audience::Agent, "item_watch").await;
        assert_eq!(code_of(&e), "op_registry.call.wrong_shape");
        let e = call(Audience::Agent, "admin_reset").await;
        assert_eq!(code_of(&e), "op_registry.call.outside_audience");
        assert_eq!(logical_class(&e), ErrorClass::Logical);
        let e = call(Audience::Agent, "no_such_op").await;
        assert_eq!(code_of(&e), "op_registry.call.unknown_operation");
        // In-audience owner call works.
        let ok = mcp::call_tool(
            &registry,
            &Ctx,
            Audience::Owner,
            "admin_reset",
            json!({}),
            CallOptions::default(),
        )
        .await
        .ok()
        .expect("owner may call");
        assert_eq!(ok, json!({"ok": true}));
    });
}
