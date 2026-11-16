# op-listen

## Overview

op-listen is a shared listener for op-registry. It gives an operation a
streaming form that clients can resume from a cursor. The crate provides:

- a cursor (`<epoch>:<seq>`) that a client sends back to resume;
- frames (snapshot, item, stop) that carry the cursor;
- a per-app gate that authorizes a caller, applies a not-before floor, and
  rechecks access while a stream is open;
- a reconnecting follower that prints frames and retries after transient failures.

Licensed under MIT OR Apache-2.0.

## Cursor

A cursor has the form `<epoch>:<seq>`. Both fields are decimal `u64` values
separated by one colon. Spaces, signs and extra fields are rejected.

- `epoch` identifies the source. For a state-like listener it is the process
  boot, so a restart starts a new epoch. For a durable log it is a constant.
- `seq` orders items within an epoch.

Cursors order by epoch first, then by seq. A cursor that does not parse is
reported with the code `op_listen.bad_cursor`.

## Frames

A listen stream emits JSON objects tagged by `frame`.

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

A source reports overflow by yielding an error with the code `op_listen.lagged`
(build it with `op_listen::lagged`). The helper does not buffer items or spawn
tasks, so the source itself must report overflow.

What happens next depends on the listener's `overflow` policy:

- `End` (the default): the lagged error is yielded and the stream ends.
- `Resnapshot`: the helper calls `open` again with no cursor. If that returns a
  snapshot, a snapshot frame with `resync: true` is sent and the stream
  continues. If it returns a replay, `op_listen.resnapshot_failed` is yielded.
  If it errors, that error is yielded.

## Follower

`follow(connect, sink, options)` connects, prints each frame as one compact
JSON line through `sink`, and reconnects when the connection fails.

`FollowOptions` defaults:

- `initial_cursor`: `None`
- `backoff`: `Backoff::follower()`, a 250 ms delay doubling after each failure up to 30 s
- `max_retries`: `None` (retry forever)

The backoff and the retry budget reset after a connection delivers at least one
frame, or after a connection that delivered nothing lived past the backoff's
healthy period (`Backoff::with_healthy_after`). A connection that fails before
that period does not reset them. The follower tracks the cursor from every frame
that has one, and reconnects with the last cursor.

Before the first frame of each reconnection, the follower prints a marker line:

```json
{"marker":"reconnected","cursor":"0:2","resync":false}
```

`cursor` is the cursor passed to `connect`, or `null`. `resync` is true only when
the first frame is a snapshot with `resync: true`.

When a stop frame arrives, the follower prints it, then prints
`{"marker":"stopped","code":"..."}` and returns `FollowEnd::Stopped`. It does
not reconnect.

Logical errors are returned at once, with no reconnect and no marker line.
Other errors and clean ends count as failed attempts. With `max_retries` set to
`n`, the follower retries up to `n` times (`n + 1` connection attempts) and then returns the
last error. A clean end counts as `op_listen.ended`. A delivered frame resets
the failure count.

## Who gates what

An app whose operations run in a client process returns `Grant::default()` from
`authorize`. Access is then decided by the server that the client talks to.

An app that serves its listeners in process does its gating in `authorize` and
`still_allowed`.

## Notes

- `Opened::Snapshot` carries `seq`, the sequence the snapshot reflects. The
  snapshot frame's cursor is `epoch:seq`.
- `Listener::Params` must implement `Serialize` and `Clone`. Serialize is used
  to build the CLI hooks, and Clone is used to reopen the source on resnapshot.
- `listen_entry` registers the listener with `push_stream_borrowed`. It leaks one
  `Arc` reference per registered listener, so register each listener once.
- A listen entry is CLI-only. It does not appear in the MCP tool list.

## Example

```rust,ignore
use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream};
use futures_util::FutureExt;
use op_listen::{listen_entry, Grant, ListenCursor, Listener, Opened};
use op_registry::{Caller, Descriptor, OpParams, Registry};
use std::sync::Arc;

struct Ctx;

#[derive(Clone, serde::Serialize, serde::Deserialize, OpParams)]
struct TopicParams {
    #[op(positional)]
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
listen_entry(
    &mut registry,
    Arc::new(Notes),
    Descriptor::new("notes", "listen"),
);
```
