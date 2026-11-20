//! Command-line command construction and dispatch for a registry.

use crate::{derive_name, CallOptions, CallPath, Cursor, OpKind, ParseCx, Registry};
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

/// A node in the command tree built from descriptors.
enum Node {
    /// A group of subcommands keyed by word, kept in insertion order.
    Group {
        /// The operation name that first created this group.
        first_op: String,
        /// Child words with their nodes.
        children: Vec<(String, Node)>,
    },
    /// A leaf command backed by one operation.
    Leaf {
        /// The operation name.
        op: String,
        /// The clap command for the leaf.
        command: Box<Command>,
    },
}

/// Returns the operation name recorded on a node.
fn node_op(node: &Node) -> &str {
    match node {
        Node::Group { first_op, .. } => first_op,
        Node::Leaf { op, .. } => op,
    }
}

/// Inserts a leaf under the given group path, creating groups as needed.
///
/// `path` holds the group words, outermost first; `leaf_word` is the verb or
/// explicit name. The registry has already rejected colliding CLI words, so a
/// collision here is an internal invariant violation.
fn insert(
    children: &mut Vec<(String, Node)>,
    path: &[&str],
    leaf_word: &str,
    op: &str,
    command: Command,
) {
    match path.split_first() {
        None => {
            if let Some((_, existing)) = children.iter().find(|(word, _)| word == leaf_word) {
                let other = node_op(existing);
                unreachable!(
                    "registry admitted colliding CLI words {:?} for {:?} and {:?}",
                    leaf_word, other, op
                );
            }
            children.push((
                leaf_word.to_owned(),
                Node::Leaf {
                    op: op.to_owned(),
                    command: Box::new(command),
                },
            ));
        }
        Some((word, rest)) => {
            let index = match children.iter().position(|(w, _)| w == word) {
                Some(index) => index,
                None => {
                    children.push((
                        (*word).to_owned(),
                        Node::Group {
                            first_op: op.to_owned(),
                            children: Vec::new(),
                        },
                    ));
                    children.len() - 1
                }
            };
            match &mut children[index].1 {
                Node::Group {
                    children: inner, ..
                } => insert(inner, rest, leaf_word, op, command),
                Node::Leaf { op: other, .. } => unreachable!(
                    "registry admitted a word that is both leaf {:?} and group for {:?}",
                    other, op
                ),
            }
        }
    }
}

/// Converts a tree node into a clap command.
fn to_command(word: &str, node: Node) -> Command {
    match node {
        Node::Leaf { command, .. } => *command,
        Node::Group { children, .. } => {
            let mut group = Command::new(word.to_owned()).subcommand_required(true);
            for (child_word, child) in children {
                group = group.subcommand(to_command(&child_word, child));
            }
            group
        }
    }
}

/// Builds the clap command for a registry.
///
/// Each descriptor becomes a leaf named by its verb, nested in one group per
/// noun word.
pub fn command<C: Sync>(registry: &Registry<C>, name: &str, about: &str) -> Command {
    let mut root = Command::new(name.to_owned())
        .about(about.to_owned())
        .subcommand_required(true);
    let mut top: Vec<(String, Node)> = Vec::new();

    for entry in &registry.entries {
        let descriptor = &entry.descriptor;
        let leaf_word = descriptor.verb().to_owned();
        let mut leaf = Command::new(leaf_word.clone()).about(descriptor.summary.clone());
        leaf = (entry.augment)(leaf);
        if descriptor.kind == OpKind::Stream {
            leaf = leaf.arg(
                Arg::new("cursor")
                    .long("cursor")
                    .value_name("opaque")
                    .required(false),
            );
        }

        let path: Vec<&str> = descriptor.noun_path().iter().map(String::as_str).collect();
        insert(&mut top, &path, &leaf_word, &descriptor.name, leaf);
    }

    for (word, node) in top {
        root = root.subcommand(to_command(&word, node));
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
    let unknown = |chain: &[String]| {
        coded_error::into_anyhow(CliError::UnknownCommand {
            name: if chain.is_empty() {
                "<missing>".to_owned()
            } else {
                chain.join(" ")
            },
        })
    };
    let descriptor = registry
        .get(&derive_name(&chain))
        .filter(|descriptor| descriptor.path == chain)
        .ok_or_else(|| unknown(&chain))?;
    let entry = registry.entry(&descriptor.name)?;

    let mut leaf_matches = matches;
    while let Some((_, child)) = leaf_matches.subcommand() {
        leaf_matches = child;
    }
    let params = (entry.decode_matches)(leaf_matches, &mut ParseCx::default())?;
    if descriptor.kind == OpKind::Stream {
        let cursor = leaf_matches
            .get_one::<String>("cursor")
            .cloned()
            .map(Cursor::new);
        Ok(CliOutput::Stream(
            registry
                .call_stream_decoded(&descriptor.name, params, ctx, opts, cursor, CallPath::Cli)
                .await?,
        ))
    } else {
        Ok(CliOutput::Value(
            registry
                .call_decoded(&descriptor.name, params, ctx, opts, CallPath::Cli)
                .await?,
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
