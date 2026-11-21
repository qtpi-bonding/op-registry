# op-listen

## Overview

op-listen is a shared listener for op-registry. It gives an operation a
streaming form that clients can resume from a cursor. The crate provides:

- a cursor (`<epoch>:<seq>`) that a client sends back to resume;
- frames (snapshot, item, stop) that carry the cursor;
- a per-app gate that authorizes a caller, applies a not-before floor, and
  rechecks access while a stream is open;
- a reconnecting follower that retries after transient failures and either
  prints frames as JSON lines (`follow`), hands decoded frames to a callback
  (`follow_typed`), or is itself a stream (`follow_stream`), so a following
  command can be a stream op (`listen_entry_following`).

Licensed under MIT OR Apache-2.0.

## Cursor

A cursor has the form `<epoch>:<seq>`. Both fields are decimal `u64` values
separated by one colon. Spaces, signs and extra fields are rejected.

- `epoch` identifies the source. For a state-like listener it is the process
  boot, so a restart starts a new epoch. For a durable log it is a constant.
- `seq` orders items within an epoch.

Cursors order by epoch first, then by seq. A cursor that does not parse is
reported with the code `op_listen.bad_cursor` (Logical).

`ListenCursor` is a typed value: `ListenCursor::new(epoch, seq)`, `Display`,
and `FromStr` (the error is an `anyhow::Error` carrying that code). It
serialises as the JSON string `"epoch:seq"`. `ListenFrame` carries it in typed
form (`cursor: ListenCursor`), but the wire format and the JSON schema are
unchanged. `ListenFrame::cursor()` returns the cursor of a snapshot or item
frame, and `None` for a stop frame. `listen_stream(listener, ctx, params,
cursor, caller)` no longer takes a `ListenOptions` argument; the type is gone.

## Frames

A listen stream emits JSON objects tagged by `frame`. Field names are camelCase
(`ListenFrame` sets `rename_all_fields = "camelCase"`); today every frame field
is one word, so the wire is the same as before 0.3.0. The `frame` values stay
`snapshot`, `item` and `stop`.

Snapshot frame:

```json
{"frame":"snapshot","snapshot":{"a":1},"resync":false,"cursor":"0:3"}
```

Item frame:

```json
{"frame":"item","item":"x","cursor":"0:4"}
```

Stop frame:

```json
{"frame":"stop","code":"op_listen.revoked","message":"revoked"}
```

A snapshot frame sets `resync` to true when the consumer must discard its
state and rebuild it from the snapshot.

## The gate

Before any frame is sent, the listener's `authorize` method runs once with the
caller and the topic. It returns a `Grant`:

- `not_before`: the earliest cursor the caller may receive. A requested cursor
  older than this floor is raised to the floor. A newer requested cursor is
  kept. Replayed items at or below the floor are never sent.
- `recheck_every`: when set, `still_allowed` runs at that interval while the
  stream is open. Checks do not overlap, and frames keep flowing while a check
  is pending. When `still_allowed` returns false, the stream sends one stop
  frame and ends. The stop frame's code and message come from the listener's
  `revoked_error`. When `recheck_every` is not set, `still_allowed` is never
  called.

If `authorize` fails, the error is returned directly. No stream is created and
`open` is not called.

## Overflow

A source reports overflow by yielding the typed `ListenError::Lagged` error,
built with the `lagged(skipped)` constructor (`op_listen::lagged`). The helper
does not buffer items or spawn tasks, so the source itself must report overflow.

What happens next depends on the listener's `overflow` policy:

- `End` (the default): the lagged error is yielded and the stream ends.
- `Resnapshot`: the helper calls `open` again with no cursor. If that returns a
  snapshot, a snapshot frame with `resync: true` is sent and the stream
  continues. If it returns a replay, `op_listen.resnapshot_failed` is yielded.
  If it errors, that error is yielded.

## Follower

`follow(connect, sink, options)` connects, writes each frame as one compact
JSON line through `sink`, and reconnects when the connection fails. The lines
are rendered from the frame's original JSON value, so the output is
byte-identical to what the source sent (plus the marker lines below).
`follow_typed(connect, sink, options)` is the same loop, but `sink` receives
typed `FollowEvent<S, I>` values instead of lines.

`FollowOptions` is `#[non_exhaustive]`: start from `FollowOptions::default()`
and assign fields. Defaults:

- `initial_cursor`: `None` (`Option<ListenCursor>`)
- `backoff`: `Backoff::follower()`
- `max_retries`: `None` (retry forever)

### Backoff

`Backoff` is a public, jitter-free, capped exponential delay shared by the
follower and by consumers. It replaces the old `backoff_first` and
`backoff_max` options.

| constructor | sequence |
|---|---|
| `Backoff::new(first, cap)` | `first`, doubling, capped at `cap`; `first` is clamped to `cap` |
| `Backoff::follower()` | 250ms, 500ms, 1s, 2s, 4s, 8s, 16s, 30s, 30s, ... |
| `Backoff::link()` | 1s, 2s, 4s, 8s, 16s, 30s, 30s, ... |

`next_delay()` returns the current delay and advances; `reset()` restarts the
sequence. `with_healthy_after(d)` sets the healthy period (default: the cap),
and `connection_ended(lived)` resets the sequence when `lived >= healthy_after`
and returns whether it did.

The follower resets the backoff and the retry budget after a connection that
delivered at least one frame, or that delivered nothing but lived for the
healthy period. A connection that fails sooner does not reset them. The
follower tracks the cursor from every frame that has one, and reconnects with
the last cursor.

### Events and markers

Each connection after a failure announces itself first. In the JSON follower
this is a marker line before the first frame of the reconnection:

```json
{"marker":"reconnected","cursor":"0:2","resync":false}
```

`cursor` is the cursor passed to `connect`, or `null`. `resync` is true only
when the first frame is a snapshot with `resync: true`.

When a stop frame arrives, the follower writes it, then
`{"marker":"stopped","code":"..."}`, and returns `FollowEnd::Stopped`. It does
not reconnect.

`follow_typed` delivers the same sequence as `FollowEvent` values, in the same
order: `Reconnected { cursor, resync }`, then `Frame(ListenFrame<S, I>)` for
each frame, and `Stopped { code }` after the stop frame. A frame that does not
decode as `ListenFrame<S, I>` ends the follow with the Logical error
`op_listen.bad_frame` (`ListenError::BadFrame`) before the sink sees it; it is
never retried.

### Errors and ending

Logical errors are returned at once, with no reconnect and no marker line.
Other errors and clean ends count as failed attempts. With `max_retries` set to
`n`, the follower retries up to `n` times (`n + 1` connection attempts) and then
returns the last error. A clean end counts as `op_listen.ended`. A delivered
frame resets the failure count.

`FollowEnd` is `#[non_exhaustive]`, so match it with a `_` arm. Its variants
are `Stopped { code }` and `SinkClosed`. The sink returns `std::io::Result<()>`;
a `BrokenPipe` error from the sink (for example `| head`) ends the follow with
`Ok(FollowEnd::SinkClosed)` and no reconnect. Any other sink error is
returned. A broken pipe raised by the stream itself is an ordinary failure, not
`SinkClosed`.

### Supervision

`supervise` follows `StreamStep<T>` values and sends structured `Supervised<T>`
events to its sink. `supervise_stream` is the same over a plain stream of items
(the end of the stream is `op_listen.ended`, retried). Both report the fieldless
`Opened` event as soon as `connect` succeeds, before any item, `Retrying { code }`
before a retry, and `Stopped { code }` when the follow ends on a stop. A stop item
is delivered only as `Stopped`, never as an `Item` first (`follow` and
`follow_typed` still write the stop frame, then the stopped marker).
`Ended` and `SessionExpired` are transient and use `op_listen.ended` and
`op_listen.session_expired`.

`FollowOptions::stop_codes` lists error codes that end any follower as
`Stopped { code }` when they arrive as a failed open or a mid-stream error, before
the Logical check. Without a listed code, a Logical error ends the follow with
`Err` and any other class retries. A stop that arrives as a frame is never decided
by this list.

### What is terminal

Only an item whose `Resumable::stop_code()` is `Some` ends a follow. The library
never decides this from a frame's shape for a consumer type. The `ListenFrame`
impl maps only `ListenFrame::Stop`. A per-topic stop is data: the consumer carries
it as an item variant whose `stop_code()` is `None` and the follow continues. A
frame whose `cursor()` is `Some` and `stop_code()` is `None` (a position frame)
is delivered and advances the resume cursor.

### A follower as a stream

`follow_stream(connect, options, marker)` is the same reconnect loop as a
`Stream` of `Followed<T>` values (`Frame(item)` or `Marker(map)`), so it can be
the body of a stream op. It shares the rules above, plus:

- A failed open is a failure like a connection that ends: it is retried while
  its error is not Logical, up to `max_retries` (behaviour change from the
  0.3 followers that never retried an open).
- The marker is yielded before the first item that follows a failed first open
  or any earlier connection. A connection that yields nothing gets no marker.
  An empty connection that lived for `backoff.healthy_after` resets the failure
  count, so such connections never add up toward `max_retries`.
- A stop item is yielded once, then the stream ends with `op_listen.stopped`
  (Logical, exit 65 on the CLI). An error whose code is in `stop_codes`, mid-stream
  or on open, ends it the same way with no frame, and `Stopped { code }` names it.
- The source must yield. The loop runs as one future that fills an unbounded
  queue, so an always-ready source is drained in one poll.

`listen_entry_following(registry, listener, spec)` registers a listener as a
reconnecting stream op, over `OpSpec<Params, Followed<ListenFrame<S, I>>>`. It
passes params and `--cursor` through, honours `OverflowPolicy::Resnapshot`, takes
`max_retries` from `CallOptions::max_retries` when set (else from
`Listener::follow_options()`), and ends with `op_listen.stopped` after a stop.
Two defaulted `Listener` methods shape it:

- `reconnect_marker(cursor, resync)` builds the marker line. The default is
  `{"marker":"reconnected","cursor":<c or null>,"resync":<b>}`. Override it to
  keep a consumer's own shape. grorg's `events watch` keeps
  `{"marker":"reconnected","afterSeq":N}`:

  ```rust,ignore
  fn reconnect_marker(&self, cursor: Option<&ListenCursor>, _resync: bool)
      -> serde_json::Map<String, serde_json::Value> {
      let mut m = serde_json::Map::new();
      m.insert("marker".into(), "reconnected".into());
      m.insert("afterSeq".into(), cursor.map(|c| c.seq).into());
      m
  }
  ```
- `follow_options()` gives the backoff, initial cursor and `stop_codes`.

### Printing to stdout

`stdout_sink()` returns a sink that writes each line to stdout and flushes it.
`follow_to_stdout(connect, options)` is `follow` with that sink, so a CLI
watch command needs no sink of its own.

## Who gates what

An app whose operations run in a client process returns `Grant::default()` from
`authorize`. Access is then decided by the server that the client talks to.

An app that serves its listeners in process does its gating in `authorize` and
`still_allowed`.

## Features

- `detach` (default): `detached_connect`, which runs a streaming operation on
  its own tokio task so it outlives the caller's connection future. Needs
  tokio's `rt` feature, which only this feature enables.
- `testkit` (implies `detach`): `op_listen::testkit` with `Followed` (a follow
  running on a background task, recording its JSON lines, with `wait_until`,
  `join`, `abort`, `assert_still_following`, and `reconnect_markers`; `spawn`
  takes the follow options to use, and dropping aborts the task) and
  `short_follow_options()`. Meant as a dev-dependency feature.

- `registry` (default, implied by `detach`): the op-registry and schemars
  dependencies and everything built on them: `listen_entry`,
  `listen_entry_following`, `Listener`, `listen_stream`, `detach`, the `Caller`
  re-export, and the `JsonSchema` and `op_registry::Cursor` conversions of the
  cursor and frame types. Without it the crate keeps the core: `follow`,
  `follow_typed`, `follow_items`, `supervise`, `supervise_stream`, `follow_stream`,
  `Backoff`, the cursor, frame and error types and `stdout_sink`. A follower-only
  client builds with `default-features = false`.

Without default features the crate has no `detach` and no tokio `rt`.

## Notes

- `Opened::Snapshot` carries `seq`, the sequence the snapshot reflects. The
  snapshot frame's cursor is `epoch:seq`.
- `Listener::Params` must implement `OpParams`, `Serialize` and `Clone`. Clone
  is used to reopen the source on resnapshot.
- `listen_entry(&mut registry, listener, spec)` takes an
  `OpSpec<Params, ListenFrame<Snapshot, Item>>` and registers the listener as a
  stream op (`spec.cursor` is set for you). It leaks one
  `Arc` reference per registered listener, so register each listener once.
- A listen entry is a streaming op and takes `--cursor`. Streams are not in the
  MCP tool list.
- On the CLI a listen op needs a caller: `CliContext::caller()` must return
  `Some` (it returns `None` by default). Without one the op fails with
  `op_listen.no_caller`. The testing kit passes its own caller.
- `op_status::status_registry` needs a `StatusSource` for the context type; see
  the op-status README.

## Example

This mirrors `op-listen/tests/registry.rs`.

```rust,ignore
use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream};
use futures_util::{FutureExt, StreamExt};
use op_listen::{listen_entry, Grant, ListenCursor, Listener, Opened};
use op_registry::{Caller, OpParams, OpSpec, Registry};
use std::sync::Arc;

struct Ctx;
impl op_registry::Resolve for Ctx {}

#[derive(Clone, serde::Serialize, serde::Deserialize, OpParams)]
struct TopicParams {
    /// Topic to follow.
    topic: String,
}

struct Notes;

impl Listener<Ctx> for Notes {
    type Params = TopicParams;
    type Snapshot = ();
    type Item = String;

    fn topic(&self, params: &TopicParams) -> String {
        params.topic.clone()
    }

    fn epoch(&self) -> u64 {
        0
    }

    fn authorize<'a>(
        &'a self,
        _ctx: &'a Ctx,
        _caller: &'a Caller,
        _topic: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<Grant>> {
        async { Ok(Grant::default()) }.boxed()
    }

    fn open<'a>(
        &'a self,
        _ctx: &'a Ctx,
        _params: TopicParams,
        _cursor: Option<ListenCursor>,
        _floor: Option<ListenCursor>,
    ) -> BoxFuture<'a, anyhow::Result<Opened<'a, (), String>>> {
        async {
            let items: BoxStream<'a, anyhow::Result<(u64, String)>> =
                stream::iter(vec![Ok((1, "hello".to_owned()))]).boxed();
            Ok(Opened::replay(items))
        }
        .boxed()
    }
}

let mut registry: Registry<Ctx> = Registry::new();
registry.group("notes", "Notes.");
listen_entry(
    &mut registry,
    Arc::new(Notes),
    OpSpec::new("notes listen", "Follow a topic"),
);
```
