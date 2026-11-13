//! Command-line command construction and dispatch for a registry.

use crate::{CallOptions, Cursor, Registry};
use anyhow::Result;
use clap::{Arg, ArgMatches, Command};
use futures_util::stream::BoxStream;
use serde_json::Value;

/// Errors produced when a command line does not identify an operation.
#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "op_registry.cli")]
#[non_exhaustive]
pub enum CliError {
    #[error("unknown or missing command {name}")]
    #[coded(code = "unknown_command", class = Logical)]
    UnknownCommand { name: String },
}

/// Builds the clap command for a registry.
pub fn command<C: Sync>(registry: &Registry<C>, name: &str, about: &str) -> Command {
    let mut root = Command::new(name.to_owned())
        .about(about.to_owned())
        .subcommand_required(true);
    let mut groups: Vec<(String, Command)> = Vec::new();
    let mut explicit: Vec<String> = Vec::new();

    for descriptor in registry.descriptors() {
        let cli_name = descriptor.cli_name();
        let hooks = registry
            .cli_hooks(&descriptor.name)
            .expect("descriptor must have CLI hooks");
        let mut leaf = Command::new(
            if cli_name == format!("{} {}", descriptor.noun, descriptor.verb) {
                descriptor.verb.clone()
            } else {
                cli_name
            },
        )
        .about(descriptor.description.clone());
        leaf = (hooks.add_args)(leaf);
        if descriptor.streaming {
            leaf = leaf.arg(
                Arg::new("cursor")
                    .long("cursor")
                    .value_name("opaque")
                    .required(false),
            );
        }

        if descriptor.cli_name() == format!("{} {}", descriptor.noun, descriptor.verb) {
            assert!(
                !explicit.contains(&descriptor.noun),
                "noun {:?} collides with an explicit operation name",
                descriptor.noun
            );
            if let Some((_, group)) = groups.iter_mut().find(|(noun, _)| noun == &descriptor.noun) {
                *group = group.clone().subcommand(leaf);
            } else {
                let group = Command::new(descriptor.noun.clone())
                    .subcommand_required(true)
                    .subcommand(leaf);
                groups.push((descriptor.noun.clone(), group));
            }
        } else {
            assert!(
                !groups.iter().any(|(noun, _)| noun == &descriptor.name),
                "explicit operation name {:?} collides with a noun group",
                descriptor.name
            );
            explicit.push(descriptor.name.clone());
            groups.push((descriptor.name.clone(), leaf));
        }
    }

    for (_, group) in groups {
        root = root.subcommand(group);
    }
    root
}

/// The result of dispatching a command.
pub enum CliOutput<'a> {
    Value(Value),
    Stream(BoxStream<'a, Result<Value>>),
}

fn command_chain(matches: &ArgMatches) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current = matches;
    while let Some((name, child)) = current.subcommand() {
        chain.push(name.to_owned());
        current = child;
    }
    chain
}

/// Parses and invokes the operation selected by clap matches.
pub async fn dispatch<'a, C: Sync>(
    registry: &'a Registry<C>,
    ctx: &'a C,
    matches: &ArgMatches,
    opts: CallOptions,
) -> Result<CliOutput<'a>> {
    let chain = command_chain(matches);
    let name = chain.join(" ");
    let descriptor = registry
        .descriptors()
        .into_iter()
        .find(|descriptor| descriptor.cli_name() == name)
        .ok_or_else(|| {
            coded_error::into_anyhow(CliError::UnknownCommand {
                name: if name.is_empty() {
                    "<missing>".to_owned()
                } else {
                    name.clone()
                },
            })
        })?;
    let hooks = registry
        .cli_hooks(&descriptor.name)
        .ok_or_else(|| anyhow::anyhow!("missing CLI hooks for {}", descriptor.name))?;

    let mut leaf_matches = matches;
    while let Some((_, child)) = leaf_matches.subcommand() {
        leaf_matches = child;
    }
    let args = (hooks.parse)(leaf_matches)?;
    if descriptor.streaming {
        let cursor = leaf_matches
            .get_one::<String>("cursor")
            .cloned()
            .map(Cursor);
        Ok(CliOutput::Stream(
            registry
                .call_stream(&descriptor.name, args, ctx, opts, cursor)
                .await?,
        ))
    } else {
        Ok(CliOutput::Value(
            registry.call(&descriptor.name, args, ctx, opts).await?,
        ))
    }
}

/// Renders a JSON result without a trailing newline.
pub fn render(out: &Value, pretty: bool) -> String {
    if pretty {
        serde_json::to_string_pretty(out).expect("JSON values are serializable")
    } else {
        serde_json::to_string(out).expect("JSON values are serializable")
    }
}

/// Maps a coded error to a caller-provided CLI exit code.
pub fn exit_code_for(err: &anyhow::Error, table: &coded_error::cli::ExitTable) -> u8 {
    coded_error::cli::exit_code_anyhow_with(err, table)
}

/// Formats an error with its code and complete display message.
pub fn error_line(err: &anyhow::Error) -> String {
    let facts = coded_error::facts_anyhow(err);
    format!("{}: {err:#}", facts.code)
}
