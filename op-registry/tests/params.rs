use clap::{error::ErrorKind, ValueEnum};
use op_registry::OpParams;
use serde::Deserialize;
use serde_json::json;

fn custom_schema() -> serde_json::Value {
    json!({"type": "string", "format": "custom"})
}

#[derive(Clone, Debug, Deserialize, PartialEq, OpParams)]
struct Params {
    /// The row id.
    #[op(positional)]
    id: String,
    #[op(positional)]
    suffix: Option<String>,
    #[op(long = "limit", short = 'l', default = 50)]
    limit: u32,
    #[op(default = "guest")]
    role: String,
    #[op(value_delimiter = ',')]
    tags: Vec<String>,
    #[op(env = "OP_TEST_NAME_UNIQUE")]
    name: Option<String>,
    #[op(value_enum)]
    mode: Mode,
    #[op(value_enum)]
    optional_mode: Option<Mode>,
    #[op(value_enum, value_delimiter = ',')]
    modes: Vec<Mode>,
    dry_run: bool,
    #[op(required_unless_present = "id")]
    query: Option<String>,
    #[op(doc = "A custom description", schema = "custom_schema")]
    custom: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, ValueEnum)]
enum Mode {
    Fast,
    Slow,
}

#[derive(Clone, Debug, Deserialize, PartialEq, ValueEnum)]
enum VisibleMode {
    Fast,
    #[value(skip)]
    Hidden,
}

#[derive(Clone, Debug, Deserialize, PartialEq, OpParams)]
struct NumericParams {
    #[op(value_delimiter = ',')]
    ids: Vec<u32>,
    #[op(default = -1)]
    offset: i32,
}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct ConditionalParams {
    target: String,
    #[op(required_unless_present = "target")]
    other: Option<String>,
}

#[derive(Clone, Debug, PartialEq, OpParams)]
struct RequiredId {
    #[op(positional)]
    id: String,
}

#[test]
fn op_params_one_declaration_gives_clap_serde_and_schema() {
    let schema = Params::schema();
    assert_eq!(schema["properties"]["id"]["description"], "The row id.");
    assert_eq!(schema["properties"]["limit"]["default"], 50);
    assert_eq!(schema["properties"]["role"]["default"], "guest");
    assert_eq!(
        schema["properties"]["custom"]["description"],
        "A custom description"
    );
    assert_eq!(schema["properties"]["custom"]["format"], "custom");
    assert_eq!(
        schema["properties"]["mode"]["enum"],
        json!(["fast", "slow"])
    );
    assert_eq!(
        schema["properties"]["optional_mode"]["enum"],
        json!(["fast", "slow"])
    );
    assert_eq!(
        schema["properties"]["modes"]["items"]["enum"],
        json!(["fast", "slow"])
    );

    std::env::set_var("OP_TEST_NAME_UNIQUE", "N");
    let value = json!({"id":"abc","suffix":"tail","limit":7,"tags":["a","b"],"name":"N","mode":"slow","optional_mode":"fast","modes":["fast","slow"],"dry_run":true,"custom":"custom"});
    let expected = Params {
        id: "abc".into(),
        suffix: Some("tail".into()),
        limit: 7,
        role: "guest".into(),
        tags: vec!["a".into(), "b".into()],
        name: Some("N".into()),
        mode: Mode::Slow,
        optional_mode: Some(Mode::Fast),
        modes: vec![Mode::Fast, Mode::Slow],
        dry_run: true,
        query: None,
        custom: "custom".into(),
    };
    assert_eq!(Params::from_json(value).unwrap(), expected);
    let missing =
        Params::from_json(json!({"mode":"fast","suffix":"tail","custom":"x"})).unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&missing).code,
        "op_registry.usage.missing"
    );
    assert!(missing.to_string().contains("id"));
    assert!(
        Params::from_json(json!({"id":"a","suffix":"b","mode":"fast","custom":"x","bad":1}))
            .unwrap_err()
            .to_string()
            .contains("bad")
    );

    let matches = Params::clap_args(clap::Command::new("t"))
        .try_get_matches_from([
            "t",
            "abc",
            "tail",
            "--tags",
            "a,b",
            "--mode",
            "slow",
            "--optional-mode",
            "fast",
            "--modes",
            "fast,slow",
            "-l",
            "7",
            "--dry-run",
            "--custom",
            "custom",
        ])
        .unwrap();
    assert_eq!(Params::from_matches(&matches).unwrap(), expected);
    let matches = Params::clap_args(clap::Command::new("t"))
        .try_get_matches_from([
            "t", "abc", "tail", "--mode", "fast", "--query", "q", "--custom", "custom",
        ])
        .unwrap();
    let result = Params::from_matches(&matches).unwrap();
    assert_eq!(result.role, "guest");
    assert_eq!(result.query.as_deref(), Some("q"));
    assert!(Params::from_json(json!({"id":"a","suffix":"b","mode":"bad","custom":"x"})).is_err());
}

#[test]
fn required_positional_is_reported_by_clap() {
    let error = RequiredId::clap_args(clap::Command::new("t"))
        .try_get_matches_from(["t"])
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    assert!(error.to_string().contains("<id>"));
}

#[test]
fn numeric_vectors_negative_defaults_and_hidden_enum_variants_work() {
    let json_value = json!({"ids": [1, 2]});
    let from_json = NumericParams::from_json(json_value).unwrap();
    assert_eq!(
        from_json,
        NumericParams {
            ids: vec![1, 2],
            offset: -1
        }
    );

    let matches = NumericParams::clap_args(clap::Command::new("t"))
        .try_get_matches_from(["t", "--ids", "1,2"])
        .unwrap();
    assert_eq!(NumericParams::from_matches(&matches).unwrap(), from_json);
    assert!(NumericParams::clap_args(clap::Command::new("t"))
        .try_get_matches_from(["t", "--ids", "bad"])
        .is_err());

    assert_eq!(<VisibleMode as ValueEnum>::value_variants().len(), 1);
    let schema = {
        #[allow(dead_code)]
        #[derive(OpParams)]
        struct HiddenParams {
            #[op(value_enum)]
            mode: VisibleMode,
        }
        HiddenParams::schema()
    };
    assert_eq!(schema["properties"]["mode"]["enum"], json!(["fast"]));
}

#[test]
fn conditional_target_missing_agrees_between_json_and_clap() {
    let json_error = ConditionalParams::from_json(json!({"other": "present"})).unwrap_err();
    assert!(json_error.to_string().contains("target"));
    let matches = ConditionalParams::clap_args(clap::Command::new("t"))
        .try_get_matches_from(["t", "--other", "present"])
        .unwrap();
    let clap_error = ConditionalParams::from_matches(&matches).unwrap_err();
    assert!(clap_error.to_string().contains("target"));
}

#[derive(Clone, Debug, Deserialize, PartialEq, OpParams)]
struct PlainBoolParams {
    flag: bool,
}

#[test]
fn plain_bool_field_absent_is_false_in_both_json_and_clap() {
    // JSON: absent plain bool should default to false (not MissingField)
    let result = PlainBoolParams::from_json(json!({})).unwrap();
    assert!(!result.flag);

    // Clap: absent bool should be false
    let matches = PlainBoolParams::clap_args(clap::Command::new("t"))
        .try_get_matches_from(["t"])
        .unwrap();
    let result = PlainBoolParams::from_matches(&matches).unwrap();
    assert!(!result.flag);

    // Both paths should agree
    let json_result = PlainBoolParams::from_json(json!({})).unwrap();
    let clap_result = PlainBoolParams::from_matches(
        &PlainBoolParams::clap_args(clap::Command::new("t"))
            .try_get_matches_from(["t"])
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json_result, clap_result);
}

#[test]
fn plain_bool_field_is_not_required_in_schema() {
    let schema = PlainBoolParams::schema();
    let required = &schema["required"];
    // flag should not be in required array
    assert!(!required
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str() == Some("flag")));
}

#[derive(Clone, Debug, Deserialize, PartialEq, OpParams)]
struct NegativeNumberParams {
    #[op(default = -1)]
    offset: i32,
    #[op(default = -1.5)]
    threshold: f64,
}

#[test]
fn negative_numbers_work_in_clap_with_negative_flag() {
    // JSON with negative numbers
    let json_result =
        NegativeNumberParams::from_json(json!({"offset": -5, "threshold": -2.5})).unwrap();
    assert_eq!(json_result.offset, -5);
    assert_eq!(json_result.threshold, -2.5);

    // Clap with negative numbers should work
    let matches = NegativeNumberParams::clap_args(clap::Command::new("t"))
        .try_get_matches_from(["t", "--offset", "-5", "--threshold", "-2.5"])
        .unwrap();
    let clap_result = NegativeNumberParams::from_matches(&matches).unwrap();
    assert_eq!(clap_result.offset, -5);
    assert_eq!(clap_result.threshold, -2.5);

    // Both paths should agree
    assert_eq!(json_result, clap_result);
}

#[derive(Clone, Debug, Deserialize, PartialEq, OpParams)]
struct OptionalBoolParams {
    flag: Option<bool>,
}

#[test]
fn option_bool_absent_is_none_in_both_json_and_clap() {
    // JSON: absent Option<bool> should be None
    let json_result = OptionalBoolParams::from_json(json!({})).unwrap();
    assert_eq!(json_result.flag, None);

    // Clap: absent Option<bool> should be None (not Some(false))
    let matches = OptionalBoolParams::clap_args(clap::Command::new("t"))
        .try_get_matches_from(["t"])
        .unwrap();
    let clap_result = OptionalBoolParams::from_matches(&matches).unwrap();
    assert_eq!(clap_result.flag, None);

    // Both paths should agree
    assert_eq!(json_result, clap_result);
}

#[test]
fn option_bool_present_is_some_true_in_both_json_and_clap() {
    // JSON: present Option<bool> should be Some(true)
    let json_result = OptionalBoolParams::from_json(json!({"flag": true})).unwrap();
    assert_eq!(json_result.flag, Some(true));

    // Clap: present flag should be Some(true)
    let matches = OptionalBoolParams::clap_args(clap::Command::new("t"))
        .try_get_matches_from(["t", "--flag"])
        .unwrap();
    let clap_result = OptionalBoolParams::from_matches(&matches).unwrap();
    assert_eq!(clap_result.flag, Some(true));

    // Both paths should agree
    assert_eq!(json_result, clap_result);
}

#[derive(Clone, Debug, Deserialize, PartialEq, ValueEnum)]
enum Speed {
    Fast,
    Slow,
}

#[derive(Clone, Debug, Deserialize, PartialEq, OpParams)]
struct EnumDefaultParams {
    #[op(value_enum, default = "Fast")]
    speed: Speed,
}

#[test]
fn value_enum_with_default_uses_valueenum_name_not_serde_name() {
    // JSON: absent value_enum with default should use the ValueEnum name (kebab-case)
    let json_result = EnumDefaultParams::from_json(json!({})).unwrap();
    assert_eq!(json_result.speed, Speed::Fast);

    // JSON: explicitly set via ValueEnum name (kebab-case)
    let json_result_explicit = EnumDefaultParams::from_json(json!({"speed": "fast"})).unwrap();
    assert_eq!(json_result_explicit.speed, Speed::Fast);

    // Clap: absent with default should give Fast
    let matches = EnumDefaultParams::clap_args(clap::Command::new("t"))
        .try_get_matches_from(["t"])
        .unwrap();
    let clap_result = EnumDefaultParams::from_matches(&matches).unwrap();
    assert_eq!(clap_result.speed, Speed::Fast);

    // Clap: with explicit value
    let matches = EnumDefaultParams::clap_args(clap::Command::new("t"))
        .try_get_matches_from(["t", "--speed", "fast"])
        .unwrap();
    let clap_result_explicit = EnumDefaultParams::from_matches(&matches).unwrap();
    assert_eq!(clap_result_explicit.speed, Speed::Fast);

    // All paths should agree
    assert_eq!(json_result, clap_result);
}
