use clap::Command;
use op_registry::{base64, FieldKind, Input, Json, OpParams, ParseCx};
use serde_json::json;

#[derive(Debug, PartialEq, OpParams)]
struct Params {
    /// The request body.
    body: Option<Json>,
    file: Option<Input>,
    items: Vec<Json>,
}

fn parse(args: &[&str], cx: &mut ParseCx) -> anyhow::Result<Params> {
    let mut argv = vec!["t"];
    argv.extend_from_slice(args);
    let m = Params::add_args(Command::new("t"))
        .try_get_matches_from(argv)
        .unwrap();
    Params::from_matches(&m, cx)
}

fn code(err: &anyhow::Error) -> &'static str {
    coded_error::facts_anyhow(err).code
}

fn temp_file(name: &str, contents: &[u8]) -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("op-registry-inputs-{}-{name}", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn json_flag_inline_text() {
    let p = parse(&["--body", r#"{"a":1}"#], &mut ParseCx::default()).unwrap();
    assert_eq!(p.body, Some(Json(json!({"a": 1}))));
    let err = parse(&["--body", "{nope"], &mut ParseCx::default()).unwrap_err();
    assert_eq!(code(&err), "op_registry.usage.invalid");
}

#[test]
fn json_flag_at_path_reads_file() {
    let path = temp_file("body.json", br#"[1,2]"#);
    let arg = format!("@{}", path.display());
    let p = parse(&["--body", &arg], &mut ParseCx::default()).unwrap();
    assert_eq!(p.body, Some(Json(json!([1, 2]))));
    std::fs::remove_file(&path).unwrap();
    let err = parse(&["--body", &arg], &mut ParseCx::default()).unwrap_err();
    assert_eq!(code(&err), "op_registry.usage.invalid");
    assert!(err.to_string().contains("body") && err.to_string().contains("body.json"));
}

#[test]
fn json_flag_at_dash_reads_stdin() {
    let mut cx = ParseCx::with_stdin(&br#"{"x":true}"#[..]);
    let p = parse(&["--body", "@-"], &mut cx).unwrap();
    assert_eq!(p.body, Some(Json(json!({"x": true}))));
}

#[test]
fn input_dash_reads_stdin_bytes() {
    let mut cx = ParseCx::with_stdin(&b"\x00\x01raw"[..]);
    let p = parse(&["--file", "-"], &mut cx).unwrap();
    assert_eq!(p.file, Some(Input(b"\x00\x01raw".to_vec())));
    let err = parse(&["--file", "-"], &mut ParseCx::default()).unwrap_err();
    assert_eq!(code(&err), "op_registry.usage.invalid");
}

#[test]
fn input_path_reads_file() {
    let path = temp_file("in.bin", b"bytes");
    let p = parse(&["--file", path.to_str().unwrap()], &mut ParseCx::default()).unwrap();
    assert_eq!(p.file, Some(Input(b"bytes".to_vec())));
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn input_json_is_base64() {
    let p = Params::from_json(
        json!({"file": "aGk=", "body": {"k": [1]}}),
        &mut ParseCx::default(),
    )
    .unwrap();
    assert_eq!(p.file, Some(Input(b"hi".to_vec())));
    assert_eq!(p.body, Some(Json(json!({"k": [1]}))));
    let err = Params::from_json(json!({"file": "aGk"}), &mut ParseCx::default()).unwrap_err();
    assert_eq!(code(&err), "op_registry.usage.invalid");
}

#[test]
fn input_schema_has_content_encoding() {
    let schema = Params::input_schema();
    assert_eq!(
        schema["properties"]["file"],
        json!({"type": "string", "contentEncoding": "base64"})
    );
    assert_eq!(
        schema["properties"]["body"]["description"],
        "The request body."
    );
    assert!(schema["properties"]["body"].get("type").is_none());
    let kinds: Vec<_> = Params::fields()
        .iter()
        .map(|f| (f.rust_name, f.kind))
        .collect();
    assert!(kinds.contains(&("body", FieldKind::Json)));
    assert!(kinds.contains(&("file", FieldKind::Input)));
}

#[test]
fn two_stdin_readers_is_stdin_twice() {
    let mut cx = ParseCx::with_stdin(&b"{}"[..]);
    let err = parse(&["--body", "@-", "--file", "-"], &mut cx).unwrap_err();
    assert_eq!(code(&err), "op_registry.usage.stdin_twice");
    let msg = err.to_string();
    assert!(msg.contains("body") && msg.contains("file"), "{msg}");
}

#[test]
fn base64_roundtrip_and_bad_padding() {
    for s in [
        &b""[..],
        b"f",
        b"fo",
        b"foo",
        b"foob",
        &[0xff, 0xfe, 0x00, 0x3e, 0x3f],
    ] {
        assert_eq!(base64::decode(&base64::encode(s)).unwrap(), s);
    }
    assert_eq!(base64::encode(b"fo"), "Zm8=");
    assert_eq!(base64::decode("Zm8=").unwrap(), b"fo");
    for bad in ["Zm8", "Zm8==", "Z===", "=Zm8", "Zm=8", "Zm!8", "Zm8=Zm9v"] {
        assert!(base64::decode(bad).is_err(), "{bad}");
    }
}

#[test]
fn vec_json_is_array_of_any() {
    assert_eq!(
        Params::input_schema()["properties"]["items"],
        json!({"type": "array", "items": {}})
    );
    let p = parse(
        &["--items", "1", "--items", r#"{"a":null}"#],
        &mut ParseCx::default(),
    )
    .unwrap();
    assert_eq!(p.items, vec![Json(json!(1)), Json(json!({"a": null}))]);
    let p = Params::from_json(
        json!({"items": [1, "x", {"a": 2}]}),
        &mut ParseCx::default(),
    )
    .unwrap();
    assert_eq!(p.items.len(), 3);
}
