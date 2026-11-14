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
/// explicit name. Panics when a word would be both a leaf and a group.
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
                match existing {
                    Node::Group { .. } => panic!(
                        "{:?} collides with {:?}: a word cannot be both a leaf and a group",
                        other, op
                    ),
                    Node::Leaf { .. } => panic!(
                        "{:?} and {:?} both define the command {:?}",
                        other, op, leaf_word
                    ),
                }
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
                Node::Leaf { op: other, .. } => panic!(
                    "{:?} collides with {:?}: a word cannot be both a leaf and a group",
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
/// A descriptor whose CLI name is `<noun words...> <verb>` becomes a leaf named
/// by its verb, nested in one group per noun word. An explicit name becomes a
/// top-level leaf.
pub fn command<C: Sync>(registry: &Registry<C>, name: &str, about: &str) -> Command {
    let mut root = Command::new(name.to_owned())
        .about(about.to_owned())
        .subcommand_required(true);
    let mut top: Vec<(String, Node)> = Vec::new();

    for descriptor in registry.descriptors() {
        let cli_name = descriptor.cli_name();
        let hooks = registry
            .cli_hooks(&descriptor.name)
            .expect("descriptor must have CLI hooks");
        let is_path = cli_name == format!("{} {}", descriptor.noun, descriptor.verb);
        let words: Vec<&str> = descriptor.noun.split_whitespace().collect();

        let leaf_word = if is_path {
            descriptor.verb.clone()
        } else {
            cli_name.clone()
        };
        let mut leaf = Command::new(leaf_word.clone()).about(descriptor.description.clone());
        leaf = (hooks.add_args)(leaf);
        if descriptor.streaming {
            leaf = leaf.arg(
                Arg::new("cursor")
                    .long("cursor")
                    .value_name("opaque")
                    .required(false),
            );
        }

        let path: &[&str] = if is_path { &words } else { &[] };
        insert(&mut top, path, &leaf_word, &descriptor.name, leaf);
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
