# op-registry 0.4.0: design

Date: 2026-11-20. Status: design for the implementation plan. Nothing here is built yet.

Library: `/Users/aicoder/Documents/my-libs/op-registry`, main f61a04e (code = 0.3.2, d58c8b8). Workspace crates:
`op-registry`, `op-registry-derive`, `op-listen`, `op-status`. All four move to 0.4.0.

Inputs: `gaps/merged.md` (rows 1-48, gp section 4), `gaps/04-addendum.md` (rows 49-55, row edits),
`gaps/04-second-pass.md` (independent review), the five leads' `*-04-check.md` and `*-04-review.md`, the v2 spec
(`docs/superpowers/specs/2026-11-20-op-registry-v2-design.md`) and the code. Every claim below about today's
behaviour was checked against the code at f61a04e; file references use `reg/` = `op-registry/src`, `der/` =
`op-registry-derive/src`, `listen/` = `op-listen/src`, `status/` = `op-status/src`.

Where gp's decisions (merged section 4) conflict with any row, edit or finding, gp wins, and a later gp entry wins
over an earlier one. The appendix maps every row and finding to the change that carries it.

---

## 1. Intent, goals, non-goals

### Intent
0.4.0 is the one library release that lets all five consumers (grorg, gait, multimatrix, structmux, archegraph) and
concordium-app port onto op-registry v2 without hand-written workarounds, and that turns gp's 2026-11-20 decisions
into library behaviour. It is breaking; consumers migrate once.

### The owner's rule: nothing is lost
gp: "we don't want to lose features." Every feature a consumer has today must have a home in 0.4.0: either a
library feature, or a stated consumer-side way to do it with the library. Section 5 lists each feature a review
raised and where it lives. A feature that would be lost is a bug in this design.

### Goals
1. Every consumer command the leads mapped (their "hardest commands" checks and full-surface reviews) writes as a
   plain declaration, with no hand-raised relation errors, no hand-registered streams, no hand-written list
   envelopes and no off-registry exit glue.
2. gp's decisions hold on every path: lists are `{"items": [...]}` everywhere; outputs have an object root; paths
   are any declared subset; the not_found test covers every subject except a declared new one; per-param `env`;
   the CLI prompts for a missing secret; ops may ask for a secret mid-op through `CallOptions::prompt_secret`.
3. Secrets are redacted at every output site, including ones registered while the op runs.
4. The test kit checks what it claims, on every op it can reach, and never hangs.
5. One release: every breaking change in one version, with `#[non_exhaustive]` and constructors where a type is
   likely to grow again, so 0.4.x adds without breaking.

### Non-goals
- No auth, identity or signing in the library (spec section 0). `Caller` stays an opaque label.
- No policy of which ops leave MCP. gp: "the policy for what goes off MCP is a convention, not a library rule."
- No `dynamic` (runtime ops) feature, no REST router. Unchanged from the spec.
- No change to coded-error.
- No consumer code. Section 6 lists what belongs to each consumer.
- No backward compatibility shims, dual reads or deprecated aliases (gp's rule).
- No resolution of refs in dependency order (archegraph's `structure --build` asks for it as a nicety; the
  consumer keeps `--build` a plain string, section 6).

---

## 2. Owner decisions (gp, 2026-11-20), in substance

Binding. Later entries supersede earlier ones where they say so.

1. 0.4.0: GO. The leads check their hardest commands, and one pilot port runs before the library lands.
2. Lists are `{"items": [...]}` on every path (CLI, typed JSON, `call`, MCP). Either-or outputs get an object root.
   This reverses the bare-array CLI decision.
3. not_found test (final, supersedes "lookups only" and addendum row 21): every op that takes a subject is tested
   by default; only a subject declared as new (create/ensure, for example `#[op(subject, new)]`) is skipped.
   `id_only` does not exempt. No exemption lists.
4. Paths (supersedes 0.3.1's required reason and the "owner/root key only" rule): every op is on every path by
   default. An op may opt into any subset (`paths: [cli]`, `[mcp]`, `[cli, typed]`, ...). No reason string is
   required. Parity checks that each op is on exactly its declared paths. What goes off MCP is a convention.
5. An agent's own key is on CLI and MCP (multimatrix `message submit`); `policy sign` stays `[cli]`. grorg
   `agent register`, `agent key ensure/renew/rotate-root/revoke-device` get `paths: [cli]` (convention, no ask).
6. Secret prompt (supersedes addendum decision (b)): the CLI prompts, with no echo, for a missing secret input
   when stdin is a terminal, then passes the bytes in. `--key-file -` keeps working for scripts. Op bodies never
   prompt.
7. Mid-op secret prompt: `CallOptions::prompt_secret(label) -> Option<Secret>`. Terminal only, no echo, `None`
   off a terminal. It covers gait's openai-signin paste step. The redactor also registers the trimmed form.
8. Env on params (supersedes the addendum's "use a global"): per-command `env` on params is a library feature
   (grorg `--owner-principal` / `GRORG_OWNER_PRINCIPAL` on the owner ops only).
9. Flags only: no positionals for non-subject params (`message submit --text`, `policy show --file`, grorg note
   `--text`).
10. archegraph optional scope: no library change; it already works.
11. grorg agent and session codes are renamed to `grorg.agent.not_found` / `grorg.session.not_found` in the 0.4.0
    port; callers ship with it. grorg change-log history entries stay as written. The grorg hub DB rename SQL needs
    gp's OK again after its fix (dump first, server stopped).
12. multimatrix `room messages`: one shape for owner and agent, a page object; `--all` follows to the end (one
    message per line).
13. multimatrix has ONE signed listener: no unsigned owner listener; gp signs with his device key. Owner-audience
    ops are allowed for the owner principal or a policy-grant holder (grorg's service key for dispatch/start; lead
    and orchestrator keys for read/status). Flags are `--addr` (dial) and `--listen` (bind). The multimatrix lead
    designs it in the port plan, which is reviewed before build.
14. archegraph hook payloads stay snake_case (they are not ops); the three concordium scripts are unchanged.

---

## 3. Terms used below

- **Surface**: one way to reach an op. Four exist: `cli` (the runner), `typed` (`call_typed`,
  `call_stream_typed`), `mcp` (`mcp::call_tool`, the rmcp adapter) and `json` (`Registry::call`, `call_stream`
  and any transport an app mounts, such as REST). Today's `CallPath` maps onto them:
  `Cli -> cli`, `Typed -> typed`, `Mcp -> mcp`, `Transport(_) -> json`.
- **Presence**: whether a field was given. Option or tri-state: `Some`; switch: `true`; `Vec`/map: non-empty;
  `Patch`: not `Keep`. Plain fields have no presence (they are always given). Today: `der/params/attrs.rs:328-340`.
- **Kit**: `op_registry::testing`.
- Each change has an id (`DEC-1`, `REL-3`, ...). Each lists: rows and findings it carries; the public API; the
  behaviour on each surface; derive-time errors; the tests that prove it.

---

## 4. Changes

### 4.1 Declarations

#### DEC-1. One op in several audiences
Rows: 17. Findings: second pass 12 (keep `audience`, add `audiences`).

API:
```rust
// ops!: a word or a list
"room messages" => { ..., audience: [agent, owner], ... }
"room create"   => { ..., audience: agent, ... }

pub struct OpSpec<P, O> {
    pub audience: Audience,               // the primary audience (unchanged)
    pub more_audiences: Vec<Audience>,    // new; default empty
    ...
}
#[non_exhaustive] pub struct Descriptor {
    pub audience: Audience,               // the primary (first declared), unchanged
    pub audiences: Vec<Audience>,         // new: every audience, primary first, no duplicates
    ...
}
```
`CallInfo.audience` becomes "the audience serving this call": on MCP the server's audience, everywhere else the
primary. Keeping `audience` and adding `audiences` makes the change additive for grorg's `OpSpec { audience, ..
OpSpec::new() }` (grorg `registry/mod.rs:478-502`), op-status (`status/ops.rs:108-133`) and archegraph's
`d.audience` asserts. `manual --json` gains the key `audiences` (a snapshot re-render).

Surfaces: CLI serves every audience (unchanged). typed and json: unchanged; `CallInfo.audience` is the primary.
MCP: `tool_list(registry, a)` lists an op when `a` is in `audiences`; `call_tool` refuses with
`op_registry.call.outside_audience` only when the server's audience is not in `audiences`
(today it compares equality, `reg/mcp/mod.rs:40` and `:75`).

Derive-time errors: an unknown word (existing message); `audience: []` ("audience needs at least one of agent,
owner, dev"); a word twice ("audience `owner` is given twice").

Tests: `tests/mcp.rs`: an op with `[agent, owner]` is in both tool lists and callable from both; `CallInfo.audience`
seen by a recording hook is the server's audience on MCP and the primary on CLI and typed; an agent-only op is
`outside_audience` from an owner server. `tests/manual.rs`: `render_json` has `audiences`. `tests/ops_ui/`: empty
list and duplicate word fail to compile.

#### DEC-2. Common error lists in the `ops!` header
Rows: 15 (with the row-50 edit: printed once). Findings: gait F9 (must survive `merge`).

API:
```rust
ops! {
    Ctx = Context; globals = Globals; pillar = "grorg";
    errors = [ClientError, ServerErrors];      // new header key
    ...
}
impl<C> Registry<C> {
    pub fn add_common_errors(&mut self, codes: Vec<coded_error::CodeEntry>);   // new
    pub fn common_errors(&self) -> &[ErrorCodeEntry];                          // new
}
```
The header list is kept apart from each op's own list. `Descriptor.errors` stays the op's own codes. The codes an op
may return are own ∪ common ∪ (its cap code, for a follow-all half). `merge` takes the union of both registries'
common errors; one code with two different classes panics naming both.

Surfaces: no behaviour on any call surface; it changes what strict codes and the kit accept and what the manual
prints (MAN-1).

Derive-time errors: the header key list in the unknown-key message gains `errors`; `errors` twice in the header.

Tests: `tests/ops_macro.rs`: a header error code is accepted by `strict_codes` for every op; a code declared in the
header and on an entry is listed once. `tests/registry.rs`: `merge` keeps both registries' common errors, groups
and the receiver's hook; a class clash panics.

#### DEC-3. `#[cfg(..)]` on `ops!` entries
Rows: 27. Today `der/ops/parse.rs:178-182` rejects every non-doc attribute.

API: `#[cfg(feature = "local")] "local setup" => { ... }`. The attribute is copied onto everything generated for
the entry: its push block in `registry()`, its param/output type aliases, and its marker type(s) in `mod ops`
(both markers of a list pair).

Surfaces: an entry whose cfg is off does not exist on any surface. A header group that ends up with no op stays
declared but builds no command (the tree is built from entries, `reg/cli/tree.rs`).

Derive-time errors: any attribute other than doc comments and `#[cfg(..)]` keeps the error, with the message
"only doc comments and #[cfg(..)] are allowed on an op entry".

Tests: `tests/ops_macro.rs`: an entry under `#[cfg(any())]` is absent from `registry()` and `ops::` has no
marker for it; under `#[cfg(all())]` it is present. `tests/ops_ui/`: `#[allow(..)]` on an entry fails.

#### DEC-4. Per-op required globals
Rows: 14, plus the addendum edit (an env value satisfies it) and second pass 17 (check placement).

API:
```rust
"init" => { ..., globals: [gait_dir], ... }        // entry key: field names of the globals struct
pub struct OpSpec<P, O> { pub needs_globals: Vec<&'static str>, ... }   // new; rust field names
#[non_exhaustive] pub struct Descriptor { pub needs_globals: Vec<String>, ... }  // new; long flags
```
The entry key `globals:` (a list of idents) is different from the header key `globals = Type;`.

Behaviour:
- CLI: `cli::parse` checks the needs right after it selects the op and before it returns, so `run_with_io` and any
  host that calls `parse` then `dispatch` both get it (second pass 17). A need is met when the global's value
  source is the command line or its env var (`ArgMatches::value_source` is `CommandLine` or `EnvVariable`), so
  `GAIT_DIR=x gait init` passes. Missing is the usage failure `op_registry.usage.missing`, exit 2, message
  "`init` needs --gait-dir (or GAIT_DIR)"; `on_usage_error` sees it. `--help` and `--version` never check needs.
- typed, json, MCP: nothing (globals never reach them).
- `cli::command` panics at build when a name is not a field of `C::Globals`, or names a switch or tri-state
  ("a needed global must take a value").

Help: the op's `--help` ends with "Needs: --gait-dir (or GAIT_DIR)." The manual prints the same line (MAN-5).
`FieldMeta` of globals gains `env` so the message can name the variable (PAR-11).

Derive-time errors: a non-ident in the list; a name twice.

Tests: `tests/runner.rs`: missing need exits 2 with the code and message; the env var satisfies it; the flag before
and after the verb satisfies it; `parse` alone returns the failure (the dispatch path); `--help` works without it;
an unknown name panics at `cli::command`.

#### DEC-5. A list pair on a one-word path
Rows: 1. Today `reg/list.rs:247-251` asserts a noun, `reg/list.rs:183` indexes `path[len-2]`,
`reg/testing/codes.rs:20` skips the cap code for one word.

API: no signature change. The cap code's noun is `path[len-2]`, or `path[0]` when the path has one word; `-`
becomes `_`. `graph-search` gives `archegraph.graph_search.too_many`. `push_list` drops the noun assert; the kit's
`declared_codes` drops its length check; `manual::codes_of` needs no change.

Surfaces: CLI `graph-search` follows all pages; `graph-search --limit 50` selects `graph_search_page`. typed, json,
MCP: two ops `graph_search` and `graph_search_page`.

Derive-time errors: none.

Tests: `tests/lists.rs`: a one-word list registers; past the cap it fails with `<pillar>.graph_search.too_many`;
`assert_error_codes` accepts that code; the manual lists it. grorg's accepted consequence (`harness model list`
shares `grorg.model.too_many` with `model list`) needs no test.

#### DEC-6. MCP: a context per call, and server instructions
Rows: 20.

API (feature `rmcp`):
```rust
impl<C: Resolve + Send + Sync + 'static> RegistryServer<C> {
    pub fn new(registry: Arc<Registry<C>>, ctx: Arc<C>, audience: Audience, info: Implementation) -> Self; // unchanged
    pub fn with_ctx_factory(registry: Arc<Registry<C>>,
                            factory: Arc<dyn Fn() -> Arc<C> + Send + Sync>,
                            audience: Audience, info: Implementation) -> Self;                       // new
    pub fn with_instructions(self, text: impl Into<String>) -> Self;                                   // new
}
```
The factory runs once per `tools/call`; the call uses the context it returns. `get_info` sends the instructions.

Surfaces: MCP only.

Tests: `tests/rmcp.rs`: two concurrent calls see two contexts (a counter in the factory); `get_info` carries the
instructions; `new` behaves as before.

### 4.2 Relations

Today: `one_of` and `one_of_optional` take only `Option` fields (`der/params/attrs.rs:233-238`), `requires_if` only
`Option` fields naming an optional field of the same struct (`attrs.rs:239-244`, `der/params/mod.rs:93-105`).
Relations are param-to-param only: globals are never members (they are a separate struct, `der/globals.rs`).

#### REL-1. Relation members of any shape with presence
Rows: 7. Answers multimatrix's `Option<Input>` question: it has presence.

API: `one_of`, `one_of_optional`, `requires_if`, `conflicts_with`, `required_unless` and `at_least_one` (REL-3,
REL-4) are allowed on Option (any base, including `Ref`, `Input`, `SecretFile`), tri-state, switch, `Vec`, map and
`Patch` fields. Plain fields and flattened groups are rejected. `requires_if` takes a string or a list (the field
is required when any named field is present).

clap: an `ArgGroup` member is the arg id; a tri-state member adds both `x` and `no_x`, a `Patch` member both `x`
and `clear_x`. clap counts only explicitly given args, so a switch's default `false` is not presence.

Surfaces: the derived `validate` uses presence on every surface (it is called from `from_matches`, `from_json`
and `call_typed`). JSON `{"all": false}` is "absent" for a switch member, the same as on the CLI; REL-6 makes the
schema agree.

Derive-time errors: "field `x`: relations need a field that can be absent (Option, Vec, BTreeMap, bool, Patch);
`x` is a plain `T`"; the existing "one_of group needs two members" now counts any presence field.

Tests: `tests/params_relations.rs`: a `one_of` over `id: Option<Ref>` and `all: bool` accepts each alone and
refuses both and neither, on CLI, JSON and typed; `{"all": false, "id": "x"}` is accepted; `requires_if` with a
`Vec` target; `one_of_optional` over a switch and two options (multimatrix `--newest`). `tests/params_ui/`: a
relation on a plain field fails; the old `op_params_one_of_needs_two_options_is_a_compile_error.rs` is replaced
by a two-members test.

#### REL-2. `#[op(required)]` on `Vec<T>` and `Option<T>`
Rows: 5. Today `required()` is plain-only (`attrs.rs:342-344`).

API: `#[op(required)] grant: Vec<Grant>`, `#[op(required)] to: Vec<String>`, `#[op(required)] visible:
Option<bool>`.
- `Vec<T>`: clap `.required(true)`; schema `required` plus `minItems: 1` on the property; JSON absent or `[]` is
  `op_registry.usage.missing`.
- `Option<T>`: clap `.required(true)`; tri-state: a required `ArgGroup` of `--x`/`--no-x`; schema `required`; JSON
  absent or `null` is `.missing`.
- The op still receives `Vec<T>`/`Option<T>`, guaranteed non-empty/`Some`. `FieldMeta.required` is true.

Surfaces: the same check in the derived `validate` on every surface (CLI also gets clap's early error, exit 2).

Derive-time errors: `required` on a plain field ("already required"), a switch, a map or a `Patch`; `required`
with `default`, `required_unless`, or any `one_of`/`at_least_one` group.

Tests: `tests/params_relations.rs`: each shape, each surface; an MCP call omitting a required `Option<bool>` is
`.missing` (today it silently becomes `false`, `der/params/json_gen.rs:59`).

#### REL-3. `conflicts_with` and `required_unless`
Rows: 6 (restored from v1). Edits: targets are struct fields or dotted flatten paths (REL-5); globals never.

API:
```rust
#[op(conflicts_with = "signed_file")]                 // or a list: ["to", "message"]
#[op(required_unless = "dry_run")]                    // or a list: required unless any is present
```
- clap: `conflicts_with_all([..])` (tri-state and Patch targets add their second id);
  `required_unless_present_any([..])`.
- validate: both present is the new `UsageError::Conflicts { field, other }`, code `op_registry.usage.conflicts`
  (Logical, exit 2 on the CLI); `required_unless` with field and every target absent is the new
  `UsageError::MissingUnless { name, unless }`, code `op_registry.usage.missing` (same code as `MissingField`;
  the manual's library-code table de-duplicates by code, ERR-3).
- Schema: REL-6 fragments.
- Manual and help: MAN-5 lists the relations under the op (`OpParams::relations()`).

Surfaces: the same on every surface.

Derive-time errors: unknown target; a target without presence; a field naming itself; `required_unless` on a plain
field or a switch; one field with both `conflicts_with` and `required_unless` naming the same target.

Tests: `tests/params_relations.rs`: multimatrix's `message submit` shape (`--to` required unless `--signed-file`,
conflicts with it) and structmux's `--confirm-fingerprint` required unless `--dry-run`, on every surface; exit 2 on
the CLI. `tests/params_ui/`: each derive error.

#### REL-4. `at_least_one`
Rows: 32. Findings: second pass 15 (own variant).

API: `#[op(at_least_one = "query")]` on two or more presence fields. clap: `ArgGroup::new(g).required(true)
.multiple(true)`. validate: none present is the new `UsageError::AtLeastOne { group }`, code
`op_registry.usage.at_least_one` ("at least one of the {group} fields is required"). Schema: `anyOf` of the
members' fragments.

A field belongs to at most one group of the family `one_of`/`one_of_optional`/`at_least_one` (today's "one field
takes one one_of group" rule extends).

Derive-time errors: fewer than two members; one group name used with two kinds.

Tests: archegraph's `graph-search` (`--name`/`--file`/`--node-kind`): none fails, one or several pass, every
surface.

#### REL-5. Relation targets across a flatten, by dotted path
Rows: 51. Answers structmux Q1 (`required_unless = "common.dry_run"`).

API: `requires_if`, `conflicts_with` and `required_unless` targets may be dotted paths whose first segment is a
`#[op(flatten)]` field of this struct (the scope-name rule, `der/params/mod.rs:132-179`). `one_of`-family groups stay
within one struct.

Mechanics: a new runtime trait (hidden, in `rt`)
```rust
pub trait Presence { fn present(&self) -> bool; }   // bool, Option<T>, Vec<T>, BTreeMap<K, V>, Patch<T>
```
The derived `validate` calls `Presence::present(&self.common.dry_run)`. clap uses the leaf's arg id (its rust
name; flattened args share one command and ids are unique, `reg/registry.rs:739-753`). The schema fragment for a
flattened leaf is built at run time from `<Group as OpParams>::fields()` (the leaf's `FieldKind`).

Derive-time errors: a first segment that is not a flatten field; an empty segment. A leaf without presence fails
to compile with the trait-bound error (E0277, "Presence is not implemented for String"); the docs say so.

Tests: structmux's `EditCommon { owner_key_file, dry_run }` flattened into `AddDevice` with
`required_unless = "common.dry_run"` on `confirm_fingerprint` and on the group's own `owner_key_file`; every
surface; the schema keeps the group's relation after absorb (REL-6).

#### REL-6. Relation schemas that match the library's presence, and a builder that carries them
Findings: second pass 7 and 8.

Today `SchemaBuilder::one_of` emits `{"required": [k]}` (`reg/params/rt.rs:193-205`), which `{"all": false}`,
`{"to": []}` and `{"id": null}` satisfy while `validate` calls them absent; `build` assumes a lone constraint is a
`oneOf` (`rt.rs:263-268`); `absorb` drops `anyOf`/`not` (`rt.rs:219-249`).

Design: one helper `rt::present_schema(key, kind) -> Value` gives the fragment "key is present":

| member | fragment |
|---|---|
| switch | `{"required":[k],"properties":{k:{"const":true}}}` |
| `Vec` | `{"required":[k],"properties":{k:{"minItems":1}}}` |
| map | `{"required":[k],"properties":{k:{"minProperties":1}}}` |
| Option, tri-state | `{"required":[k],"properties":{k:{"not":{"type":"null"}}}}` |
| `Patch` | `{"required":[k]}` (`null` is `Clear`, which is present) |

Relations use fragments: `one_of`: `{"oneOf":[P(a),P(b)]}`; `one_of_optional`: `{"oneOf":[P(a),P(b),{"not":
{"anyOf":[P(a),P(b)]}}]}`; `at_least_one`: `{"anyOf":[..]}`; `requires_if` (f when o): `{"if":P(o),"then":P(f)}`;
`conflicts_with`: `{"not":{"allOf":[P(a),P(b)]}}`; `required_unless`: `{"anyOf":[P(f),P(o1),..]}`.
`dependentRequired` is no longer emitted (no compat; the sampler drops its reader, KIT-4).

The builder stores each constraint as a whole object. `build`: none; one constraint merged into the root as is;
two or more under `allOf`. `absorb` copies a flattened group's `oneOf`, `anyOf`, `not`, `if`/`then` and every
`allOf` member as constraints.

Surfaces: schemas only (tool `inputSchema`, manual, descriptors). Behaviour is `validate`'s.

Tests: a new dev-dependency `jsonschema` (MIT) validates, for each relation fixture, a table of inputs: the schema
accepts an input exactly when `from_json` + `validate` accepts it (including `false`, `[]`, `null` members). A
struct whose only relation is a `conflicts_with` produces a valid schema. A flattened group's `anyOf` survives.

#### REL-7. Struct-level validator, public `usage()`, and the usage hook for op-side usage errors
Rows: 12.

API:
```rust
#[derive(OpParams)]
#[op(validate = check_bulk)]                 // struct-level, new
struct PersistParams { ... }
fn check_bulk(p: &PersistParams) -> Result<(), op_registry::UsageError> { ... }

pub fn usage(error: UsageError) -> anyhow::Error;   // loses #[doc(hidden)], documented (reg/params/mod.rs:221-224)
```
The derived `validate` runs the built-in relations, then each flattened group's `validate`, then the struct's
validator; its `Err` becomes `usage(err)`. Today a struct-level `#[op(..)]` is silently ignored (the derive never
reads `input.attrs`); 0.4.0 parses it and rejects unknown keys.

The runner calls `RunHooks::on_usage_error` for every failure whose exit is 2, wherever it came from (decode,
validate, a needed global, or an op that returns `op_registry.usage.*`). Today it is called only for parse and
decode failures (`reg/cli/run.rs:147-151`, `:374-381`). It is called once per run, before `post_run`.

Surfaces: the validator runs on every surface before refs resolve. The usage hook is CLI only.

Derive-time errors: an unknown struct-level key (existing message, now raised); `validate` without a path value.
Struct-level keys in 0.4.0: `validate`, `allow_hyphen` (PAR-7).

Tests: `tests/params_relations.rs`: gait's "`--all` needs `--yes` unless `--dry-run`" on every surface, exit 2 on
the CLI; `tests/runner.rs`: an op returning `usage(..)` reaches `on_usage_error` exactly once and exits 2.
`tests/params_ui/`: an unknown struct-level key fails.

### 4.3 Paths

#### PATH-1. Any subset of surfaces, no reason
gp decision 4. Replaces 0.3.1's `paths: [cli]` plus required `reason:` (`der/ops/parse.rs:342-364`, `:469-485`;
`reg/descriptor.rs:132-145`; `reg/registry.rs:511-515`).

API:
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface { Cli, Typed, Mcp, Json }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paths(/* private bit set */);
impl Paths {
    pub const ALL: Paths;
    pub fn of(surfaces: &[Surface]) -> Paths;
    pub fn contains(self, s: Surface) -> bool;
    pub fn iter(self) -> impl Iterator<Item = Surface>;
}
impl Default for Paths { /* ALL */ }
// Serialize/Deserialize as a list of words: ["cli", "typed"]

// ops!
"policy sign" => { ..., paths: [cli], ... }
"agent key ensure" => { ..., paths: [cli, typed], ... }
```
Removed: `Paths::All`, `Paths::TypedOnly`, `Paths::CliOnly`, `OpSpec.cli_only_reason`,
`Descriptor.cli_only_reason`, the `reason:` key.

Meaning change, stated in the CHANGELOG: in 0.3.x `paths: [cli]` meant "CLI and typed"; in 0.4.0 it means the CLI
only. Write `[cli, typed]` for the old meaning.

Behaviour:
- One check, `check_path(entry, surface)`, refuses an undeclared surface with `op_registry.call.wrong_path`. It now
  also runs in `call_typed` and `call_stream_typed`, which skip it today (`reg/invoke.rs:335-397`).
- CLI tree: an op is a command when its paths contain `cli` and it is not the page half of a list
  (`reg/cli/tree.rs:20-25`).
- MCP: `tool_list` lists an op when its paths contain `mcp`, it is not hidden, not a stream, and the audience
  matches (DEC-1).
- json: `Registry::call`, `call_stream` and mounts accept an op when its paths contain `json`.
- Both halves of a list pair share the entry's paths.
- A stream declared on `mcp` is still not a tool (spec 5.5); parity reports it absent with the reason "stream".

Derive-time errors: an unknown word ("`paths` takes cli, typed, mcp and json"); an empty list; a word twice; the
`reason:` key: "unknown key `reason`: 0.4.0 needs no reason for `paths`; remove it".

Tests: replace `tests/cli_only.rs` with `tests/paths.rs`: for each of `[cli]`, `[typed]`, `[mcp]`, `[json]`,
`[cli, typed]`, `[mcp, json]`, the op works on its declared surfaces and is `wrong_path` on every other, including
typed; the CLI tree, `tool_list` and the manual show it accordingly. Delete
`tests/ops_ui/ops_cli_without_reason_is_a_compile_error.rs` and `ops_reason_without_cli_is_a_compile_error.rs`;
add UI tests for an unknown word, an empty list, a duplicate and `reason:`. Parity: KIT-6.

### 4.4 Params, refs and env

#### PAR-1. Optional subject
Rows: 4, with the addendum edit (seed `id` before sampling) and the stream answer for multimatrix.

API: `#[op(subject = "build", scope = ["blueprint", "project.root"])] id: Option<Ref>`. Flag `--id`, key `id`,
not in `required`. Absent: no ref slot, no resolution, the op gets `None`. Present: resolved as the subject, with
its scope and its `id_only`/`new` rules (PAR-4). It may be a member of any relation (REL-1), so `one_of =
"target"` over `id` and `action` works. No `default` on a ref (the op maps `None` to "me", "newest" or "list").

Surfaces: the same on every surface. A stream op's params (for example `listen_entry`'s `L::Params`) may hold an
optional subject in a group; `listen/entry.rs:19-33` passes them unchanged.

Derive-time errors: `Vec<Ref>` or `Patch<Ref>` as the subject ("the subject is one `Ref` or `Option<Ref>`"); a
default on a ref (existing).

Tests: `tests/refs.rs`: absent reaches the op as `None` without calling the resolver; present is resolved with its
scope; `one_of` over `id`/`action`; a stream op with an optional-subject group, with `listen_entry`
(`op-listen/tests/registry.rs`). Kit coverage: KIT-3.

#### PAR-2. Scope and `id_only` on non-subject refs
Rows: 8, with the addendum edits (optional scope members already work; add a test and a README line).

API: `#[op(kind = "blueprint", scope = ["project.root"])] blueprint: Option<Ref>`,
`#[op(kind = "action", id_only)] waits_for: Option<Ref>`, also on `Ref` and `Vec<Ref>`. Scope names follow the
subject's rules (fields or dotted flatten paths, unique leaf keys).

Behaviour: non-subject refs still resolve before the subject (`reg/invoke.rs:258-266`), so a non-subject's scope
may not name another `Ref` field. An absent optional scope member is left out of the `Scope`
(`reg/params/rt.rs:390-405`), which the README now states.

Surfaces: the same on every surface.

Derive-time errors: "the scope of non-subject ref `x` names the ref `y`; refs resolve non-subjects first, so only
the subject's scope may name a ref"; the old "id_only and scope belong to the subject" error goes.

Tests: `tests/refs.rs`: a non-subject ref with a scope reaches the resolver with it; `id_only` on a non-subject
skips the resolver; `--id` without `--blueprint` reaches the build resolver with no `blueprint` key (archegraph's
case).

#### PAR-3. `Patch<Ref>`
Rows: 40, with the addendum edit (classify, `ref_info`, a `Patch` arm in `ref_gen`).

API: `#[op(kind = "place", id_only, clearable)] home_place: Patch<Ref>`. `Set(ref)` is resolved like any
non-subject ref; `Keep` and `Clear` have no slot. `--clear-home-place` as today; JSON `null` is `Clear`.

Changes: `der/params/types.rs:146` admits `Ref` in `Patch`; `attrs.rs:359-362` counts `Patch(Ref)` as a ref shape;
`ref_gen.rs:85-88` gets a `Patch` arm (`if let Patch::Set(r) = &mut self.x { slots.push(..) }`).

Derive-time errors: a `Patch<Ref>` subject ("the subject is not clearable").

Tests: `tests/refs.rs`: `Set` resolves, `Keep` and `Clear` do not call the resolver; CLI `--clear-x` and JSON
`null`.

#### PAR-4. New subjects: `#[op(subject = "agent", new)]`
gp decision 3. Replaces row 21 and the addendum's `id_only` skip (second pass 9, grorg review 1).

API: `#[op(subject = "agent", new)] id: Ref` (or `Option<Ref>`) for ops that take an id or name that may not exist
yet (create, ensure, idempotent probes). Effect: the subject is never resolved; the op gets
`Resolved::Id(raw)`; `Resolve::check_id` still runs (PAR-5); the kit's not_found test skips the op (KIT-3).
`FieldMeta.subject` records it (PAR-11).

Surfaces: the same on every surface.

Derive-time errors: `new` on a non-subject; `new` with `id_only` ("`new` already passes the id through").

Tests: `tests/refs.rs`: a `new` subject never calls the resolver; `check_id` runs for it. Kit: KIT-3.

#### PAR-5. `Resolve::check_id`
Rows: 42.

API:
```rust
pub trait Resolve: Send + Sync {
    fn resolve<'a>(...) -> BoxFuture<'a, anyhow::Result<Resolved>>;           // unchanged
    fn check_id(&self, kind: &str, raw: &str) -> Result<(), UsageError> { Ok(()) }   // new, default Ok
}
```
Called for every slot that skips resolution (`id_only` and `new`), on every surface, before the op; an `Err` is a
usage error (exit 2 on the CLI). Server-resolved kinds are not checked (the server answers).

Tests: `tests/refs.rs`: a malformed principal id is `op_registry.usage.invalid`, exit 2 on the CLI, on every
surface; a well-formed one passes.

#### PAR-6. Typed values with `FromStr`, and sample values
Rows: 25. Findings: second pass 10 (samples that cannot decode).

API: `#[op(from_str)] addr: SocketAddr`, also `Option<T>`, `Vec<T>`, and in `OpGlobals`. Bound: `T: FromStr`,
`<T as FromStr>::Err: Display`. `#[op(sample = "127.0.0.1:7000")]` on any string-valued field puts
`"examples": ["127.0.0.1:7000"]` in its schema; the kit's sampler uses `examples[0]` (KIT-4).

Behaviour: clap parses a `String`, then `T::from_str`; JSON reads a string, then `T::from_str`; a failure is
`op_registry.usage.invalid` naming the flag or key with the error's `Display` (a `secret` field says only
"invalid value", as today). Schema `{"type": "string"}`. A `default` on a `from_str` field is a `T` expression.

Surfaces: the same on every surface (globals: CLI only).

Derive-time errors: `from_str` with `value_enum`; `from_str` on `Ref`, `Json`, `Input`, `SecretFile`, `bool`;
`sample` on a non-string field.

Tests: `tests/params_flags.rs`: `SocketAddr`, a custom `Grant` type in `Vec`, a global `PrincipalId`; bad input is
exit 2 with the parser's message; schema has `examples`.

#### PAR-7. Opt-in hyphen values
Rows: 28. Findings: gait F13 (a struct-level switch so none are forgotten).

API: `#[op(allow_hyphen)] value: String` gives clap `allow_hyphen_values(true)`. On the struct,
`#[op(allow_hyphen)]` applies it to every valued field of that struct (not to flattened groups). Not the default: a
typo would swallow the next flag.

Surfaces: CLI only (JSON has no flags).

Derive-time errors: on a switch, tri-state or map field.

Tests: `config set --value -1` and `reply --message "- ok"` parse; without the attribute they fail as today.

#### PAR-8. `value_enum` ignoring case
Rows: 39.

API: `#[op(value_enum, ignore_case)]`: clap `.ignore_case(true)`; JSON `enum_value(name, true)`
(`reg/params/rt.rs:64-66` today passes `false`). The schema keeps the canonical names.

Derive-time errors: `ignore_case` without `value_enum`.

Tests: `--status IN_PROGRESS` and JSON `"In-Progress"` decode; the schema lists `in-progress`.

#### PAR-9. Literal defaults in help, and `hide_default`
Rows: 41, with the addendum edit and archegraph N3/N4.

API: a literal `default` (integer, float, bool or string literal, or a variant path on a `value_enum` field) is
shown in help as `[default: 50]` (clap `default_value(text)`; for an enum, the variant's clap name). A computed
default (a call, a const) is not shown (spec 7). An empty string default is not shown. `#[op(default = 0,
hide_default)]` hides a sentinel default from help; the schema keeps `default`. Decoding is unchanged (the derive
applies defaults itself, `der/params/clap_gen.rs:156-160`).

Derive-time errors: `hide_default` without `default`.

Tests: help shows `[default: 50]` and `[default: syntax]`; `default = ""` and `hide_default` show nothing; the
schema keeps the default.

#### PAR-10. `env` on params
gp decision 8. Findings: second pass 6, grorg review 5.

API: `#[op(env = "GRORG_OWNER_PRINCIPAL")] owner_principal: Option<String>` on plain or optional scalars (string,
path, numbers, enums, `from_str`) and switches. Today `der/params/attrs.rs:158-163` rejects it on params.

Behaviour:
- CLI: clap `.env(X)`, with `.hide_env_values(true)` (a `secret` field: `.hide_env(true)`), the same as globals
  (`der/globals.rs:73-82`). The value is read only when the flag is absent and goes through the same decode and
  validation as the flag, so a malformed env value is the same usage error.
- typed, json, MCP: never read. To keep one call the same on every surface, an op whose params (flattened groups
  included) have an `env` field may not include `json` or `mcp` in its paths: `Registry` panics when it is pushed,
  naming the op, the flag and the variable ("--owner-principal reads GRORG_OWNER_PRINCIPAL, which only the command
  line does; declare `paths: [cli]` or `[cli, typed]`"). grorg's two ops are `paths: [cli]` (gp decision 5).
- Help shows `[env: GRORG_OWNER_PRINCIPAL]` (clap); the manual includes the help. The schema is unchanged.
  `FieldMeta.env` records the variable.

Derive-time errors: `env` on a `Ref`, `Json`, `Input`, `SecretFile`, `Vec`, map, `Patch` or flatten field. The UI
test `op_params_env_on_params_is_a_compile_error.rs` is replaced by one for `env` on a `Vec`.

Tests: `tests/params_flags.rs`: env fills an absent flag; the flag wins over env; a malformed env value is exit 2;
help shows the variable; a registry that puts such an op on `mcp` panics.

#### PAR-11. `FieldMeta` and `FieldKind` grow, once
Rows: 54 (`FieldKind::Path`), the addendum's row-21 edit (subject flags). Findings: second pass "make all
`FieldMeta`/`SecretSentinel`/`Io` changes once with constructors".

API:
```rust
#[non_exhaustive]
pub struct FieldMeta {
    pub rust_name: &'static str, pub flag: String, pub key: String, pub help: String,
    pub kind: FieldKind, pub required: bool, pub secret: bool, pub from_group: Option<&'static str>,
    pub subject: Option<SubjectMeta>,    // new
    pub env: Option<String>,             // new (params and globals)
    pub prompt: Option<String>,          // new: the label of a prompted SecretFile (SEC-4)
}
impl FieldMeta {
    pub fn new(rust_name: &'static str, kind: FieldKind) -> Self;   // flag and key derived from the name
    pub fn with_flag(self, ..) -> Self; pub fn with_key(self, ..) -> Self; pub fn with_help(self, ..) -> Self;
    pub fn required(self, yes: bool) -> Self; pub fn secret(self, yes: bool) -> Self;
    pub fn with_subject(self, s: SubjectMeta) -> Self; pub fn with_env(self, ..) -> Self;
    pub fn with_prompt(self, ..) -> Self;
}
#[non_exhaustive]
pub struct SubjectMeta { pub kind: String, pub id_only: bool, pub new: bool }

#[non_exhaustive]
pub enum FieldKind { Scalar, Switch, TriState, Repeated, Map, Json, Input, Ref, SecretFile,
                     Path,    // new: PathBuf and Option<PathBuf>
                     Patch }  // new: Patch<T> (today reported as Scalar)
```
The derive and `list.rs`'s paging fields use the constructor (a derive expands in the user's crate, so it cannot
use a struct literal of a non-exhaustive type).

Tests: compile-only use in the crate's tests; KIT-3 and KIT-4 read the new fields.

### 4.5 Secrets and prompts

#### SEC-1. One shared redactor per call, reaching every output site
Rows: 10 (the "shared handle"). Findings: second pass 5 (blocker: a leak if built as written).

Today `Redactor` is a plain `Vec<String>` that is copied (`reg/params/mod.rs:76-128`): into the `Invocation`
(the CLI error line, `CallFailure`, `tool_error`), into `CallInfo` (the hook's `on_error`), and into the notice
sink, which is wrapped only when the redactor is non-empty at call start (`reg/notices.rs:61-69`). A secret added
during the call would reach none of them.

Design: `Redactor` becomes a shared handle.
```rust
#[derive(Clone, Default)]               // Clone shares the same list
pub struct Redactor(Arc<RwLock<Vec<String>>>);
impl Redactor {
    pub fn new() -> Self;
    pub fn add(&self, bytes: &[u8]);          // was &mut self
    pub fn extend(&self, other: &Redactor);   // was &mut self
    pub fn redact(&self, text: &str) -> String;
    pub fn is_empty(&self) -> bool;
    pub fn detached(&self) -> Redactor;       // new: an independent copy
}
```
The pipeline makes one redactor per call (from the params' secrets and, on the CLI, the secret globals) and hands
the same handle to every holder. Every output site and how it gets the handle:

| site | today | 0.4.0 |
|---|---|---|
| CLI error line (`finish_err`) | `Invocation.redactor` copy | same handle, redacts at write time |
| `Outcome::Err.message` (`post_run`, telemetry) | computed in `finish_err` | same, after the op, so later additions count |
| stream item errors on the CLI | invocation redactor | same handle |
| notices (CLI `StderrSink`, MCP `ChannelSink`, a typed caller's sink) | wrapped only if non-empty at start | always wrapped when a sink exists |
| `CallInfo::redact` in `on_error` | copy | same handle |
| `CallFailure` (`call`, `call_tool`) and `tool_error` | copy | same handle |
| typed errors, `call_stream` errors | unredacted raw `anyhow` | `CallFailure` with the handle (OUT-7) |
| usage errors before params exist | argv scrub (`reg/cli/parse.rs`) | unchanged |

What was already sent (a notice before `opts.redact`) is not redacted after the fact; the docs say "register a
secret before you print anything that may contain it".

Surfaces: every surface.

Tests: `tests/redaction.rs`: an op with no declared secrets calls `opts.redact(b"s3cr3t-value")`, then sends a
notice and fails with an error, both containing it. On the CLI: the error line, the notice line and
`Outcome::Err.message` (a recording `RunHooks`) lack it. A recording `CallHook`'s `on_error` through
`info.redact(..)` lacks it. `call` (`CallFailure` Display and `redacted_message`), `call_tool` + `tool_error`,
`call_typed` and a stream item error lack it. A redactor cloned before the addition sees it; `detached` does not.

#### SEC-2. `CallOptions::redact`, and secret key-file paths
Rows: 10, with the addendum edit (the kit checks absence, not that `redact` was called) and structmux Q2.

API:
```rust
impl CallOptions {
    pub fn redact(&self, bytes: &[u8]);   // adds the value, its trimmed form and each trimmed line
}
```
`#[op(secret)]` on a `PathBuf` (or `Option<PathBuf>`) is the convention for "a key file the op reads itself with
its own checks" (structmux owner keys, multimatrix's service key file). Parsing is unchanged; the path text stays
redacted as today (`rt.rs:432-436`). The op reads the file and calls `opts.redact(&contents)` before anything can
print it. The kit (KIT-5) passes the sentinel key file to such fields and checks that its contents never appear;
an op that fails before reading (a sentinel key is not a valid key) passes as long as nothing echoes.

List page functions receive no `CallOptions` (`der/ops/expand.rs:172-180`), so a list op cannot register secrets;
the docs say a secret-reading op is not a list.

Surfaces: every surface (unary, local and stream ops receive `CallOptions`).

Tests: in `tests/redaction.rs` (SEC-1); `tests/secrets.rs`: a `secret` `PathBuf` op that reads and registers its
file passes `assert_secrets_not_echoed`; one that echoes the contents without registering fails it.

#### SEC-3. The trimmed form is registered
gp decision 7. Findings: gait review F2.

`Redactor::add` already registers the whole value and each trimmed non-empty line (`reg/params/mod.rs:96-105`), so a
one-line key with a trailing newline is caught in both forms today. 0.4.0 also registers the whole value trimmed
(a multi-line value with surrounding blank space). The rule holds for `SecretFile` contents, `opts.redact`,
`prompt_secret` and prompted fields.

Tests: `tests/redaction.rs`: a `SecretFile` read from stdin as `"sk-abc123\n"` whose op fails with an error
containing `sk-abc123` is redacted; same for a value with leading and trailing blank lines.

#### SEC-4. The CLI prompts for a missing secret
gp decision 6. Findings: second pass 3 (clap rejects a missing required arg before any runner code runs).

API: `#[op(prompt)]` or `#[op(prompt = "API key")]` on a required `SecretFile` field (plain, not `Option`). The
label defaults to the flag name.

Behaviour:
- CLI: the derive makes that clap arg not required, so clap never rejects its absence. In `from_matches`, an absent
  prompted field asks the parse context's prompter (SEC-7) with the label: one line read from the terminal with
  echo off. The bytes become `SecretFile::new(None, bytes)`, registered with the redactor like any `SecretFile`.
  The prompt is written to the terminal, never to stdout. With no prompter (stdin or stderr is not a terminal, or
  the `prompt` feature is off) the field is `op_registry.usage.missing` (exit 2), as a missing required flag is
  today; the message adds "(no terminal to ask on)". A read error is `op_registry.usage.invalid` "cannot read from
  the terminal: ..". `--key-file -` keeps reading stdin (with the SEC-6 hint); a given path is read as today.
- The prompt runs during decode, before the op (`reg/cli/run.rs:364-385`).
- typed, json, MCP: no prompter; absent is `op_registry.usage.missing`. The schema keeps the field `required`.

Help: the flag's help gains "Asked for on a terminal when left out." (generated). The manual shows it.

Derive-time errors: `prompt` on anything but a plain `SecretFile`; `prompt` on an `Option<SecretFile>` ("an
optional secret is not prompted; ask with CallOptions::prompt_secret in the op").

Tests: `tests/secrets.rs` with a fake prompter (SEC-7): missing flag asks with the label, the bytes reach the op,
and the value is redacted from the error line; no prompter: exit 2 `.missing`; `--key-file -` reads stdin and does
not ask; a given path does not ask; JSON without the key is `.missing`; the prompter is never called on JSON, MCP or
typed.

#### SEC-5. `CallOptions::prompt_secret`
gp decision 7. Findings: gait review F1 (the `auth login` paste step, after the URL is printed).

API (features `secrets` and `prompt`):
```rust
impl CallOptions {
    pub fn prompt_secret(&self, label: &str) -> Option<Secret<Zeroizing<String>>>;
}
```
Behaviour: on the CLI, when the run's `Io` has a prompter (stdin and stderr are terminals), it reads one line with
echo off, registers the value and its trimmed form with the call's redactor (SEC-1, SEC-3), and returns it without
the line ending. Everywhere else (typed, json, MCP, or a CLI run with no terminal) it returns `None`, so the op
falls back to its non-interactive form (gait: the two-step `login`, then complete with `--callback-file -`). A read
error returns `None` and sends one notice "cannot read from the terminal: ..". The runner puts the prompter into
`CallOptions` (a private field); no other surface sets it.

Tests: `tests/secrets.rs`: a CLI run with a fake prompter: the op gets the value, and an error containing it is
redacted; the same op over `call`, `call_tool` and `call_typed` gets `None`; a CLI run with no prompter gets `None`.

#### SEC-6. A hint when `-` reads from a terminal
Rows: 53.

Behaviour: when a field reads stdin (`Input` `-`, `Json` `@-`, `SecretFile` `-`) and stdin is a terminal, the
runner writes one notice before reading: "reading --key-file from stdin (end with Ctrl-D)". It goes through the
run's notice sink (text on a terminal, `{"notice": ..}` when stderr is piped) and never contains the value.

Surfaces: CLI only (other surfaces have no stdin).

Tests: `Io::memory(..).with_stdin_tty(true)`: stderr has the hint once; `false`: no hint; the hint never contains
the bytes.

#### SEC-7. `Io`, the `Prompter` trait and the `prompt` feature
Findings: second pass 3(c) (`Io::memory` tests cannot drive a real terminal) and "change `Io` once".

API:
```rust
pub trait Prompter: Send + Sync {
    /// Reads one line from the terminal with echo off, without its line ending.
    fn read_secret(&self, label: &str) -> std::io::Result<Zeroizing<String>>;
}

#[non_exhaustive]
pub struct Io {
    pub stdout: Box<dyn Write + Send>, pub stderr: SharedWriter, pub stdin: Option<Box<dyn Read + Send>>,
    pub stdout_tty: bool, pub stderr_tty: bool,
    pub stdin_tty: bool,                         // new
    pub prompter: Option<Arc<dyn Prompter>>,     // new
}
impl Io {
    pub fn std() -> Io;   // stdin_tty from the process; prompter = the tty prompter when stdin and stderr are
                          // terminals and the `prompt` feature is on
    pub fn memory(stdin: &[u8], stdout_tty: bool, stderr_tty: bool) -> (Io, Captured);   // unchanged
    pub fn with_stdin_tty(self, yes: bool) -> Io;                 // new
    pub fn with_prompter(self, p: Arc<dyn Prompter>) -> Io;       // new
}
```
`Io::memory` keeps its signature, so archegraph's eleven test call sites (archegraph review finding 7) need no edit.
`ParseCx` carries the prompter, `stdin_tty` and the hint sink (private fields).

Feature `prompt` (default on) = `secrets` + one dependency for echo-off terminal reads: `rpassword` 7 (Apache-2.0,
permissive; the implementer confirms the license of the exact version in its crate metadata before adding it).
gait-min builds op-registry with `default-features = false` (spec 11), so it gains no dependency; with the feature
off, prompting fields stay "missing" and `prompt_secret` returns `None`.

Tests: as SEC-4/5/6 through `with_prompter` and `with_stdin_tty`; a build with `--no-default-features --features
manual` compiles and a prompted field is `.missing`.

#### SEC-8. `SecretFile` on JSON and MCP surfaces
Findings: second pass 18.

On `json` and `mcp` a `SecretFile` is a path the server reads (`reg/params/rt.rs:515-537`). The spec gains: "a
`SecretFile` on an op reachable on `mcp` needs `#[op(secret_inline)]`, or the op leaves `mcp` out of its paths."
The kit enforces it in `assert_parity` (KIT-6). No consumer has such an op today.

Tests: `tests/testing_kit.rs`: an MCP-reachable op with a plain `SecretFile` fails parity; with `secret_inline`, or
with `paths: [cli]`, it passes.

#### SEC-9. `write_private` durability
Rows: 23.

Signature unchanged: `write_private(path, bytes, overwrite) -> io::Result<()>`. It now creates missing parent
directories (mode 0700 on unix for each directory it creates), writes, calls `sync_all` on the file, and on unix
fsyncs the parent directory. Mode 0600 and the no-overwrite rule are unchanged (`reg/output.rs:113-137`).

Tests: `tests/output.rs`: a missing parent is created 0700; the file is 0600; an existing file is refused without
`overwrite`.

### 4.6 Outputs and lists

#### OUT-1. Array outputs are `{"items": [...]}` on every JSON surface
gp decision 2. Rows: 49 (replaces the array half of row 2). Findings: second pass 2 (blocker: `$defs`) and 14 (edge
outputs); the lead questions "who wraps a plain `Vec<T>`" (structmux Q4, gait break 1): the library.

Behaviour:
- Value: in the unary runner (`reg/registry.rs:574-584`), an output whose JSON value is an array is returned as
  `{"items": <array>}`. This covers every follow-all half (`reg/list.rs:259-267`), every `Vec<T>` output, and a
  `serde_json::Value` output that happens to be an array. The decision is by value, on every build, with or without
  the `schema` feature.
- Schema: in `describe` (`reg/registry.rs:504-548`), for every kind except `Stream`, a root `"type": "array"`
  schema becomes
  `{"$schema": <moved>, "title": <moved>, "type": "object", "properties": {"items": <the array schema without
  $schema/title/$defs/definitions>}, "required": ["items"], "additionalProperties": false, "$defs": <moved>,
  "definitions": <moved>}`. Hoisting `$defs`/`definitions` keeps every `#/$defs/T` reference valid (schemars 1 puts
  them at the root and items as `{"$ref": "#/$defs/T"}`).
- Not wrapped: stream items (one NDJSON value per item, schema per item); the typed path (returns the Rust
  `Vec<T>`); `--text` (renders the Rust value, OUT-3).
- The kit's list/get check reads the item schema at `/properties/items/items` instead of `/items`
  (`reg/testing/help.rs:166-171`), so the "items differ from get" warning keeps working.

Surfaces: CLI prints `{"items": [...]}`; `call` and `call_tool` return it; MCP `structuredContent` is that object
and `outputSchema` the wrapped schema; manual and `manual --json` show the wrapped schema; typed unchanged.

Derive-time errors: none.

Tests: `tests/output_schema.rs`: `Vec<Named>` output: every `$ref` in the wrapped schema resolves by JSON pointer,
`$defs` is at the root, the root is an object; a stream whose item is a `Vec` is not wrapped. `tests/lists.rs`: CLI
follow-all prints `{"items": [...]}`, the page half `{"items", "nextCursor"}`; `call`/`call_tool` return objects;
`call_typed` returns `Vec<T>`; `--text` prints rows. `tests/testing_kit.rs`: the list/get warning still fires when
items differ from `get` and stays quiet when they match. A `Value` output that returns an array is wrapped.

#### OUT-2. Object roots for either-or outputs, and the rule for non-object outputs
gp decision 2. Rows: 2 (with the addendum edit). Findings: second pass 14 and 16; archegraph's tagged outputs; gait
F3 (`config show` returns `Value`).

Today `reg/mcp/rmcp.rs:59-61` passes any schema object to `with_raw_output_schema` and `:175` puts any value in
`structuredContent`.

Behaviour:
- In `describe`: a root `oneOf` or `anyOf` whose every branch is an object gets `"type": "object"`. A branch is an
  object when it has `"type": "object"`, or `properties`, or is a `$ref` (followed into `$defs`) to one, or an
  `allOf` with an object member. A tagged enum's value is already an object; only the schema root changes.
- Classes of output root after OUT-1 and this step: **object**; **any** (no `type`, no combinator, no `$ref`, no
  `properties`: what a `serde_json::Value` output gives); **non-object** (scalar, nullable, mixed union).
- MCP: `tool_list` gives `outputSchema` only for an object root. In the rmcp adapter a value that is a JSON object is
  `CallToolResult::structured(value)`; any other value is a success with one text block holding the compact JSON
  and no `structuredContent`.
- CLI, typed, `call`: unchanged values.
- The kit fails a non-object root (KIT-6); an "any" root passes. With gp's rule, a consumer gives either-or outputs
  an object form; no consumer has a scalar or mixed output today (second pass 16).
- The library's own `manual` op returns an untagged enum whose `Json` variant holds a `Value`
  (`reg/manual.rs`, `ManualOutput`): its schema is hand-written as `{"type": "object", ...}` (all three variants are
  objects at run time), so the library passes its own kit.

Tests: `tests/mcp.rs`: an internally tagged enum output has an object-root `outputSchema` and structured content;
an enum whose variants are `$ref` newtypes is classified as object; a `u32` output (registered without the kit)
has no `outputSchema` and returns text content; a `Value` output has no `outputSchema` and returns structured
content when the value is an object. `tests/manual.rs`: the `manual` op's schema root is an object.

#### OUT-3. `RenderText` for `Vec<T>`, and `text:` on list pairs
Rows: 13. Findings: second pass 19 (reverses ruling R12 at `reg/registry.rs:89-92`).

API: `impl<T: RenderText> RenderText for Vec<T>` (one item per line, joined by `\n`). A `list:` entry may say
`text: true` (`der/ops/parse.rs:448-454` no longer rejects it; `exits:` without `on` still is): the follow-all half
renders the `Vec<T>`; the page half renders the items, then `next cursor: <c>` when there is one. `respec` sets each
half's `render_text`. A consumer `impl RenderText for Vec<Local>` was already impossible (E0117), so the blanket impl
breaks nothing.

Surfaces: `--text` is CLI only; MCP and typed return JSON.

Derive-time errors: `text:` on a list needs `RenderText` for the item (a trait-bound error).

Tests: `tests/lists.rs`: `--text` on both halves; `tests/ops_ui/ops_text_on_list_is_a_compile_error.rs` deleted.

#### OUT-4. Follow-all page size
Rows: 31.

API: list key `follow_page: 500` (default: `page_size`); `OpSpec.follow_page: Option<u32>`. Used only by the
follow-all half (`reg/list.rs:258-266` today passes `page_size`).

Derive-time errors: `follow_page` on a non-list entry; `follow_page: 0`.

Tests: `tests/lists.rs`: the page function sees `limit = 500` when following and the declared `page_size` as the
page half's default.

#### OUT-5. Follow-all guards, coded
Rows: 37. Today a repeated cursor is an uncoded `anyhow!` (`reg/list.rs:215-217`) and empty pages with fresh cursors
loop forever (the cap counts items only, `:206`).

Behaviour: a repeated cursor is the new `op_registry.list.cursor_repeated` (Logical) from a new `ListError` type
(namespace `op_registry.list`). An empty page that has a next cursor counts one toward the cap, so the cap bounds
pages too and ends with the existing `<pillar>.<noun>.too_many`.

Tests: `tests/lists.rs`: a page function that repeats a cursor fails with the code; one that returns empty pages
with fresh cursors forever ends with `too_many`.

#### OUT-6. Output keys in declaration order
Rows: 34. Findings: second pass 20 (risk checked: gait already enables it; nothing hashes `serde_json::Map` text).

Change: `op-registry`'s `serde_json` dependency enables `preserve_order`. Feature unification turns it on in every
consumer build, so printed JSON, goldens and manual snapshots move once.

Tests: `tests/output.rs`: a struct with fields `zeta, alpha` prints `zeta` first; the manual snapshot
(`tests/manual_snapshot.md`) is re-rendered once.

#### OUT-7. Typed and stream calls return `CallFailure`
Rows: 35 (do it in 0.4.0 or never). Extended to `call_stream`, which also returns raw errors today
(`reg/invoke.rs:111-124`).

API:
```rust
pub async fn call_typed<O: Op<Ctx = C>>(&self, ctx: &C, params: O::Params, opts: CallOptions)
    -> Result<O::Output, CallFailure>;
pub async fn call_stream_typed<'a, O: Op<Ctx = C>>(&'a self, ctx: &'a C, params: O::Params,
    opts: CallOptions, cursor: Option<Cursor>)
    -> Result<BoxStream<'a, Result<O::Output, CallFailure>>, CallFailure>;
pub async fn call_stream<'a>(&'a self, name: &str, args: Value, ctx: &'a C, opts: CallOptions,
    cursor: Option<Cursor>) -> Result<BoxStream<'a, Result<Value, CallFailure>>, CallFailure>;

impl coded_error::CodedError for CallFailure { /* code, class and overrides of the inner error */ }  // new
impl CallFailure { pub fn into_coded(self) -> anyhow::Error; }  // new: keeps the code, Display redacted
```
Why `into_coded`: `CallFailure`'s `source()` is empty on purpose (its messages are unredacted), so a plain `?` into
`anyhow::Error` gives an uncoded error (coded-error's `facts_anyhow` looks for a `Coded` in the chain,
`coded-error/src/carry.rs:269-285`). `into_coded()` wraps it with `coded_error::into_anyhow`, so the code and class
survive and the message stays redacted. Downcasting to the concrete error type uses `failure.error()` (spec 4.4
holds). The docs say: on a typed path, use `.map_err(CallFailure::into_coded)?`, not a bare `?`.

Surfaces: typed and json streams.

Tests: `tests/call.rs`: a typed call's error redacts a secret param; `failure.error()` downcasts to the op's error
type; `facts_anyhow(&failure.into_coded())` has the op's code; stream open and item errors are `CallFailure`s.

#### OUT-8. Typed stream that outlives the borrow
Rows: 29.

API:
```rust
impl<C: Resolve + Send + Sync + 'static> Registry<C> {
    pub async fn call_stream_typed_owned<O: Op<Ctx = C>>(self: Arc<Self>, ctx: Arc<C>, params: O::Params,
        opts: CallOptions, cursor: Option<Cursor>)
        -> Result<BoxStream<'static, Result<O::Output, CallFailure>>, CallFailure>;
}
```
The stream holds the two `Arc`s. Same checks as `call_stream_typed` (paths, validate, hook around the open).

Tests: `tests/call.rs`: the returned stream is moved into a spawned task after the registry and context handles in
the caller are dropped, and yields every item.

### 4.7 Streams (op-listen, op-status)

#### STR-1. A stop item delivered once; stop codes on mid-stream errors
Rows: 33. Findings: multimatrix review 2 (stop codes for a mid-stream error).

Today `run_connection` emits `Frame` then `Stopped` for one stop item (`listen/follow.rs:544-552`), and stop codes
are checked only when `connect` fails (`:441-450`); `follow`, `follow_typed` and `follow_items` pass no stop codes
(`:217`, `:262`, `:295`).

API:
```rust
#[non_exhaustive]
pub struct FollowOptions<C = ListenCursor> {
    pub initial_cursor: Option<C>, pub backoff: Backoff, pub max_retries: Option<u32>,
    pub stop_codes: Vec<String>,          // new: every follower honours it
}
pub async fn supervise<T, F, K>(connect: F, opts: FollowOptions<T::Cursor>, sink: K)
    -> anyhow::Result<FollowEnd>;        // the `stop_codes` parameter moves into FollowOptions
```
Behaviour: `supervise` and `follow_items` deliver a stop item only as `Stopped { code }` (no `Item` first). `follow`
and `follow_typed` keep writing the stop frame, then the stopped marker (README:149 behaviour). In every follower,
an error during a connection whose code is in `stop_codes` ends the follow as `Stopped { code }` before the
Logical check; any other code keeps today's rule (Logical ends with `Err`, other classes retry).

Tests: `op-listen/tests/follow_items.rs`, `supervise.rs`: one `Stopped` for a stop item; a mid-stream error with a
listed code ends `Stopped`; an unlisted Logical code ends `Err`; an unlisted Transient code retries.

#### STR-2. A non-terminal per-topic stop
Findings: multimatrix review 2 (`agent-room listen` reopens after a per-topic `Stop`).

No new API. Rule, documented in the op-listen README: only an item whose `Resumable::stop_code()` is `Some` ends a
follow, and the `ListenFrame` impl (STR-3) maps only `ListenFrame::Stop`. A per-topic stop is data: the consumer
carries it inside its `Item` type (multimatrix: `TopicStop` as an item variant), whose `stop_code()` is `None`, and
uses `stop_codes` (STR-1) for the terminal case.

Tests: `op-listen/tests/follow_items.rs`: an item type with a per-topic stop variant flows through and the follow
continues; a later terminal stop ends it.

#### STR-3. Typed followers
Rows: 30.

API:
```rust
impl<S, I> Resumable for ListenFrame<S, I> {
    type Cursor = ListenCursor;   // cursor(): the frame's; stop_code(): Stop's code; resync(): Snapshot.resync
}
pub async fn supervise_stream<T, F, K>(connect: F, opts: FollowOptions<T::Cursor>, sink: K)
    -> anyhow::Result<FollowEnd>
where T: Resumable + Send + 'static,
      F: FnMut(Option<T::Cursor>) -> BoxFuture<'static, anyhow::Result<BoxStream<'static, anyhow::Result<T>>>>,
      K: FnMut(Supervised<T>) -> std::io::Result<()>;
```
`supervise_stream` takes a plain stream (no `StreamStep` wrapper); the end of a stream is `Ended` (Transient,
retried). Today `follow_typed` passes no stop codes and drops `Opened`/`Retrying` (`follow.rs:226-266`);
`supervise_stream` delivers them.

Tests: `op-listen/tests/follow_typed.rs`: typed frames resume from the last cursor; a stop frame ends with its
code; stream end reconnects.

#### STR-4. A reconnecting follower as a stream op
Rows: 11, with the addendum edit (the listener shapes the reconnect marker). Findings: second pass 1 (blocker: the
kit hangs on a follower), grorg review 6 (marker shape, `Resnapshot`, params and cursor pass-through, stop once).

API:
```rust
// op-listen
#[derive(Serialize, JsonSchema)] #[serde(untagged)]
pub enum Followed<F> { Frame(F), Marker(serde_json::Map<String, Value>) }

pub fn follow_stream<'a, T, F>(connect: F, opts: FollowOptions<T::Cursor>,
                               marker: impl Fn(Option<&T::Cursor>, bool) -> Map<String, Value> + Send + 'a)
    -> BoxStream<'a, anyhow::Result<Followed<T>>>
where T: Resumable + Send + 'a,
      F: FnMut(Option<T::Cursor>) -> BoxFuture<'a, anyhow::Result<BoxStream<'a, anyhow::Result<T>>>> + Send + 'a;

pub fn listen_entry_following<C, L>(registry: &mut Registry<C>, listener: Arc<L>,
    spec: OpSpec<L::Params, Followed<ListenFrame<L::Snapshot, L::Item>>>)
where C: Resolve + 'static, L: Listener<C>;

pub trait Listener<C> {                  // two new defaulted methods
    fn reconnect_marker(&self, cursor: Option<&ListenCursor>, resync: bool) -> Map<String, Value> {
        /* {"marker":"reconnected","cursor":<c or null>,"resync":<b>} */
    }
    fn follow_options(&self) -> FollowOptions { FollowOptions::default() }
    ...
}

// op-registry
impl CallOptions {
    pub max_retries: Option<u32>,                        // new pub field: a hint for reconnecting streams
    pub fn with_max_retries(self, n: u32) -> Self;
}
#[non_exhaustive] pub enum ListenError { ..., Stopped { code: String } }   // new: op_listen.stopped, Logical
```
Behaviour of `follow_stream` (a stream, not a callback, so it can be an op body; it borrows the context for `'a`):
1. Connects with the last cursor (or the initial one), reconnects with the backoff, and stops after
   `max_retries` consecutive failures with the last error.
2. Before the first item of each reconnection it yields `Followed::Marker(marker(cursor, resync))`.
3. Yields every item as `Followed::Frame`.
4. On a stop item (`stop_code()` is `Some`), it yields the stop frame once, then ends with
   `Err(op_listen.stopped { code })`, Logical, so the CLI exit follows the class (65). A mid-stream error whose code is
   in `stop_codes` ends with the same error without a frame. Other Logical errors end with that error.

`listen_entry_following` registers a stream op whose body is `follow_stream` over `listen_stream(listener, ctx,
params.clone(), cursor, caller)`: params and `--cursor` pass through unchanged (grorg `events watch`'s
`one_of_optional` and cursor-over-`--after-seq` rule hold), `OverflowPolicy::Resnapshot` works because it lives
inside `listen_stream`, and `max_retries` is `opts.max_retries` when set, else `listener.follow_options()`.

grorg overrides `reconnect_marker` to keep `{"marker":"reconnected","afterSeq":N}` (N from the cursor's seq). The
library's default carries `cursor`, which grorg's `watch-actions.sh` would read into its cursor state, so grorg
must override (grorg review 6a).

Surfaces: CLI prints NDJSON (markers and frames). typed: `call_stream_typed` yields `Followed<ListenFrame<..>>`.
json: `call_stream` yields the same values. MCP: streams are not tools (spec 5.5).

Tests: `op-listen/tests/follow.rs` (new cases): a fake connect that fails twice then serves: marker before the
first frame of the reconnection; `max_retries: Some(0)` ends after the first failure; a stop item yields the frame
once then `op_listen.stopped`; a listed mid-stream code ends with it; a grorg-style marker override is printed
verbatim; `Resnapshot` overflow still resnapshots under `follow_stream`. `op-listen/tests/registry.rs`: a CLI run of a
`listen_entry_following` op prints NDJSON and exits 65 after a stop.

#### STR-5. op-listen `registry` feature
Rows: 45.

Change: a `registry` feature (default on) makes `op-registry` and `schemars` optional. Under it: `entry`,
`listener`, `run`, `detach` (which already needs `op-registry`), the `Caller` re-export, `ListenCursor`'s
`TryFrom<&op_registry::Cursor>` and `JsonSchema` impls, and `ListenFrame`'s `JsonSchema` derive (as
`cfg_attr`). Core without it: `follow`, `backoff`, `cursor`, `frame`, `error`, `stdout`. concordium-app's
follower-only client builds with `default-features = false`.

Tests: CI builds op-listen with `--no-default-features` and runs `follow`/`follow_typed` tests in that mode.

#### STR-6. op-status: a remote snapshot source, and the exported group summary
Rows: 16 (with grorg added), 46.

API:
```rust
pub trait StatusSource {
    fn remote_snapshot<'a>(&'a self, filter: &'a StatusFilter, max: usize)
        -> Option<BoxFuture<'a, anyhow::Result<StatusSnapshot>>> { None }   // new, default None
    ...
}
pub const GROUP_SUMMARY: &str = "Component status.";   // new; status_registry uses it (status/ops.rs:107)
```
`take_snapshot` prefers `remote_snapshot` when it returns `Some` (the owner CLI proxies the daemon's snapshot). A
consumer's own `status` group uses `GROUP_SUMMARY`, so `merge` does not panic on a different summary
(`reg/registry.rs:326-336`).

Tests: `op-status/tests/ops.rs`: a source with `remote_snapshot` returns its snapshot unchanged through `status get`;
`merge` with a consumer group declared from `GROUP_SUMMARY` passes.

### 4.8 Errors and exits

#### ERR-1. An error code with a declared exit
Rows: 18. Findings: second pass 13 (grorg's code is `grorg.client.wait_timeout`; `Outcome` and the manual must use
the declared exit).

API:
```rust
"action wait" => { ..., exits: { 3 => "timed out" on "grorg.client.wait_timeout" }, ... }
pub struct OpSpec<P, O> { pub error_exits: Vec<(u8, &'static str, &'static str)>, ... }  // new: (exit, meaning, code)
pub struct Exit { pub code: u8, pub meaning: String, pub on: Option<String> }          // new field `on`
#[non_exhaustive] pub enum Outcome {
    Ok { op: String, exit: u8 },
    Err { op: Option<String>, error: anyhow::Error, message: String, exit: u8 },        // new field `exit`
}
```
An exit with `on "<code>"` is an error exit; one without is an ok exit as today. `OkExit` on the output is needed only
when the entry has ok exits (today any `exits:` sets `ok_exit`, `der/ops/expand.rs:128-134`). Error exits are
allowed on list entries; ok exits stay rejected there.

Behaviour: CLI: an error whose code matches exits with the declared number; the error line is still written;
`Outcome::Err.exit` is that number and `Outcome::exit_code()` returns it, so `post_run` and telemetry see 3, not 65.
typed, json, MCP: unchanged (the code carries it). The manual's exits list says "- 3: timed out (on
grorg.client.wait_timeout)" and its code table shows 3 in the exit column for that code. `assert_exit_table`
fails when an error exit's code is not among the op's codes (own or common).

Derive-time errors: an exit number twice in one entry; `on` without a string literal; the existing 3..=63 range.

Tests: `tests/runner.rs`: the timeout code exits 3 with the error line; `post_run` sees exit 3; another code exits
by class. `tests/manual.rs`: the line and the table. `tests/testing_kit.rs`: an undeclared `on` code fails
`assert_exit_table`.

#### ERR-2. One error mapper per registry
Rows: 19.

API: `Registry::set_error_mapper(&mut self, mapper: fn(anyhow::Error) -> anyhow::Error)`. Applied once, in the
pipeline, to the error of resolution or the op (and to each stream item error), before `on_error`, strict codes, the
CLI exit and every surface's error output. Not applied to decode and validate errors (usage). `merge` keeps the
receiver's mapper, as it keeps the receiver's hook.

Surfaces: every surface.

Tests: `tests/call.rs`: an op returning an `io::Error` mapped to a coded error shows the mapped code on CLI, `call`,
`call_tool` and typed; the hook's `on_error` sees the mapped error; a usage error is not mapped.

#### ERR-3. Library codes added in 0.4.0
New codes: `op_registry.usage.conflicts`, `op_registry.usage.at_least_one`, `op_registry.list.cursor_repeated`,
`op_listen.stopped`. `UsageError::MissingUnless` shares `op_registry.usage.missing`; the manual's library-code table
de-duplicates by code. `UsageError`, `ListenError` and the new `ListError` are `#[non_exhaustive]`.

Tests: `tests/manual.rs`: the library-code table lists each code once.

### 4.9 Manual and help

#### MAN-1. Common codes printed once
Rows: 50 (and row 15's edit). Findings: grorg review 7 (13,037-line manual).

The Markdown manual prints "## Common codes" once, after the exit table; each op's code table lists only its own
codes (and its cap code), then "Also the common codes." `render_json` gains top-level `commonCodes`; each op's `codes`
holds its own only. Strict codes, `assert_error_codes` and `assert_exit_table` use own ∪ common.

Tests: `tests/manual.rs`: a header code appears once; op tables do not repeat it; JSON has `commonCodes`.

#### MAN-2. Extras and the fallback in the manual
Rows: 43.

The manual gains "## Other commands": each extra's name and `about` (and its long about), and the fallback's name and
full help. They are listed as CLI-only, "not an op". `render_json` gains `otherCommands`.

Tests: `tests/manual.rs`: an extra and a FreeText fallback appear with their help.

#### MAN-3. A fallback's help is findable from root help
Rows: 52. Findings: gait F10 (the help says `gait` in `gait-min`).

Root help lists `<name>: <first help line> (more: <bin> help <name>)` (today `reg/cli/fallback.rs:50-56`). In a
fallback's `help`, `{bin}` is replaced by `Registry::bin()` wherever the help is shown.

Tests: `tests/fallback.rs`: root help has the pointer; `gait-min help turn` shows `gait-min`.

#### MAN-4. Ops off the command line, and the paths line
Follows from PATH-1 (an op with `paths: [mcp]` has no CLI help, and the manual renders CLI commands only today).

The manual keeps one section per CLI command, adding a "Paths: cli, typed" line when an op is not on every surface. A
new section "## Operations off the command line" lists every visible op without `cli`: its name, summary, long help,
paths, access, exits and codes. `check_exit_table` checks those entries too.

Tests: `tests/manual.rs`: a `[mcp]` op and a `[typed]` op are listed with their paths.

#### MAN-5. Relations, needs, env and prompts are shown
From REL-3, REL-4, DEC-4, PAR-10, SEC-4.

`OpParams` gains `fn relations() -> Vec<String> { Vec::new() }` (a default method; the derive emits plain lines such
as "--message is required unless --signed-file is given", "--to conflicts with --signed-file", "give exactly one of
--id, --action"). `Descriptor.relations: Vec<String>` (new). The op's `--help` ends with them (as `after_long_help`),
and the manual prints them under "Rules:". The needs line (DEC-4), `[env: X]` (PAR-10) and the prompt sentence
(SEC-4) are in the help text.

Tests: `tests/manual.rs` and `tests/cli_tree.rs`: a struct with each relation kind lists them; help text holds them.

#### MAN-6. Help wording per subject kind
Rows: 24.

`assert_help_complete` keys the one-wording rule for `--id` on (flag, subject kind), using `FieldMeta.subject`; every
other flag keeps one wording.

Tests: `tests/testing_kit.rs`: `--id` worded differently for kinds `session` and `provider` passes; two wordings for one
kind fail.

#### MAN-7. A leaf param named `version` is help-checked
Rows: 38.

`walk` skips only the root's own `help`/`version` args (`reg/testing/help.rs:101-105` skips every arg with those
ids), so `policy sign --version` must have help.

Tests: `tests/testing_kit.rs`: a leaf `--version` param without help fails.

### 4.10 The testing kit

#### KIT-1. The kit runs every op it can reach, on a surface it declares
Rows: 3, with the addendum edit (calls carry a caller) and row 54's dependency.

Today `assert_local_ops_offline` and `assert_unknown_ref_not_found` skip ops whose paths are not `All`
(`reg/testing/offline.rs:23`, `reg/testing/refs.rs:57`), and `call_errors` uses `Registry::call`
(`reg/testing/codes.rs:81-88`), which refuses a CLI-only op with `wrong_path`, which `offence()` then allows because
every `op_registry.*` code is allowed (`codes.rs:33`): the fixture passes and checks nothing.

Design: every kit runner calls an op through the crate-internal JSON entry on the first surface its paths contain, in
the order `json`, `mcp`, `cli` (a CLI-only op is decoded from JSON and run with `CallPath::Cli`). An op whose paths
are only `typed` is skipped and named in the report ("typed only: not run"). Kit calls carry
`CallOptions::caller = Some(Caller("op-registry-kit"))`, so listen ops reach their body instead of
`op_listen.no_caller` (`listen/entry.rs:35-37`), and `max_retries = Some(0)` (KIT-2). `op_registry.call.wrong_path`
and `op_registry.call.wrong_shape` become offences in `offence()`.

Consequence, intended: consumers' kit suites that passed vacuously now run. Fixtures must exist (structmux and
multimatrix say so).

Tests: `tests/testing_kit.rs`: a `[cli]` op is run by `assert_error_codes`, `assert_local_ops_offline` and
`assert_unknown_ref_not_found`; a `[typed]` op is skipped and named; a `wrong_path` result is an offence; a listen op
reaches its body.

#### KIT-2. Bounded stream calls
Findings: second pass 1 (blocker). Lands with KIT-1 and before STR-4.

Today `call_errors` drains a stream to its end (`codes.rs:64-79`); with a caller (KIT-1) and a reconnecting follower
(STR-4, `FollowOptions::max_retries` defaults to `None` = forever, `follow.rs:85-93`) it would hang.

Design: for a stream op the kit stops at the first item (`Ok` or `Err`) or at the end, and runs the whole call under a
10-second timeout. A timeout is a problem: "<op>: the stream gave no item and did not end within 10 s; give a
Fixture or a context that answers". Kit calls set `max_retries = Some(0)`, which `follow_stream` honours. The
`testing` feature adds `tokio` with `time` (the consumers' test runtimes are tokio).

Tests: `tests/testing_kit.rs`: a stream that never yields is reported after the timeout (with a paused-time test
runtime); an endless stream is cut after its first item; a failing follower ends after one attempt.

#### KIT-3. The not_found test covers every subject except a new one
gp decision 3. Findings: second pass 9, grorg review 1, gait F5, structmux check break 3.

`assert_unknown_ref_not_found(registry, ctx, overrides)` keeps its signature. It runs on every op that has a subject
(`FieldMeta.subject` is `Some`) whose subject is not `new`, including `id_only` subjects, server-resolved kinds,
optional subjects and streams, on the surface chosen by KIT-1. It seeds `id` with the random ref before sampling the
rest (`sample_params_seeded`, KIT-4), so `if`/`then` rules keyed on `id` are honoured. It expects a Logical error
whose code ends `.not_found`. The subject is found by `FieldMeta.subject`, not by "a `Ref` keyed `id`"
(`reg/testing/refs.rs:10-21`). No exemption list.

Tests: `tests/testing_kit.rs`: an `id_only` subject whose op answers a value fails the test; one that answers
`.not_found` passes; a `new` subject is not run; an optional subject is run with `id` set.

#### KIT-4. Sampling that decodes
Rows: 54, 55, the row-4 and row-5/7 edits. Findings: second pass 10.

API:
```rust
pub fn sample_params(schema: &Value) -> Value;                                   // unchanged
pub fn sample_params_seeded(schema: &Value, seed: Map<String, Value>) -> Value;  // new
```
Rules added to the sampler:
- a string with `examples` takes `examples[0]` (PAR-6);
- REL-6 fragments: a chosen `oneOf`/`anyOf` branch's fragment is satisfied (`const: true` gives `true`, `minItems: n`
  gives n items, `minProperties` gives `{"k": "x"}`, `not null` gives a sampled value); `if`/`then` adds the `then`
  fragment when the `if` key is present; `not: {allOf: [..]}` is respected by never adding the last key that would
  satisfy it; a property's `minItems: n` gives n items; `dependentRequired` reading is removed;
- seeded keys are placed first and kept.

Kit runners post-process sampled arguments using the op's `FieldMeta`: a `Path` field gets `<fresh temp dir>/<key>`
(so `--out` never writes into the crate); a `SecretFile` field gets a temp file holding a fixed sample line. The temp
dir is made under `std::env::temp_dir()` per run and removed after it (no new dependency). Overrides (`Fixture`) always
win.

When a sampled call fails with `op_registry.usage.*` (a validator or a parser rejected the sample), the runners report
"<op>: the sampled arguments did not decode (<code>: <message>); give a Fixture" as a problem, instead of passing
vacuously (`assert_local_ops_offline` counts any non-panic as a pass today, `offline.rs:26-35`).

Tests: `tests/testing_kit.rs`: each sampler rule; a `one_of` over `--id`/`--all` samples one present member; a
required `Vec` gets one item; a `PathBuf` output lands in the temp dir; a `from_str` field with `sample` decodes; one
without it is reported as "did not decode".

#### KIT-5. The secret sentinel
Rows: 22. Findings: second pass 11 (gait builds the struct literally at two sites), row 10's kit edit, blocker 5's kit
note.

API:
```rust
#[non_exhaustive]
pub struct SecretSentinel<'a> { pub value: &'a str, pub key_file: &'a Path, pub globals: Vec<OsString> }
impl<'a> SecretSentinel<'a> {
    pub fn new(value: &'a str, key_file: &'a Path) -> Self;
    pub fn with_globals(self, globals: &[&str]) -> Self;
}
```
Changes to `assert_secrets_not_echoed`:
- the globals go right after the binary name in every argv it runs (structmux owner ops fail in `session()` without
  `--addr`/`--dir`);
- a `#[op(secret)]` `Path` field gets the sentinel key file's path (today it gets the sentinel value,
  `reg/testing/secrets.rs:76-89`);
- a prompted `SecretFile` (SEC-4) is also run once with the flag left out and a fake prompter that answers the sentinel
  value;
- it runs with a recording `RunHooks` and adds every `on_usage_error` message and every `post_run` `Outcome` message to
  the places it searches;
- it checks absence only (addendum row-10 edit).

The app's own `CallHook::on_error` is not visible to the kit; SEC-1's library tests cover the hook.

Tests: `tests/secrets.rs`: globals reach the argv; a key file read and registered passes; echoed contents fail; a prompted
field is exercised; an `Outcome` message holding the sentinel fails.

#### KIT-6. Parity is exact, and output roots are objects
PATH-1, OUT-1, OUT-2, SEC-8. Findings: second pass 14(b).

`assert_parity(registry, mounts)` keeps its signature. For each op and each surface (`cli`, `mcp`, each mount as
`json`): reachable exactly when declared. Allowed absences: not declared; a stream on `mcp` or a mount ("stream"); the
page half of a list on `cli` (the pair is one command). Problems: declared but absent; present but not declared; a
non-object output root (not "any") on an op reachable on `mcp` or `json` ("<op>: the output root is <kind>; give it an
object form"); an MCP-reachable op with a plain `SecretFile` (SEC-8). `parity_report` lines name the declared paths.
`Reach`/`ParityLine`/`Mount` are unchanged; the reason "cli only: <reason>" is gone.

Tests: `tests/testing_kit.rs`: each problem and each allowed absence; a `Value` output passes; a `u32` output fails.

### 4.11 Fallbacks and extras

#### FB-1. Extras and fallbacks return `anyhow::Result<u8>`
Rows: 9.

API:
```rust
pub fn push_extra(&mut self, command: clap::Command,
    run: for<'a> fn(&'a C::Globals, &'a ArgMatches) -> BoxFuture<'a, anyhow::Result<u8>>);
pub fn fallback_flags_only(&mut self, name: &'static str, help: &'static str,
    run: fn(Vec<OsString>) -> BoxFuture<'static, anyhow::Result<u8>>);
pub fn fallback_free_text(&mut self, name: &'static str, help: &'static str,
    flags: &'static [FallbackFlag], near_miss: bool,
    run: for<'a> fn(&'a C::Globals, Vec<OsString>) -> BoxFuture<'a, anyhow::Result<u8>>);   // FB-2
```
`Err` gives `post_run(Outcome::Err)`, then the error line redacted with the secret globals, exit by class (2 for a usage
code); `Ok(n)` exits `n` as today. The internals already carry `Result` (`reg/cli/extra.rs:10-11`,
`reg/cli/fallback.rs:13-17`, `reg/cli/run.rs:320-329`). Not ops: no MCP, no typed.

Tests: `tests/fallback.rs`, `tests/runner.rs`: an extra and each fallback returning a coded `Err` write the error line
last and exit by class; `post_run` sees `Err`.

#### FB-2. The FreeText fallback: empty rest, globals anywhere, declared value flags, near-miss switch
Rows: 26, with the addendum edit (declared value flags, shorts). Findings: gait F6.

API:
```rust
#[derive(Clone, Copy, Debug)]
pub struct FallbackFlag { pub long: &'static str, pub short: Option<char>, pub takes_value: bool }
```
Routing (spec 2.3, updated):
1. Leading registry globals and their values are skipped, then the first token routes as today: `--` sends the rest
   unchecked; a token starting with `-` goes to the fallback; a command or group word runs the command; a single token
   with no whitespace within edit distance 2 of a command is a usage error only when `near_miss` is true; anything else
   goes to the fallback.
2. (a) An empty rest (nothing but globals) goes to the fallback with an empty `Vec` (today it returns to the normal
   parse and exits 2, `fallback.rs:204-207`).
3. (b) When the fallback runs, registry globals are taken out of the whole tail up to `--`, so `gait "task" --gait-dir
   d` works. A declared fallback flag with `takes_value` keeps its next token as its value, unscanned, so
   `--allow --pretty` gives `--allow` the value `--pretty`. Tokens after `--` are never scanned. All taken globals are
   parsed with clap as today.
4. The clash check (`cli::command`, `reg/cli/tree.rs:85-94`) compares longs and shorts with registry and library
   globals (`-h` included).

Tests: `tests/fallback.rs`: empty argv and globals-only reach the fallback with `[]`; trailing globals are taken; a
value flag's value that spells a global is kept; `--` stops the scan; `near_miss: false` sends `stat` to the fallback;
a short clash panics.

#### FB-3. Extra args that reuse a global name fail at build
Rows: 44.

`push_extra` panics when the command (or a subcommand) has a long flag equal to a registry global or a library flag
(`pretty`, `compact`, `text`, `help`, `version`), naming the extra and the flag. `cli::command` repeats the check for
extras added by `merge`.

Tests: `tests/runner.rs`: an extra with `--pretty` panics; one with `--name` (not a global) passes.

#### FB-4. `RunHooks::on_selected`
Rows: 36.

API:
```rust
#[non_exhaustive]
pub enum Selected { Op { name: String }, Extra { name: String },
                    Fallback { name: &'static str, args: Vec<OsString> }, Help, Version }
pub trait RunHooks { fn on_selected(&self, selected: &Selected) {} ... }   // new default method
```
Called by `run_with_io` once a parse succeeds, before the context is built and before an extra or fallback runs
(multimatrix picks its telemetry mode; structmux skips telemetry before a fork).

Tests: `tests/runner.rs`: each selection kind reaches the hook once; a usage failure does not.

#### FB-5. A sync selection for hosts that act before any runtime
Findings: structmux review 1 (blocker for its plan: `structmuxd` forks, then builds its own runtime; inside the
library's `async` runner that loses worker threads and panics on a nested `block_on`). gait F7 (read `--agent-dir`
before the runtime exists).

API:
```rust
pub fn select<C: CliContext>(registry: &Registry<C>, argv: &[OsString]) -> Result<Selected, UsageFailure>;  // sync
impl<C: CliContext + Sync> Registry<C> {
    pub fn fallback_flags_only_before_runtime(&mut self, name: &'static str, help: &'static str);  // new
}
```
`select` runs the same routing and parse as `cli::parse` (which is already sync, `reg/cli/parse.rs:116`) and returns
what was chosen. A FlagsOnly fallback registered with `fallback_flags_only_before_runtime` has no runner: it routes,
shows in help and in the manual, and `help <name>` works, but the host must dispatch it itself after `select` (on the
main thread, before any runtime). If `cli::run` ever reaches it, the run fails with an uncoded error naming
`cli::select` (exit 70). gait reads its globals before the runtime through `cli::parse`, which returns the typed
globals.

Host pattern, in the README:
```rust
match op_registry::cli::select(&registry, &argv) {
    Ok(Selected::Fallback { args, .. }) => daemon_main(args),          // forks, builds its own runtime
    _ => current_thread_runtime().block_on(cli::run(&registry, argv, &hooks)),
}
```

Tests: `tests/fallback.rs`: `select` gives `Fallback` for an empty or flag-led argv and `Op` for a command; a bare
unknown word is a usage failure; `cli::run` reaching a before-runtime fallback exits 70.

### 4.12 Docs only

- **DOC-1** (row 47): op-listen README: `CliContext::caller()` must return `Some` for listen ops on the CLI;
  `status_registry` needs a `StatusSource`; `listen_entry_following` and the marker override.
- **DOC-2** (row 48): README: the `schema` feature is not additive across a workspace build (`reg/output.rs:99-111`);
  build `gait-min` with `cargo build -p gait-min`.
- **DOC-3** (merged 3c, and changes above): spec text fixes: a second stdin reader is a run-time
  `op_registry.usage.stdin_twice`; extras return `Result<u8>`; `resolve_server = [..]` spelling; a missing `--limit`
  takes the default only with `--cursor`; strict codes are opt-in and `op_registry.*` is allowed; the kit skips
  typed-only ops; CHANGELOG/README "removed" list loses param `env` and gains notes that `conflicts_with` and
  `required_unless` are back; globals after the verb hold for FreeText (FB-2); redaction holds on every surface (SEC-1,
  OUT-7); section 4.4's path rule becomes PATH-1; 5.3 lists become `{"items"}` (OUT-1) and R12 is reversed (OUT-3);
  signal exits (130/143) belong to the binary, not the runner (archegraph review 5); a `SecretFile` on MCP needs
  `secret_inline` (SEC-8).

---

## 5. Every consumer feature has a home

| feature (consumer) | home |
|---|---|
| `auth add` asks for the key on a terminal, no echo (gait) | SEC-4 `#[op(prompt)]` on `key_file` |
| `auth login` openai-signin paste step after the URL (gait) | SEC-5 `prompt_secret`; off a terminal the two-step form |
| `--key-file -` / piped secrets for scripts (gait, structmux) | unchanged, plus the SEC-6 hint |
| `--owner-principal` from `GRORG_OWNER_PRINCIPAL` (grorg) | PAR-10 |
| reconnect marker `{"marker":"reconnected","afterSeq":N}` (grorg) | STR-4 `Listener::reconnect_marker` override |
| `status watch` over gRPC with resnapshot (grorg) | STR-4 (Resnapshot inside `listen_stream`), STR-6 |
| per-topic non-terminal stop in `agent-room listen` (multimatrix) | STR-2 (item variant) + STR-1 stop codes |
| `room messages` for owner and agent (multimatrix) | DEC-1 audiences; one page shape (gp 12) |
| `dm list --limit N` newest N (multimatrix) | the page half of the list pair (`dm list --limit N`) |
| `--signed-file` excludes `--identity-dir` (multimatrix) | dropped by the consumer: a global cannot be a relation member; `--signed-file` reads no key |
| `--signed-file` / `--to` / `--message` rules (multimatrix) | REL-2, REL-3, or the consumer's two-op split |
| `--newest` in a group with `--after/--before` (multimatrix) | REL-1 |
| `status get` key order (multimatrix) | OUT-6 |
| owner CLI proxies the daemon's status (multimatrix) | STR-6 `remote_snapshot` |
| daemon run forks before any runtime (structmux) | FB-5 |
| daemon flags discoverable from help (structmux) | MAN-3 pointer, fallback `help` text, MAN-2 |
| `--confirm-fingerprint` required unless `--dry-run` across the shared group (structmux) | REL-3 + REL-5 |
| at least one `--grant` / `--entry` (structmux) | REL-2 |
| `SocketAddr`, `PrincipalId`, `Grant` values (structmux) | PAR-6 |
| malformed principal id is exit 2 (structmux) | PAR-5 |
| `[default: 0]` meaning "the daemon's own" (structmux) | PAR-9 `hide_default` |
| owner key file read with the app's checks, redacted (structmux, multimatrix) | SEC-2 |
| `setup --owner-key-out` survives a crash before the claim (structmux) | SEC-9 |
| no telemetry before the fork (structmux) | FB-4 `on_selected` |
| per-op `--addr`/`--dir`/`--gait-dir` needed (structmux, gait, multimatrix) | DEC-4 |
| `local` ops behind `--features local` (gait) | DEC-3 |
| `config set --value -1`, `--args -y pkg` (gait) | PAR-7 |
| turn with globals after the task, empty argv (gait) | FB-2 |
| `gait stat` near-miss refusal (gait) | FB-2 `near_miss: true` |
| workflow and turn failures with an error line (gait) | FB-1 |
| `--id` or `--all` on persist/discard, `--all` needs `--yes` unless `--dry-run` (gait) | PAR-1, REL-1, REL-7 |
| kind-dependent register rules, exit 2 (gait) | REL-7, REL-1 |
| unknown session id reported (gait bug 1) | KIT-3 now tests `id_only` subjects |
| `undo --list` vs `--turn` (gait) | REL-3 |
| optional build subject, "newest"/"the paused one" (archegraph) | PAR-1 |
| `--id` without `--blueprint` (archegraph) | already works; PAR-2 test and README |
| one-word list paths `graph-search` (archegraph) | DEC-5 |
| list text output (archegraph, structmux) | OUT-3 |
| one graph load per follow (archegraph) | OUT-4 |
| `--name/--file/--kind` at least one (archegraph) | REL-4 |
| `--show` excludes `--code-root`/`--lang` (archegraph) | REL-3 + REL-1 |
| `classify` of io/prost/git errors in one place (archegraph) | ERR-2 |
| per-call MCP state (`recorded_build_id`), server instructions (archegraph) | DEC-6 |
| tagged enum outputs on MCP (archegraph) | OUT-2 |
| `--kind` enum, any case (archegraph, grorg prompts) | PAR-8 |
| `[default: syntax]` in help (archegraph) | PAR-9 |
| `graph-neighbors` `{symbol, edges, truncated}` with `--limit` (archegraph) | a plain op; a plain op may declare `limit` (only list pages add it, `reg/list.rs:94-99`) |
| `action wait` timeout distinguishable (grorg) | ERR-1 exit 3 |
| shared server error catalogs (grorg, multimatrix) | DEC-2, MAN-1 |
| a committed manual of reasonable size (grorg) | MAN-1 |
| `--waits-for` nameless kind, `--home-place` clearable ref (grorg) | PAR-2, PAR-3 |
| `agent status` defaults to me (grorg) | PAR-1 |
| typed bridge streams owned by a task (concordium-app) | OUT-8, STR-3 |
| follower-only client without clap/schemars (concordium-app) | STR-5 |
| redacted typed errors (concordium-app) | OUT-7 |

---

## 6. Explicitly not library work

Each item belongs to the named consumer. Callers outside the pillars are in the integration list.

### grorg
- Rename `grorg.action.agent_not_found` / `session_not_found` to `grorg.agent.not_found` / `grorg.session.not_found`
  in grorg-ops, grorg-server and the catalog; merge the duplicate client enums; delete the kit test's KNOWN list
  (gp decision 11; grorg review 3). A server wire change: deploys with grorg-server.
- `paths: [cli]` on `agent register` and `agent key ensure/renew/rotate-root/revoke-device` (gp decision 5); update
  `canonical-ops.txt`, the parity expectation and help tests.
- Adopt PAR-10 on `--owner-principal`; delete the hand read at `agent_key.rs:269-273`.
- `--adopt-root`/`--adopt-owner-root` as a `one_of_optional` over two switches (REL-1).
- `out` params from `String` to `PathBuf` (grorg review 13) so KIT-4 samples them into a temp dir.
- Watch ops onto `listen_entry_following` with the marker override; delete `follow.rs`.
- Commit the manual after MAN-1; fold `assigneeIds` into list items; `Pb<T>` mirror/Wire for camelCase.
- `Patch<Ref>` for `--home-place` is optional (grorg review 10).
- `grorg.fleet.*` area for place/environment/harness/provider/model: kept (the kit checks only the `.not_found`
  suffix); gp may say one line (grorg review 12).
- Machine to Place gRPC rename is already on grorg main (915a229); it gates the integration.

### multimatrix
- One signed listener (gp decision 13), designed in the port plan and reviewed before build: `--addr` / `--listen`,
  no owner listener, policy grants for grorg's service key and the lead and orchestrator keys.
- Merge `agent-room` into `room`; `room messages` one page shape (gp decision 12); the room context chooses its
  client by which globals are set.
- `message submit` with `--text` (gp decision 9), `--identity-dir` and `--addr` as globals with `globals: [..]`
  (DEC-4); the two-op split for `--signed-file` is the lead's choice.
- Rename client codes to end `.not_found` (`multimatrix.cli.action_room.not_found`), `dispatch.no_host` to
  `dispatch.no_place`; `--worker-dead-after-secs` to `--daemon-dead-after-secs` and its env var.
- `TopicStop` as an item variant, `stop_codes = [multimatrix.access.not_found]` (STR-2); `Position` frames as
  snapshots or dropped.
- `room-put`, `service-key-public` and `serve` stay extras (FB-1); check extra flags against globals (FB-3).
- Keep the policy grants file in snake_case (it is data, not an op); keep `policy sign`'s own 0644 atomic write.
- Re-pin `grorg-client` to >= 915a229 and adapt to Place in the same round as the port.
- `dm list` as a list pair; the newest-N use is the page half.

### gait
- `auth add --key-file` (required `SecretFile`, `#[op(prompt)]`); `auth login` split so the completing step takes
  `--callback-file` and uses `prompt_secret` for the paste on a terminal.
- `persist`/`discard` split into two structs with `id: Option<Ref>` (`id_only`), `all` switch, `validate =
  check_bulk`; `force` only on discard.
- `register` rules in one `validate`; `url/command` and `content/path` as `one_of_optional` groups.
- `TURN_FLAGS` rewritten as `FallbackFlag`s, with a test comparing them to `TurnArgs`; delete the hidden global
  copies and `usage_error_exit`/`report_code`.
- `local` ops: `--id` optional subject, `hide_default`; fold all blocks into one `ops!` with `#[cfg]`.
- Tag free-text fields `allow_hyphen` (or struct-level).
- Keep gait's own `manual` group (do not call `with_manual` in full gait).
- `GAIT_AGENT_DIR` to `GAIT_IDENTITY_DIR`, `daemons` to `processes` (addendum A1-A4), deployed with structmux and
  concordium.
- Optional: implement `Resolve` for `session` so lookups go through the resolver.

### structmux
- `structmuxd` main pre-dispatches the daemon path with `cli::select` (FB-5); drop "fallback runs the old code" from
  Task 6.
- `add-device` without `--confirm-fingerprint` is exit 2 (not a preview); new code
  `structmux.owner.fingerprint_mismatch`.
- `--code-file` holds the bare code (delete the `setup code:` prefix parse); callers write `jq -r .setupCode`.
- Port `structmux-daemon/tests/owner_tool.rs` and the trace/sentinel tests; client printed commands
  (`join_device.rs:154,716`, `fleet_state.rs:2946`, `approval_tests.rs:677`).
- `catalog_cli.rs` compares parsed `items`; `export-settings` keeps its own 0644 write.
- `jq` preflight in native place installs (concordium side, in the integration list).

### archegraph
- Hook payloads stay snake_case (gp decision 14).
- `structure --build` stays a plain `Option<String>`, ownership checked in the op.
- `graph-neighbors` is a plain op; `graph-search` gets a low `cap` (about 1000) with `page_size: 50`.
- Drop `ARCHEGRAPH_EXIT`; signal exits stay in the binary.
- MCP tool renames at the cut-over (`build` to `build_run`, `dag_status` to `build_status`, ...); skills, README and
  runtime strings that print commands change in one commit with the flags.
- `record_build_id` state through the DEC-6 factory, not a shared context.

### concordium-app
- Port Machine to Place (`list_places`, `fleet::Place`) against grorg main; move its typed bridge onto
  `call_stream_typed_owned` and `supervise_stream`; build op-listen without `registry` if it wants.

### Renames (vocabulary, not library)
`machine` to `place`, `worker` to `daemon`, `--host` to `--addr`/`--daemon` (glossary decision E). The library
renames nothing.

---

## 7. Version and breaking changes

All four crates go to 0.4.0 (`Cargo.toml` `workspace.package.version`, and the path-dependency versions). Breaking.

New dependencies: `rpassword` 7 (optional, feature `prompt`, default on); `tokio` with `time` under the `testing`
feature; `jsonschema` as a dev-dependency only; `serde_json`'s `preserve_order` feature.

### Breaking signatures and types
1. `Paths`: enum `{All, TypedOnly, CliOnly}` becomes a set with `Paths::ALL`, `Paths::of(&[Surface])`; new enum
   `Surface`; `OpSpec.cli_only_reason` and `Descriptor.cli_only_reason` removed; `ops!` key `reason:` removed;
   `paths: [cli]` now excludes `typed` (PATH-1).
2. `Registry::push_extra(command, fn(&Globals, &ArgMatches) -> BoxFuture<anyhow::Result<u8>>)` (FB-1).
3. `Registry::fallback_flags_only(name, help, fn(Vec<OsString>) -> BoxFuture<'static, anyhow::Result<u8>>)` (FB-1).
4. `Registry::fallback_free_text(name, help, &[FallbackFlag], near_miss: bool, fn(&Globals, Vec<OsString>) ->
   BoxFuture<anyhow::Result<u8>>)` (FB-1, FB-2).
5. `Registry::call_typed` returns `Result<O::Output, CallFailure>` (OUT-7).
6. `Registry::call_stream_typed` returns `Result<BoxStream<Result<O::Output, CallFailure>>, CallFailure>` (OUT-7).
7. `Registry::call_stream` returns `Result<BoxStream<Result<Value, CallFailure>>, CallFailure>` (OUT-7).
8. `FieldMeta` is `#[non_exhaustive]` with a constructor and new fields; hand-written `OpParams`/`OpGlobals` change
   (PAR-11).
9. `FieldKind` gains `Path` and `Patch`: `PathBuf` fields report `Path`, `Patch<T>` fields `Patch` (PAR-11).
10. `testing::SecretSentinel` is `#[non_exhaustive]` with `new` and `with_globals` (KIT-5); gait's
    `tests/auth_models.rs:144` and `tests/ops_registry.rs:323` change.
11. `cli::Io` is `#[non_exhaustive]` with two new fields; `Io::memory` keeps its signature (SEC-7).
12. `Redactor` is a shared handle: `Clone` shares, `add`/`extend` take `&self`, new `detached` (SEC-1).
13. `cli::Outcome::Err` gains `exit: u8` (exhaustive patterns break) (ERR-1).
14. `Exit` gains `on: Option<String>` (struct literals break) (ERR-1).
15. op-listen: `supervise(connect, opts, sink)` loses its `stop_codes` parameter, which moves to
    `FollowOptions::stop_codes`; `FollowOptions` stays `#[non_exhaustive]` (STR-1).
16. Struct-level `#[op(..)]` on an `OpParams` struct is parsed: unknown keys that were silently ignored now fail to
    compile (REL-7).

### Additive (no break)
`OpSpec` fields `more_audiences`, `needs_globals`, `error_exits`, `follow_page` (consumers use `..OpSpec::new()`);
`Descriptor` fields `audiences`, `needs_globals`, `relations` (non-exhaustive); `Registry::add_common_errors`,
`common_errors`, `set_error_mapper`, `call_stream_typed_owned`, `fallback_flags_only_before_runtime`; `cli::select`,
`Selected`, `FallbackFlag`, `Prompter`; `CallOptions::redact`, `prompt_secret`, `max_retries`, `with_max_retries`;
`CallFailure: CodedError`, `into_coded`; `Resolve::check_id`; `RunHooks::on_selected`; `OpParams::relations`;
`Listener::reconnect_marker`, `follow_options`; `follow_stream`, `Followed`, `listen_entry_following`,
`supervise_stream`, `Resumable for ListenFrame`; `ListenError::Stopped`; `StatusSource::remote_snapshot`,
`op_status::GROUP_SUMMARY`; `RegistryServer::with_ctx_factory`, `with_instructions`; `RenderText for Vec<T>`;
`UsageError::{Conflicts, MissingUnless, AtLeastOne}`; `testing::sample_params_seeded`; `usage()` documented.

### Behaviour changes consumers see
- Every array output is `{"items": [...]}` on CLI, `call`, `call_tool` and the manual (OUT-1).
- JSON keys print in declaration order (OUT-6).
- Relation schemas change shape (REL-6); `dependentRequired` is gone.
- MCP: non-object outputs lose `outputSchema` and return text (OUT-2); multi-audience membership (DEC-1).
- The kit runs CLI-only ops, tests `id_only` subjects for not_found, reports undecodable samples, bounds streams,
  checks exact paths and output roots (KIT-1..6): suites that passed vacuously turn red until fixtures are added.
- FreeText routing (FB-2); `room listen`-style followers exit 65 after a stop (STR-4); `supervise`/`follow_items`
  no longer deliver a stop item as an item (STR-1).
- Usage hook fires for op-side usage errors (REL-7); error-code exits (ERR-1).

---

## 8. Build order inside the library

Dependencies: REL-1 before REL-2..5; REL-6 with REL-1..5; REL-7 before DEC-4; PAR-11 before KIT-3/4/5 and
PAR-1/4/10; SEC-1 before SEC-2..5 and OUT-7; KIT-2 with KIT-1 and before STR-4; DEC-1 before OUT-2; OUT-1 with the
help.rs fix; ERR-2 before SEC-1's pipeline edit; OUT-7 before OUT-8; STR-1 and STR-3 before STR-4; OUT-6 last.

Suggested steps:
1. Silent and hanging bugs: KIT-1 + KIT-2 (together), OUT-5, DEC-5.
2. Types that change once: PAR-11, SEC-7 (`Io`), KIT-5's `SecretSentinel`, PATH-1.
3. Outputs: DEC-1, OUT-1 (with `$defs` hoist and help.rs), OUT-2, DEC-6.
4. Relations in one pass over `attrs.rs`/`clap_gen.rs`/`schema.rs`/`rt.rs`: REL-1, REL-6, REL-2, REL-3, REL-4, REL-5,
   REL-7.
5. Refs and field types: PAR-1, PAR-4, PAR-2, PAR-5, PAR-3, PAR-6, PAR-7, PAR-8, PAR-9, PAR-10.
6. Runner: FB-1, FB-2, FB-3, FB-4, FB-5, DEC-4, DEC-3, MAN-3.
7. Errors: DEC-2, ERR-1, ERR-2, ERR-3.
8. Secrets: SEC-1, SEC-2, SEC-3, SEC-4, SEC-5, SEC-6, SEC-8, SEC-9.
9. Typed: OUT-7, OUT-8. Lists: OUT-3, OUT-4.
10. Kit: KIT-3, KIT-4, KIT-6; help: MAN-6, MAN-7; manual: MAN-1, MAN-2, MAN-4, MAN-5.
11. op-listen: STR-1, STR-2, STR-3, STR-4, STR-5. op-status: STR-6.
12. Last: OUT-6 (re-render the manual snapshot once), DOC-1..3, CHANGELOG, version 0.4.0.

---

## Appendix: where each row and finding went

| source | change |
|---|---|
| row 1 | DEC-5 |
| row 2 | OUT-2 (array half in OUT-1) |
| row 3 | KIT-1 |
| row 4 | PAR-1, KIT-3 |
| row 5 | REL-2, KIT-4 |
| row 6 | REL-3 |
| row 7 | REL-1, KIT-4 |
| row 8 | PAR-2 |
| row 9 | FB-1 |
| row 10 | SEC-1, SEC-2, KIT-5 |
| row 11 | STR-4 |
| row 12 | REL-7 |
| row 13 | OUT-3 |
| row 14 | DEC-4 |
| row 15 | DEC-2, MAN-1 |
| row 16 | STR-6 |
| row 17 | DEC-1 |
| row 18 | ERR-1 |
| row 19 | ERR-2 |
| row 20 | DEC-6 |
| row 21 | replaced by PAR-4 and KIT-3 (gp decision 3) |
| row 22 | KIT-5 |
| row 23 | SEC-9 |
| row 24 | MAN-6 |
| row 25 | PAR-6 |
| row 26 | FB-2 |
| row 27 | DEC-3 |
| row 28 | PAR-7 |
| row 29 | OUT-8 |
| row 30 | STR-3 |
| row 31 | OUT-4 |
| row 32 | REL-4 |
| row 33 | STR-1 |
| row 34 | OUT-6 |
| row 35 | OUT-7 |
| row 36 | FB-4 |
| row 37 | OUT-5 |
| row 38 | MAN-7 |
| row 39 | PAR-8 |
| row 40 | PAR-3 |
| row 41 | PAR-9 |
| row 42 | PAR-5 |
| row 43 | MAN-2 |
| row 44 | FB-3 |
| row 45 | STR-5 |
| row 46 | STR-6 |
| row 47 | DOC-1 |
| row 48 | DOC-2 |
| row 49 | OUT-1 |
| row 50 | MAN-1 |
| row 51 | REL-5 |
| row 52 | MAN-3 |
| row 53 | SEC-6 |
| row 54 | PAR-11, KIT-4 |
| row 55 | KIT-4 |
| merged 3c | DOC-3 |
| second pass 1 | KIT-2, STR-4 |
| second pass 2 | OUT-1 |
| second pass 3 | SEC-4, SEC-5, SEC-7 |
| second pass 4 | integration list (grorg/multimatrix/app lockstep) |
| second pass 5 | SEC-1 |
| second pass 6 | PAR-10 |
| second pass 7, 8 | REL-6 |
| second pass 9 | PAR-4, KIT-3 |
| second pass 10 | PAR-6, KIT-4 |
| second pass 11 | KIT-5 |
| second pass 12 | DEC-1 |
| second pass 13 | ERR-1 |
| second pass 14 | OUT-1, OUT-2, KIT-6 |
| second pass 15 | REL-4 |
| second pass 16 | OUT-2 |
| second pass 17 | DEC-4 |
| second pass 18 | SEC-8 |
| second pass 19 | OUT-3 |
| second pass 20 | OUT-6 (no change) |
| second pass 21 | integration list |
| gait review F1, F2, F3, F6, F7, F9, F10, F13 | SEC-5, SEC-3, OUT-2, FB-2, FB-5, DEC-2, MAN-3, PAR-7 |
| gait check: env satisfies a global; who wraps Vec | DEC-4, OUT-1 |
| grorg review 1, 5, 6, 7, 8, 11, 14 | PAR-4/KIT-3, PAR-10, STR-4, MAN-1, DEC-1 (additive), DEC-5 (accepted), PAR-8 |
| multimatrix review 2, 6, 9, 13, 17 | STR-1/STR-2, DEC-4, FB-3, STR-4, OUT-6/STR-6 |
| multimatrix check: relations param-to-param; `Option<Input>`; listen_entry groups; PathBuf samples | REL intro, REL-1, PAR-1, KIT-4 |
| structmux check Q1-Q4; review 1 | REL-5, SEC-2, MAN-3, OUT-1; FB-5 |
| archegraph check/review: optional scope, oneOf root, plain `limit`, N3, N4, N5, findings 7, 8 | PAR-2, OUT-2, section 5, PAR-9, PAR-9, DOC-3, SEC-7 (`Io::memory` unchanged), DEC-1 |

## Orchestrator rulings on the design's open items (2026-11-20)
- (a) rpassword: an optional dependency, on by default, used for the no-echo prompt.
- (b) A param `env` applies on the CLI path only. On typed, call/call_tool and MCP the value must be passed. No panic and no build error for an env param on a JSON-reachable op.
- (c) `paths` is a literal subset: `[cli]` is CLI only. A consumer that calls the op in-process writes `[cli, typed]`.
- (d) A stopped follower exits 65 (Logical).
- (e) `room messages --all` is a stream (NDJSON, one message per line), like other follow outputs. It does not break the one-JSON-value rule for unary ops.
- (f) grorg `grorg.fleet.*` place codes become `grorg.place.*`, following gp's `<pillar>.<object>.<what>` rule.
- (g) The grorg hub DB SQL stays pending gp's fresh OK once desktop-grorg-place-step-history lands.

## Lead corrections (final check, 2026-11-20). These override the text above.
### multimatrix
- STR-2: a consumer frame type such as multimatrix's ListenManyFrame implements Resumable itself. `stop_code()` decides what is terminal: there, only a Stop coded `multimatrix.access.not_found`. Every other Stop is an item variant (as TopicStop is) and the stream continues. A Stop arrives as a frame, not an error; `stop_codes` covers only mid-stream errors. The library must never treat every Stop frame as terminal.
- Position frames are resume points (`cursor()` = Some) and stay a frame variant; they are never dropped, and never a snapshot. `ListenManyItem.topics[]` stays in the item type.
- Note for section 7: follow_stream retries a failed open as Transient (up to max_retries). Today's agent-room never retries an open; this is a deliberate improvement.
### gait
- Ruling: the not_found exemption marker is named `absent_ok`, not `new`: `#[op(subject = .., absent_ok)]`. It covers ops that accept an id that may not exist: creates and ensures (gp's "declared creates"), and idempotent ops that succeed on a missing id by design (gait `auth status` answers "not configured"; `auth remove/unlink` of a missing provider succeeds). The kit skips not_found for these only. Combining it with `id_only` is allowed: drop PAR-4's rejection, because id_only means "no resolver", which is a different axis. Every other subject is tested.
- Gait port notes, additions:
  - the create-type ops (auth.rs:105-135, InferenceAddParams, ModeAddParams, config set, register, local setup) take `absent_ok`;
  - so do auth login/logout (today `invalid_input` for an unknown provider) and auth status/remove/unlink;
  - the 16 real id_only lookups are kit-tested;
  - gait mounts no MCP, so gait declares its ops `paths: [cli]` (or `[cli, typed]` where called in-process), which keeps the SecretFile ops (`auth add --key-file`, `auth login --callback-file`) off JSON paths;
  - TURN_HELP (ops/turn.rs:48-49) uses `{bin}`;
  - replace `agent_dir_arg` (lib.rs:59-77) and `apply_agent_dir_flag` with `cli::select`/`cli::parse` before the runtime;
  - re-bless the goldens (seven `{items}` outputs, key order).
- SEC-8 fix: gait does have plain SecretFile ops (above); the sentence saying no consumer does is wrong.
- F2 and F3 need no library change: Redactor::add already registers trimmed lines (params/mod.rs:96-105), and a serde_json::Value output is the "any" class.
### grorg
- `agent key list` and `schedule clear` take `absent_ok` (an unknown agent gives `{}`; schedule clear is idempotent). No server change.
- Ruling (f) made precise: option (a). Each fleet object gets its own code area: `grorg.place.*`, `grorg.environment.*`, `grorg.harness.*`, `grorg.provider.*`, `grorg.model.*` (already_exists, edge_taken, name_taken, not_found, retired, retired_parent). The server passes the object into fleet errors. grorg sites: grorg-ops/src/error.rs:304, grorg-client/src/error_catalog.rs:55-60, registry/fleet.rs:40, grorg-ops/src/fleet.rs, fleet_ops.rs:160,165,229,361,430,507,600, fleet_verbs.rs:440,452,469, grorg-mcp tests fleet_tools.rs:180,298 and fleet_verbs_tools.rs:433,452,497. Deploys with grorg-server and multimatrix (Place lockstep).
- PAR-10 text is superseded by ruling (b): no panic and no build error for an env param on a JSON or MCP path. The env applies on the CLI only. Drop the test "a registry that puts such an op on mcp panics" and replace it with "env is ignored off the CLI; the value must be passed".
- grorg compile breaks: `Paths::All` at crates/grorg-client/tests/testing_kit.rs:52 (Paths becomes a set), and tests that compare printed JSON text (OUT-6). Audience is additive (DEC-1), so OpSpec literals, `with_manual(.., Audience::Agent)` and `Outcome::Err { error, .. }` still compile.
