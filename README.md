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

## Naming

Before naming flags, JSON keys or error areas, read [docs/glossary.md](docs/glossary.md):
the shared terms and what each means. It is a reference, not a rule; a product's own
terms go in that product's repo.

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
  (`agent`, `owner` or `dev`, one word or a list: `audience: [agent, owner]`).
- Optional: `errors: [ErrType]` (the coded error types the op may return, whose
  codes are its contract), `fields:` (a function giving span fields from the
  params), `exits: { 3 => "meaning" }` (extra exit codes, 3 to 63; `{ 3 => "timed out" on
  "pillar.object.code" }` makes it an error exit for that code), `text: true`
  (the output implements `RenderText` for text output), `hidden: true`,
  `default: true` (runs when the group is named with no verb, at most one per
  group), `paths: [..]` (see "Paths"), `globals: [gait_dir]` (globals the op
  needs on the command line; see "Params"), and, for a
  `stream:`, `cursor: true`; for a `list:`, `cap:`, `page_size:` and
  `follow_page:` (the page size of the follow-all half). `unpaged: true` marks a
  non-list op with the verb `list`, so `assert_help_complete` does not warn about it.
  `#[cfg(..)]` on an entry (and doc comments) is allowed: an entry whose cfg is
  off exists on no surface.
- Header keys: `Ctx`, `globals`, `pillar`, `bin`/`version`/`about` (all three or
  none), `groups` (every group needs a summary), `resolve_server = ["kind", ..]`
  (kinds the server resolves) and `errors = [ErrType, ..]` (common error types
  every op may return; the manual prints their codes once).

A `list:` entry makes two ops from one function: `concept_list` follows every
page and prints `{"items": [...]}`, and `concept_list_page` takes `--limit` and
`--cursor` and prints `{"items": [...], "nextCursor": "..."}`. On the CLI both
are the one command `concept list`: with no paging flag it follows all pages,
with `--limit` or `--cursor` it returns one (a missing `--limit` takes the
default page size, and only with `--cursor`). A list entry may be one word
(`graph-search`). An unpaged call that passes `cap`
(default 10,000) fails with `<pillar>.<object>.too_many` and tells the caller to
page; it never truncates silently. A repeated cursor fails with
`op_registry.list.cursor_repeated`. A list entry may declare `text: true`
(`--text` prints one item per line, and the page half adds `next cursor: ..`) and
error exits, but not ok exits.

## Paths

`paths: [..]` is a literal subset of the surfaces `cli`, `typed` (in-process
calls), `mcp` and `json` (`Registry::call`, `call_stream` and mounts). The
default is every surface. `paths: [cli]` is the command line only; write
`[cli, typed]` for an op that is also called in process. A call on an undeclared
surface fails with the coded `op_registry.call.wrong_path`, `call_typed` and
`call_stream_typed` included. Both halves of a list pair share the entry's
paths. A stream is never an MCP tool. The manual adds a "Paths:" line for an op
not on every surface and lists ops off the command line in their own section.
`assert_parity` checks the declaration exactly. The library has no auth concept
and the subset is not a permission.

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
- Relations (`one_of`, `one_of_optional`, `at_least_one`, `requires_if`,
  `conflicts_with`, `required_unless`) work on members of any shape that can be
  absent: `Option` (including `Ref`, `Input`, `SecretFile`), tri-state, switch,
  `Vec`, map and `Patch`. Presence is what the CLI sees: a `false` switch, an
  empty `Vec` or map and a JSON `null` are absent on every surface. Targets of
  `requires_if`, `conflicts_with` and `required_unless` may be dotted paths into
  a `#[op(flatten)]` group (`"common.dry_run"`; the leaf type must implement
  `Presence`, else E0277). `#[op(required)]` also works on `Vec<T>` (non-empty)
  and `Option<T>`. A struct-level `#[op(validate = check)]` runs a
  `fn(&Params) -> Result<(), UsageError>` after the relations (an op returns
  `op_registry::usage(err)` for the same hook behaviour). The help, the
  manual and the schema show each rule; a relation line names every enforced
  spelling ("--x or --no-x", "--x or --clear-x"). Relation schemas match
  `validate` exactly (a flattened group keeps its relations).
- The subject of an op is `--id <REF>`, marked `#[op(subject = "concept")]` on a
  `Ref` field. Other objects are flags named by kind (`--collection`). There are
  no positional arguments. `#[op(scope = ["blueprint", "project.root"])]` on the
  subject names the field values its resolver sees: a field of the same struct,
  or a dotted path through a flattened group. A `PathBuf` is a scope value (a
  non-UTF-8 path fails with `op_registry.usage.invalid`). Keys in `Scope` are
  the leaf field's camelCase name (`root`, `gcodeStore`). `scope` on a
  `#[op(flatten)]` field gives the subject inside that group its scope. A subject
  may be an `Option<Ref>` (absent: the op gets `None` and nothing resolves).
  `#[op(subject = "agent", absent_ok)]` marks an id that may not exist yet (a
  create, an ensure, an idempotent probe): it is never resolved, the op gets
  `Resolved::Id(raw)` and the kit skips its not_found test; `id_only` may be
  added. `Resolve::check_id(kind, raw)` runs for every `id_only` or `absent_ok`
  slot before the op and may refuse a malformed id with a usage error. Non-subject
  refs (`#[op(kind = "blueprint", scope = [..])]`, `id_only`, also `Vec<Ref>`
  and `Patch<Ref>`) resolve first: a non-subject's scope may not name a ref,
  including through a flatten group; it is not checked. An absent optional scope
  member is left out of the `Scope`.
- `Option<T>` is optional, `bool` is a switch, `Vec<T>` repeats. `#[op(default = N)]`,
  `#[op(keys = [...])]`, `#[op(value_enum)]` and `#[op(secret)]` adjust a field.
  A literal default shows in help (`[default: 50]`); `hide_default` hides a
  sentinel. `#[op(value_enum, ignore_case)]` matches variants ignoring case.
  `#[op(from_str)]` decodes any `T: FromStr` (`SocketAddr`) from a string on the
  CLI and JSON; `#[op(sample = "127.0.0.1:7000")]` gives its schema an example
  the kit uses. `#[op(allow_hyphen)]` (a field or the struct) lets a value start
  with `-`. `#[op(env = "X")]` on a plain or optional scalar or a switch reads
  the variable on the CLI only, when the flag is absent (an empty variable is
  absent); typed, JSON and MCP callers pass the value. Help shows `[env: X]`.
- `Json` takes a JSON string, `@path` or `@-` (stdin); `Input` and `SecretFile`
  take a path or `-` (stdin). On JSON paths `Json` is the value itself and
  `Input` is a base64 string. Only one field per call may read stdin; a second
  read fails with `op_registry.usage.stdin_twice` at run time.
- `Patch<T>` (`#[op(clearable)]`) keeps, sets or clears a stored value.
- Secrets (`#[op(secret)]`, `SecretFile` contents and secret globals, whether
  given on the command line or through the environment) are redacted from the
  error line and notices. A JSON decode error of a secret field says only
  `invalid value`. `SecretFile` needs the `secrets` feature.
  - One redactor is shared by the whole call: `CallOptions::redact(bytes)` adds a
    value (and its trimmed form and each trimmed line) so it is removed from the
    CLI error line, stream item errors, notices, `CallFailure`, `tool_error` and
    every `opts` clone, including a nested op call. Register a secret before you
    print anything that may contain it.
  - `#[op(secret)]` on a `PathBuf` marks a key file the op reads itself; the op
    reads it and calls `opts.redact(&contents)`.
  - A secret-reading op is not a list: a list page function gets no
    `CallOptions`.
  - `#[op(prompt)]` or `#[op(prompt = "API key")]` on a required `SecretFile`:
    when the flag is left out and stdin and stderr are terminals, the CLI asks
    for one line with echo off (`--key-file -` still reads stdin). With no
    terminal, or without the `prompt` feature, it is `op_registry.usage.missing`.
    `CallOptions::prompt_secret(label)` asks the same way from inside an op and
    returns `None` on every other surface, on a CLI run with no terminal, on an
    empty answer, and always without the `prompt` feature. A `-` read from a
    terminal prints a hint first. Tests drive prompts with
    `Io::memory(..).with_prompter(..)` and `with_stdin_tty(true)`.
  - A `SecretFile` on an op reachable on `mcp` needs `#[op(secret_inline)]`, or
    the op leaves `mcp` out of its paths; `assert_parity` checks it.
- `#[derive(OpGlobals)]` declares process-wide flags and environment variables
  (`--hub`, `APP_HUB`). They may come before or after the verb and are not tool
  params. `--help` and `--version` succeed even when a required global is
  missing. An op that needs a global names it with `globals: [hub]`; a missing
  one is a usage error (exit 2), met by the flag or its environment variable.

Removed from v1 with no replacement: `registry!`, `Operation`, `CliHooks`, the
`name:` override, `#[op(positional)]`, `#[op(short)]`, `#[op(long)]`,
`#[op(value_delimiter)]`, and the descriptor fields `requires`, `since`,
`deprecated` and `aliases`. `conflicts_with` and `required_unless` are back, and
a param `env` is allowed (CLI only).

## Output

- stdout carries one JSON value per call. A stream prints one JSON value per
  line. The output is pretty when stdout or stderr is a terminal and compact when
  both are piped; `--pretty` and `--compact` force it.
- An output that is a JSON array is returned as `{"items": [...]}` on every JSON
  surface (CLI, `call`, `call_tool`, the schema and the manual); the decision is
  by value, so a `Value` output that is an array is wrapped too. The typed path
  keeps the `Vec<T>`, and stream items are not wrapped. An `Option<Vec<T>>`
  output is not wrapped; the kit reports its root as not an object. Give an
  either-or output an object root (an internally tagged enum has one); on MCP only
  an object root has an `outputSchema`, and any other value is returned as text.
  `Vec<T>` implements `RenderText` when `T` does.
- Keys print in declaration order (`serde_json` `preserve_order`).
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

Ops declare extra exits with `exits:` in the range 3 to 63; one with
`on "<code>"` is an error exit: an error with that code exits with the number
(the error line is still written, and `post_run` sees it). A follow that ends on
a stop (`op_listen.stopped`) exits 65. Usage errors carry
library codes: `op_registry.usage.{missing, invalid, unknown_key, repeated_key,
one_of, requires_if, stdin_twice}`, `op_registry.cli.unknown_command`, and for
calls `op_registry.call.{unknown_operation, wrong_shape, outside_audience,
wrong_path}`; also `op_registry.usage.{conflicts, at_least_one}` and
`op_registry.list.cursor_repeated`. `get` and `delete` of a missing object are
`<pillar>.<object>.not_found`, never a null or a success.

## Contexts, hooks and the call pipeline

- `CliContext::from_globals` builds the context lazily, so a `local:` op and
  `--help` never connect. `Resolve` maps a `Ref` to an id for ops that take
  one; the default does nothing.
- Every call path (CLI, `mcp::call_tool`, `Registry::call`, `call_typed`) runs
  one pipeline: decode, validate, resolve refs, run. An op marked
  `paths: [typed]` is reachable only in process, and so on for every subset
  (see "Paths"); any undeclared surface fails with the coded
  `op_registry.call.wrong_path`.
- Redaction: `Registry::call`, `mcp::call_tool`, `call_typed`, `call_stream` and
  `call_stream_typed` fail with a `CallFailure`.
  Its `Display`, `Debug` and `redacted_message()` have the call's secrets
  replaced by `<secret>`; `error()` and `into_inner()` give the raw error for
  downcasting and must not be printed. `mcp::tool_error` and the CLI error line
  are redacted. On a typed path write `.map_err(CallFailure::into_coded)?`, not a
  bare `?`: `into_coded` keeps the code and class with a redacted message. `CallHook::on_error` gets the raw error; pass any text taken
  from it through `CallInfo::redact`. `RunHooks::post_run` gets an `Outcome`
  whose `message()` is redacted and whose `Debug` prints only that.
- `Registry::set_error_mapper(fn(anyhow::Error) -> anyhow::Error)` rewrites the
  error of resolution or the op (and each stream item error) once, before
  `on_error`, strict codes, the exit and every surface's output. Usage errors
  are not mapped.
- `Registry::set_call_hook` takes a `CallHook` (`around` wraps the call,
  `on_error` sees a failure). `CallInfo` carries the op name, the path
  (`Cli`, `Mcp`, ...) and the span fields, taken before refs resolve. Ref
  resolution runs inside the hook, so `around` wraps it and `on_error` sees a
  `*.not_found`. Errors that happen before the call info exists (unknown op,
  wrong path or shape, decode or validation failure) do not reach `on_error`.
- `cli::RunHooks` (`pre_parse`, `on_selected`, `on_usage_error`, `post_run`)
  wraps the runner; flush logs in `post_run`. `on_usage_error` fires for every
  exit-2 failure, including a usage error an op returns.
- The root can hold extra non-registry subcommands and one fallback
  (`FlagsOnly` or `FreeText`) that receives raw arguments when no command
  matches. Extras and fallbacks return `anyhow::Result<u8>`: `Ok(n)` exits `n`
  and an `Err` is reported like an op's error.
  `-h`, `--help` and `--version` as the first token never reach a fallback. A
  FreeText fallback also gets an empty rest, and registry globals are taken from
  its whole tail up to `--`.

### Hosts that act before any runtime

A host that must fork or pick its own runtime before the async runner (a daemon)
asks the registry what the command line selects, synchronously. Register the
daemon path with `Registry::fallback_flags_only_before_runtime(name, help)` (no
runner; `cli::run` reaching it exits 70), then:

```rust,ignore
// Both arms are an `ExitCode`: `daemon_main(args)` returns the same type as `cli::run`.
match op_registry::cli::select(&registry, &argv) {
    Ok(cli::Selected::Fallback { args, .. }) => daemon_main(args), // forks, builds its own runtime
    _ => current_thread_runtime().block_on(cli::run(&registry, argv, &hooks)),
}
```

`cli::parse` is sync too and returns the typed globals.

## Manual

With the `manual` feature, `Registry::with_manual(manual, audience)` adds a
`manual` op that renders every op the registry holds into one document;
`manual --json` gives the descriptors (with `commonCodes`, `audiences` and
`otherCommands`), `--out` writes a file. Common error codes are printed once;
each op lists its own. "Other commands" lists extras and the fallback with
their help (CLI only, not ops); "Operations off the command line" lists ops
without `cli`; an op not on every surface has a "Paths:" line; relations, the
needs line, `[env: X]` and the prompt sentence appear under the op. Only the command
line may write: on any other path (an MCP tool, a mount, a typed call) `out` is
refused with `op_registry.usage.invalid`.

## Tool lists and other transports

`mcp::tool_list(&registry, audience)` lists the ops that declare `mcp` and
serve that audience (an op may serve several): not hidden, not a stream. Annotations say whether an op only reads
or is destructive. `mcp::call_tool` runs one. The `rmcp` feature adds
`RegistryServer`, an rmcp `ServerHandler` for one audience;
`RegistryServer::with_ctx_factory` builds a context per `tools/call` and
`with_instructions` sets the server instructions. Other transports
(REST and so on) are mounted by the app over `Registry::call`.

## Tests the library gives you

Enable `op-registry` with `features = ["testing"]` as a dev-dependency (it
implies `manual`). The checks panic with every problem they find:

- `assert_parity(&registry, &mounts)`: every op reaches exactly the surfaces its
  `paths` declares (`cli`, `mcp`, each mount as `json`). A stream is absent from
  `mcp` and mounts, the page half of a list from the CLI; hidden ops are not
  exempt. An op reachable on `mcp` or `json` whose output root is not an object
  (a `serde_json::Value` is fine) and an `mcp` op with a plain `SecretFile` are
  problems.
- `assert_canonical`, `assert_casing(&registry, Casing::Camel)`,
  `assert_help_complete`, `assert_manual_snapshot`.
- `assert_exit_table`, `assert_error_codes`, `strict_codes`,
  `assert_unknown_ref_not_found`, `assert_local_ops_offline`.
- `assert_secrets_not_echoed` (with `SecretSentinel::new(..).with_globals(..)`),
  `sample_params` and `sample_params_seeded`.

The kit runs every op it can reach on a surface the op declares (a CLI-only op
included; a typed-only op is skipped and named), so a suite needs fixtures for
ops that cannot run on sampled arguments; a sample that does not decode is a
problem. Stream ops are called to their first item or end, under a 10 s limit,
with `max_retries = Some(0)`; this needs a tokio runtime with time
(`#[tokio::test]`), and under another executor the stream checks report a
problem. Kit calls carry the caller `op-registry-kit`.

## Cargo features

| feature | adds | default |
|---|---|---|
| (core) | registry, derive, runner, output, exits, help | yes |
| `schema` | output schemas via schemars 1 | yes |
| `manual` | the manual and the `manual` op | yes |
| `secrets` | `SecretFile` (zeroize) | yes |
| `prompt` | echo-off prompts for secrets (implies `secrets`; `rpassword` 7) | yes |
| `testing` | the test kit above (implies `manual`) | no, dev-dependency |
| `rmcp` | `RegistryServer` | no |

`op-registry` builds with `default-features = false`; `features = ["manual"]`
is the smallest useful build. Without `prompt`, a prompted field is
`op_registry.usage.missing` and `prompt_secret` returns `None`. The one lib
dependency is `coded-error`.

Cargo unifies features across a workspace build, so the `schema` feature is not
additive in practice: if any crate in the build enables it (the default), every
crate in that build gets output schemas. Build a binary that must stay small,
such as `gait-min`, on its own with `cargo build -p gait-min`, not with
`--workspace`.
