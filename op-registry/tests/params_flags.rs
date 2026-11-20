use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::{error::ErrorKind, Command, ValueEnum};
use op_registry::{FieldKind, OpParams, ParseCx};
use serde_json::json;

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
