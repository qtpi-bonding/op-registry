# Changelog

## 0.4.0

Breaking. All four crates move to 0.4.0. There is no backward compatibility:
removed items are removed, with no shims. The changes are grouped below;
the spec is `docs/superpowers/specs/2026-11-20-op-registry-0.4.0-design.md`.

New dependencies: `rpassword` 7 (optional, feature `prompt`, which is in
`default`); `tokio` with `time` under the `testing` feature; `jsonschema` as a
dev-dependency only; `serde_json`'s `preserve_order` feature. `default`
features are now `schema`, `manual`, `secrets`, `prompt`.

### Breaking signatures and types

1. `Paths`: the enum `{All, TypedOnly, CliOnly}` becomes a set with
   `Paths::ALL` and `Paths::of(&[Surface])`; the new enum `Surface` is
   `Cli`, `Typed`, `Mcp`, `Json`. `OpSpec.cli_only_reason` and
   `Descriptor.cli_only_reason` are removed, and so is the `ops!` key
   `reason:`. `paths: [cli]` now excludes `typed`.
2. `Registry::push_extra(command, fn(&Globals, &ArgMatches) ->
   BoxFuture<anyhow::Result<u8>>)`.
3. `Registry::fallback_flags_only(name, help, fn(Vec<OsString>) ->
   BoxFuture<'static, anyhow::Result<u8>>)`.
4. `Registry::fallback_free_text(name, help, &[FallbackFlag], near_miss:
   bool, fn(&Globals, Vec<OsString>) -> BoxFuture<anyhow::Result<u8>>)`.
5. `Registry::call_typed` returns `Result<O::Output, CallFailure>`.
6. `Registry::call_stream_typed` returns
   `Result<BoxStream<Result<O::Output, CallFailure>>, CallFailure>`.
7. `Registry::call_stream` returns
   `Result<BoxStream<Result<Value, CallFailure>>, CallFailure>`. A bare `?`
   into `anyhow::Error` loses the code (the failure has no `source`, on purpose):
   use `.map_err(CallFailure::into_coded)?`. `failure.error()` downcasts to the
   op's own error type.
8. `FieldMeta` is `#[non_exhaustive]` with a constructor and new fields
   (`env`, `subject`, `required`, and others); a hand-written `OpParams` or
   `OpGlobals` changes.
9. `FieldKind` gains `Path` and `Patch`: `PathBuf` fields report `Path` and
   `Patch<T>` fields report `Patch`.
10. `testing::SecretSentinel` is `#[non_exhaustive]` with `new` and
    `with_globals`.
11. `cli::Io` is `#[non_exhaustive]` and gains `stdin_tty` and `prompter`.
    `Io::memory` keeps its signature; `Io::with_stdin_tty` and
    `Io::with_prompter` are new.
12. `Redactor` is a shared handle: `Clone` shares the list, `add` and `extend`
    take `&self`, and `Redactor::detached` makes an unshared one.
13. `cli::Outcome::Err` gains `exit: u8` (exhaustive patterns break).
14. `Exit` gains `on: Option<String>` (struct literals break).
15. op-listen: `supervise(connect, opts, sink)` loses its `stop_codes`
    parameter, which moves to `FollowOptions::stop_codes`; `FollowOptions` stays
    `#[non_exhaustive]`.
16. A struct-level `#[op(..)]` on an `OpParams` struct is parsed: unknown keys
    that were silently ignored now fail to compile. The keys are `validate` and
    `allow_hyphen`.

Also:

- `SubjectMeta` is `{ kind, id_only, absent_ok }`. `#[op(subject = "..",
  absent_ok)]` marks a subject whose id may not exist (a create, an ensure, an
  idempotent probe). It is never resolved (the op gets `Resolved::Id(raw)`),
  `Resolve::check_id` still runs, and the kit's not_found test skips it. It may
  be combined with `id_only`; on a non-subject it is a derive error. (The spec
  drafts called it `new`; the shipped name is `absent_ok`.)
- `UsageError::MissingUnless` is a tuple variant `MissingUnless(MissingUnless)`
  wrapping `pub struct MissingUnless { name, unless }` (code
  `op_registry.usage.missing`): match it as `UsageError::MissingUnless(m)`. The
  pinned coded-error derive rejects two variants with one code, which is why it
  is not a struct variant. `UsageError` gains variants and `ListenError` gains
  `Stopped`; the new `ListError` is `#[non_exhaustive]`.

### Additive

- `OpSpec` fields `more_audiences`, `needs_globals`, `error_exits` and
  `follow_page` (use `..OpSpec::new()`); `Descriptor` fields `audiences`,
  `needs_globals` and `relations` (non-exhaustive).
- `Registry::add_common_errors`, `common_errors`, `set_error_mapper`,
  `call_stream_typed_owned`, `fallback_flags_only_before_runtime`;
  `ops!` header key `errors = [..]`; `#[cfg(..)]` on `ops!` entries; the entry
  key `globals: [..]`; `audience: [agent, owner]`; list key `follow_page`;
  `text: true` on a list entry.
- `cli::select`, `Selected`, `FallbackFlag`, `Prompter`, `RunHooks::on_selected`.
- `CallOptions::redact`, `prompt_secret`, `max_retries`, `with_max_retries`.
- `CallFailure: CodedError`, `CallFailure::into_coded`.
- `Resolve::check_id`; `OpParams::relations`; `usage()` is documented public.
- Params: `#[op(from_str)]`, `#[op(sample = "..")]`, `#[op(allow_hyphen)]`
  (field or struct), `#[op(value_enum, ignore_case)]`, `#[op(default = .., hide_default)]`,
  `#[op(env = "X")]` on params, `#[op(prompt)]` / `#[op(prompt = "label")]` on a
  `SecretFile`, `#[op(secret_inline)]`, `Patch<Ref>`, an optional subject
  (`Option<Ref>`), scope and `id_only` on non-subject refs.
- Relations: members of any shape with presence; `conflicts_with`,
  `required_unless`, `at_least_one`; `required` on `Vec<T>` and `Option<T>`;
  dotted targets through a flatten group; a struct-level `#[op(validate = f)]`.
- `RenderText for Vec<T>`.
- `UsageError::{Conflicts, MissingUnless, AtLeastOne}`; library codes
  `op_registry.usage.conflicts`, `op_registry.usage.at_least_one`,
  `op_registry.list.cursor_repeated`, `op_listen.stopped`.
- Relation errors by surface: on the CLI, relation violations are reported by
  clap as `op_registry.usage.invalid`/`.missing` (exit 2); JSON, MCP and typed
  callers get `.conflicts`/`.one_of`/`.at_least_one`/... (also exit 2).
- `RegistryServer::with_ctx_factory` and `with_instructions` (feature `rmcp`).
- `testing::sample_params_seeded`.
- op-listen: `Listener::reconnect_marker` and `follow_options`; `follow_stream`,
  `Followed`, `listen_entry_following`, `supervise_stream`, `Resumable for
  ListenFrame`, `ListenError::Stopped`; a `registry` feature (default on):
  op-listen builds with `default-features = false` and no op-registry.
- op-status: `StatusSource::remote_snapshot`, `op_status::GROUP_SUMMARY`.
- A `prompt` feature (default on): echo-off prompts through `rpassword` 7.

### Behaviour changes

- Every array output is `{"items": [...]}` on the CLI, `call`, `call_tool`,
  `outputSchema` and the manual. `$defs` and `definitions` are hoisted so every
  `$ref` resolves. An output typed `Option<Vec<T>>` is not wrapped. The
  typed path returns the Rust `Vec<T>`.
- A root `oneOf` or `anyOf` of objects gets `"type": "object"`. On MCP, only an
  object root has an `outputSchema`; any other output is a text block with the
  compact JSON.
- JSON keys print in declaration order (`preserve_order`); goldens and manual
  snapshots move once.
- Relation schemas change shape and match what `validate` accepts;
  `dependentRequired` is gone. A relation counts presence: a `false` switch, an
  empty `Vec` or map and `null` are absent. A required `Option<bool>` that is
  omitted is `op_registry.usage.missing` (it was silently `false`).
- Relation help lines name every enforced spelling: "--x or --no-x" for a
  tri-state, "--x or --clear-x" for a `Patch` (MAN-5). The help, the manual
  ("Rules:") and the needs line list relations.
- `paths` is a literal subset (see breaking item 1): `paths: [cli]` meant the
  CLI plus typed in 0.3.x and now means the CLI only; write `[cli, typed]` for
  the old meaning. The path is checked in `call_typed` and `call_stream_typed`
  too.
- A param `env` is read on the CLI only; typed, JSON and MCP callers pass the
  value. An op with an `env` param may be on any paths. An empty variable counts
  as absent.
- `follow_stream` (and every follower) retries a failed open as Transient, up
  to `max_retries`. Consumers that never retried an open (multimatrix
  `agent-room`) now do.
- Every follower now prints/yields the reconnect marker before the first item
  after a failed first open (previously only after a successfully opened earlier
  connection). A connection that yields nothing gets no marker. A listed stop
  code (mid-stream or on open) ends `follow_stream` and the following entries as
  `op_listen.stopped { code }` (Logical, exit 65). A reconnecting follower
  exits 65 after a stop. `supervise` and `follow_items` no longer deliver a stop
  item as an item.
- The kit runs every op it can reach, on a surface the op declares
  (CLI-only ops included; typed-only ops are skipped and named), tests
  `id_only` subjects for not_found, reports samples that did not decode, bounds
  stream calls (first item or end, 10 s), and checks parity exactly with
  object output roots. Suites that passed vacuously turn red until they have
  fixtures. The kit's stream checks need a tokio runtime with time
  (`#[tokio::test]`); under another executor they report a problem.
- Secrets: one shared redactor per call reaches the CLI error line, stream item
  errors, notices and the `opts` clones; a nested op call shares the outer
  call's redactor through `opts`. A secret an op reads itself is registered with
  `opts.redact(..)`: register it before anything that may contain it is
  printed. `Redactor::add` registers the untrimmed whole value plus each
  trimmed line (values under 4 bytes are skipped).
  `#[op(secret)]` on a `PathBuf` marks a key-file path. A secret-reading op is
  not a list (list page functions get no `CallOptions`). A `SecretFile` on an
  op reachable on `mcp` needs `#[op(secret_inline)]` or leaves `mcp` out of its
  paths (`assert_parity` checks it).
- On the CLI a missing prompted `SecretFile` is asked for through the run's
  prompter: the echo-off terminal prompter when stdin and stderr are terminals
  (feature `prompt`), or one installed with `Io::with_prompter` (any `secrets`
  build). With no prompter it is `op_registry.usage.missing`. Without the
  `prompt` feature a real terminal installs no prompter, so only a
  host-installed one is asked. `CallOptions::prompt_secret` returns `None`
  unless the run has a prompter (a nested typed call inherits it), and always
  `None` without `prompt`.
- Errors: `set_error_mapper` rewrites op and stream item errors once (usage
  codes are skipped); an error code can carry a declared exit
  (`exits: { 3 => ".." on "code" }`); `RunHooks::on_usage_error` fires for every
  exit-2 failure, op-side ones included; list guards are coded
  (`op_registry.list.cursor_repeated`, `too_many` also bounds empty pages).
- FreeText routing: an empty rest goes to the fallback; registry globals are
  taken from the whole tail up to `--`; a declared value flag keeps its value;
  the near-miss check is opt-in.
- MCP: an op can serve several audiences; a context per `tools/call` is
  possible with `with_ctx_factory`.
- The manual prints common codes once, "Other commands" (extras and the
  fallback), "Operations off the command line", a "Paths:" line and each op's
  rules. A leaf param named `version` is help-checked.

## 0.3.2

Additive. All four crates move to 0.3.2.

- `#[op(scope = [..])]` may name a field of a flattened group by a dotted path
  from the op's params struct: `scope = ["project.root", "store.gcode_store"]`.
  The first segment must be a `#[op(flatten)]` field (else a compile error naming
  the path); a missing field is rustc's own error, pointing at the path literal.
  Paths may go through several groups.
- The `Scope` key is the leaf field's camelCase name (`root`, `gcodeStore`) for
  every entry, direct or dotted, so a resolver cannot tell them apart. Two
  entries with the same leaf are a compile error. This also makes a direct
  multi-word entry (`"gcode_store"`) arrive as `gcodeStore` (it was
  `gcode_store`); single-word names are unchanged.
- `PathBuf` and `Option<PathBuf>` fields can be scope values. A path that is not
  UTF-8 fails the call with `op_registry.usage.invalid`, never a panic.
- `#[op(flatten, scope = [..])]` gives a subject declared inside that group the
  named scope, resolved against the whole params struct.
- `OpParams::ref_slots` now returns `anyhow::Result<Vec<RefSlot>>` so a scope
  value can fail. Only derive output and `ListParams` implement it.

## 0.3.1

Additive. All four crates move to 0.3.1.

- `paths: [cli]` in `ops!` (and `Paths::CliOnly` with `OpSpec::cli_only_reason`
  on the push API) marks an op that is on the CLI and the typed path and never
  an MCP tool, a `call` or `call_tool` target, or a JSON or REST mount. A
  `reason: "..."` is required: a missing one is a compile error in `ops!` and a
  panic at `push_*`. It is for ops that act with an owner key or sign; every
  other op stays on every path. The descriptor carries `cli_only_reason`.
- `assert_parity` accepts a CLI-only op (on the CLI, in no tool list or mount),
  reports it as `cli only: <reason>` on MCP, and fails if it appears in a mount.
- The generated manual marks the op `CLI-only: <reason>`.

## 0.3.0

op-registry v2. All four crates (op-registry, op-registry-derive, op-listen,
op-status) move to 0.3.0. There is no backward compatibility: removed items are
removed, with no shims, and consumers migrate once. op-registry has no auth,
identity or signing; `Caller` is an opaque label the library never checks.

### Breaking changes

op-registry and op-registry-derive, removed:

- the `registry!` macro, replaced by `ops!`;
- the `Operation` type, replaced by `OpSpec` and the `push_op`, `push_local`,
  `push_stream` and list entries;
- `CliHooks`; a params type carries its own hooks;
- the `name:` override: op names are always derived from the path;
- `#[op(positional)]`: the subject is `--id <REF>` and other objects are flags
  named by kind;
- `#[op(short)]` and `#[op(long)]`: flags are long kebab-case only; the only
  short flag is `-h`;
- `#[op(value_delimiter)]`;
- the descriptor fields `requires`, `since`, `deprecated` and `aliases`.

op-registry, changed:

- JSON keys are camelCase on every JSON path, input and output. CLI flags are
  kebab-case (`--agent-id` sets `agentId`).
- Usage errors exit 2 (they were 65). The exit table is 0 ok, 2 usage, 65
  logical, 69 infrastructure, 70 unknown, 75 transient. Ops may declare extra
  exits in 3 to 63.
- Tool and route names follow the derived name (`concept get` is `concept_get`;
  a hyphen in a noun becomes `_`).
- `hidden`, `text` and `exits` are acted on, not only declared.
- List ops come as a pair (`<name>` follows all pages, `<name>_page` returns
  one page).
- Extras and fallbacks return a `u8` exit code, not `ExitCode`.
- `Registry::call` returns `Result<Value, CallFailure>` instead of
  `anyhow::Result<Value>`, and `mcp::call_tool` fails with `CallFailure`, which
  replaces `mcp::ToolError`. Its `Display`, `Debug` and `redacted_message()`
  have the call's secrets redacted; `error()` and `into_inner()` give the raw
  error for downcasting.
- Stdin may be read by one field per call. A second read fails at run time with
  `op_registry.usage.stdin_twice`, not at build time.

op-status and op-listen, wire change:

- op-status JSON keys are camelCase: `takenAtUnixMs`, `lastErrorCode`,
  `lastErrorMessage`, `consecutiveFailures`, `changedAtUnixMs`. Every consumer
  of `status get`, `status watch` and the `StatusSnapshot` and `Component` types
  must read the new keys. State values (`OK`, `DEGRADED`, ...) are unchanged.
- op-listen's `ListenFrame` uses `rename_all_fields = "camelCase"`. Its current
  fields are single words, so the frames on the wire are unchanged; the `frame`
  tag values stay `snapshot`, `item` and `stop`.
- `listen_entry` takes an `OpSpec<L::Params, ListenFrame<L::Snapshot, L::Item>>`
  instead of a `Descriptor`.

### Added

- `ops!`: one block per binary; doc comments are the help text; kinds `op:`,
  `local:`, `stream:` and `list:`; marker types per op for typed calls; `errors:`,
  `fields:`, `exits:`, `text:`, `hidden:`, `default:`, `paths: [typed]`.
- `OpParams` v2: help from doc comments, subject `--id`, `Ref`, `Json`, `Input`,
  `SecretFile` (feature `secrets`), `Patch<T>`, `#[op(keys)]`, `#[op(value_enum)]`,
  `#[op(secret)]`, shared param groups, `oneOf` and `requires_if` relations.
- `OpGlobals`, `CliContext` and lazy contexts; `Resolve` for refs.
- The runner `cli::run` and `run_with_io`, `RunHooks`, `Io::memory`, `--pretty`
  and `--compact`, errors as JSON on stderr, notices and progress.
- Group defaults, bare top-level verbs, extra subcommands and a root fallback
  (`FlagsOnly`, `FreeText`).
- `CallHook` and `CallInfo`; `Registry::call` and `call_typed` with the `Op`
  marker trait; one pipeline for every path.
- Paged lists: `Page`, `PageRequest`, `TooMany`, the follow-all-pages pair.
- Secret redaction in the error line and notices, including secret globals
  from argv or the environment.
- `CallFailure`, the redacted failure of a JSON call; `CallInfo::redact` and
  `cli::Outcome::message()` (a redacted copy of the error) for hooks.
- The manual: `Registry::with_manual` and the `manual` op (feature `manual`).
  Its `--out` writes a file on the command line only.
- The `rmcp` feature: `RegistryServer`, serving one audience.
- The testing kit (feature `testing`): `assert_parity`, `assert_canonical`,
  `assert_casing`, `assert_help_complete`, `assert_manual_snapshot`,
  `assert_exit_table`, `assert_error_codes`, `strict_codes`,
  `assert_unknown_ref_not_found`, `assert_local_ops_offline`,
  `assert_secrets_not_echoed` and `sample_params`.
- op-listen: `supervise` with structured `Opened` (fieldless and sent as soon
  as `connect` succeeds), retry, stop, and session expiration events.
- Cargo features of op-registry: `schema`, `manual` and `secrets` (default),
  `testing` (dev-dependency only) and `rmcp`. `--no-default-features` builds.
- Library error codes: `op_registry.usage.*`, `op_registry.cli.unknown_command`
  and `op_registry.call.*`.

## 0.2.0

All four crates (op-registry, op-registry-derive, op-listen, op-status) move to
0.2.0. Breaking changes are in op-listen, op-status and op-registry. There are
no compatibility shims: removed items are removed.

### Breaking changes

op-listen:

- `FollowOptions::backoff_first` and `backoff_max` are replaced by one field,
  `backoff: Backoff`. `FollowOptions` is now `#[non_exhaustive]`: use
  `FollowOptions::default()` and assign fields. `initial_cursor` is
  `Option<ListenCursor>`.
- `ListenFrame` carries a typed `cursor: ListenCursor` instead of a string. The
  wire format (a JSON string `"epoch:seq"`) and the JSON schema are unchanged.
  `ListenFrame` is `#[non_exhaustive]`.
- `FollowEnd` is `#[non_exhaustive]` and gains `SinkClosed`: a sink that
  reports a broken pipe ends the follow with `Ok(FollowEnd::SinkClosed)`
  instead of an error. Matches need a `_` arm.
- `listen_stream` no longer takes a `ListenOptions` parameter, and
  `ListenOptions` is removed. Overflow (lag) is detected by type
  (`ListenError::Lagged`), not by error-code string.
- `ListenError` gains the variant `BadFrame { reason }` (code
  `op_listen.bad_frame`, Logical).
- Cargo features: `detach` (default; `detached_connect`, enables tokio `rt`)
  and `testkit` (implies `detach`; `op_listen::testkit`). Without default
  features tokio's `rt` is not enabled.

op-status:

- `open_status` takes `Option<op_listen::ListenCursor>`. `StatusCursorRef` is
  removed.
- op-status now depends on op-listen (and so on op-registry, tokio and the rest
  of that tree) even with default features. The feature list is unchanged
  (`registry`, `listen`).

op-registry:

- `Cursor`'s field is private. Build with `Cursor::new(text)`, read with
  `Cursor::as_str()`; `Display` is implemented. `Cursor("1:7".into())` no
  longer compiles.
- `mcp::tool_error` takes a third parameter, `fallback_message: &str`, used as
  the message of non-logical failures.
- Duplicate operation names and colliding CLI paths now panic at `push` and
  `merge`, not later in `cli::command`.
- `registry!` entries accept `op_with:` and `stream_op_with:`, whose functions
  receive the caller's `CallOptions`. `op:` and `stream_op:` are unchanged.
- op-registry now depends on coded-error (same pinned revision).

### Added

- op-listen: `Backoff` (`new`, `link`, `follower`, `with_healthy_after`,
  `next_delay`, `reset`, `connection_ended`), no jitter. The
  follower resets it after a healthy connection.
- op-listen: `follow_typed` and `FollowEvent`, over the same reconnect loop as
  `follow`. JSON `follow` output is byte-identical to 0.1.
- op-listen: `stdout_sink` and `follow_to_stdout`.
- op-listen: `ListenFrame::cursor()`.
- op-registry: `Registry::find_cli` (indexed lookup) and `Descriptor::is_path`.

### Fixed

- The derive macro reports named-field errors with `syn::Error` instead of
  panicking, and generated default-serialisation sites name the struct and field
  in their panic message. `listen_entry` no longer falls back to a null output
  schema.
- README text about `request failed` redaction and the follower overview.
