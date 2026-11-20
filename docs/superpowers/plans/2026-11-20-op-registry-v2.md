# op-registry v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace op-registry v1 with v2: one `ops!` declaration per binary that gives a typed in-process path, a CLI runner, MCP tools and transport mounts that run the same way, plus the help, manual and consumer test kit.

**Architecture:** The workspace keeps its four crates. `op-registry-derive` gets a rewritten `OpParams` derive, a new `OpGlobals` derive and a function-like `ops!` proc macro. All three expand to calls on `op-registry`'s public push API. `op-registry` keeps typed entries. Every path (typed, CLI, MCP, a transport) decodes into the typed params and then goes through one private pipeline: validate, resolve refs, build `CallInfo`, run under the `CallHook`, run the op. The CLI runner, MCP adapter, manual and testing kit read the registry and never call ops another way. op-status and op-listen are kept compiling at every task and moved to camelCase in the last task.

**Tech Stack:** Rust 2021 (rust-version 1.89), clap 4, serde/serde_json, anyhow, thiserror 2, futures-util, coded-error 0.3.0, syn 2/quote. Optional: schemars 1 (`schema`), zeroize 1 (`secrets`), rmcp 1 (`rmcp`).

**Spec:** `docs/superpowers/specs/2026-11-20-op-registry-v2-design.md`. It is binding. Its last section, "gp's schema decisions", overrides any body text that conflicts with it. Read the spec section each task cites before you start the task.

## Global Constraints

- op-registry has no auth, identity or signing. `Caller` is an opaque label that the library passes through and never checks.
- The only allowed lib dependency is coded-error, pinned at `rev = "de21d22e31baff78eae7d4ca817b87430bff1f81"` (version 0.3.0, on its main). There is no concord-telemetry and no `tracing` dependency in `[dependencies]`.
- Core deps are clap, serde, serde_json, anyhow, thiserror 2, futures-util and coded-error, plus syn, quote and proc-macro2 in the derive crate. Anything else goes behind a feature: schemars 1 → `schema`, zeroize → `secrets`, rmcp → `rmcp`. base64 is written in-crate (see Task 5), not taken as a dependency.
- Features: `default = ["schema", "manual", "secrets"]`; `testing = ["manual"]` (used as a dev-dependency only); `rmcp`. `dynamic` is not built.
- `op-registry` with `--no-default-features` must compile, because gait-min builds with `default-features = false, features = ["manual"]`.
- No backward compatibility. These are removed outright and get no shims: `registry!`, `Operation`, `CliHooks`, the `name:` override, `#[op(positional)]`, `#[op(short)]`, `#[op(long)]`, `#[op(value_delimiter)]`, `requires`, `since`, `deprecated` and `aliases`. When a task replaces a v1 piece, the same task deletes the old piece and its tests.
- JSON keys are camelCase on every JSON path, for input and output. CLI flags are long kebab-case flags only. The only short flag is `-h`.
- Exit table: 0 ok, 2 usage, 65 logical, 69 infrastructure, 70 unknown, 75 transient. Declared op exits fall in 3..=63.
- License: MIT OR Apache-2.0. Workspace version: 0.3.0.
- **Builds:** every cargo command runs through `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot`, as in `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry`. Use scoped `-p` runs only. Do not run workspace-wide test runs on this Mac.
- **"Green" after every task** means all four of these pass:
  `cargo-slot cargo test -p op-registry --all-features`, `cargo-slot cargo test -p op-status --features listen`, `cargo-slot cargo test -p op-listen --features testkit` and `cargo-slot cargo check -p op-registry --no-default-features`. Each one is the full cargo-slot path given above.
- **Git:** stage files by name. Never use `git commit -a` or `git add -A`. Never print git remote URLs.

## Rulings on spec ambiguities

These choices close gaps the spec leaves open. The tasks below rely on them.

1. **`cli::run` is `async`.** The library has no runtime, and the app supplies one (`#[tokio::main]`). Signature: `pub async fn run<C>(registry: &Registry<C>, argv: Vec<OsString>, hooks: &dyn RunHooks) -> ExitCode`.
2. **Only one stdin reader per call is a runtime check.** It is `op_registry.usage.stdin_twice` when two values in one call read stdin (`-` or `@-`). The derive does not forbid two `Json`/`Input` fields, because two of them is legitimate.
3. **`resolve: server` is declared in the `ops!` header** as `resolve_server = ["agent"];`, and in code as `Registry::resolve_server(kind)`.
4. **`id_only` skips the resolver.** The op receives `Resolved::Id(raw)`. Server kinds receive `Resolved::Raw(raw)`.
5. **`manual` is an op** (coordinator ruling, per spec 7 and gp's parity rule). `Registry::with_manual(manual, audience)` registers a `Local` op at path `manual`. It is on the CLI, on MCP for that audience and on the typed path, like any op. A consumer whose own words already include `manual` (gait `manual install`) hits the normal op-or-group build check, and resolves it in its adoption job.
6. **Op function signatures.** `op`/`local`: `async fn(&C, P, CallOptions) -> anyhow::Result<O>`. `stream`: `fn(&C, P, Option<Cursor>, CallOptions) -> BoxFuture<Result<BoxStream<Result<O>>>>` (v1's borrowed form). `list`: `async fn(&C, P, PageRequest) -> anyhow::Result<Page<T>>`, exactly as written in spec 5.3.
7. **The CLI caller comes from the context:** `CliContext::caller(&self) -> Option<Caller>` (default `None`).
8. **`too_many` code strings are built once per list op at registration** and leaked to `&'static str`, because coded-error codes are `&'static str`. A `list:` op with no noun word panics at push.
9. **CLI usage exit 2** covers `op_registry.usage.*`, `op_registry.cli.*` and clap parse errors. Clap error mapping: a missing required argument → `usage.missing`, an unknown subcommand → `cli.unknown_command`, anything else → `usage.invalid`.
10. **A hook that never polls `call`** gives an uncoded error, "call hook did not run the call", which exits 70. It does not panic or hang.
11. **Strings that start with `-`** must use `--flag=-value`. Signed numeric flags accept negative numbers.
12. **Stdout broken pipe** ends the run quietly: no error line, exit 0.
13. **A follow-all list whose next cursor repeats an earlier cursor** fails with an uncoded error, "page cursor repeated" (exit 70), instead of looping forever.
14. **Scope values** are the sibling fields' resolved ids, or their raw text if unresolved. They are keyed by Rust field name. Non-subject refs resolve before the subject.
15. **`secret_inline` JSON form:** a string is a path, and `{"inline": "<base64>"}` is the bytes.
16. **Commit messages** in this plan carry no trailer line. The user's commit rule says plans never prescribe one. The executing agent follows its own harness attribution.

## Review Focus

- **FreeText typo vs one-word task.** `gait bulid` is within edit distance 2 of the group `build`, so it is a usage error naming `build`. `gait refactor` (far from every command) and `gait "fix the bug"` (contains whitespace) go to the fallback. Test in Task 15.
- **Values that start with `-`.** `--note=-x` parses as `"-x"` and `--count -3` parses as `-3`. `--note -x` is a usage error (exit 2), not a panic. Test in Task 13.
- **Stdout closed early** (`| head -1` on a stream or a big array). No panic, no error line, exit 0. Test in Task 14.
- **Empty and broken paging.** A list with zero items prints `[]` (follow-all) or `{"items":[],"nextCursor":null}` (page). A server that repeats a cursor fails instead of looping. Test in Task 9.
- **A CallHook that drops `call` without polling it.** The caller gets an error (exit 70 on the CLI). There is no hang or panic, and `on_error` is called. Test in Task 6.

---

## File structure (end state)

`op-registry/src/`:
- `lib.rs`: module list, re-exports, `__private` (macro support only).
- `descriptor.rs`: `Access`, `Audience`, `OpKind`, `Paths`, `Exit`, `Descriptor`, `derive_name`.
- `params/mod.rs`: `OpParams`, `FieldMeta`, `FieldKind`, `ParseCx`, `UsageError`, `Redactor`.
- `params/types.rs`: `Patch<T>`, `Json`, `Input`.
- `params/base64.rs`: in-crate standard base64 encode and decode.
- `params/secret.rs`: (`secrets`) `Secret<T>`, `SecretFile`. `Redactor` is in `params/mod.rs`, always compiled.
- `refs.rs`: `Ref`, `Resolved`, `Scope`, `RefSlot`, `Resolve` (all added in Task 7).
- `call.rs`: `CallOptions`, `Caller`, `Cursor`, `CallPath`, `TraceParent`, `CallInfo`, `CallError`.
- `notices.rs`: `Notices`, `NoticeSink`, `Notice`.
- `hook.rs`: `CallHook`, `NoHook`.
- `registry.rs`: `Registry<C>`, `OpSpec<P, O>`, entries, groups, merge, build-time checks.
- `invoke.rs`: the one pipeline, plus `Registry::call`, `call_stream`, `call_typed`, `call_stream_typed`, and `Op`.
- `list.rs`: `PageRequest`, `Page<T>`, `ListError`, follow-all.
- `exit.rs`: exit table, `OkExit`, `exit_code`.
- `output.rs`: `Style`, `RenderText`, `print_json`, `write_error`, error, notice and progress lines, `write_private`, `OutputSchema`.
- `globals.rs`: `OpGlobals`, `CliContext`.
- `cli/mod.rs`, `cli/tree.rs`, `cli/parse.rs`, `cli/run.rs`, `cli/io.rs`, `cli/fallback.rs`, `cli/extra.rs`.
- `mcp/mod.rs`, `mcp/rmcp.rs` (`rmcp`).
- `manual.rs` (`manual`).
- `testing/mod.rs`, `testing/sample.rs`, `testing/parity.rs`, `testing/help.rs`, `testing/casing.rs`, `testing/snapshot.rs`, `testing/codes.rs`, `testing/secrets.rs`, `testing/refs.rs`, `testing/offline.rs` (`testing`).

`op-registry-derive/src/`: `lib.rs` (entry points), `params/{mod,attrs,types,schema,clap_gen,json_gen}.rs`, `globals.rs`, `ops/{mod,parse,expand}.rs`.

`op-registry/tests/`: `common/mod.rs` (shared fixture context), `descriptor.rs`, `registry.rs`, `params_flags.rs`, `params_relations.rs`, `params_inputs.rs`, `params_ui.rs` + `params_ui/`, `call.rs`, `refs.rs`, `secrets.rs`, `lists.rs`, `ops_macro.rs` + `ops_ui/`, `mcp.rs`, `output.rs`, `cli_tree.rs`, `runner.rs`, `fallback.rs`, `manual.rs` + `manual_snapshot.md`, `rmcp.rs`, `testing_kit.rs`.

---

### Task 1: Foundation: coded-error 0.3.0, features, `OutputSchema`

Spec: sections 0, 6.3 and 11.

**Files:**
- Modify: `Cargo.toml` (workspace: version 0.3.0, coded-error rev, `zeroize = "1"`, `rmcp = { version = "1", default-features = false, features = ["server"] }`)
- Modify: `op-registry/Cargo.toml`, `op-registry-derive/Cargo.toml`, `op-status/Cargo.toml`, `op-listen/Cargo.toml` (path deps `version = "0.3.0"`)
- Modify: `op-registry/src/lib.rs`
- Create: `op-registry/src/output.rs` (the `OutputSchema` trait only, for now)
- Modify: `op-status/src/ops.rs`, `op-listen/src/entry.rs` (`output_schema_of` → `OutputSchema`)
- Test: `op-registry/tests/output_schema.rs`

**Interfaces:**
- Produces: `pub trait OutputSchema { fn output_schema() -> Option<serde_json::Value>; }`. With `schema` on, it is a blanket impl for `T: schemars::JsonSchema` returning `Some(schema_for!(T))`. With `schema` off, it is a blanket impl for every `T` returning `None`. It is re-exported at the crate root. `__private::output_schema_of` is deleted.
- op-registry features: `default = ["schema", "manual", "secrets"]`, `schema = ["dep:schemars"]`, `manual = []`, `secrets = ["dep:zeroize"]`, `testing = ["manual"]`, `rmcp = ["dep:rmcp"]`. op-status `registry` and op-listen depend on op-registry with default features, and keep their own `schemars` dependency.

- [ ] **Step 1: Write the failing test** `output_schema_is_some_with_schema_feature`. It asserts that `<Echo as OutputSchema>::output_schema().unwrap()["properties"]["id"]["type"] == "string"` for a `#[derive(Serialize, JsonSchema)] struct Echo { id: String }`. Gate the file with `#![cfg(feature = "schema")]`.
- [ ] **Step 2: Run it and confirm it fails.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test output_schema`. Expected: compile error, `OutputSchema` not found.
- [ ] **Step 3: Implement.** Bump the coded-error rev to `de21d22e31baff78eae7d4ca817b87430bff1f81` and keep its features `["serde", "cli", "http"]`. Add the features and make `schemars` optional. Add `OutputSchema`, and replace every `output_schema_of::<T>()` call (in `registry!`, op-status and op-listen) with `<T as OutputSchema>::output_schema().unwrap_or_default()`. Bump every crate version to 0.3.0.
- [ ] **Step 4: Run the test and the green set.** The test passes. All four green commands from Global Constraints pass, including `--no-default-features`.
- [ ] **Step 5: Commit.** `git add Cargo.toml op-registry/Cargo.toml op-registry-derive/Cargo.toml op-status/Cargo.toml op-listen/Cargo.toml op-registry/src/lib.rs op-registry/src/output.rs op-status/src/ops.rs op-listen/src/entry.rs op-registry/tests/output_schema.rs Cargo.lock`, then `git commit -m "op-registry v2: coded-error 0.3.0, cargo features, OutputSchema"`.

---

### Task 2: Descriptor v2 and the registry core (push API)

Spec: sections 2.1 and 2.2.

**Files:**
- Rewrite: `op-registry/src/descriptor.rs`, `op-registry/src/registry.rs`
- Modify: `op-registry/src/call.rs` (`CallError` gains `WrongPath`), `op-registry/src/lib.rs`
- Modify (minimal adaptation, still v1 behaviour): `op-registry/src/cli.rs`, `op-registry/src/mcp.rs`, `op-registry/src/parity.rs`. They read `descriptor.path` and `descriptor.kind` instead of noun/verb/streaming, and dispatch via the entry's typed CLI decoder. Tasks 11, 13 and 18 replace these files.
- Modify: `op-status/src/ops.rs`, `op-listen/src/entry.rs`, and their tests that call `push_unary`, `push_stream_borrowed`, `Descriptor::new` or `registry!`
- Delete: `op-registry/tests/descriptor.rs`, `registry.rs`, `call_options.rs`, `frontends.rs`, `noun_paths.rs` and `parity.rs` (all are v1 `registry!` tests), and `op-registry/tests/ui.rs` with `op-registry/tests/ui/`
- Create: `op-registry/tests/descriptor.rs`, `op-registry/tests/registry.rs`, `op-registry/tests/common/mod.rs`

**Interfaces:**
- Produces (`descriptor.rs`):
  - `Access`: unchanged fields. `Access::from_words(&[&str]) -> Access` accepts `reads|writes|network|spawns|destructive`, where `destructive` also sets `writes`.
  - `Audience { Agent, Owner, Dev }`: unchanged.
  - `#[non_exhaustive] enum OpKind { Unary, Local, Stream, ListAll, ListPage }`.
  - `enum Paths { All, TypedOnly }`.
  - `struct Exit { code: u8, meaning: String }`.
  - `#[non_exhaustive] struct Descriptor` with the fields `name: String`, `path: Vec<String>`, `summary: String`, `long_help: String`, `access: Access`, `audience: Audience`, `kind: OpKind`, `hidden: bool`, `group_default: bool`, `text: bool`, `exits: Vec<Exit>`, `errors: Vec<ErrorCodeEntry>`, `paths: Paths`, `cursor: bool`, `input_schema: Value`, `output_schema: Option<Value>`, `list_pair: Option<String>` (the other op's name) and `unpaged: bool`.
  - `ErrorCodeEntry { code: String, class: Option<ErrorClass> }`, a serialisable copy of `coded_error::CodeEntry`.
  - `pub fn derive_name(path: &[String]) -> String`: words joined by `_`, with every `-` turned into `_`.
  - Descriptor accessors: `verb()` (last word) and `noun_path()` (all words but the last).
- Produces (`registry.rs`):
  - `pub struct OpSpec<P, O>` with public fields `path: &'static str` (space-separated words), `summary`, `long_help`, `access`, `audience`, `hidden`, `group_default`, `text`, `exits: Vec<(u8, &'static str)>`, `errors: Vec<coded_error::CodeEntry>`, `paths: Paths`, `cursor: bool`, `cap: usize` (default 10_000), `page_size: u32` (default 100), `unpaged: bool` `fields: Option<fn(&P) -> Vec<(&'static str, String)>>`, `ok_exit: Option<fn(&O) -> u8>` and `render_text: Option<fn(&O) -> String>`. `O` is the op's output: the item type for streams, and `T` for list pairs (Task 9). It has `OpSpec::new(path, summary)`, and `Default` covers the rest. `ok_exit` and `render_text` are stored here and acted on from Task 12.
  - `Registry<C>`:
    - `new()` and `set_pillar(&'static str)`.
    - `group(&mut self, path: &str, summary: &str)`.
    - `push_op<P, O, F>(&mut self, spec: OpSpec<P, O>, f: F)` where `F: for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync + 'static` and `O: Serialize + OutputSchema + Send + 'static`.
    - `push_local<P, O, F>`: the same shape, with kind `Local`.
    - `push_stream<P, O, F>`: v1's `push_stream_borrowed` shape. The name `push_stream_borrowed` and v1's owned `push_stream` are deleted.
    - `merge(&mut self, other)`, `descriptors()`, `get(name)` and `names()`.
  - The registry does not require `P: Serialize` (CLI decodes straight to `P`).
- Build-time checks (each one is a `panic!` naming both ops):
  - a duplicate name;
  - a word that is both an op and a group, unless the op is that group's `group_default`;
  - a noun path whose group has no summary (`group` must come first);
  - two `group_default` ops in one group.
- `CallError::WrongPath { name }`, code `op_registry.call.wrong_path`, Logical.

- [ ] **Step 1: Write the failing tests.**
  - `descriptor.rs`:
    - `name_joins_words_and_snakes_hyphens`: `derive_name(["dispatch-target","list"]) == "dispatch_target_list"`.
    - `one_word_path_is_bare_verb`: path `"index"` gives name `index`, `verb() == "index"` and an empty noun path.
    - `access_from_words_destructive_writes`.
  - `registry.rs`:
    - `push_op_registers_derived_name`.
    - `duplicate_name_panics_naming_both`: `#[should_panic(expected = "concept_get")]`.
    - `word_both_op_and_group_panics`: `build` and `build status`.
    - `group_default_may_share_its_group_word`: `build run` with `group_default` plus `build status` is OK.
    - `two_defaults_in_group_panic`.
    - `missing_group_summary_panics`: `expected = "group \"concept\" has no summary"`.
    - `merge_keeps_order_and_checks_collisions`.
    - `typed_only_op_called_by_json_is_wrong_path`: `registry.call("x", json!({}), &ctx, CallOptions::default())` gives the code `op_registry.call.wrong_path`.
  - `common/mod.rs`: `pub struct Ctx;` plus small param and output fixtures, reused by later tasks.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test descriptor --test registry`. Expected: compile errors (`derive_name`, `OpSpec` and `push_op` are missing).
- [ ] **Step 3: Implement.**
  - Store each entry as typed closures behind erased boxes: `Box<dyn Any + Send + Sync>` for the typed fn, a JSON decoder `fn(Value, &mut ParseCx) -> Result<Box<dyn Any + Send>>`, a clap augmenter and a matches decoder.
  - Delete `registry!`, `Operation`, `__private::Sealed`, `__private::access!`, `CliHooks` and `cli_hooks()`. In `Descriptor`, delete `noun`, `verb`, `name()`, `requires`, `since`, `deprecated`, `aliases`, `renders_text`, `exit_code_on_ok`, `streaming`, `is_path` and `cli_name`.
  - `ParseCx` is introduced here as an empty struct in `params/mod.rs`, and Task 3 fills it in. `OpParams` keeps v1's methods until Task 3, with `from_matches` and `from_json` gaining a `&mut ParseCx` parameter.
  - Migrate op-status `status_registry` and op-listen `listen_entry` to `push_op` and `push_stream`. `listen_entry` takes an `OpSpec<L::Params, serde_json::Value>` instead of a `Descriptor`. op-status declares the group `status` ("Component status.").
- [ ] **Step 4: Run the tests and the green set.** The new tests pass, and all four green commands pass.
- [ ] **Step 5: Commit.** Stage the files listed above by name, including the deletions (`git rm` for the deleted tests). Message: `op-registry v2: descriptor v2 and typed push API; drop registry!, Operation, CliHooks`.

---

### Task 3: `OpParams` derive v2: flags, keys, scalar and collection types, help

Spec: sections 3.1, 3.3 (lists, maps, options, switches), 3.5 (scalar rows) and 7 (field docs).

**Files:**
- Rewrite: `op-registry-derive/src/lib.rs` into `op-registry-derive/src/params/{mod,attrs,types,schema,clap_gen,json_gen}.rs`
- Modify: `op-registry/src/params/mod.rs` (moved from `params.rs`)
- Modify: `op-status/src/ops.rs` (drop `#[op(long = ...)]`)
- Modify: `op-listen` tests and testkit params that use removed attributes
- Delete: `op-registry/tests/params.rs`, `op-registry/tests/params_ui/op_params_required_unless_present_needs_option_is_a_compile_error.{rs,stderr}`
- Create: `op-registry/tests/params_flags.rs`. New `params_ui` cases: `op_params_long_attribute_is_a_compile_error.rs`, `op_params_short_attribute_is_a_compile_error.rs`, `op_params_positional_attribute_is_a_compile_error.rs`, `op_params_env_on_params_is_a_compile_error.rs`, each with its `.stderr`. Keep the three still-valid v1 ui cases and refresh their `.stderr`.

**Interfaces:**
- Produces:
  ```rust
  pub trait OpParams: Sized + Send + 'static {
      fn fields() -> Vec<FieldMeta>;
      fn input_schema() -> serde_json::Value;
      fn add_args(cmd: clap::Command) -> clap::Command;
      fn from_matches(m: &clap::ArgMatches, cx: &mut ParseCx) -> anyhow::Result<Self>;
      fn from_json(v: serde_json::Value, cx: &mut ParseCx) -> anyhow::Result<Self>;
      fn validate(&self) -> anyhow::Result<()> { Ok(()) }
      fn secrets(&self) -> Vec<Vec<u8>> { Vec::new() }
  }
  ```
  Task 7 adds a fifth provided method, `ref_slots`. `Ref`, `Scope` and `RefSlot` are not used before Task 7.
- `FieldMeta { rust_name: &'static str, flag: String, key: String, help: String, kind: FieldKind, required: bool, secret: bool, from_group: Option<&'static str> }`.
- `FieldKind { Scalar, Switch, TriState, Repeated, Map, Json, Input, Ref, SecretFile }`, `#[non_exhaustive]`.
- `ParseCx` (stdin and secrets get added in Tasks 5 and 8): `ParseCx::default()`.
- `UsageError`: add `OneOf { group: String }` (`one_of`), `RequiresIf { field: String, other: String }` (`requires_if`) and `StdinTwice { first: String, second: String }` (`stdin_twice`), all Logical. Keep `missing`, `invalid`, `unknown_key` and `repeated_key`.
- Attribute keys accepted: `default`, `value_enum`, `keys`, and `secret` (parsed now, acted on in Task 8). Unknown keys are compile errors that name the key. Supported field types: `String`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `usize`, `f64`, `PathBuf`, `bool`, `Option<T>`, `Vec<T>`, `BTreeMap<String, String>`, and any `#[op(value_enum)]` type. Later tasks add types.
- Rules:
  - The flag is `--` plus the field name with `_` → `-`. The JSON key is camelCase (`agent_id` → `agentId`). The schema property is the JSON key.
  - The field's doc comment (all lines joined by spaces) goes to clap `.help()` and schema `description`.
  - `Vec<T>` is `ArgAction::Append` with no delimiter, and a JSON array.
  - `Option<bool>` is tri-state: `--x` and `--no-x` conflict, and both absent gives `None`. On JSON it is a boolean or absent.
  - `BTreeMap<String,String>` is a repeated `--name k=v`. With `keys = [...]` the keys are closed: an unknown key is `usage.unknown_key` and the schema has `properties` plus `additionalProperties: false`. Without `keys` the schema is `additionalProperties: {"type":"string"}`. A repeated key is `usage.repeated_key`. Map keys never get camelCased.
  - Signed integers and `f64` set `allow_negative_numbers(true)`.
  - An unknown JSON key is `usage.unknown_key` (param `params`).
  - `from_json` accepts only camelCase keys.

- [ ] **Step 1: Write the failing tests** in `params_flags.rs`, with a fixture struct that has `agent_id: String`, `limit: u32` (`default = 50`), `tags: Vec<String>`, `portable: Option<bool>`, `dry_run: bool`, `metadata: BTreeMap<String,String>`, `labels: BTreeMap<String,String>` (`keys = ["env","team"]`), `count: Option<i64>`, `note: Option<String>`, `out: Option<PathBuf>` and `mode: Mode` (`value_enum`).
  - `flag_is_kebab_and_key_is_camel`: `fields()` has flag `agent-id` and key `agentId`, and `input_schema()["properties"]` contains `agentId`.
  - `doc_comment_is_help_and_description`.
  - `repeated_flag_collects_vec`: `--tags a --tags b` gives `["a","b"]`.
  - `comma_is_not_a_delimiter`: `--tags a,b` gives `["a,b"]`.
  - `tri_state_bool`: `--portable` → `Some(true)`, `--no-portable` → `Some(false)`, neither → `None`, both → a clap error.
  - `open_map_accepts_any_key`.
  - `closed_map_rejects_unknown_key`: code `op_registry.usage.unknown_key`.
  - `repeated_map_key_is_usage_error`.
  - `json_rejects_snake_key`: `{"agent_id": "x"}` gives `usage.unknown_key`.
  - `json_and_cli_decode_equal`.
  - `negative_number_parses`: `--count -3` gives `Some(-3)`.
  - `hyphen_string_needs_equals`: `--note=-x` gives `Some("-x")`, while `--note -x` is a clap error, not a panic.
  - `only_short_flag_is_h`: walk `add_args(Command::new("t"))`'s args and assert that none has a short except the auto help.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test params_flags --test params_ui`. Expected: FAIL (old attributes and snake keys).
- [ ] **Step 3: Implement** the rewritten derive into the module split listed under Files. One `Field` model is parsed once in `attrs.rs`, and `schema.rs`, `clap_gen.rs` and `json_gen.rs` each emit one method. Do not port v1's string-concatenated `quote!` blob.
- [ ] **Step 4: Run the tests and the green set.** All tests pass. Refresh the trybuild `.stderr` files with `TRYBUILD=overwrite` once, then review the diff by eye.
- [ ] **Step 5: Commit.** `op-registry v2: OpParams derive with kebab flags, camelCase keys, repeated lists, maps, tri-state`.

---

### Task 4: `OpParams` relations: `one_of`, `requires_if`, `clearable`, `flatten`

Spec: sections 3.3 (clearing), 3.4 and 3.6.

**Files:**
- Modify: `op-registry-derive/src/params/*`
- Create: `op-registry/src/params/types.rs` (`Patch<T>`)
- Modify: `op-registry/src/registry.rs` (flatten clash check at push)
- Test: `op-registry/tests/params_relations.rs`, plus `params_ui/op_params_one_of_needs_two_options_is_a_compile_error.rs` and `params_ui/op_params_clearable_needs_option_is_a_compile_error.rs`

**Interfaces:**
- Produces: `#[derive(Clone, Debug, PartialEq, Default)] pub enum Patch<T> { #[default] Keep, Set(T), Clear }`. A `#[op(clearable)] x: Option<T>` field becomes the field type `Patch<T>`. The user writes `Patch<T>` with `#[op(clearable)]`, and the derive requires both. It adds `--clear-x`, which conflicts with `--x` (clap error, so usage exit 2). On JSON, `null` means `Clear` and absent means `Keep`. The schema allows `null` (`"type": [T, "null"]`).
- `#[op(one_of = "g")]` on two or more `Option` fields means exactly one is given. `#[op(one_of_optional = "g")]` means at most one. These produce a clap `ArgGroup` (required or not) and a schema `oneOf`. For `one_of_optional`, the schema is `oneOf` over each field alone plus "none of them". `validate()` returns `usage.one_of`.
- `#[op(requires_if = "other")]`: the field is required when `other` is present. The schema gets `dependentRequired: { otherKey: [thisKey] }` and `validate()` returns `usage.requires_if`. The CLI uses clap `required_if_eq_any`, or checks in `validate()`.
- `#[op(flatten)] common: G` with `G: OpParams`: G's fields, args and keys merge into the parent. `fields()` marks them `from_group: Some("G")`. A clash between flattened names and the op's own names is a panic at `Registry::push_*` naming both fields and the op, because the derive can't see G's fields.
- `validate()` is called on every path. Decoders call it, and the typed path calls it in Task 6.

- [ ] **Step 1: Write the failing tests.**
  - `one_of_exactly_one_cli_and_json`: none → `usage.one_of` on JSON and a clap error on the CLI; two → error; one → OK.
  - `one_of_optional_allows_none`.
  - `one_of_schema_has_oneof`.
  - `requires_if_missing_is_usage_error`: JSON `{"uri":"x"}` without `collection` gives `usage.requires_if`.
  - `requires_if_schema_dependent_required`: `schema["dependentRequired"]["uri"] == json!(["collection"])`.
  - `clearable_cli_three_states`: `--host-machine m` → `Set`, `--clear-host-machine` → `Clear`, neither → `Keep`, both → clap error.
  - `clearable_json_null_is_clear_absent_is_keep`.
  - `flatten_adds_group_flags_and_keys`: `ProjectArgs { root: Option<PathBuf>, gcode_store: Option<String> }` gives `--gcode-store` and key `gcodeStore` on the parent.
  - `flatten_clash_panics_at_push`: `#[should_panic(expected = "root")]`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test params_relations --test params_ui`. Expected: compile errors on the unknown attributes.
- [ ] **Step 3: Implement** the attributes in the derive modules, `Patch<T>`, and the clash check in `registry.rs` (compare `P::fields()` flags among themselves).
- [ ] **Step 4: Run the tests and the green set.** All pass.
- [ ] **Step 5: Commit.** `op-registry v2: one_of, requires_if, clearable Patch, flattened groups`.

---

### Task 5: `Json` and `Input` params, stdin, in-crate base64

Spec: section 3.5.

**Files:**
- Modify: `op-registry/src/params/types.rs` (`Json`, `Input`), `op-registry/src/params/mod.rs` (`ParseCx` stdin)
- Create: `op-registry/src/params/base64.rs`
- Modify: the derive (`Json`, `Input` and `Vec<Json>` field kinds)
- Test: `op-registry/tests/params_inputs.rs`

**Interfaces:**
- Produces:
  - `pub struct Json(pub serde_json::Value)`. On the CLI it accepts a JSON string, `@path` (file contents parsed as JSON) or `@-` (stdin). On JSON paths it is any value, with schema `{}` plus a description.
  - `pub struct Input(pub Vec<u8>)`. On the CLI it is a path, or `-` for stdin. On JSON it is a base64 string, with schema `{"type":"string","contentEncoding":"base64"}`.
  - `ParseCx { stdin: Option<Box<dyn Read + Send>>, stdin_used_by: Option<String>, secrets: Vec<Vec<u8>> }` with `ParseCx::with_stdin(r)` and `ParseCx::read_stdin(&mut self, field: &str) -> anyhow::Result<Vec<u8>>`. The second call returns `usage.stdin_twice { first, second }`. With no stdin present, it returns `usage.invalid` ("stdin is not available on this path").
  - `base64::{encode(&[u8]) -> String, decode(&str) -> Result<Vec<u8>, UsageError>}` uses the standard alphabet with padding required. Invalid input gives `usage.invalid`.
- An unreadable `@path` or path gives `usage.invalid` naming the field and path. Invalid JSON text gives `usage.invalid`.

- [ ] **Step 1: Write the failing tests.**
  - `json_flag_inline_text`.
  - `json_flag_at_path_reads_file`: use a temp file under `std::env::temp_dir()` with a unique name.
  - `json_flag_at_dash_reads_stdin`.
  - `input_dash_reads_stdin_bytes`.
  - `input_json_is_base64`: `"aGk="` gives `b"hi"`.
  - `input_schema_has_content_encoding`.
  - `two_stdin_readers_is_stdin_twice`: `--body @- --file -` gives `op_registry.usage.stdin_twice`.
  - `base64_roundtrip_and_bad_padding`.
  - `vec_json_is_array_of_any`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test params_inputs`.
- [ ] **Step 3: Implement** as specified.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: Json and Input params, one stdin reader per call`.

---

### Task 6: The call pipeline: `CallOptions`, `CallInfo`, `CallHook`, `Notices`, typed path

Spec: sections 4.3, 4.4 and 4.5.

**Files:**
- Modify: `op-registry/src/call.rs`
- Create: `op-registry/src/hook.rs`, `op-registry/src/notices.rs`, `op-registry/src/invoke.rs`
- Modify: `op-registry/src/registry.rs` (`set_call_hook`), `op-registry/src/lib.rs`
- Modify: `op-listen/src/{detach,entry}.rs` and `op-status/src/ops.rs` for the new `CallOptions`
- Test: `op-registry/tests/call.rs`

**Interfaces:**
- Produces:
  ```rust
  #[non_exhaustive] #[derive(Clone, Default)]
  pub struct CallOptions { pub caller: Option<Caller>, pub trace: TraceParent, pub notices: Notices, pub transport: Option<String> }
  // with_caller, with_trace, with_notices, with_transport builders
  #[derive(Clone, Debug, Default, PartialEq, Eq)]
  pub struct TraceParent { pub traceparent: Option<String>, pub tracestate: Option<String>, pub baggage: Option<String> }
  #[derive(Clone, Debug, PartialEq, Eq)] pub enum CallPath { Typed, Cli, Mcp, Transport(String) }
  #[non_exhaustive] #[derive(Clone, Debug)]
  pub struct CallInfo { pub name: String, pub access: Access, pub audience: Audience, pub path: CallPath,
                        pub caller: Option<Caller>, pub fields: Vec<(&'static str, String)>, pub trace: TraceParent }
  pub trait CallHook: Send + Sync {
      fn around<'a>(&'a self, info: &CallInfo, call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()> { let _ = info; call }
      fn on_error(&self, info: &CallInfo, err: &anyhow::Error) { let _ = (info, err); }
  }
  pub struct NoHook;  // impl CallHook
  pub trait NoticeSink: Send + Sync { fn send(&self, notice: Notice); }
  #[derive(Clone, Debug, PartialEq)] pub enum Notice { Text(String), Progress { done: u64, total: Option<u64>, label: String } }
  #[derive(Clone, Default)] pub struct Notices(Option<Arc<dyn NoticeSink>>);
  // Notices::new(sink), notice(&self, impl Into<String>), progress(&self, done, total: Option<u64>, label: &str); no sink = dropped
  pub trait Op { type Ctx; type Params: OpParams; type Output; const NAME: &'static str; }
  impl<C> Registry<C> {
      pub fn set_call_hook(&mut self, hook: Arc<dyn CallHook>);
      pub async fn call(&self, name: &str, args: Value, ctx: &C, opts: CallOptions) -> anyhow::Result<Value>;
      pub async fn call_stream<'a>(&'a self, name: &str, args: Value, ctx: &'a C, opts: CallOptions, cursor: Option<Cursor>)
          -> anyhow::Result<BoxStream<'a, anyhow::Result<Value>>>;
      pub async fn call_typed<O: Op<Ctx = C>>(&self, ctx: &C, params: O::Params, opts: CallOptions) -> anyhow::Result<O::Output>;
      pub async fn call_stream_typed<'a, O: Op<Ctx = C>>(&'a self, ctx: &'a C, params: O::Params, opts: CallOptions, cursor: Option<Cursor>)
          -> anyhow::Result<BoxStream<'a, anyhow::Result<O::Output>>>;
  }
  ```
- Pipeline (private `invoke.rs`, one function used by every path):
  1. Look up the op. Reject `Paths::TypedOnly` on non-typed paths (`wrong_path`). Reject a shape mismatch (`wrong_shape`).
  2. Decode, unless the call is typed.
  3. `validate()`.
  4. Resolve refs (a no-op until Task 7).
  5. Build `CallInfo`. `fields` comes from `OpSpec::fields`. `path` is `Typed` for `call_typed`, and `Transport(opts.transport.unwrap_or("json"))` for `call`. The adapters in Tasks 11 and 14 pass `Cli` or `Mcp` through a `pub(crate)` entry.
  6. Run the op inside `hook.around(&info, call)`, storing the typed result in a slot the library owns.
  7. On `Err`, call `hook.on_error(&info, &err)` exactly once.
  - A slot left empty after `around` gives the uncoded error "call hook did not run the call".
  - For streams, the hook wraps the stream open only.
- `call_typed` looks up `O::NAME` and downcasts the stored typed fn. A type mismatch gives `wrong_shape`. Errors keep their concrete type (`coded_error::downcast_ref::<AppError>(&err)` works).
- The default hook is `NoHook`. `merge` keeps `self`'s hook.

- [ ] **Step 1: Write the failing tests.**
  - `hook_sees_every_path_with_path_label`: a recording hook; `call_typed` → `Typed`, and `call` with `with_transport("rest")` → `Transport("rest")`. The hook sees name, access, audience and caller.
  - `fields_fn_feeds_call_info`: `fields: Some(|p| vec![("collection_id", p.collection.clone())])`.
  - `trace_parent_passed_through_untouched`.
  - `on_error_called_once_on_failure`.
  - `typed_error_keeps_concrete_type`.
  - `typed_path_runs_validate`: a `one_of` violation through `call_typed` gives `usage.one_of`.
  - `typed_only_op_callable_typed`.
  - `hook_dropping_call_is_error_not_hang`: `around` returns `Box::pin(async {})`. The call returns `Err` whose message contains "call hook did not run the call", and `on_error` is called. Use `#[tokio::test]` with a 1s `tokio::time::timeout` around the call.
  - `notices_reach_sink`: an op calls `opts.notices.notice("x")` and `progress(1, Some(2), "l")`, and a `Vec`-backed sink sees both.
  - `stream_open_goes_through_hook`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test call`.
- [ ] **Step 3: Implement** `invoke.rs` and the types. `Registry::call` and `call_stream` become thin wrappers over it. Delete v1's free functions `call::call` and `call::call_stream`.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: one call pipeline with CallInfo, CallHook, notices and the typed path`.

---

### Task 7: Refs and resolution

Spec: sections 3.2 and 6.4.

**Files:**
- Create: `op-registry/src/refs.rs`
- Modify: `op-registry/src/invoke.rs`, `op-registry/src/registry.rs`, the derive (`subject`, `id_only`, `scope`, `kind`, and the `Ref` type)
- Modify: op-status and op-listen generic bounds (`C: Resolve`)
- Test: `op-registry/tests/refs.rs`, plus `params_ui/op_params_two_subjects_is_a_compile_error.rs` and `params_ui/op_params_scope_names_unknown_field_is_a_compile_error.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq)] pub struct Ref { /* raw + Option<Resolved> */ }
  // Ref::new(raw), as_raw(&self) -> &str, resolved(&self) -> &Resolved (panics "ref not resolved" if read before resolution — never happens inside an op), id(&self) -> &str
  #[non_exhaustive] #[derive(Clone, Debug, PartialEq, Eq)] pub enum Resolved { Id(String), Raw(String) }
  #[derive(Clone, Debug, Default, PartialEq, Eq)] pub struct Scope(BTreeMap<String, String>); // get(&self, name) -> Option<&str>
  pub trait Resolve: Send + Sync {
      /// Default: an uncoded error "context has no resolver for kind `<kind>`" (exit 70).
      /// assert_unknown_ref_not_found (Task 19) flags it, so an app with refs cannot forget it.
      fn resolve<'a>(&'a self, kind: &'a str, r: &'a Ref, scope: &'a Scope) -> BoxFuture<'a, anyhow::Result<Resolved>> { /* default above */ }
  }
  pub struct RefSlot<'a> { pub kind: &'static str, pub subject: bool, pub id_only: bool, pub scope: Scope, pub slot: &'a mut Ref }
  // added to OpParams in this task, with this default:
  //     fn ref_slots(&mut self) -> Vec<RefSlot<'_>> { Vec::new() }
  impl<C> Registry<C> { pub fn resolve_server(&mut self, kind: &'static str); }
  ```
- `Registry<C>`'s push and call impls now require `C: Resolve`. A context with no refs writes `impl Resolve for Ctx {}`.
- Derive:
  - `#[op(subject = "build")] id: Ref` always has flag `--id` and key `id`, whatever the field is named. More than one subject is a compile error.
  - `#[op(subject = "agent", id_only)]`.
  - `#[op(scope = ["blueprint"])]` on the subject names sibling fields. An unknown name is a compile error.
  - `#[op(kind = "collection")] collection: Ref` is a plain object flag. `Option<Ref>` and `Vec<Ref>` are allowed for `kind` refs.
  - `ref_slots()` returns non-subject refs first, then the subject. Scope values are filled with the siblings' resolved ids (ruling 14).
- Invoke resolves each slot after validation:
  - a server kind → `Resolved::Raw(raw)`;
  - `id_only` → `Resolved::Id(raw)`;
  - otherwise `ctx.resolve(kind, ref, scope)`, and errors propagate unchanged (the app's `<pillar>.<kind>.not_found` and `.ambiguous`).

- [ ] **Step 1: Write the failing tests.** Use a fixture resolver knowing `build` `b1` (name `nightly`, blueprint `bp`) and `collection` `c1`. It fails with a test-local coded `fix.build.not_found` and `fix.build.ambiguous`.
  - `subject_is_id_flag_and_key`.
  - `name_resolves_to_id_before_op`: the op sees `Resolved::Id("b1")` for `--id nightly`.
  - `unknown_ref_is_apps_not_found_on_every_path`: all three legs give `fix.build.not_found`. The CLI leg (there is no CLI tree until Task 13) builds `P::add_args(Command::new("t"))`, parses `["t", "--id", "nope"]`, decodes with `P::from_matches`, and calls `call_typed`. The JSON leg uses `call`, and the typed leg uses `call_typed` with `Ref::new("nope")`.
  - `resolve_default_is_error`: a context with `impl Resolve for Ctx {}` gives an `Err` containing "no resolver for kind".
  - `scope_passes_resolved_sibling`.
  - `server_kind_skips_resolver`: the resolver panics if called, and the op sees `Resolved::Raw`.
  - `id_only_skips_resolver`.
  - `kind_ref_flag_resolves`: `--waits-for x` with an unknown x gives not_found.
  - `ref_schema_is_string`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test refs --test params_ui`.
- [ ] **Step 3: Implement.** op-status and op-listen gain the `C: Resolve` bound. Their test contexts write `impl Resolve for Ctx {}`.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: Ref, subject --id, scopes and resolution on every path`.

---

### Task 8: Secrets

Spec: section 3.8.

**Files:**
- Create: `op-registry/src/params/secret.rs` (`#[cfg(feature = "secrets")]`)
- Modify: the derive (`SecretFile` type, `secret`, `secret_inline`), `op-registry/src/invoke.rs` (collect the redaction set and wrap notices), `op-registry/src/output.rs` (`write_private`)
- Test: `op-registry/tests/secrets.rs` (`#![cfg(feature = "secrets")]`)

**Interfaces:**
- Produces:
  ```rust
  pub struct Secret<T>(T);  // Debug + Display print "<secret>"; new(T), expose(&self) -> &T
  pub struct SecretFile { /* path: Option<PathBuf>, show_path: bool, bytes: Secret<zeroize::Zeroizing<Vec<u8>>> */ }
  // SecretFile::bytes(&self) -> &[u8], path(&self) -> Option<&Path>; Debug prints the path (or "<secret>" when the field is #[op(secret)]) and never the bytes
  #[derive(Clone, Debug, Default)] pub struct Redactor { /* exact strings */ }
  // Redactor::add(&mut self, bytes: &[u8]) (adds the whole value and each non-empty line as UTF-8 lossy, skipping values shorter than 4 bytes), redact(&self, text: &str) -> String (replaces each with "<secret>", longest first)
  pub fn write_private(path: &Path, bytes: &[u8], overwrite: bool) -> std::io::Result<()>  // output.rs, core (not feature-gated); unix mode 0o600 via OpenOptionsExt; create_new unless overwrite
  ```
- Derive:
  - `SecretFile` on the CLI is a path, or `-` for stdin (through `ParseCx::read_stdin`). It is read at parse time.
  - On JSON it is a path string. With `secret_inline`, it is also `{"inline": "<base64>"}`.
  - `#[op(secret)]` on any field means: no default or env in help, the manual or the schema; the schema gets `writeOnly: true`.
  - `secrets()` returns `#[op(secret)]` values (as their string form) and every `SecretFile`'s bytes.
- `Redactor` lives in `params/mod.rs` and is always compiled (it needs no zeroize), so adapter signatures don't vary with features. Only `Secret` and `SecretFile` sit behind `secrets`.
- Invoke builds a `Redactor` from `P::secrets()`. It wraps `opts.notices` so that text and labels are redacted, and hands the redactor to the adapter alongside the result, through a `pub(crate)` `Invocation` type, so the CLI and MCP adapters redact error messages in Tasks 11 and 14.

- [ ] **Step 1: Write the failing tests.**
  - `secret_debug_prints_placeholder`.
  - `secret_file_reads_path_at_parse`.
  - `secret_file_dash_reads_stdin`.
  - `secret_field_schema_write_only_no_default`.
  - `secret_inline_accepts_base64_object`.
  - `redactor_redacts_value_and_lines`: contents `"line-one-key\nline-two-key"`, and a message containing only `line-two-key` gets redacted.
  - `notice_with_secret_is_redacted`.
  - `write_private_mode_0600_and_no_overwrite`: the second write without `overwrite` fails with `AlreadyExists`, and `#[cfg(unix)]` checks the mode `0o600`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test secrets`.
- [ ] **Step 3: Implement** as specified. With `secrets` off, the derive emits a compile error for a `SecretFile` field ("enable op-registry's `secrets` feature").
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: SecretFile, #[op(secret)], redaction and write_private`.

---

### Task 9: List pairs

Spec: section 5.3, and the gp decisions on lists and the cap.

**Files:**
- Create: `op-registry/src/list.rs`
- Modify: `op-registry/src/registry.rs` (`push_list`), `op-registry/src/invoke.rs`
- Test: `op-registry/tests/lists.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq)] pub struct PageRequest { pub limit: u32, pub cursor: Option<String> }
  #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)] #[serde(rename_all = "camelCase")]
  pub struct Page<T> { pub items: Vec<T>, pub next_cursor: Option<String> }  // items always serialized; nextCursor serialized as null when None
  #[derive(Debug, thiserror::Error)] pub struct TooMany { code: &'static str, cap: usize, op: String }
  // impl CodedError: code() = the leaked "<pillar>.<last noun word>.too_many", class Logical;
  // message: "{op} has more than {cap} items; page with --limit/--cursor"
  impl<C: Resolve> Registry<C> {
      pub fn push_list<P, T, F>(&mut self, spec: OpSpec<P, T>, f: F)
      where P: OpParams + Clone, T: Serialize + OutputSchema + Send + 'static,
            F: for<'a> Fn(&'a C, P, PageRequest) -> BoxFuture<'a, anyhow::Result<Page<T>>> + Send + Sync + 'static;
  }
  ```
- `push_list` registers two entries:
  - `<name>` (kind `ListAll`): params `P`, output `Vec<T>`. It follows pages with `limit = page_size` until `next_cursor` is `None` or empty. When the item count passes `cap`, it fails with `TooMany`. A repeated cursor gives the uncoded "page cursor repeated" error.
  - `<name>_page` (kind `ListPage`): params `P` plus `limit: u32` (required) and `cursor: Option<String>`, output `Page<T>`. The input schema is P's schema plus `limit` (integer, required) and `cursor` (string).
  - Each descriptor's `list_pair` names the other. Both share `OpSpec` metadata.
  - A path without a noun word panics ("list op needs a noun").
  - The registry must have a pillar set, or the push panics ("set_pillar before push_list").
- The `ListPage` JSON decoder splits `limit` and `cursor` off before `P::from_json`. The CLI selection (Task 13) builds the same split.
- Typed markers (used by Task 10): `ListAll`'s Output is `Vec<T>` and `ListPage`'s Params is `PageParams<P> { pub inner: P, pub limit: u32, pub cursor: Option<String> }`.
- `PageParams<P>` has a hand-written `impl<P: OpParams> OpParams for PageParams<P>` in `list.rs`:
  - `fields()` is P's fields plus `limit` and `cursor`.
  - `input_schema()` is P's schema with `limit` (integer, required) and `cursor` (string) added.
  - `add_args` adds `--limit` and `--cursor` to P's args.
  - `from_json` and `from_matches` split off `limit` and `cursor`, then delegate to P.
  - `validate`, `secrets` and (from Task 7) `ref_slots` delegate to `inner`.
  - The `_page` entry is an ordinary entry with params `PageParams<P>`.
  - Test: `page_params_roundtrip_json_and_cli`.

- [ ] **Step 1: Write the failing tests.**
  - `follow_all_returns_bare_array_across_pages`.
  - `follow_all_over_cap_is_too_many`: cap 5 and 3 pages of 3 give the code `fix.concept.too_many`, class Logical, with a message mentioning `--limit`.
  - `follow_all_never_truncates`.
  - `page_op_returns_items_and_next_cursor`: `{"items":[..],"nextCursor":"c2"}`.
  - `empty_list_is_empty_array_and_empty_page`: `[]` and `{"items":[],"nextCursor":null}`.
  - `repeated_cursor_fails`.
  - `pair_names_and_list_pair_links`.
  - `page_schema_requires_limit`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test lists`.
- [ ] **Step 3: Implement** as specified.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: list pairs with follow-all cap and paged op`.

---

### Task 10: The `ops!` macro and typed markers

Spec: section 2.1 (the whole declaration).

**Files:**
- Create: `op-registry-derive/src/ops/{mod,parse,expand}.rs`
- Modify: `op-registry-derive/src/lib.rs` (`#[proc_macro] pub fn ops`), `op-registry/src/lib.rs` (`pub use op_registry_derive::ops;`)
- Test: `op-registry/tests/ops_macro.rs` and `op-registry/tests/ops_ui.rs` + `ops_ui/`

**Interfaces:**
- Consumes: `Registry::{new, set_pillar, group, resolve_server, push_op, push_local, push_stream, push_list, set_call_hook}`, `OpSpec`, `Op`, `Access::from_words` and `coded_error::CodeCatalog::codes()`.
- Grammar:
  ```text
  ops! {
      Ctx = <type>;
      [globals = <type>;]
      [pillar = "<str>";]
      [bin = "<str>"; version = <expr>; about = "<str>";]
      [resolve_server = ["<kind>", ...];]
      [groups = { "<path>" => "<summary>", ... };]
      ( #[doc]* "<path>" => { key: value, ... }, )*
  }
  ```
- Entry keys:
  - `params: T` (required);
  - exactly one of `op:`, `local:`, `stream:`, `list:` (a path);
  - `output: T` (`op`, `local` and `stream` items), or `item: T` (only with `list:`);
  - `access: ident | [ident, ...]` (required);
  - `audience: agent|owner|dev` (required);
  - `errors: [Type, ...]`, `fields: path`, `exits: { N => "meaning", ... }`, `text: bool`, `hidden: bool`, `default: bool`, `paths: [typed]`, `cursor: bool` (stream only), `cap: N`, `page_size: N`, `unpaged: bool` (list only).
- Doc comments: the first paragraph is the summary, and the rest (joined by newlines) is the long help.
- Expands to:
  - `pub fn registry() -> ::op_registry::Registry<Ctx>`. It calls `Registry::new()`, then `set_pillar`, `set_bin(bin, version, about)` (store these three on `Registry` here, with a `bin()` accessor), `resolve_server` and `group` for each group, then pushes each entry in declaration order. `errors` become `[<T as CodeCatalog>::codes(), ...].concat()`.
  - `pub mod ops { ... }`: one unit struct per op, named PascalCase of the op name (`concept_get` → `ConceptGet`), implementing `::op_registry::Op` with `NAME` and the types. A `list:` entry emits `ConceptList` (Output `Vec<Item>`) and `ConceptListPage` (Params `PageParams<P>`, Output `Page<Item>`).
  - `globals = G` is parsed and ignored here. Task 13 adds its compile-time check against `CliContext::Globals`.
  - `exits:` and `text:` fill `OpSpec::exits` and `OpSpec::text` only here. Task 12 sets `ok_exit` and `render_text`.
- Compile errors (one ui test each):
  - an unknown key;
  - a missing `params`;
  - no kind, or two kinds;
  - `item:` without `list:`;
  - `output:` with `list:`;
  - an exit code outside 3..=63;
  - `cursor` on a non-stream;
  - `default: true` on a one-word path.
  Duplicate names and group checks stay runtime panics in `Registry` (Task 2).

- [ ] **Step 1: Write the failing tests.**
  - `ops_macro.rs`: a fixture `ops!` with the spec's three example entries, adapted to fixture types, plus a stream and a local op.
    - `doc_comment_is_summary_and_long_help`.
    - `derived_names_and_marker_types`: `ops::ConceptGet::NAME == "concept_get"`.
    - `call_typed_through_marker_returns_typed_output`.
    - `list_entry_generates_pair_and_markers`.
    - `errors_codes_flattened_from_catalog`: the descriptor's `errors` holds the fixture enum's codes in order, including a `#[coded(transparent, catalog)]` inner code.
    - `access_list_sets_flags`.
    - `local_op_kind_local`.
    - `groups_registered_with_summary`.
    - `resolve_server_kinds_applied`.
  - `ops_ui/`: one case per compile error listed above.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test ops_macro --test ops_ui`.
- [ ] **Step 3: Implement** the parser (`syn::parse::Parse` for the header and entries) and the expansion. Emit all paths as `::op_registry::...`.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: ops! declaration macro with typed op markers`.

---

### Task 11: MCP core

Spec: sections 2.1 (access, hidden), 4.4 (paths), 5.5 (streams) and 8 (tools).

**Files:**
- Rewrite: `op-registry/src/mcp.rs` → `op-registry/src/mcp/mod.rs`
- Modify: op-status and op-listen tests that call `mcp::tool_list` and `call_tool`
- Test: `op-registry/tests/mcp.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
  pub struct ToolSpec { pub name: String, pub description: String, pub input_schema: Value,
                        #[serde(skip_serializing_if = "Option::is_none")] pub output_schema: Option<Value>,
                        pub annotations: ToolAnnotations }
  #[derive(Clone, Debug, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
  pub struct ToolAnnotations { pub read_only_hint: bool, pub destructive_hint: bool }
  pub fn tool_list<C>(registry: &Registry<C>, audience: Audience) -> Vec<ToolSpec>;
  pub async fn call_tool<C: Resolve>(registry: &Registry<C>, ctx: &C, audience: Audience, name: &str, args: Value, opts: CallOptions) -> Result<Value, ToolError>;
  pub fn tool_error(err: &ToolError, source: &str, fallback_message: &str) -> ToolFailure;  // unchanged from v1, plus Redactor applied to message
  ```
- `tool_list` includes ops of that audience that are not hidden, not `Stream` and not `TypedOnly`. Both halves of a list pair are included.
  - The description is the summary, plus a blank line and the long help when present.
  - `read_only_hint = access.reads && !access.writes && !access.destructive`, and `destructive_hint = access.destructive`.
- `call_tool` runs the pipeline with `CallPath::Mcp`.
  - Another audience gives `outside_audience`, a stream gives `wrong_shape` and a typed-only op gives `wrong_path`.
  - A hidden op is callable.
  - `ToolError` carries the call's `Redactor`, and `tool_error` redacts the message with it.

- [ ] **Step 1: Write the failing tests.**
  - `tool_list_filters_hidden_stream_typed_and_audience`.
  - `hidden_op_still_callable_on_mcp`.
  - `annotations_from_access`.
  - `input_keys_camel_including_flattened`.
  - `globals_are_not_tool_params`: a fixture with a flattened group and no globals in the schema.
  - `list_pair_both_tools`.
  - `call_tool_path_is_mcp`: a recording hook.
  - `tool_error_redacts_secret`.
  - `output_schema_none_without_schema_feature` (`#[cfg(not(feature = "schema"))]`; it compiles out under `--all-features`, and the `--no-default-features` check covers it).
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test mcp`.
- [ ] **Step 3: Implement.** Update the op-status and op-listen tests to the new `ToolSpec`.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: MCP tool specs with annotations through the call pipeline`.

---

### Task 12: Output and exit codes

Spec: sections 5.1, 5.2, 5.4, 6.1, 6.2 and 6.5 (CLI).

**Files:**
- Create: `op-registry/src/exit.rs`
- Modify: `op-registry/src/output.rs`, `op-registry/src/invoke.rs` (the ok-exit and text results), `op-registry-derive/src/ops/expand.rs` (wire `exits` and `text`)
- Delete from `op-registry/src/cli.rs`: `render`, `exit_code_for` and `error_line`, and their uses in op-status and op-listen tests
- Test: `op-registry/tests/output.rs`

**Interfaces:**
- Produces:
  ```rust
  pub trait OkExit { fn exit_code(&self) -> u8 { 0 } }
  pub trait RenderText { fn render_text(&self) -> String; }
  #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Style { Pretty, Compact }
  impl Style { pub fn detect(stdout_tty: bool, stderr_tty: bool, force: Option<Style>) -> Style; } // Pretty if either tty, force wins
  pub mod exit { pub const OK: u8 = 0; pub const USAGE: u8 = 2; pub const LOGICAL: u8 = 65;
                 pub const INFRASTRUCTURE: u8 = 69; pub const UNKNOWN: u8 = 70; pub const TRANSIENT: u8 = 75;
                 pub fn exit_code(err: &anyhow::Error) -> u8; pub fn ok_exit(declared: &[Exit], code: u8) -> u8; }
  pub fn render(value: &Value, style: Style) -> String;                    // no trailing newline
  pub fn print_json(out: &mut dyn Write, value: &Value, style: Style) -> io::Result<()>; // value + "\n"
  pub fn error_value(err: &anyhow::Error, redactor: &Redactor) -> Value;  // {"error":{"code","class","message"}}, message = format!("{err:#}") redacted
  pub fn write_error(out: &mut dyn Write, err: &anyhow::Error, redactor: &Redactor, stderr_tty: bool) -> io::Result<()>; // tty: indented; else one compact line
  pub fn notice_line(n: &Notice) -> Value;  // {"notice": "..."} or {"progress": {"done","total","label"}}
  ```
- `exit_code`: a code starting with `op_registry.usage.` or `op_registry.cli.` gives 2. Otherwise SYSEXITS by class (uncoded counts as Unknown, so 70).
- `ok_exit(declared, code)`: 0 stays 0; a declared code in 3..=63 is returned as is; anything else is 70.
- The pipeline (`Invocation`) now carries `exit: u8` and `text: Option<String>`.
  - `exit` is `ok_exit(&descriptor.exits, (spec.ok_exit)(&out))` when `OpSpec::ok_exit` is `Some`, else 0.
  - `text` is `(spec.render_text)(&out)`, computed only when the caller asks (`want_text`) and `render_text` is `Some`. `Descriptor::text` is `spec.render_text.is_some()`.
- `ops!` (in `ops/expand.rs`): `exits:` emits `ok_exit: Some(|o: &O| <O as ::op_registry::OkExit>::exit_code(o))`, and `text: true` emits `render_text: Some(|o: &O| <O as ::op_registry::RenderText>::render_text(o))`. A missing impl is an ordinary trait-bound compile error. Add `ops_ui/ops_exits_without_ok_exit_is_a_compile_error.rs`.

- [ ] **Step 1: Write the failing tests.**
  - `exit_table_values`.
  - `usage_codes_exit_2`: `usage.missing` and `cli.unknown_command`.
  - `classes_map_to_sysexits`.
  - `uncoded_exits_70`.
  - `declared_ok_exit_returned`: 4 is declared and returned.
  - `undeclared_ok_exit_is_70`: an op returning 9 that is not declared gives 70, and 64 gives 70.
  - `style_pretty_if_either_tty_compact_if_both_piped_force_wins`.
  - `error_line_compact_shape`: piped output is one line, parses as JSON, and has `error.code`, `error.class` and `error.message`.
  - `error_line_tty_is_indented`.
  - `error_message_has_context_chain_and_is_redacted`.
  - `notice_and_progress_lines_have_distinct_keys`.
  - `text_rendered_only_when_asked`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test output`.
- [ ] **Step 3: Implement** as specified.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: exit table, ok exits, JSON error and notice lines, text output`.

---

### Task 13: CLI command tree, `OpGlobals`, `CliContext`, parse

Spec: sections 2.1 (hidden, group default), 3.7, 4.1, 5.3 (one CLI command per pair), 5.4, 5.5 (`--cursor`) and 7 (help).

**Files:**
- Create: `op-registry/src/globals.rs`, `op-registry/src/cli/{mod,tree,parse}.rs`, `op-registry-derive/src/globals.rs`
- Delete: `op-registry/src/cli.rs` (the v1 `command`, `dispatch`, `CliOutput` and `CliError` move to `cli/mod.rs` in v2 form)
- Modify: `op-registry-derive/src/ops/expand.rs` (the `globals = G` check), op-status and op-listen tests that used `cli::command` and `cli::dispatch`
- Test: `op-registry/tests/cli_tree.rs`, `ops_ui/ops_globals_mismatch_is_a_compile_error.rs`

**Interfaces:**
- Produces:
  ```rust
  pub trait OpGlobals: Sized + Send + Sync + 'static {
      fn fields() -> Vec<FieldMeta>;
      fn add_args(cmd: clap::Command) -> clap::Command;   // every arg .global(true)
      fn from_matches(m: &clap::ArgMatches) -> anyhow::Result<Self>;
  }
  pub trait CliContext: Sized + Send + Sync {
      type Globals: OpGlobals;
      fn from_globals(globals: &Self::Globals) -> anyhow::Result<Self>;
      fn caller(&self) -> Option<Caller> { None }
  }
  #[derive(OpGlobals)]  // same field rules as OpParams + #[op(env = "...")]; no Ref/Json/Input/flatten
  pub mod cli {
      pub fn command<C: CliContext>(registry: &Registry<C>) -> clap::Command;
      pub enum Parsed { Op { name: String, matches: clap::ArgMatches, style: Option<Style>, text: bool, cursor: Option<Cursor> },
                        Extra { name: String, matches: clap::ArgMatches }, Fallback { args: Vec<OsString> },
                        Help(String), Version(String) }
      pub struct UsageFailure { pub code: &'static str, pub message: String }  // message already secret-safe
      pub fn parse<C: CliContext>(registry: &Registry<C>, argv: &[OsString]) -> Result<(Parsed, C::Globals), UsageFailure>;
      #[derive(Debug, thiserror::Error, coded_error::CodedError)] pub enum CliError { UnknownCommand { name: String } } // op_registry.cli.unknown_command
  }
  ```
  (`Parsed::Extra` and `Parsed::Fallback` are produced from Task 15 on, and `Parsed::Op` covers ops only until then.)
- Tree rules:
  - Root: the name, about and version from `registry.bin()`. `C::Globals` args are added globally, along with the library globals `--pretty` and `--compact` (they conflict with each other). `-h`/`--help` and `--version` come from clap.
  - Groups use their summary as `about`.
  - Leaves use `about = summary` and `long_about = summary + "\n\n" + long_help`.
  - `hidden` → `.hide(true)` (parse still works).
  - `TypedOnly` ops are absent.
  - A group with a `group_default` op gets that op's args on the group command, with `subcommand_required(false)` and `args_conflicts_with_subcommands(true)`. A non-verb word after the group is a usage error.
  - A list pair is one leaf at the `ListAll` op's path, with P's args plus `--limit <N>` and `--cursor <C>`. With either flag given, the parse selects `<name>_page`, using `limit = page_size` when only `--cursor` is given.
  - A stream op gets `--cursor` only when `cursor: true`.
  - A `text: true` op's leaf gets `--text`.
  - Every flag from `#[op(secret)]` is marked internally (`Arg::id` set kept in the registry), so `parse` re-renders clap errors that name a secret arg without the offending value: the message becomes `invalid value for --<flag>`.
  - Clap error mapping follows ruling 9. `--help` and `--version` give `Parsed::Help(rendered)` and `Parsed::Version(..)`.
- At `command`, panic when a library global (`pretty`, `compact`, `text`, `help`, `version`) clashes with a `C::Globals` field.
- Between Task 13 and Task 14 there is no CLI dispatch. Task 13's tests assert only on `Parsed` (op name, flags) and `UsageFailure`. The op-status and op-listen tests that used v1 `cli::dispatch` switch to `Registry::call` (unary) and `Registry::call_stream` (streams) with JSON args. Task 14 moves the ones that test CLI behaviour onto `run_with_io`.

- [ ] **Step 1: Write the failing tests.**
  - `noun_groups_nest_and_verbs_are_leaves`.
  - `one_word_path_is_top_level`.
  - `group_default_runs_with_no_verb`: `build --blueprint x` gives op `build_run`, and `build status` gives `build_status`.
  - `non_verb_after_default_group_is_usage_error`: `build idd` gives a `UsageFailure` with code exactly `op_registry.cli.unknown_command`, and never `build_run`. Clap reports an unexpected argument here, so `parse` maps an unexpected bare word right after a group to `cli.unknown_command`.
  - `hidden_op_absent_from_help_but_parses`.
  - `typed_only_op_absent`.
  - `list_pair_one_command_selects_by_flags`: no flags gives `concept_list`, `--limit 5` gives `concept_list_page`, and `--cursor c` gives page with the default limit.
  - `stream_cursor_flag_only_when_declared`.
  - `text_flag_only_on_text_ops`.
  - `globals_before_or_after_verb`.
  - `global_env_fallback`: set a unique env var name.
  - `pretty_compact_conflict`.
  - `help_from_doc_comments_on_leaf_and_flags`.
  - `secret_value_not_in_clap_error`: an invalid value for a secret `u32` flag gives a message without the value.
  - `negative_and_hyphen_values`: `--count -3` works, and `--note -x` gives a `UsageFailure` (no panic).
  - `usage_missing_flag_code`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test cli_tree --test ops_ui`.
- [ ] **Step 3: Implement.** Add `ops_globals_mismatch_is_a_compile_error`.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: CLI tree from the registry, OpGlobals, CliContext and parse`.

---

### Task 14: The CLI runner

Spec: sections 4.2, 4.3 (trace env), 4.6, 5.1, 5.2, 5.3 and 5.5.

**Files:**
- Create: `op-registry/src/cli/run.rs`, `op-registry/src/cli/io.rs`
- Test: `op-registry/tests/runner.rs`

**Interfaces:**
- Consumes: `cli::parse`, the pipeline's `pub(crate)` CLI entry, `output::*` and `exit::*`.
- Produces:
  ```rust
  pub trait RunHooks {
      fn pre_parse(&self, argv: &[OsString]) { let _ = argv; }
      fn on_usage_error(&self, failure: &UsageFailure) { let _ = failure; }
      fn post_run(&self, outcome: &Outcome) { let _ = outcome; }
  }
  pub struct NoHooks;  // impl RunHooks
  #[non_exhaustive] pub enum Outcome { Ok { op: String, exit: u8 }, Err { op: Option<String>, error: anyhow::Error } }
  pub struct Io { pub stdout: Box<dyn Write + Send>, pub stderr: SharedWriter, pub stdin: Option<Box<dyn Read + Send>>,
                  pub stdout_tty: bool, pub stderr_tty: bool }
  #[derive(Clone)] pub struct SharedWriter(Arc<Mutex<Box<dyn Write + Send>>>);  // impl Write
  impl Io { pub fn std() -> Io; pub fn memory(stdin: &[u8], stdout_tty: bool, stderr_tty: bool) -> (Io, Captured); }
  #[derive(Clone)] pub struct Captured { /* shared buffers */ }  // stdout() -> String, stderr() -> String
  pub async fn run<C: CliContext + Resolve>(registry: &Registry<C>, argv: Vec<OsString>, hooks: &dyn RunHooks) -> ExitCode; // = run_with_io(.., Io::std())
  pub async fn run_with_io<C: CliContext + Resolve>(registry: &Registry<C>, argv: Vec<OsString>, hooks: &dyn RunHooks, io: Io) -> u8;
  pub async fn dispatch<C: CliContext + Resolve>(registry: &Registry<C>, ctx: &C, parsed: Parsed, io: &mut Io) -> Outcome;
  ```
- Run order:
  1. `pre_parse(&argv)`, then `parse`.
  2. On a usage failure: `on_usage_error`, `post_run(Outcome::Err)`, write the error line, return 2.
  3. On help or version: print it to stdout and return 0.
  4. `C::from_globals(&globals)`. An error here gives `Outcome::Err` and exits by class.
  5. Build `CallOptions`: `caller = ctx.caller()`; trace from the env vars `TRACEPARENT`, `TRACESTATE` and `BAGGAGE`; `notices` = a sink writing `notice_line` to `io.stderr` (compact JSON when piped, plain text `notice: ...` or `progress: done/total label` on a tty).
  6. Dispatch with `CallPath::Cli`.
  7. Unary: compute the value, then `post_run`, then print (JSON in the detected `Style`, or text plus `\n` when `--text`), then return the ok exit.
  8. Stream: print each item as one compact line and flush per line. On a mid-stream error, stop, call `post_run`, write the error line, and exit by class.
  9. Error: `post_run` first, then `write_error` (redacted with the call's `Redactor`) as the last stderr write, then `exit_code`.
- No `process::exit`, and no signal handler.
- A `BrokenPipe` from stdout ends the run quietly with exit 0 (ruling 12).

- [ ] **Step 1: Write the failing tests.** Use `Io::memory`, a fixture `CliContext`, and a recording `RunHooks` that logs call order into a `Mutex<Vec<&str>>`.
  - `prints_one_json_value_compact_when_piped`.
  - `prints_pretty_when_stdout_tty`.
  - `pretty_flag_forces`.
  - `text_flag_prints_render_text`.
  - `usage_error_exits_2_and_calls_on_usage_error`.
  - `hook_order_pre_parse_post_run_then_error_line`: the hook writes `"log text"` to the shared stderr in `post_run`, and the last stderr line is the error JSON.
  - `logical_error_exits_65_with_error_last_line`.
  - `declared_ok_exit_4`.
  - `stream_prints_ndjson_lines`.
  - `stream_mid_error_ends_with_error_line_and_class_exit`.
  - `follow_all_list_prints_bare_array`.
  - `limit_prints_items_object`.
  - `traceparent_env_reaches_call_info`: use a unique env var setting and restore it after.
  - `notice_lines_on_piped_stderr_are_json`.
  - `context_from_globals_error_exits_by_class`.
  - `local_op_never_calls_connect`: a fixture context whose `connect()` panics. This is a sanity check only; the full check is Task 19.
  - `secret_never_in_error_line`.
  - `broken_pipe_on_stdout_exits_0_quietly`: the stdout writer returns `ErrorKind::BrokenPipe` and stderr stays empty.
  - `help_exits_0`.
  - `caller_from_context_reaches_op`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test runner`.
- [ ] **Step 3: Implement** as specified.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: cli::run with run hooks, NDJSON streams and the last-line error`.

---

### Task 15: Extras and fallbacks

Spec: sections 2.3 and 2.4, and the "Help under a fallback" decision.

**Files:**
- Create: `op-registry/src/cli/extra.rs`, `op-registry/src/cli/fallback.rs`
- Modify: `op-registry/src/cli/{tree,parse,run}.rs`, `op-registry/src/registry.rs`
- Test: `op-registry/tests/fallback.rs`

**Interfaces:**
- Produces:
  ```rust
  impl<C: CliContext> Registry<C> {
      pub fn push_extra(&mut self, command: clap::Command,
          run: for<'a> fn(&'a C::Globals, &'a clap::ArgMatches) -> BoxFuture<'a, u8>);
      pub fn fallback_flags_only(&mut self, name: &'static str, help: &'static str,
          run: fn(Vec<OsString>) -> BoxFuture<'static, u8>);
      pub fn fallback_free_text(&mut self, name: &'static str, help: &'static str, flags: &'static [&'static str],
          run: for<'a> fn(&'a C::Globals, Vec<OsString>) -> BoxFuture<'a, u8>);
  }
  // stored privately as enum Fallback<G> { FlagsOnly{..}, FreeText{..} } inside Registry<C> as Fallback<C::Globals>;
  // the registry keeps the fallback and extras in fields typed by C::Globals via a private trait object, never Any.
  // At most one fallback; setting a second panics.
  pub fn edit_distance(a: &str, b: &str) -> usize;  // cli::fallback, Levenshtein, pub for tests
  ```
- Extras:
  - The command is added at the root and gets the registry globals, with env fallback.
  - `run` gets the parsed globals and the extra's own matches.
  - `push_extra` panics when the command name equals an op or group word ("extra `serve` clashes with op word").
  - The globals type is checked at compile time through `C::Globals`, so there is no runtime type check.
  - Extras and fallbacks return `u8`, because `ExitCode` is opaque. `run` converts with `ExitCode::from`.
- FlagsOnly:
  - Empty args, or a first arg starting with `-`, go to the fallback with the raw args.
  - A first bare word that is not a command or group is `cli.unknown_command`, exit 2.
  - `-h`, `--help` and `--version` as the first token always show the registry's help or version.
- FreeText:
  1. Skip leading registry globals and their values.
  2. Then: `--` sends everything after it to the fallback.
  3. A `-` token that is not a registry global goes to the fallback.
  4. A command or group word runs the command.
  5. A single token with no whitespace within edit distance ≤ 2 of a command or group word gives `cli.unknown_command`, with the message "unknown command `<tok>`; did you mean `<word>`?".
  6. Anything else goes to the fallback with the parsed globals.
- `help <fallback-name>` prints the fallback's help. The root help lists the fallback (`after_help`: "<name>: <help first line>").
- At `cli::command`, panic when a FreeText fallback's declared `flags` reuse a registry global's flag name.

- [ ] **Step 1: Write the failing tests.**
  - `extra_gets_globals_and_matches`.
  - `extra_env_fallback`.
  - `extra_clashing_with_op_word_panics`.
  - `flags_only_empty_and_leading_flag_go_to_fallback`.
  - `flags_only_unknown_bare_word_is_usage_2`: `idd` gives exit 2, and the fallback is not run.
  - `help_and_version_never_reach_fallback`: both FlagsOnly and FreeText.
  - `help_fallback_name_prints_its_help`.
  - `root_help_lists_fallback`.
  - `free_text_double_dash_escape`: `-- log` gives a fallback with `["log"]` even though `log` is a command.
  - `free_text_unknown_dash_flag_goes_to_fallback`: `--codebase X fix it`.
  - `free_text_globals_parsed_then_fallback`.
  - `free_text_command_word_runs_command`.
  - `free_text_typo_is_usage_error`: `bulid` with group `build` gives exit 2, with a message containing "build".
  - `free_text_far_word_goes_to_fallback`: `refactor`.
  - `free_text_sentence_never_matches`: `"buld it"` goes to the fallback.
  - `free_text_flag_clash_panics`.
  - `edit_distance_cases`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test fallback`.
- [ ] **Step 3: Implement** as specified, with `u8` exit returns as described.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: extra commands and FlagsOnly/FreeText fallbacks`.

---

### Task 16: Manual

Spec: sections 6.3 (the manual lists codes) and 7.

**Files:**
- Create: `op-registry/src/manual.rs` (`#[cfg(feature = "manual")]`)
- Modify: `op-registry/src/registry.rs` (`with_manual`), `op-registry/src/invoke.rs` (entries' internal closures receive `&Registry<C>`)
- Test: `op-registry/tests/manual.rs`, `op-registry/tests/manual_snapshot.md`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, Default)] pub struct Manual { pub front_matter: Option<FrontMatter>, pub sections: Vec<Section> }
  #[derive(Clone, Debug)] pub struct FrontMatter { pub name: String, pub description: String }
  #[derive(Clone, Debug)] pub struct Section { pub title: String, pub guides: Vec<Guide> }
  #[derive(Clone, Debug)] pub struct Guide { pub title: String, pub body: String, pub commands: Vec<String> } // command paths, e.g. "concept get"
  pub fn render<C: CliContext>(registry: &Registry<C>, manual: &Manual) -> anyhow::Result<String>; // Err if a guide names a missing path
  pub fn render_json<C: CliContext>(registry: &Registry<C>) -> serde_json::Value;
  impl<C: CliContext + Resolve> Registry<C> { pub fn with_manual(&mut self, manual: Manual, audience: Audience); }
  #[derive(OpParams)] pub struct ManualParams {
      /// Write the Markdown manual to this file instead of returning it.
      pub out: Option<PathBuf>,
      /// Return every descriptor, schema, exit and code as JSON.
      pub json: bool }
  #[derive(Serialize, JsonSchema)] #[serde(untagged)]
  pub enum ManualOutput { Markdown { markdown: String }, Written { written: PathBuf }, Json(serde_json::Value) } // impl RenderText: Markdown → the text, others → compact JSON
  ```
- Markdown layout, in this exact order:
  1. Optional `---\nname: ..\ndescription: ..\n---\n\n`.
  2. `# <bin>` and the `about` text.
  3. `## Exit codes`: a table `| Exit | Meaning |` with rows 0 ok, 2 usage, 65 logical, 69 infrastructure, 70 unknown, 75 transient.
  4. One `## <section title>` per section, with `### <guide title>`, the body, and `Commands: `a`, `b``.
  5. `## Commands`: per visible CLI command (hidden and typed-only left out, list pair shown once), `### <bin> <path>`, then a ```` ```text ```` block with the leaf's `render_long_help()`, then `Access: reads, network`, then (if any) `Exits:` lines `- 4: records drifted`, then (if any) a table `| Code | Class | Exit |`. Class `None` prints `varies` with exit `varies`. List ops also list `<pillar>.<noun>.too_many` (logical, 65).
  6. `## Library codes`: every `op_registry.*` code with class and CLI exit (2 for usage and cli codes).
- `render_json`: `{"bin","version","exits":[{"code","meaning"}],"ops":[descriptor JSON + "codes":[{code,class,exit}]]}`. Output schemas are present only with `schema`.
- The `manual` op (present only after `with_manual`) is an ordinary registry entry:
  - Path `manual`, kind `Local`, access `[reads, writes]`, the given audience, `text: true`, summary "Show this tool's manual.".
  - Typed marker `op_registry::manual::ManualOp` (`NAME = "manual"`).
  - It reaches the CLI, MCP and the typed path through the one pipeline.
  - It needs the whole registry, so the pipeline's internal entry closure gets `&Registry<C>` as an extra argument. User ops' closures ignore it, and only `with_manual`'s closure uses it to call `render` and `render_json` at call time, so ops pushed or merged later are included.
  - `--json` returns `ManualOutput::Json(render_json(..))`.
  - `--out <path>` writes the Markdown (creating parent dirs) and returns `Written`.
  - With neither, it returns `Markdown`, and on the CLI `--text` prints it raw.
  - The manual lists itself like any op.
  - With the `manual` feature off, `with_manual`, `ManualParams` and `ManualOutput` do not exist.
- Help text never includes env values (`hide_env_values(true)` on every arg with env).

- [ ] **Step 1: Write the failing tests.**
  - `manual_matches_snapshot`: compare `render` of the fixture registry with `tests/manual_snapshot.md`. Write that file by hand in this step from the layout below, for the fixture's two ops, one group, one guide, one declared exit, a `varies` code and a list pair. It ships in the commit, so the first run fails on the missing render rather than writing it. `UPDATE_SNAPSHOTS=1` rewrites it later.
  - `missing_guide_path_is_error`.
  - `front_matter_rendered`.
  - `codes_listed_with_class_and_exit`, including `varies`.
  - `too_many_listed_for_list_ops`.
  - `hidden_and_typed_only_not_in_manual`.
  - `manual_op_out_writes_file`: through the CLI via `run_with_io`.
  - `manual_json_has_ops_and_codes`.
  - `manual_is_an_op_on_every_path`: `manual` is in `tool_list(audience)`, `call_tool` returns `{"markdown": ..}`, and `call_typed::<ManualOp>` works.
  - `manual_includes_ops_merged_after_with_manual`.
  - `help_never_shows_env_value`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --test manual`.
- [ ] **Step 3: Implement** as specified.
- [ ] **Step 4: Run the tests and the green set.** Also run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo check -p op-registry --no-default-features --features manual`, which is gait-min's set.
- [ ] **Step 5: Commit.** `op-registry v2: generated manual and the manual op`.

---

### Task 17: rmcp adapter

Spec: section 8 (rmcp).

**Files:**
- Create: `op-registry/src/mcp/rmcp.rs` (`#[cfg(feature = "rmcp")]`)
- Test: `op-registry/tests/rmcp.rs` (`#![cfg(feature = "rmcp")]`), with dev-dependency `rmcp = { workspace = true, features = ["server", "client"] }` plus tokio `io-util` for an in-memory duplex

**Interfaces:**
- Produces:
  ```rust
  pub struct RegistryServer<C> { /* Arc<Registry<C>>, Arc<C>, Audience, rmcp::model::Implementation */ }
  impl<C: Resolve + Send + Sync + 'static> RegistryServer<C> {
      pub fn new(registry: Arc<Registry<C>>, ctx: Arc<C>, audience: Audience, info: rmcp::model::Implementation) -> Self;
  }
  impl<C: Resolve + Send + Sync + 'static> rmcp::ServerHandler for RegistryServer<C> { /* get_info, list_tools, call_tool */ }
  ```
- `list_tools` maps `tool_list(audience)` to `rmcp::model::Tool`, with annotations and an output schema.
- `call_tool` runs through `call_tool`:
  - Success gives `CallToolResult` with `structured_content = value` and a text block holding the compact JSON.
  - An error gives `is_error = true` with structured content `{"error":{"code","class","message","fingerprint"}}` from `tool_error(.., "mcp", "internal error")`.
  - `_meta` keys `traceparent`, `tracestate` and `baggage` go into `CallOptions::trace`. The caller stays `None`, because identity is the app's.
  - `Notices` map to the request peer's `notify_progress` (when the request carried a progress token) and `notify_logging_message` (for text).
- Pin the rmcp version `1`, the same major archegraph, catuskoti and grorg use. The code follows whatever `rmcp` 1.x's `ServerHandler` exposes. The implementer reads the rmcp 1.x docs (docs.rs) for the method names.

- [ ] **Step 1: Write the failing tests.** Use a tokio duplex with an rmcp client against `RegistryServer`.
  - `lists_audience_tools_with_annotations`.
  - `call_returns_structured_content`.
  - `error_is_tool_error_with_code`.
  - `meta_traceparent_reaches_call_info`: a recording hook.
  - `progress_notice_reaches_client`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --features rmcp --test rmcp`.
- [ ] **Step 3: Implement** as specified.
- [ ] **Step 4: Run the tests and the green set.** Also run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo tree -p op-registry -e normal --no-default-features` and confirm that no rmcp, schemars or zeroize appears.
- [ ] **Step 5: Commit.** `op-registry v2: rmcp adapter serving one audience`.

---

### Task 18: Testing kit I: parity, canonical, casing, help

Spec: section 9, rows `assert_parity`, `assert_canonical`, `assert_casing` and `assert_help_complete`, and section 5.3 (warnings).

**Files:**
- Create: `op-registry/src/testing/{mod,sample,parity,help,casing}.rs` (`#[cfg(feature = "testing")]`)
- Delete: `op-registry/src/parity.rs` (v1), and update its uses in op-status and op-listen tests
- Modify: `op-registry/Cargo.toml` (`[[test]] name = "testing_kit"` with `required-features = ["testing"]`)
- Test: `op-registry/tests/testing_kit.rs`

**Interfaces:**
- Produces (each `assert_*` panics with a full multi-line report of every problem, never only the first):
  ```rust
  pub struct Mount<'a> { pub name: &'a str, pub audience: Audience, pub ops: &'a [&'a str] }
  pub fn assert_parity<C: CliContext>(registry: &Registry<C>, mounts: &[Mount<'_>]);
  pub fn parity_report<C: CliContext>(registry: &Registry<C>, mounts: &[Mount<'_>]) -> Vec<ParityLine>; // name, path, Reachable | Absent{reason}
  pub fn assert_canonical<C>(registry: &Registry<C>, canonical: &[&str]);   // set equality on names; report missing and extra
  #[derive(Clone, Copy)] pub enum Casing { Camel }
  pub fn assert_casing<C>(registry: &Registry<C>, casing: Casing);          // input + output schema property keys, recursive; map additionalProperties skipped
  pub struct HelpRules<'a> { pub banned_words: &'a [&'a str] }
  pub fn assert_help_complete<C: CliContext>(registry: &Registry<C>, rules: &HelpRules<'_>) -> Vec<String>; // returns warnings
  pub fn sample_params(schema: &Value) -> Value;  // minimal valid JSON for a derived input schema
  ```
- Parity checks, for every op that is not `TypedOnly`:
  - It is on the CLI at its path (list pairs share one command).
  - It is in `tool_list(audience)`, unless it is a `Stream` (reason "stream") or hidden. A hidden op is checked structurally, with no call and no context: its descriptor is `Paths::All`, its kind is not `Stream`, and `registry.get(name)` finds it. Those are the same conditions `call_tool` uses (Task 11), so a hidden op that passes is callable on MCP.
  - It is in every mount of its audience.
  - Every extra is listed as CLI-only with "not an op".
  - Every mount name not in the registry is a problem.
  - A `TypedOnly` op present on the CLI or MCP is a problem.
- Help checks: every visible op, group and flag has non-empty help, and one flag name has one wording across all ops. The help text must not contain:
  - `$HOME` (`std::env::var("HOME")`), `/Users/` or `/home/`;
  - any banned word, matched case-insensitively.
- Help warnings (returned, not panics): a verb `list` whose op is not a list op and not `unpaged`; a list pair whose item schema differs from a sibling `get`'s output schema.
- `sample_params` fills required properties: string → `"x"`, integer → 1, number → 1.0, boolean → false, enum → first, array → `[]`, object map → `{}`, `oneOf` → the first branch's required, `dependentRequired` honoured.

- [ ] **Step 1: Write the failing tests.**
  - `parity_passes_for_fixture`.
  - `parity_reports_missing_mount_op`.
  - `parity_flags_typed_only_on_mcp`: build by hand through a mount listing it.
  - `parity_lists_stream_reason_and_extra_reason`.
  - `canonical_reports_missing_and_extra`.
  - `casing_flags_snake_output_key`: an output struct without `rename_all`.
  - `casing_ignores_map_keys`.
  - `help_flags_missing_doc`.
  - `help_flags_two_wordings_for_one_flag`.
  - `help_flags_home_path_and_banned_word`: `"daemon"`.
  - `help_warns_list_verb_not_list`.
  - `help_unpaged_silences_warning`.
  - `help_warns_item_differs_from_get`.
  - `sample_params_satisfies_from_json` for every fixture op.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --all-features --test testing_kit`.
- [ ] **Step 3: Implement** as specified, and delete the v1 `parity` module.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: testing kit parity, canonical, casing and help checks`.

---

### Task 19: Testing kit II: snapshot, exit table, error codes, secrets, not_found, offline

Spec: section 9, the remaining rows, and section 6.3 (the strict code check in tests).

**Files:**
- Create: `op-registry/src/testing/{snapshot,codes,secrets,refs,offline}.rs`
- Modify: `op-registry/src/invoke.rs` (a `#[cfg(feature = "testing")]` strict-code check), `op-registry/src/registry.rs`
- Test: `op-registry/tests/testing_kit.rs` (extend)

**Interfaces:**
- Produces:
  ```rust
  pub fn assert_manual_snapshot<C: CliContext>(registry: &Registry<C>, manual: &Manual, path: &Path); // UPDATE_SNAPSHOTS=1 rewrites
  pub fn assert_exit_table<C: CliContext>(registry: &Registry<C>, manual: &Manual); // table rows, every declared exit and code present in render(); exits in 3..=63
  pub fn strict_codes<C>(registry: &mut Registry<C>);  // after this, a call returning a code outside the op's errors ∪ op_registry.* ∪ its too_many panics naming op and code; uncoded errors panic too
  pub struct Fixture<'a> { pub op: &'a str, pub args: Value }
  pub async fn assert_error_codes<C: Resolve>(registry: &Registry<C>, ctx: &C, fixtures: &[Fixture<'_>]); // runs each via call(); reports every undeclared code
  pub struct SecretSentinel<'a> { pub value: &'a str, pub key_file: &'a Path }  // key_file holds the sentinel contents
  pub async fn assert_secrets_not_echoed<C: CliContext + Resolve>(registry: &Registry<C>, manual: Option<&Manual>, sentinel: &SecretSentinel<'_>);
  pub async fn assert_unknown_ref_not_found<C: Resolve>(registry: &Registry<C>, ctx: &C, overrides: &[Fixture<'_>]);
  pub async fn assert_local_ops_offline<C: Resolve>(registry: &Registry<C>, ctx: &C, overrides: &[Fixture<'_>]);
  ```
- `assert_secrets_not_echoed` checks the sentinel value and the key file's contents (and each of its lines) are absent from:
  - every `--help`, the manual, and every input and output schema;
  - a clap error from an invalid value on each secret flag;
  - the error line and the notices of a CLI run per op with secret fields. Use `sample_params` → argv, with secret fields set to the sentinel and `SecretFile` fields set to the key file, through `run_with_io` and `Io::memory`;
  - stdout of that run.
- `assert_unknown_ref_not_found`: for each op with a subject, it builds `sample_params` (or the override) with `id` set to a random 16-hex ref, calls it on the JSON path, and asserts `Err` with class Logical and a code ending in `.not_found`.
- `assert_local_ops_offline`: runs each `Local` op with `sample_params` (or the override) on the given ctx. Whether the op returns `Ok` or `Err` doesn't matter. A connector panic in the app's ctx fails the test, and the report names the op.

- [ ] **Step 1: Write the failing tests.**
  - `snapshot_mismatch_reports_and_update_rewrites`: use a temp path and call `pub fn check_manual_snapshot(registry, manual, path, update: bool) -> Result<(), String>`. `assert_manual_snapshot` wraps it by reading `UPDATE_SNAPSHOTS`.
  - `exit_table_check_passes_and_flags_missing_code`.
  - `strict_codes_panics_on_undeclared_code`.
  - `error_codes_report_lists_offender`.
  - `secrets_sentinel_never_echoed_passes`.
  - `secrets_check_catches_leaky_op`: an op that puts the secret in its output fails.
  - `unknown_ref_check_passes_with_resolver_and_fails_with_null_returning_op`.
  - `local_offline_catches_connect`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-registry --all-features --test testing_kit`.
- [ ] **Step 3: Implement** as specified.
- [ ] **Step 4: Run the tests and the green set.**
- [ ] **Step 5: Commit.** `op-registry v2: testing kit snapshot, exit table, codes, secrets, not_found and offline checks`.

---

### Task 20: op-status and op-listen on v2 conventions; docs

Spec: section 5.1 (the library's own types go camelCase in the same release), plus the README and CHANGELOG.

**Files:**
- Modify: `op-status/src/types.rs` (`#[serde(rename_all = "camelCase")]` on every struct; enum variant values keep their current rename), `op-status/src/ops.rs`
- Modify: `op-listen/src/frame.rs` (`rename_all_fields = "camelCase"` on `ListenFrame`; the `frame` tag values stay `snapshot`/`item`/`stop`), `op-listen/src/entry.rs`
- Modify: op-status and op-listen tests and goldens (`op-listen/tests/frame_golden.rs`, `follow_golden.rs`) to the camelCase wire
- Modify: `README.md`, `op-status/README.md`, `op-listen/README.md` (v2 quick start with `ops!`), `CHANGELOG.md` (a 0.3.0 entry listing every removal from Global Constraints and every new piece)
- Test: `op-status/tests/casing.rs`, `op-listen/tests/casing.rs`

**Interfaces:**
- Consumes: `testing::assert_casing` and `testing::assert_parity`.
- `status_registry` and `listen_entry` keep their v2 signatures from Tasks 2 and 7.

- [ ] **Step 1: Write the failing tests.**
  - `status_registry_is_camel_case`: `assert_casing(&status_registry::<Fixture>(..), Casing::Camel)`.
  - `snapshot_serializes_taken_at_unix_ms`: the JSON has key `takenAtUnixMs`.
  - `listen_frame_is_camel_case`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo test -p op-status --features listen,op-registry/testing --test casing`, and the same for op-listen with `--features testkit,op-registry/testing`.
- [ ] **Step 3: Implement** the renames, update the goldens, and write the README and CHANGELOG.
- [ ] **Step 4: Run the tests and the green set.** Also run `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo clippy -p op-registry --all-features -- -D warnings` and the same for `-p op-registry-derive`, `-p op-status --features listen` and `-p op-listen --features testkit`.
- [ ] **Step 5: Commit.** `op-status, op-listen: camelCase wire; v2 docs and changelog`.

---

## Later (not in this plan)

- The `dynamic` feature: `push_dynamic(descriptor, input_schema, call)` for runtime ops (structmux daemon catalog from proto descriptors). Spec 8 marks it optional and later.
- A `rest` feature with an axum router (spec 8: "can come later").
- Consumer adoption jobs (grorg, archegraph, catuskoti, multimatrix, gait, structmux, empeiria), each in its own repo, upstream first.
