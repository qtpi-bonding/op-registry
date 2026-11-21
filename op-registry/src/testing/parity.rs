//! `assert_parity` and `assert_canonical`: every op on every path.

use super::{fail_on, fields_of};
use crate::mcp::tool_list;
use crate::shape::{root_class, RootClass};
use crate::{Audience, CliContext, Descriptor, FieldKind, OpKind, Registry, Surface};
use std::collections::BTreeSet;

/// A mounted surface: a named place (REST, a typed client) that serves a set
/// of ops to one audience.
pub struct Mount<'a> {
    pub name: &'a str,
    pub audience: Audience,
    /// The op names the mount serves.
    pub ops: &'a [&'a str],
}

/// Whether an op is on a surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    Reachable,
    Absent { reason: String },
}

/// One op on one surface (`cli`, `mcp` or a mount's name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParityLine {
    pub name: String,
    pub path: String,
    pub reach: Reach,
}

/// Reasons an op may be absent from a surface without it being a problem.
const ALLOWED: [&str; 3] = ["stream", "not declared", "not an op"];

fn allowed(reason: &str) -> bool {
    ALLOWED.contains(&reason)
}

fn reachable(name: &str, path: &str) -> ParityLine {
    ParityLine {
        name: name.to_owned(),
        path: path.to_owned(),
        reach: Reach::Reachable,
    }
}

fn absent(name: &str, path: &str, reason: &str) -> ParityLine {
    ParityLine {
        name: name.to_owned(),
        path: path.to_owned(),
        reach: Reach::Absent {
            reason: reason.to_owned(),
        },
    }
}

/// Whether `path` names a leaf command below the root.
fn cli_leaf(root: &clap::Command, path: &[String]) -> Option<bool> {
    let mut cmd = root;
    for word in path {
        cmd = cmd.find_subcommand(word)?;
    }
    Some(!cmd.get_subcommands().any(|c| c.get_name() != "help"))
}

/// Why a hidden op could not be called on MCP, if it could not: the same
/// conditions `call_tool` checks.
fn hidden_problem<C: Sync>(registry: &Registry<C>, d: &Descriptor) -> Option<&'static str> {
    if d.kind == OpKind::Stream {
        Some("hidden op is a stream")
    } else if registry.get(&d.name).is_none() {
        Some("hidden op is not found by name")
    } else {
        None
    }
}

fn audience_name(a: Audience) -> &'static str {
    match a {
        Audience::Agent => "agent",
        Audience::Owner => "owner",
        Audience::Dev => "dev",
    }
}

/// What is wrong with the output root, if it is not an object (a missing
/// schema says nothing and passes).
fn non_object_root(d: &Descriptor) -> Option<&'static str> {
    match root_class(d.output_schema.as_ref()?) {
        RootClass::NonObject(what) => Some(what),
        RootClass::Object | RootClass::Any => None,
    }
}

/// Whether the root input property `key` has a `oneOf` object branch that
/// requires `inline`. Only the root property is read, never a relation
/// fragment inside `oneOf`, `anyOf` or `allOf`.
fn takes_inline(d: &Descriptor, key: &str) -> bool {
    let pointer = format!("/properties/{}", key.replace('~', "~0").replace('/', "~1"));
    d.input_schema
        .pointer(&pointer)
        .and_then(|p| p.get("oneOf"))
        .and_then(|b| b.as_array())
        .is_some_and(|branches| {
            branches.iter().any(|b| {
                b.get("type").and_then(|t| t.as_str()) == Some("object")
                    && b.get("required")
                        .and_then(|r| r.as_array())
                        .is_some_and(|r| r.iter().any(|v| v == "inline"))
            })
        })
}

/// Every op on every surface, and the problems that are not one op on one
/// surface (extras, unknown mount names, ops declared on a surface they are not on).
fn analyse<C: CliContext>(
    registry: &Registry<C>,
    mounts: &[Mount<'_>],
) -> (Vec<ParityLine>, Vec<String>) {
    let root = crate::cli::command(registry);
    let mut lines = Vec::new();
    let mut problems = Vec::new();
    let tools: Vec<(Audience, BTreeSet<String>)> =
        [Audience::Agent, Audience::Owner, Audience::Dev]
            .into_iter()
            .map(|a| {
                (
                    a,
                    tool_list(registry, a).into_iter().map(|t| t.name).collect(),
                )
            })
            .collect();
    let tool_names = |audience: Audience| &tools.iter().find(|(a, _)| *a == audience).unwrap().1;

    for d in registry.descriptors() {
        let on_cli = cli_leaf(&root, &d.path);
        let on_mcp = tools.iter().any(|(_, names)| names.contains(&d.name));
        if d.paths.contains(Surface::Cli) {
            lines.push(match on_cli {
                Some(_) => reachable(&d.name, "cli"),
                None => absent(
                    &d.name,
                    "cli",
                    &format!("no command at `{}`", d.path.join(" ")),
                ),
            });
        } else {
            lines.push(absent(&d.name, "cli", "not declared"));
            if on_cli == Some(true) {
                problems.push(format!(
                    "op {} is on the CLI but does not declare cli",
                    d.name
                ));
            }
        }
        let wrong_audience: Vec<Audience> = tools
            .iter()
            .filter(|(a, names)| !d.audiences.contains(a) && names.contains(&d.name))
            .map(|(a, _)| *a)
            .collect();
        for a in wrong_audience {
            problems.push(format!(
                "op {} is on the MCP tools of audience {} but is not of that audience",
                d.name,
                audience_name(a)
            ));
        }
        let mcp = if !d.paths.contains(Surface::Mcp) {
            if on_mcp {
                problems.push(format!("op {} is on MCP but does not declare mcp", d.name));
            }
            absent(&d.name, "mcp", "not declared")
        } else if d.kind == OpKind::Stream {
            absent(&d.name, "mcp", "stream")
        } else if d.hidden {
            match hidden_problem(registry, d) {
                None => reachable(&d.name, "mcp"),
                Some(why) => absent(&d.name, "mcp", why),
            }
        } else if d.audiences.iter().all(|a| tool_names(*a).contains(&d.name)) {
            reachable(&d.name, "mcp")
        } else {
            absent(&d.name, "mcp", "missing from tool_list")
        };
        let mut on_surface = mcp.reach == Reach::Reachable;
        lines.push(mcp);
        let mcp_reachable = on_surface;
        for mount in mounts {
            let here = mount.ops.contains(&d.name.as_str());
            if !d.audiences.contains(&mount.audience) {
                if here {
                    problems.push(format!(
                        "op {} is in mount {} but is not of audience {}",
                        d.name,
                        mount.name,
                        audience_name(mount.audience)
                    ));
                }
                continue;
            }
            if !d.paths.contains(Surface::Json) {
                if here {
                    problems.push(format!(
                        "op {} is in mount {} but does not declare json",
                        d.name, mount.name
                    ));
                }
                lines.push(absent(&d.name, mount.name, "not declared"));
                continue;
            }
            lines.push(if here {
                on_surface = true;
                reachable(&d.name, mount.name)
            } else if d.kind == OpKind::Stream {
                // A mount is a unary transport; a stream has a reason to be absent.
                absent(&d.name, mount.name, "stream")
            } else {
                absent(&d.name, mount.name, "missing from mount")
            });
        }
        if on_surface {
            if let Some(what) = non_object_root(d) {
                problems.push(format!(
                    "{}: the output root is {what}; give it an object form",
                    d.name
                ));
            }
        }
        if mcp_reachable {
            for field in fields_of(registry, d)
                .iter()
                .filter(|f| f.kind == FieldKind::SecretFile)
            {
                if !takes_inline(d, &field.key) {
                    problems.push(format!(
                        "{}: --{} is a SecretFile on mcp; add #[op(secret_inline)] or leave mcp out of paths",
                        d.name, field.flag
                    ));
                }
            }
        }
    }

    for mount in mounts {
        for name in mount.ops.iter().filter(|n| registry.get(n).is_none()) {
            problems.push(format!(
                "mount {} lists `{name}`, which is not in the registry",
                mount.name
            ));
        }
    }
    for extra in &registry.extras {
        let name = extra.command.get_name();
        lines.push(reachable(name, "cli"));
        lines.push(absent(name, "mcp", "not an op"));
    }
    (lines, problems)
}

/// Where every op is reachable, one line per op and surface.
pub fn parity_report<C: CliContext>(
    registry: &Registry<C>,
    mounts: &[Mount<'_>],
) -> Vec<ParityLine> {
    analyse(registry, mounts).0
}

/// Checks that every op declaring `cli` is on the CLI at its path; that every
/// op declaring `mcp` is in `tool_list` (a stream or a hidden op excepted; a
/// hidden op must still be callable); that every op declaring `json` is in
/// every mount of its audience; that an op is on no surface it does not
/// declare, per audience for an op of several; that no op reachable on `mcp`
/// or a mount has a non-object output root; that an op reachable on `mcp`
/// takes every `SecretFile` inline; and that every mount name is an op.
///
/// # Panics
///
/// Panics with every problem found.
pub fn assert_parity<C: CliContext>(registry: &Registry<C>, mounts: &[Mount<'_>]) {
    let (lines, others) = analyse(registry, mounts);
    let mut problems: Vec<String> = lines
        .iter()
        .filter_map(|line| match &line.reach {
            Reach::Absent { reason } if !allowed(reason) => {
                Some(format!("{} on {}: {reason}", line.name, line.path))
            }
            _ => None,
        })
        .collect();
    problems.extend(others);
    fail_on("assert_parity", &problems);
}

/// Checks that the op names equal `canonical`, as sets.
///
/// # Panics
///
/// Panics listing the names missing from the registry and the extra ones.
pub fn assert_canonical<C: Sync>(registry: &Registry<C>, canonical: &[&str]) {
    let have: BTreeSet<&str> = registry.names().into_iter().collect();
    let want: BTreeSet<&str> = canonical.iter().copied().collect();
    let mut problems = Vec::new();
    for name in want.difference(&have) {
        problems.push(format!("`{name}` is canonical but not in the registry"));
    }
    for name in have.difference(&want) {
        problems.push(format!("`{name}` is in the registry but not canonical"));
    }
    fail_on("assert_canonical", &problems);
}
