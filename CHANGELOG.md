# Changelog

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
