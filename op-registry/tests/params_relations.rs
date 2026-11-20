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
    assert_agrees::<ReqVec>(&[("to", list.clone())]);
    assert_agrees::<ReqOptBool>(&[("visible", some(&[json!(true), json!(false), Value::Null]))]);
    assert_agrees::<ReqOptStr>(&[("name", option.clone())]);
    assert_agrees::<ReqJson>(&[("data", some(&[json!({"a": 1}), Value::Null]))]);
    assert_agrees::<MessageSubmit>(&[("to", list.clone()), ("signedFile", option.clone())]);
    assert_agrees::<Confirm>(&[
        ("dryRun", switch.clone()),
        ("confirmFingerprint", option.clone()),
    ]);
    assert_agrees::<GraphSearch>(&[
        ("name", option.clone()),
        ("file", option.clone()),
        ("nodeKind", option.clone()),
    ]);
    assert_agrees::<AddDevice>(&[
        ("confirmFingerprint", option.clone()),
        ("ownerKeyFile", option.clone()),
        ("dryRun", switch.clone()),
    ]);
    assert_agrees::<Forced>(&[
        ("force", option.clone()),
        ("note", some(&[json!("x"), Value::Null])),
        ("flag", some(&[json!(true), json!(false), Value::Null])),
    ]);
    assert_agrees::<ConflictsOnly>(&[("a", option.clone()), ("b", list.clone())]);
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

// ---- required, conflicts_with, required_unless, at_least_one, dotted targets ----

/// The exit code `go` returns on `args`.
fn exit_of<P: OpParams + Send + 'static>(args: &[&str]) -> u8 {
    let argv: Vec<OsString> = ["app", "go"]
        .iter()
        .chain(args)
        .map(OsString::from)
        .collect();
    let (io, _cap) = Io::memory(b"", false, false);
    futures_executor::block_on(cli::run_with_io(&registry_of::<P>(), argv, &NoHooks, io))
}

#[derive(Debug, OpParams)]
struct ReqVec {
    #[op(required)]
    to: Vec<String>,
}

#[derive(Debug, OpParams)]
struct ReqOptBool {
    #[op(required)]
    visible: Option<bool>,
}

#[derive(Debug, OpParams)]
struct ReqOptStr {
    #[op(required)]
    name: Option<String>,
}

#[derive(Debug, OpParams)]
struct ReqJson {
    #[op(required)]
    data: Option<op_registry::Json>,
}

#[test]
fn required_vec_shapes() {
    assert_eq!(exit_of::<ReqVec>(&[]), 2);
    assert_eq!(exit_of::<ReqVec>(&["--to", "a"]), 0);
    assert!(on_cli::<ReqVec>(&["--to", "a", "--to", "b"]));
    assert_eq!(
        cli::<ReqVec>(&[]).unwrap_err().kind(),
        ErrorKind::MissingRequiredArgument
    );
    for input in [json!({}), json!({"to": []}), json!({"to": null})] {
        assert_eq!(
            code(json_of::<ReqVec>(input.clone())),
            "op_registry.usage.missing",
            "{input}"
        );
        assert!(!on_json::<ReqVec>(input));
    }
    assert!(on_json::<ReqVec>(json!({"to": ["a"]})));
    assert!(!on_typed(ReqVec { to: vec![] }));
    assert!(on_typed(ReqVec {
        to: vec!["a".into()]
    }));
    let schema = ReqVec::input_schema();
    assert_eq!(schema["required"], json!(["to"]));
    assert_eq!(schema["properties"]["to"]["minItems"], json!(1));
    assert!(ReqVec::fields()[0].required);
}

#[test]
fn required_option_shapes() {
    assert_eq!(exit_of::<ReqOptStr>(&[]), 2);
    assert_eq!(exit_of::<ReqOptStr>(&["--name", "n"]), 0);
    assert!(!on_json::<ReqOptStr>(json!({})));
    assert!(!on_json::<ReqOptStr>(json!({"name": null})));
    assert!(on_json::<ReqOptStr>(json!({"name": "n"})));
    assert!(!on_typed(ReqOptStr { name: None }));
    assert!(on_typed(ReqOptStr {
        name: Some("n".into())
    }));
    // Any JSON value, but not `null`.
    assert!(!on_json::<ReqJson>(json!({"data": null})));
    assert!(on_json::<ReqJson>(json!({"data": [1]})));
    assert!(ReqOptStr::fields()[0].required);
}

#[test]
fn required_option_bool_missing_on_mcp_is_missing() {
    // Today an omitted `Option<bool>` silently became `false`.
    assert_eq!(
        code(json_of::<ReqOptBool>(json!({}))),
        "op_registry.usage.missing"
    );
    assert_eq!(
        code(json_of::<ReqOptBool>(json!({"visible": null}))),
        "op_registry.usage.missing"
    );
    assert!(!on_json::<ReqOptBool>(json!({})));
    assert!(on_json::<ReqOptBool>(json!({"visible": false})));
    assert!(on_json::<ReqOptBool>(json!({"visible": true})));
    // CLI: a required group of --visible / --no-visible.
    assert_eq!(exit_of::<ReqOptBool>(&[]), 2);
    assert_eq!(exit_of::<ReqOptBool>(&["--visible"]), 0);
    assert_eq!(exit_of::<ReqOptBool>(&["--no-visible"]), 0);
    assert_eq!(exit_of::<ReqOptBool>(&["--visible", "--no-visible"]), 2);
    assert!(!on_typed(ReqOptBool { visible: None }));
    assert!(on_typed(ReqOptBool {
        visible: Some(false)
    }));
}

#[derive(Debug, OpParams)]
struct MessageSubmit {
    #[op(required_unless = "signed_file")]
    to: Vec<String>,
    #[op(conflicts_with = "to")]
    signed_file: Option<PathBuf>,
}

fn submit(to: &[&str], signed: Option<&str>) -> MessageSubmit {
    MessageSubmit {
        to: to.iter().map(|t| (*t).to_owned()).collect(),
        signed_file: signed.map(PathBuf::from),
    }
}

#[test]
fn message_submit_shape() {
    // CLI
    assert_eq!(exit_of::<MessageSubmit>(&["--to", "a"]), 0);
    assert_eq!(exit_of::<MessageSubmit>(&["--signed-file", "f"]), 0);
    assert_eq!(exit_of::<MessageSubmit>(&[]), 2);
    assert_eq!(
        exit_of::<MessageSubmit>(&["--to", "a", "--signed-file", "f"]),
        2
    );
    // JSON
    assert!(on_json::<MessageSubmit>(json!({"to": ["a"]})));
    assert!(on_json::<MessageSubmit>(json!({"signedFile": "f"})));
    assert!(on_json::<MessageSubmit>(
        json!({"to": [], "signedFile": "f"})
    ));
    assert_eq!(
        code(json_of::<MessageSubmit>(json!({}))),
        "op_registry.usage.missing"
    );
    assert_eq!(
        code(json_of::<MessageSubmit>(
            json!({"to": ["a"], "signedFile": "f"})
        )),
        "op_registry.usage.conflicts"
    );
    // typed
    assert!(on_typed(submit(&["a"], None)));
    assert!(on_typed(submit(&[], Some("f"))));
    assert!(!on_typed(submit(&[], None)));
    assert!(!on_typed(submit(&["a"], Some("f"))));
    // The messages name the fields.
    let err = json_of::<MessageSubmit>(json!({"to": ["a"], "signedFile": "f"})).unwrap_err();
    assert_eq!(err.to_string(), "signedFile conflicts with to");
    let err = json_of::<MessageSubmit>(json!({})).unwrap_err();
    assert_eq!(err.to_string(), "to is required unless signedFile is given");
}

#[derive(Debug, OpParams)]
struct Confirm {
    dry_run: bool,
    #[op(required_unless = "dry_run")]
    confirm_fingerprint: Option<String>,
}

#[test]
fn confirm_fingerprint_required_unless_dry_run() {
    assert_eq!(exit_of::<Confirm>(&["--dry-run"]), 0);
    assert_eq!(exit_of::<Confirm>(&["--confirm-fingerprint", "f"]), 0);
    assert_eq!(exit_of::<Confirm>(&[]), 2);
    assert!(on_json::<Confirm>(json!({"dryRun": true})));
    assert!(on_json::<Confirm>(json!({"confirmFingerprint": "f"})));
    // `false` is absent, so it does not excuse the fingerprint.
    assert!(!on_json::<Confirm>(json!({"dryRun": false})));
    assert!(!on_json::<Confirm>(json!({})));
    assert!(on_typed(Confirm {
        dry_run: true,
        confirm_fingerprint: None
    }));
    assert!(!on_typed(Confirm {
        dry_run: false,
        confirm_fingerprint: None
    }));
}

#[derive(Debug, OpParams)]
struct GraphSearch {
    #[op(at_least_one = "query")]
    name: Option<String>,
    #[op(at_least_one = "query")]
    file: Option<PathBuf>,
    #[op(at_least_one = "query")]
    node_kind: Option<String>,
}

#[test]
fn at_least_one_graph_search() {
    assert_eq!(exit_of::<GraphSearch>(&[]), 2);
    assert_eq!(exit_of::<GraphSearch>(&["--name", "n"]), 0);
    assert_eq!(
        exit_of::<GraphSearch>(&["--node-kind", "k", "--file", "f"]),
        0
    );
    assert_eq!(
        exit_of::<GraphSearch>(&["--name", "n", "--file", "f", "--node-kind", "k"]),
        0
    );
    assert_eq!(
        code(json_of::<GraphSearch>(json!({}))),
        "op_registry.usage.at_least_one"
    );
    assert_eq!(
        code(json_of::<GraphSearch>(json!({"name": null}))),
        "op_registry.usage.at_least_one"
    );
    assert!(on_json::<GraphSearch>(json!({"file": "f"})));
    assert!(on_json::<GraphSearch>(
        json!({"name": "n", "nodeKind": "k"})
    ));
    assert!(on_typed(GraphSearch {
        name: None,
        file: None,
        node_kind: Some("k".into())
    }));
    assert!(!on_typed(GraphSearch {
        name: None,
        file: None,
        node_kind: None
    }));
    let schema = GraphSearch::input_schema();
    assert_eq!(schema["anyOf"].as_array().unwrap().len(), 3, "{schema}");
}

#[derive(Debug, OpParams)]
struct EditCommon {
    #[op(required_unless = "dry_run")]
    owner_key_file: Option<PathBuf>,
    dry_run: bool,
}

#[derive(Debug, OpParams)]
struct AddDevice {
    #[op(required_unless = "common.dry_run")]
    confirm_fingerprint: Option<String>,
    #[op(flatten)]
    common: EditCommon,
}

fn add_device(confirm: Option<&str>, owner: Option<&str>, dry_run: bool) -> AddDevice {
    AddDevice {
        confirm_fingerprint: confirm.map(str::to_owned),
        common: EditCommon {
            owner_key_file: owner.map(PathBuf::from),
            dry_run,
        },
    }
}

#[test]
fn dotted_required_unless_across_flatten() {
    // CLI
    assert_eq!(exit_of::<AddDevice>(&["--dry-run"]), 0);
    assert_eq!(
        exit_of::<AddDevice>(&["--confirm-fingerprint", "f", "--owner-key-file", "k"]),
        0
    );
    assert_eq!(exit_of::<AddDevice>(&["--owner-key-file", "k"]), 2);
    assert_eq!(exit_of::<AddDevice>(&["--confirm-fingerprint", "f"]), 2);
    assert_eq!(exit_of::<AddDevice>(&[]), 2);
    // JSON
    assert!(on_json::<AddDevice>(json!({"dryRun": true})));
    assert!(on_json::<AddDevice>(
        json!({"confirmFingerprint": "f", "ownerKeyFile": "k"})
    ));
    assert!(!on_json::<AddDevice>(json!({"ownerKeyFile": "k"})));
    assert!(!on_json::<AddDevice>(json!({"dryRun": false})));
    assert_eq!(
        json_of::<AddDevice>(json!({"ownerKeyFile": "k"}))
            .unwrap_err()
            .to_string(),
        "confirmFingerprint is required unless dryRun is given"
    );
    // typed
    assert!(on_typed(add_device(None, None, true)));
    assert!(on_typed(add_device(Some("f"), Some("k"), false)));
    assert!(!on_typed(add_device(None, Some("k"), false)));
    assert!(!on_typed(add_device(Some("f"), None, false)));
    // Both relations survive absorb.
    let schema = AddDevice::input_schema();
    assert_eq!(schema["allOf"].as_array().unwrap().len(), 2, "{schema}");
    // `null` is absent in validate, so the dotted target admits it.
    assert!(jsonschema::is_valid(
        &schema,
        &json!({"confirmFingerprint": "f", "ownerKeyFile": "k", "dryRun": null})
    ));
}

#[derive(Debug, OpParams)]
struct Mode {
    #[op(clearable)]
    note: Patch<String>,
    flag: Option<bool>,
}

#[derive(Debug, OpParams)]
struct Forced {
    #[op(conflicts_with = ["mode.flag", "mode.note"])]
    force: Option<String>,
    #[op(flatten)]
    mode: Mode,
}

#[test]
fn dotted_conflicts_use_the_leafs_own_arg_ids() {
    // A tri-state and a Patch leaf: `--no-flag` and `--clear-note` count.
    assert_eq!(exit_of::<Forced>(&["--force", "x"]), 0);
    assert_eq!(exit_of::<Forced>(&["--flag", "--no-flag"]), 2);
    assert_eq!(exit_of::<Forced>(&["--force", "x", "--flag"]), 2);
    assert_eq!(exit_of::<Forced>(&["--force", "x", "--no-flag"]), 2);
    assert_eq!(exit_of::<Forced>(&["--force", "x", "--clear-note"]), 2);
    assert_eq!(exit_of::<Forced>(&["--force", "x", "--note", "n"]), 2);
    assert_eq!(exit_of::<Forced>(&["--no-flag", "--clear-note"]), 0);
    assert!(on_json::<Forced>(json!({"force": "x"})));
    assert!(on_json::<Forced>(json!({"force": "x", "flag": null})));
    assert!(!on_json::<Forced>(json!({"force": "x", "flag": false})));
    assert!(!on_json::<Forced>(json!({"force": "x", "note": null})));
    assert_eq!(
        json_of::<Forced>(json!({"force": "x", "flag": true}))
            .unwrap_err()
            .to_string(),
        "force conflicts with flag"
    );
}

#[derive(Debug, OpParams)]
struct ConflictsOnly {
    #[op(conflicts_with = "b")]
    a: Option<String>,
    b: Vec<String>,
}

#[test]
fn a_lone_conflicts_with_is_a_valid_schema() {
    let schema = ConflictsOnly::input_schema();
    assert!(jsonschema::meta::is_valid(&schema), "{schema}");
    assert!(schema["not"].is_object());
    assert!(!jsonschema::is_valid(
        &schema,
        &json!({"a": "x", "b": ["y"]})
    ));
    assert!(jsonschema::is_valid(&schema, &json!({"a": "x", "b": []})));
}

#[test]
fn every_shape_clap_group_ids_are_valid() {
    // clap panics on a bad group id: drive tri-state and Patch members.
    for args in [
        &["--clear-note"][..],
        &["--note", "n"],
        &["--flag"],
        &["--no-flag"],
        &["--labels", "k=v"],
        &[],
    ] {
        assert_eq!(exit_of::<EveryShape>(args), 0, "{args:?}");
    }
    for args in [
        &["--clear-note", "--flag"][..],
        &["--no-flag", "--labels", "k=v"],
        &["--flag", "--no-flag"],
    ] {
        assert_eq!(exit_of::<EveryShape>(args), 2, "{args:?}");
    }
}
