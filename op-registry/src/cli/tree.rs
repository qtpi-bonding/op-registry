//! The clap command tree of a registry.

use super::{Fallback, ID_COMPACT, ID_PRETTY, ID_TEXT};
use crate::{CliContext, OpGlobals, OpKind, Registry, Surface};
use clap::{Arg, ArgAction, Command};

/// Names the library owns on every command.
pub(super) const LIBRARY_FLAGS: [&str; 5] = ["pretty", "compact", "text", "help", "version"];

/// Panics when an extra's command, or a subcommand of it, has a long flag that
/// a registry global or the library owns.
pub(super) fn check_extra_flags<C: CliContext>(command: &Command) {
    let globals = <C::Globals as OpGlobals>::fields();
    let mut stack = vec![command];
    while let Some(cmd) = stack.pop() {
        for flag in cmd.get_arguments().filter_map(Arg::get_long) {
            assert!(
                !LIBRARY_FLAGS.contains(&flag) && !globals.iter().any(|f| f.flag == flag),
                "extra `{}`: --{flag} is already a global flag",
                command.get_name()
            );
        }
        stack.extend(cmd.get_subcommands());
    }
}

/// A node of the tree: a group of children or a leaf op (an entry index).
enum Node {
    Group {
        /// The entry that runs when the group is named with no verb.
        default: Option<usize>,
        children: Vec<(String, Node)>,
    },
    Leaf(usize),
}

/// Whether an entry is a command of its own: its paths contain `cli`, and it is not the
/// paged half of a list pair (the pair is one command).
pub(crate) fn on_cli<C>(registry: &Registry<C>, index: usize) -> bool {
    let d = &registry.entries[index].descriptor;
    d.paths.contains(Surface::Cli) && d.kind != OpKind::ListPage
}

/// The entry a command chain selects: a leaf at exactly that path, else the
/// `group_default` of the group it names.
pub(crate) fn lookup<C>(registry: &Registry<C>, chain: &[String]) -> Option<usize> {
    if chain.is_empty() {
        return None;
    }
    let entries = || (0..registry.entries.len()).filter(|&i| on_cli(registry, i));
    entries()
        .find(|&i| registry.entries[i].descriptor.path == chain)
        .or_else(|| {
            entries().find(|&i| {
                let d = &registry.entries[i].descriptor;
                d.group_default && d.noun_path() == chain
            })
        })
}

/// The words that start a command: op and group words, extras and `help`.
pub(crate) fn top_words<C>(registry: &Registry<C>) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let paths = (0..registry.entries.len())
        .filter(|&i| on_cli(registry, i))
        .map(|i| &registry.entries[i].descriptor.path)
        .chain(registry.groups.iter().map(|g| &g.path));
    for path in paths {
        if let Some(word) = path.first() {
            if !words.contains(word) {
                words.push(word.clone());
            }
        }
    }
    words.extend(
        registry
            .extras
            .iter()
            .map(|e| e.command.get_name().to_owned()),
    );
    words.push("help".to_owned());
    words
}

/// Builds the clap command for a registry: the binary's name, about and
/// version, the globals, and one group per noun word with a leaf per op.
///
/// # Panics
///
/// Panics when a `C::Globals` flag is one the library owns (`--pretty`,
/// `--compact`, `--text`, `--help`, `--version`), when a free-text fallback
/// declares a flag that a global already has, when an extra's name is an
/// op or group word, or when an extra (merged in) has a long flag that a
/// global or the library owns.
pub fn command<C: CliContext>(registry: &Registry<C>) -> Command {
    for field in <C::Globals as OpGlobals>::fields() {
        assert!(
            !LIBRARY_FLAGS.contains(&field.flag.as_str()),
            "global `{}` (--{}) clashes with a library global",
            field.rust_name,
            field.flag
        );
    }
    if let Some(Fallback::FreeText { name, flags, .. }) = &registry.fallback {
        let globals = <C::Globals as OpGlobals>::fields();
        for flag in flags.iter() {
            let flag = flag.trim_start_matches('-');
            assert!(
                !LIBRARY_FLAGS.contains(&flag) && !globals.iter().any(|f| f.flag == flag),
                "fallback `{name}` flag --{flag} clashes with a registry global"
            );
        }
    }
    let mut root = Command::new(registry.bin())
        .about(registry.about())
        .version(registry.version())
        .disable_version_flag(true)
        .subcommand_required(true)
        .arg(
            Arg::new("version")
                .long("version")
                .action(ArgAction::Version)
                .help("Print the version."),
        )
        .arg(
            Arg::new(ID_PRETTY)
                .long("pretty")
                .global(true)
                .action(ArgAction::SetTrue)
                .conflicts_with(ID_COMPACT)
                .help("Indent the JSON output."),
        )
        .arg(
            Arg::new(ID_COMPACT)
                .long("compact")
                .global(true)
                .action(ArgAction::SetTrue)
                .conflicts_with(ID_PRETTY)
                .help("Print the JSON output on one line."),
        );
    root = <C::Globals as OpGlobals>::add_args(root);
    if let Some(fallback) = &registry.fallback {
        root = root.after_help(fallback.listing());
    }

    let mut top: Vec<(String, Node)> = Vec::new();
    for index in (0..registry.entries.len()).filter(|&i| on_cli(registry, i)) {
        let d = &registry.entries[index].descriptor;
        if d.group_default {
            let group = if registry.groups.iter().any(|g| g.path == d.path) {
                d.path.clone()
            } else {
                d.noun_path().to_vec()
            };
            if let Some(Node::Group { default, .. }) = group_node(&mut top, &group) {
                *default = Some(index);
            }
            if group == d.path {
                continue;
            }
        }
        insert(&mut top, &d.path, index);
    }
    for (word, node) in top {
        assert!(
            !registry.extras.iter().any(|e| e.command.get_name() == word),
            "extra `{word}` clashes with op word"
        );
        root = root.subcommand(to_command(registry, std::slice::from_ref(&word), node));
    }
    for extra in &registry.extras {
        check_extra_flags::<C>(&extra.command);
        root = root.subcommand(extra.command.clone());
    }
    root
}

/// The group node at `path`, created with its parents; `None` for an empty
/// path (the root).
fn group_node<'a>(top: &'a mut Vec<(String, Node)>, path: &[String]) -> Option<&'a mut Node> {
    let (word, rest) = path.split_first()?;
    let at = match top.iter().position(|(w, _)| w == word) {
        Some(at) => at,
        None => {
            top.push((
                word.clone(),
                Node::Group {
                    default: None,
                    children: Vec::new(),
                },
            ));
            top.len() - 1
        }
    };
    let node = &mut top[at].1;
    if rest.is_empty() {
        return Some(node);
    }
    match node {
        Node::Group { children, .. } => group_node(children, rest),
        Node::Leaf(_) => unreachable!("the registry admitted a word that is both leaf and group"),
    }
}

/// Inserts a leaf at `path`, creating its groups.
fn insert(top: &mut Vec<(String, Node)>, path: &[String], index: usize) {
    let (verb, noun) = path.split_last().expect("an op has a path");
    let children = if noun.is_empty() {
        top
    } else {
        match group_node(top, noun) {
            Some(Node::Group { children, .. }) => children,
            _ => unreachable!("the registry admitted a word that is both leaf and group"),
        }
    };
    assert!(
        !children.iter().any(|(w, _)| w == verb),
        "the registry admitted colliding CLI words at {path:?}"
    );
    children.push((verb.clone(), Node::Leaf(index)));
}

/// Whether every op under the node is hidden.
fn all_hidden<C>(registry: &Registry<C>, node: &Node) -> bool {
    match node {
        Node::Leaf(i) => registry.entries[*i].descriptor.hidden,
        Node::Group { children, .. } => children.iter().all(|(_, n)| all_hidden(registry, n)),
    }
}

fn to_command<C: Sync>(registry: &Registry<C>, path: &[String], node: Node) -> Command {
    let word = path.last().expect("a node has a word").clone();
    match node {
        Node::Leaf(index) => {
            let d = &registry.entries[index].descriptor;
            let mut cmd = Command::new(word).about(d.summary.clone());
            if !d.long_help.is_empty() {
                cmd = cmd.long_about(format!("{}\n\n{}", d.summary, d.long_help));
            }
            decorate(registry, index, cmd).hide(d.hidden)
        }
        Node::Group { default, children } => {
            let summary = registry
                .groups
                .iter()
                .find(|g| g.path == path)
                .map(|g| g.summary.clone())
                .unwrap_or_default();
            let hidden = children.iter().all(|(_, n)| all_hidden(registry, n));
            let mut cmd = Command::new(word).about(summary).hide(hidden);
            cmd = match default {
                Some(index) => decorate(registry, index, cmd)
                    .subcommand_required(false)
                    .args_conflicts_with_subcommands(true),
                None => cmd.subcommand_required(true),
            };
            for (child, node) in children {
                let mut child_path = path.to_vec();
                child_path.push(child);
                cmd = cmd.subcommand(to_command(registry, &child_path, node));
            }
            cmd
        }
    }
}

/// Adds an op's own flags to a command: its params, a list's paging flags, a
/// declared stream cursor and `--text`.
fn decorate<C: Sync>(registry: &Registry<C>, index: usize, cmd: Command) -> Command {
    let entry = &registry.entries[index];
    let d = &entry.descriptor;
    let mut cmd = match (&d.kind, &d.list_pair) {
        // A list pair is one command: the paged half knows every flag.
        (OpKind::ListAll, Some(page)) => {
            let page_entry = registry
                .entry(page)
                .expect("a list pair names its paged half");
            (page_entry.augment)(cmd).mut_arg("limit", |arg| {
                arg.default_value(entry.page_size.to_string())
                    .hide_default_value(true)
            })
        }
        _ => (entry.augment)(cmd),
    };
    if d.kind == OpKind::Stream && d.cursor {
        cmd = cmd.arg(
            Arg::new("cursor")
                .long("cursor")
                .value_name("CURSOR")
                .help("Resume after this cursor."),
        );
    }
    if d.text {
        cmd = cmd.arg(
            Arg::new(ID_TEXT)
                .long("text")
                .action(ArgAction::SetTrue)
                .help("Print the text form instead of JSON."),
        );
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Extra;
    use std::sync::Arc;

    #[derive(crate::OpGlobals)]
    #[allow(dead_code)]
    struct G {
        /// The endpoint.
        endpoint: Option<String>,
    }

    struct Ctx;
    impl crate::Resolve for Ctx {}
    impl CliContext for Ctx {
        type Globals = G;
        fn from_globals(_: &G) -> anyhow::Result<Self> {
            Ok(Ctx)
        }
    }

    /// `merge` copies another registry's extras without `push_extra`'s check, so
    /// `command` repeats it. The two registries share one `C::Globals`, so no
    /// public call can build the clash: the extra is placed directly.
    #[test]
    fn merged_extra_clash_panics_at_command() {
        let mut registry = Registry::<Ctx>::new();
        registry.extras.push(Extra {
            command: Command::new("serve").arg(Arg::new("e").long("endpoint")),
            run: Arc::new(|_| Box::pin(async { Ok(0) })),
        });
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| command(&registry)))
            .unwrap_err();
        let message = payload.downcast::<String>().unwrap();
        assert!(
            message.contains("extra `serve`: --endpoint is already a global flag"),
            "{message}"
        );
    }
}
