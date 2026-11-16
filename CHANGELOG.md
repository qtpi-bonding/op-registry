# Changelog

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
  `next_delay`, `reset`, `is_reset`, `connection_ended`), no jitter. The
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
