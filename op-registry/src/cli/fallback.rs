//! The root fallbacks: what runs when the command line names no command.

use super::parse::{decode_failure, map_error, unknown_command};
use super::tree::top_words;
use super::{Parsed, UsageFailure};
use crate::{CliContext, OpGlobals, Registry};
use clap::{ArgMatches, Command};
use futures_util::future::BoxFuture;
use std::ffi::OsString;
use std::sync::Arc;

/// The erased runner of a free-text fallback; it decodes the globals itself.
pub(crate) type FreeTextRun = Arc<
    dyn for<'a> Fn(&'a ArgMatches, Vec<OsString>) -> BoxFuture<'a, anyhow::Result<u8>>
        + Send
        + Sync,
>;

/// A flag a free-text fallback reads itself.
#[derive(Clone, Copy, Debug)]
pub struct FallbackFlag {
    /// The long name, without dashes.
    pub long: &'static str,
    /// The short name, when it has one.
    pub short: Option<char>,
    /// Whether the next token is the flag's value (kept unscanned for
    /// registry globals).
    pub takes_value: bool,
}

/// The root fallback of a registry.
pub(crate) enum Fallback {
    /// Gets the raw arguments of an empty or flag-led command line.
    FlagsOnly {
        name: &'static str,
        help: &'static str,
        run: fn(Vec<OsString>) -> BoxFuture<'static, anyhow::Result<u8>>,
    },
    /// Routes like `FlagsOnly` but has no runner: the host dispatches it after
    /// `cli::select`, before any runtime.
    BeforeRuntime {
        name: &'static str,
        help: &'static str,
    },
    /// Gets everything that is not a command, after the registry's globals.
    FreeText {
        name: &'static str,
        help: &'static str,
        flags: &'static [FallbackFlag],
        near_miss: bool,
        run: FreeTextRun,
    },
}

impl Fallback {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Fallback::FlagsOnly { name, .. }
            | Fallback::BeforeRuntime { name, .. }
            | Fallback::FreeText { name, .. } => name,
        }
    }

    /// The help text, with `{bin}` replaced by the binary's name.
    pub(crate) fn help(&self, bin: &str) -> String {
        match self {
            Fallback::FlagsOnly { help, .. }
            | Fallback::BeforeRuntime { help, .. }
            | Fallback::FreeText { help, .. } => help.replace("{bin}", bin),
        }
    }

    /// The line the root help lists:
    /// `<name>: <first line of help> (more: <bin> help <name>)`.
    pub(crate) fn listing(&self, bin: &str) -> String {
        let help = self.help(bin);
        format!(
            "{name}: {} (more: {bin} help {name})",
            help.lines().next().unwrap_or_default(),
            name = self.name()
        )
    }
}

impl<C: CliContext + Sync> Registry<C> {
    /// Sets the root fallback that gets the raw arguments of an empty command
    /// line or one whose first argument starts with `-`. A first bare word
    /// that is not a command stays a usage error. `run` returns the exit code;
    /// an `Err` ends the run like a failed op (`post_run`, the error line, the
    /// exit code of its class).
    ///
    /// # Panics
    ///
    /// Panics when the registry already has a fallback.
    pub fn fallback_flags_only(
        &mut self,
        name: &'static str,
        help: &'static str,
        run: fn(Vec<OsString>) -> BoxFuture<'static, anyhow::Result<u8>>,
    ) {
        self.set_fallback(Fallback::FlagsOnly { name, help, run });
    }

    /// Sets a root fallback like [`Registry::fallback_flags_only`] that has no
    /// runner: it routes, shows in help and in the manual, and `help <name>`
    /// works, but the host dispatches it itself after [`crate::cli::select`],
    /// on the main thread before any runtime. If [`crate::cli::run`] reaches
    /// it, the run fails with an uncoded error naming `cli::select` (exit 70).
    ///
    /// # Panics
    ///
    /// Panics when the registry already has a fallback.
    pub fn fallback_flags_only_before_runtime(&mut self, name: &'static str, help: &'static str) {
        self.set_fallback(Fallback::BeforeRuntime { name, help });
    }

    /// Sets the root fallback that gets every argument that is not a command:
    /// free text. An empty command line (or only globals) reaches it with an
    /// empty `Vec`. Registry globals are taken out of the whole tail up to
    /// `--`; a declared flag with `takes_value` keeps its next token as its
    /// value, unscanned. `flags` must not reuse a registry or library global's
    /// long or short name, which `cli::command` checks. With `near_miss`, a
    /// single word within edit distance 2 of a command is a usage error
    /// instead of free text. `run` gets the parsed globals and returns the
    /// exit code; an `Err` ends the run like a failed op.
    ///
    /// # Panics
    ///
    /// Panics when the registry already has a fallback.
    pub fn fallback_free_text(
        &mut self,
        name: &'static str,
        help: &'static str,
        flags: &'static [FallbackFlag],
        near_miss: bool,
        run: for<'a> fn(&'a C::Globals, Vec<OsString>) -> BoxFuture<'a, anyhow::Result<u8>>,
    ) {
        let run: FreeTextRun = Arc::new(
            move |matches: &ArgMatches, args: Vec<OsString>| -> BoxFuture<'_, anyhow::Result<u8>> {
                Box::pin(async move {
                    let globals = <C::Globals as OpGlobals>::from_matches(matches)?;
                    run(&globals, args).await
                })
            },
        );
        self.set_fallback(Fallback::FreeText {
            name,
            help,
            flags,
            near_miss,
            run,
        });
    }

    fn set_fallback(&mut self, fallback: Fallback) {
        assert!(
            self.fallback.is_none(),
            "the registry already has a fallback (`{}`)",
            self.fallback.as_ref().map_or("", Fallback::name)
        );
        self.fallback = Some(fallback);
    }
}

/// The Levenshtein distance between two strings, in characters.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != *cb))
                .min(above + 1)
                .min(row[j] + 1);
            diagonal = above;
        }
    }
    row[b.len()]
}

/// The registry global a token names (`--name`, `--name=value` or `-s`).
fn global_arg<'a>(cmd: &'a Command, token: &str) -> Option<&'a clap::Arg> {
    if let Some(rest) = token.strip_prefix("--") {
        let name = rest.split_once('=').map_or(rest, |(name, _)| name);
        cmd.get_arguments()
            .find(|a| a.is_global_set() && a.get_long() == Some(name))
    } else if let Some(short) = token.strip_prefix('-').filter(|s| s.chars().count() == 1) {
        let short = short.chars().next();
        cmd.get_arguments()
            .find(|a| a.is_global_set() && a.get_short() == short)
    } else {
        None
    }
}

/// How many tokens a registry global at the start of `tokens` takes: 0 when
/// the first token is no global, else the token and its value.
fn global_len(cmd: &Command, tokens: &[OsString]) -> usize {
    let Some(token) = tokens.first() else {
        return 0;
    };
    let token = token.to_string_lossy();
    let Some(arg) = global_arg(cmd, &token) else {
        return 0;
    };
    let value_follows = arg.get_action().takes_values() && !token.contains('=');
    (1 + usize::from(value_follows)).min(tokens.len())
}

/// How many leading tokens are registry globals with their values.
fn skip_globals(cmd: &Command, tokens: &[OsString]) -> usize {
    let mut at = 0;
    loop {
        let len = global_len(cmd, &tokens[at..]);
        if len == 0 {
            return at;
        }
        at += len;
    }
}

/// Whether a token is a declared fallback flag that takes a value as its
/// next token (`--flag` or `-f`, not `--flag=value`).
fn takes_next_value(flags: &[FallbackFlag], token: &str) -> bool {
    if let Some(long) = token.strip_prefix("--") {
        return flags.iter().any(|f| f.takes_value && f.long == long);
    }
    let mut chars = token.strip_prefix('-').unwrap_or_default().chars();
    match (chars.next(), chars.next()) {
        (Some(short), None) => flags
            .iter()
            .any(|f| f.takes_value && f.short == Some(short)),
        _ => false,
    }
}

/// Splits a fallback's tail into the registry globals (with their values) and
/// the rest, scanning up to the first `--`; everything from `--` on is kept
/// unscanned.
fn take_globals(
    cmd: &Command,
    flags: &[FallbackFlag],
    tail: &[OsString],
) -> (Vec<OsString>, Vec<OsString>) {
    let (mut globals, mut kept) = (Vec::new(), Vec::new());
    let mut at = 0;
    while let Some(token) = tail.get(at) {
        if token == "--" {
            break;
        }
        let len = global_len(cmd, &tail[at..]);
        if len > 0 {
            globals.extend_from_slice(&tail[at..at + len]);
            at += len;
        } else if takes_next_value(flags, &token.to_string_lossy()) {
            let end = (at + 2).min(tail.len());
            kept.extend_from_slice(&tail[at..end]);
            at = end;
        } else {
            kept.push(token.clone());
            at += 1;
        }
    }
    kept.extend_from_slice(&tail[at..]);
    (globals, kept)
}

/// What routing decides: the parse, or nothing when no fallback applies.
type Routed<C> = Option<(Parsed, Option<<C as CliContext>::Globals>)>;

/// Routes a command line through the fallback: `Ok(None)` when the normal
/// parse decides it (a command, help or version, or no fallback).
pub(super) fn route<C: CliContext>(
    registry: &Registry<C>,
    cmd: &Command,
    argv: &[OsString],
) -> Result<Routed<C>, UsageFailure> {
    let Some(fallback) = &registry.fallback else {
        return Ok(None);
    };
    let tokens = argv.get(1..).unwrap_or_default();
    // Help and version first always show the registry's.
    if matches!(
        tokens.first().and_then(|t| t.to_str()),
        Some("-h" | "--help" | "--version")
    ) {
        return Ok(None);
    }
    let skipped = match fallback {
        Fallback::FlagsOnly { .. } | Fallback::BeforeRuntime { .. } => 0,
        Fallback::FreeText { .. } => skip_globals(cmd, tokens),
    };
    let rest = &tokens[skipped..];
    let text = |at: usize| rest.get(at).map(|t| t.to_string_lossy().into_owned());

    if text(0).as_deref() == Some("help") && text(1).as_deref() == Some(fallback.name()) {
        return Ok(Some((Parsed::Help(fallback.help(registry.bin())), None)));
    }
    let mut taken: Vec<OsString> = Vec::new();
    let args = match fallback {
        Fallback::FlagsOnly { .. } | Fallback::BeforeRuntime { .. } => {
            let flag_led = text(0).is_none_or(|t| t.starts_with('-'));
            if !flag_led {
                return Ok(None);
            }
            return Ok(Some((
                Parsed::Fallback {
                    args: rest.to_vec(),
                    matches: ArgMatches::default(),
                },
                None,
            )));
        }
        Fallback::FreeText {
            flags, near_miss, ..
        } => match text(0) {
            None => Vec::new(),
            Some(first) if first == "--" => rest[1..].to_vec(),
            Some(first) => {
                if !first.starts_with('-') {
                    let words = top_words(registry);
                    if words.contains(&first) {
                        return Ok(None);
                    }
                    if *near_miss && rest.len() == 1 && !first.chars().any(char::is_whitespace) {
                        if let Some(word) = words
                            .iter()
                            .map(|w| (edit_distance(&first, w), w))
                            .filter(|(d, _)| *d <= 2)
                            .min_by_key(|(d, _)| *d)
                            .map(|(_, w)| w)
                        {
                            return Err(unknown_command(format!(
                                "unknown command `{first}`; did you mean `{word}`?"
                            )));
                        }
                    }
                }
                let (globals, kept) = take_globals(cmd, flags, rest);
                taken = globals;
                kept
            }
        },
    };
    // The globals are the leading tokens and those taken from the tail; the
    // root needs no command for them.
    let prefix = argv
        .iter()
        .take(1)
        .chain(&tokens[..skipped])
        .chain(taken.iter());
    let matches = match cmd
        .clone()
        .subcommand_required(false)
        .try_get_matches_from(prefix)
    {
        Ok(matches) => matches,
        Err(err) => return Err(map_error(registry, cmd, argv, &err)),
    };
    let globals = <C::Globals as OpGlobals>::from_matches(&matches).map_err(decode_failure)?;
    Ok(Some((Parsed::Fallback { args, matches }, Some(globals))))
}
