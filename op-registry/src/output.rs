//! Output: schema discovery, text rendering, and the JSON, error and notice
//! lines the command-line adapter prints.

use crate::{Notice, Redactor};
use serde_json::{json, Value};
use std::io::{self, Write};

/// An output type's text rendering (`text: true` in `ops!`).
pub trait RenderText {
    fn render_text(&self) -> String;
}

impl<T: RenderText> RenderText for Vec<T> {
    /// One item per line.
    fn render_text(&self) -> String {
        self.iter()
            .map(RenderText::render_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The text of a list page: its items, then `next cursor: <c>` when there is one.
#[doc(hidden)]
pub fn render_page<T: RenderText>(page: &crate::Page<T>) -> String {
    let mut lines: Vec<String> = page.items.iter().map(RenderText::render_text).collect();
    lines.extend(
        page.next_cursor
            .as_ref()
            .map(|c| format!("next cursor: {c}")),
    );
    lines.join("\n")
}

/// An output type's exit code on success (`exits:` in `ops!`). The code must
/// be declared; see [`crate::exit::ok_exit`].
pub trait OkExit {
    fn exit_code(&self) -> u8 {
        0
    }
}

/// How JSON is laid out on a terminal or a pipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Indented, for people.
    Pretty,
    /// One line, for programs.
    Compact,
}

impl Style {
    /// Pretty if either stream is a terminal, else compact; `force` wins.
    pub fn detect(stdout_tty: bool, stderr_tty: bool, force: Option<Style>) -> Style {
        match force {
            Some(style) => style,
            None if stdout_tty || stderr_tty => Style::Pretty,
            None => Style::Compact,
        }
    }
}

/// Renders a JSON value without a trailing newline.
pub fn render(value: &Value, style: Style) -> String {
    match style {
        Style::Pretty => serde_json::to_string_pretty(value),
        Style::Compact => serde_json::to_string(value),
    }
    .expect("JSON values are serializable")
}

/// Writes a JSON value followed by a newline, in one write so a writer
/// shared under a lock never splits the line.
pub fn print_json(out: &mut dyn Write, value: &Value, style: Style) -> io::Result<()> {
    out.write_all(format!("{}\n", render(value, style)).as_bytes())
}

/// The error line: `{"error": {"code", "class", "message"}}`, the message
/// being the whole context chain with secrets redacted.
pub fn error_value(err: &anyhow::Error, redactor: &Redactor) -> Value {
    let facts = coded_error::facts_anyhow(err);
    json!({"error": {
        "code": facts.code,
        "class": facts.class.as_str(),
        "message": redactor.redact(&format!("{err:#}")),
    }})
}

/// Writes the error line: indented on a terminal, else one compact line.
pub fn write_error(
    out: &mut dyn Write,
    err: &anyhow::Error,
    redactor: &Redactor,
    stderr_tty: bool,
) -> io::Result<()> {
    let style = if stderr_tty {
        Style::Pretty
    } else {
        Style::Compact
    };
    print_json(out, &error_value(err, redactor), style)
}

/// The line for a notice: `{"notice": ...}` or `{"progress": {...}}`.
pub fn notice_line(n: &Notice) -> Value {
    match n {
        Notice::Text(text) => json!({"notice": text}),
        Notice::Progress { done, total, label } => {
            json!({"progress": {"done": done, "total": total, "label": label}})
        }
    }
}

// Output schema discovery: `Some(schema)` when the `schema` feature is on.

/// Produces the JSON schema of an operation output type, when available.
pub trait OutputSchema {
    fn output_schema() -> Option<serde_json::Value>;
}

#[cfg(feature = "schema")]
impl<T: schemars::JsonSchema> OutputSchema for T {
    fn output_schema() -> Option<serde_json::Value> {
        Some(serde_json::to_value(schemars::schema_for!(T)).expect("schemars output is valid JSON"))
    }
}

#[cfg(not(feature = "schema"))]
impl<T> OutputSchema for T {
    fn output_schema() -> Option<serde_json::Value> {
        None
    }
}

/// Writes `bytes` to `path` for key material: a new file at mode 0600 on
/// unix. An existing file is an `AlreadyExists` error unless `overwrite`, in
/// which case it is truncated and its mode set to 0600. Missing parent
/// directories are created (mode 0700 on unix). The file is synced to disk,
/// and on unix so is its directory, so the write survives a crash.
pub fn write_private(path: &std::path::Path, bytes: &[u8], overwrite: bool) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        let mut dirs = std::fs::DirBuilder::new();
        dirs.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            dirs.mode(0o700);
        }
        dirs.create(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    std::fs::File::open(parent.unwrap_or(std::path::Path::new(".")))?.sync_all()?;
    Ok(())
}
