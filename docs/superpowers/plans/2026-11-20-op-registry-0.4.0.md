# op-registry 0.4.0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship op-registry 0.4.0 (all four crates): the one breaking release that lets grorg, gait, multimatrix, structmux, archegraph and concordium-app port onto v2 without hand-written workarounds, with gp's 2026-11-20 decisions as library behaviour.

**Architecture:** No new crates. Changes land in the existing pipeline (`op-registry/src/invoke.rs`), the derive (`op-registry-derive/src/params/*`, `ops/*`), the CLI runner (`op-registry/src/cli/*`), the MCP adapter, the testing kit (`op-registry/src/testing/*`) and op-listen/op-status. One new module, `op-registry/src/shape.rs`, owns output roots (`{"items"}` wrap, `$defs` hoist, object-root classing). Tasks 1-19 front-load what the archegraph pilot needs and every cheap breaking type change; the commit closing Task 19 is the **pilot commit**.

**Tech Stack:** Rust 2021 (rust-version 1.89), clap 4, serde/serde_json (now with `preserve_order`), anyhow, thiserror 2, futures-util, coded-error (pin unchanged), syn 2/quote. Optional: schemars 1 (`schema`), zeroize 1 (`secrets`), rpassword 7 (`prompt`, new), rmcp 1 (`rmcp`), tokio `time` (`testing`, new). Dev only: jsonschema (new), tokio `test-util`.

**Spec:** `docs/superpowers/specs/2026-11-20-op-registry-0.4.0-design.md` (b4c3346). Binding. Its last two sections, "Orchestrator rulings" and "Lead corrections", override the body; the errata below list every spot where the body still says otherwise. Each task names the change ids (DEC-1, REL-3, ...) it carries; read those spec sections before starting the task.

## Spec errata (the body is wrong here; follow this)

1. **`new` is `absent_ok`** (gait lead ruling). Everywhere the body says `#[op(subject = "..", new)]` (section 2 decision 3, PAR-1, PAR-4, PAR-11, KIT-3, section 5) read `#[op(subject = "..", absent_ok)]`. `SubjectMeta` is `{ kind: String, id_only: bool, absent_ok: bool }`. `absent_ok` **may** be combined with `id_only`: drop PAR-4's derive error "`new` already passes the id through". `absent_ok` on a non-subject stays a derive error. PAR-4's effect is unchanged: an `absent_ok` subject is never resolved, the op gets `Resolved::Id(raw)`, `check_id` runs, and the kit's not_found test skips it.
2. **PAR-10 has no panic** (ruling b, grorg correction). A param `env` is read on the CLI only. An op with an `env` param may be on any paths; there is no build error and no push-time panic. Drop the test "a registry that puts such an op on mcp panics"; the test is "env is ignored off the CLI; the value must be passed".
3. **Stops and Position frames** (multimatrix correction). A follower decides a stop only through `Resumable::stop_code()`; the library never treats a frame as terminal by its shape for a consumer frame type. `FollowOptions::stop_codes` covers mid-stream **errors** only; a stop arrives as a frame. A frame whose `cursor()` is `Some` and `stop_code()` is `None` (multimatrix's Position frame) is yielded as a frame and advances the resume cursor; it is never dropped and never turned into a snapshot. Section 6's "`stop_codes = [multimatrix.access.not_found]`" and "`Position` frames as snapshots or dropped" are superseded (consumer work, no library change).
4. **`follow_stream` retries a failed open** as Transient, up to `max_retries`. This is a behaviour change for consumers that never retried an open (multimatrix `agent-room`); it goes in the CHANGELOG's behaviour list.
5. **SEC-8:** "No consumer has such an op today" is wrong: gait's `auth add --key-file` and `auth login --callback-file` are plain `SecretFile` ops. gait declares them `paths: [cli]`, so they pass the parity rule.
6. **SEC-3 is tests only** (gait correction: F2 and F3 need no library change). `Redactor::add` already registers the whole value and each trimmed line, which covers the trimmed whole value. Do not add a new candidate; Task 25 adds the SEC-3 tests that pin the behaviour.
7. **grorg fleet codes** (ruling f, grorg correction): section 6's "`grorg.fleet.*` kept" is superseded by per-object areas. Consumer work; no library change. Likewise ruling (e) (`room messages --all` is a stream) is consumer work.
8. **Build order deviations from section 8**, all forced by the orchestrator's ordering or by test dependencies:
   - KIT-2 lands first (Task 1), before PATH-1 and KIT-1.
   - OUT-6 moves early (Task 7) so the pilot's goldens move once.
   - MAN-4 lands with PATH-1 (Task 2), since PATH-1's tests check the manual.
   - OUT-7 lands before SEC-1 (Tasks 17 and 25), because SEC-1's tests assert on typed `CallFailure`s. SEC-2's `CallOptions::redact` lands with SEC-1, because SEC-1's tests call it.
   - SEC-8's parity check lands with KIT-6 (Task 19).
   - ERR-3's library-code de-duplication lands with the first shared code (Task 11).

## Global Constraints

- **Builds:** every cargo command goes through `/Users/aicoder/Documents/systematic-action/scripts/cargo-slot cargo ...`. In the Run lines below, `cargo-slot` means that full path. Per task, run only the scoped test targets named in the task plus `cargo-slot cargo check -p <each crate touched> --all-features --tests` (every test target of a touched crate must still compile). The full gates run once, in Task 34: `cargo test --workspace --all-features`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo fmt --check`.
- **Tests first** in every task: write the failing test, run it red, implement, run it green.
- **Release:** all four crates go to 0.4.0 (workspace `version` and every path-dependency `version`). CHANGELOG lists every breaking signature (spec section 7, items 1-16, with the errata applied). README and the v2 spec are updated where behaviour changed.
- **No auth, identity or signing** in op-registry. `Caller` stays an opaque label. The secrets work is redaction and prompts only.
- **Dependencies:** `rpassword` 7 is optional, behind feature `prompt`, which is in `default`. New dependencies must be permissive-licensed (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode); confirm the license of the exact version from its crate metadata before adding it. `jsonschema` is a dev-dependency only. `testing` gains tokio `time` only: never tokio `rt` outside op-listen's `detach` (scripts/tokio-rt-guard.sh).
- **No backward compatibility:** no shims, dual reads or deprecated aliases. When a task removes a mechanism, it deletes that mechanism's tests and UI fixtures in the same task.
- `op-registry --no-default-features --features manual` must still compile (gait-min).
- JSON keys camelCase on every JSON path; long kebab-case flags; exit table 0/2/65/69/70/75, declared exits 3..=63 (unchanged).
- **Git:** stage files by name (`git add <paths>`), never `-a`/`-A`; never push. Commit messages end with the executing harness's attribution line and carry no `Claude-Session` trailer.
- **Library tests that relied on vacuous kit passes** (Tasks 3, 18, 19) get real fixtures in the same task; never weaken a kit check to keep an old test green.

## Review Focus

- **The shared Redactor reaching every output site (Task 25).** A secret the op registers with `opts.redact` mid-call must be gone from: the CLI error line after the ERR-2 mapper rewrote the error (the mapped message embeds the secret); a stream item error on the CLI; a notice sent after the registration through a typed caller's own sink; and an `opts` clone the op handed to a helper before registering (clones share the handle). Tests in Task 25: `mapped_error_is_redacted`, `stream_item_error_on_cli_is_redacted`, `typed_caller_sink_notice_is_redacted`, `options_clone_shares_redactor`.
- **The `{"items"}` wrap with `$defs` hoisting (Task 6).** A recursive item type (`Node { children: Vec<Node> }`), a `Vec<Vec<Named>>` output and a schema keyed `definitions` (draft-07) must all keep every `$ref` resolvable after the wrap; an `Option<Vec<T>>` output (root `type: ["array","null"]`) is not wrapped, prints `null` or `{"items"}` by value, and the kit reports its root as non-object. Tests in Task 6: `recursive_item_refs_resolve`, `nested_vec_refs_resolve`, `definitions_keyed_schema_hoisted`, `optional_vec_root_not_wrapped`; Task 19: `optional_vec_output_fails_parity_root`.
- **An env variable set to the empty string** (`GAIT_DIR=`, `GRORG_OWNER_PRINCIPAL=`) counts as absent: a needed global is still missing (exit 2), an env param is still absent. Tests: Task 16 `empty_env_param_is_absent`, Task 23 `empty_env_does_not_meet_need`.
- **A prompt answered with an empty line.** A prompted `SecretFile` gives `op_registry.usage.missing` with "(empty answer)" and the op never runs; `prompt_secret` returns `None`. Tests in Task 28: `prompt_empty_answer_is_missing`, `prompt_secret_empty_answer_is_none`.
- **A follower whose connections open and end with no item.** No marker line is printed for a connection that yielded nothing; each such connection counts toward `max_retries`; the kit's `max_retries = Some(0)` ends it after one. Tests in Task 31: `empty_connections_print_no_marker`, `empty_connections_count_toward_max_retries`.

---

## File structure (what changes)

`op-registry/src/`:
- `shape.rs` (new): `wrap_items`, `wrap_array_schema`, `RootClass`, `root_class`, `mark_object_root`.
- `descriptor.rs`: `Surface`, `Paths` set, `Exit.on`, `Descriptor.{audiences, needs_globals, relations}`.
- `call.rs`: `CallOptions.{max_retries, redact, prompt_secret}`, `CallFailure: CodedError`, `into_coded`.
- `params/mod.rs`: shared `Redactor`, `FieldMeta`/`FieldKind`/`SubjectMeta`, new `UsageError` variants, public `usage`, `OpParams::relations`.
- `params/rt.rs`: `Presence`, `present_schema`, `SchemaBuilder` constraints, `from_str` and `ignore_case` helpers.
- `refs.rs`: `Resolve::check_id`.
- `registry.rs`: `OpSpec` new fields, `add_common_errors`, `set_error_mapper`, `describe` (wrap, object roots).
- `invoke.rs`: path check on typed, error mapper, one redactor per call, `call_stream_typed_owned`.
- `list.rs`: one-word nouns, `follow_page`, `ListError`, empty-page counting, list text.
- `output.rs`: `RenderText for Vec<T>`, durable `write_private`.
- `cli/*`: needs check, `select`, `Selected`, `on_selected`, `FallbackFlag`, FreeText routing, extras' `Result<u8>`, error exits, prompts, stdin hint, `Io` fields, `Prompter`.
- `mcp/*`: audiences, ctx factory, instructions, object-root output.
- `manual.rs`: paths line, off-CLI section, common codes, other commands, rules.
- `testing/*`: bounded streams, surface choice, seeded sampling, temp dirs, `SecretSentinel`, exact parity.

`op-registry-derive/src/`: `ops/parse.rs`, `ops/expand.rs` (paths, audiences, `follow_page`, `text` on lists, `#[cfg]`, `globals:`, header `errors`, error exits); `params/*` (relations, presence, `required` on Vec/Option, struct-level attrs, `absent_ok`, optional and `Patch` refs, `from_str`, `sample`, `allow_hyphen`, `ignore_case`, default display, `env`, `prompt`); `globals.rs` (`env` in `FieldMeta`, `from_str`).

`op-listen/src/`: `follow.rs` (`stop_codes`, `supervise_stream`, `follow_stream`, `Followed`), `frame.rs` (`Resumable` impl), `entry.rs` (`listen_entry_following`), `listener.rs` (two defaulted methods), `error.rs` (`Stopped`), `lib.rs` and `Cargo.toml` (`registry` feature). `op-status/src/`: `source.rs`, `ops.rs`, `lib.rs`.

---

# Part A: pilot features (Tasks 1-19)

### Task 1: KIT-2, bounded stream calls in the kit

Spec: KIT-2; STR-4's `CallOptions::max_retries`.

**Files:**
- Modify: `op-registry/Cargo.toml` (`testing = ["manual", "dep:tokio", "tokio/time"]`; dev tokio adds `test-util`), `op-registry/src/call.rs`, `op-registry/src/testing/mod.rs`, `op-registry/src/testing/codes.rs`, `op-registry/src/testing/offline.rs`, `op-registry/src/testing/refs.rs`
- Test: `op-registry/tests/testing_kit.rs`

**Interfaces:**
- Produces:
  ```rust
  impl CallOptions { pub max_retries: Option<u32>; pub fn with_max_retries(self, n: u32) -> Self; }  // pub field, default None
  pub(crate) const KIT_STREAM_TIMEOUT: std::time::Duration;   // 10 s, testing/mod.rs
  pub(crate) fn kit_options() -> CallOptions;                   // max_retries = Some(0); Task 3 adds the caller
  pub(crate) enum Called { Errors(Vec<anyhow::Error>), TimedOut }
  pub(crate) async fn call_errors<C: Resolve>(registry: &Registry<C>, ctx: &C, d: &Descriptor, args: Value) -> Called;
  ```
- `call_errors` on a stream stops at the first item (`Ok` or `Err`) or the end, and wraps the whole call (open included) in `tokio::time::timeout(KIT_STREAM_TIMEOUT, ..)`. Every runner turns `TimedOut` into the problem `"<op>: the stream gave no item and did not end within 10 s; give a Fixture or a context that answers"`. The kit's docs say its stream checks need a tokio runtime with time enabled.

- [ ] **Step 1: Write the failing tests** in `tests/testing_kit.rs`:
  - `kit_stream_that_never_yields_is_reported_after_timeout`: `#[tokio::test(start_paused = true)]`; a stream op returning `futures_util::stream::pending()`; `assert_error_codes` panics (catch with `std::panic::AssertUnwindSafe(..).catch_unwind()`) with a message containing `"did not end within 10 s"`.
  - `kit_endless_stream_is_cut_after_first_item`: a stream op returning `stream::repeat(Ok(1))`; `assert_error_codes` returns.
  - `kit_calls_carry_max_retries_zero`: an op records `opts.max_retries`; after `assert_local_ops_offline` it is `Some(0)`.
  - `with_max_retries_sets_field`: `CallOptions::default().with_max_retries(3).max_retries == Some(3)`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test testing_kit kit_`
- [ ] **Step 3: Implement** as in Interfaces. (The test "a failing follower ends after one attempt" needs `follow_stream`; it is in Task 31.)
- [ ] **Step 4: Run green**, the same command, then `cargo-slot cargo check -p op-registry --all-features --tests` and `cargo-slot cargo tree -p op-listen --no-default-features -e normal,features -i tokio | grep 'tokio feature "rt"'` (expected: no output).
- [ ] **Step 5: Commit.** `op-registry 0.4: kit stream calls stop at the first item, 10 s timeout, max_retries hint`

---

### Task 2: PATH-1 paths as a set, and MAN-4

Spec: PATH-1, MAN-4; ruling (c).

**Files:**
- Modify: `op-registry/src/descriptor.rs`, `op-registry/src/hook.rs`, `op-registry/src/registry.rs` (`OpSpec.paths`, drop `cli_only_reason`, `describe`, `respec`), `op-registry/src/invoke.rs`, `op-registry/src/cli/tree.rs`, `op-registry/src/mcp/mod.rs`, `op-registry/src/manual.rs`, `op-registry/src/testing/{parity.rs,codes.rs,offline.rs,refs.rs}`, `op-registry/src/lib.rs`, `op-registry-derive/src/ops/{parse.rs,expand.rs}`, `op-registry/Cargo.toml` (`[[test]] cli_only` becomes `paths`), any op-listen/op-status use of `Paths::`
- Create: `op-registry/tests/paths.rs`; `op-registry/tests/ops_ui/ops_paths_unknown_word_is_a_compile_error.rs`, `ops_paths_empty_is_a_compile_error.rs`, `ops_paths_duplicate_is_a_compile_error.rs`, `ops_reason_key_is_a_compile_error.rs` (+ `.stderr` each)
- Delete: `op-registry/tests/cli_only.rs`, `tests/ops_ui/ops_cli_without_reason_is_a_compile_error.{rs,stderr}`, `tests/ops_ui/ops_reason_without_cli_is_a_compile_error.{rs,stderr}`
- Test: `op-registry/tests/paths.rs`, `op-registry/tests/manual.rs`, `op-registry/tests/ops_ui.rs`

**Interfaces:**
- Produces: `Surface`, `Paths` exactly as spec PATH-1 (`ALL`, `of`, `contains`, `iter`, `Default = ALL`; serde as a list of words in the fixed order cli, typed, mcp, json). Removed: `Paths::{All, TypedOnly, CliOnly}`, `OpSpec.cli_only_reason`, `Descriptor.cli_only_reason`, the `reason:` key.
  ```rust
  impl CallPath { pub fn surface(&self) -> Surface }   // Cli→Cli, Typed→Typed, Mcp→Mcp, Transport(_)→Json
  pub(crate) fn check_path<C>(entry: &Entry<C>, name: &str, surface: Surface) -> anyhow::Result<()>;  // wrong_path
  ```
- `check_path` also runs in `call_typed` and `call_stream_typed`. CLI tree: a command when `paths.contains(Cli)` and not a `ListPage`. `tool_list`: `paths.contains(Mcp)`, not hidden, not a stream, audience match. json: `contains(Json)`. Both list halves share paths.
- Manual (MAN-4): a `Paths: cli, typed` line in an op's section when `paths != Paths::ALL`; a new section `## Operations off the command line` listing every visible op without `cli` (name, summary, long help, paths, access, exits, codes); `check_exit_table` checks those entries too. Parity keeps compiling with "absent because not declared" as the allowed reason; exactness is Task 19.
- Derive errors (exact text): "`paths` takes cli, typed, mcp and json"; "`paths` needs at least one surface"; "`paths` lists `cli` twice"; "unknown key `reason`: 0.4.0 needs no reason for `paths`; remove it".

- [ ] **Step 1: Write the failing tests.** `tests/paths.rs` (`required-features = ["testing"]`):
  - `each_paths_set_reaches_exactly_its_surfaces`: for each of `[cli]`, `[typed]`, `[mcp]`, `[json]`, `[cli, typed]`, `[mcp, json]`, call the op through `cli::run_with_io`, `call_typed`, `mcp::call_tool`, `Registry::call`; declared surfaces succeed, every other fails with `op_registry.call.wrong_path` (CLI: the command does not exist, exit 2).
  - `cli_tree_and_tool_list_follow_paths`; `paths_serialize_as_words` (`Paths::of(&[Surface::Typed, Surface::Cli])` serializes `["cli","typed"]`, `ALL` all four); `list_halves_share_paths`.
  - `tests/manual.rs`: `manual_shows_paths_line`; `manual_lists_ops_off_the_command_line` (a `[mcp]` op and a `[typed]` op appear with their paths); `exit_table_checks_off_cli_ops`.
  - Four UI tests with the messages above.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test paths --test manual --test ops_ui`
- [ ] **Step 3: Implement.** Delete the old CLI-only fixtures and tests listed above. Re-render `tests/manual_snapshot.md` only if a snapshot test covers paths (the JSON `paths` key changes shape).
- [ ] **Step 4: Run green**, same command; `cargo-slot cargo check -p op-registry -p op-listen -p op-status --all-features --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: paths are any subset of cli/typed/mcp/json; manual lists ops off the CLI`

---

### Task 3: KIT-1, the kit runs every op it can reach

Spec: KIT-1.

**Files:**
- Modify: `op-registry/src/testing/{mod.rs,codes.rs,offline.rs,refs.rs}`, `op-registry/src/invoke.rs`
- Test: `op-registry/tests/testing_kit.rs`, `op-listen/tests/registry.rs`

**Interfaces:**
- Consumes: `Called`, `kit_options` (Task 1); `Paths`, `Surface`, `check_path` (Task 2).
- Produces:
  ```rust
  pub(crate) fn kit_path(d: &Descriptor) -> Option<CallPath>;   // first of json, mcp, cli in d.paths; None = typed only
  pub(crate) async fn call_stream_json<'a>(&'a self, name: &str, args: Value, ctx: &'a C,
      opts: CallOptions, cursor: Option<Cursor>, path: CallPath) -> Invocation<BoxStream<'a, Result<Value>>>;  // invoke.rs
  ```
  `kit_path` maps json→`CallPath::Transport("op-registry-kit".into())`, mcp→`Mcp`, cli→`Cli`. `kit_options()` now also sets `caller = Some(Caller("op-registry-kit".into()))`. `call_errors` uses `call_json` / `call_stream_json` with `kit_path`. In `offence()`, `op_registry.call.wrong_path` and `op_registry.call.wrong_shape` are offences; other `op_registry.*` codes stay allowed. A typed-only op is skipped: each runner prints `eprintln!("{title}: {op}: typed only: not run")` and appends the same line to the report when it panics.
- Remove the `paths != All` skips in `offline.rs` and `refs.rs`.

- [ ] **Step 1: Write the failing tests.**
  - `kit_runs_cli_only_op_in_error_codes`: a `[cli]` op returning an undeclared code makes `assert_error_codes` panic naming it (today it passes vacuously).
  - `kit_runs_cli_only_op_offline` and `kit_runs_cli_only_op_for_unknown_ref`: same op shape through `assert_local_ops_offline` and `assert_unknown_ref_not_found`; a recording op proves its body ran with `CallPath::Cli`.
  - `kit_skips_typed_only_op_and_names_it`: a `[typed]` op's body never runs.
  - `wrong_path_is_an_offence`: unit test on `offence` through a fixture forcing a wrong-path call.
  - `op-listen/tests/registry.rs`: `kit_listen_op_reaches_body`: a `listen_entry` op under `assert_error_codes` with a fixture; its listener's `authorize` is called (no `op_listen.no_caller`).
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test testing_kit` and `cargo-slot cargo test -p op-listen --features testkit --test registry`
- [ ] **Step 3: Implement.** Fix library tests that now run ops they skipped by adding `Fixture`s.
- [ ] **Step 4: Run green**, same commands; `cargo-slot cargo check -p op-registry -p op-listen -p op-status --all-features --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: the kit runs CLI-only and MCP-only ops on a declared surface, with a caller`

---

### Task 4: Lists: DEC-5 one-word paths, OUT-4 `follow_page`, OUT-5 coded guards

Spec: DEC-5, OUT-4, OUT-5, ERR-3 (`op_registry.list.cursor_repeated`).

**Files:**
- Modify: `op-registry/src/list.rs`, `op-registry/src/registry.rs` (`OpSpec.follow_page`), `op-registry/src/testing/codes.rs` (`declared_codes`), `op-registry/src/lib.rs`, `op-registry-derive/src/ops/{parse.rs,expand.rs}`
- Create: `op-registry/tests/ops_ui/ops_follow_page_on_non_list_is_a_compile_error.rs`, `ops_follow_page_zero_is_a_compile_error.rs` (+ `.stderr`)
- Test: `op-registry/tests/lists.rs`, `op-registry/tests/manual.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct OpSpec<P, O> { pub follow_page: Option<u32>, .. }   // default None = page_size
  #[derive(Debug, thiserror::Error, coded_error::CodedError)] #[coded(namespace = "op_registry.list")] #[non_exhaustive]
  pub enum ListError { #[error("page cursor {cursor} repeated")] #[coded(code = "cursor_repeated", class = Logical)] CursorRepeated { cursor: String } }
  ```
- `too_many_code(pillar, path)`: noun = `path[len-2]`, or `path[0]` for one word; `-` → `_`. `push_list` drops the noun assert; `declared_codes` drops `path.len() >= 2`. The follow-all half passes `follow_page.unwrap_or(page_size)` as `limit`. An empty page with a next cursor counts one toward the cap. Derive: list key `follow_page: <u32>`; errors "`follow_page` applies only to a `list:` entry", "`follow_page` must be at least 1".

- [ ] **Step 1: Write the failing tests** in `tests/lists.rs`: `one_word_list_registers_and_caps` (`graph-search`, code `fix.graph_search.too_many`); `assert_error_codes` accepts that code; `tests/manual.rs` `one_word_list_cap_code_in_manual`; `follow_all_uses_follow_page` (page fn sees `limit == 500` following, page half default `page_size`); `repeated_cursor_is_coded` (`op_registry.list.cursor_repeated`, Logical); `empty_pages_forever_end_with_too_many` (cap 3, empty pages with fresh cursors); two UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test lists --test manual --test ops_ui`
- [ ] **Step 3: Implement.** Delete the old `repeated_cursor_fails` expectation of an uncoded error.
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: one-word list paths, follow_page, coded cursor_repeated, empty pages count`

---

### Task 5: PAR-11, `FieldMeta`, `FieldKind` and `SubjectMeta` grow once

Spec: PAR-11 (with erratum 1: `SubjectMeta.absent_ok`).

**Files:**
- Modify: `op-registry/src/params/mod.rs`, `op-registry/src/list.rs` (paging fields), `op-registry/src/lib.rs`, `op-registry-derive/src/params/{mod.rs,attrs.rs,types.rs}`, `op-registry-derive/src/globals.rs`, every hand-written `FieldMeta` literal in the workspace's tests
- Test: `op-registry/tests/descriptor.rs`, `op-registry/tests/refs.rs`

**Interfaces:**
- Produces `FieldMeta` (`#[non_exhaustive]`, new fields `subject: Option<SubjectMeta>`, `env: Option<String>`, `prompt: Option<String>`) with the constructor and `with_*` builders exactly as spec PAR-11. `FieldMeta::new(rust_name, kind)` sets `flag` = kebab-case of the name, `key` = camelCase, `help` empty, others false/None.
  ```rust
  #[non_exhaustive] #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct SubjectMeta { pub kind: String, pub id_only: bool, pub absent_ok: bool }
  impl SubjectMeta { pub fn new(kind: impl Into<String>) -> Self; pub fn id_only(self, yes: bool) -> Self; pub fn absent_ok(self, yes: bool) -> Self; }
  #[non_exhaustive] pub enum FieldKind { Scalar, Switch, TriState, Repeated, Map, Json, Input, Ref, SecretFile, Path, Patch }
  ```
- The derive emits `FieldMeta::new(..).with_flag(..)..` (never a struct literal); a subject field gets `with_subject(SubjectMeta::new(kind).id_only(..))` (`absent_ok` is wired in Task 13). `PathBuf`/`Option<PathBuf>` report `Path`; `Patch<T>` reports `Patch`. Globals emit `with_env` for their env var.

- [ ] **Step 1: Write the failing tests:** `field_meta_new_derives_flag_and_key` (`FieldMeta::new("owner_key", FieldKind::Scalar)` has flag `owner-key`, key `ownerKey`); `pathbuf_field_reports_path_kind`; `patch_field_reports_patch_kind`; `subject_field_meta_names_kind` (`fields()` of a `#[op(subject = "agent", id_only)]` struct has `subject == Some(SubjectMeta { kind: "agent", id_only: true, absent_ok: false })`); `global_field_meta_has_env`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test descriptor --test refs`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**; `cargo-slot cargo check -p op-registry -p op-listen -p op-status --all-features --tests`; `cargo-slot cargo check -p op-registry --no-default-features --features manual`.
- [ ] **Step 5: Commit.** `op-registry 0.4: FieldMeta non_exhaustive with constructor; SubjectMeta; FieldKind Path and Patch`

---

### Task 6: OUT-1, `{"items": [...]}` on every JSON surface, with `$defs` hoisting

Spec: OUT-1. **Risk task:** a wrong hoist leaves dangling `$ref`s in every consumer's tool schema.

**Files:**
- Create: `op-registry/src/shape.rs`
- Modify: `op-registry/src/registry.rs` (`unary_entry` value wrap, `describe` schema wrap), `op-registry/src/list.rs` (follow-all half goes through the same wrap), `op-registry/src/testing/help.rs` (item schema at `/properties/items/items`), `op-registry/src/lib.rs`
- Test: `op-registry/tests/output_schema.rs`, `op-registry/tests/lists.rs`, `op-registry/tests/mcp.rs`, `op-registry/tests/testing_kit.rs`; update every existing test that expects a bare array

**Interfaces:**
- Produces (`shape.rs`, `pub(crate)`):
  ```rust
  pub(crate) fn wrap_items(value: Value) -> Value;          // Array(a) → {"items": a}; anything else unchanged
  pub(crate) fn wrap_array_schema(schema: Value) -> Value;  // only when root "type" == "array" (a string, not a list)
  ```
- `wrap_array_schema` output, key order fixed: `$schema` (moved, if present), `title` (moved), `"type": "object"`, `"properties": {"items": <the array schema minus $schema/title/$defs/definitions>}`, `"required": ["items"]`, `"additionalProperties": false`, `$defs` (moved), `definitions` (moved). Nothing inside the moved maps is rewritten: `#/$defs/T` and `#/definitions/T` pointers stay valid because both maps stay at the root.
- Value wrap happens where a unary output is erased to `Value` (every build, with or without `schema`). Schema wrap happens in `describe` for every kind except `Stream`. Not wrapped: stream items, `call_typed` (Rust value), `--text`.

- [ ] **Step 1: Write the failing tests.**
  - `tests/output_schema.rs`: `vec_output_schema_wrapped_with_root_defs` (`Vec<Named>`: root object, `required == ["items"]`, `$defs` at the root, every `$ref` resolves by JSON pointer from the root, with a helper `fn all_refs_resolve(schema: &Value) -> bool`); `recursive_item_refs_resolve`; `nested_vec_refs_resolve` (`Vec<Vec<Named>>`); `definitions_keyed_schema_hoisted` (hand-made `OutputSchema` impl with `definitions`); `optional_vec_root_not_wrapped`; `stream_of_vec_items_not_wrapped`.
  - `tests/lists.rs`: `cli_follow_all_prints_items_object`; `page_half_prints_items_and_next_cursor`; `call_returns_items_object`; `call_typed_returns_vec`; `value_output_array_is_wrapped`.
  - `tests/mcp.rs`: `call_tool_returns_items_object`.
  - `tests/testing_kit.rs`: `list_get_warning_fires_when_items_differ` and `..._quiet_when_items_match` against the wrapped schema.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test output_schema --test lists --test mcp --test testing_kit`
- [ ] **Step 3: Implement.** Re-render `tests/manual_snapshot.md` once.
- [ ] **Step 4: Run green**, same command; `cargo-slot cargo test -p op-registry --all-features --test manual --test call --test runner`; crate `check --tests` for all three crates (op-listen/op-status tests that print lists move to `{"items"}`).
- [ ] **Step 5: Commit.** `op-registry 0.4: array outputs are {"items": [...]} on every JSON surface; $defs hoisted`

---

### Task 7: OUT-6 key order, OUT-3 list text

Spec: OUT-6, OUT-3. Moved early (erratum 8).

**Files:**
- Modify: `Cargo.toml` (workspace `serde_json = { version = "1", features = ["preserve_order"] }`), `op-registry/src/output.rs`, `op-registry/src/list.rs`, `op-registry/src/registry.rs`, `op-registry-derive/src/ops/{parse.rs,expand.rs}`
- Delete: `op-registry/tests/ops_ui/ops_text_on_list_is_a_compile_error.{rs,stderr}`
- Test: `op-registry/tests/output.rs`, `op-registry/tests/lists.rs`; re-render `op-registry/tests/manual_snapshot.md` and fix golden texts in op-listen/op-status tests

**Interfaces:**
- Produces:
  ```rust
  impl<T: RenderText> RenderText for Vec<T> { /* items joined by "\n" */ }
  #[doc(hidden)] pub fn render_page<T: RenderText>(page: &Page<T>) -> String;   // items, then "next cursor: <c>" when Some
  #[doc(hidden)] pub fn push_list_text<P, T, F>(&mut self, spec: OpSpec<P, T>, f: F) where T: RenderText + ..;  // as push_list, plus both halves' render_text
  ```
  `ops!` with `text: true` on a `list:` entry expands to `push_list_text`; `exits:` without `on` on a list stays rejected.

- [ ] **Step 1: Write the failing tests:** `output_keys_keep_declaration_order` (struct `{ zeta, alpha }` prints `zeta` first, on the CLI and through `call`); `list_text_follow_all_one_row_per_line`; `list_text_page_ends_with_next_cursor`; `list_text_page_without_cursor_has_no_cursor_line`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test output --test lists`
- [ ] **Step 3: Implement**; re-render the manual snapshot.
- [ ] **Step 4: Run green**, same command plus `--test manual --test ops_ui`; `cargo-slot cargo test -p op-listen --features testkit --test follow_golden --test frame_golden`; `cargo-slot cargo test -p op-status --all-features`.
- [ ] **Step 5: Commit.** `op-registry 0.4: JSON keys in declaration order; --text on list pairs`

---

### Task 8: DEC-1 several audiences, DEC-6 MCP context factory and instructions

Spec: DEC-1, DEC-6.

**Files:**
- Modify: `op-registry/src/descriptor.rs`, `op-registry/src/registry.rs`, `op-registry/src/call.rs`, `op-registry/src/invoke.rs`, `op-registry/src/mcp/{mod.rs,rmcp.rs}`, `op-registry/src/manual.rs` (JSON `audiences`), `op-registry-derive/src/ops/{parse.rs,expand.rs}`
- Create: `op-registry/tests/ops_ui/ops_audience_empty_is_a_compile_error.rs`, `ops_audience_twice_is_a_compile_error.rs` (+ `.stderr`)
- Test: `op-registry/tests/mcp.rs`, `op-registry/tests/rmcp.rs`, `op-registry/tests/manual.rs`

**Interfaces:**
- Produces: `OpSpec.more_audiences: Vec<Audience>`; `Descriptor.audiences: Vec<Audience>` (primary first, no duplicates); `CallOptions` private `serving: Option<Audience>` set by `call_tool`; `CallInfo.audience = opts.serving.unwrap_or(primary)`. `tool_list(reg, a)` lists when `audiences.contains(&a)`; `call_tool` refuses `outside_audience` only when not contained. `RegistryServer::with_ctx_factory` and `with_instructions` exactly as spec DEC-6; the factory runs once per `tools/call`; `get_info` returns the instructions.
- Derive: `audience: agent` or `audience: [agent, owner]`; errors "audience needs at least one of agent, owner, dev", "audience `owner` is given twice".

- [ ] **Step 1: Write the failing tests:** `multi_audience_op_in_both_tool_lists_and_callable`; `call_info_audience_is_server_on_mcp_primary_elsewhere` (recording `CallHook`, MCP/CLI/typed); `agent_only_op_outside_owner_server`; `manual_json_has_audiences`; rmcp: `ctx_factory_runs_per_call` (two concurrent calls, counter is 2, each sees its own ctx); `instructions_in_get_info`; `new_server_unchanged`; two UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test mcp --test rmcp --test manual --test ops_ui`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: ops in several audiences; MCP context per call and server instructions`

---

### Task 9: OUT-2 object roots for either-or outputs

Spec: OUT-2.

**Files:**
- Modify: `op-registry/src/shape.rs`, `op-registry/src/registry.rs` (`describe`), `op-registry/src/mcp/{mod.rs,rmcp.rs}`, `op-registry/src/manual.rs` (`ManualOutput` schema hand-written as object)
- Test: `op-registry/tests/mcp.rs`, `op-registry/tests/rmcp.rs`, `op-registry/tests/manual.rs`

**Interfaces:**
- Consumes: `shape.rs` (Task 6).
- Produces (`pub(crate)`):
  ```rust
  pub(crate) enum RootClass { Object, Any, NonObject }
  pub(crate) fn root_class(schema: &Value) -> RootClass;     // Any: no type, no combinator, no $ref, no properties
  pub(crate) fn mark_object_root(schema: &mut Value);        // root oneOf/anyOf whose every branch is an object gets "type": "object"
  ```
  A branch is an object when it has `"type": "object"`, or `properties`, or is a `$ref` (followed into the root `$defs`/`definitions`, cycle-safe) to one, or an `allOf` with an object member. A missing schema (feature `schema` off) is `Any`.
- `ToolSpec.output_schema` is `Some` only for `Object`. The rmcp adapter returns `CallToolResult::structured(value)` for a JSON object value, else a success with one text block holding the compact JSON and no structured content.

- [ ] **Step 1: Write the failing tests:** `tagged_enum_output_has_object_root_and_structured_content`; `ref_newtype_variants_classed_object`; `u32_output_has_no_output_schema_and_text_content`; `value_output_no_schema_structured_when_object`; `manual_op_schema_root_is_object`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test mcp --test rmcp --test manual`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: object roots for either-or outputs; MCP structured content only for objects`

---

### Task 10: REL-1 presence members, REL-6 relation schemas

Spec: REL-1, REL-6. The base for Tasks 11-12.

**Files:**
- Modify: `op-registry/src/params/rt.rs`, `op-registry-derive/src/params/{attrs.rs,clap_gen.rs,schema.rs,mod.rs}`, `op-registry/Cargo.toml` (dev `jsonschema`, MIT; confirm the version's license; pick a version whose MSRV is ≤ 1.89), op-registry's `testing/sample.rs` (drop the `dependentRequired` reader)
- Create: `op-registry/tests/params_ui/op_params_relation_on_plain_field_is_a_compile_error.rs`, `op_params_one_of_needs_two_members_is_a_compile_error.rs` (+ `.stderr`)
- Delete: `op-registry/tests/params_ui/op_params_one_of_needs_two_options_is_a_compile_error.{rs,stderr}`
- Test: `op-registry/tests/params_relations.rs`

**Interfaces:**
- Produces (`rt`, `#[doc(hidden)]`):
  ```rust
  pub trait Presence { fn present(&self) -> bool; }   // impls: bool, Option<T>, Vec<T>, BTreeMap<K, V>, Patch<T> (not Keep)
  pub fn present_schema(key: &str, kind: FieldKind) -> Value;   // the table in spec REL-6
  impl SchemaBuilder { pub fn constraint(&mut self, c: Value); /* build: none; one merged into root; 2+ under allOf */ }
  ```
  `absorb` copies a flattened group's `oneOf`, `anyOf`, `not`, `if`/`then` and every `allOf` member as constraints. `one_of`, `one_of_optional` and `requires_if` use the REL-6 fragments; `requires_if` takes a string or a list. clap: tri-state members add `x` and `no_x`, `Patch` members `x` and `clear_x`. Derive error text from spec REL-1.

- [ ] **Step 1: Write the failing tests** in `tests/params_relations.rs`, each on CLI (`run_with_io`), JSON (`call`) and typed (`call_typed`):
  - `one_of_over_optional_ref_and_switch` (`id: Option<Ref>`, `all: bool`): each alone accepted, both and neither refused; `{"all": false, "id": "x"}` accepted.
  - `requires_if_with_vec_target`; `requires_if_list_targets`; `one_of_optional_over_switch_and_two_options`.
  - `schema_agrees_with_validate`: for each relation fixture a table of inputs (including `false`, `[]`, `null` members); `jsonschema::is_valid(schema, input) == (from_json + validate).is_ok()`.
  - `schema_with_single_non_one_of_constraint_is_valid`; `flattened_any_of_survives_absorb`.
  - Two UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test params_relations --test params_ui`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command plus `--test testing_kit`; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: relations over any presence field; schemas that match validate`

---

### Task 11: REL-2 `required` on Vec/Option, REL-3 `conflicts_with`/`required_unless`, REL-4 `at_least_one`, REL-5 dotted targets

Spec: REL-2, REL-3, REL-4, REL-5, ERR-3.

**Files:**
- Modify: `op-registry/src/params/{mod.rs,rt.rs}`, `op-registry-derive/src/params/{attrs.rs,clap_gen.rs,json_gen.rs,schema.rs,mod.rs}`, `op-registry/src/manual.rs` (library-code table de-duplicated by code)
- Create: one `params_ui` fixture per derive error in REL-2, REL-3, REL-4, REL-5 (names `op_params_<what>_is_a_compile_error.rs`)
- Test: `op-registry/tests/params_relations.rs`, `op-registry/tests/manual.rs`

**Interfaces:**
- Consumes: `Presence`, `present_schema`, `SchemaBuilder::constraint` (Task 10).
- Produces new `UsageError` variants:
  ```rust
  #[error("{field} conflicts with {other}")] #[coded(code = "conflicts", class = Logical)] Conflicts { field: String, other: String },
  #[error("{name} is required unless {unless} is given")] #[coded(code = "missing", class = Logical)] MissingUnless { name: String, unless: String },
  #[error("at least one of the {group} fields is required")] #[coded(code = "at_least_one", class = Logical)] AtLeastOne { group: String },
  ```
- Derive attrs: `#[op(required)]` on `Vec<T>`/`Option<T>` (clap `.required(true)`, tri-state via a required `ArgGroup`; schema `required` plus `minItems: 1` for Vec; JSON absent/`[]`/`null` is `.missing`); `conflicts_with = ".."|[..]`; `required_unless = ".."|[..]`; `at_least_one = "group"`; dotted targets `"common.dry_run"` whose first segment is a flatten field, validated by `Presence::present(&self.common.dry_run)` and clap's leaf arg id; a flattened leaf's fragment built at run time from `<Group as OpParams>::fields()`.

- [ ] **Step 1: Write the failing tests** (every surface; CLI exit 2): `required_vec_shapes`; `required_option_bool_missing_on_mcp_is_missing` (today becomes `false`); `message_submit_shape` (`--to` required unless `--signed-file`, conflicts with it); `confirm_fingerprint_required_unless_dry_run`; `at_least_one_graph_search` (`--name`/`--file`/`--node-kind`); `dotted_required_unless_across_flatten` (structmux `EditCommon { owner_key_file, dry_run }` in `AddDevice`, on `confirm_fingerprint` and on the group's own `owner_key_file`); `schema_agrees_with_validate` extended with these fixtures; `manual.rs` `library_code_table_lists_each_code_once`; one UI test per derive error.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test params_relations --test params_ui --test manual`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: required Vec/Option, conflicts_with, required_unless, at_least_one, dotted targets`

---

### Task 12: REL-7 struct validator, public `usage()`, usage hook for op-side usage errors

Spec: REL-7.

**Files:**
- Modify: `op-registry-derive/src/params/{mod.rs,attrs.rs}` (parse struct-level `#[op(..)]`), `op-registry/src/params/mod.rs` (`usage` documented, not hidden), `op-registry/src/cli/run.rs`
- Create: `op-registry/tests/params_ui/op_params_unknown_struct_key_is_a_compile_error.rs`, `op_params_validate_without_path_is_a_compile_error.rs` (+ `.stderr`)
- Test: `op-registry/tests/params_relations.rs`, `op-registry/tests/runner.rs`

**Interfaces:**
- Produces: `#[op(validate = path)]` on the struct, `fn(&Self) -> Result<(), UsageError>`; derived `validate` = built-in relations, then each flattened group's `validate`, then the struct validator (its `Err` through `usage(err)`). Struct-level keys accepted in this task: `validate` (Task 16 adds `allow_hyphen`). `pub fn usage(error: UsageError) -> anyhow::Error`.
- Runner: `RunHooks::on_usage_error` is called once per run for every failure whose exit is 2 (decode, validate, op returning `op_registry.usage.*`; Task 23 adds the needs check), before `post_run`.

- [ ] **Step 1: Write the failing tests:** `struct_validator_all_needs_yes_unless_dry_run` (gait's rule, every surface, exit 2); `op_usage_error_reaches_hook_once_and_exits_2`; `validate_runs_before_refs_resolve` (resolver not called on failure); two UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test params_relations --test runner --test params_ui`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: struct-level validate, public usage(), usage hook for every exit-2 failure`

---

### Task 13: Subjects: PAR-1 optional subject, PAR-4 `absent_ok`, PAR-5 `check_id`

Spec: PAR-1, PAR-4 (erratum 1), PAR-5.

**Files:**
- Modify: `op-registry-derive/src/params/{attrs.rs,ref_gen.rs,types.rs,clap_gen.rs,json_gen.rs,schema.rs}`, `op-registry/src/refs.rs`, `op-registry/src/invoke.rs` (`check_id` for slots that skip resolution)
- Create: `params_ui` fixtures `op_params_vec_ref_subject_is_a_compile_error.rs`, `op_params_absent_ok_on_non_subject_is_a_compile_error.rs` (+ `.stderr`)
- Test: `op-registry/tests/refs.rs`, `op-listen/tests/registry.rs`

**Interfaces:**
- Produces: `#[op(subject = "build", scope = [..])] id: Option<Ref>` (flag `--id`, key `id`, not required; absent = no slot); `#[op(subject = "agent", absent_ok)]` on `Ref`/`Option<Ref>`, with or without `id_only`: never resolved, op gets `Resolved::Id(raw)`, `FieldMeta.subject.absent_ok = true`.
  ```rust
  pub trait Resolve: Send + Sync {
      fn check_id(&self, kind: &str, raw: &str) -> Result<(), UsageError> { Ok(()) }   // new
  }
  ```
  `check_id` runs for every `id_only` or `absent_ok` slot, on every surface, before the op; `Err` is `usage(err)` (exit 2 on the CLI). Server-resolved kinds are not checked. Derive errors: "the subject is one `Ref` or `Option<Ref>`"; "`absent_ok` applies only to the subject".

- [ ] **Step 1: Write the failing tests:** `optional_subject_absent_is_none_without_resolver`; `optional_subject_present_resolved_with_scope`; `one_of_over_id_and_action`; `absent_ok_subject_never_resolved`; `absent_ok_with_id_only_allowed`; `check_id_runs_for_absent_ok_and_id_only`; `malformed_id_is_usage_invalid_every_surface` (exit 2 on CLI); `op-listen/tests/registry.rs` `listen_entry_with_optional_subject_group`; two UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test refs --test params_ui` and `cargo-slot cargo test -p op-listen --features testkit --test registry`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same commands; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: optional subjects, absent_ok subjects, Resolve::check_id`

---

### Task 14: Non-subject refs: PAR-2 scope and `id_only`, PAR-3 `Patch<Ref>`

Spec: PAR-2, PAR-3.

**Files:**
- Modify: `op-registry-derive/src/params/{attrs.rs,ref_gen.rs,types.rs}`, `op-registry/src/params/rt.rs` (scope helpers, if needed). The README line for PAR-2 is written in Task 34.
- Create: `params_ui/op_params_non_subject_scope_names_ref_is_a_compile_error.rs`, `op_params_patch_ref_subject_is_a_compile_error.rs` (+ `.stderr`)
- Test: `op-registry/tests/refs.rs`

**Interfaces:**
- Produces: `#[op(kind = "..", scope = [..])]` and `#[op(kind = "..", id_only)]` on non-subject `Ref`, `Option<Ref>`, `Vec<Ref>`; `#[op(kind = "place", id_only, clearable)] home_place: Patch<Ref>` (`Set` has a slot; `Keep`/`Clear` none; `--clear-home-place`; JSON `null` is `Clear`). Derive errors: "the scope of non-subject ref `x` names the ref `y`; refs resolve non-subjects first, so only the subject's scope may name a ref"; "the subject is not clearable". The old "id_only and scope belong to the subject" error is deleted with its fixture, if one exists.

- [ ] **Step 1: Write the failing tests:** `non_subject_ref_scope_reaches_resolver`; `non_subject_id_only_skips_resolver`; `id_without_blueprint_has_no_blueprint_scope_key` (archegraph); `patch_ref_set_resolves_keep_clear_do_not`; `patch_ref_clear_flag_and_json_null`; two UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test refs --test params_ui`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: scope and id_only on non-subject refs; Patch<Ref>`

---

### Task 15: Decoding: PAR-6 `from_str` and `sample`, PAR-8 `ignore_case`

Spec: PAR-6, PAR-8.

**Files:**
- Modify: `op-registry-derive/src/params/{attrs.rs,clap_gen.rs,json_gen.rs,schema.rs}`, `op-registry-derive/src/globals.rs`, `op-registry/src/params/rt.rs` (`enum_value(name, ignore_case)`; `from_str` decode helper)
- Create: `params_ui` fixtures for: `from_str` with `value_enum`; `from_str` on `Ref`; `sample` on a non-string field; `ignore_case` without `value_enum`
- Test: `op-registry/tests/params_flags.rs`

**Interfaces:**
- Produces: `#[op(from_str)]` on `T`, `Option<T>`, `Vec<T>` and in `OpGlobals` (`T: FromStr`, `T::Err: Display`); failure is `op_registry.usage.invalid` naming the flag/key with the parser's message (a `secret` field: "invalid value"); schema `{"type": "string"}`; `default` is a `T` expression. `#[op(sample = "..")]` on any string-valued field adds `"examples": [".."]`. `#[op(value_enum, ignore_case)]`: clap `.ignore_case(true)`, JSON case-insensitive, schema keeps canonical names. Derive errors from spec PAR-6/PAR-8.

- [ ] **Step 1: Write the failing tests:** `socket_addr_from_str_cli_and_json`; `vec_of_custom_grant_from_str`; `global_principal_id_from_str`; `bad_from_str_is_exit_2_with_parser_message`; `secret_from_str_failure_says_invalid_value`; `sample_puts_examples_in_schema`; `ignore_case_status_cli_and_json` (`IN_PROGRESS`, `"In-Progress"`; schema lists `in-progress`); four UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test params_flags --test params_ui`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: from_str values, sample examples, value_enum ignore_case`

---

### Task 16: CLI flag behaviour: PAR-7 `allow_hyphen`, PAR-9 literal defaults in help, PAR-10 `env` on params

Spec: PAR-7, PAR-9, PAR-10 (erratum 2).

**Files:**
- Modify: `op-registry-derive/src/params/{attrs.rs,clap_gen.rs,mod.rs}` (struct-level `allow_hyphen`)
- Create: `params_ui` fixtures: `allow_hyphen` on a switch; `hide_default` without `default`; `env` on a `Vec` (`op_params_env_on_vec_is_a_compile_error.rs`)
- Delete: `op-registry/tests/params_ui/op_params_env_on_params_is_a_compile_error.{rs,stderr}`
- Test: `op-registry/tests/params_flags.rs`

**Interfaces:**
- Produces: `#[op(allow_hyphen)]` per field and on the struct (every valued field of that struct, not flattened groups); literal `default` shown as `[default: 50]` (enum: its clap name; empty string and computed defaults not shown); `#[op(hide_default)]`; `#[op(env = "X")]` on plain/optional scalars and switches: clap `.env(X)` + `.hide_env_values(true)` (`secret`: `.hide_env(true)`), same decode as the flag, `FieldMeta.env = Some("X")`, never read on typed/json/MCP. No push-time check on paths.

- [ ] **Step 1: Write the failing tests:** `allow_hyphen_value_minus_one`; `struct_allow_hyphen_reply_message`; `without_allow_hyphen_fails_as_before`; `help_shows_literal_defaults` (`[default: 50]`, `[default: syntax]`); `empty_and_hidden_defaults_not_shown_schema_keeps_default`; `env_fills_absent_flag`; `flag_wins_over_env`; `malformed_env_value_is_exit_2`; `help_shows_env_var`; `env_ignored_off_cli_value_must_be_passed` (same op over `call` without the key is `.missing` while the env var is set); `empty_env_param_is_absent`; three UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test params_flags --test params_ui`
- [ ] **Step 3: Implement.** For `empty_env_param_is_absent`, confirm clap 4 treats an empty env value as absent; if it does not, filter it in the derive.
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: allow_hyphen, literal defaults in help, hide_default, env on params (CLI only)`

---

### Task 17: ERR-2 error mapper, OUT-7 typed and stream calls return `CallFailure`

Spec: ERR-2, OUT-7. OUT-7 lands before SEC-1 (erratum 8).

**Files:**
- Modify: `op-registry/src/registry.rs` (`error_mapper`, `merge`), `op-registry/src/invoke.rs`, `op-registry/src/call.rs`, `op-registry/src/mcp/mod.rs`, `op-registry/src/cli/run.rs`, `op-listen/src/*` and `op-status/src/*` call sites of `call_stream`/`call_typed`
- Test: `op-registry/tests/call.rs`

**Interfaces:**
- Produces:
  ```rust
  pub fn set_error_mapper(&mut self, mapper: fn(anyhow::Error) -> anyhow::Error);   // Registry
  // signatures of call_typed, call_stream_typed, call_stream: exactly spec OUT-7
  impl coded_error::CodedError for CallFailure { /* code, class and overrides read from facts_anyhow(&self.error) */ }
  impl CallFailure { pub fn into_coded(self) -> anyhow::Error; }   // coded_error::into_anyhow(self): code kept, Display redacted
  ```
- The mapper is applied once in the pipeline to resolution and op errors and to each stream item error, before `on_error`, strict codes, the CLI exit and every surface's output; never to decode/validate errors. `merge` keeps the receiver's mapper.

- [ ] **Step 1: Write the failing tests:** `mapper_rewrites_io_error_on_every_surface` (CLI error line code, `call`, `call_tool`, `call_typed`); `hook_on_error_sees_mapped_error`; `usage_error_not_mapped`; `merge_keeps_receiver_mapper`; `typed_error_redacts_secret_param`; `typed_failure_downcasts_to_op_error`; `into_coded_keeps_code` (`facts_anyhow(&f.into_coded()).code` is the op's code); `stream_open_and_item_errors_are_call_failures`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test call --test registry`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; `cargo-slot cargo check -p op-registry -p op-listen -p op-status --all-features --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: one error mapper per registry; typed and stream calls return CallFailure`

---

### Task 18: KIT-3 not_found by subject, KIT-4 sampling that decodes

Spec: KIT-3 (erratum 1), KIT-4.

**Files:**
- Modify: `op-registry/src/testing/{refs.rs,sample.rs,offline.rs,codes.rs,mod.rs}`
- Test: `op-registry/tests/testing_kit.rs`

**Interfaces:**
- Consumes: `FieldMeta.subject`, `FieldKind::Path`/`SecretFile` (Task 5); REL-6 fragments (Task 10); `examples` (Task 15); `kit_path` (Task 3).
- Produces:
  ```rust
  pub fn sample_params_seeded(schema: &Value, seed: serde_json::Map<String, Value>) -> Value;
  pub(crate) struct KitDir { /* path under std::env::temp_dir(), "op-registry-kit-<pid>-<n>"; removed on Drop */ }
  pub(crate) fn kit_args(d: &Descriptor, fields: &[FieldMeta], sampled: Value, dir: &KitDir) -> Value;  // Path → "<dir>/<key>"; SecretFile → a temp file holding "op-registry-kit-sample\n"
  ```
  Descriptors do not carry `FieldMeta`; read them from the erased entry, adding `pub(crate) fields: fn() -> Vec<FieldMeta>` to it if it has no field list yet. `assert_unknown_ref_not_found` runs on every op whose `FieldMeta.subject` is `Some` and not `absent_ok` (including `id_only`, server kinds, optional subjects, streams), seeds `id` with the random ref, expects a Logical code ending `.not_found`. Any sampled call failing with `op_registry.usage.*` is the problem "<op>: the sampled arguments did not decode (<code>: <message>); give a Fixture". Fixtures always win.

- [ ] **Step 1: Write the failing tests:** one per sampler rule (`examples[0]`; `const: true`; `minItems`; `minProperties`; `not null`; `if`/`then`; `not allOf` avoided; seeded keys kept first); `one_of_id_or_all_samples_one_member`; `required_vec_gets_one_item`; `pathbuf_output_lands_in_kit_dir` (and the dir is gone afterwards); `from_str_with_sample_decodes`; `from_str_without_sample_reported_did_not_decode`; `id_only_subject_answering_value_fails_not_found`; `id_only_subject_answering_not_found_passes`; `absent_ok_subject_not_run`; `optional_subject_run_with_id_set`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test testing_kit`
- [ ] **Step 3: Implement**; add fixtures to library tests that now fail honestly.
- [ ] **Step 4: Run green**, same command; `cargo-slot cargo test -p op-listen --features testkit --test testkit --test registry`; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: kit not_found covers every non-absent_ok subject; sampling that decodes, temp dirs`

---

### Task 19: KIT-6 exact parity and object roots (with SEC-8), MAN-6, MAN-7

Spec: KIT-6, SEC-8 (erratum 5), MAN-6, MAN-7.

**Files:**
- Modify: `op-registry/src/testing/{parity.rs,help.rs}`
- Test: `op-registry/tests/testing_kit.rs`

**Interfaces:**
- Consumes: `Paths` (Task 2), `root_class` (Task 9), `FieldMeta.subject` (Task 5).
- `assert_parity`/`parity_report` keep their signatures; `Reach`/`ParityLine`/`Mount` unchanged. Per op and surface (cli, mcp, each mount as json): reachable exactly when declared. Allowed absences: not declared; a stream on mcp or a mount ("stream"); a list page half on cli. Problems: declared but absent; present but not declared; `root_class == NonObject` on an op reachable on mcp or json ("<op>: the output root is <scalar|nullable|mixed>; give it an object form"); an mcp-reachable op with a `SecretFile` field whose input-schema property has no `inline` object form ("<op>: --<flag> is a SecretFile on mcp; add #[op(secret_inline)] or leave mcp out of paths"). The reason "cli only: <reason>" is gone.
- MAN-6: `assert_help_complete` keys the one-wording rule for `--id` on (flag, subject kind). MAN-7: `walk` skips only the root command's own `help`/`version`.

- [ ] **Step 1: Write the failing tests:** `parity_declared_but_absent`; `parity_present_but_not_declared`; `parity_allows_stream_and_page_half`; `value_output_passes_root_check`; `u32_output_fails_root_check`; `optional_vec_output_fails_parity_root`; `mcp_secret_file_fails_parity`; `mcp_secret_inline_passes`; `cli_only_secret_file_passes`; `id_wording_per_subject_kind_passes`; `two_id_wordings_for_one_kind_fail`; `leaf_version_param_without_help_fails`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test testing_kit --test paths`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; `cargo-slot cargo test -p op-registry --all-features` (whole crate, once, to give the pilot a clean base); `cargo-slot cargo test -p op-listen --features testkit`; `cargo-slot cargo test -p op-status --all-features`.
- [ ] **Step 5: Commit.** `op-registry 0.4: exact parity, object output roots, SecretFile on MCP; help kit per subject kind and leaf --version`

> **PILOT COMMIT.** The commit closing Task 19 is the archegraph pilot pin. Record its sha in the orchestrator's integration list. It carries everything archegraph exercises (DEC-5, OUT-2, PAR-1, REL-1, PAR-2, OUT-3, ERR-2, DEC-6, OUT-4, REL-4, plus PAR-8, PAR-9, REL-3, DEC-1, OUT-1, OUT-6, OUT-7, PATH-1, PAR-11 and KIT-1..4, KIT-6). Breaking changes still to come in Part B that may touch the pilot: FB-1 extras/fallback signatures, `Redactor` (`&self`), `Outcome::Err.exit`, `Exit.on`, `Io` non-exhaustive, `SecretSentinel`, op-listen `supervise`. The pilot re-pins to the final 0.4.0 commit before the tag; fixes the pilot finds go into Part B.

---

# Part B: the rest (Tasks 20-34)

### Task 20: FB-1 extras and fallbacks return `Result<u8>`, FB-3 extra flag clashes, FB-4 `on_selected`

Spec: FB-1, FB-3, FB-4.

**Files:**
- Modify: `op-registry/src/cli/{extra.rs,fallback.rs,run.rs,tree.rs,mod.rs}`, `op-registry/src/lib.rs`
- Test: `op-registry/tests/runner.rs`, `op-registry/tests/fallback.rs`

**Interfaces:**
- Produces: `push_extra`, `fallback_flags_only` signatures from spec FB-1 (`fallback_free_text` changes in Task 21). `Err` → `post_run(Outcome::Err)`, the error line redacted with the secret globals, exit by class (2 for a usage code); `Ok(n)` exits `n`.
  ```rust
  #[non_exhaustive] #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum Selected { Op { name: String }, Extra { name: String }, Fallback { name: &'static str, args: Vec<OsString> }, Help, Version }
  pub trait RunHooks { fn on_selected(&self, selected: &Selected) {} /* existing methods */ }
  ```
  `on_selected` is called by `run_with_io` once a parse succeeds, before the context is built and before an extra or fallback runs. `push_extra` panics when the command or a subcommand has a long flag equal to a registry global or `pretty`, `compact`, `text`, `help`, `version` ("extra `<name>`: --<flag> is already a global flag"); `cli::command` repeats the check for extras added by `merge`.

- [ ] **Step 1: Write the failing tests:** `extra_err_writes_error_line_and_exits_by_class`; `flags_only_fallback_err_post_run_sees_err`; `extra_ok_exits_n`; `extra_with_pretty_flag_panics`; `extra_with_name_flag_ok`; `merged_extra_clash_panics_at_command`; `on_selected_once_per_kind` (op, extra, fallback, help, version); `usage_failure_does_not_call_on_selected`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test runner --test fallback`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: extras and fallbacks return Result<u8>; extra flag clashes; RunHooks::on_selected`

---

### Task 21: FB-2 FreeText routing, MAN-3 fallback help pointer

Spec: FB-2, MAN-3.

**Files:**
- Modify: `op-registry/src/cli/{fallback.rs,parse.rs,tree.rs,run.rs}`
- Test: `op-registry/tests/fallback.rs`

**Interfaces:**
- Produces: `FallbackFlag { long, short, takes_value }` (`Clone, Copy, Debug`); `fallback_free_text(name, help, flags: &'static [FallbackFlag], near_miss: bool, run)` as spec FB-1/FB-2. Routing exactly as spec FB-2 items 1-4. Root help line `<name>: <first help line> (more: <bin> help <name>)`; `{bin}` in a fallback's help is replaced by `Registry::bin()` wherever shown.

- [ ] **Step 1: Write the failing tests:** `empty_argv_reaches_fallback_with_empty_vec`; `globals_only_reaches_fallback_with_empty_vec`; `trailing_globals_taken_from_tail`; `value_flag_value_spelling_a_global_kept`; `double_dash_stops_scan`; `near_miss_false_sends_stat_to_fallback`; `near_miss_true_refuses_stat`; `short_flag_clash_panics`; `root_help_has_fallback_pointer`; `fallback_help_replaces_bin` (`gait-min help turn` shows `gait-min`).
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test fallback`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: FreeText fallback routing with declared flags and near_miss; help pointer and {bin}`

---

### Task 22: FB-5 sync selection before any runtime

Spec: FB-5.

**Files:**
- Modify: `op-registry/src/cli/{parse.rs,fallback.rs,run.rs,mod.rs}`, `op-registry/src/manual.rs`
- Test: `op-registry/tests/fallback.rs`

**Interfaces:**
- Consumes: `Selected` (Task 20).
- Produces:
  ```rust
  pub fn select<C: CliContext>(registry: &Registry<C>, argv: &[OsString]) -> Result<Selected, UsageFailure>;   // sync, same routing and parse as cli::parse
  impl<C: CliContext + Sync> Registry<C> { pub fn fallback_flags_only_before_runtime(&mut self, name: &'static str, help: &'static str); }
  ```
  A before-runtime fallback routes, shows in help and the manual, and `help <name>` works; if `cli::run` reaches it, the run fails with the uncoded error "fallback `<name>` runs before the runtime; dispatch it after cli::select" (exit 70).

- [ ] **Step 1: Write the failing tests:** `select_empty_argv_is_fallback`; `select_flag_led_argv_is_fallback`; `select_command_is_op`; `select_bare_unknown_word_is_usage_failure`; `run_reaching_before_runtime_fallback_exits_70`; `before_runtime_fallback_in_help_and_manual`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test fallback`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: cli::select and before-runtime fallbacks`

---

### Task 23: DEC-3 `#[cfg]` on entries, DEC-4 per-op required globals

Spec: DEC-3, DEC-4.

**Files:**
- Modify: `op-registry-derive/src/ops/{parse.rs,expand.rs}`, `op-registry/src/registry.rs` (`OpSpec.needs_globals`), `op-registry/src/descriptor.rs` (`needs_globals`), `op-registry/src/cli/{parse.rs,tree.rs}`, `op-registry/src/manual.rs`
- Create: `ops_ui/ops_allow_attr_on_entry_is_a_compile_error.rs`, `ops_globals_twice_is_a_compile_error.rs`, `ops_globals_non_ident_is_a_compile_error.rs` (+ `.stderr`)
- Test: `op-registry/tests/ops_macro.rs`, `op-registry/tests/runner.rs`, `op-registry/tests/manual.rs`

**Interfaces:**
- Produces: `#[cfg(..)]` on an `ops!` entry is copied onto its push block, its param/output aliases and its marker(s); other attributes fail with "only doc comments and #[cfg(..)] are allowed on an op entry". Entry key `globals: [gait_dir]` → `OpSpec.needs_globals: Vec<&'static str>` (rust names) → `Descriptor.needs_globals: Vec<String>` (long flags). `cli::parse` checks needs after selecting the op (value source `CommandLine` or `EnvVariable`); missing is `op_registry.usage.missing`, exit 2, message "`init` needs --gait-dir (or GAIT_DIR)", reaching `on_usage_error`; `--help`/`--version` never check. `cli::command` panics for an unknown global name or a switch/tri-state ("a needed global must take a value"). Help ends with "Needs: --gait-dir (or GAIT_DIR)."; the manual prints the same line.

- [ ] **Step 1: Write the failing tests:** `cfg_off_entry_absent_and_no_marker`; `cfg_on_entry_present`; `missing_need_exits_2_with_message`; `env_meets_need`; `empty_env_does_not_meet_need`; `flag_before_or_after_verb_meets_need`; `parse_alone_returns_need_failure`; `help_works_without_need`; `unknown_need_panics_at_command`; `need_reaches_usage_hook`; `manual_prints_needs_line`; three UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test ops_macro --test runner --test manual --test ops_ui`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: #[cfg] on ops! entries; per-op required globals`

---

### Task 24: DEC-2 common errors, ERR-1 error exits, MAN-1 common codes printed once

Spec: DEC-2, ERR-1, MAN-1.

**Files:**
- Modify: `op-registry-derive/src/ops/{parse.rs,expand.rs}`, `op-registry/src/registry.rs` (`common_errors`, `merge`, `OpSpec.error_exits`), `op-registry/src/descriptor.rs` (`Exit.on`), `op-registry/src/cli/run.rs` (`Outcome::Err.exit`), `op-registry/src/exit.rs`, `op-registry/src/manual.rs`, `op-registry/src/testing/codes.rs`
- Create: `ops_ui/ops_errors_twice_in_header_is_a_compile_error.rs`, `ops_exit_number_twice_is_a_compile_error.rs`, `ops_exit_on_without_string_is_a_compile_error.rs` (+ `.stderr`)
- Test: `op-registry/tests/ops_macro.rs`, `op-registry/tests/registry.rs`, `op-registry/tests/runner.rs`, `op-registry/tests/manual.rs`, `op-registry/tests/testing_kit.rs`

**Interfaces:**
- Produces: header `errors = [A, B];` → `registry.add_common_errors(..)` with the same `CodeCatalog::codes()` expansion the entry key uses; `add_common_errors(&mut self, Vec<CodeEntry>)`, `common_errors(&self) -> &[ErrorCodeEntry]`; `merge` unions them and panics on one code with two classes naming both. Codes an op may return = own ∪ common ∪ its cap code (strict codes, `assert_error_codes`, `assert_exit_table`).
- `exits: { 3 => "timed out" on "grorg.client.wait_timeout" }` → `OpSpec.error_exits: Vec<(u8, &'static str, &'static str)>`; `Exit { code, meaning, on: Option<String> }`; `Outcome::Err { op, error, message, exit }`; `Outcome::exit_code()` returns it. `OkExit` is needed only with ok exits; error exits allowed on lists. CLI: matching code exits with the number, error line still written. `assert_exit_table` fails an `on` code not among the op's codes.
- Manual: "## Common codes" once after the exit table; op tables list own codes then "Also the common codes."; exit line "- 3: timed out (on grorg.client.wait_timeout)"; the code table shows 3 for that code. `render_json` gains `commonCodes`; each op's `codes` holds its own.

- [ ] **Step 1: Write the failing tests:** `header_code_accepted_by_strict_for_every_op`; `code_in_header_and_entry_listed_once`; `merge_unions_common_errors`; `merge_class_clash_panics`; `error_exit_code_exits_3_with_error_line`; `post_run_sees_exit_3`; `other_code_exits_by_class`; `manual_error_exit_line_and_table`; `manual_common_codes_once_and_json_common_codes`; `undeclared_on_code_fails_exit_table`; three UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test ops_macro --test registry --test runner --test manual --test testing_kit --test ops_ui`
- [ ] **Step 3: Implement**; re-render `tests/manual_snapshot.md`.
- [ ] **Step 4: Run green**, same command; `cargo-slot cargo check -p op-registry -p op-listen -p op-status --all-features --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: common error lists; error-code exits; common codes printed once`

---

### Task 25: SEC-1 one shared Redactor per call reaching every output site (with SEC-2 `redact`, SEC-3 tests)

Spec: SEC-1, SEC-2 (the `CallOptions::redact` half), SEC-3 (erratum 6). **Risk task:** a site that keeps a copy leaks a secret registered mid-call.

**Files:**
- Modify: `op-registry/src/params/mod.rs` (`Redactor`), `op-registry/src/call.rs` (`CallOptions` private `redactor`, `redact`), `op-registry/src/invoke.rs`, `op-registry/src/notices.rs`, `op-registry/src/hook.rs`, `op-registry/src/cli/run.rs`, `op-registry/src/mcp/{mod.rs,rmcp.rs}`, `op-registry/src/output.rs`
- Test: `op-registry/tests/redaction.rs`

**Interfaces:**
- Produces `Redactor` exactly as spec SEC-1 (`Arc<RwLock<Vec<String>>>`; `Clone` shares; `new`, `add(&self)`, `extend(&self)`, `redact`, `is_empty`, `detached`). `add` keeps today's candidates (whole value, each trimmed line, `MIN_LEN` 4).
  ```rust
  impl CallOptions { pub fn redact(&self, bytes: &[u8]); }   // adds to the call's handle; a CallOptions clone shares it
  ```
- The pipeline makes one redactor per call (params' secrets, plus the CLI's secret globals) and puts the same handle in: `CallOptions`, `Invocation`, `CallInfo`, the notice wrapper (always wrapped when a sink exists; redacts at send time), `CallFailure` (`call`, `call_tool`, typed, streams), `tool_error`, the CLI error line (redacts at write time), `Outcome::Err.message` (computed after the op). Usage errors before params exist keep the argv scrub. Docs: "register a secret before you print anything that may contain it".

- [ ] **Step 1: Write the failing tests** in `tests/redaction.rs`, all with an op that declares no secrets and calls `opts.redact(b"s3cr3t-value")`, then sends a notice and fails with an error, both containing it:
  - `cli_error_line_notice_and_outcome_lack_secret` (recording `RunHooks`).
  - `hook_on_error_redact_lacks_secret` (recording `CallHook`, `info.redact(..)`).
  - `call_failure_display_and_redacted_message_lack_secret`; `call_tool_and_tool_error_lack_secret`; `call_typed_failure_lacks_secret`; `stream_item_error_lacks_secret` (via `call_stream`).
  - Review Focus: `mapped_error_is_redacted` (an ERR-2 mapper that formats the secret into a new error); `stream_item_error_on_cli_is_redacted`; `typed_caller_sink_notice_is_redacted`; `options_clone_shares_redactor` (op clones `opts` into a helper before calling `redact` on the original).
  - `clone_before_add_sees_it_detached_does_not`.
  - SEC-3: `secret_file_with_trailing_newline_redacted` (stdin `"sk-abc123\n"`, error containing `sk-abc123`); `secret_with_blank_lines_redacted` (`"\n\nsk-abc123\n\n"`).
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test redaction`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command plus `--test secrets --test call --test mcp --test runner`; `cargo-slot cargo check -p op-registry -p op-listen -p op-status --all-features --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: one shared redactor per call reaching every output site; CallOptions::redact`

---

### Task 26: OUT-8 typed stream that outlives the borrow

Spec: OUT-8.

**Files:**
- Modify: `op-registry/src/invoke.rs`
- Test: `op-registry/tests/call.rs`

**Interfaces:**
- Produces `call_stream_typed_owned` exactly as spec OUT-8 (holds both `Arc`s; same path check, validate and hook around the open as `call_stream_typed`).

- [ ] **Step 1: Write the failing test** `owned_stream_survives_dropped_handles`: `tokio::spawn` the stream after dropping the caller's registry and context `Arc`s; it yields every item. Also `owned_stream_checks_path` (`[cli]` op is `wrong_path`).
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test call owned_`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command.
- [ ] **Step 5: Commit.** `op-registry 0.4: call_stream_typed_owned`

---

### Task 27: SEC-7 `Io` and `Prompter`, SEC-6 stdin hint, SEC-9 durable `write_private`

Spec: SEC-7, SEC-6, SEC-9.

**Files:**
- Modify: `op-registry/Cargo.toml` (`rpassword = { version = "7", optional = true }`; `prompt = ["secrets", "dep:rpassword"]`; `default = ["schema", "manual", "secrets", "prompt"]`), `op-registry/src/cli/{io.rs,run.rs,parse.rs}`, `op-registry/src/params/mod.rs` (`ParseCx` private fields), `op-registry/src/params/rt.rs` (stdin reads send the hint), `op-registry/src/output.rs`
- Test: `op-registry/tests/secrets.rs`, `op-registry/tests/params_inputs.rs`, `op-registry/tests/output.rs`, `op-registry/tests/secrets_off.rs`

**Interfaces:**
- Produces (under `secrets`): `pub trait Prompter: Send + Sync { fn read_secret(&self, label: &str) -> std::io::Result<Zeroizing<String>>; }`. `Io` becomes `#[non_exhaustive]` with `stdin_tty: bool` and (under `secrets`) `prompter: Option<Arc<dyn Prompter>>`; `Io::with_stdin_tty(self, bool)`, `Io::with_prompter(self, Arc<dyn Prompter>)`; `Io::memory` signature unchanged (stdin_tty false, no prompter); `Io::std()` sets `stdin_tty` from the process and, under `prompt`, a private `TtyPrompter` (rpassword, prompt written to the terminal) when stdin and stderr are terminals. `ParseCx` carries the prompter, `stdin_tty` and the hint sink (private).
- SEC-6: a field reading stdin (`Input -`, `Json @-`, `SecretFile -`) while `stdin_tty` writes one notice first: "reading --<flag> from stdin (end with Ctrl-D)" through the run's notice sink.
- SEC-9: `write_private` creates missing parents (0700 on unix for each it creates), writes, `sync_all`, fsyncs the parent dir on unix; 0600 and no-overwrite unchanged.

- [ ] **Step 1: Write the failing tests:** `stdin_hint_once_when_tty`; `no_hint_when_not_tty`; `hint_never_contains_bytes`; `memory_io_has_no_prompter_and_no_tty`; `write_private_creates_parent_0700_file_0600`; `write_private_refuses_existing_without_overwrite`. (The build without `prompt` is checked in Step 4.)
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test secrets --test params_inputs --test output`
- [ ] **Step 3: Implement.** Before adding rpassword, read its `license` in the fetched crate's `Cargo.toml` (expected Apache-2.0); stop and report if it is not permissive.
- [ ] **Step 4: Run green**, same command; `cargo-slot cargo check -p op-registry --no-default-features --features manual`; `cargo-slot cargo check -p op-registry --no-default-features --features secrets`; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: Io with stdin_tty and Prompter, prompt feature, stdin hint, durable write_private`

---

### Task 28: SEC-4 CLI prompts for a missing secret, SEC-5 `prompt_secret`

Spec: SEC-4, SEC-5.

**Files:**
- Modify: `op-registry-derive/src/params/{attrs.rs,clap_gen.rs,mod.rs}`, `op-registry/src/params/rt.rs`, `op-registry/src/call.rs` (`CallOptions` private `prompter`), `op-registry/src/cli/run.rs` (puts the prompter into `CallOptions`)
- Create: `params_ui/op_params_prompt_on_option_is_a_compile_error.rs`, `op_params_prompt_on_non_secret_file_is_a_compile_error.rs` (+ `.stderr`)
- Test: `op-registry/tests/secrets.rs`

**Interfaces:**
- Consumes: `Prompter`, `Io::with_prompter` (Task 27); shared `Redactor` (Task 25); `FieldMeta.prompt` (Task 5).
- Produces: `#[op(prompt)]` / `#[op(prompt = "API key")]` on a plain `SecretFile` (label defaults to the flag name; `FieldMeta.prompt = Some(label)`); the clap arg is not required; an absent prompted field asks `cx` prompter; bytes become `SecretFile::new(None, bytes)` and are registered. No prompter: `.missing` with "(no terminal to ask on)"; read error: `.invalid` "cannot read from the terminal: .."; empty answer: `.missing` with "(empty answer)". Help gains "Asked for on a terminal when left out." JSON/MCP/typed: absent is `.missing`, schema keeps `required`. Derive errors from spec SEC-4.
  ```rust
  impl CallOptions { pub fn prompt_secret(&self, label: &str) -> Option<Secret<Zeroizing<String>>>; }   // under `secrets`; None without `prompt`
  ```
  On the CLI with a prompter: reads one line, registers it (and its trimmed form via `add`), returns it without the line ending; empty answer → `None`; read error → `None` plus one notice "cannot read from the terminal: ..". Everywhere else `None`.

- [ ] **Step 1: Write the failing tests** (fake prompter recording its calls): `missing_prompted_flag_asks_with_label`; `prompted_bytes_reach_op_and_are_redacted`; `no_prompter_is_missing_exit_2`; `key_file_dash_reads_stdin_without_asking`; `given_path_does_not_ask`; `json_without_key_is_missing_and_prompter_unused` (also MCP and typed); `prompt_empty_answer_is_missing`; `prompt_secret_on_cli_returns_value_and_redacts`; `prompt_secret_off_cli_is_none` (`call`, `call_tool`, `call_typed`); `prompt_secret_no_prompter_is_none`; `prompt_secret_empty_answer_is_none`; `prompt_secret_read_error_sends_notice`; two UI tests.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test secrets --test params_ui`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; `cargo-slot cargo check -p op-registry --no-default-features --features manual`; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: #[op(prompt)] secrets asked on a terminal; CallOptions::prompt_secret`

---

### Task 29: KIT-5 the secret sentinel (with SEC-2's kit side)

Spec: KIT-5, SEC-2.

**Files:**
- Modify: `op-registry/src/testing/secrets.rs`
- Test: `op-registry/tests/secrets.rs`, `op-registry/tests/testing_kit.rs`

**Interfaces:**
- Produces `SecretSentinel` exactly as spec KIT-5 (`#[non_exhaustive]`, `new(value, key_file)`, `with_globals(&[&str])`). `assert_secrets_not_echoed` (signature unchanged): globals right after the binary name in every argv; a `#[op(secret)]` `Path` field gets `key_file`'s path; a prompted `SecretFile` also runs once with the flag left out and a fake prompter answering `value`; a recording `RunHooks` adds every `on_usage_error` message and `post_run` `Outcome` message to the searched places; it checks absence only.

- [ ] **Step 1: Write the failing tests:** `sentinel_globals_reach_argv`; `secret_path_op_reading_and_registering_passes`; `secret_path_op_echoing_contents_fails`; `prompted_field_exercised_with_fake_prompter`; `outcome_message_with_sentinel_fails`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test secrets --test testing_kit`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: SecretSentinel with globals; kit covers key files, prompts and outcomes`

---

### Task 30: STR-1 stop codes, STR-2 per-topic stops, STR-3 typed followers

Spec: STR-1, STR-2, STR-3 (erratum 3).

**Files:**
- Modify: `op-listen/src/{follow.rs,frame.rs,lib.rs}`, op-status call sites of `supervise`
- Test: `op-listen/tests/{follow_items.rs,supervise.rs,follow_typed.rs}`

**Interfaces:**
- Produces: `FollowOptions.stop_codes: Vec<String>` (default empty); `supervise(connect, opts, sink)` (parameter removed). `supervise` and `follow_items` deliver a stop item only as `Stopped { code }`; `follow` and `follow_typed` keep the stop frame then the stopped marker. In every follower, a mid-stream error whose code is in `stop_codes` ends `Stopped { code }` before the Logical check; otherwise Logical ends `Err`, other classes retry. Terminality comes only from `Resumable::stop_code()`.
  ```rust
  impl<S, I> Resumable for ListenFrame<S, I> { type Cursor = ListenCursor; /* cursor(), stop_code() = Stop's code, resync() = Snapshot.resync */ }
  pub async fn supervise_stream<T, F, K>(connect: F, opts: FollowOptions<T::Cursor>, sink: K) -> anyhow::Result<FollowEnd>;  // bounds as spec STR-3
  ```

- [ ] **Step 1: Write the failing tests:** `stop_item_delivered_once_as_stopped` (supervise and follow_items); `listed_mid_stream_code_ends_stopped`; `unlisted_logical_ends_err`; `unlisted_transient_retries`; `per_topic_stop_variant_flows_and_follow_continues` then a terminal stop ends it; `position_like_frame_advances_cursor_and_is_delivered` (an item type with a variant whose `cursor()` is `Some`, `stop_code()` `None`; the reconnect uses its cursor); `typed_frames_resume_from_last_cursor`; `typed_stop_frame_ends_with_code`; `stream_end_reconnects`; `supervise_stream_delivers_opened_and_retrying`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-listen --features testkit --test follow_items --test supervise --test follow_typed`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; `cargo-slot cargo check -p op-listen -p op-status --all-features --tests`.
- [ ] **Step 5: Commit.** `op-listen 0.4: stop_codes in FollowOptions honoured by every follower; Resumable for ListenFrame; supervise_stream`

---

### Task 31: STR-4 a reconnecting follower as a stream op

Spec: STR-4 (erratum 4). Needs Task 1's `max_retries`.

**Files:**
- Modify: `op-listen/src/{follow.rs,entry.rs,listener.rs,error.rs,lib.rs}`
- Test: `op-listen/tests/follow.rs`, `op-listen/tests/registry.rs`, `op-listen/tests/testkit.rs`

**Interfaces:**
- Consumes: `CallOptions::max_retries` (Task 1), `stop_codes` and `Resumable for ListenFrame` (Task 30).
- Produces `Followed<F>`, `follow_stream`, `listen_entry_following`, `Listener::reconnect_marker` (default `{"marker":"reconnected","cursor":<c or null>,"resync":<b>}`), `Listener::follow_options` (default `FollowOptions::default()`), exactly as spec STR-4; `ListenError::Stopped { code }`, code `op_listen.stopped`, Logical. `follow_stream` behaviour is spec STR-4 items 1-4, plus: a failed open is retried as Transient up to `max_retries`; the marker is emitted only before the first item of a reconnection (no marker for a connection that yields nothing); a connection that ends with no item counts as a consecutive failure. `listen_entry_following` passes params and `--cursor` unchanged and uses `opts.max_retries` when set, else `listener.follow_options().max_retries`.

- [ ] **Step 1: Write the failing tests** in `op-listen/tests/follow.rs`: `marker_before_first_frame_of_reconnection` (connect fails twice, then serves); `max_retries_zero_ends_after_first_failure`; `stop_item_yields_frame_once_then_stopped_error`; `listed_mid_stream_code_ends_with_stopped`; `grorg_marker_override_printed_verbatim` (`{"marker":"reconnected","afterSeq":7}`); `resnapshot_overflow_under_follow_stream`; `failed_open_is_retried`; `empty_connections_print_no_marker`; `empty_connections_count_toward_max_retries`. `registry.rs`: `following_entry_prints_ndjson_and_exits_65_after_stop`; `following_entry_passes_params_and_cursor`. `testkit.rs`: `kit_failing_follower_ends_after_one_attempt` (`assert_error_codes` over a following op whose connect always fails returns without the 10 s timeout).
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-listen --features testkit --test follow --test registry --test testkit`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same command; `cargo-slot cargo check -p op-listen -p op-status --all-features --tests`.
- [ ] **Step 5: Commit.** `op-listen 0.4: follow_stream and listen_entry_following with a reconnect marker hook`

---

### Task 32: STR-5 op-listen `registry` feature, STR-6 op-status remote snapshot and `GROUP_SUMMARY`

Spec: STR-5, STR-6.

**Files:**
- Modify: `op-listen/Cargo.toml` (`default = ["registry", "detach"]`, `registry = ["dep:op-registry", "dep:schemars"]`, `detach = ["registry", "tokio/rt"]`, `testkit = ["detach"]`; `op-registry` and `schemars` optional), `op-listen/src/{lib.rs,cursor.rs,frame.rs}` (`cfg`/`cfg_attr`), `op-status/Cargo.toml` (`listen = ["registry", "op-listen/registry"]`), `op-status/src/{source.rs,ops.rs,lib.rs}`
- Test: `op-listen/tests/*` (gate registry-only tests with `#![cfg(feature = "registry")]` or `required-features`), `op-status/tests/ops.rs`

**Interfaces:**
- Under `registry`: `entry`, `listener`, `run`, `detach`, the `Caller` re-export, `ListenCursor`'s `TryFrom<&op_registry::Cursor>` and `JsonSchema` impls, `ListenFrame`'s `JsonSchema` derive. Core without it: `follow`, `backoff`, `cursor`, `frame`, `error`, `stdout`. `follow_stream` and `Followed` are core (no op-registry use); `Followed`'s `JsonSchema` derive is `cfg_attr(feature = "registry")`.
- op-status: `StatusSource::remote_snapshot` and `pub const GROUP_SUMMARY: &str = "Component status.";` exactly as spec STR-6; `take_snapshot` prefers `remote_snapshot` when `Some`; `status_registry` uses `GROUP_SUMMARY`.

- [ ] **Step 1: Write the failing tests:** `op-status/tests/ops.rs`: `remote_snapshot_returned_unchanged_by_status_get`; `merge_with_group_from_group_summary_passes`. op-listen: the existing `follow`/`follow_typed` tests must run with `--no-default-features`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-status --all-features --test ops` and `cargo-slot cargo test -p op-listen --no-default-features --test follow --test follow_typed`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**, same commands; `cargo-slot cargo check -p op-listen --no-default-features`; `cargo-slot cargo check -p op-status`; `cargo-slot cargo check -p op-status --features listen`; `cargo-slot cargo tree -p op-listen --no-default-features -e normal,features -i tokio | grep 'tokio feature "rt"'` (no output); `cargo-slot cargo tree -p op-listen --no-default-features -i op-registry` (fails: not in the tree).
- [ ] **Step 5: Commit.** `op-listen 0.4: registry feature; op-status remote_snapshot and GROUP_SUMMARY`

---

### Task 33: MAN-2 other commands in the manual, MAN-5 relations, needs, env and prompts shown

Spec: MAN-2, MAN-5.

**Files:**
- Modify: `op-registry/src/manual.rs`, `op-registry/src/params/mod.rs` (`OpParams::relations`), `op-registry/src/descriptor.rs` (`relations`), `op-registry/src/registry.rs` (`describe`), `op-registry/src/cli/tree.rs` (`after_long_help`), `op-registry-derive/src/params/mod.rs`
- Test: `op-registry/tests/manual.rs`, `op-registry/tests/cli_tree.rs`

**Interfaces:**
- Produces: `fn relations() -> Vec<String> { Vec::new() }` on `OpParams`; the derive emits one plain line per relation: "--message is required unless --signed-file is given", "--to conflicts with --signed-file", "give exactly one of --id, --action", "give at most one of ..", "give at least one of ..", "--x is required when --y is given" (flattened groups' lines included); `Descriptor.relations: Vec<String>`; the op's `--help` ends with them as `after_long_help`; the manual prints them under "Rules:". Manual "## Other commands": each extra's name, `about`, long about; the fallback's name and full help; marked "CLI only, not an op"; `render_json` gains `otherCommands`.

- [ ] **Step 1: Write the failing tests:** `relations_lines_for_each_kind`; `help_ends_with_rules`; `manual_prints_rules`; `help_holds_needs_env_and_prompt_lines`; `manual_lists_extra_and_free_text_fallback`; `manual_json_other_commands`.
- [ ] **Step 2: Run red.** `cargo-slot cargo test -p op-registry --all-features --test manual --test cli_tree`
- [ ] **Step 3: Implement**; re-render `tests/manual_snapshot.md`.
- [ ] **Step 4: Run green**, same command; crate `check --tests`.
- [ ] **Step 5: Commit.** `op-registry 0.4: manual shows other commands and each op's rules`

---

### Task 34: Release 0.4.0: version, CHANGELOG, README, v2 spec, full gates

Spec: section 7, DOC-1, DOC-2, DOC-3, PAR-2's README line.

**Files:**
- Modify: `Cargo.toml` (`workspace.package.version = "0.4.0"`), `op-registry/Cargo.toml`, `op-listen/Cargo.toml`, `op-status/Cargo.toml` (path-dependency `version = "0.4.0"`), `Cargo.lock`, `CHANGELOG.md`, `README.md`, `op-listen/README.md`, `op-status/README.md` (if it names `supervise` or the summary), `docs/superpowers/specs/2026-11-20-op-registry-v2-design.md`, `scripts/check.sh` (its coded-error pin guard names `ffeaf39..`, but the pin is `de21d22e31baff78eae7d4ca817b87430bff1f81`; correct the guard)

**Content:**
- CHANGELOG `## 0.4.0`: "Breaking. All four crates move to 0.4.0." Then "### Breaking signatures and types": spec section 7 items 1-16 verbatim in substance, plus: `SubjectMeta { kind, id_only, absent_ok }`; `#[op(subject, absent_ok)]` (not `new`); `call_typed`/`call_stream_typed`/`call_stream` return `CallFailure` (use `.map_err(CallFailure::into_coded)?`); `Io` gains `stdin_tty` and `prompter`; `default` features gain `prompt`. "### Additive": spec section 7's list. "### Behaviour changes": spec section 7's list, plus erratum 4 (`follow_stream` retries a failed open), the `paths: [cli]` meaning change ("write `[cli, typed]` for the old meaning"), and "a param `env` is read on the CLI only; typed, JSON and MCP callers pass the value".
- README (op-registry): paths subsets; `{"items"}`; object roots; relations incl. presence, dotted targets, `required` on Vec/Option, struct `validate`; optional and `absent_ok` subjects, `check_id`; "an absent optional scope member is left out of the `Scope`" (PAR-2); `from_str`, `sample`, `allow_hyphen`, `ignore_case`, defaults in help, `env` (CLI only); secrets: `opts.redact`, `#[op(secret)]` key-file paths, "register a secret before you print anything that may contain it", "a secret-reading op is not a list", prompts and `prompt_secret`, the `prompt` feature; `cli::select` host pattern (FB-5); `set_error_mapper`; error exits; common errors; DOC-2 (the `schema` feature is not additive across a workspace build; build `gait-min` with `cargo build -p gait-min`).
- op-listen README (DOC-1): `CliContext::caller()` must return `Some` for listen ops on the CLI; `status_registry` needs a `StatusSource`; `listen_entry_following` and the marker override (grorg's `afterSeq` example); STR-2's rule (only `stop_code()` ends a follow; per-topic stops are item variants; Position-style frames advance the cursor); `stop_codes` is for mid-stream errors; the `registry` feature.
- v2 spec (DOC-3): every item in spec DOC-3's list, with "PATH-1" text for section 4.4's path rule and `absent_ok` naming.

- [ ] **Step 1: Bump versions and write the docs** as above.
- [ ] **Step 2: Run the full gates**, once each, output to a file, then grep it:
  - `cargo-slot cargo fmt --check`
  - `cargo-slot cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo-slot cargo test --workspace --all-features`
  - `PATH=/Users/aicoder/Documents/systematic-action/scripts:$PATH bash scripts/check.sh` (runs the pin and tokio guards and the per-feature checks through cargo-slot)
  Expected: all pass; `check: ok`. Fix only failures and rerun only the failing command.
- [ ] **Step 3: Commit.** `op-registry 0.4.0: version, CHANGELOG, README, v2 spec`
