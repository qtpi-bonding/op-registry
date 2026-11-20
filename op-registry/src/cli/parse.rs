//! Parsing argv into the selected op, its leaf matches and the globals.

use super::tree::{command, lookup};
use super::{CliError, ID_COMPACT, ID_PRETTY, ID_TEXT};
use crate::{CliContext, Cursor, OpGlobals, OpKind, Redactor, Registry, Style};
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::parser::ValueSource;
use clap::ArgMatches;
use coded_error::CodedError;
use std::ffi::OsString;

/// What a command line selects.
#[derive(Debug)]
pub enum Parsed {
    /// A registered op.
    Op {
        /// The op to run: for a list, `<name>` or `<name>_page`.
        name: String,
        /// The leaf command's matches, for the op's own decoder.
        matches: ArgMatches,
        /// `--pretty` or `--compact`, when given.
        style: Option<Style>,
        /// Whether `--text` was given.
        text: bool,
        /// The stream's `--cursor`, when given.
        cursor: Option<Cursor>,
    },
    /// An extra (non-op) subcommand.
    Extra {
        /// The subcommand's name.
        name: String,
        /// Its matches.
        matches: ArgMatches,
    },
    /// The root fallback gets the raw arguments.
    Fallback {
        /// The arguments, unparsed.
        args: Vec<OsString>,
    },
    /// `--help` output, rendered.
    Help(String),
    /// `--version` output.
    Version(String),
}

/// A command line that is not valid: its code and a message that never
/// carries a secret value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageFailure {
    /// The error code: `op_registry.usage.*` or `op_registry.cli.*`.
    pub code: &'static str,
    /// What is wrong, ready to print.
    pub message: String,
}

impl std::fmt::Display for UsageFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for UsageFailure {}

const MISSING: &str = "op_registry.usage.missing";
const INVALID: &str = "op_registry.usage.invalid";

fn unknown_command(message: String) -> UsageFailure {
    UsageFailure {
        code: CliError::UnknownCommand {
            name: String::new(),
        }
        .code(),
        message,
    }
}

/// Parses `argv` (program name first) against the registry's command tree.
///
/// A list command with `--limit` or `--cursor` selects the `_page` op;
/// the globals are read from the same matches. Help and version carry no
/// globals (`None`), so a required global never blocks them.
pub fn parse<C: CliContext>(
    registry: &Registry<C>,
    argv: &[OsString],
) -> Result<(Parsed, Option<C::Globals>), UsageFailure> {
    let cmd = command(registry);
    let matches = match cmd.clone().try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(err) => {
            return match err.kind() {
                ErrorKind::DisplayHelp => Ok((Parsed::Help(err.to_string()), None)),
                ErrorKind::DisplayVersion => {
                    Ok((Parsed::Version(err.to_string().trim_end().to_owned()), None))
                }
                _ => Err(map_error(registry, &cmd, argv, &err)),
            }
        }
    };

    let mut chain = Vec::new();
    let mut leaf = &matches;
    while let Some((name, sub)) = leaf.subcommand() {
        chain.push(name.to_owned());
        leaf = sub;
    }
    let Some(index) = lookup(registry, &chain) else {
        return Err(unknown_command(format!(
            "unknown or missing command {}",
            if chain.is_empty() {
                "<missing>".to_owned()
            } else {
                chain.join(" ")
            }
        )));
    };
    let globals = <C::Globals as OpGlobals>::from_matches(leaf).map_err(decode_failure)?;

    let descriptor = &registry.entries[index].descriptor;
    let mut name = descriptor.name.clone();
    if let (OpKind::ListAll, Some(page)) = (descriptor.kind, &descriptor.list_pair) {
        let given = |id: &str| leaf.value_source(id) == Some(ValueSource::CommandLine);
        if given("limit") || given("cursor") {
            name = page.clone();
        }
    }
    let flag = |id: &str| {
        leaf.try_get_one::<bool>(id)
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false)
    };
    let style = if flag(ID_PRETTY) {
        Some(Style::Pretty)
    } else if flag(ID_COMPACT) {
        Some(Style::Compact)
    } else {
        None
    };
    let cursor = (descriptor.kind == OpKind::Stream)
        .then(|| leaf.try_get_one::<String>("cursor").ok().flatten().cloned())
        .flatten()
        .map(Cursor::new);
    Ok((
        Parsed::Op {
            name,
            matches: leaf.clone(),
            style,
            text: flag(ID_TEXT),
            cursor,
        },
        Some(globals),
    ))
}

/// A usage failure for a decoder error.
fn decode_failure(err: anyhow::Error) -> UsageFailure {
    let code = if coded_error::facts_anyhow(&err).code == MISSING {
        MISSING
    } else {
        INVALID
    };
    UsageFailure {
        code,
        message: err.to_string(),
    }
}

/// The text of a clap error without its `error:` label, usage block and tips.
fn clean(rendered: &str) -> String {
    let text = rendered.trim_start();
    let text = text.strip_prefix("error:").unwrap_or(text).trim_start();
    text.split("\n\n").next().unwrap_or(text).trim().to_owned()
}

/// Whether `text` names the long flag `--flag` (not a longer flag).
fn names_flag(text: &str, flag: &str) -> bool {
    let needle = format!("--{flag}");
    text.match_indices(&needle).any(|(at, _)| {
        text[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '-' || c == '_'))
    })
}

/// The text of every context value of the error that names an argument.
fn named_args(err: &clap::Error) -> Vec<String> {
    let mut out = Vec::new();
    for (kind, value) in err.context() {
        if !matches!(kind, ContextKind::InvalidArg | ContextKind::PriorArg) {
            continue;
        }
        match value {
            ContextValue::String(s) => out.push(s.clone()),
            ContextValue::Strings(v) => out.extend(v.iter().cloned()),
            _ => {}
        }
    }
    out
}

/// The long flags of every secret field: the globals' and each op's.
pub(super) fn secret_flags<C: CliContext>(registry: &Registry<C>) -> Vec<String> {
    let mut flags: Vec<String> = <C::Globals as OpGlobals>::fields()
        .into_iter()
        .filter(|f| f.secret)
        .map(|f| f.flag)
        .collect();
    for entry in &registry.entries {
        flags.extend(entry.secret_flags.iter().cloned());
    }
    flags
}

/// A redactor for the values `argv` gives to the secret `flags`
/// (`--flag value` and `--flag=value`).
pub(super) fn argv_redactor(flags: &[String], argv: &[OsString]) -> Redactor {
    let mut redactor = Redactor::default();
    for (at, token) in argv.iter().enumerate() {
        let token = token.to_string_lossy();
        let Some(rest) = token.strip_prefix("--") else {
            continue;
        };
        let (name, inline) = match rest.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (rest, None),
        };
        if flags.iter().any(|f| f == name) {
            let value =
                inline.or_else(|| argv.get(at + 1).map(|v| v.to_string_lossy().into_owned()));
            if let Some(value) = value {
                redactor.add(value.as_bytes());
            }
        }
    }
    redactor
}

/// Maps a clap error to a usage failure (ruling 9), without secret values.
fn map_error<C: CliContext>(
    registry: &Registry<C>,
    cmd: &clap::Command,
    argv: &[OsString],
    err: &clap::Error,
) -> UsageFailure {
    let flags = secret_flags(registry);

    let code = match err.kind() {
        ErrorKind::MissingRequiredArgument => MISSING,
        ErrorKind::MissingSubcommand | ErrorKind::InvalidSubcommand => {
            unknown_command(String::new()).code
        }
        ErrorKind::UnknownArgument if stray_word_after_group(cmd, err) => {
            unknown_command(String::new()).code
        }
        _ => INVALID,
    };

    // An error that names a secret flag may carry its value: say nothing of it.
    let value_error = !matches!(
        err.kind(),
        ErrorKind::MissingRequiredArgument | ErrorKind::ArgumentConflict
    );
    let named = named_args(err);
    if value_error {
        if let Some(flag) = flags
            .iter()
            .find(|flag| named.iter().any(|text| names_flag(text, flag)))
        {
            return UsageFailure {
                code,
                message: format!("invalid value for --{flag}"),
            };
        }
    }

    // Any other message: scrub the values given to secret flags.
    let redactor = argv_redactor(&flags, argv);
    UsageFailure {
        code,
        message: redactor.redact(&clean(&err.to_string())),
    }
}

/// Whether an unexpected-argument error is a bare word given where a group
/// expects a verb: the usage line names the failing command, which is a group.
fn stray_word_after_group(cmd: &clap::Command, err: &clap::Error) -> bool {
    let bare = named_args(err)
        .first()
        .is_some_and(|word| !word.starts_with('-'));
    if !bare {
        return false;
    }
    let Some(ContextValue::StyledStr(usage)) = err.get(ContextKind::Usage) else {
        return true;
    };
    let usage = usage.to_string();
    let words = usage
        .trim_start()
        .trim_start_matches("Usage:")
        .split_whitespace()
        .take_while(|w| !w.starts_with(['[', '<', '-']))
        .skip(1);
    let mut at = cmd;
    for word in words {
        match at.find_subcommand(word) {
            Some(next) => at = next,
            None => return true,
        }
    }
    at.get_subcommands().any(|c| c.get_name() != "help")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpGlobals, OpParams, OpSpec, Page, PageRequest, ParseCx};
    use futures_util::future::BoxFuture;

    #[derive(OpGlobals)]
    struct NoGlobals {}

    struct Ctx;
    impl crate::Resolve for Ctx {}
    impl CliContext for Ctx {
        type Globals = NoGlobals;
        fn from_globals(_: &NoGlobals) -> anyhow::Result<Self> {
            Ok(Ctx)
        }
    }

    #[derive(Clone, OpParams)]
    struct Filter {
        tag: Option<String>,
    }

    #[test]
    fn page_leaf_matches_decode_with_the_default_limit() {
        let mut registry = Registry::<Ctx>::new();
        registry.set_pillar("fix");
        registry.group("thing", "Things.");
        let mut spec = OpSpec::<Filter, String>::new("thing list", "List.");
        spec.page_size = 7;
        registry.push_list(
            spec,
            |_c: &Ctx,
             _p: Filter,
             _r: PageRequest|
             -> BoxFuture<'_, anyhow::Result<Page<String>>> {
                Box::pin(async {
                    Ok(Page {
                        items: vec![],
                        next_cursor: None,
                    })
                })
            },
        );
        let argv: Vec<OsString> = ["app", "thing", "list", "--cursor", "c", "--tag", "t"]
            .into_iter()
            .map(OsString::from)
            .collect();
        let (Parsed::Op { name, matches, .. }, _) = parse(&registry, &argv).unwrap() else {
            panic!("not an op")
        };
        assert_eq!(name, "thing_list_page");
        let entry = registry.entry(&name).unwrap();
        let params = (entry.decode_matches)(&matches, &mut ParseCx::default()).unwrap();
        let params = params.downcast::<crate::PageParams<Filter>>().unwrap();
        assert_eq!(params.limit, 7);
        assert_eq!(params.cursor.as_deref(), Some("c"));
        assert_eq!(params.inner.tag.as_deref(), Some("t"));
    }
}
