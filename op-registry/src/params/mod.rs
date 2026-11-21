//! The `OpParams` trait, field metadata and usage errors.

#[doc(hidden)]
pub mod base64;
#[doc(hidden)]
pub mod rt;
#[cfg(feature = "secrets")]
mod secret;
mod types;

#[cfg(feature = "secrets")]
pub use secret::{Secret, SecretFile};
pub use types::{Input, Json, Patch};

use std::io::Read;

use serde_json::Value;

/// Per-parse state shared by the parameter decoders.
#[derive(Default)]
pub struct ParseCx {
    /// Where `-` and `@-` read from; `None` on paths without stdin.
    pub(crate) stdin: Option<Box<dyn Read + Send>>,
    /// The first field that read stdin.
    pub(crate) stdin_used_by: Option<String>,
    /// Whether stdin is a terminal.
    stdin_tty: bool,
    /// Where the stdin hint goes.
    hint: crate::Notices,
    /// Reads prompted secrets; used by prompted fields.
    #[cfg(feature = "secrets")]
    prompter: Option<std::sync::Arc<dyn crate::cli::Prompter>>,
}

impl std::fmt::Debug for ParseCx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParseCx")
            .field("stdin", &self.stdin.is_some())
            .field("stdin_used_by", &self.stdin_used_by)
            .field("stdin_tty", &self.stdin_tty)
            .finish()
    }
}

impl ParseCx {
    /// A context whose stdin is `reader`.
    pub fn with_stdin(reader: impl Read + Send + 'static) -> Self {
        Self {
            stdin: Some(Box::new(reader)),
            ..Self::default()
        }
    }

    /// A context for a CLI run: `stdin`, whether it is a terminal, the sink
    /// of the stdin hint and the prompter.
    pub(crate) fn for_run(
        stdin: Option<Box<dyn Read + Send>>,
        stdin_tty: bool,
        hint: crate::Notices,
        #[cfg(feature = "secrets")] prompter: Option<std::sync::Arc<dyn crate::cli::Prompter>>,
    ) -> Self {
        Self {
            stdin,
            stdin_used_by: None,
            stdin_tty,
            hint,
            #[cfg(feature = "secrets")]
            prompter,
        }
    }

    /// Reads all of stdin for `field`. Only one field may do so per parse.
    pub fn read_stdin(&mut self, field: &str) -> anyhow::Result<Vec<u8>> {
        if let Some(first) = &self.stdin_used_by {
            return Err(usage(UsageError::StdinTwice {
                first: first.clone(),
                second: field.to_owned(),
            }));
        }
        let Some(reader) = self.stdin.as_mut() else {
            return Err(usage(UsageError::InvalidField {
                name: field.to_owned(),
                reason: "stdin is not available on this path".to_owned(),
            }));
        };
        self.stdin_used_by = Some(field.to_owned());
        if self.stdin_tty {
            self.hint
                .notice(format!("reading --{field} from stdin (end with Ctrl-D)"));
        }
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(|e| {
            usage(UsageError::InvalidField {
                name: field.to_owned(),
                reason: format!("cannot read stdin: {e}"),
            })
        })?;
        Ok(bytes)
    }
}

/// Replaces known secret strings in text with `<secret>`.
///
/// A shared handle: a clone sees every secret added to any of its clones, so
/// one call hands the same redactor to every place that prints. It keeps
/// exact strings only, so text an app has transformed (cut, re-encoded) is
/// not caught. [`Redactor::detached`] gives an independent copy.
#[derive(Clone, Default)]
pub struct Redactor(std::sync::Arc<std::sync::RwLock<Vec<String>>>);

impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redactor")
            .field("secrets", &self.read().len())
            .finish()
    }
}

impl Redactor {
    /// The shortest value worth redacting; shorter ones would mangle text.
    const MIN_LEN: usize = 4;

    /// An empty redactor.
    pub fn new() -> Self {
        Self::default()
    }

    /// The secrets, longest first. A poisoned lock still holds a whole list:
    /// every writer leaves it consistent.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Vec<String>> {
        self.0.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Vec<String>> {
        self.0.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a secret: the whole value and each non-empty trimmed line of it,
    /// read as UTF-8 (lossy). Values shorter than 4 bytes are skipped. Every
    /// clone of this handle sees it.
    pub fn add(&self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let lines = text.lines().map(str::trim);
        let mut secrets = self.write();
        for candidate in std::iter::once(text.as_ref()).chain(lines) {
            if candidate.len() >= Self::MIN_LEN && !secrets.iter().any(|s| s == candidate) {
                secrets.push(candidate.to_owned());
            }
        }
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    }

    /// Adds every secret of `other`.
    pub fn extend(&self, other: &Redactor) {
        if std::sync::Arc::ptr_eq(&self.0, &other.0) {
            return;
        }
        let incoming = other.read().clone();
        let mut secrets = self.write();
        for secret in incoming {
            if !secrets.contains(&secret) {
                secrets.push(secret);
            }
        }
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    }

    /// Whether nothing is to be redacted.
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    /// An independent copy of the secrets so far: later additions to either
    /// side do not reach the other.
    pub fn detached(&self) -> Redactor {
        Redactor(std::sync::Arc::new(std::sync::RwLock::new(
            self.read().clone(),
        )))
    }

    /// `text` with each secret replaced by `<secret>`, longest first.
    pub fn redact(&self, text: &str) -> String {
        self.read().iter().fold(text.to_owned(), |out, s| {
            out.replace(s.as_str(), "<secret>")
        })
    }
}

/// How a field is presented on the command line and in the schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldKind {
    /// One value (`String`, numbers, enums), possibly optional.
    Scalar,
    /// A `bool` switch.
    Switch,
    /// An `Option<bool>` with `--x` and `--no-x`.
    TriState,
    /// A `Vec<T>`: a repeated flag.
    Repeated,
    /// A `BTreeMap<String, String>`: a repeated `k=v` flag.
    Map,
    /// Arbitrary JSON.
    Json,
    /// Bytes read from a path or stdin.
    Input,
    /// A reference to another object.
    Ref,
    /// A secret read from a file.
    SecretFile,
    /// A `PathBuf` or `Option<PathBuf>`.
    Path,
    /// A clearable `Patch<T>`.
    Patch,
}

/// The subject marker of a field: the kind it names and how it is resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubjectMeta {
    /// The kind of object the subject names.
    pub kind: String,
    /// The id passes through without resolution.
    pub id_only: bool,
    /// The subject may be absent: the id passes through, and the object need not exist.
    pub absent_ok: bool,
}

impl SubjectMeta {
    /// A subject of `kind`, resolved normally.
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id_only: false,
            absent_ok: false,
        }
    }

    pub fn id_only(mut self, yes: bool) -> Self {
        self.id_only = yes;
        self
    }

    pub fn absent_ok(mut self, yes: bool) -> Self {
        self.absent_ok = yes;
        self
    }
}

/// What the derive knows about one field.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FieldMeta {
    /// The Rust field name.
    pub rust_name: &'static str,
    /// The long flag without its dashes (`agent-id`).
    pub flag: String,
    /// The JSON key (`agentId`).
    pub key: String,
    /// The field's doc comment, lines joined by spaces.
    pub help: String,
    pub kind: FieldKind,
    /// Must be given: no default, not optional.
    pub required: bool,
    pub secret: bool,
    /// The shared param group the field came from, if any.
    pub from_group: Option<&'static str>,
    /// Set on the op's subject.
    pub subject: Option<SubjectMeta>,
    /// The environment variable the field falls back to.
    pub env: Option<String>,
    /// The label of a prompted `SecretFile`.
    pub prompt: Option<String>,
}

impl FieldMeta {
    /// A field named `rust_name`: the flag is its kebab-case, the key its camelCase.
    pub fn new(rust_name: &'static str, kind: FieldKind) -> Self {
        let mut key = String::new();
        for (i, part) in rust_name.split('_').filter(|p| !p.is_empty()).enumerate() {
            if i == 0 {
                key.push_str(part);
            } else {
                let mut chars = part.chars();
                if let Some(c) = chars.next() {
                    key.extend(c.to_uppercase());
                    key.push_str(chars.as_str());
                }
            }
        }
        Self {
            rust_name,
            flag: rust_name.replace('_', "-"),
            key,
            help: String::new(),
            kind,
            required: false,
            secret: false,
            from_group: None,
            subject: None,
            env: None,
            prompt: None,
        }
    }

    pub fn with_flag(mut self, flag: impl Into<String>) -> Self {
        self.flag = flag.into();
        self
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }

    pub fn required(mut self, yes: bool) -> Self {
        self.required = yes;
        self
    }

    pub fn secret(mut self, yes: bool) -> Self {
        self.secret = yes;
        self
    }

    pub fn with_subject(mut self, subject: SubjectMeta) -> Self {
        self.subject = Some(subject);
        self
    }

    pub fn with_env(mut self, env: impl Into<String>) -> Self {
        self.env = Some(env.into());
        self
    }

    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }
}

/// One parameter struct decoded from clap matches or JSON.
pub trait OpParams: Sized + Send + 'static {
    fn fields() -> Vec<FieldMeta>;
    fn input_schema() -> Value;
    fn add_args(cmd: clap::Command) -> clap::Command;
    fn from_matches(m: &clap::ArgMatches, cx: &mut ParseCx) -> anyhow::Result<Self>;
    fn from_json(v: Value, cx: &mut ParseCx) -> anyhow::Result<Self>;
    fn validate(&self) -> anyhow::Result<()> {
        Ok(())
    }
    /// The refs of this struct: non-subject refs first, then the subject.
    ///
    /// Fails when a scope value cannot be read (a path that is not UTF-8).
    fn ref_slots(&mut self) -> anyhow::Result<Vec<crate::RefSlot<'_>>> {
        Ok(Vec::new())
    }
    fn secrets(&self) -> Vec<Vec<u8>> {
        Vec::new()
    }
    /// The struct's relations as plain lines, flattened groups' included:
    /// "--to is required unless --signed-file is given", "give exactly one
    /// of --id, --action". Help and the manual print them as the op's rules.
    fn relations() -> Vec<String> {
        Vec::new()
    }
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "op_registry.usage")]
#[non_exhaustive]
pub enum UsageError {
    #[error("missing field {name}")]
    #[coded(code = "missing", class = Logical)]
    MissingField { name: String },
    #[error("invalid field {name}: {reason}")]
    #[coded(code = "invalid", class = Logical)]
    InvalidField { name: String, reason: String },
    #[error("unknown key {key} for {param}")]
    #[coded(code = "unknown_key", class = Logical)]
    UnknownKey { param: String, key: String },
    #[error("repeated key {key} for {param}")]
    #[coded(code = "repeated_key", class = Logical)]
    RepeatedKey { param: String, key: String },
    #[error("exactly one of the {group} fields is required")]
    #[coded(code = "one_of", class = Logical)]
    OneOf { group: String },
    #[error("{field} is required when {other} is given")]
    #[coded(code = "requires_if", class = Logical)]
    RequiresIf { field: String, other: String },
    #[error("{field} conflicts with {other}")]
    #[coded(code = "conflicts", class = Logical)]
    Conflicts { field: String, other: String },
    /// A `required_unless` field with every target absent. It shares the code
    /// of [`UsageError::MissingField`], which the derive does not allow within
    /// one enum, so the variant forwards to [`MissingUnless`].
    #[error(transparent)]
    #[coded(transparent, catalog)]
    MissingUnless(MissingUnless),
    #[error("at least one of the {group} fields is required")]
    #[coded(code = "at_least_one", class = Logical)]
    AtLeastOne { group: String },
    #[error("{first} and {second} both read stdin")]
    #[coded(code = "stdin_twice", class = Logical)]
    StdinTwice { first: String, second: String },
}

/// The error of a `required_unless` field that is absent with every target
/// absent. Its code is `op_registry.usage.missing`, the code of
/// [`UsageError::MissingField`]; the manual lists the code once.
#[derive(Debug, thiserror::Error)]
#[error("{name} is required unless {unless} is given")]
pub struct MissingUnless {
    pub name: String,
    pub unless: String,
}

impl MissingUnless {
    const CODE: &'static str = "op_registry.usage.missing";
}

impl coded_error::CodedError for MissingUnless {
    fn code(&self) -> &'static str {
        Self::CODE
    }
    fn class(&self) -> coded_error::ErrorClass {
        coded_error::ErrorClass::Logical
    }
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

impl coded_error::CodeCatalog for MissingUnless {
    fn codes() -> Vec<coded_error::CodeEntry> {
        vec![coded_error::CodeEntry {
            code: Self::CODE,
            class: Some(coded_error::ErrorClass::Logical),
        }]
    }
}

/// Wraps a [`UsageError`] as the error an op or a validator returns: it exits
/// 2 and reaches [`RunHooks::on_usage_error`](crate::cli::RunHooks::on_usage_error).
pub fn usage(error: UsageError) -> anyhow::Error {
    coded_error::into_anyhow(error)
}

#[cfg(test)]
mod redactor_tests {
    use super::Redactor;

    #[test]
    fn poisoned_lock_still_redacts() {
        let r = Redactor::new();
        r.add(b"poison-secret");
        let held = r.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.0.write().unwrap();
            panic!("poison the lock");
        })
        .join();
        assert!(r.0.is_poisoned());
        assert_eq!(r.redact("x poison-secret"), "x <secret>");
        r.add(b"after-poison");
        assert_eq!(r.redact("after-poison"), "<secret>");
    }
}
