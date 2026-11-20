//! `assert_parity` and `assert_canonical`: every op on every path.

use super::fail_on;
use crate::mcp::tool_list;
use crate::{Audience, CliContext, Descriptor, OpKind, Paths, Registry};
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
const ALLOWED: [&str; 3] = ["stream", "typed only", "not an op"];

/// The prefix of the reason a CLI-only op is absent from MCP; the declared
/// reason follows it.
const CLI_ONLY: &str = "cli only: ";

fn allowed(reason: &str) -> bool {
    ALLOWED.contains(&reason) || reason.starts_with(CLI_ONLY)
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
    if d.paths != Paths::All {
        Some("hidden op is not on every path")
    } else if d.kind == OpKind::Stream {
        Some("hidden op is a stream")
    } else if registry.get(&d.name).is_none() {
        Some("hidden op is not found by name")
    } else {
        None
    }
}

/// Every op on every surface, and the problems that are not one op on one
/// surface (extras, unknown mount names, typed-only ops that leaked).
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
        if d.paths == Paths::TypedOnly {
            lines.push(absent(&d.name, "cli", "typed only"));
            lines.push(absent(&d.name, "mcp", "typed only"));
            if on_cli == Some(true) {
                problems.push(format!("typed-only op {} is on the CLI", d.name));
            }
            if on_mcp {
                problems.push(format!("typed-only op {} is on MCP", d.name));
            }
            for mount in mounts.iter().filter(|m| m.ops.contains(&d.name.as_str())) {
                problems.push(format!(
                    "typed-only op {} is in mount {}",
                    d.name, mount.name
                ));
            }
            continue;
        }
        if d.paths == Paths::CliOnly {
            lines.push(match on_cli {
                Some(_) => reachable(&d.name, "cli"),
                None => absent(
                    &d.name,
                    "cli",
                    &format!("no command at `{}`", d.path.join(" ")),
                ),
            });
            lines.push(absent(
                &d.name,
                "mcp",
                &format!("{CLI_ONLY}{}", d.cli_only_reason),
            ));
            if on_mcp {
                problems.push(format!("cli-only op {} is on MCP", d.name));
            }
            for mount in mounts.iter().filter(|m| m.ops.contains(&d.name.as_str())) {
                problems.push(format!("cli-only op {} is in mount {}", d.name, mount.name));
            }
            continue;
        }
        lines.push(match on_cli {
            Some(_) => reachable(&d.name, "cli"),
            None => absent(
                &d.name,
                "cli",
                &format!("no command at `{}`", d.path.join(" ")),
            ),
        });
        lines.push(if d.kind == OpKind::Stream {
            absent(&d.name, "mcp", "stream")
        } else if d.hidden {
            match hidden_problem(registry, d) {
                None => reachable(&d.name, "mcp"),
                Some(why) => absent(&d.name, "mcp", why),
            }
        } else if tool_names(d.audience).contains(&d.name) {
            reachable(&d.name, "mcp")
        } else {
            absent(&d.name, "mcp", "missing from tool_list")
        });
        for mount in mounts.iter().filter(|m| m.audience == d.audience) {
            let here = mount.ops.contains(&d.name.as_str());
            lines.push(if here {
                reachable(&d.name, mount.name)
            } else if d.kind == OpKind::Stream {
                // A mount is a unary transport; a stream has a reason to be absent.
                absent(&d.name, mount.name, "stream")
            } else {
                absent(&d.name, mount.name, "missing from mount")
            });
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

/// Checks that every op that is not `TypedOnly` is on the CLI at its path, in
/// `tool_list` (a stream or a hidden op excepted; a hidden op must still be
/// callable) and in every mount of its audience; that no `TypedOnly` op is on
/// a surface; that a `CliOnly` op is on the CLI and in no `tool_list` or mount
/// (the report names it as CLI-only with its reason); and that every mount
/// name is an op.
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
