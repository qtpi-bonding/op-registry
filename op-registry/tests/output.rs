//! The exit table, JSON error and notice lines, and `Style`.

use anyhow::Context;
use op_registry::exit::{self, exit_code};
use op_registry::{
    error_value, notice_line, print_json, render, write_error, Exit, Notice, Redactor, Style,
};
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.out")]
enum E {
    #[error("bad request")]
    #[coded(code = "bad", class = Logical)]
    Bad,
    #[error("disk gone")]
    #[coded(code = "disk", class = Infrastructure)]
    Disk,
    #[error("try later")]
    #[coded(code = "later", class = Transient)]
    Later,
    #[error("who knows")]
    #[coded(code = "odd", class = Unknown)]
    Odd,
}

fn redactor(secret: &str) -> Redactor {
    let mut r = Redactor::default();
    r.add(secret.as_bytes());
    r
}

#[test]
fn exit_table_values() {
    assert_eq!(
        [
            exit::OK,
            exit::USAGE,
            exit::LOGICAL,
            exit::INFRASTRUCTURE,
            exit::UNKNOWN,
            exit::TRANSIENT
        ],
        [0, 2, 65, 69, 70, 75]
    );
}

#[test]
fn usage_codes_exit_2() {
    let missing =
        coded_error::into_anyhow(op_registry::UsageError::MissingField { name: "x".into() });
    assert_eq!(
        coded_error::facts_anyhow(&missing).code,
        "op_registry.usage.missing"
    );
    assert_eq!(exit_code(&missing), 2);
    let unknown =
        coded_error::into_anyhow(op_registry::cli::CliError::UnknownCommand { name: "x".into() });
    assert_eq!(
        coded_error::facts_anyhow(&unknown).code,
        "op_registry.cli.unknown_command"
    );
    assert_eq!(exit_code(&unknown), 2);
}

#[test]
fn classes_map_to_sysexits() {
    assert_eq!(exit_code(&coded_error::into_anyhow(E::Bad)), 65);
    assert_eq!(exit_code(&coded_error::into_anyhow(E::Disk)), 69);
    assert_eq!(exit_code(&coded_error::into_anyhow(E::Later)), 75);
    assert_eq!(exit_code(&coded_error::into_anyhow(E::Odd)), 70);
}

#[test]
fn uncoded_exits_70() {
    assert_eq!(exit_code(&anyhow::anyhow!("plain")), 70);
}

fn declared() -> Vec<Exit> {
    serde_json::from_value(json!([{"code": 4, "meaning": "four"}])).unwrap()
}

#[test]
fn ok_exit_rules() {
    let d = declared();
    assert_eq!(exit::ok_exit(&d, 0), 0);
    assert_eq!(exit::ok_exit(&d, 4), 4);
    assert_eq!(exit::ok_exit(&d, 9), 70);
    assert_eq!(exit::ok_exit(&d, 64), 70);
    assert_eq!(exit::ok_exit(&[], 4), 70);
}

#[test]
fn style_pretty_if_either_tty_compact_if_both_piped_force_wins() {
    assert_eq!(Style::detect(true, false, None), Style::Pretty);
    assert_eq!(Style::detect(false, true, None), Style::Pretty);
    assert_eq!(Style::detect(false, false, None), Style::Compact);
    assert_eq!(
        Style::detect(true, true, Some(Style::Compact)),
        Style::Compact
    );
    assert_eq!(
        Style::detect(false, false, Some(Style::Pretty)),
        Style::Pretty
    );
}

#[test]
fn render_and_print_json() {
    let v = json!({"a": 1});
    assert_eq!(render(&v, Style::Compact), r#"{"a":1}"#);
    assert_eq!(render(&v, Style::Pretty), "{\n  \"a\": 1\n}");
    let mut out = Vec::new();
    print_json(&mut out, &v, Style::Compact).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "{\"a\":1}\n");
}

#[test]
fn error_line_compact_shape() {
    let err = coded_error::into_anyhow(E::Bad);
    let mut out = Vec::new();
    write_error(&mut out, &err, &Redactor::default(), false).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.lines().count(), 1);
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["error"]["code"], "test.out.bad");
    assert_eq!(v["error"]["class"], "logical");
    assert_eq!(v["error"]["message"], "bad request");
}

#[test]
fn error_line_tty_is_indented() {
    let err = coded_error::into_anyhow(E::Bad);
    let mut out = Vec::new();
    write_error(&mut out, &err, &Redactor::default(), true).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.lines().count() > 1);
    assert!(text.contains("\n  \"error\""));
    assert!(text.ends_with("}\n"));
}

#[test]
fn error_message_has_context_chain_and_is_redacted() {
    let err = coded_error::into_anyhow(E::Bad);
    let err = Err::<(), _>(err)
        .context("reading key hunter2-secret")
        .unwrap_err();
    let v = error_value(&err, &redactor("hunter2-secret"));
    let message = v["error"]["message"].as_str().unwrap();
    assert_eq!(message, "reading key <secret>: bad request");
    assert_eq!(v["error"]["code"], "test.out.bad");
}

#[test]
fn notice_and_progress_lines_have_distinct_keys() {
    let n = notice_line(&Notice::Text("hello".into()));
    assert_eq!(n, json!({"notice": "hello"}));
    let p = notice_line(&Notice::Progress {
        done: 2,
        total: Some(5),
        label: "files".into(),
    });
    assert_eq!(
        p,
        json!({"progress": {"done": 2, "total": 5, "label": "files"}})
    );
    let p = notice_line(&Notice::Progress {
        done: 2,
        total: None,
        label: "files".into(),
    });
    assert_eq!(p["progress"]["total"], Value::Null);
}
