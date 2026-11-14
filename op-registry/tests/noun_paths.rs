#![allow(clippy::err_expect, clippy::ok_expect)]
//! Noun paths (`noun: "agent dispatch-target"`) and the `destructive` access.

use op_registry::cli::{self, CliOutput};
use op_registry::mcp::{self, ToolSpec};
use op_registry::parity::{assert_parity, cli_surface};
use op_registry::{registry, Access, Audience, CallOptions, Descriptor, OpParams, Registry};
use serde_json::{json, Value};

struct Ctx;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, OpParams)]
struct Empty {
    #[allow(dead_code)]
    value: Option<i32>,
}

#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
struct Done {
    which: String,
}

async fn get(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    Ok(Done {
        which: "get".into(),
    })
}
async fn list(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    Ok(Done {
        which: "list".into(),
    })
}
async fn set(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    Ok(Done {
        which: "set".into(),
    })
}
async fn deep(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    Ok(Done {
        which: "deep".into(),
    })
}
async fn wipe(_: &Ctx, _: Empty) -> anyhow::Result<Done> {
    Ok(Done {
        which: "wipe".into(),
    })
}

fn single_word() -> Registry<Ctx> {
    registry! {
        Ctx = Ctx;
        entries: [
            { noun: "item", verb: "get", description: "gets", access: reads,
              audience: Agent, params: Empty, output: Done, op: get, },
            { noun: "item", verb: "list", description: "lists", access: reads,
              audience: Agent, params: Empty, output: Done, op: list, },
            { name: "ping", noun: "system", verb: "ping", description: "pings",
              access: none, audience: Agent, params: Empty, output: Done, op: get, },
        ]
    }
}

fn pathed() -> Registry<Ctx> {
    registry! {
        Ctx = Ctx;
        entries: [
            { noun: "agent", verb: "get", description: "gets", access: reads,
              audience: Agent, params: Empty, output: Done, op: get, },
            { noun: "agent", verb: "list", description: "lists", access: reads,
              audience: Agent, params: Empty, output: Done, op: list, },
            { noun: "agent dispatch-target", verb: "set", description: "sets",
              access: writes, audience: Agent, params: Empty, output: Done, op: set, },
            { noun: "agent dispatch-target deep", verb: "go", description: "goes",
              access: writes, audience: Agent, params: Empty, output: Done, op: deep, },
        ]
    }
}

fn names_of(command: &clap::Command) -> Vec<String> {
    command
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .map(|c| c.get_name().to_owned())
        .collect()
}

fn child<'a>(command: &'a clap::Command, name: &str) -> &'a clap::Command {
    command
        .get_subcommands()
        .find(|c| c.get_name() == name)
        .unwrap_or_else(|| panic!("no subcommand {name}"))
}

fn panic_text(build: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let payload = std::panic::catch_unwind(build).expect_err("construction must panic");
    if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else {
        String::new()
    }
}

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_owned(),
        description: String::new(),
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
    }
}

#[test]
fn single_word_nouns_keep_their_descriptor_json_cli_tree_and_mcp_names() {
    let registry = single_word();
    let descriptors = registry.descriptors();
    let value = serde_json::to_value(descriptors[0]).unwrap();
    assert_eq!(value["name"], "item_get");
    assert_eq!(value["noun"], "item");
    assert_eq!(value["verb"], "get");
    assert_eq!(
        value["access"],
        json!({"reads": true, "writes": false, "network": false,
               "spawns": false, "destructive": false})
    );
    assert_eq!(descriptors[0].cli_name(), "item get");
    assert_eq!(descriptors[2].cli_name(), "ping");

    let command = cli::command(&registry, "app", "toy");
    assert_eq!(names_of(&command), ["item", "ping"]);
    assert_eq!(names_of(child(&command, "item")), ["get", "list"]);
    assert!(names_of(child(&command, "ping")).is_empty());

    let tools = mcp::tool_list(&registry, Audience::Agent);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["item_get", "item_list", "ping"]);
}

#[test]
fn noun_path_derives_the_underscore_name_and_keeps_the_cli_words() {
    let descriptor = Descriptor::new("agent dispatch-target", "set");
    assert_eq!(descriptor.name, "agent_dispatch_target_set");
    assert_eq!(descriptor.cli_name(), "agent dispatch-target set");
    assert_eq!(descriptor.noun, "agent dispatch-target");

    // A hyphen in the verb becomes `_` in the name; the CLI words are as written.
    assert_eq!(
        Descriptor::new("action", "set-parent").name,
        "action_set_parent"
    );
    assert_eq!(
        Descriptor::new("action", "set-parent").cli_name(),
        "action set-parent"
    );
    assert_eq!(Descriptor::new("item", "get").name, "item_get");
    assert_eq!(Descriptor::new("item", "get").cli_name(), "item get");
    // A single hyphenated word is not a path: its name is as before.
    assert_eq!(Descriptor::new("foo-bar", "get").name, "foo-bar_get");
    assert_eq!(Descriptor::new("foo-bar", "get").cli_name(), "foo-bar get");

    let registry = pathed();
    let tools = mcp::tool_list(&registry, Audience::Agent);
    assert!(tools.iter().any(|t| t.name == "agent_dispatch_target_set"));
    assert!(tools
        .iter()
        .any(|t| t.name == "agent_dispatch_target_deep_go"));
}

#[test]
fn hyphenated_verb_cli_and_mcp_names_agree_and_parity_holds() {
    let registry: Registry<Ctx> = registry! {
        Ctx = Ctx;
        entries: [
            { noun: "action", verb: "set-parent", description: "sets parent",
              access: writes, audience: Agent, params: Empty, output: Done, op: set, },
        ]
    };
    let command = cli::command(&registry, "app", "toy");
    assert_eq!(names_of(&command), ["action"]);
    assert_eq!(names_of(child(&command, "action")), ["set-parent"]);

    let tools = mcp::tool_list(&registry, Audience::Agent);
    assert!(tools.iter().any(|t| t.name == "action_set_parent"));
    assert!(!tools.iter().any(|t| t.name == "action_set-parent"));

    let cli = cli_surface(&command);
    assert_parity(&registry, &cli, &tools, &[], &[], Audience::Agent)
        .unwrap_or_else(|report| panic!("parity must hold:\n{report}"));
}

#[test]
fn verb_hyphen_normalisation_does_not_touch_a_single_word_noun() {
    let descriptor = Descriptor::new("agent-x", "set-y");
    assert_eq!(descriptor.name, "agent-x_set_y");
    assert_eq!(descriptor.cli_name(), "agent-x set-y");
}

#[test]
fn noun_path_builds_nested_groups_sharing_the_first_word_with_a_single_word_noun() {
    let command = cli::command(&pathed(), "app", "toy");
    assert_eq!(names_of(&command), ["agent"], "one agent group");
    let agent = child(&command, "agent");
    let mut inside = names_of(agent);
    inside.sort();
    assert_eq!(inside, ["dispatch-target", "get", "list"]);
    let target = child(agent, "dispatch-target");
    let mut inside = names_of(target);
    inside.sort();
    assert_eq!(inside, ["deep", "set"]);
    assert_eq!(names_of(child(target, "deep")), ["go"]);
    command.clone().debug_assert();
}

#[tokio::test]
async fn noun_path_three_words_dispatches_by_the_full_chain() {
    let registry = pathed();
    for (argv, which) in [
        (vec!["agent", "get"], "get"),
        (vec!["agent", "dispatch-target", "set"], "set"),
        (vec!["agent", "dispatch-target", "deep", "go"], "deep"),
    ] {
        let mut full = vec!["app"];
        full.extend(argv.iter().copied());
        let matches = cli::command(&registry, "app", "toy")
            .try_get_matches_from(full)
            .unwrap();
        let out = cli::dispatch(&registry, &Ctx, &matches, CallOptions::default())
            .await
            .unwrap();
        let CliOutput::Value(value) = out else {
            panic!("expected a value");
        };
        assert_eq!(value["which"], which, "{argv:?}");
    }
}

#[test]
fn explicit_name_still_wins_over_the_derived_path_name() {
    let registry: Registry<Ctx> = registry! {
        Ctx = Ctx;
        entries: [
            { name: "retarget", noun: "agent dispatch-target", verb: "set",
              description: "sets", access: writes, audience: Agent,
              params: Empty, output: Done, op: set, },
        ]
    };
    let descriptor = registry.descriptors()[0];
    assert_eq!(descriptor.name, "retarget");
    assert_eq!(descriptor.cli_name(), "retarget");
    let command = cli::command(&registry, "app", "toy");
    assert_eq!(names_of(&command), ["retarget"]);
    let tools = mcp::tool_list(&registry, Audience::Agent);
    assert_eq!(tools[0].name, "retarget");
}

#[test]
fn a_word_that_is_both_leaf_and_group_panics_naming_both() {
    // The leaf `agent dispatch-target` first, then the group of that word.
    let text = panic_text(|| {
        let registry: Registry<Ctx> = registry! {
            Ctx = Ctx;
            entries: [
                { noun: "agent", verb: "dispatch-target", description: "leaf",
                  access: reads, audience: Agent, params: Empty, output: Done, op: get, },
                { noun: "agent dispatch-target", verb: "set", description: "group",
                  access: writes, audience: Agent, params: Empty, output: Done, op: set, },
            ]
        };
        cli::command(&registry, "app", "toy");
    });
    assert!(text.contains("\"agent_dispatch_target\""), "{text}");
    assert!(text.contains("\"agent_dispatch_target_set\""), "{text}");

    // The other order.
    let text = panic_text(|| {
        let registry: Registry<Ctx> = registry! {
            Ctx = Ctx;
            entries: [
                { noun: "agent dispatch-target", verb: "set", description: "group",
                  access: writes, audience: Agent, params: Empty, output: Done, op: set, },
                { noun: "agent", verb: "dispatch-target", description: "leaf",
                  access: reads, audience: Agent, params: Empty, output: Done, op: get, },
            ]
        };
        cli::command(&registry, "app", "toy");
    });
    assert!(text.contains("\"agent_dispatch_target\""), "{text}");
    assert!(text.contains("\"agent_dispatch_target_set\""), "{text}");
}

#[test]
fn cli_surface_walks_all_levels() {
    let command = cli::command(&pathed(), "app", "toy");
    let surface = cli_surface(&command);
    let names: Vec<&str> = surface.names.iter().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "agent_dispatch-target_deep_go",
            "agent_dispatch-target_set",
            "agent_get",
            "agent_list",
        ]
    );
    // Existing shapes are unchanged.
    let single = cli_surface(&cli::command(&single_word(), "app", "toy"));
    let names: Vec<&str> = single.names.iter().map(String::as_str).collect();
    assert_eq!(names, ["item_get", "item_list", "ping"]);
}

#[test]
fn parity_matches_hyphenated_path_words_to_underscored_registry_names() {
    let registry = pathed();
    let cli = cli_surface(&cli::command(&registry, "app", "toy"));
    let tools = mcp::tool_list(&registry, Audience::Agent);
    assert_parity(&registry, &cli, &tools, &[], &[], Audience::Agent)
        .unwrap_or_else(|report| panic!("parity must hold:\n{report}"));
}

#[test]
fn parity_allow_lists_stay_exact_and_stale_entries_still_fail() {
    let registry: Registry<Ctx> = registry! {
        Ctx = Ctx;
        entries: [
            { noun: "agent dispatch-target", verb: "set", description: "sets",
              access: writes, audience: Agent, params: Empty, output: Done, op: set, },
        ]
    };
    let cli_names = cli::command(&registry, "app", "toy");
    let mut cli = cli_surface(&cli_names);
    // A CLI-only command written with a hyphen, as the allow-list spells it.
    cli.names.insert("legacy-thing".to_owned());
    let tools = vec![spec("agent_dispatch_target_set")];

    assert_parity(
        &registry,
        &cli,
        &tools,
        &["legacy-thing"],
        &[],
        Audience::Agent,
    )
    .unwrap_or_else(|report| panic!("exact entry must pass:\n{report}"));

    // The underscored spelling is not the same string: not matched, and stale.
    let report = assert_parity(
        &registry,
        &cli,
        &tools,
        &["legacy_thing"],
        &[],
        Audience::Agent,
    )
    .err()
    .expect("inexact allow-list entry must fail")
    .to_string();
    assert!(report.contains("legacy-thing"), "{report}");
    assert!(
        report.contains("allow_cli_only entry legacy_thing matches nothing"),
        "{report}"
    );

    // A stale entry naming a registry operation also fails.
    let report = assert_parity(
        &registry,
        &cli,
        &tools,
        &["legacy-thing", "agent_dispatch_target_set"],
        &[],
        Audience::Agent,
    )
    .err()
    .expect("stale entry must fail")
    .to_string();
    assert!(
        report.contains("allow_cli_only entry agent_dispatch_target_set matches nothing"),
        "{report}"
    );
}

#[test]
fn macro_access_destructive_sets_writes_and_destructive() {
    let registry: Registry<Ctx> = registry! {
        Ctx = Ctx;
        entries: [
            { noun: "item", verb: "wipe", description: "wipes", access: destructive,
              audience: Agent, params: Empty, output: Done, op: wipe, },
        ]
    };
    let access = registry.descriptors()[0].access;
    let mut expected = Access::default();
    expected.writes = true;
    expected.destructive = true;
    assert_eq!(access, expected);
    assert_eq!(Access::destructive(), expected);
    assert!(!access.reads);
}

#[test]
fn destructive_operations_are_still_in_the_tool_list_and_the_descriptor_says_so() {
    let registry: Registry<Ctx> = registry! {
        Ctx = Ctx;
        entries: [
            { noun: "item", verb: "wipe", description: "wipes", access: destructive,
              audience: Agent, params: Empty, output: Done, op: wipe, },
        ]
    };
    let tools = mcp::tool_list(&registry, Audience::Agent);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "item_wipe");
    let value: Value = serde_json::to_value(registry.descriptors()[0]).unwrap();
    assert_eq!(value["access"]["destructive"], true);
    assert_eq!(value["access"]["writes"], true);
}
