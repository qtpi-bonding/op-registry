//! Secrets: `Secret`, `SecretFile`, `#[op(secret)]`, redaction, `write_private`.
#![cfg(feature = "secrets")]

use clap::Command;
use futures_util::future::BoxFuture;
use op_registry::{
    CallOptions, Notice, NoticeSink, Notices, OpParams, OpSpec, ParseCx, Redactor, Registry,
    Secret, SecretFile,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Debug, OpParams)]
struct Keyed {
    /// The key file.
    key: SecretFile,
}

#[derive(Debug, OpParams)]
struct Inline {
    #[op(secret_inline)]
    key: SecretFile,
}

#[derive(Debug, OpParams)]
struct Token {
    /// An API token.
    #[op(secret)]
    #[op(default = "fallback")]
    token: String,
}

#[derive(Debug, OpParams)]
struct HiddenKey {
    #[op(secret)]
    key: Option<SecretFile>,
}

fn parse<P: OpParams>(args: &[&str], cx: &mut ParseCx) -> anyhow::Result<P> {
    let mut argv = vec!["t"];
    argv.extend_from_slice(args);
    let m = P::add_args(Command::new("t"))
        .try_get_matches_from(argv)
        .unwrap();
    P::from_matches(&m, cx)
}

fn temp_file(name: &str, contents: &[u8]) -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("op-registry-secrets-{}-{name}", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn secret_debug_prints_placeholder() {
    let s = Secret::new("hunter2".to_owned());
    assert_eq!(format!("{s:?}"), "<secret>");
    assert_eq!(format!("{s}"), "<secret>");
    assert_eq!(s.expose(), "hunter2");
}

#[test]
fn secret_file_reads_path_at_parse() {
    let path = temp_file("key.pem", b"line-one-key\nline-two-key");
    let p: Keyed = parse(&["--key", path.to_str().unwrap()], &mut ParseCx::default()).unwrap();
    assert_eq!(p.key.bytes(), b"line-one-key\nline-two-key");
    assert_eq!(p.key.path(), Some(path.as_path()));
    let shown = format!("{:?}", p.key);
    assert!(
        shown.contains("key.pem") && !shown.contains("line-one-key"),
        "{shown}"
    );
    std::fs::remove_file(&path).unwrap();
    let err =
        parse::<Keyed>(&["--key", path.to_str().unwrap()], &mut ParseCx::default()).unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "op_registry.usage.invalid"
    );
}

#[test]
fn secret_file_dash_reads_stdin() {
    let mut cx = ParseCx::with_stdin(&b"from-stdin"[..]);
    let p: Keyed = parse(&["--key", "-"], &mut cx).unwrap();
    assert_eq!(p.key.bytes(), b"from-stdin");
    assert_eq!(p.key.path(), None);
    let mut cx = ParseCx::with_stdin(&b"x"[..]);
    cx.read_stdin("other").unwrap();
    let err = parse::<Keyed>(&["--key", "-"], &mut cx).unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "op_registry.usage.stdin_twice"
    );
}

#[test]
fn secret_field_schema_write_only_no_default() {
    let schema = Token::input_schema();
    let prop = &schema["properties"]["token"];
    assert_eq!(prop["writeOnly"], json!(true), "{schema}");
    assert!(prop.get("default").is_none(), "{schema}");
    assert!(Token::fields()[0].secret);
    // the default still applies
    let p = Token::from_json(json!({}), &mut ParseCx::default()).unwrap();
    assert_eq!(p.token, "fallback");
    assert_eq!(p.secrets(), vec![b"fallback".to_vec()]);
    // a plain SecretFile is a path string
    assert_eq!(
        Keyed::input_schema()["properties"]["key"],
        json!({"type": "string", "description": "The key file."})
    );
    let hidden = HiddenKey::input_schema();
    assert_eq!(hidden["properties"]["key"]["writeOnly"], json!(true));
}

#[test]
fn secret_inline_accepts_base64_object() {
    let p = Inline::from_json(json!({"key": {"inline": "aGk="}}), &mut ParseCx::default()).unwrap();
    assert_eq!(p.key.bytes(), b"hi");
    assert_eq!(p.key.path(), None);
    let path = temp_file("inline.key", b"on-disk");
    let p = Inline::from_json(
        json!({"key": path.to_str().unwrap()}),
        &mut ParseCx::default(),
    )
    .unwrap();
    assert_eq!(p.key.bytes(), b"on-disk");
    std::fs::remove_file(&path).unwrap();
    // without secret_inline an object is refused
    let err =
        Keyed::from_json(json!({"key": {"inline": "aGk="}}), &mut ParseCx::default()).unwrap_err();
    assert_eq!(
        coded_error::facts_anyhow(&err).code,
        "op_registry.usage.invalid"
    );
    let schema = Inline::input_schema();
    assert!(schema["properties"]["key"]["oneOf"].is_array(), "{schema}");
}

#[test]
fn hidden_path_is_not_printed() {
    let path = temp_file("hidden.key", b"abcdefgh");
    let p: HiddenKey = parse(&["--key", path.to_str().unwrap()], &mut ParseCx::default()).unwrap();
    let shown = format!("{p:?}");
    assert!(
        shown.contains("<secret>") && !shown.contains("hidden.key"),
        "{shown}"
    );
    let secrets = p.secrets();
    assert!(secrets.contains(&b"abcdefgh".to_vec()));
    assert!(secrets.contains(&path.to_str().unwrap().as_bytes().to_vec()));
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn redactor_redacts_value_and_lines() {
    let r = Redactor::default();
    r.add(b"line-one-key\nline-two-key");
    assert_eq!(
        r.redact("failed with line-two-key here"),
        "failed with <secret> here"
    );
    assert_eq!(r.redact("all: line-one-key\nline-two-key"), "all: <secret>");
    // short values are skipped
    let short = Redactor::default();
    short.add(b"abc");
    assert_eq!(short.redact("abc"), "abc");
    assert!(!format!("{r:?}").contains("line-one"));
}

struct Collect(Mutex<Vec<Notice>>);
impl NoticeSink for Collect {
    fn send(&self, notice: Notice) {
        self.0.lock().unwrap().push(notice);
    }
}

struct Ctx;
impl op_registry::Resolve for Ctx {}

#[test]
fn notice_with_secret_is_redacted() {
    let mut registry: Registry<Ctx> = Registry::new();
    registry.push_op(
        OpSpec::<Keyed, serde_json::Value>::new("leak", "Leaks."),
        |_c: &Ctx, p: Keyed, o: CallOptions| -> BoxFuture<'_, anyhow::Result<serde_json::Value>> {
            Box::pin(async move {
                let text = String::from_utf8(p.key.bytes().to_vec()).unwrap();
                o.notices.notice(format!("using {text}"));
                o.notices.progress(1, None, &text);
                Err(anyhow::anyhow!("bad key {text}"))
            })
        },
    );
    let path = temp_file("notice.key", b"s3cret-value");
    let sink = Arc::new(Collect(Mutex::new(Vec::new())));
    let opts = CallOptions::default().with_notices(Notices::new(sink.clone()));
    let result = futures_executor::block_on(registry.call(
        "leak",
        json!({"key": path.to_str().unwrap()}),
        &Ctx,
        opts,
    ));
    std::fs::remove_file(&path).unwrap();
    assert!(result.is_err());
    let got = sink.0.lock().unwrap().clone();
    assert_eq!(
        got,
        vec![
            Notice::Text("using <secret>".to_owned()),
            Notice::Progress {
                done: 1,
                total: None,
                label: "<secret>".to_owned()
            },
        ]
    );
}

#[test]
fn write_private_mode_0600_and_no_overwrite() {
    let path = std::env::temp_dir().join(format!("op-registry-wp-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    op_registry::write_private(&path, b"one", false).unwrap();
    let err = op_registry::write_private(&path, b"two", false).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&path).unwrap(), b"one");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    op_registry::write_private(&path, b"three", true).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"three");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_file(&path).unwrap();
}
