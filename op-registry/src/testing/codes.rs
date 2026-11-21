//! `assert_exit_table`, `strict_codes` and `assert_error_codes`: every code
//! an op returns is one it declared (spec 6.3), and the manual lists them.

use super::{fail_on, fail_on_noting, kit_options, kit_path, typed_only_line, KIT_STREAM_TIMEOUT};
use crate::manual::{render, Manual};
use crate::{CallPath, CliContext, Descriptor, OpKind, Registry, Resolve};
use futures_util::StreamExt;
use serde_json::Value;

/// One call of an op a test runs: the op's name and its JSON arguments.
pub struct Fixture<'a> {
    pub op: &'a str,
    pub args: Value,
}

/// The codes an op may return: its declared errors, the registry's common
/// errors and the cap code of a list op. Every `op_registry.*` code is
/// allowed on top of these.
fn declared_codes<C: Sync>(registry: &Registry<C>, d: &Descriptor) -> Vec<String> {
    let mut codes: Vec<String> = d.errors.iter().map(|e| e.code.clone()).collect();
    codes.extend(registry.common_errors().iter().map(|e| e.code.clone()));
    if d.kind == OpKind::ListAll {
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
    // A call that could not reach the op is the kit's own mistake.
    if code == "op_registry.call.wrong_path" || code == "op_registry.call.wrong_shape" {
        return Some(format!("a call the kit should not have made: {err:#}"));
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

/// What one kit call of an op produced.
pub(crate) enum Called {
    /// The errors of the call; empty when it succeeded.
    Errors(Vec<anyhow::Error>),
    /// A stream op gave no item and did not end within [`KIT_STREAM_TIMEOUT`].
    TimedOut,
    /// A stream op was called outside a tokio runtime.
    NoRuntime,
}

impl Called {
    /// The errors, or the problem a timeout or a missing runtime is, for op
    /// `name`.
    pub(crate) fn errors_or_problem(self, name: &str) -> Result<Vec<anyhow::Error>, String> {
        match self {
            Called::Errors(errors) => Ok(errors),
            Called::TimedOut => Err(format!(
                "{name}: the stream gave no item and did not end within 10 s; give a Fixture or a context that answers"
            )),
            Called::NoRuntime => Err(format!(
                "{name}: stream checks need a tokio runtime with time (use #[tokio::test])"
            )),
        }
    }
}

/// Whether the caller runs inside a tokio runtime with its time driver.
///
/// `Handle::try_current` needs tokio's `rt` feature, which the `testing`
/// feature does not enable (only `time`), so this probes instead: tokio looks
/// up the time driver when it builds a `Sleep` and panics when there is no
/// runtime.
fn in_tokio_runtime() -> bool {
    std::panic::catch_unwind(|| drop(tokio::time::sleep(std::time::Duration::ZERO))).is_ok()
}

/// The errors one call of `d` on `path` produced: the call's own, and for a
/// stream the error of its first item. A stream is read to its first item
/// (`Ok` or `Err`) or its end, under a 10 s timeout that covers opening it.
pub(crate) async fn call_errors<C: Resolve>(
    registry: &Registry<C>,
    ctx: &C,
    d: &Descriptor,
    args: Value,
    path: CallPath,
) -> Called {
    if d.kind == OpKind::Stream {
        if !in_tokio_runtime() {
            return Called::NoRuntime;
        }
        let call = async {
            match registry
                .call_stream_json(&d.name, args, ctx, kit_options(), None, path)
                .await
                .result
            {
                Err(err) => vec![err],
                Ok(mut items) => match items.next().await {
                    Some(Err(err)) => vec![err],
                    _ => Vec::new(),
                },
            }
        };
        match tokio::time::timeout(KIT_STREAM_TIMEOUT, call).await {
            Ok(errors) => Called::Errors(errors),
            Err(_) => Called::TimedOut,
        }
    } else {
        Called::Errors(
            registry
                .call_json(&d.name, args, ctx, kit_options(), path, false)
                .await
                .result
                .err()
                .into_iter()
                .collect(),
        )
    }
}

/// Runs each fixture on the first surface its op declares (json, mcp, cli) and
/// reports every error whose
/// code the op did not declare, and every uncoded error.
///
/// An op declared on typed only is not run; the report names it. Do not
/// combine it with [`strict_codes`]: a strict panic stops the report at the
/// first offender.
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
    let mut skipped = Vec::new();
    for fixture in fixtures {
        let Some(d) = registry.get(fixture.op) else {
            problems.push(format!(
                "fixture names `{}`, which is not in the registry",
                fixture.op
            ));
            continue;
        };
        let Some(path) = kit_path(d) else {
            skipped.push(typed_only_line("assert_error_codes", d));
            continue;
        };
        match call_errors(registry, ctx, d, fixture.args.clone(), path)
            .await
            .errors_or_problem(&d.name)
        {
            Ok(errors) => {
                for err in errors {
                    if let Some(what) = offence(registry, d, &err) {
                        problems.push(format!("{} returned {what}", d.name));
                    }
                }
            }
            Err(problem) => problems.push(problem),
        }
    }
    fail_on_noting("assert_error_codes", &problems, &skipped);
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
    if !registry.common_errors().is_empty() {
        let common = section(rendered, "## Common codes").unwrap_or_default();
        for entry in registry.common_errors() {
            if !common.contains(&format!("| {} |", entry.code)) {
                problems.push(format!(
                    "the common code `{}` is not in the manual's \"Common codes\"",
                    entry.code
                ));
            }
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
            if let Some(on) = &exit.on {
                if !declared_codes(registry, d).contains(on) {
                    problems.push(format!(
                        "{}: exit {} is declared on `{on}`, which is not one of the op's codes",
                        d.name, exit.code
                    ));
                }
            }
        }
        if d.hidden || d.kind == crate::OpKind::ListPage {
            continue;
        }
        // A command has a section under its path; an op off the command line
        // has one under its name.
        let heading = if crate::cli::on_cli(registry, index) {
            format!("### {} {}", registry.bin(), d.path.join(" "))
        } else {
            format!("### {}", d.name)
        };
        let Some(text) = section(rendered, &heading) else {
            problems.push(format!("{}: the manual has no section `{heading}`", d.name));
            continue;
        };
        for exit in &d.exits {
            let line = match &exit.on {
                Some(on) => format!("- {}: {} (on {on})", exit.code, exit.meaning),
                None => format!("- {}: {}", exit.code, exit.meaning),
            };
            if !text.contains(&line) {
                problems.push(format!(
                    "{}: declared exit {} ({}) is not in the manual",
                    d.name, exit.code, exit.meaning
                ));
            }
        }
        // Common codes are printed once, in "## Common codes", not per op.
        for code in declared_codes(registry, d) {
            let common = registry.common_errors().iter().any(|c| c.code == code);
            if !common && !text.contains(&format!("| {code} |")) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallError, CallOptions, OpParams, OpSpec};
    use futures_util::future::BoxFuture;
    use serde::Serialize;

    struct Ctx;
    impl Resolve for Ctx {}

    #[derive(OpParams)]
    struct P {}

    #[derive(Serialize)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    struct Out {}

    #[test]
    fn wrong_path_is_an_offence() {
        let mut r = Registry::<Ctx>::new();
        r.set_pillar("fix");
        r.group("tool", "Tools.");
        r.push_op(
            OpSpec::<P, Out>::new("tool op", "Runs."),
            |_c: &Ctx, _p: P, _o: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
                Box::pin(async { Ok(Out {}) })
            },
        );
        let d = r.get("tool_op").unwrap();
        for err in [
            CallError::WrongPath {
                name: "tool_op".into(),
            },
            CallError::WrongShape {
                name: "tool_op".into(),
            },
        ] {
            let err = coded_error::into_anyhow(err);
            assert!(offence(&r, d, &err).is_some(), "{err:#}");
        }
        let other = coded_error::into_anyhow(CallError::UnknownOperation { name: "x".into() });
        assert!(offence(&r, d, &other).is_none());
    }
}
