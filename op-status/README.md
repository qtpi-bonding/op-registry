# op-status

Shared status types, the `StatusSource` trait, attribute checks, and a `status get`
operation for op-registry. MIT OR Apache-2.0.

This crate is documentation-first: it defines the wire contract that services
use to report component health. It is read-only. A service exposes a
`StatusSource`; the crate turns that into a `status get` operation.

## Features

- `default = []`: types, `StatusSource`, `check` and `open_status`.
- `registry`: enables `ops` (`status_registry`, `StatusOptions`, `GetParams`).
  Pulls in `op-registry`, `clap` and `schemars`.
- `listen`: enables `status watch` (implies `registry`).

op-status now always depends on `op-listen` (without its default features), and
so on `op-registry`, tokio and the rest of op-listen's tree, even with default
features. `open_status` takes the cursor as `Option<op_listen::ListenCursor>`;
the old `StatusCursorRef` is gone. Parse a client string with
`text.parse::<ListenCursor>()` and render one with
`ListenCursor::new(epoch, seq).to_string()`.

## Contract summary

### Snapshot

`StatusSnapshot` is the output of `status get`, with fields in this order:

| field               | type               |
|---------------------|--------------------|
| `service`           | string             |
| `version`           | string             |
| `takenAtUnixMs`      | i64                |
| `epoch`             | u64                |
| `components`        | array of Component |
| `truncated`         | bool               |

JSON output follows declaration order. Keys are camelCase on the wire (the
Rust fields stay snake_case); state values keep their `SCREAMING_SNAKE_CASE`
spelling. This changed in 0.3.0: consumers must read the new keys. `truncated` is `true` when more
components matched than were returned.

### Component

`Component` fields, in this exact order (always all serialised; `None` becomes `null`):

1. `kind`: string
2. `id`: string, unique within its kind
3. `parent`: `{kind, id}` or `null`
4. `state`: State (see below)
5. `ready`: bool
6. `terminal`: bool
7. `lastErrorCode`: string, empty when none
8. `lastErrorMessage`: string, empty when none
9. `consecutiveFailures`: u32 or `null`
10. `changedAtUnixMs`: i64
11. `attrs`: map of allow-listed scalar attributes

There is no duration-since-failure field.

### States

`UNSPECIFIED`, `UNKNOWN`, `STARTING`, `OK`, `DEGRADED`, `PAUSED`, `DOWN`, `STOPPED`.

### Attributes

Attribute values are scalars: string, integer (i64), float (f64) or bool.
Numbers stay numbers in JSON. Limits:

- at most 32 attributes per component;
- keys at most 64 bytes;
- string values at most 256 bytes;
- every key must appear in the source's `allowed_attr_keys()`.

## Privacy

Status output must never contain:

- message bodies,
- prompts,
- room ids,
- transcript or node output,
- keys or tokens,
- policy docs,
- file paths or session metadata.

Privacy is by allow-list only. A service decides what may be reported by listing
attribute keys in `allowed_attr_keys`; anything else is rejected or dropped. There
is no deny-list fallback.

## Implementing a source

Implement `StatusSource`:

- `service()`: name and version of the service.
- `epoch()`: changes when the source restarts or resets.
- `now_unix_ms()`: optional; defaults to the system clock.
- `snapshot(filter, max)`: return matching components, applying the filter
  yourself, and return **at most `max + 1`** components so the caller can
  detect truncation.
- `allowed_attr_keys()`: the static allow-list of attribute keys.
- `remote_snapshot(filter, max)`: optional; defaults to `None`. A source that
  proxies another process (the owner CLI asking the daemon) returns
  `Some(future)` with the remote's `StatusSnapshot`, and `status get` and
  `take_snapshot` return it unchanged instead of calling `snapshot`.
- `watch(from_seq)`: optional; returns the changes with seq above `from_seq`
  (`None` = live changes only), replaying the retained backlog first. Defaults
  to `None`. A source that implements it also overrides `provides_watch()` to
  return `true` and `seq_window()` to report `SeqWindow { oldest, newest }`: a
  cursor seq is retained iff `oldest <= seq <= newest`.

Your context type must implement `op_registry::Resolve` (an empty impl is
enough) because the registry is generic over it. Then register the operation
and merge it into the app registry:

```rust,ignore
use op_status::ops::{status_registry, StatusOptions};

let mut app: Registry<MyService> = build_app_registry();
app.merge(status_registry::<MyService>(
    StatusOptions::new().max_components(500),
));
```

`status_registry` registers the group `status` and `status get` (reads access,
unary). `StatusOptions` defaults to `Audience::Agent` and
`max_components` 1000.

The group's summary is the exported constant `op_status::GROUP_SUMMARY`
(`"Component status."`). An app that declares its own `status` group, for
example in its `ops!` header, uses that constant as the summary so that
`Registry::merge` does not panic on two different summaries.

## `status get`

Parameters: `--kind`, `--id`, `--limit`.

- `limit` is the requested value, or `max_components` if absent, clamped to `max_components`.
- The source is asked for `limit`; rows beyond `limit` are dropped.
- `truncated` is `true` exactly when the source returned more than `limit` rows.
  A result is never partial without `truncated: true`.

## Attribute checks: debug vs release

Every returned component is checked against `check_component`.

- **Debug builds** (`debug_assertions`): a violation fails the whole call with a
  coded error (for example `op_status.attr_not_allowed`, class Logical). The
  error names the offending key.
- **Release builds**: offending attributes are removed and never passed on.
  Dropped: any key not on the allow-list, any key over 64 bytes, and any string
  value over 256 bytes. If more than 32 attributes remain, the first 32 in key
  order are kept.

Do not rely on release behaviour for correctness; fix the source so that debug
builds pass.

## `status watch` (feature `listen`)

With the `listen` feature (off by default)
`status_registry` also registers `status watch` when `StatusOptions::watch` is
on (the default) and the source returns `true` from `provides_watch()`. It is a
streaming entry on the default paths (never an MCP tool, as for every stream), `reads` access, with the
audience from the options. Parameters are those of `status get`.

The stream is an `op-listen` stream: it shares the `epoch:seq` cursor, the
frame envelope, the follower and the gate. The first frame is a full filtered
snapshot (the same path as `status get`, including `truncated`), then one item
frame per changed component. Change components pass the same attribute and
length checks as `status get`.

`watch` is **not an exact resume**. The snapshot's `resync` flag tells the
client what to do with it:

- `resync: true`: no cursor, a different epoch, or a seq the source no longer
  retains. Discard local state and rebuild it from the snapshot.
- `resync: false`: epoch matches and the seq is retained. The snapshot is a
  refresh; keep local state and apply the items that follow.

The change buffer is bounded (`WATCH_BUFFER`, 256). On overflow the stream
fails with `op_status.lagged` (Transient), which `StatusListener` maps to
`op_listen.lagged`; op-listen then opens again and sends a fresh snapshot with
`resync: true`. For exact resume use an events log.

`StatusListener::authorize` grants everyone (no floor, no recheck): who can
call is decided by the app's reachability and `StatusOptions::audience`. An app
that needs a gate wraps `StatusListener` or passes its own `Listener`.
`Listener::epoch` has no ctx, so `StatusListener` reports the epoch of the
source seen by its latest `open`; the epoch is one shared cell, so if a source's epoch changes while streams are live, older streams' later frames carry the new epoch (epochs change on restart in practice, which ends all streams). A proper fix needs `op-listen`'s `Listener::epoch` to take the ctx. `listen_entry` leaks one listener per
registration; build the registry once.

### Supplying `watch` from a service

Implement `watch`, `seq_window` and `provides_watch` on your `StatusSource`.
`watch(Some(n))` must yield exactly the retained changes with seq above `n`
then live ones, in strictly increasing seq. grorg's server can implement its
`WatchStatus` rpc by calling `op_status::open_status` directly (no `status watch` entry
needed), so the resync rules exist once. `open_status` reads
`seq_window().newest`, takes the snapshot, then subscribes with
`watch(Some(newest))`: a change emitted in between appears exactly once, in the
stream.

## Testing sources

`check::assert_allowed_keys(source_keys, observed)` panics, naming the key, when
an observed attribute key is not on the allow-list. Use it in source tests.

## Free-text fields

`kind`, `id`, `lastErrorCode`, `lastErrorMessage` (and the snapshot's
`service` and `version`) are bounded to 256 bytes too. Debug builds refuse a
longer value with `op_status.field_too_long` (Logical, naming the field by its
Rust name, e.g. `last_error_message`);
release builds truncate at a char boundary. A source must still keep message
bodies, paths and other forbidden content out of them.
