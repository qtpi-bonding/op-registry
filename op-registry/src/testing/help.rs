//! `assert_help_complete`: every visible op, group and flag is documented.

use super::fail_on;
use crate::{CliContext, Descriptor, OpKind, Registry};
use clap::Command;
use serde_json::Value;
use std::collections::BTreeMap;

/// What help text must not contain.
pub struct HelpRules<'a> {
    /// Words that must not appear, matched case-insensitively.
    pub banned_words: &'a [&'a str],
}

/// Checks the help of every visible op, group and flag: it is not empty, one
/// flag name has one wording across all ops, and no text holds the home
/// directory, `/Users/`, `/home/` or a banned word.
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
    let mut wordings: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    walk(&root, &mut vec![], &mut problems, &mut texts, &mut wordings);

    for (flag, by_wording) in &wordings {
        if by_wording.len() > 1 {
            let mut report = format!("flag --{flag} has {} wordings:", by_wording.len());
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

fn walk(
    cmd: &Command,
    path: &mut Vec<String>,
    problems: &mut Vec<String>,
    texts: &mut Vec<(String, String)>,
    wordings: &mut BTreeMap<String, BTreeMap<String, Vec<String>>>,
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
        if id == "help" || id == "version" {
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
            wordings
                .entry(long.to_owned())
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
        walk(sub, path, problems, texts, wordings);
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
        let single = get.output_schema.as_ref().map(|s| inline(s, s, 0));
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
    Some(inline(root.pointer("/properties/items/items")?, root, 0))
}

/// Replaces every `$ref` with its target in `root`, and drops the keys that
/// differ between a root schema and a definition (`$schema`, `title`, `$defs`).
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
                .filter(|(key, _)| !matches!(key.as_str(), "$schema" | "title" | "$defs"))
                .map(|(key, value)| (key.clone(), inline(value, root, depth + 1)))
                .collect()
        }
        Value::Array(items) => items.iter().map(|v| inline(v, root, depth + 1)).collect(),
        other => other.clone(),
    }
}
