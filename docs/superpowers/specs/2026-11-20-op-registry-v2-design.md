# op-registry v2: design spec

Date: 2026-11-20. Status: revision 3 (final), after all seven lead reviews
(empeiria, structmux, catuskoti, multimatrix, archegraph, grorg, gait) and
gp's schema decisions.
Design only; no code changes.

op-registry v2 is the one library every factory CLI and MCP surface is built on.
It takes the best of the three registries we have today: op-registry v1 (grorg,
multimatrix), archegraph's `register_ops!` and catuskoti's `register_ops!`.
Scope (gp): user-facing commands only, meaning what a person or an agent types.

Inputs: the CLI audits, fit answers and reviews; op-registry at grorg's pin
(02aab58); archegraph, catuskoti and grorg sources.

## 0. Binding rules

- No auth, identity or signing in op-registry (gp, 2026-11-14). Each project's
  context does that. `Caller` stays an opaque label the library passes through
  and never checks.
- Core libs stay uncoupled. Allowed lib edge: coded-error. Not allowed:
  concord-telemetry. Tracing goes through hooks the app supplies.
- No backward compat. v1's `registry!`, `CliHooks`, the `name:` override,
  `positional`, `short` and `value_delimiter` go. Consumers migrate once.
- gait's `min` build gains no new deps. Heavy parts sit behind cargo features.
- Every path is kept and at parity (gp, 2026-11-20). An op is reachable
  in-process (typed), from the CLI, from MCP and from any transport the app
  mounts (such as REST). Some ops may be in-process only. Any CLI op must be
  structurally able to go to every other path. No path is removed.
- The library is gp's own and public (MIT OR Apache-2.0).

## 1. What v1 has today, and what it lacks

v1 has descriptors, the `OpParams` derive (clap, serde and schema from one
struct), unary and stream ops, nested noun groups, `render`, `error_line`,
`exit_code_for`, the MCP tool list and `assert_parity`, plus op-status and
op-listen. It lacks:
- clap help from doc comments (the derive never calls `.help()`; grorg's 102
  flags with no help);
- action on `hidden`, `renders_text` and `exit_code_on_ok`;
- usage exit 2 (usage errors are Logical, so they exit 65);
- globals, a context factory, local ops, hooks, a manual, a help test, a
  canonical-list test and a typed path;
- snake names for one-word hyphenated nouns (`dispatch-target_list`);
- a word that is both an op and a group (archegraph `build`);
- a runner, so every host copies the same glue.

## 2. The op model

### 2.1 Declaration

An op is declared once, in one `ops!` block per binary. Doc comments on the
entry are the help text.

```rust
op_registry::ops! {
    Ctx = Context;
    globals = Globals;
    pillar = "catuskoti";
    groups = {
        "concept" => "Concepts in a collection.",
    };
    /// Show one concept.
    ///
    /// Longer help, shown by `--help` and in the manual.
    "concept get" => { params: GetConcept, output: Concept, access: reads,
                       audience: agent, op: concept_get,
                       errors: [ConceptError], fields: concept_fields },
    /// Make a new key pair on this machine.
    "key keygen" => { params: Keygen, output: KeyPath, access: writes,
                      audience: owner, local: keygen },
    /// List concepts.
    "concept list" => { params: ListConcepts, item: ConceptSummary,
                        access: reads, audience: agent, list: concept_page },
}
```

The macro also emits one marker type per op (`ops::ConceptGet`) for the typed
path (section 4.4).

- **Path.** A string of words. The last word is the verb; the words before it
  are the noun path (`build status`). A one-word path is a bare top-level verb
  (`index`, `plan`). Taken from: v1 noun paths; archegraph's bare verbs.
- **Name.** Always derived: words joined with `_`, every `-` turned into `_`.
  No override, so the CLI, MCP, REST routes and the canonical list cannot
  disagree. Taken from: archegraph and catuskoti snake_case tool names. This
  renames some tools and routes (catuskoti `get_concept` becomes `concept_get`);
  their callers migrate in the same ship (section 12).
- **Kinds.** `op:` unary; `stream:` (5.5); `list:` paged (5.3); `local:`
  unary, async, needing no connection. A local op gets the same lazy context
  (4.1) and runs through the CallHook like any op. Taken from: grorg's
  `LOCAL_OPERATIONS`, multimatrix's policy ops; empeiria's `round --once` (git
  fetches, a telemetry flush) shows local ops must be async and traced.
- **Typed params and result.** `OpParams` in, `Serialize + JsonSchema` out.
- **Audience.** Each op names one audience. One registry per binary holds ops
  of several audiences; each MCP or REST surface serves one audience's ops. The
  CLI serves all of them. Taken from: multimatrix (owner listener, agent
  listener and local policy in one binary).
- **Access.** reads, writes, network, spawns, destructive. Shown in help, the
  manual and MCP tool annotations (`readOnlyHint`, `destructiveHint`). It adds
  no flags (section 3.9). Taken from: v1.
- **Hidden.** `hidden: true` keeps the op callable on every path but leaves it
  out of help listings, the manual and the MCP tool list. Taken from: v1's
  declared field, now acted on (gait).
- **Group default.** `default: true` makes an op run when its group is called
  with no verb: `archegraph build --blueprint x` runs `build run`, and
  `archegraph build status` still works. At most one per group. A word after
  the group that is not a verb is a usage error, never the default op. Taken
  from: archegraph `build` and `test` (archegraph review: safe).
- **Errors.** `errors: [...]` lists the coded error types the op may return.
  Their codes are the op's stable contract (section 6.3).
- **Span fields.** `fields: f` names a function `fn(&Params) -> Vec<(&'static
  str, String)>` that pulls span fields from the typed params (catuskoti's
  `collection_id`). They go into `CallInfo` (section 4.3).
- **Extra exits** (`exits:`) and **text** (`text: true`), sections 5.4 and 6.2.
- `requires`, `since`, `deprecated` and `aliases` are removed.

### 2.2 Registry and merging

- One `Registry<C>` per binary. `merge` stays, for op-status's `status get`,
  op-listen's entries and a shared crate's ops.
- `push` stays public for code-built entries; `CliHooks` goes because the
  params type carries its own hooks.
- Build-time checks panic naming both ops: duplicate names, a word that is
  both an op and a group (unless it is the group's default), a missing group
  summary, two defaults in one group, a flattened flag name clashing (3.6), an
  extra command clashing with an op word (2.4). A second stdin reader is not a
  build error (see 3.5): it fails at run time.

### 2.3 Root fallback

A registry may name one fallback that gets the raw arguments when no command
matches. It is not an op: no MCP tool, no typed path. Its shape is declared:

- `Fallback::FlagsOnly(f)`: runs when the arguments are empty or the first
  argument starts with `-`. A leading flag goes to the fallback, so `empeiria
  --config x` and `structmuxd --listen ...` run the daemon. Any first bare word
  that is not a command is a usage error with exit 2, so `structmuxd idd`
  never starts a second daemon. Taken from: structmux and empeiria reviews.
- `Fallback::FreeText(f)`: for gait's `gait "task"`. The runner skips leading
  registry globals and their values, then reads the next token:
  - `--`: everything after it goes to the fallback, unchecked. This is the
    escape hatch (`gait -- log` runs a turn with the text "log").
  - A token starting with `-` that is not a registry global: straight to the
    fallback, with no verb check. The turn's own value flags (`--codebase X`,
    `-s X`, `--inference ls`) are never read as verbs.
  - A command or group word: runs the command.
  - A single token with no whitespace, within edit distance 2 of a command or
    group: a usage error naming the command, only when the registry asks for
    near-miss checks (`near_miss`). A sentence never matches.
  - Anything else: the fallback, with the globals already parsed.
  - An empty rest (nothing but globals) goes to the fallback with no arguments.
  - Once the fallback runs, registry globals are also taken from the rest of
    the arguments, up to `--`, so `gait "task" --gait-dir d` works (globals after
    the verb hold for FreeText as for every command). A declared fallback value
    flag keeps its next token as its value, unscanned.
  The fallback's own flags must not reuse a registry global's name; the build
  checks the names the app declares for it. gait's hidden `__brain`,
  `__run-tool` and `__role` entries are dispatched in `main.rs` before the
  runner and stay out of the registry. Taken from: gait review.

### 2.4 Non-registry subcommands

Some binaries keep a subcommand that is not an op, such as multimatrix
`serve`. The registry takes `extra(command: clap::Command, run: fn)` entries.
They share the registry's globals (with env fallback), get the parsed globals
and their own matches, and return `anyhow::Result<u8>`: `Ok(n)` exits `n`, and
an `Err` is reported like an op's error (the error line, then the exit by class).
Fallbacks return the same type. They are not ops, so they
have no MCP tool and no typed path; parity lists them as CLI-only with the
reason "not an op". Taken from: multimatrix (`serve` with `--nats-url`).

## 3. Params and flags

All of this is the `OpParams` derive. One field gives one flag, one JSON key
on every transport and one schema property.

### 3.1 Long kebab flags only

`agent_id` gives the flag `--agent-id` and the JSON key `agentId` on MCP,
REST, the typed path's JSON form and the schema (gp: camelCase input keys
everywhere, the same as output). No `long`
override, no `short`, no `positional`. The only short flag is `-h`. Taken from:
archegraph's rule; the convention; catuskoti (positional URIs break on values
starting with `-`).

### 3.2 Subject and other objects

gp's decision A: the subject is `--id <REF>`; every other object in the call
is a flag named by its kind (`--collection`, `--blueprint`, `--build`,
`--waits-for`). archegraph preferred kind-named subjects; gp chose `--id`.

- A field of type `Ref` with `#[op(subject = "build")]` is `--id` (key `id`).
  One subject per op. `#[op(subject = "agent", id_only)]` takes ids only and
  skips name resolution. `#[op(subject = "agent", absent_ok)]` is for ops
  that accept an id that may not exist (creates, ensures and idempotent probes):
  the subject is never resolved, the op gets the raw id, and the kit's not_found
  test skips it. The two may be combined.
- `#[op(scope = ["blueprint"])]` on the subject names sibling params the
  resolver needs: a concept URI is unique only within a collection, a build
  only within a blueprint.
  A scope name is a field of the same struct (`"blueprint"`) or a dotted path
  through a flattened group (`"project.root"`, 3.6), resolved at compile time.
  `Scope` keys are the leaf field's camelCase name (`root`, `gcodeStore`) in both
  forms, and two entries with one leaf are a compile error. `String`, `Ref` and
  `PathBuf` (and `Option` of these) are scope values; a path that is not UTF-8
  fails the call with `op_registry.usage.invalid`. `scope` on a `#[op(flatten)]`
  field gives a subject declared inside that group its scope, resolved against
  the whole struct. archegraph's `--root` and `--gcode-store` reach its
  `blueprint`, `build` and `symbol` resolvers this way.
- Before the op runs, on every path, the library calls the context's resolver:

  ```rust
  trait Resolve {
      async fn resolve(&self, kind: &str, r: &Ref, scope: &Scope) -> Result<Resolved>;
  }
  ```

  `Scope` holds the named field values, typed as strings, by camelCase key. The op
  receives `Resolved`, never the raw text.
- **Pass-through.** A kind may be declared server-resolved. The library then
  skips `Resolve` for it (header key `resolve_server = ["kind", ..]`) and hands the raw ref to the op as
  `Resolved::Raw(ref)`; the server resolves id-or-name and returns
  `.not_found` or `.ambiguous` itself. This saves a round trip per op (grorg
  resolves server-side since 7ce4d36). The not_found test (section 9) still
  holds, because it checks the error that comes back, not where it was made.
  Kinds with no names (grorg Actions) are `id_only` on every op.
- An unknown ref is the app's coded `<pillar>.<kind>.not_found` (Logical). A
  ref matching two objects is `.ambiguous`, listing the matching ids. Objects
  with no unique names (multimatrix rooms) accept ids and the app's own forms
  (`room:<id>`, an Action id); a duplicate display name is `.ambiguous`.
- Other object flags are plain `Ref` fields with `#[op(kind = "collection")]`
  and are resolved the same way, so a mistyped `--waits-for` is not_found too.

Taken from: the merged convention; gp's A; catuskoti and archegraph reviews.

### 3.3 Lists, options, switches

- `Vec<T>` is a repeated flag (`--node a --node b`), never a comma list. On
  JSON paths it is an array. A batch of structured items is a `Vec<Json>`
  field, or one `Json` field holding an array (catuskoti batch ops).
- `BTreeMap<String, String>` is a repeated `--name k=v`. With `keys = [...]`
  the keys are a closed set (unknown keys are usage errors); without it the
  keys are open (grorg `--metadata`). The schema is an object with
  `additionalProperties: {type: string}` or the listed properties. Map keys are
  data, so the camelCase rule never touches them.
- `Option<T>` is an optional flag.
- `bool` is a switch (multimatrix `--newest` becomes one). `Option<bool>` is
  tri-state: the derive makes `--x` and `--no-x`, and leaving both out is
  `None` (grorg `--portable`, `--is-default`, 8 sites). On JSON paths it is a
  boolean or absent.
- **Clearing a field.** `#[op(clearable)]` on an `Option<T>` field of an
  update op adds `--clear-<field>` and turns the field into `Patch<T>`:
  `Keep` (neither given), `Set(v)` or `Clear`. `--x` and `--clear-x` are a
  usage error together. On JSON paths `null` means clear and absent means
  keep. grorg maps it to the update mask (`agent update --clear-host-machine`).

### 3.4 oneOf and requires_if

- `#[op(one_of = "target")]` on two or more `Option` fields: exactly one must
  be given. `one_of_optional`: at most one. clap gets an `ArgGroup`, the schema
  a `oneOf`. This replaces grorg's after-the-fact `ArgGroup` patch.
- `#[op(requires_if = "uri")]` stays: the field is required when the named
  field is present (catuskoti: a URI lookup needs `--collection`). The schema
  gets an `if`/`then` fragment (0.4.0 no longer emits `dependentRequired`).
  Every path checks it before the op runs.
- `conflicts_with` and `required_unless` are back (v1 had them; the first v2
  draft dropped them): `#[op(conflicts_with = "x")]` and
  `#[op(required_unless = "x")]`, each taking a name or a list, with targets
  that are fields or dotted flatten paths. `at_least_one` joins the `one_of`
  family. Relation members may be `Option`, `Vec`, map, switch, tri-state or
  `Patch`; presence is what the CLI sees (an `{"all": false}` switch is absent).
- A param `env` is allowed in 0.4.0 (`#[op(env = "X")]`, plain or optional
  scalars and switches); it is read on the CLI only, and typed, JSON and MCP
  callers pass the value. It is not on the list of removed items.

### 3.5 Input types

| Type | CLI | JSON paths |
|---|---|---|
| `String`, integers, `f64`, `PathBuf`, a `ValueEnum` | one value | value |
| `Ref` | id or name | string |
| `Json` | a JSON string, `@path`, or `@-` for stdin | any JSON |
| `Input` (bytes) | a path, or `-` for stdin | base64 string (`contentEncoding`) |
| `SecretFile` | a path, or `-` for stdin | see 3.8 |

`Input` serves multimatrix `submit-message --signed-file` (a SignedMessage
protobuf). `Json` serves structmux `owner call --method m --body ...`. Only one
param per op may read stdin; a second read fails at run time with
`op_registry.usage.stdin_twice`. Anything else is
a compile error, as in v1.

### 3.6 Shared param groups

`#[op(flatten)] common: ProjectArgs` puts every field of another `OpParams`
struct into the op, as flags on the CLI and as keys on MCP, REST and the typed
path. This is how a per-call setting reaches every path: archegraph's `root`
and `gcodeStore` are per-call MCP params, so they are a flattened group, not
globals. A group is defined once, so its help wording is the same everywhere.
Flattened names that clash with the op's own fields fail at build. A subject's
`scope = [..]` can reach into a group by dotted path (`"project.root"`), so a
shared `--root` can scope ref resolution (see the Ref section). Taken from:
archegraph review.

### 3.7 Globals

One `#[derive(OpGlobals)]` struct per binary, same field rules, including
`#[op(env = "...")]`. Globals are process settings (endpoints, addresses,
identity dirs, `--gait-dir`), clap `global(true)`, accepted before or after
the verb. They build the CLI's context (4.1) and never appear on MCP, REST or
the typed path. The library adds `--pretty`, `--compact` and, where an op has
text, `--text`. Rule: if an MCP or REST caller must set it per call, it is a
flattened group (3.6), not a global. Taken from: grorg, gait, catuskoti,
multimatrix (`--nats-url` on every command, `serve` included).

### 3.8 Secrets

- **`SecretFile`.** A path or `-`. The library reads it at parse time into
  `Secret<Zeroizing<Vec<u8>>>`: zeroized on drop, printing `<secret>`. The path
  stays printable unless the field is also `#[op(secret)]`. On JSON paths it is
  a path string; an op reachable on `mcp` needs `#[op(secret_inline)]` (an
  in-process or MCP caller passes base64 bytes instead) or leaves `mcp` out of
  its paths. `assert_parity` enforces this.
- **`#[op(secret)]`** on any field: no default or env value in help, the
  manual or the schema; schema `writeOnly`; clap errors naming the flag are
  re-rendered without the value; the value is redacted from the call's error
  line and notices.
- **Redaction holds on every surface.** A call's redactor is one shared handle:
  a secret registered mid-call (`CallOptions::redact`) is removed from the CLI
  error line, `CallFailure` (`call`, `call_tool`, `call_typed` and the stream
  calls), `tool_error`, notices and hooks' messages.
- **Contents are redacted too.** Each `SecretFile` read in a call has its
  contents (and each of their lines) redacted from errors and notices. Text an
  app transforms first (archegraph: agent output cut to 200 chars) is not
  caught; apps must keep secret-bearing text out of errors.
- **Key material out.** An op that makes keys writes them itself with
  `output::write_private(path, bytes)`: a new file at mode 0600, no overwrite
  unless the op has `--overwrite`. Its output carries the path and a public
  fingerprint only, so the bytes never reach stdout, a hook or a notice.

Taken from: structmux review; gait's key-prefix bug (gp G). `zeroize` sits
behind the `secrets` feature (default on).

### 3.9 Dry run and confirmation

gp's decision I: ops act by default. `--dry-run` is declared per op as an
ordinary bool param where useful, never added by the library (archegraph
`build --dry-run` already means "plan only"). `--yes` is declared per op, only
for bulk destructive acts. Both are plain params, so they exist on every path.

## 4. Contexts and calls

### 4.1 Lazy contexts

```rust
trait CliContext: Sized + Send + Sync {
    type Globals: OpGlobals;
    fn from_globals(globals: &Self::Globals) -> anyhow::Result<Self>;
}
```

- `from_globals` is cheap and makes no connection. The context holds lazy
  connections (addresses, identity dirs) and connects inside the op that needs
  one. A local op never touches them, so no connection is opened for it.
  Taken from: multimatrix (three listeners, one binary).
- An op may connect with its own params: the context exposes
  `connect_with(options)` and the op passes, for example, grorg `agent
  register`'s `--dir`, `--root` and `--owner-principal`. The default lazy
  connection is never opened for that op. grorg's async `open_with` refactor
  is part of its migration. gait resolves its dir here too: `--gait-dir`, then
  `$GAIT_DIR`, then `.gaitref`.
- `CliContext` is the CLI's way in only. MCP, REST and in-process callers
  build the context their own way (catuskoti's service: GGUF embedder, error
  sinks) and pass `&C`. Taken from: catuskoti review.
- Signing and identity live inside the context, never in the library.
- `testing::assert_local_ops_offline` runs each local op with a context whose
  connectors fail the test if used. The app supplies that context.

### 4.2 Run hooks

The CLI runner is `cli::run(&registry, argv, &hooks) -> ExitCode`. It never
calls `process::exit`, so the app's telemetry guard and spans flush on drop.
`hooks: RunHooks` is a trait with no-op defaults:

- `pre_parse(&argv)`: before parsing. archegraph starts telemetry here.
- `on_usage_error(&UsageFailure)`: a clap or library usage error. archegraph
  records `archegraph.cli.usage` here.
- `post_run(&Outcome)`: after the op, before the runner writes the result or
  the error line. The app flushes telemetry here, so no log text lands after
  the error (section 5.2).

The pieces stay public (`command`, `parse`, `dispatch`, `render`,
`write_error`, `exit_code`) for tests and odd hosts. An app keeps its own
error tagging (catuskoti `tag_error`) by doing it inside the op or the
CallHook. Taken from: archegraph, multimatrix and catuskoti reviews.

### 4.3 CallInfo and the CallHook

Every call on every path (typed, CLI, MCP, REST, local ops, stream open) goes
through one hook:

```rust
trait CallHook: Send + Sync {
    fn around<'a>(&'a self, info: &CallInfo, call: BoxFuture<'a, ()>) -> BoxFuture<'a, ()>;
    fn on_error(&self, info: &CallInfo, err: &anyhow::Error);
}
```

The library runs the op inside `call` and keeps the typed result itself, so
the hook is the same for typed and JSON calls. `CallInfo` holds the op name,
access, audience, path (typed, cli, mcp, or a transport's name such as
`rest`), the opaque caller, `fields` (from the op's `fields:` function on the
typed params) and `trace_parent`: the incoming W3C `traceparent`, `tracestate`
and `baggage` as plain strings. The CLI reads them from the `TRACEPARENT`,
`TRACESTATE` and `BAGGAGE` env vars, the MCP adapter from request `_meta`, a
REST mount from its headers. The library does not interpret them (archegraph
e2e 34, grorg's command span).

The app wraps `call` in its own span (`archegraph.op` with `op` and an empty
`build_id`; catuskoti's with `collection_id`) and reports a failure once in
`on_error`. Ops record more fields on the current span (archegraph
`record_build_id`). No `tracing` dependency. This replaces archegraph's
`dispatch_cli` span and `AlreadyReported`, and catuskoti's `op_span` and
ErrorSink plumbing.

### 4.4 The typed path

`Registry::call_typed::<ops::ConceptGet>(&ctx, params, opts) ->
Result<Concept, CallFailure>` runs the op in-process with typed params and a
typed output. It resolves refs, checks oneOf and requires_if, and runs the
CallHook like every other path. Errors keep their concrete coded type, so a
caller can downcast them with `failure.error()`; the failure's message is
redacted, and `.map_err(CallFailure::into_coded)?` keeps the code in an
`anyhow::Error`. episutra's bridge uses it in place of calling
`Operation::run`. JSON paths (`call`, `call_tool`, a REST mount) go through
the same entry after decoding. Taken from: catuskoti review; gp's parity rule.

(PATH-1) An op declares any subset of the surfaces `cli`, `typed`, `mcp` and
`json`, with no reason: `paths: [cli]` is the command line only,
`paths: [cli, typed]` the command line and in-process, `paths: [mcp, json]`
the two JSON paths. The default is every surface. A call on a surface the op
does not declare returns the coded `op_registry.call.wrong_path`, on
`call_typed` and `call_stream_typed` too. op-registry has no auth concept: the
subset is generic, and what an op may do on a surface is the app's policy.
Parity checks the declaration exactly (section 9). In 0.3.x `paths: [cli]` meant
the CLI plus typed; write `[cli, typed]` for that.

### 4.5 Notices and progress

`CallOptions` carries a `Notices` handle: `notice(text)` and
`progress(done, total, label)`. The CLI writes them to stderr (section 5.2).
The MCP adapter sends progress and log notifications. The typed path gives
them to a sink the caller supplies, or drops them. Taken from: catuskoti
(`batch_ingest` progress, the MockEmbedder warning).

### 4.6 Signals

The library installs no signal handler. Ctrl-C reaches the process as usual;
an op that needs cleanup (archegraph `build`) handles it itself. The runner
still returns an `ExitCode` when the op returns. Signal exits (130, 143) belong
to the binary, not to the runner.

## 5. Output

### 5.1 stdout

- One JSON value per call on stdout (a stream prints NDJSON, 5.5).
- gp D: pretty when stdout or stderr is a terminal, compact when both are
  piped. `--pretty` and `--compact` force it.
- camelCase keys in and out (gp B and the input-key decision). The library
  does not rewrite output keys; `assert_casing` checks every input and output
  schema (section 9). The library's own types follow it too: op-status's
  snapshot (`takenAtUnixMs`), op-listen's frames and grorg's watch frames go
  camelCase in the same op-registry release.

### 5.2 stderr: notices, logs and the error

stderr carries library notices and progress, the app's own log text
(telemetry's fmt layer) and the error. They do not collide because:
- When stderr is piped, every library write is one compact JSON line with a
  distinct top-level key: `{"notice": ...}`, `{"progress": {"done", "total"}}`
  and `{"error": {"code", "class", "message"}}`. App log text is not JSON.
- The error line is always the last line on stderr: the runner writes it after
  `post_run`, where the app flushes its log. Scripts parse the last line.
- When stderr is a terminal, notices print as text and the error as indented
  JSON, still last.
Apps should keep log text off stderr in CLI mode, but the rules hold anyway.
Taken from: gp D; catuskoti and archegraph reviews.

### 5.3 Lists and paging

gp: with no paging flags, a list follows all pages and prints `{"items": [...]}`
(every array output is wrapped, OUT-1; `call`, `call_tool` and the manual show
the same object); with `--limit` or `--cursor` it prints `{"items": [...], "nextCursor": "..."}`.
One op cannot have two output schemas chosen by a flag (archegraph review).

Resolution: a `list:` entry is written once, as one page function:

```rust
async fn concept_page(ctx: &Context, p: ListConcepts, page: PageRequest)
    -> Result<Page<ConceptSummary>>;
```

The registry generates two ops from it, each with one schema:
- `concept_list`: params without paging, output `{"items": ConceptSummary[]}`. It calls
  the page function until `nextCursor` is empty and returns all items. It
  stops at a safety cap (default 10,000 items, set per op with `cap:`) and
  fails with `<pillar>.<object>.too_many` (Logical), telling the caller to page
  with `--limit`/`--cursor`. It never truncates silently (gp). The code is
  built from the registry's declared `pillar` (data, set by the app) and the
  op's last noun word (`grorg.agent.too_many`), and the manual lists it.
- `concept_list_page`: the same params plus `limit` (required) and `cursor`
  (optional), output `{items, nextCursor}`. `items` is always written, empty
  or not.

On the CLI they share one command, `concept list`. With no `--limit` and no
`--cursor` it runs `concept_list`; with either it runs `concept_list_page`
(a missing `--limit` takes the op's declared default page size, and only
with `--cursor`; with neither flag the command follows all pages). MCP, REST and
the typed path see two ops with honest schemas. Parity knows the pair. Items
should be the type `get` returns; `assert_help_complete` warns when a sibling
`get` returns a different type.

archegraph `graph-search` (50 by default, `--all` for no limit) becomes this
pair: no flags follows all, `--limit 50` pages. Its `--all` goes. grorg's
`--page-size`/`--page-token` become `--limit`/`--cursor`.

**Which ops are `list:`.** An op is `list:` when its server call is paged
(takes a page size and returns a next token). An op that returns a bounded
set in one call (grorg `subtree`, `dependencies`, `children`, `get-many`) is a
plain `op:` returning an array, even when its verb reads like a list. The
help test warns when a verb named `list` is not `list:`, and an op the app
marks `unpaged` silences it. When items differ from `get` (grorg's per-item
`assigneeIds` would need N+1 calls), the warning stands until the server
changes; it does not fail the build.

### 5.4 Text where an op opts in

An op with `text: true` gets `--text`, which prints `RenderText::render_text`
instead of JSON. A `list:` entry may say `text: true` too (OUT-3): `RenderText`
is implemented for `Vec<T>`, one item per line, and the page half adds
`next cursor: <c>` (this reverses ruling R12, which rejected text on lists). JSON stays the default and MCP always returns JSON. archegraph
note: this is not a rename of `Display`. Seven ops print hand-built camelCase
JSON today (and MCP serializes the struct, a second shape); `review` and
`structure` print only textproto or text. Each needs a typed, serde-camelCase
output first; the text form then becomes `RenderText`.

### 5.5 Streams

- NDJSON on stdout: one compact value per line, flushed per line.
- `cursor: true` on a stream op adds `--cursor` to resume. Without it there is
  no cursor (multimatrix presence watch: a snapshot, then changes).
- An error mid-stream ends the stream; the error line is last on stderr and
  the exit follows its class.
- On MCP, stream ops are not tools today. Parity lists them as not on MCP
  with the built-in reason "stream"; that is structural, not a removed path.
  REST mounts and the typed path (`call_stream_typed`) reach them.

### 5.6 Output schema

Each op's output schema is in its descriptor, the MCP tool spec
(`outputSchema`) and `manual --json`. It sits behind the default feature
`schema` (schemars 1).

## 6. Errors and exit codes

### 6.1 The exit table

gp C: 0 ok, 2 usage, 65 logical, 69 infrastructure, 70 unknown, 75 transient.
Usage covers clap parse errors and `op_registry.usage.*`, which keep class
Logical on JSON paths but exit 2 on the CLI. An uncoded error exits 70.

### 6.2 Op-declared exits

`exits: { 4 => "records drifted" }`, with `OkExit::exit_code` on the output
returning 0 or a declared code. Declared codes are in 3..=63. Any other value
is a bug and exits 70. On JSON paths there is no exit code, so the output
carries the state as a field. Taken from: archegraph review (3), drift (4).

### 6.3 Error codes are contract

- Each op lists its error types (`errors: [...]`). The manual lists every code
  each op can return, with its class and exit. Scripts switch on `code`, not
  on the exit number (structmux `worker-enrol.sh` branches on exits 6 and 3
  today and migrates to codes).
- The codes come from coded-error 0.3.0 (de21d22):
  `CodeCatalog::codes() -> Vec<CodeEntry { code, class: Option<ErrorClass> }>`.
  Each type in `errors: [...]` must implement `CodeCatalog`; an error enum
  that wraps others forwards their codes with `#[coded(transparent, catalog)]`.
  A `class` of `None` (the class is decided at run time) prints as "varies"
  in the manual. op-registry pins coded-error at 0.3.0 or later.
- In tests, `strict_codes` (opt-in; the kit's `assert_error_codes` uses the same
  check) fails when an op returns a code not in its own, common or cap codes;
  `op_registry.*` codes are always allowed.
  Renaming a code is a breaking change and shows in the manual snapshot.

### 6.4 Missing objects

gp: `get` of a missing object is `<pillar>.<object>.not_found`, never null.
`delete` or `remove` of a missing object is also not_found; callers that want
idempotence treat not_found as done. catuskoti's `get` (null today) and
`remove` (idempotent today) change; `catuskoti-sync.sh` migrates in the same
ship. The resolver makes this hold for every op with a subject.

### 6.5 Error output

- CLI: the error JSON line on stderr (5.2), with the full context chain in
  `message`, after secret redaction.
- MCP: `tool_error` as v1: code, class, fingerprint, and a redacted message
  for non-logical classes.
- Library codes: `op_registry.usage.{missing,invalid,unknown_key,repeated_key,
  one_of,requires_if,stdin_twice}`, `op_registry.cli.unknown_command`,
  `op_registry.call.{unknown_operation,wrong_shape,outside_audience,wrong_path}`.

## 7. Help and manual

- **Help from doc comments.** An entry's first doc paragraph is the summary,
  the rest long help. Field docs are flag help and the schema `description`.
  Group summaries come from `groups`. One wording per flag name.
- **No machine paths or internal prose.** No env values or computed defaults
  in help ("default: $GAIT_DIR, else ~/.gait" as text). The help test fails on
  the user's home, `/Users/`, `/home/` and the app's banned words ("round-7",
  "Task 17", "daemon" per gp E).
- **Generated manual.** `manual::render(&registry, &manual)` writes Markdown:
  optional front matter (a SKILL.md's `name` and `description`); the exit
  table; sections with headings holding guides (title, body, command paths; a
  missing path fails); then per command its `--help`, access, declared exits,
  and error codes with class and exit. The structure lives in the consumer
  (archegraph's `manual.yaml` becomes sections; YAML parsing stays there). The
  optional `manual` op takes `--out <path>` (archegraph writes
  `.claude/skills/archegraph-manual/SKILL.md`) and `--json` (all descriptors,
  schemas, exits and codes).
- **Snapshot check.** A test compares the checked-in manual with a fresh
  render; `UPDATE_SNAPSHOTS=1` rewrites it. This replaces archegraph's hash.

## 8. MCP and other transports

- **Tools from the same ops.** One tool per visible, non-stream op in the
  served audience. Name = op name. Input keys are the params in camelCase
  (`--gcode-store` is `gcodeStore`), including flattened groups. Output = the same JSON the CLI prints, as structured
  content. Globals are not tool params.
- **rmcp.** The core stays plain data (`ToolSpec`, `call_tool`, `tool_error`).
  The `rmcp` feature adds an adapter that serves a registry for one audience,
  passes `_meta` trace context into `CallInfo`, and maps notices to MCP
  notifications. It replaces archegraph's and catuskoti's
  `register_ops_mcp_block!`.
- **REST and other mounts.** catuskoti's REST service is not a thin adapter:
  it builds its own context. The library gives it `Registry::call(name, json,
  &ctx, opts)` and the descriptors; the app mounts routes (`POST /v1/<name>`)
  and maps errors. A `rest` feature with an axum router can come later; the
  canonical-list test covers the routes now.
- **Runtime ops (later, optional).** The `dynamic` feature adds
  `push_dynamic(descriptor, input_schema, call)` for ops known only at run
  time, called with JSON. MCP only; for structmux daemon's catalog from proto
  descriptors. Not in the first cut.

## 9. Tests the library gives every consumer

`op_registry::testing`, behind the `testing` feature (dev-dependency only).

| Test | Checks | Taken from |
|---|---|---|
| `assert_parity` | every op is on exactly the surfaces its `paths` declares (reachable when declared, absent otherwise; a stream is absent from `mcp` and mounts, the page half of a list from the CLI); an output root that is not an object, and an MCP-reachable plain `SecretFile`, are problems; MCP, CLI, typed and declared mounts agree; extras and streams have reasons; list pairs match | v1, archegraph, gp's parity rule |
| `assert_help_complete` | every op, group and flag has help; one wording per flag; no banned paths or words | grorg, archegraph, gait |
| `assert_manual_snapshot` | the checked-in manual equals a fresh render | archegraph |
| `assert_exit_table` | the table, declared exits and codes are in the manual; exits in 3..=63 | archegraph |
| `assert_error_codes(&ctx)` | in the app's fixture runs, every returned code is in the op's list | structmux |
| `assert_secrets_not_echoed` | a sentinel secret value, and a sentinel key file's contents, never show in help, the manual, the schema, a clap error, an error line, a notice, or stdout | structmux, gait |
| `assert_canonical(&registry, CANONICAL)` | op names equal a checked-in list | catuskoti |
| `assert_unknown_ref_not_found(&ctx)` | each op with a subject except an `absent_ok` one (typed-only ops are skipped), given a random ref, fails Logical `*.not_found` | gait bug, gp |
| `assert_local_ops_offline(&ctx)` | local ops never touch a connection (typed-only ops are skipped and named) | multimatrix |
| `assert_casing(Camel)` | input and output schema keys are camelCase (map keys excepted) | gp B, input-key decision |

## 10. Out of scope

These keep their own entry point, through `extra` (2.4) or the fallback (2.3)
when they share a binary with ops:
- Daemons: multimatrix `serve`, structmuxd's daemon run, empeiria,
  archegraph-watch, gait-acp.
- GUIs (structmux-gpui); test drivers (structmux-driver, driver-headless,
  frontend-headless, multimatrix `room-put`).
- gait's turn (`gait "task"`, `-i`, `-e`), `workflow run|continue|log` and
  hidden entries (`__brain`, `__run-tool`, `__role`).

They may use `op_registry::output` (`print_json`, `write_error`, `exit_code`)
so their one-shot output and exits match. structmux-migrate-keyring is
deleted (gp H).

## 11. Cargo features

| Feature | Adds | Default |
|---|---|---|
| (core) | registry, derive, runner, output, exits, help | yes |
| `schema` | output schemas via schemars 1; the `JsonSchema` bound on outputs | yes |
| `manual` | manual render and the `manual` op | yes |
| `secrets` | `SecretFile`, zeroize | yes |
| `testing` | section 9 | no (dev-dep) |
| `rmcp` | the MCP adapter | no |
| `dynamic` | runtime ops | no, later |

Core deps: clap, serde, serde_json, anyhow, thiserror 2, futures-util,
coded-error, syn and quote. Input schemas are built by the derive by hand, so
the core needs no schemars at all; with `schema` off, outputs need only
`Serialize` and descriptors carry no output schema.

gait (decided in its review): the workspace stays on schemars 0.8 in
gait-types, gait-core and gait-workflow. The CLI output structs live in
gait-bin and derive schemars 1 under gait-bin's own `schema` feature, which
turns on op-registry's. `gait` builds with it; `gait-min` builds op-registry
with `default-features = false` and `features = ["manual"]`. Both schemars
versions coexist in the full build. So `manual --json` from gait-min has no
output schemas, on purpose. At migration gait-lead checks `cargo tree -i
schemars` and `-i zeroize` for gait-min; if zeroize is new there, gait-min
leaves `secrets` off too (gait-min reads no key files).

## 12. Migration order and size

Before coding: every consumer lead agrees to this spec in writing (gp's
library rule: shared logic goes into the my-libs crate, reviewed, and gp is
told). coded-error 0.3.0 has landed with `CodeCatalog`. Then op-registry
v2 lands on its own `main` with its tests (size L). Each consumer migrates
once, upstream first, and bumps its pin. Flag and verb renames from the audits
follow the CLI schema; this migration changes the plumbing, plus what the
library forces (no positionals, `--id`, repeated lists, derived names,
not_found).

1. **grorg (reference), M-L.** About 111 verbs. Drop `LOCAL_OPERATIONS`, the
   hand globals, the `ArgGroup` patch, the stream special path and the exit
   glue. Help appears from doc comments; grorg writes the missing ones.
   concord-telemetry stays in grorg-cli's hooks. Paged server calls become
   `list:` pairs (`--page-size`/`--page-token` to `--limit`/`--cursor`);
   bounded ones stay `op:`. Agent refs use `resolve: server`; Actions are
   `id_only`. Tri-state bools and `--clear-<field>` replace today's unset
   fields; `--metadata` is an open map. The async `open_with` refactor is in
   scope. Watch frames go camelCase.
2. **archegraph, M-L.** About 40 ops. The `Operation` trait and both macros
   go. Params become `OpParams` (7 positionals go); `root` and `gcode_store`
   become a flattened group. Typed camelCase outputs, then `RenderText`.
   `OperationExitCode` becomes `OkExit`. `manual.yaml` becomes manual
   sections. The `SURFACES` table becomes `assert_parity`. The `archegraph.op`
   span moves into the CallHook; telemetry start, usage recording and flush
   into the run hooks. `build` and `test` use group defaults; `graph-search`
   becomes a list pair. MCP names follow the derived rule (no external
   callers).
3. **catuskoti, M.** 18 ops. Macros go; `CANONICAL_OP_NAMES` feeds
   `assert_canonical`. Ops become paths (`concept get`). `--db` and
   `--extractor` become globals for the CLI context; the service keeps its own
   context. `--id` with `--collection` scope; `requires_if` for URIs. `get`
   becomes not_found, `remove` not_found on missing. episutra's bridge moves to
   `call_typed`; the REST routes, `catuskoti-sync.sh` and episutra migrate in
   the same ship.
4. **multimatrix, M.** One registry with owner, agent and local ops, a lazy
   context, globals with env fallback (`--nats-url`), `serve` as an extra
   command. Drop the trailing-args hack and the `room list`/`dm list` special
   case. Merge agent-room into room (gp F). `submit-message` body becomes
   `--message`, `--signed-file` an `Input`; worker system prompts,
   `lead-report.sh`, `watch-actions.sh` and `room-text.py` migrate.
5. **gait, M.** About 25 management verbs. Positional session ids become
   `--id` with the session resolver (fixes the unknown-id bug). FreeText
   fallback for the turn, with `--` as the escape; hidden registry ops;
   `__brain`/`__run-tool`/`__role` stay in `main.rs` before the runner.
   `init`, `link` and `manual install` are local ops. Globals `--gait-dir` and
   `--agent-dir`. Output structs move to gait-bin under its `schema` feature.
6. **structmux, S-M per binary.** No clap today, so clap is new there. First
   `structmux-owner` (every verb; `call` with `Json`; keys written by the op at
   0600; exits 3 to 11 replaced by the class table and codes;
   `worker-enrol.sh` switches on codes). Then structmuxd's one-shots (`id`,
   `setup-code`, `agent-hash`, `catalog`, `reset`) with a FlagsOnly fallback
   for the daemon run. The dynamic MCP catalog is later.

## 13. Risks for review

1. **The runner owns parse and output.** The run hooks, CallHook and public
   pieces must cover every host, or hosts bypass it and drift. grorg's and
   gait's needs are not reviewed yet.
2. **Secret redaction is exact-match**; transformed text escapes it.
3. **List pairs** double the MCP tools for list ops, and following all pages
   has no cap.
4. **Derived names** rename archegraph and catuskoti tools and routes; each
   rename ships with its callers.
5. **JSON errors on stderr** break every concordium matcher on `code:
   message` (scripts, bats tests, lead tools). Each consumer's ship migrates
   the matchers for its own codes, so no ship leaves a stale matcher.

## 14. Needs gp

Nothing open. The param-key casing and the follow-all cap are decided (see
below) and applied in the body.

## gp's schema decisions (2026-11-20, binding; reflected above)

- A: subject `--id <REF>`; other objects are flags named by kind
  (`--waits-for`, `--collection`, `--blueprint`, `--build`). No positionals.
- B: JSON casing is camelCase.
- C: the class exit table (0, 2, 65, 69, 70, 75) as a superset; ops may
  declare extra exits.
- D: errors are JSON on stderr. Pretty when stdout or stderr is a terminal,
  compact when piped; `--pretty`/`--compact` force it.
- E (revised 2026-11-20): names come from the shared glossary
  (docs/glossary.md), a reference list with meanings, nothing banned. A grorg or
  multimatrix "machine" record is a place (`--place`), a structmuxd is a daemon
  (`--daemon`), `--computer` is only a real computer, an address is `--addr`
  (dial) or `--listen` (bind), never `--host`. F: multimatrix merges agent-room into
  room. G: gait auth list never prints key material. H: structmux-migrate-keyring
  is deleted.
- I: act by default; `--dry-run` where useful; `--yes` only for bulk
  destructive acts.
- Lists: no paging flags follows all pages and prints a bare array; `--limit`
  or `--cursor` prints `{"items": [...], "nextCursor": "..."}`.
- Every path (typed `call_typed`, CLI, MCP, any mounted transport such as
  REST) is preserved and at parity. Some ops may be in-process only; any CLI op
  must be structurally able to go to every other path.
- Missing objects: `get` and `delete`/`remove` of a missing object are
  `<pillar>.<object>.not_found`, never null or success; callers wanting
  idempotence treat not_found as done.
- Param keys (gp, 2026-11-20): MCP, REST and every JSON input use camelCase keys, the same as output. Only CLI flags are kebab-case (`--agent-id`, key `agentId`).
- Follow-all-pages cap (gp, 2026-11-20): an unpaged list stops at a safety cap (default 10,000 items, set per op) and fails with `<pillar>.<object>.too_many` telling the caller to page with `--limit`/`--cursor`. It never truncates silently.
- Help under a fallback (orchestrator, from empeiria's final pass): `-h`, `--help` and `--version` are reserved runner flags. As the first token they always show the registry's help or version and never reach a FlagsOnly or FreeText fallback. A fallback's own help is shown by `<bin> help <fallback-name>` and listed in the root help.
