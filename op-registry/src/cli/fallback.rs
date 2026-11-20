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

/// The root fallback of a registry.
pub(crate) enum Fallback {
    /// Gets the raw arguments of an empty or flag-led command line.
    FlagsOnly {
        name: &'static str,
        help: &'static str,
        run: fn(Vec<OsString>) -> BoxFuture<'static, u8>,
    },
    /// Gets everything that is not a command, after the registry's globals.
    FreeText {
        name: &'static str,
        help: &'static str,
        flags: &'static [&'static str],
        run: FreeTextRun,
    },
}

impl Fallback {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Fallback::FlagsOnly { name, .. } | Fallback::FreeText { name, .. } => name,
        }
    }

    pub(crate) fn help(&self) -> &'static str {
        match self {
            Fallback::FlagsOnly { help, .. } | Fallback::FreeText { help, .. } => help,
        }
    }

    /// The line the root help lists: `<name>: <first line of help>`.
    pub(crate) fn listing(&self) -> String {
        format!(
            "{}: {}",
            self.name(),
            self.help().lines().next().unwrap_or_default()
        )
    }
}

impl<C: CliContext + Sync> Registry<C> {
    /// Sets the root fallback that gets the raw arguments of an empty command
    /// line or one whose first argument starts with `-`. A first bare word
    /// that is not a command stays a usage error. `run` returns the exit code.
    ///
    /// # Panics
    ///
    /// Panics when the registry already has a fallback.
    pub fn fallback_flags_only(
        &mut self,
        name: &'static str,
        help: &'static str,
        run: fn(Vec<OsString>) -> BoxFuture<'static, u8>,
    ) {
        self.set_fallback(Fallback::FlagsOnly { name, help, run });
    }

    /// Sets the root fallback that gets every argument after the registry's
    /// globals that is not a command: free text. `flags` are the long flags
    /// (no dashes) the fallback reads itself; they must not reuse a registry
    /// global's name, which `cli::command` checks. `run` gets the parsed
    /// globals and returns the exit code.
    ///
    /// # Panics
    ///
    /// Panics when the registry already has a fallback.
    pub fn fallback_free_text(
        &mut self,
        name: &'static str,
        help: &'static str,
        flags: &'static [&'static str],
        run: for<'a> fn(&'a C::Globals, Vec<OsString>) -> BoxFuture<'a, u8>,
    ) {
        let run: FreeTextRun = Arc::new(
            move |matches: &ArgMatches, args: Vec<OsString>| -> BoxFuture<'_, anyhow::Result<u8>> {
                Box::pin(async move {
                    let globals = <C::Globals as OpGlobals>::from_matches(matches)?;
                    Ok(run(&globals, args).await)
                })
            },
        );
        self.set_fallback(Fallback::FreeText {
            name,
            help,
            flags,
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

/// How many leading tokens are registry globals with their values.
fn skip_globals(cmd: &Command, tokens: &[OsString]) -> usize {
    let mut at = 0;
    while let Some(token) = tokens.get(at) {
        let token = token.to_string_lossy();
        let arg = if let Some(rest) = token.strip_prefix("--") {
            let name = rest.split_once('=').map_or(rest, |(name, _)| name);
            cmd.get_arguments()
                .find(|a| a.is_global_set() && a.get_long() == Some(name))
        } else if let Some(short) = token.strip_prefix('-').filter(|s| s.chars().count() == 1) {
            let short = short.chars().next();
            cmd.get_arguments()
                .find(|a| a.is_global_set() && a.get_short() == short)
        } else {
            None
        };
        let Some(arg) = arg else { break };
        let value_follows = arg.get_action().takes_values() && !token.contains('=');
        at += 1 + usize::from(value_follows);
    }
    at.min(tokens.len())
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
        Fallback::FlagsOnly { .. } => 0,
        Fallback::FreeText { .. } => skip_globals(cmd, tokens),
    };
    let rest = &tokens[skipped..];
    let text = |at: usize| rest.get(at).map(|t| t.to_string_lossy().into_owned());

    if text(0).as_deref() == Some("help") && text(1).as_deref() == Some(fallback.name()) {
        return Ok(Some((Parsed::Help(fallback.help().to_owned()), None)));
    }
    let args = match fallback {
        Fallback::FlagsOnly { .. } => {
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
        Fallback::FreeText { .. } => {
            let Some(first) = text(0) else {
                return Ok(None);
            };
            if first == "--" {
                rest[1..].to_vec()
            } else if first.starts_with('-') {
                rest.to_vec()
            } else {
                let words = top_words(registry);
                if words.contains(&first) {
                    return Ok(None);
                }
                if rest.len() == 1 && !first.chars().any(char::is_whitespace) {
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
                rest.to_vec()
            }
        }
    };
    // The globals are the leading tokens; the root needs no command for them.
    let prefix = argv.iter().take(1).chain(&tokens[..skipped]);
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
