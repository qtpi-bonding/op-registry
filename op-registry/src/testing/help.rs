//! `assert_help_complete`: every visible op, group and flag is documented.

use super::fail_on;
use crate::shape::mark_object_root;
use crate::{CliContext, Descriptor, OpKind, Registry};
use clap::{ArgAction, Command};
use serde_json::Value;
use std::collections::BTreeMap;

/// What help text must not contain.
#[derive(Default)]
pub struct HelpRules<'a> {
    /// Words that must not appear, matched case-insensitively.
    pub banned_words: &'a [&'a str],
    /// Flag names, without dashes, that may have different wordings on
    /// different ops; each still needs non-empty help.
    pub homonyms: &'a [&'a str],
}

/// Checks the help of every visible op, group and flag: it is not empty, one
/// flag name has one wording across all ops (`--id` one wording per subject
/// kind, except names listed in `HelpRules::homonyms`), and no text holds the
/// home directory, `/Users/`, `/home/` or a banned word. Homonym flags still
/// need non-empty help.
///
/// Returns the warnings: a verb `list` on an op that is not a list op and not
/// `unpaged`, and a list pair whose item schema differs from a sibling
/// `get`'s output schema.
///
/// # Panics
///
/// Panics with every problem found; warnings never panic.
pub fn assert_help_complete<C: CliContext>(
    registry: &Registry<C>,
    rules: &HelpRules<'_>,
) -> Vec<String> {
    let root = crate::cli::command(registry);
    let mut problems = Vec::new();
    let mut texts: Vec<(String, String)> = Vec::new();
    let mut wordings: Wordings = BTreeMap::new();
    let kinds = id_kinds(registry);
    walk(
        &root,
        &mut vec![],
        &kinds,
        &mut problems,
        &mut texts,
        &mut wordings,
    );

    for ((flag, kind), by_wording) in &wordings {
        if by_wording.len() > 1 && !rules.homonyms.contains(&flag.as_str()) {
            let of_kind = kind
                .as_ref()
                .map(|k| format!(" for subject kind {k}"))
                .unwrap_or_default();
            let mut report = format!("flag --{flag}{of_kind} has {} wordings:", by_wording.len());
            for (wording, places) in by_wording {
                report.push_str(&format!("\n  \"{wording}\" on {}", places.join(", ")));
            }
            problems.push(report);
        }
    }
    for d in registry.descriptors().into_iter().filter(|d| !d.hidden) {
        texts.push((format!("op {} summary", d.name), d.summary.clone()));
        texts.push((format!("op {} long help", d.name), d.long_help.clone()));
    }
    let home = std::env::var("HOME").ok().filter(|h| h.len() > 1);
    for (place, text) in &texts {
        let lower = text.to_lowercase();
        let mut hit = |what: &str| problems.push(format!("{place} contains {what}: \"{text}\""));
        if let Some(home) = &home {
            if text.contains(home.as_str()) {
                hit("the home directory");
            }
        }
        for path in ["/Users/", "/home/"] {
            if text.contains(path) {
                hit(&format!("`{path}`"));
            }
        }
        for word in rules.banned_words {
            if lower.contains(&word.to_lowercase()) {
                hit(&format!("the banned word `{word}`"));
            }
        }
    }
    fail_on("assert_help_complete", &problems);
    warnings(registry)
}

/// One flag, and for `--id` the kind of the subject it names, with each
/// wording it has and the commands that use it.
type Wordings = BTreeMap<(String, Option<String>), BTreeMap<String, Vec<String>>>;

/// The subject kind of each op's `--id`, by command path.
fn id_kinds<C: Sync>(registry: &Registry<C>) -> BTreeMap<Vec<String>, String> {
    registry
        .entries
        .iter()
        .filter_map(|e| {
            let field = e.param_fields.iter().find(|f| f.flag == "id")?;
            let subject = field.subject.as_ref()?;
            Some((e.descriptor.path.clone(), subject.kind.clone()))
        })
        .collect()
}

fn walk(
    cmd: &Command,
    path: &mut Vec<String>,
    kinds: &BTreeMap<Vec<String>, String>,
    problems: &mut Vec<String>,
    texts: &mut Vec<(String, String)>,
    wordings: &mut Wordings,
) {
    let here = if path.is_empty() {
        "the root command".to_owned()
    } else {
        format!("`{}`", path.join(" "))
    };
    let mut push = |what: &str, text: Option<String>| {
        if let Some(text) = text {
            texts.push((format!("{here} {what}"), text));
        }
    };
    push("about", cmd.get_about().map(|s| s.to_string()));
    push("long about", cmd.get_long_about().map(|s| s.to_string()));
    push("after help", cmd.get_after_help().map(|s| s.to_string()));
    push("before help", cmd.get_before_help().map(|s| s.to_string()));
    if !path.is_empty()
        && cmd
            .get_about()
            .is_none_or(|a| a.to_string().trim().is_empty())
    {
        problems.push(format!("{here} has no summary"));
    }
    for arg in cmd.get_arguments().filter(|a| !a.is_hide_set()) {
        let id = arg.get_id().as_str();
        // Only the root's own `help` and `version`; a leaf param may be named `version`.
        if path.is_empty() && (id == "help" || id == "version") {
            continue;
        }
        if matches!(
            arg.get_action(),
            ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
        ) {
            continue;
        }
        let label = match arg.get_long() {
            Some(long) => format!("--{long}"),
            None => format!("<{id}>"),
        };
        let help = arg.get_help().map(|s| s.to_string()).unwrap_or_default();
        if help.trim().is_empty() {
            problems.push(format!("{label} on {here} has no help"));
        }
        if let Some(long) = arg.get_long() {
            let kind = if long == "id" {
                kinds.get(path.as_slice()).cloned()
            } else {
                None
            };
            wordings
                .entry((long.to_owned(), kind))
                .or_default()
                .entry(help.clone())
                .or_default()
                .push(here.clone());
        }
        texts.push((format!("{label} on {here}"), help));
        if let Some(long) = arg.get_long_help() {
            texts.push((format!("{label} on {here} (long)"), long.to_string()));
        }
    }
    for sub in cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help" && !c.is_hide_set())
    {
        path.push(sub.get_name().to_owned());
        walk(sub, path, kinds, problems, texts, wordings);
        path.pop();
    }
}

fn warnings<C: Sync>(registry: &Registry<C>) -> Vec<String> {
    let mut warnings = Vec::new();
    for d in registry.descriptors().into_iter().filter(|d| !d.hidden) {
        let is_list = matches!(d.kind, OpKind::ListAll | OpKind::ListPage);
        if d.verb() == "list" && !is_list && !d.unpaged {
            warnings.push(format!(
                "{} has the verb `list` but is not a list op; make it a `list:` entry or mark it `unpaged`",
                d.name
            ));
        }
        if d.kind != OpKind::ListAll {
            continue;
        }
        let get = registry.descriptors().into_iter().find(|g| {
            g.verb() == "get" && g.noun_path() == d.noun_path() && g.kind != OpKind::ListPage
        });
        let Some(get) = get else { continue };
        // Both sides are marked the same way: `get`'s schema already is, and
        // an item taken from the list's array is marked as a root here.
        let single = get.output_schema.as_ref().map(|s| {
            let mut single = inline(s, s, 0);
            mark_object_root(&mut single);
            single
        });
        if let (Some(items), Some(single)) = (item_schema(d), single) {
            if items != single {
                warnings.push(format!(
                    "{} items differ from the output of {}",
                    d.name, get.name
                ));
            }
        }
    }
    warnings
}

/// The item schema of a list op's wrapped array output, with references inlined.
fn item_schema(d: &Descriptor) -> Option<Value> {
    let root = d.output_schema.as_ref()?;
    let mut item = inline(root.pointer("/properties/items/items")?, root, 0);
    mark_object_root(&mut item);
    Some(item)
}

/// Replaces every `$ref` with its target in `root`, and drops the keys that
/// differ between a root schema and a definition (`$schema`, `title`, `$defs`,
/// `definitions`).
fn inline(schema: &Value, root: &Value, depth: usize) -> Value {
    match schema {
        Value::Object(object) => {
            if let Some(Value::String(reference)) = object.get("$ref") {
                if depth < 16 {
                    if let Some(target) = reference.strip_prefix('#').and_then(|p| root.pointer(p))
                    {
                        return inline(target, root, depth + 1);
                    }
                }
            }
            object
                .iter()
                .filter(|(key, _)| {
                    !matches!(key.as_str(), "$schema" | "title" | "$defs" | "definitions")
                })
                .map(|(key, value)| (key.clone(), inline(value, root, depth + 1)))
                .collect()
        }
        Value::Array(items) => items.iter().map(|v| inline(v, root, depth + 1)).collect(),
        other => other.clone(),
    }
}
