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

// ---- SEC-4 / SEC-5: asking on the terminal ----

mod prompting {
    use super::*;
    use op_registry::cli::{self, Io, NoHooks, Prompter};
    use op_registry::{Audience, CliContext, Op, OpGlobals};
    use serde_json::Value;
    use std::collections::VecDeque;
    use std::io;
    use zeroize::Zeroizing;

    #[derive(Debug, OpGlobals)]
    struct NoGlobals {}

    struct PCtx;
    impl op_registry::Resolve for PCtx {}
    impl CliContext for PCtx {
        type Globals = NoGlobals;
        fn from_globals(_g: &NoGlobals) -> anyhow::Result<Self> {
            Ok(PCtx)
        }
    }

    /// A prompter that answers from a script and records its labels.
    struct Fake {
        answers: Mutex<VecDeque<io::Result<String>>>,
        labels: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(answers: Vec<io::Result<String>>) -> Arc<Fake> {
            Arc::new(Fake {
                answers: Mutex::new(answers.into()),
                labels: Mutex::new(Vec::new()),
            })
        }
        fn says(line: &str) -> Arc<Fake> {
            Fake::new(vec![Ok(line.to_owned())])
        }
        fn labels(&self) -> Vec<String> {
            self.labels.lock().unwrap().clone()
        }
    }

    impl Prompter for Fake {
        fn read_secret(&self, label: &str) -> io::Result<Zeroizing<String>> {
            self.labels.lock().unwrap().push(label.to_owned());
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .expect("the prompter was asked more often than the test scripted")
                .map(Zeroizing::new)
        }
    }

    #[derive(Debug, OpParams)]
    struct Asked {
        /// The API key.
        #[op(prompt = "API key")]
        key: SecretFile,
    }

    #[derive(Debug, OpParams)]
    struct AskedDefault {
        #[op(prompt)]
        api_key: SecretFile,
    }

    #[derive(Debug, OpParams)]
    struct Nothing {
        /// Makes the op fail with the prompted value in the message.
        fail: bool,
    }

    fn key_text(p: &Asked) -> String {
        String::from_utf8(p.key.bytes().to_vec()).unwrap()
    }

    fn registry() -> Registry<PCtx> {
        let mut r: Registry<PCtx> = Registry::new();
        r.set_pillar("fix");
        r.push_op(
            OpSpec::<Asked, Value>::new("show", "Shows the key."),
            |_c: &PCtx, p: Asked, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
                Box::pin(async move { Ok(json!({"got": key_text(&p)})) })
            },
        );
        r.push_op(
            OpSpec::<Asked, Value>::new("leak", "Fails with the key."),
            |_c: &PCtx, p: Asked, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
                Box::pin(async move { Err(anyhow::anyhow!("bad key {}", key_text(&p))) })
            },
        );
        r.push_op(
            OpSpec::<AskedDefault, Value>::new("labelled", "Default label."),
            |_c: &PCtx, p: AskedDefault, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
                Box::pin(async move { Ok(json!({"len": p.api_key.bytes().len()})) })
            },
        );
        r.push_op(
            OpSpec::<Nothing, Value>::new("ask", "Asks inside the op."),
            |_c: &PCtx, p: Nothing, o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
                Box::pin(async move {
                    let got = o.prompt_secret("Paste the code");
                    let text = got.as_ref().map(|s| s.expose().to_string());
                    if p.fail {
                        return Err(anyhow::anyhow!("rejected {}", text.unwrap_or_default()));
                    }
                    Ok(json!({"got": text}))
                })
            },
        );
        r
    }

    fn run(args: &[&str], io: Io) -> u8 {
        let argv = std::iter::once("app")
            .chain(args.iter().copied())
            .map(std::ffi::OsString::from)
            .collect();
        futures_executor::block_on(cli::run_with_io(&registry(), argv, &NoHooks, io))
    }

    fn with_prompter(stdin: &[u8], p: &Arc<Fake>) -> (Io, op_registry::cli::Captured) {
        let (io, cap) = Io::memory(stdin, false, false);
        (io.with_prompter(p.clone()), cap)
    }

    #[test]
    fn missing_prompted_flag_asks_with_label() {
        let p = Fake::says("hunter2-secret");
        let (io, cap) = with_prompter(b"", &p);
        assert_eq!(run(&["show"], io), 0, "{}", cap.stderr());
        assert_eq!(p.labels(), vec!["API key"]);
        // the default label is the flag name
        let p = Fake::says("abcdefgh");
        let (io, cap) = with_prompter(b"", &p);
        assert_eq!(run(&["labelled"], io), 0, "{}", cap.stderr());
        assert_eq!(p.labels(), vec!["api-key"]);
        assert!(cap.stdout().contains("\"len\":8"), "{}", cap.stdout());
    }

    #[test]
    fn prompted_bytes_reach_op_and_are_redacted() {
        let p = Fake::says("hunter2-secret");
        let (io, cap) = with_prompter(b"", &p);
        assert_eq!(run(&["show"], io), 0, "{}", cap.stderr());
        assert!(cap.stdout().contains("hunter2-secret"), "{}", cap.stdout());
        let p = Fake::says("hunter2-secret");
        let (io, cap) = with_prompter(b"", &p);
        assert_ne!(run(&["leak"], io), 0);
        assert!(
            cap.stderr().contains("bad key <secret>"),
            "{}",
            cap.stderr()
        );
        assert!(!cap.stderr().contains("hunter2"), "{}", cap.stderr());
    }

    #[test]
    fn no_prompter_is_missing_exit_2() {
        let (io, cap) = Io::memory(b"", false, false);
        assert_eq!(run(&["show"], io), 2, "{}", cap.stderr());
        let err = cap.stderr();
        assert!(err.contains("op_registry.usage.missing"), "{err}");
        assert!(err.contains("(no terminal to ask on)"), "{err}");
    }

    #[test]
    fn key_file_dash_reads_stdin_without_asking() {
        let p = Fake::new(vec![]);
        let (io, cap) = with_prompter(b"from-stdin-key", &p);
        assert_eq!(run(&["show", "--key", "-"], io), 0, "{}", cap.stderr());
        assert!(cap.stdout().contains("from-stdin-key"), "{}", cap.stdout());
        assert!(p.labels().is_empty());
    }

    #[test]
    fn given_path_does_not_ask() {
        let path = temp_file("prompt-given.key", b"on-disk-key");
        let p = Fake::new(vec![]);
        let (io, cap) = with_prompter(b"", &p);
        assert_eq!(
            run(&["show", "--key", path.to_str().unwrap()], io),
            0,
            "{}",
            cap.stderr()
        );
        std::fs::remove_file(&path).unwrap();
        assert!(cap.stdout().contains("on-disk-key"), "{}", cap.stdout());
        assert!(p.labels().is_empty());
    }

    #[test]
    fn json_without_key_is_missing_and_prompter_unused() {
        // call, call_tool and call_typed carry no prompter at all, and the
        // field stays required in the schema.
        let r = registry();
        let err =
            futures_executor::block_on(r.call("show", json!({}), &PCtx, CallOptions::default()))
                .unwrap_err();
        assert_eq!(
            coded_error::facts_anyhow(err.error()).code,
            "op_registry.usage.missing"
        );
        let err = futures_executor::block_on(op_registry::mcp::call_tool(
            &r,
            &PCtx,
            Audience::Agent,
            "show",
            json!({}),
            CallOptions::default(),
        ))
        .unwrap_err();
        assert_eq!(
            coded_error::facts_anyhow(err.error()).code,
            "op_registry.usage.missing"
        );
        assert_eq!(Asked::input_schema()["required"], json!(["key"]));
        assert!(Asked::fields()[0].required);
        assert_eq!(Asked::fields()[0].prompt.as_deref(), Some("API key"));
        assert_eq!(AskedDefault::fields()[0].prompt.as_deref(), Some("api-key"));
    }

    struct Show;
    impl Op for Show {
        type Ctx = PCtx;
        type Params = Asked;
        type Output = Value;
        const NAME: &'static str = "show";
    }

    #[test]
    fn typed_params_are_always_present() {
        // A typed caller builds the params itself: nothing to ask.
        let r = registry();
        let key = SecretFile::new(None, b"typed-key-value".to_vec());
        let got = futures_executor::block_on(r.call_typed::<Show>(
            &PCtx,
            Asked { key },
            CallOptions::default(),
        ))
        .unwrap();
        assert_eq!(got, json!({"got": "typed-key-value"}));
    }

    struct Ask;
    impl Op for Ask {
        type Ctx = PCtx;
        type Params = Nothing;
        type Output = Value;
        const NAME: &'static str = "ask";
    }

    #[test]
    fn prompt_empty_answer_is_missing() {
        let p = Fake::says("");
        let (io, cap) = with_prompter(b"", &p);
        assert_eq!(run(&["show"], io), 2, "{}", cap.stderr());
        let err = cap.stderr();
        assert!(err.contains("op_registry.usage.missing"), "{err}");
        assert!(err.contains("(empty answer)"), "{err}");
        // the op never ran
        assert_eq!(cap.stdout(), "");
        assert_eq!(p.labels(), vec!["API key"]);
    }

    #[test]
    fn prompter_read_error_is_invalid() {
        let p = Fake::new(vec![Err(io::Error::other("tty gone"))]);
        let (io, cap) = with_prompter(b"", &p);
        assert_eq!(run(&["show"], io), 2, "{}", cap.stderr());
        let err = cap.stderr();
        assert!(err.contains("op_registry.usage.invalid"), "{err}");
        assert!(
            err.contains("cannot read from the terminal: tty gone"),
            "{err}"
        );
    }

    #[test]
    fn prompt_secret_on_cli_returns_value_and_redacts() {
        let p = Fake::says("pasted-code-123");
        let (io, cap) = with_prompter(b"", &p);
        assert_eq!(run(&["ask"], io), 0, "{}", cap.stderr());
        assert!(cap.stdout().contains("pasted-code-123"), "{}", cap.stdout());
        assert_eq!(p.labels(), vec!["Paste the code"]);
        let p = Fake::says("pasted-code-123");
        let (io, cap) = with_prompter(b"", &p);
        assert_ne!(run(&["ask", "--fail"], io), 0);
        assert!(
            cap.stderr().contains("rejected <secret>"),
            "{}",
            cap.stderr()
        );
        assert!(!cap.stderr().contains("pasted-code"), "{}", cap.stderr());
    }

    #[test]
    fn prompt_secret_off_cli_is_none() {
        let r = registry();
        let got = futures_executor::block_on(r.call(
            "ask",
            json!({"fail": false}),
            &PCtx,
            CallOptions::default(),
        ))
        .unwrap();
        assert_eq!(got, json!({"got": null}));
        let got = futures_executor::block_on(op_registry::mcp::call_tool(
            &r,
            &PCtx,
            Audience::Agent,
            "ask",
            json!({"fail": false}),
            CallOptions::default(),
        ))
        .unwrap();
        assert_eq!(got, json!({"got": null}));
        let got = futures_executor::block_on(r.call_typed::<Ask>(
            &PCtx,
            Nothing { fail: false },
            CallOptions::default(),
        ))
        .unwrap();
        assert_eq!(got, json!({"got": null}));
    }

    #[test]
    fn prompt_secret_no_prompter_is_none() {
        let (io, cap) = Io::memory(b"", false, false);
        assert_eq!(run(&["ask"], io), 0, "{}", cap.stderr());
        assert!(cap.stdout().contains("\"got\":null"), "{}", cap.stdout());
    }

    #[test]
    fn prompt_secret_empty_answer_is_none() {
        let p = Fake::says("");
        let (io, cap) = with_prompter(b"", &p);
        assert_eq!(run(&["ask"], io), 0, "{}", cap.stderr());
        assert!(cap.stdout().contains("\"got\":null"), "{}", cap.stdout());
        assert_eq!(cap.stderr(), "", "an empty answer is not an error");
    }

    #[test]
    fn prompt_secret_read_error_sends_notice() {
        let p = Fake::new(vec![Err(io::Error::other("tty gone"))]);
        let (io, cap) = with_prompter(b"", &p);
        assert_eq!(run(&["ask"], io), 0, "{}", cap.stderr());
        assert!(cap.stdout().contains("\"got\":null"), "{}", cap.stdout());
        let err = cap.stderr();
        assert_eq!(
            err.matches("cannot read from the terminal: tty gone")
                .count(),
            1,
            "{err}"
        );
    }

    #[test]
    fn prompted_help_says_it_is_asked_for() {
        let (io, cap) = Io::memory(b"", false, false);
        let _ = run(&["show", "--help"], io);
        assert!(
            cap.stdout()
                .contains("Asked for on a terminal when left out."),
            "{}",
            cap.stdout()
        );
    }
}

// ---- KIT-5 / SEC-2: the kit's secret sentinel ----

#[cfg(feature = "testing")]
mod kit {
    use super::*;
    use op_registry::testing::{assert_secrets_not_echoed, SecretSentinel};
    use op_registry::{CliContext, OpGlobals};
    use serde_json::Value;
    use std::path::PathBuf;

    const SENTINEL: &str = "SENTINEL-token-4471";
    const KEY_LINES: &str = "KEYLINE-alpha-9921\nKEYLINE-beta-9922\n";

    /// A context that needs `--hub`, like a binary whose ops fail without
    /// their address.
    #[derive(Debug, OpGlobals)]
    struct Needs {
        /// The hub address.
        hub: String,
    }

    struct Ctx;
    impl op_registry::Resolve for Ctx {}
    impl CliContext for Ctx {
        type Globals = Needs;
        fn from_globals(g: &Needs) -> anyhow::Result<Self> {
            anyhow::ensure!(!g.hub.is_empty(), "no hub");
            Ok(Ctx)
        }
    }

    #[derive(Debug, OpParams)]
    struct Tokened {
        /// A token.
        #[op(secret)]
        token: Option<String>,
    }

    #[derive(Debug, OpParams)]
    struct KeyPath {
        /// A key file the op reads itself.
        #[op(secret)]
        key_file: PathBuf,
    }

    #[derive(Debug, OpParams)]
    struct Prompted {
        /// The key.
        #[op(prompt)]
        key: SecretFile,
    }

    fn key_file(name: &str) -> PathBuf {
        temp_file(name, KEY_LINES.as_bytes())
    }

    fn registry_with(f: impl FnOnce(&mut Registry<Ctx>)) -> Registry<Ctx> {
        let mut r = Registry::<Ctx>::new();
        r.set_pillar("fix");
        r.set_bin("fixd", "1.0.0", "A fixture.");
        f(&mut r);
        r
    }

    fn panic_text(f: impl FnOnce()) -> String {
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
            .expect_err("expected a panic");
        err.downcast_ref::<String>().cloned().unwrap_or_default()
    }

    fn check(r: &Registry<Ctx>, sentinel: &SecretSentinel<'_>) {
        futures_executor::block_on(assert_secrets_not_echoed(r, None, sentinel));
    }

    #[test]
    fn sentinel_globals_reach_argv() {
        let file = key_file("kit-globals.pem");
        // Echoes the token it was given; without `--hub` it never runs.
        let r = registry_with(|r| {
            r.push_op(
                OpSpec::<Tokened, Value>::new("leak", "Echoes the token."),
                |_c: &Ctx, p: Tokened, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
                    Box::pin(async move { Ok(json!({"said": p.token})) })
                },
            );
        });
        check(&r, &SecretSentinel::new(SENTINEL, &file));
        let sentinel = SecretSentinel::new(SENTINEL, &file).with_globals(&["--hub", "h"]);
        let text = panic_text(|| check(&r, &sentinel));
        assert!(text.contains("stdout of the CLI run of leak"), "{text}");
        assert!(text.contains("sentinel value"), "{text}");
        std::fs::remove_file(&file).unwrap();
    }

    fn key_path_registry(register: bool, echo: bool) -> Registry<Ctx> {
        registry_with(|r| {
            r.push_op(
                OpSpec::<KeyPath, Value>::new("sign", "Signs with the key file."),
                move |_c: &Ctx,
                      p: KeyPath,
                      o: CallOptions|
                      -> BoxFuture<'_, anyhow::Result<Value>> {
                    Box::pin(async move {
                        let contents = std::fs::read(&p.key_file)?;
                        if register {
                            o.redact(&contents);
                        }
                        let text = String::from_utf8_lossy(&contents).into_owned();
                        if echo {
                            return Ok(json!({"key": text}));
                        }
                        anyhow::bail!("bad key {text}")
                    })
                },
            );
        })
    }

    #[test]
    fn secret_path_op_reading_and_registering_passes() {
        let file = key_file("kit-path-ok.pem");
        let sentinel = SecretSentinel::new(SENTINEL, &file).with_globals(&["--hub", "h"]);
        check(&key_path_registry(true, false), &sentinel);
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn secret_path_op_echoing_contents_fails() {
        let file = key_file("kit-path-leak.pem");
        let sentinel = SecretSentinel::new(SENTINEL, &file).with_globals(&["--hub", "h"]);
        let text = panic_text(|| check(&key_path_registry(false, true), &sentinel));
        assert!(text.contains("stdout of the CLI run of sign"), "{text}");
        assert!(text.contains("KEYLINE-beta-9922"), "{text}");
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn prompted_field_exercised_with_fake_prompter() {
        let file = key_file("kit-prompt.pem");
        // Leaks only what the prompter answered, never the key file.
        let r = registry_with(|r| {
            r.push_op(
                OpSpec::<Prompted, Value>::new("ask", "Echoes a prompted key."),
                |_c: &Ctx, p: Prompted, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Value>> {
                    Box::pin(async move {
                        let got = String::from_utf8_lossy(p.key.bytes()).into_owned();
                        Ok(json!({"got": if got == SENTINEL { got } else { "other".into() }}))
                    })
                },
            );
        });
        let sentinel = SecretSentinel::new(SENTINEL, &file).with_globals(&["--hub", "h"]);
        let text = panic_text(|| check(&r, &sentinel));
        assert!(text.contains("with `--key` left out"), "{text}");
        assert!(text.contains("sentinel value"), "{text}");
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn outcome_message_with_sentinel_fails() {
        let file = key_file("kit-outcome.pem");
        // The error line carries the unregistered contents, so the message
        // the `post_run` hook sees does too.
        let sentinel = SecretSentinel::new(SENTINEL, &file).with_globals(&["--hub", "h"]);
        let text = panic_text(|| check(&key_path_registry(false, false), &sentinel));
        assert!(text.contains("the `post_run` outcome message"), "{text}");
        assert!(text.contains("KEYLINE-alpha-9921"), "{text}");
        std::fs::remove_file(&file).unwrap();
    }
}
