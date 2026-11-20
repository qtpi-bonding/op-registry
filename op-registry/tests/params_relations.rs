mod common;

use std::path::PathBuf;

use clap::{error::ErrorKind, Command};
use common::{Ctx, Out};
use futures_util::future::BoxFuture;
use op_registry::{
    cli::{self, Io, NoHooks},
    CallOptions, CliContext, FieldKind, Op, OpParams, OpSpec, ParseCx, Patch, Ref, Registry,
    Resolve, Resolved, Scope,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::marker::PhantomData;

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
        Target {
            id: Some("a".into()),
            name: None
        }
    );
    assert_eq!(
        cli::<Target>(&[]).unwrap_err().kind(),
        ErrorKind::MissingRequiredArgument
    );
    assert_eq!(
        cli::<Target>(&["--id", "a", "--name", "b"])
            .unwrap_err()
            .kind(),
        ErrorKind::ArgumentConflict
    );
    assert_eq!(
        cli::<Target>(&["--name", "b"]).unwrap().unwrap(),
        Target {
            id: None,
            name: Some("b".into())
        }
    );
}

#[test]
fn one_of_optional_allows_none() {
    assert_eq!(
        cli::<Maybe>(&[]).unwrap().unwrap(),
        Maybe {
            id: None,
            name: None
        }
    );
    assert_eq!(
        json_of::<Maybe>(json!({})).unwrap(),
        Maybe {
            id: None,
            name: None
        }
    );
    assert_eq!(
        cli::<Maybe>(&["--id", "a", "--name", "b"])
            .unwrap_err()
            .kind(),
        ErrorKind::ArgumentConflict
    );
    assert_eq!(
        code(json_of::<Maybe>(json!({"id":"a","name":"b"}))),
        "op_registry.usage.one_of"
    );
}

#[test]
fn one_of_schema_has_oneof() {
    let present = |k: &str| json!({"required":[k],"properties":{k:{"not":{"type":"null"}}}});
    let schema = Target::input_schema();
    assert_eq!(schema["oneOf"], json!([present("id"), present("name")]));
    let schema = Maybe::input_schema();
    let alts = schema["oneOf"].as_array().unwrap();
    assert_eq!(alts.len(), 3);
    assert_eq!(alts[0], present("id"));
    assert_eq!(
        alts[2],
        json!({"not":{"anyOf":[present("id"),present("name")]}})
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
    assert!(cli::<Lookup>(&["--uri", "x", "--collection", "c"])
        .unwrap()
        .is_ok());
}

#[test]
fn requires_if_schema_is_if_then() {
    let schema = Lookup::input_schema();
    assert_eq!(
        schema["if"],
        json!({"required":["uri"],"properties":{"uri":{"not":{"type":"null"}}}})
    );
    assert_eq!(
        schema["then"],
        json!({"required":["collection"],"properties":{"collection":{"not":{"type":"null"}}}})
    );
    assert!(schema.get("dependentRequired").is_none(), "{schema}");
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
    let parsed = cli::<Wrapped>(&["--name", "n", "--gcode-store", "s"])
        .unwrap()
        .unwrap();
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
    assert_eq!(
        code(json_of::<Outer>(json!({}))),
        "op_registry.usage.one_of"
    );
    assert_eq!(
        code(json_of::<Outer>(json!({"a":"1","x":"v"}))),
        "op_registry.usage.requires_if"
    );
    assert!(json_of::<Outer>(json!({"a":"1","x":"v","y":"w"})).is_ok());
    let schema = Outer::input_schema();
    let all = schema["allOf"].as_array().unwrap();
    assert_eq!(all.len(), 2, "{schema}");
    assert_eq!(all[0]["oneOf"].as_array().unwrap().len(), 2);
    assert!(all[1]["if"].is_object() && all[1]["then"].is_object());
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

// ---- Relations over any field with presence ----

/// A context that resolves every ref to itself, so `Ref` members run.
struct RCtx;
impl Resolve for RCtx {
    fn resolve<'a>(
        &'a self,
        _kind: &'a str,
        r: &'a Ref,
        _scope: &'a Scope,
    ) -> BoxFuture<'a, anyhow::Result<Resolved>> {
        Box::pin(async move { Ok(Resolved::Id(r.as_raw().to_owned())) })
    }
}

#[derive(op_registry::OpGlobals)]
struct NoGlobals {}

impl CliContext for RCtx {
    type Globals = NoGlobals;
    fn from_globals(_: &NoGlobals) -> anyhow::Result<Self> {
        Ok(RCtx)
    }
}

/// The typed handle of the op `go` over `P`.
struct Go<P>(PhantomData<fn() -> P>);
impl<P: OpParams> Op for Go<P> {
    type Ctx = RCtx;
    type Params = P;
    type Output = Out;
    const NAME: &'static str = "go";
}

fn registry_of<P: OpParams + Send + 'static>() -> Registry<RCtx> {
    let mut r: Registry<RCtx> = Registry::new();
    r.set_pillar("fix");
    r.set_bin("app", "1.0.0", "A test app.");
    r.push_op(
        OpSpec::<P, Out>::new("go", "Go."),
        |_c: &RCtx, _p: P, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async { Ok(Out { ok: true }) })
        },
    );
    r
}

/// Whether `go` accepts `args` on the command line.
fn on_cli<P: OpParams + Send + 'static>(args: &[&str]) -> bool {
    let argv: Vec<OsString> = ["app", "go"]
        .iter()
        .chain(args)
        .map(OsString::from)
        .collect();
    let (io, _cap) = Io::memory(b"", false, false);
    futures_executor::block_on(cli::run_with_io(&registry_of::<P>(), argv, &NoHooks, io)) == 0
}

/// Whether `go` accepts `args` as JSON.
fn on_json<P: OpParams + Send + 'static>(args: Value) -> bool {
    futures_executor::block_on(registry_of::<P>().call("go", args, &RCtx, CallOptions::default()))
        .is_ok()
}

/// Whether `go` accepts `params` as a typed call.
fn on_typed<P: OpParams + Send + 'static>(params: P) -> bool {
    futures_executor::block_on(registry_of::<P>().call_typed::<Go<P>>(
        &RCtx,
        params,
        CallOptions::default(),
    ))
    .is_ok()
}

#[derive(Debug, OpParams)]
struct IdOrAll {
    #[op(kind = "thing", one_of = "t")]
    id: Option<Ref>,
    #[op(one_of = "t")]
    all: bool,
}

fn id_or_all(id: Option<&str>, all: bool) -> IdOrAll {
    IdOrAll {
        id: id.map(Ref::new),
        all,
    }
}

#[test]
fn one_of_over_optional_ref_and_switch() {
    // CLI
    assert!(on_cli::<IdOrAll>(&["--id", "x"]));
    assert!(on_cli::<IdOrAll>(&["--all"]));
    assert!(!on_cli::<IdOrAll>(&["--id", "x", "--all"]));
    assert!(!on_cli::<IdOrAll>(&[]));
    // JSON
    assert!(on_json::<IdOrAll>(json!({"id": "x"})));
    assert!(on_json::<IdOrAll>(json!({"all": true})));
    assert!(!on_json::<IdOrAll>(json!({"id": "x", "all": true})));
    assert!(!on_json::<IdOrAll>(json!({})));
    assert!(on_json::<IdOrAll>(json!({"all": false, "id": "x"})));
    assert!(!on_json::<IdOrAll>(json!({"all": false})));
    // typed
    assert!(on_typed(id_or_all(Some("x"), false)));
    assert!(on_typed(id_or_all(None, true)));
    assert!(!on_typed(id_or_all(Some("x"), true)));
    assert!(!on_typed(id_or_all(None, false)));
}

#[derive(Debug, OpParams)]
struct TaggedReason {
    tags: Vec<String>,
    #[op(requires_if = "tags")]
    reason: Option<String>,
}

fn tagged(tags: &[&str], reason: Option<&str>) -> TaggedReason {
    TaggedReason {
        tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        reason: reason.map(str::to_owned),
    }
}

#[test]
fn requires_if_with_vec_target() {
    assert!(!on_cli::<TaggedReason>(&["--tags", "a"]));
    assert!(on_cli::<TaggedReason>(&["--tags", "a", "--reason", "r"]));
    assert!(on_cli::<TaggedReason>(&["--reason", "r"]));
    assert!(on_cli::<TaggedReason>(&[]));
    assert!(!on_json::<TaggedReason>(json!({"tags": ["a"]})));
    assert!(on_json::<TaggedReason>(
        json!({"tags": ["a"], "reason": "r"})
    ));
    // An empty list is absent, so it requires nothing.
    assert!(on_json::<TaggedReason>(json!({"tags": []})));
    assert!(on_json::<TaggedReason>(json!({})));
    assert!(!on_typed(tagged(&["a"], None)));
    assert!(on_typed(tagged(&["a"], Some("r"))));
    assert!(on_typed(tagged(&[], None)));
}

#[derive(Debug, OpParams)]
struct AnyOfTargets {
    a: Option<String>,
    b: bool,
    #[op(requires_if = ["a", "b"])]
    f: Option<String>,
}

fn any_of_targets(a: Option<&str>, b: bool, f: Option<&str>) -> AnyOfTargets {
    AnyOfTargets {
        a: a.map(str::to_owned),
        b,
        f: f.map(str::to_owned),
    }
}

#[test]
fn requires_if_list_targets() {
    assert!(!on_cli::<AnyOfTargets>(&["--a", "1"]));
    assert!(!on_cli::<AnyOfTargets>(&["--b"]));
    assert!(on_cli::<AnyOfTargets>(&["--b", "--f", "x"]));
    assert!(on_cli::<AnyOfTargets>(&["--a", "1", "--f", "x"]));
    assert!(on_cli::<AnyOfTargets>(&[]));
    assert!(on_cli::<AnyOfTargets>(&["--f", "x"]));
    assert!(!on_json::<AnyOfTargets>(json!({"a": "1"})));
    assert!(!on_json::<AnyOfTargets>(json!({"b": true})));
    assert!(on_json::<AnyOfTargets>(json!({"b": true, "f": "x"})));
    assert!(on_json::<AnyOfTargets>(json!({"b": false})));
    assert!(on_json::<AnyOfTargets>(json!({})));
    assert!(!on_typed(any_of_targets(Some("1"), false, None)));
    assert!(!on_typed(any_of_targets(None, true, None)));
    assert!(on_typed(any_of_targets(None, true, Some("x"))));
    assert!(on_typed(any_of_targets(None, false, None)));
}

#[derive(Debug, OpParams)]
struct Newest {
    #[op(one_of_optional = "pick")]
    newest: bool,
    #[op(one_of_optional = "pick")]
    id: Option<String>,
    #[op(one_of_optional = "pick")]
    name: Option<String>,
}

fn newest(newest: bool, id: Option<&str>, name: Option<&str>) -> Newest {
    Newest {
        newest,
        id: id.map(str::to_owned),
        name: name.map(str::to_owned),
    }
}

#[test]
fn one_of_optional_over_switch_and_two_options() {
    assert!(on_cli::<Newest>(&[]));
    assert!(on_cli::<Newest>(&["--newest"]));
    assert!(on_cli::<Newest>(&["--id", "a"]));
    assert!(!on_cli::<Newest>(&["--newest", "--id", "a"]));
    assert!(!on_cli::<Newest>(&["--id", "a", "--name", "b"]));
    assert!(on_json::<Newest>(json!({})));
    assert!(on_json::<Newest>(json!({"newest": false})));
    assert!(on_json::<Newest>(json!({"newest": true})));
    assert!(on_json::<Newest>(json!({"newest": false, "name": "b"})));
    assert!(!on_json::<Newest>(json!({"newest": true, "name": "b"})));
    assert!(!on_json::<Newest>(json!({"id": "a", "name": "b"})));
    assert!(on_typed(newest(false, None, None)));
    assert!(on_typed(newest(true, None, None)));
    assert!(on_typed(newest(false, None, Some("b"))));
    assert!(!on_typed(newest(true, Some("a"), None)));
}

// ---- Schemas match validate ----

#[derive(Debug, OpParams)]
struct VecOrAll {
    #[op(kind = "thing", one_of = "t")]
    ids: Vec<Ref>,
    #[op(one_of = "t")]
    all: bool,
}

#[derive(Debug, OpParams)]
struct VecField {
    reason: Option<String>,
    #[op(requires_if = "reason")]
    tags: Vec<String>,
}

#[derive(Debug, OpParams)]
struct EveryShape {
    #[op(clearable, one_of_optional = "g")]
    note: Patch<String>,
    #[op(one_of_optional = "g")]
    labels: BTreeMap<String, String>,
    #[op(one_of_optional = "g")]
    flag: Option<bool>,
}

/// One candidate value of a key; `None` leaves the key out.
type Candidates = (&'static str, Vec<Option<Value>>);

/// Every input over the cross product of the candidates.
fn table(keys: &[Candidates]) -> Vec<Value> {
    let mut inputs = vec![serde_json::Map::new()];
    for (key, candidates) in keys {
        let mut next = Vec::new();
        for input in &inputs {
            for candidate in candidates {
                let mut input = input.clone();
                if let Some(v) = candidate {
                    input.insert((*key).to_owned(), v.clone());
                }
                next.push(input);
            }
        }
        inputs = next;
    }
    inputs.into_iter().map(Value::Object).collect()
}

fn some(values: &[Value]) -> Vec<Option<Value>> {
    std::iter::once(None)
        .chain(values.iter().cloned().map(Some))
        .collect()
}

/// The schema accepts an input exactly when `from_json` (which validates) does.
fn assert_agrees<P: OpParams>(keys: &[Candidates]) {
    let schema = P::input_schema();
    assert!(jsonschema::meta::is_valid(&schema), "{schema}");
    for input in table(keys) {
        let decoded = P::from_json(input.clone(), &mut ParseCx::default());
        assert_eq!(
            jsonschema::is_valid(&schema, &input),
            decoded.is_ok(),
            "schema and validate disagree on {input}\nschema: {schema}\nvalidate: {:?}",
            decoded.err()
        );
    }
}

#[test]
fn schema_agrees_with_validate() {
    let switch = some(&[json!(false), json!(true), Value::Null]);
    let option = some(&[json!("x"), Value::Null]);
    let list = some(&[json!([]), json!(["a"]), Value::Null]);
    assert_agrees::<IdOrAll>(&[("id", option.clone()), ("all", switch.clone())]);
    assert_agrees::<VecOrAll>(&[("ids", list.clone()), ("all", switch.clone())]);
    assert_agrees::<Target>(&[("id", option.clone()), ("name", option.clone())]);
    assert_agrees::<Maybe>(&[("id", option.clone()), ("name", option.clone())]);
    assert_agrees::<Lookup>(&[("uri", option.clone()), ("collection", option.clone())]);
    assert_agrees::<TaggedReason>(&[("tags", list.clone()), ("reason", option.clone())]);
    assert_agrees::<VecField>(&[("tags", list.clone()), ("reason", option.clone())]);
    assert_agrees::<AnyOfTargets>(&[
        ("a", option.clone()),
        ("b", switch.clone()),
        ("f", option.clone()),
    ]);
    assert_agrees::<Newest>(&[
        ("newest", switch.clone()),
        ("id", option.clone()),
        ("name", option.clone()),
    ]);
    assert_agrees::<EveryShape>(&[
        ("note", some(&[json!("x"), Value::Null])),
        ("labels", some(&[json!({}), json!({"k": "v"}), Value::Null])),
        ("flag", some(&[json!(true), json!(false), Value::Null])),
    ]);
}

#[test]
fn vec_ref_member_schema_agrees() {
    let present_ids = op_registry::__private::rt::present_schema("ids", FieldKind::Repeated);
    assert_eq!(
        present_ids,
        json!({"required":["ids"],"properties":{"ids":{"type":"array","minItems":1}}})
    );
    let schema = VecOrAll::input_schema();
    assert_eq!(schema["oneOf"][0], present_ids);
    // An empty list is absent in the schema and in validate.
    assert!(!jsonschema::is_valid(&schema, &json!({"ids": []})));
    assert_eq!(
        code(json_of::<VecOrAll>(json!({"ids": []}))),
        "op_registry.usage.one_of"
    );
    assert!(jsonschema::is_valid(&schema, &json!({"ids": ["a"]})));
    assert!(json_of::<VecOrAll>(json!({"ids": ["a"]})).is_ok());
    assert!(jsonschema::is_valid(
        &schema,
        &json!({"ids": [], "all": true})
    ));
    assert!(json_of::<VecOrAll>(json!({"ids": [], "all": true})).is_ok());
}

#[derive(OpParams)]
struct OnlyRequires {
    a: Option<String>,
    #[op(requires_if = "a")]
    b: Option<String>,
}

#[test]
fn schema_with_single_non_one_of_constraint_is_valid() {
    let schema = OnlyRequires::input_schema();
    assert!(schema.get("oneOf").is_none(), "{schema}");
    assert!(schema["if"].is_object() && schema["then"].is_object());
    assert!(jsonschema::meta::is_valid(&schema), "{schema}");
    assert!(!jsonschema::is_valid(&schema, &json!({"a": "1"})));
    assert!(jsonschema::is_valid(&schema, &json!({"a": "1", "b": "2"})));
}

#[test]
fn flattened_any_of_survives_absorb() {
    use op_registry::__private::rt::SchemaBuilder;
    let any_of = json!([{"required":["a"]},{"required":["b"]}]);
    let group = json!({
        "type": "object",
        "properties": {"a": {}, "b": {}},
        "anyOf": any_of,
    });
    let mut builder = SchemaBuilder::default();
    builder.absorb(group.clone());
    assert_eq!(builder.build()["anyOf"], any_of);

    // `not`, `if`/`then` and every `allOf` member survive too, next to another constraint.
    let mut builder = SchemaBuilder::default();
    builder.absorb(group);
    builder.absorb(json!({"not": {"required": ["c"]}}));
    builder.absorb(json!({"if": {"required": ["a"]}, "then": {"required": ["b"]}}));
    builder.absorb(json!({"allOf": [{"required": ["d"]}, {"required": ["e"]}]}));
    let schema = builder.build();
    let all = schema["allOf"].as_array().unwrap();
    assert_eq!(all.len(), 5, "{schema}");
    assert_eq!(all[0]["anyOf"], any_of);
    assert_eq!(all[1], json!({"not": {"required": ["c"]}}));
    assert_eq!(
        all[2],
        json!({"if": {"required": ["a"]}, "then": {"required": ["b"]}})
    );
    assert_eq!(all[3], json!({"required": ["d"]}));
}
