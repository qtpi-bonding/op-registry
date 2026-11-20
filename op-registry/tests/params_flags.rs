use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::{error::ErrorKind, Command, ValueEnum};
use op_registry::{FieldKind, OpGlobals, OpParams, ParseCx};
use serde_json::json;
use std::net::SocketAddr;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
enum Mode {
    Fast,
    Slow,
}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct Params {
    /// The owning agent.
    /// Spans two doc lines.
    agent_id: String,
    #[op(default = 50)]
    limit: u32,
    tags: Vec<String>,
    portable: Option<bool>,
    dry_run: bool,
    metadata: BTreeMap<String, String>,
    #[op(keys = ["env", "team"])]
    labels: BTreeMap<String, String>,
    count: Option<i64>,
    note: Option<String>,
    out: Option<PathBuf>,
    #[op(value_enum)]
    mode: Mode,
}

fn command() -> Command {
    Params::add_args(Command::new("t"))
}

fn parse(args: &[&str]) -> Result<Params, clap::Error> {
    let mut argv = vec!["t", "--agent-id", "a", "--mode", "fast"];
    argv.extend_from_slice(args);
    let matches = command().try_get_matches_from(argv)?;
    Ok(Params::from_matches(&matches, &mut ParseCx::default()).unwrap())
}

fn cli_err_code(args: &[&str]) -> &'static str {
    let mut argv = vec!["t", "--agent-id", "a", "--mode", "fast"];
    argv.extend_from_slice(args);
    let matches = command().try_get_matches_from(argv).unwrap();
    let err = Params::from_matches(&matches, &mut ParseCx::default()).unwrap_err();
    coded_error::facts_anyhow(&err).code
}

fn json_err_code(value: serde_json::Value) -> &'static str {
    let err = Params::from_json(value, &mut ParseCx::default()).unwrap_err();
    coded_error::facts_anyhow(&err).code
}

#[test]
fn flag_is_kebab_and_key_is_camel() {
    let fields = Params::fields();
    let f = fields.iter().find(|f| f.rust_name == "agent_id").unwrap();
    assert_eq!(f.flag, "agent-id");
    assert_eq!(f.key, "agentId");
    assert!(f.required);
    assert!(Params::input_schema()["properties"]
        .as_object()
        .unwrap()
        .contains_key("agentId"));
    let kinds: Vec<_> = fields.iter().map(|f| (f.rust_name, f.kind)).collect();
    assert!(kinds.contains(&("portable", FieldKind::TriState)));
    assert!(kinds.contains(&("dry_run", FieldKind::Switch)));
    assert!(kinds.contains(&("tags", FieldKind::Repeated)));
    assert!(kinds.contains(&("metadata", FieldKind::Map)));
    assert!(kinds.contains(&("limit", FieldKind::Scalar)));
}

#[test]
fn doc_comment_is_help_and_description() {
    let want = "The owning agent. Spans two doc lines.";
    assert_eq!(
        Params::input_schema()["properties"]["agentId"]["description"],
        want
    );
    let fields = Params::fields();
    assert_eq!(
        fields.iter().find(|f| f.key == "agentId").unwrap().help,
        want
    );
    let cmd = command();
    let arg = cmd
        .get_arguments()
        .find(|a| a.get_id() == "agent_id")
        .unwrap();
    assert_eq!(arg.get_help().unwrap().to_string(), want);
}

#[test]
fn repeated_flag_collects_vec() {
    assert_eq!(
        parse(&["--tags", "a", "--tags", "b"]).unwrap().tags,
        ["a", "b"]
    );
}

#[test]
fn comma_is_not_a_delimiter() {
    assert_eq!(parse(&["--tags", "a,b"]).unwrap().tags, ["a,b"]);
}

#[test]
fn tri_state_bool() {
    assert_eq!(parse(&["--portable"]).unwrap().portable, Some(true));
    assert_eq!(parse(&["--no-portable"]).unwrap().portable, Some(false));
    assert_eq!(parse(&[]).unwrap().portable, None);
    let err = parse(&["--portable", "--no-portable"]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    let json = Params::from_json(
        json!({"agentId":"a","mode":"fast","portable":false}),
        &mut ParseCx::default(),
    )
    .unwrap();
    assert_eq!(json.portable, Some(false));
}

#[test]
fn open_map_accepts_any_key() {
    let p = parse(&["--metadata", "x=1", "--metadata", "some-Key=a=b"]).unwrap();
    assert_eq!(p.metadata["x"], "1");
    assert_eq!(p.metadata["some-Key"], "a=b");
    let schema = Params::input_schema();
    assert_eq!(
        schema["properties"]["metadata"]["additionalProperties"],
        json!({"type":"string"})
    );
}

#[test]
fn closed_map_rejects_unknown_key() {
    assert_eq!(
        cli_err_code(&["--labels", "nope=1"]),
        "op_registry.usage.unknown_key"
    );
    assert_eq!(
        json_err_code(json!({"agentId":"a","mode":"fast","labels":{"nope":"1"}})),
        "op_registry.usage.unknown_key"
    );
    assert_eq!(
        parse(&["--labels", "env=prod"]).unwrap().labels["env"],
        "prod"
    );
    let labels = &Params::input_schema()["properties"]["labels"];
    assert_eq!(labels["additionalProperties"], false);
    assert!(labels["properties"]["team"].is_object());
}

#[test]
fn repeated_map_key_is_usage_error() {
    assert_eq!(
        cli_err_code(&["--metadata", "x=1", "--metadata", "x=2"]),
        "op_registry.usage.repeated_key"
    );
}

#[test]
fn json_rejects_snake_key() {
    assert_eq!(
        json_err_code(json!({"agent_id":"x","mode":"fast"})),
        "op_registry.usage.unknown_key"
    );
}

#[test]
fn json_and_cli_decode_equal() {
    let cli = parse(&[
        "--limit",
        "7",
        "--tags",
        "a",
        "--tags",
        "b",
        "--portable",
        "--dry-run",
        "--metadata",
        "k=v",
        "--labels",
        "team=x",
        "--count=-3",
        "--note=n",
        "--out",
        "/x/y",
    ])
    .unwrap();
    let json = Params::from_json(
        json!({"agentId":"a","limit":7,"tags":["a","b"],"portable":true,"dryRun":true,
               "metadata":{"k":"v"},"labels":{"team":"x"},"count":-3,"note":"n",
               "out":"/x/y","mode":"fast"}),
        &mut ParseCx::default(),
    )
    .unwrap();
    assert_eq!(cli, json);
}

#[test]
fn defaults_and_required() {
    assert_eq!(parse(&[]).unwrap().limit, 50);
    assert_eq!(Params::input_schema()["properties"]["limit"]["default"], 50);
    assert_eq!(
        json_err_code(json!({"mode":"fast"})),
        "op_registry.usage.missing"
    );
    let required = Params::input_schema()["required"].clone();
    assert_eq!(required, json!(["agentId", "mode"]));
}

#[test]
fn negative_number_parses() {
    assert_eq!(parse(&["--count", "-3"]).unwrap().count, Some(-3));
}

#[test]
fn hyphen_string_needs_equals() {
    assert_eq!(parse(&["--note=-x"]).unwrap().note.as_deref(), Some("-x"));
    assert!(parse(&["--note", "-x"]).is_err());
}

#[test]
fn only_short_flag_is_h() {
    let cmd = command();
    for arg in cmd.get_arguments() {
        assert!(
            arg.get_short().is_none() || arg.get_id() == "help",
            "unexpected short flag on {}",
            arg.get_id()
        );
    }
}

// ---- PAR-6 from_str, sample; PAR-8 ignore_case ----

#[derive(Clone, Debug, PartialEq)]
struct Grant {
    scope: String,
    level: String,
}

impl FromStr for Grant {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (scope, level) = s
            .split_once(':')
            .ok_or_else(|| format!("grant `{s}` is not scope:level"))?;
        Ok(Grant {
            scope: scope.to_owned(),
            level: level.to_owned(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PrincipalId(String);

impl FromStr for PrincipalId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        s.strip_prefix("p-")
            .map(|rest| PrincipalId(rest.to_owned()))
            .ok_or_else(|| "a principal id starts with p-".to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct Typed {
    #[op(from_str, sample = "127.0.0.1:7000")]
    addr: SocketAddr,
    #[op(from_str)]
    peer: Option<SocketAddr>,
    #[op(from_str)]
    grants: Vec<Grant>,
    #[op(from_str, default = 8080)]
    port: u16,
}

fn typed_cli(args: &[&str]) -> anyhow::Result<Typed> {
    let mut argv = vec!["t"];
    argv.extend_from_slice(args);
    let matches = Typed::add_args(Command::new("t"))
        .try_get_matches_from(argv)
        .unwrap();
    Typed::from_matches(&matches, &mut ParseCx::default())
}

#[test]
fn socket_addr_from_str_cli_and_json() {
    let cli = typed_cli(&["--addr", "127.0.0.1:7000", "--peer", "[::1]:9"]).unwrap();
    assert_eq!(cli.addr, "127.0.0.1:7000".parse().unwrap());
    assert_eq!(cli.peer, Some("[::1]:9".parse().unwrap()));
    assert_eq!(cli.port, 8080);
    let json = Typed::from_json(
        json!({"addr": "127.0.0.1:7000", "peer": "[::1]:9"}),
        &mut ParseCx::default(),
    )
    .unwrap();
    assert_eq!(json, cli);
    let fields = Typed::fields();
    assert_eq!(fields[0].kind, FieldKind::Scalar);
    assert_eq!(fields[1].kind, FieldKind::Scalar);
    assert_eq!(fields[2].kind, FieldKind::Repeated);
    let schema = Typed::input_schema();
    assert_eq!(schema["properties"]["addr"]["type"], "string");
    assert_eq!(schema["properties"]["peer"]["type"], "string");
    assert_eq!(schema["properties"]["grants"]["items"]["type"], "string");
}

#[test]
fn vec_of_custom_grant_from_str() {
    let cli = typed_cli(&[
        "--addr",
        "127.0.0.1:1",
        "--grants",
        "rooms:read",
        "--grants",
        "mail:write",
    ])
    .unwrap();
    let want = vec![
        Grant {
            scope: "rooms".into(),
            level: "read".into(),
        },
        Grant {
            scope: "mail".into(),
            level: "write".into(),
        },
    ];
    assert_eq!(cli.grants, want);
    let json = Typed::from_json(
        json!({"addr": "127.0.0.1:1", "grants": ["rooms:read", "mail:write"]}),
        &mut ParseCx::default(),
    )
    .unwrap();
    assert_eq!(json.grants, want);
}

#[derive(Debug, OpGlobals)]
struct Globals {
    #[op(from_str, env = "T_PRINCIPAL_UNSET")]
    principal: Option<PrincipalId>,
}

#[test]
fn global_principal_id_from_str() {
    let cmd = Globals::add_args(Command::new("t"));
    let m = cmd
        .clone()
        .try_get_matches_from(["t", "--principal", "p-42"])
        .unwrap();
    assert_eq!(
        Globals::from_matches(&m).unwrap().principal,
        Some(PrincipalId("42".into()))
    );
    let m = cmd
        .try_get_matches_from(["t", "--principal", "bob"])
        .unwrap();
    let err = Globals::from_matches(&m).unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "op_registry.usage.invalid"
    );
    assert!(
        err.to_string().contains("a principal id starts with p-"),
        "{err}"
    );
}

#[test]
fn bad_from_str_is_exit_2_with_parser_message() {
    let err = typed_cli(&["--addr", "nonsense"]).unwrap_err();
    let facts = coded_error::facts_anyhow(&err);
    assert_eq!(facts.code, "op_registry.usage.invalid");
    let text = err.to_string();
    assert!(text.contains("addr"), "{text}");
    assert!(text.contains("invalid socket address"), "{text}");

    let err = typed_cli(&["--addr", "127.0.0.1:1", "--grants", "nocolon"]).unwrap_err();
    assert!(err
        .to_string()
        .contains("grant `nocolon` is not scope:level"));

    let err = Typed::from_json(json!({"addr": "nonsense"}), &mut ParseCx::default()).unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "op_registry.usage.invalid"
    );
    let text = err.to_string();
    assert!(
        text.contains("addr") && text.contains("invalid socket address"),
        "{text}"
    );
}

#[derive(Debug, OpParams)]
struct SecretPin {
    #[op(secret, from_str)]
    pin: u32,
}

#[test]
fn secret_from_str_failure_says_invalid_value() {
    let matches = SecretPin::add_args(Command::new("t"))
        .try_get_matches_from(["t", "--pin", "hunter2-not-a-number"])
        .unwrap();
    let err = SecretPin::from_matches(&matches, &mut ParseCx::default()).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("invalid value"), "{text}");
    assert!(!text.contains("hunter2"), "{text}");
    let err = SecretPin::from_json(
        json!({"pin": "hunter2-not-a-number"}),
        &mut ParseCx::default(),
    )
    .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("invalid value") && !text.contains("hunter2"),
        "{text}"
    );
}

#[derive(Debug, OpParams)]
#[allow(dead_code)]
struct Sampled {
    #[op(sample = "agent-1")]
    agent: String,
    #[op(sample = "/tmp/x")]
    path: Option<PathBuf>,
    #[op(sample = "a")]
    names: Vec<String>,
}

#[test]
fn sample_puts_examples_in_schema() {
    let schema = Sampled::input_schema();
    assert_eq!(
        schema["properties"]["agent"]["examples"],
        json!(["agent-1"])
    );
    assert_eq!(schema["properties"]["path"]["examples"], json!(["/tmp/x"]));
    assert_eq!(
        schema["properties"]["names"]["items"]["examples"],
        json!(["a"])
    );
    let typed = Typed::input_schema();
    assert_eq!(
        typed["properties"]["addr"]["examples"],
        json!(["127.0.0.1:7000"])
    );
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
enum Status {
    Open,
    #[value(alias = "in_progress")]
    InProgress,
}

#[derive(Debug, PartialEq, OpParams)]
struct Filter {
    #[op(value_enum, ignore_case)]
    status: Status,
    #[op(value_enum, ignore_case)]
    more: Vec<Status>,
}

#[test]
fn ignore_case_status_cli_and_json() {
    for text in ["IN_PROGRESS", "In-Progress", "in-progress"] {
        let matches = Filter::add_args(Command::new("t"))
            .try_get_matches_from(["t", "--status", text, "--more", "OPEN"])
            .unwrap();
        let f = Filter::from_matches(&matches, &mut ParseCx::default()).unwrap();
        assert_eq!(f.status, Status::InProgress, "{text}");
        assert_eq!(f.more, vec![Status::Open]);
    }
    for text in ["IN_PROGRESS", "In-Progress"] {
        let f = Filter::from_json(
            json!({"status": text, "more": ["OPEN", "In-Progress"]}),
            &mut ParseCx::default(),
        )
        .unwrap();
        assert_eq!(f.status, Status::InProgress);
        assert_eq!(f.more, vec![Status::Open, Status::InProgress]);
    }
    let schema = Filter::input_schema();
    assert_eq!(
        schema["properties"]["status"]["enum"],
        json!(["open", "in-progress"])
    );
}

#[test]
fn without_ignore_case_the_names_stay_exact() {
    let err = Params::from_json(
        json!({"agentId": "a", "mode": "FAST"}),
        &mut ParseCx::default(),
    )
    .unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "op_registry.usage.invalid"
    );
}
