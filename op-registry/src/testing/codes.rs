//! `assert_exit_table`, `strict_codes` and `assert_error_codes`: every code
//! an op returns is one it declared (spec 6.3), and the manual lists them.

use super::fail_on;
use crate::manual::{render, Manual};
use crate::{CallOptions, CliContext, Descriptor, OpKind, Registry, Resolve};
use futures_util::StreamExt;
use serde_json::Value;

/// One call of an op a test runs: the op's name and its JSON arguments.
pub struct Fixture<'a> {
    pub op: &'a str,
    pub args: Value,
}

/// The codes an op may return: its declared errors, and the cap code of a
/// list op. Every `op_registry.*` code is allowed on top of these.
fn declared_codes<C: Sync>(registry: &Registry<C>, d: &Descriptor) -> Vec<String> {
    let mut codes: Vec<String> = d.errors.iter().map(|e| e.code.clone()).collect();
    if d.kind == OpKind::ListAll && d.path.len() >= 2 {
        codes.push(crate::list::too_many_code(registry.pillar(), &d.path));
    }
    codes
}

/// What is wrong with `err` for op `d`: it is uncoded, or its code is not
/// declared. `None` when the error is fine.
fn offence<C: Sync>(registry: &Registry<C>, d: &Descriptor, err: &anyhow::Error) -> Option<String> {
    let code = coded_error::facts_anyhow(err).code;
    if code == coded_error::UNKNOWN_CODE {
        return Some(format!("an uncoded error: {err:#}"));
    }
    if code.starts_with("op_registry.") || declared_codes(registry, d).iter().any(|c| c == code) {
        return None;
    }
    Some(format!("the undeclared code `{code}`: {err:#}"))
}

/// Panics when `err`, returned by op `name`, is outside the op's declared
/// codes. Called by the call pipeline under [`strict_codes`].
pub(crate) fn check_strict<C: Sync>(registry: &Registry<C>, name: &str, err: &anyhow::Error) {
    if let Some(d) = registry.get(name) {
        if let Some(what) = offence(registry, d, err) {
            panic!("strict_codes: op `{name}` returned {what}");
        }
    }
}

/// From now on a call that returns a code outside the op's declared errors,
/// `op_registry.*` and the op's `too_many`, or any uncoded error, panics
/// naming the op and the code.
pub fn strict_codes<C>(registry: &mut Registry<C>) {
    registry.strict_codes = true;
}

/// The errors one call of `d` produced: the call's own, and for a stream
/// every item error. Empty when the call succeeded.
pub(crate) async fn call_errors<C: Resolve>(
    registry: &Registry<C>,
    ctx: &C,
    d: &Descriptor,
    args: Value,
) -> Vec<anyhow::Error> {
    if d.kind == OpKind::Stream {
        match registry
            .call_stream(&d.name, args, ctx, CallOptions::default(), None)
            .await
        {
            Err(err) => vec![err],
            Ok(mut items) => {
                let mut errors = Vec::new();
                while let Some(item) = items.next().await {
                    if let Err(err) = item {
                        errors.push(err);
                    }
                }
                errors
            }
        }
    } else {
        registry
            .call(&d.name, args, ctx, CallOptions::default())
            .await
            .err()
            .map(crate::CallFailure::into_inner)
            .into_iter()
            .collect()
    }
}

/// Runs each fixture through `Registry::call` and reports every error whose
/// code the op did not declare, and every uncoded error.
///
/// Do not combine it with [`strict_codes`]: a strict panic stops the report
/// at the first offender.
///
/// # Panics
///
/// Panics with every offender.
pub async fn assert_error_codes<C: Resolve>(
    registry: &Registry<C>,
    ctx: &C,
    fixtures: &[Fixture<'_>],
) {
    let mut problems = Vec::new();
    for fixture in fixtures {
        let Some(d) = registry.get(fixture.op) else {
            problems.push(format!(
                "fixture names `{}`, which is not in the registry",
                fixture.op
            ));
            continue;
        };
        for err in call_errors(registry, ctx, d, fixture.args.clone()).await {
            if let Some(what) = offence(registry, d, &err) {
                problems.push(format!("{} returned {what}", d.name));
            }
        }
    }
    fail_on("assert_error_codes", &problems);
}

/// The text of the manual section of `heading`, up to the next heading.
fn section<'a>(rendered: &'a str, heading: &str) -> Option<&'a str> {
    let marker = format!("\n{heading}\n");
    let start = rendered.find(&marker)? + marker.len();
    let rest = &rendered[start..];
    let end = rest.find("\n### ").or_else(|| rest.find("\n## "));
    Some(end.map_or(rest, |end| &rest[..end]))
}

/// The problems of an already rendered manual: a missing exit-table row,
/// a declared exit outside 3..=63, and a declared exit or code that a
/// visible command's section does not list.
pub fn check_exit_table<C: CliContext>(registry: &Registry<C>, rendered: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for (code, meaning) in crate::manual::EXITS {
        if !rendered.contains(&format!("\n| {code} | {meaning} |")) {
            problems.push(format!(
                "the exit table has no row `| {code} | {meaning} |`"
            ));
        }
    }
    for index in 0..registry.entries.len() {
        let d = &registry.entries[index].descriptor;
        for exit in &d.exits {
            if !(3..=63).contains(&exit.code) {
                problems.push(format!(
                    "{}: declared exit {} is outside 3..=63",
                    d.name, exit.code
                ));
            }
        }
        if d.hidden || !crate::cli::on_cli(registry, index) {
            continue;
        }
        let heading = format!("### {} {}", registry.bin(), d.path.join(" "));
        let Some(text) = section(rendered, &heading) else {
            problems.push(format!("{}: the manual has no section `{heading}`", d.name));
            continue;
        };
        for exit in &d.exits {
            if !text.contains(&format!("- {}: {}", exit.code, exit.meaning)) {
                problems.push(format!(
                    "{}: declared exit {} ({}) is not in the manual",
                    d.name, exit.code, exit.meaning
                ));
            }
        }
        for code in declared_codes(registry, d) {
            if !text.contains(&format!("| {code} |")) {
                problems.push(format!("{}: code `{code}` is not in the manual", d.name));
            }
        }
    }
    problems
}

/// Renders the manual and checks its exit table, and that every declared
/// exit and error code of every visible command is in it.
///
/// # Panics
///
/// Panics with every problem found, or when the manual cannot be rendered.
pub fn assert_exit_table<C: CliContext>(registry: &Registry<C>, manual: &Manual) {
    let rendered = render(registry, manual)
        .unwrap_or_else(|err| panic!("assert_exit_table: the manual does not render: {err:#}"));
    fail_on("assert_exit_table", &check_exit_table(registry, &rendered));
}
