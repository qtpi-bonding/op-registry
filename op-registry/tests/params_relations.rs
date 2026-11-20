mod common;

use std::path::PathBuf;

use clap::{error::ErrorKind, Command};
use common::{Ctx, Out};
use futures_util::future::BoxFuture;
use op_registry::{CallOptions, OpParams, OpSpec, ParseCx, Patch, Registry};
use serde_json::{json, Value};

fn cli<P: OpParams>(args: &[&str]) -> Result<anyhow::Result<P>, clap::Error> {
    let mut argv = vec!["t"];
    argv.extend_from_slice(args);
    let matches = P::add_args(Command::new("t")).try_get_matches_from(argv)?;
    Ok(P::from_matches(&matches, &mut ParseCx::default()))
}

fn json_of<P: OpParams>(value: Value) -> anyhow::Result<P> {
    P::from_json(value, &mut ParseCx::default())
}

fn code<T>(r: anyhow::Result<T>) -> &'static str {
    coded_error::facts_anyhow(&r.err().expect("expected an error")).code
}

#[derive(Debug, PartialEq, OpParams)]
struct Target {
    #[op(one_of = "target")]
    id: Option<String>,
    #[op(one_of = "target")]
    name: Option<String>,
}

#[derive(Debug, PartialEq, OpParams)]
struct Maybe {
    #[op(one_of_optional = "pick")]
    id: Option<String>,
    #[op(one_of_optional = "pick")]
    name: Option<String>,
}

#[derive(Debug, PartialEq, OpParams)]
struct Lookup {
    uri: Option<String>,
    #[op(requires_if = "uri")]
    collection: Option<String>,
}

#[derive(Debug, PartialEq, OpParams)]
struct Update {
    #[op(clearable)]
    host_machine: Patch<String>,
}

#[derive(Debug, PartialEq, OpParams)]
struct ProjectArgs {
    /// The project root.
    root: Option<PathBuf>,
    gcode_store: Option<String>,
}

#[derive(Debug, PartialEq, OpParams)]
struct Wrapped {
    name: String,
    #[op(flatten)]
    project: ProjectArgs,
}

#[derive(OpParams)]
#[allow(dead_code)]
struct Clashing {
    root: Option<String>,
    #[op(flatten)]
    project: ProjectArgs,
}

#[test]
fn one_of_exactly_one_cli_and_json() {
    assert_eq!(
        code(json_of::<Target>(json!({}))),
        "op_registry.usage.one_of"
    );
    assert_eq!(
        code(json_of::<Target>(json!({"id":"a","name":"b"}))),
        "op_registry.usage.one_of"
    );
    assert_eq!(
        json_of::<Target>(json!({"id":"a"})).unwrap(),
        Target { id: Some("a".into()), name: None }
    );
    assert_eq!(cli::<Target>(&[]).unwrap_err().kind(), ErrorKind::MissingRequiredArgument);
    assert_eq!(
        cli::<Target>(&["--id", "a", "--name", "b"]).unwrap_err().kind(),
        ErrorKind::ArgumentConflict
    );
    assert_eq!(
        cli::<Target>(&["--name", "b"]).unwrap().unwrap(),
        Target { id: None, name: Some("b".into()) }
    );
}

#[test]
fn one_of_optional_allows_none() {
    assert_eq!(
        cli::<Maybe>(&[]).unwrap().unwrap(),
        Maybe { id: None, name: None }
    );
    assert_eq!(json_of::<Maybe>(json!({})).unwrap(), Maybe { id: None, name: None });
    assert_eq!(
        cli::<Maybe>(&["--id", "a", "--name", "b"]).unwrap_err().kind(),
        ErrorKind::ArgumentConflict
    );
    assert_eq!(
        code(json_of::<Maybe>(json!({"id":"a","name":"b"}))),
        "op_registry.usage.one_of"
    );
}

#[test]
fn one_of_schema_has_oneof() {
    let schema = Target::input_schema();
    assert_eq!(
        schema["oneOf"],
        json!([{"required":["id"]},{"required":["name"]}])
    );
    let schema = Maybe::input_schema();
    let alts = schema["oneOf"].as_array().unwrap();
    assert_eq!(alts.len(), 3);
    assert_eq!(alts[0], json!({"required":["id"]}));
    assert_eq!(
        alts[2],
        json!({"not":{"anyOf":[{"required":["id"]},{"required":["name"]}]}})
    );
}

#[test]
fn requires_if_missing_is_usage_error() {
    assert_eq!(
        code(json_of::<Lookup>(json!({"uri":"x"}))),
        "op_registry.usage.requires_if"
    );
    let ok = json_of::<Lookup>(json!({"uri":"x","collection":"c"})).unwrap();
    assert_eq!(ok.collection.as_deref(), Some("c"));
    assert!(json_of::<Lookup>(json!({})).is_ok());
    assert_eq!(
        code(cli::<Lookup>(&["--uri", "x"]).unwrap()),
        "op_registry.usage.requires_if"
    );
    assert!(cli::<Lookup>(&["--uri", "x", "--collection", "c"]).unwrap().is_ok());
}

#[test]
fn requires_if_schema_dependent_required() {
    assert_eq!(
        Lookup::input_schema()["dependentRequired"]["uri"],
        json!(["collection"])
    );
}

#[test]
fn clearable_cli_three_states() {
    let host = |args: &[&str]| cli::<Update>(args).unwrap().unwrap().host_machine;
    assert_eq!(host(&["--host-machine", "m"]), Patch::Set("m".to_owned()));
    assert_eq!(host(&["--clear-host-machine"]), Patch::Clear);
    assert_eq!(host(&[]), Patch::Keep);
    assert_eq!(
        cli::<Update>(&["--host-machine", "m", "--clear-host-machine"])
            .unwrap_err()
            .kind(),
        ErrorKind::ArgumentConflict
    );
}

#[test]
fn clearable_json_null_is_clear_absent_is_keep() {
    let host = |v: Value| json_of::<Update>(v).unwrap().host_machine;
    assert_eq!(host(json!({"hostMachine":null})), Patch::Clear);
    assert_eq!(host(json!({})), Patch::Keep);
    assert_eq!(host(json!({"hostMachine":"m"})), Patch::Set("m".to_owned()));
    assert_eq!(
        Update::input_schema()["properties"]["hostMachine"]["type"],
        json!(["string", "null"])
    );
}

#[test]
fn flatten_adds_group_flags_and_keys() {
    let parsed = cli::<Wrapped>(&["--name", "n", "--gcode-store", "s"]).unwrap().unwrap();
    assert_eq!(parsed.project.gcode_store.as_deref(), Some("s"));
    let parsed = json_of::<Wrapped>(json!({"name":"n","gcodeStore":"s","root":"/r"})).unwrap();
    assert_eq!(parsed.project.gcode_store.as_deref(), Some("s"));
    assert_eq!(parsed.project.root, Some(PathBuf::from("/r")));
    assert_eq!(
        code(json_of::<Wrapped>(json!({"name":"n","nope":1}))),
        "op_registry.usage.unknown_key"
    );
    let schema = Wrapped::input_schema();
    assert!(schema["properties"]["gcodeStore"].is_object());
    assert_eq!(schema["required"], json!(["name"]));
    let fields = Wrapped::fields();
    let own = fields.iter().find(|f| f.key == "name").unwrap();
    assert_eq!(own.from_group, None);
    let grouped = fields.iter().find(|f| f.key == "gcodeStore").unwrap();
    assert_eq!(grouped.from_group, Some("ProjectArgs"));
    assert_eq!(grouped.flag, "gcode-store");
    assert!(Wrapped::add_args(Command::new("t"))
        .get_arguments()
        .any(|a| a.get_long() == Some("gcode-store")));
}

#[test]
fn flatten_carries_group_relations() {
    #[derive(OpParams)]
    struct Grouped {
        #[op(one_of = "g")]
        a: Option<String>,
        #[op(one_of = "g")]
        b: Option<String>,
    }
    #[derive(OpParams)]
    struct Outer {
        #[op(flatten)]
        inner: Grouped,
        x: Option<String>,
        #[op(requires_if = "x")]
        y: Option<String>,
    }
    assert_eq!(code(json_of::<Outer>(json!({}))), "op_registry.usage.one_of");
    assert_eq!(
        code(json_of::<Outer>(json!({"a":"1","x":"v"}))),
        "op_registry.usage.requires_if"
    );
    assert!(json_of::<Outer>(json!({"a":"1","x":"v","y":"w"})).is_ok());
    let schema = Outer::input_schema();
    assert_eq!(schema["oneOf"].as_array().unwrap().len(), 2);
    assert_eq!(schema["dependentRequired"]["x"], json!(["y"]));
}

#[test]
#[should_panic(expected = "root")]
fn flatten_clash_panics_at_push() {
    let mut registry: Registry<Ctx> = Registry::new();
    registry.push_op(
        OpSpec::<Clashing, Out>::new("clash", "Clash."),
        |_c: &Ctx, _p: Clashing, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async { Ok(Out { ok: true }) })
        },
    );
}

#[test]
fn flatten_without_clash_pushes() {
    let mut registry: Registry<Ctx> = Registry::new();
    registry.push_op(
        OpSpec::<Wrapped, Out>::new("fine", "Fine."),
        |_c: &Ctx, _p: Wrapped, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async { Ok(Out { ok: true }) })
        },
    );
    assert!(registry.get("fine").is_some());
}
