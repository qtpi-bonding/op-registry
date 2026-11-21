//! The generated manual: Markdown for people and agents, JSON for programs,
//! and the `manual` op that serves both.

use crate::registry::TypedUnary;
use crate::{
    cli, Access, Audience, CallError, CallPath, CliContext, Descriptor, ErrorCodeEntry, Op, OpKind,
    OpParams, OpSpec, Paths, Registry, RenderText, Resolve, Surface,
};
use anyhow::{anyhow, Result};
use coded_error::{CodeCatalog, ErrorClass};
use serde::Serialize;
use serde_json::{json, Value};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;

/// The consumer's part of the manual: front matter and guides.
#[derive(Clone, Debug, Default)]
pub struct Manual {
    /// A SKILL.md's front matter, when the manual is one.
    pub front_matter: Option<FrontMatter>,
    /// Sections in order, each a heading over its guides.
    pub sections: Vec<Section>,
}

/// The `name` and `description` a SKILL.md opens with.
#[derive(Clone, Debug)]
pub struct FrontMatter {
    pub name: String,
    pub description: String,
}

/// A manual section: a `##` heading over guides.
#[derive(Clone, Debug)]
pub struct Section {
    pub title: String,
    pub guides: Vec<Guide>,
}

/// A guide: a title, a body, and the commands it covers.
#[derive(Clone, Debug)]
pub struct Guide {
    pub title: String,
    pub body: String,
    /// Command paths, e.g. `"concept get"`.
    pub commands: Vec<String>,
}

/// The exit table of every binary: code and meaning.
pub(crate) const EXITS: [(u8, &str); 6] = [
    (crate::exit::OK, "ok"),
    (crate::exit::USAGE, "usage"),
    (crate::exit::LOGICAL, "logical"),
    (crate::exit::INFRASTRUCTURE, "infrastructure"),
    (crate::exit::UNKNOWN, "unknown"),
    (crate::exit::TRANSIENT, "transient"),
];

/// A code with its class and its command-line exit; both `None` when the
/// class is decided at run time.
struct CodeRow {
    code: String,
    class: Option<ErrorClass>,
    /// The exit the op declares `on` this code; it wins over the class.
    declared_exit: Option<u8>,
}

impl CodeRow {
    fn exit(&self) -> Option<u8> {
        if self.declared_exit.is_some() {
            return self.declared_exit;
        }
        let class = self.class?;
        if self.code.starts_with("op_registry.usage.") || self.code.starts_with("op_registry.cli.")
        {
            return Some(crate::exit::USAGE);
        }
        Some(coded_error::cli::ExitTable::SYSEXITS.code_for(class))
    }

    fn table_row(&self) -> String {
        let class = self.class.map_or("varies", |c| c.as_str());
        let exit = self.exit().map_or("varies".to_owned(), |e| e.to_string());
        format!("| {} | {class} | {exit} |", self.code)
    }

    fn json(&self) -> Value {
        json!({"code": self.code, "class": self.class.map(|c| c.as_str()), "exit": self.exit()})
    }
}

/// The codes an op lists itself: its declared errors that are not common
/// codes, plus the cap code of a list op. The common codes are printed once.
fn codes_of<C: Sync>(registry: &Registry<C>, d: &Descriptor) -> Vec<CodeRow> {
    let declared_exit = |code: &str| {
        d.exits
            .iter()
            .find(|e| e.on.as_deref() == Some(code))
            .map(|e| e.code)
    };
    let mut rows: Vec<CodeRow> = d
        .errors
        .iter()
        // A common code is printed once, unless the op declares an exit on it:
        // then the op's table shows that exit.
        .filter(|e| {
            declared_exit(&e.code).is_some()
                || !registry.common_errors.iter().any(|c| c.code == e.code)
        })
        .map(|ErrorCodeEntry { code, class }| CodeRow {
            code: code.clone(),
            class: *class,
            declared_exit: declared_exit(code),
        })
        .collect();
    if d.kind == OpKind::ListAll {
        let code = crate::list::too_many_code(registry.pillar(), &d.path);
        rows.push(CodeRow {
            declared_exit: declared_exit(&code),
            code,
            class: Some(ErrorClass::Logical),
        });
    }
    rows
}

/// The codes every op of the registry may return.
fn common_codes<C>(registry: &Registry<C>) -> Vec<CodeRow> {
    registry
        .common_errors
        .iter()
        .map(|ErrorCodeEntry { code, class }| CodeRow {
            code: code.clone(),
            class: *class,
            declared_exit: None,
        })
        .collect()
}

/// The codes the library itself returns.
fn library_codes() -> Vec<CodeRow> {
    [
        crate::UsageError::codes(),
        cli::CliError::codes(),
        CallError::codes(),
    ]
    .into_iter()
    .flatten()
    .map(|e| CodeRow {
        code: e.code.to_owned(),
        class: e.class,
        declared_exit: None,
    })
    // Two variants may share a code (`MissingField`, `MissingUnless`): list it once.
    .fold(Vec::new(), |mut rows: Vec<CodeRow>, row| {
        if !rows.iter().any(|r| r.code == row.code) {
            rows.push(row);
        }
        rows
    })
}

fn access_words(access: &Access) -> String {
    let words: Vec<&str> = [
        (access.reads, "reads"),
        (access.writes, "writes"),
        (access.network, "network"),
        (access.spawns, "spawns"),
        (access.destructive, "destructive"),
    ]
    .into_iter()
    .filter_map(|(on, word)| on.then_some(word))
    .collect();
    if words.is_empty() {
        "none".to_owned()
    } else {
        words.join(", ")
    }
}

fn code_table(rows: &[CodeRow]) -> String {
    let mut table = "| Code | Class | Exit |\n|---|---|---|".to_owned();
    for row in rows {
        table.push('\n');
        table.push_str(&row.table_row());
    }
    table
}

/// Renders the Markdown manual: front matter, the exit table, the consumer's
/// sections, then every visible command with its help, access, exits and
/// codes, and the library's own codes.
///
/// Fails when a guide names a command path the registry does not have.
pub fn render<C: CliContext>(registry: &Registry<C>, manual: &Manual) -> Result<String> {
    for guide in manual.sections.iter().flat_map(|s| &s.guides) {
        for path in &guide.commands {
            let chain: Vec<String> = path.split_whitespace().map(str::to_owned).collect();
            if cli::lookup(registry, &chain).is_none() {
                return Err(anyhow!(
                    "guide {:?} names the command {path:?}, which does not exist",
                    guide.title
                ));
            }
        }
    }

    let mut blocks: Vec<String> = Vec::new();
    if let Some(front) = &manual.front_matter {
        blocks.push(format!(
            "---\nname: {}\ndescription: {}\n---",
            front.name, front.description
        ));
    }
    blocks.push(format!("# {}", registry.bin()));
    blocks.push(registry.about().to_owned());
    blocks.push("## Exit codes".to_owned());
    let mut table = "| Exit | Meaning |\n|---|---|".to_owned();
    for (code, meaning) in EXITS {
        table.push_str(&format!("\n| {code} | {meaning} |"));
    }
    blocks.push(table);
    let common = common_codes(registry);
    if !common.is_empty() {
        blocks.push("## Common codes".to_owned());
        blocks.push("Every command may return these.".to_owned());
        blocks.push(code_table(&common));
    }
    for section in &manual.sections {
        blocks.push(format!("## {}", section.title));
        for guide in &section.guides {
            blocks.push(format!("### {}", guide.title));
            blocks.push(guide.body.trim_end().to_owned());
            if !guide.commands.is_empty() {
                let commands: Vec<String> =
                    guide.commands.iter().map(|c| format!("`{c}`")).collect();
                blocks.push(format!("Commands: {}", commands.join(", ")));
            }
        }
    }

    let mut root = cli::command(registry);
    root.build();
    let others = other_commands(registry, &root);
    if !others.is_empty() {
        blocks.push("## Other commands".to_owned());
        blocks.push(
            "These run from the command line only. They are not ops: no JSON, MCP or typed call reaches them."
                .to_owned(),
        );
        for other in &others {
            blocks.push(format!("### {}", other.heading));
            blocks.push("CLI only, not an op.".to_owned());
            blocks.push(format!("```text\n{}\n```", other.help.trim_end()));
        }
    }

    blocks.push("## Commands".to_owned());
    for index in 0..registry.entries.len() {
        let d = &registry.entries[index].descriptor;
        if !cli::on_cli(registry, index) || d.hidden {
            continue;
        }
        blocks.push(format!("### {} {}", registry.bin(), d.path.join(" ")));
        let mut command = &root;
        for word in &d.path {
            command = command
                .find_subcommand(word)
                .ok_or_else(|| anyhow!("the command tree has no `{}`", d.path.join(" ")))?;
        }
        let help = command.clone().render_long_help().to_string();
        blocks.push(format!("```text\n{}\n```", help.trim_end()));
        blocks.push(format!("Access: {}", access_words(&d.access)));
        if d.paths != Paths::ALL {
            blocks.push(paths_line(d.paths));
        }
        push_exits_and_codes(&mut blocks, registry, d);
    }

    let off_cli: Vec<&Descriptor> = registry
        .entries
        .iter()
        .map(|e| &e.descriptor)
        .filter(|d| !d.hidden && !d.paths.contains(Surface::Cli) && d.kind != OpKind::ListPage)
        .collect();
    if !off_cli.is_empty() {
        blocks.push("## Operations off the command line".to_owned());
        for d in off_cli {
            blocks.push(format!("### {}", d.name));
            blocks.push(d.summary.clone());
            if !d.long_help.is_empty() {
                blocks.push(d.long_help.clone());
            }
            blocks.push(paths_line(d.paths));
            if !d.relations.is_empty() {
                let lines: Vec<String> = d.relations.iter().map(|r| format!("- {r}")).collect();
                blocks.push(format!("Rules:\n{}", lines.join("\n")));
            }
            blocks.push(format!("Access: {}", access_words(&d.access)));
            push_exits_and_codes(&mut blocks, registry, d);
        }
    }

    blocks.push("## Library codes".to_owned());
    blocks.push(code_table(&library_codes()));
    Ok(blocks.join("\n\n") + "\n")
}

/// A command that is not an op: an extra or the root fallback.
struct OtherCommand {
    heading: String,
    name: String,
    /// `extra` or `fallback`.
    kind: &'static str,
    about: String,
    long_about: String,
    help: String,
}

/// The extras in declaration order, then the fallback. `root` is the built
/// command tree, which holds every extra.
fn other_commands<C: CliContext>(
    registry: &Registry<C>,
    root: &clap::Command,
) -> Vec<OtherCommand> {
    let mut others: Vec<OtherCommand> = registry
        .extras
        .iter()
        .map(|extra| &extra.command)
        .filter(|c| !c.is_hide_set())
        .map(|c| {
            let name = c.get_name().to_owned();
            let built = root.find_subcommand(&name).unwrap_or(c);
            let text = |s: Option<&clap::builder::StyledStr>| s.map(|s| s.to_string());
            OtherCommand {
                heading: format!("{} {name}", registry.bin()),
                about: text(c.get_about()).unwrap_or_default(),
                long_about: text(c.get_long_about()).unwrap_or_default(),
                help: built.clone().render_long_help().to_string(),
                kind: "extra",
                name,
            }
        })
        .collect();
    if let Some(fallback) = &registry.fallback {
        let help = fallback.help(registry.bin());
        others.push(OtherCommand {
            heading: fallback.name().to_owned(),
            name: fallback.name().to_owned(),
            kind: "fallback",
            about: help.lines().next().unwrap_or_default().to_owned(),
            long_about: help.clone(),
            help,
        });
    }
    others
}

fn paths_line(paths: Paths) -> String {
    let words: Vec<&str> = paths.iter().map(Surface::word).collect();
    format!("Paths: {}", words.join(", "))
}

fn push_exits_and_codes<C: CliContext>(
    blocks: &mut Vec<String>,
    registry: &Registry<C>,
    d: &Descriptor,
) {
    if !d.exits.is_empty() {
        let lines: Vec<String> = d
            .exits
            .iter()
            .map(|e| match &e.on {
                Some(on) => format!("- {}: {} (on {on})", e.code, e.meaning),
                None => format!("- {}: {}", e.code, e.meaning),
            })
            .collect();
        blocks.push(format!("Exits:\n{}", lines.join("\n")));
    }
    let codes = codes_of(registry, d);
    if !codes.is_empty() {
        blocks.push(code_table(&codes));
    }
    if !registry.common_errors.is_empty() {
        blocks.push("Also the common codes.".to_owned());
    }
}

/// Every descriptor, schema, exit and code as JSON. Output schemas are
/// present only with the `schema` feature.
pub fn render_json<C: CliContext>(registry: &Registry<C>) -> Value {
    let ops: Vec<Value> = registry
        .entries
        .iter()
        .map(|entry| {
            let d = &entry.descriptor;
            let mut op = serde_json::to_value(d).expect("a descriptor is serializable");
            let fields = op.as_object_mut().expect("a descriptor is an object");
            if d.output_schema.is_none() {
                fields.remove("outputSchema");
            }
            let codes: Vec<Value> = codes_of(registry, d).iter().map(CodeRow::json).collect();
            fields.insert("codes".to_owned(), Value::Array(codes));
            op
        })
        .collect();
    let exits: Vec<Value> = EXITS
        .iter()
        .map(|(code, meaning)| json!({"code": code, "meaning": meaning}))
        .collect();
    let mut root = cli::command(registry);
    root.build();
    let other_commands: Vec<Value> = other_commands(registry, &root)
        .iter()
        .map(|o| {
            json!({
                "name": o.name,
                "kind": o.kind,
                "about": o.about,
                "longAbout": o.long_about,
                "help": o.help.trim_end(),
                "cliOnly": true,
            })
        })
        .collect();
    json!({
        "bin": registry.bin(),
        "version": registry.version(),
        "exits": exits,
        "commonCodes": common_codes(registry).iter().map(CodeRow::json).collect::<Vec<_>>(),
        "otherCommands": other_commands,
        "ops": ops,
    })
}

/// The parameters of the `manual` op.
#[derive(Clone, Debug, Default, OpParams)]
pub struct ManualParams {
    /// Write the Markdown manual to this file instead of returning it. Only
    /// the command line may: elsewhere it is a usage error.
    pub out: Option<PathBuf>,
    /// Return every descriptor, schema, exit and code as JSON.
    pub json: bool,
}

/// What the `manual` op returns.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum ManualOutput {
    /// The manual, when neither `--out` nor `--json` was given.
    Markdown { markdown: String },
    /// Where the manual was written.
    Written { written: PathBuf },
    /// The JSON form.
    Json(Value),
}

/// All three variants are objects at run time, so the schema says so; the
/// derived one would leave the `Json` variant without a type.
#[cfg(feature = "schema")]
impl schemars::JsonSchema for ManualOutput {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ManualOutput".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "title": "ManualOutput",
            "description": "What the `manual` op returns.",
            "type": "object"
        })
    }
}

impl RenderText for ManualOutput {
    fn render_text(&self) -> String {
        match self {
            ManualOutput::Markdown { markdown } => markdown.clone(),
            other => serde_json::to_string(other).expect("the manual output is serializable"),
        }
    }
}

/// The typed handle on the `manual` op, for `call_typed`.
pub struct ManualOp<C>(PhantomData<fn() -> C>);

impl<C> Op for ManualOp<C> {
    type Ctx = C;
    type Params = ManualParams;
    type Output = ManualOutput;
    const NAME: &'static str = "manual";
}

impl<C: CliContext + Resolve + 'static> Registry<C> {
    /// Adds the `manual` op: it renders `manual` together with every op the
    /// registry holds when it is called, so ops added later are included. With
    /// `--json` it wins over `--out`.
    ///
    /// # Panics
    ///
    /// Panics on the build-time checks of [`Registry::push_op`].
    pub fn with_manual(&mut self, manual: Manual, audience: Audience) {
        let mut spec =
            OpSpec::<ManualParams, ManualOutput>::new("manual", "Show this tool's manual.");
        spec.access = Access::from_words(&["reads", "writes"]);
        spec.audience = audience;
        spec.render_text = Some(|out| out.render_text());
        let manual = Arc::new(manual);
        let run: TypedUnary<C, ManualParams, ManualOutput> =
            Arc::new(move |registry, _ctx, params, opts| {
                let on_cli = opts.path == Some(CallPath::Cli);
                let result = run_manual(registry, &manual, params, on_cli);
                Box::pin(async move { result })
            });
        let entry = Self::unary_entry(&spec, OpKind::Local, run);
        self.push_entry(entry);
    }
}

fn run_manual<C: CliContext>(
    registry: &Registry<C>,
    manual: &Manual,
    params: ManualParams,
    on_cli: bool,
) -> Result<ManualOutput> {
    if params.json {
        return Ok(ManualOutput::Json(render_json(registry)));
    }
    // An MCP tool or a mount must not overwrite a file of its host.
    if params.out.is_some() && !on_cli {
        return Err(crate::__private::usage(crate::UsageError::InvalidField {
            name: "out".to_owned(),
            reason: "only the command line may write the manual to a file".to_owned(),
        }));
    }
    let markdown = render(registry, manual)?;
    match params.out {
        Some(path) => {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, markdown)?;
            Ok(ManualOutput::Written { written: path })
        }
        None => Ok(ManualOutput::Markdown { markdown }),
    }
}
