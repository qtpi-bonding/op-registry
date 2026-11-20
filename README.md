# op-registry

Declare each operation once; get the CLI, the tool list, a JSON call path and
a typed call path from that one declaration. MIT OR Apache-2.0.

op-registry has no auth, identity or signing. `Caller` is an opaque label the
library passes through and never checks. Whoever hosts the registry decides who
may call what.

Two crates build on it: `op-listen` (`op-listen/`) adds a resumable streaming
listener with cursors, frames and a reconnecting follower, see
[op-listen/README.md](op-listen/README.md). `op-status` (`op-status/`) adds shared
status types and a `status get` operation, see
[op-status/README.md](op-status/README.md).

## Quick start

An op is a function over a context, a params struct and an output type. One
`ops!` block lists the ops of a binary. Doc comments on an entry are its help
text. This example is compiled and run by `op-registry/tests/quickstart.rs`.

```rust,ignore
use op_registry::cli::{self, NoHooks};
use op_registry::{Audience, CallOptions, OpGlobals, OpParams};
use serde::Serialize;

/// Process settings: the `--hub` flag of the binary.
#[derive(OpGlobals)]
pub struct Globals {
    /// Hub to talk to.
    pub hub: Option<String>,
}

/// The context every op receives. Built once per run from the globals.
pub struct Context {
    hub: Option<String>,
}

impl op_registry::Resolve for Context {}

impl op_registry::CliContext for Context {
    type Globals = Globals;
    fn from_globals(globals: &Globals) -> anyhow::Result<Self> {
        Ok(Context { hub: globals.hub.clone() })
    }
}

#[derive(OpParams)]
pub struct ListItems {
    /// Largest number of items to return.
    #[op(default = 10)]
    pub limit: u32,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Items {
    pub item_names: Vec<String>,
}

async fn item_list(
    ctx: &Context,
    params: ListItems,
    _opts: CallOptions,
) -> anyhow::Result<Items> {
    let hub = ctx.hub.as_deref().unwrap_or("local");
    let item_names = (1..=params.limit).map(|n| format!("{hub}-{n}")).collect();
    Ok(Items { item_names })
}

mod app {
    use super::*;

    op_registry::ops! {
        Ctx = Context;
        globals = Globals;
        pillar = "app";
        bin = "app"; version = "0.1.0"; about = "Example app.";
        groups = {
            "item" => "Items.",
        };
        /// List items.
        "item list" => { params: ListItems, output: Items,
                         access: reads, audience: agent, op: item_list },
    }
}

// main: `app::registry()` builds a `Registry<Context>`.
let registry = app::registry();

// 1. The CLI. `run` reads the process arguments' vector and returns the exit code.
//    `app --hub h item list --limit 2` prints {"itemNames":["h-1","h-2"]}.
let code = cli::run(&registry, std::env::args_os().collect(), &NoHooks).await;

// 2. The tool list of one audience, and a call as a tool.
let tools = op_registry::mcp::tool_list(&registry, Audience::Agent);
let value = op_registry::mcp::call_tool(
    &registry, &context, Audience::Agent, "item_list",
    serde_json::json!({"limit": 1}), CallOptions::default(),
).await?;

// 3. The typed path: no JSON, the marker type names the op.
let list = registry
    .call_typed::<app::ops::ItemList>(&context, ListItems { limit: 3 }, CallOptions::default())
    .await?;
```

`cli::run` never calls `process::exit`; `main` turns its `ExitCode` into the
process exit. `cli::run_with_io` takes an `Io` (`Io::memory` for tests).

A type used in an `ops!` block must be `pub` when the block sits in a `pub`
module; a private type there gives rustc error E0446.

## What `ops!` declares

An entry is `"noun verb" => { ... }`. The last word of the path is the verb;
words before it form nested CLI groups (`"concept get"`); one word is a bare
top-level verb (`"index"`). The op name is derived, never overridden: the words
joined with `_`, every `-` turned into `_` (`concept get` is `concept_get`). The
CLI, the tool list and the typed path all use that one name. Each op also gets a
marker type in the generated `ops` module (`ops::ConceptGet`).

Entry keys:

- Kind, one of: `op:` unary, `local:` unary and needing no connection, `stream:`
  streaming, `list:` paged list. A function for `op:`, `local:` and `stream:`
  takes `(&Ctx, Params, CallOptions)`; a stream function also takes the
  `Option<Cursor>` before the options. A `list:` function takes
  `(&Ctx, Params, PageRequest)` and returns `Page<Item>`.
- `params:`, `output:` (or `item:` for a list), `access:` (`reads`, `writes`,
  `network`, `spawns`, `destructive`, one word or a list) and `audience:`
  (`agent`, `owner` or `dev`).
- Optional: `errors: [ErrType]` (the coded error types the op may return, whose
  codes are its contract), `fields:` (a function giving span fields from the
  params), `exits: { 3 => "meaning" }` (extra exit codes, 3 to 63), `text: true`
  (the output implements `RenderText` for text output), `hidden: true`,
  `default: true` (runs when the group is named with no verb, at most one per
  group), `paths: [typed]` (reachable only in process), and, for a `stream:`,
  `cursor: true`; for a `list:`, `cap:` and `page_size:`. `unpaged: true` marks a non-list op
  with the verb `list`, so `assert_help_complete` does not warn about it.
- Header keys: `Ctx`, `globals`, `pillar`, `bin`/`version`/`about` (all three or
  none), `groups` (every group needs a summary) and `resolve_server`.

A `list:` entry makes two ops from one function: `concept_list` follows every
page and prints a bare array, and `concept_list_page` takes `--limit` and
`--cursor` and prints `{"items": [...], "nextCursor": "..."}`. On the CLI both
are the one command `concept list`: with no paging flag it follows all pages,
with `--limit` or `--cursor` it returns one. An unpaged call that passes `cap`
(default 10,000) fails with `<pillar>.<object>.too_many` and tells the caller to
page; it never truncates silently. A list entry cannot declare `text:` or
`exits:`.

Build-time checks panic naming the ops: duplicate names, a word that is both an
op and a group (unless it is the group's default), a missing group summary, two
defaults in a group, a flag clash between flattened param groups.

`Registry::merge`, `push_op`, `push_local`, `push_stream` and the list
equivalents build entries in code; `op-status` and `op-listen` use them.

## Params

`#[derive(OpParams)]` gives one params struct its clap command, its JSON
decoder and its input schema. Doc comments become flag help. Field types decide
the flag:

- Flags are long kebab-case only (`--agent-id`). The only short flag is `-h`.
  JSON keys are camelCase (`agentId`) on every JSON path, input and output.
- The subject of an op is `--id <REF>`, marked `#[op(subject = "concept")]` on a
  `Ref` field. Other objects are flags named by kind (`--collection`). There are
  no positional arguments.
- `Option<T>` is optional, `bool` is a switch, `Vec<T>` repeats. `#[op(default = N)]`,
  `#[op(keys = [...])]`, `#[op(value_enum)]` and `#[op(secret)]` adjust a field.
- `Json` takes a JSON string, `@path` or `@-` (stdin); `Input` and `SecretFile`
  take a path or `-` (stdin). On JSON paths `Json` is the value itself and
  `Input` is a base64 string. Only one field per call may read stdin; a second
  read fails with `op_registry.usage.stdin_twice` at run time.
- `Patch<T>` (`#[op(clearable)]`) keeps, sets or clears a stored value.
- Secrets (`#[op(secret)]`, `SecretFile` contents and secret globals, whether
  given on the command line or through the environment) are redacted from the
  error line and notices. A JSON decode error of a secret field says only
  `invalid value`. `SecretFile` needs the `secrets` feature.
- `#[derive(OpGlobals)]` declares process-wide flags and environment variables
  (`--hub`, `APP_HUB`). They may come before or after the verb and are not tool
  params.

Removed from v1 with no replacement: `registry!`, `Operation`, `CliHooks`, the
`name:` override, `#[op(positional)]`, `#[op(short)]`, `#[op(long)]`,
`#[op(value_delimiter)]`, and the descriptor fields `requires`, `since`,
`deprecated` and `aliases`.

## Output

- stdout carries one JSON value per call. A stream prints one JSON value per
  line. The output is pretty when stdout or stderr is a terminal and compact when
  both are piped; `--pretty` and `--compact` force it.
- The library does not rewrite output keys. Output types should say
  `#[serde(rename_all = "camelCase")]`; `testing::assert_casing` checks every
  input and output schema.
- stderr carries notices, progress and the error. When piped, each library write
  is one compact JSON line with its own key (`{"notice": ...}`,
  `{"progress": ...}`, `{"error": {"code", "class", "message"}}`). The error line
  is the last line on stderr. On a terminal, notices print as text.
- Output types may implement `RenderText` and set `text: true` to give a text
  form; `OkExit` lets a successful op choose a declared exit code.

## Exit codes

| code | meaning |
|---|---|
| 0 | ok |
| 2 | usage error (bad flag, missing value, unknown command) |
| 65 | logical error (the call was understood and refused) |
| 69 | infrastructure error |
| 70 | unknown (an error with no code) |
| 75 | transient, retry may help |

Ops declare extra exits with `exits:` in the range 3 to 63. Usage errors carry
library codes: `op_registry.usage.{missing, invalid, unknown_key, repeated_key,
one_of, requires_if, stdin_twice}`, `op_registry.cli.unknown_command`, and for
calls `op_registry.call.{unknown_operation, wrong_shape, outside_audience,
wrong_path}`. `get` and `delete` of a missing object are
`<pillar>.<object>.not_found`, never a null or a success.

## Contexts, hooks and the call pipeline

- `CliContext::from_globals` builds the context lazily, so a `local:` op and
  `--help` never connect. `Resolve` maps a `Ref` to an id for ops that take
  one; the default does nothing.
- Every call path (CLI, `mcp::call_tool`, `Registry::call`, `call_typed`) runs
  one pipeline: decode, validate, resolve refs, run. An op marked
  `paths: [typed]` is reachable only in process.
- Redaction: `Registry::call` and `mcp::call_tool` fail with a `CallFailure`.
  Its `Display`, `Debug` and `redacted_message()` have the call's secrets
  replaced by `<secret>`; `error()` and `into_inner()` give the raw error for
  downcasting and must not be printed. `mcp::tool_error` and the CLI error line
  are redacted. `call_typed` and `call_stream` return the raw error: the caller
  holds the params. `CallHook::on_error` gets the raw error; pass any text taken
  from it through `CallInfo::redact`. `RunHooks::post_run` gets an `Outcome`
  whose `message()` is redacted and whose `Debug` prints only that.
- `Registry::set_call_hook` takes a `CallHook` (`around` wraps the call,
  `on_error` sees a failure). `CallInfo` carries the op name, the path
  (`Cli`, `Mcp`, ...) and the span fields, taken before refs resolve. Ref
  resolution runs inside the hook, so `around` wraps it and `on_error` sees a
  `*.not_found`. Errors that happen before the call info exists (unknown op,
  wrong path or shape, decode or validation failure) do not reach `on_error`.
- `cli::RunHooks` (`pre_parse`, `on_usage_error`, `post_run`) wraps the runner;
  flush logs in `post_run`.
- The root can hold extra non-registry subcommands and one fallback
  (`FlagsOnly` or `FreeText`) that receives raw arguments when no command
  matches. Extras and fallbacks return a plain `u8` exit code.
  `-h`, `--help` and `--version` as the first token never reach a fallback.

## Manual

With the `manual` feature, `Registry::with_manual(manual, audience)` adds a
`manual` op that renders every op the registry holds into one document;
`manual --json` gives the descriptors, `--out` writes a file.

## Tool lists and other transports

`mcp::tool_list(&registry, audience)` lists the ops of one audience: not
hidden, not a stream, not typed-only. Annotations say whether an op only reads
or is destructive. `mcp::call_tool` runs one. The `rmcp` feature adds
`RegistryServer`, an rmcp `ServerHandler` for one audience. Other transports
(REST and so on) are mounted by the app over `Registry::call`.

## Tests the library gives you

Enable `op-registry` with `features = ["testing"]` as a dev-dependency (it
implies `manual`). The checks panic with every problem they find:

- `assert_parity(&registry, &mounts)`: every op reaches the CLI, the tool list
  and each mount, or says why not. Streams are exempt from mounts.
- `assert_canonical`, `assert_casing(&registry, Casing::Camel)`,
  `assert_help_complete`, `assert_manual_snapshot`.
- `assert_exit_table`, `assert_error_codes`, `strict_codes`,
  `assert_unknown_ref_not_found`, `assert_local_ops_offline`.
- `assert_secrets_not_echoed`, `sample_params`.

## Cargo features

| feature | adds | default |
|---|---|---|
| (core) | registry, derive, runner, output, exits, help | yes |
| `schema` | output schemas via schemars 1 | yes |
| `manual` | the manual and the `manual` op | yes |
| `secrets` | `SecretFile` (zeroize) | yes |
| `testing` | the test kit above (implies `manual`) | no, dev-dependency |
| `rmcp` | `RegistryServer` | no |

`op-registry` builds with `default-features = false`; `features = ["manual"]`
is the smallest useful build. The one lib dependency is `coded-error`.
