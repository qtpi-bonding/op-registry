//! Checks that the CLI and MCP surfaces agree with a registry.

use crate::{mcp::ToolSpec, Audience, OpKind, Registry};
use clap::Command;
use std::collections::BTreeSet;

/// Normalises a name for matching: `-` becomes `_`.
fn normalise(name: &str) -> String {
    name.replace('-', "_")
}

/// The operation names exposed by a frontend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Surface {
    pub names: BTreeSet<String>,
}

/// Collects operation names from a clap command.
pub fn cli_surface(cmd: &Command) -> Surface {
    let mut names = BTreeSet::new();
    collect_leaves(cmd, &mut Vec::new(), &mut names);
    Surface { names }
}

/// Walks every level of the clap tree below `cmd`, recording each leaf by its
/// path from the root joined with `_`. `help` subcommands are skipped.
fn collect_leaves(cmd: &Command, path: &mut Vec<String>, names: &mut BTreeSet<String>) {
    for subcommand in cmd.get_subcommands() {
        if subcommand.get_name() == "help" {
            continue;
        }
        path.push(subcommand.get_name().to_owned());
        if subcommand
            .get_subcommands()
            .any(|child| child.get_name() != "help")
        {
            collect_leaves(subcommand, path, names);
        } else {
            names.insert(path.join("_"));
        }
        path.pop();
    }
}

/// A complete description of frontend parity failures.
#[derive(Debug)]
pub struct ParityReport {
    pub problems: Vec<String>,
}

impl std::fmt::Display for ParityReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.problems.join("\n"))
    }
}

impl std::error::Error for ParityReport {}

/// Verifies that the CLI and MCP surfaces expose the registry consistently.
pub fn assert_parity<C: Sync>(
    registry: &Registry<C>,
    cli: &Surface,
    mcp: &[ToolSpec],
    allow_cli_only: &[&str],
    allow_mcp_only: &[&str],
    audience: Audience,
) -> Result<(), ParityReport> {
    let registry_names: BTreeSet<String> = registry
        .descriptors()
        .into_iter()
        .map(|descriptor| descriptor.name.clone())
        .collect();
    let registry_normalised: BTreeSet<String> =
        registry_names.iter().map(|name| normalise(name)).collect();
    let expected_mcp: BTreeSet<String> = registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.kind != OpKind::Stream && descriptor.audience == audience)
        .map(|descriptor| descriptor.name.clone())
        .collect();
    let mcp_names: BTreeSet<String> = mcp.iter().map(|tool| tool.name.clone()).collect();
    let allow_cli: BTreeSet<&str> = allow_cli_only.iter().copied().collect();
    let allow_mcp: BTreeSet<&str> = allow_mcp_only.iter().copied().collect();
    let mut problems = Vec::new();

    let cli_normalised: BTreeSet<String> = cli.names.iter().map(|name| normalise(name)).collect();
    for name in registry_names
        .iter()
        .filter(|name| !cli_normalised.contains(&normalise(name)))
    {
        problems.push(format!("operation {name} is missing from the CLI"));
    }

    for name in cli
        .names
        .iter()
        .filter(|name| !registry_normalised.contains(&normalise(name)))
    {
        if !allow_cli.contains(name.as_str()) {
            problems.push(format!(
                "CLI command {name} is not a registry operation and is not in allow_cli_only"
            ));
        }
    }

    for name in expected_mcp.difference(&mcp_names) {
        problems.push(format!("operation {name} is missing from MCP"));
    }

    for tool in mcp {
        let descriptor = registry.get(&tool.name);
        if let Some(descriptor) = descriptor {
            if descriptor.kind == OpKind::Stream {
                problems.push(format!(
                    "MCP tool {} exposes a streaming operation and must be absent from MCP",
                    tool.name
                ));
            } else if descriptor.audience != audience {
                problems.push(format!(
                    "MCP tool {} exposes an operation for a different audience",
                    tool.name
                ));
            } else if !expected_mcp.contains(&tool.name) && !allow_mcp.contains(tool.name.as_str())
            {
                problems.push(format!(
                    "MCP tool {} is not an expected registry operation and is not in allow_mcp_only",
                    tool.name
                ));
            }
        } else if !allow_mcp.contains(tool.name.as_str()) {
            problems.push(format!(
                "MCP tool {} is not an expected registry operation and is not in allow_mcp_only",
                tool.name
            ));
        }
    }

    for name in allow_cli_only {
        let is_extra = cli.names.contains(*name) && !registry_normalised.contains(&normalise(name));
        if !is_extra {
            problems.push(format!("allow_cli_only entry {name} matches nothing"));
        }
    }
    for name in allow_mcp_only {
        let is_extra = mcp_names.contains(*name) && !expected_mcp.contains(*name);
        if !is_extra {
            problems.push(format!("allow_mcp_only entry {name} matches nothing"));
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(ParityReport { problems })
    }
}
