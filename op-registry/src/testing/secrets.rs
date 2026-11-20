//! `assert_secrets_not_echoed`: a sentinel secret never shows anywhere the
//! app prints.

use super::fail_on;
use super::sample_params;
use crate::cli::{run_with_io, Io, NoHooks};
use crate::manual::{render, Manual};
use crate::{CliContext, FieldKind, FieldMeta, ParseCx, Registry, Resolve};
use clap::Command;
use serde_json::{Map, Value};
use std::ffi::OsString;
use std::path::Path;

/// A made-up secret and a file holding made-up key material. Each non-empty
/// line of the file is searched for on its own, so the lines must be
/// distinctive (a short or common line would match ordinary text).
pub struct SecretSentinel<'a> {
    /// The sentinel secret value, given to every secret flag.
    pub value: &'a str,
    /// A file whose contents are the sentinel, given to every `SecretFile`.
    pub key_file: &'a Path,
}

/// What must never appear, each with its name for the report.
fn needles(sentinel: &SecretSentinel<'_>) -> Vec<(String, String)> {
    assert!(
        !sentinel.value.is_empty(),
        "assert_secrets_not_echoed: the sentinel value is empty"
    );
    let contents = std::fs::read(sentinel.key_file).unwrap_or_else(|err| {
        panic!(
            "assert_secrets_not_echoed: cannot read the key file {}: {err}",
            sentinel.key_file.display()
        )
    });
    let contents = String::from_utf8_lossy(&contents).into_owned();
    let mut found = vec![("sentinel value".to_owned(), sentinel.value.to_owned())];
    if !contents.trim().is_empty() {
        found.push(("key file contents".to_owned(), contents.clone()));
    }
    for line in contents.lines().map(str::trim).filter(|l| !l.is_empty()) {
        found.push(("key file line".to_owned(), line.to_owned()));
    }
    found
}

/// The long help of every command in the tree, with the command's words.
fn helps(cmd: &mut Command, path: &mut Vec<String>, out: &mut Vec<(String, String)>) {
    let name = if path.is_empty() {
        "the root command".to_owned()
    } else {
        format!("`{}`", path.join(" "))
    };
    out.push((
        format!("--help of {name}"),
        cmd.render_long_help().to_string(),
    ));
    for sub in cmd.get_subcommands_mut() {
        path.push(sub.get_name().to_owned());
        helps(sub, path, out);
        path.pop();
    }
}

fn is_secret(field: &FieldMeta) -> bool {
    field.secret || field.kind == FieldKind::SecretFile
}

/// Sample arguments for an op with every secret field set: `SecretFile`
/// fields to the key file, other secret fields to the sentinel.
fn secret_args(schema: &Value, fields: &[FieldMeta], sentinel: &SecretSentinel<'_>) -> Value {
    let mut args = match sample_params(schema) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    for field in fields.iter().filter(|f| is_secret(f)) {
        let text = Value::String(if field.kind == FieldKind::SecretFile {
            sentinel.key_file.to_string_lossy().into_owned()
        } else {
            sentinel.value.to_owned()
        });
        let value = match field.kind {
            FieldKind::Repeated => Value::Array(vec![text]),
            FieldKind::Map => Value::Object(Map::from_iter([("k".to_owned(), text)])),
            FieldKind::Switch | FieldKind::TriState => continue,
            _ => text,
        };
        args.insert(field.key.clone(), value);
    }
    Value::Object(args)
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The command line that carries `args`: the op's words, then one flag per
/// key.
fn argv_of(bin: &str, path: &[String], fields: &[FieldMeta], args: &Value) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![bin.into()];
    argv.extend(path.iter().map(OsString::from));
    for (key, value) in args.as_object().into_iter().flatten() {
        let Some(field) = fields.iter().find(|f| &f.key == key) else {
            continue;
        };
        let flag = format!("--{}", field.flag);
        match (field.kind, value) {
            (_, Value::Null) => {}
            (FieldKind::Switch, Value::Bool(on)) => {
                if *on {
                    argv.push(flag.into());
                }
            }
            (FieldKind::TriState, Value::Bool(on)) => {
                argv.push(
                    if *on {
                        flag
                    } else {
                        format!("--no-{}", field.flag)
                    }
                    .into(),
                );
            }
            (FieldKind::Repeated, Value::Array(items)) => {
                for item in items {
                    argv.extend([flag.clone().into(), text_of(item).into()]);
                }
            }
            (FieldKind::Map, Value::Object(map)) => {
                for (k, v) in map {
                    argv.extend([flag.clone().into(), format!("{k}={}", text_of(v)).into()]);
                }
            }
            (_, value) => argv.extend([flag.into(), text_of(value).into()]),
        }
    }
    argv
}

/// Runs `argv` on in-memory streams: stderr and stdout.
async fn run<C: CliContext + Resolve>(
    registry: &Registry<C>,
    argv: Vec<OsString>,
) -> (String, String) {
    let (io, captured) = Io::memory(b"", false, false);
    run_with_io(registry, argv, &NoHooks, io).await;
    (captured.stderr(), captured.stdout())
}

/// Checks that the sentinel value, the key file's contents and each of its
/// lines are absent from:
///
/// - every `--help`, the manual (`manual`, or an empty one), and every input
///   and output schema;
/// - the clap error of a bad use of each secret flag;
/// - the error line, the notices and stdout of a CLI run of each op with
///   secret fields, given `sample_params` with those fields set;
/// - the JSON decode error, if any, of those same arguments.
///
/// # Panics
///
/// Panics with every place a needle was found, or when the key file cannot
/// be read.
pub async fn assert_secrets_not_echoed<C: CliContext + Resolve>(
    registry: &Registry<C>,
    manual: Option<&Manual>,
    sentinel: &SecretSentinel<'_>,
) {
    let needles = needles(sentinel);
    let mut places: Vec<(String, String)> = Vec::new();
    let mut problems = Vec::new();

    let mut root = crate::cli::command(registry);
    root.build();
    helps(&mut root, &mut Vec::new(), &mut places);
    match render(registry, &manual.cloned().unwrap_or_default()) {
        Ok(text) => places.push(("the manual".to_owned(), text)),
        Err(err) => problems.push(format!("the manual does not render: {err:#}")),
    }
    for d in registry.descriptors() {
        places.push((
            format!("the input schema of {}", d.name),
            d.input_schema.to_string(),
        ));
        if let Some(schema) = &d.output_schema {
            places.push((
                format!("the output schema of {}", d.name),
                schema.to_string(),
            ));
        }
    }

    for index in 0..registry.entries.len() {
        let entry = &registry.entries[index];
        let d = &entry.descriptor;
        if !crate::cli::on_cli(registry, index) {
            continue;
        }
        let fields = &entry.param_fields;
        let bin = registry.bin();
        for field in fields.iter().filter(|f| f.secret) {
            let flag = format!("--{}", field.flag);
            let mut bad_uses: Vec<Vec<OsString>> = Vec::new();
            let mut stray = argv_of(bin, &d.path, &[], &Value::Null);
            stray.extend([
                flag.clone().into(),
                sentinel.value.into(),
                "--no-such-flag".into(),
            ]);
            bad_uses.push(stray);
            if !matches!(field.kind, FieldKind::Repeated | FieldKind::Map) {
                let mut twice = argv_of(bin, &d.path, &[], &Value::Null);
                twice.extend(
                    [&flag, sentinel.value, &flag, sentinel.value]
                        .into_iter()
                        .map(OsString::from),
                );
                bad_uses.push(twice);
            }
            for argv in bad_uses {
                let (stderr, stdout) = run(registry, argv).await;
                let what = format!("the clap error of a bad `{flag}` on `{}`", d.path.join(" "));
                places.push((what.clone(), stderr));
                places.push((what, stdout));
            }
        }
        if fields.iter().any(is_secret) {
            let args = secret_args(&d.input_schema, fields, sentinel);
            let (stderr, stdout) = run(registry, argv_of(bin, &d.path, fields, &args)).await;
            places.push((
                format!("the error line or notices of the CLI run of {}", d.name),
                stderr,
            ));
            places.push((format!("stdout of the CLI run of {}", d.name), stdout));
            if let Err(err) = (entry.decode_json)(args, &mut ParseCx::default()) {
                places.push((
                    format!("the JSON decode error of {}", d.name),
                    format!("{err:#}"),
                ));
            }
        }
    }

    for (place, text) in &places {
        for (label, needle) in &needles {
            if text.contains(needle.as_str()) {
                problems.push(format!(
                    "{place} contains the {label} `{}`",
                    needle.trim_end()
                ));
            }
        }
    }
    fail_on("assert_secrets_not_echoed", &problems);
}
