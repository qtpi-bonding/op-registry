//! Registers a listener as a streaming registry operation.

use crate::{follow_stream, Followed, ListenCursor, ListenFrame, Listener};
use futures_util::{FutureExt, StreamExt};
use op_registry::{CallOptions, Cursor, OpSpec, Registry};
use std::sync::Arc;

/// Registers `listener` in `registry` as a streaming operation.
///
/// The output type is [`ListenFrame`] for this listener, so the descriptor's
/// output schema is the frame schema. The operation is not exposed through the
/// MCP tool list.
///
/// A listen stream borrows its listener for as long as the stream lives, while
/// a registry entry only lends the stream the context. To make that borrow
/// sound without unsafe code, one strong reference to `listener` is leaked
/// (`Box::leak`) when the entry is registered; the listener therefore lives for
/// the rest of the process. Register each listener once.
pub fn listen_entry<C, L>(
    registry: &mut Registry<C>,
    listener: Arc<L>,
    mut spec: OpSpec<L::Params, ListenFrame<L::Snapshot, L::Item>>,
) where
    C: op_registry::Resolve + 'static,
    L: Listener<C>,
{
    // The entry resumes from a cursor, so the leaf takes `--cursor`.
    spec.cursor = true;
    let leaked: &'static Arc<L> = Box::leak(Box::new(listener));
    let listener: &'static L = leaked.as_ref();
    registry.push_stream::<L::Params, ListenFrame<L::Snapshot, L::Item>, _>(
        spec,
        move |ctx: &C, params: L::Params, cursor: Option<Cursor>, opts: CallOptions| {
            async move {
                let Some(caller) = opts.caller else {
                    return Err(coded_error::into_anyhow(crate::ListenError::NoCaller));
                };
                let cursor = match cursor {
                    Some(cursor) => Some(ListenCursor::try_from(&cursor)?),
                    None => None,
                };
                let frames =
                    crate::run::listen_stream(listener, ctx, params, cursor, caller).await?;
                Ok(frames.boxed())
            }
            .boxed()
        },
    );
}

/// Registers `listener` in `registry` as a reconnecting streaming operation.
///
/// Like [`listen_entry`], but the op follows: it reconnects with the last
/// cursor, prints [`Listener::reconnect_marker`] before the first frame of
/// each reconnection, and ends with `op_listen.stopped` (Logical, exit 65)
/// after a stop frame. Params and `--cursor` pass through unchanged.
/// `max_retries` is the call's [`CallOptions::max_retries`] when set, else
/// that of [`Listener::follow_options`]. The same leak as in [`listen_entry`]
/// applies: register each listener once.
#[allow(clippy::type_complexity)]
pub fn listen_entry_following<C, L>(
    registry: &mut Registry<C>,
    listener: Arc<L>,
    mut spec: OpSpec<L::Params, Followed<ListenFrame<L::Snapshot, L::Item>>>,
) where
    C: op_registry::Resolve + 'static,
    L: Listener<C>,
{
    spec.cursor = true;
    let leaked: &'static Arc<L> = Box::leak(Box::new(listener));
    let listener: &'static L = leaked.as_ref();
    registry.push_stream::<L::Params, Followed<ListenFrame<L::Snapshot, L::Item>>, _>(
        spec,
        move |ctx: &C, params: L::Params, cursor: Option<Cursor>, opts: CallOptions| {
            async move {
                let Some(caller) = opts.caller.clone() else {
                    return Err(coded_error::into_anyhow(crate::ListenError::NoCaller));
                };
                let cursor = match cursor {
                    Some(cursor) => Some(ListenCursor::try_from(&cursor)?),
                    None => None,
                };
                let mut follow_opts = listener.follow_options();
                if opts.max_retries.is_some() {
                    follow_opts.max_retries = opts.max_retries;
                }
                if cursor.is_some() {
                    follow_opts.initial_cursor = cursor;
                }
                let connect = move |cursor: Option<ListenCursor>| {
                    let params = params.clone();
                    let caller = caller.clone();
                    async move {
                        let frames =
                            crate::run::listen_stream(listener, ctx, params, cursor, caller)
                                .await?;
                        Ok(frames.boxed())
                    }
                    .boxed()
                };
                Ok(follow_stream(
                    connect,
                    follow_opts,
                    move |cursor, resync| listener.reconnect_marker(cursor, resync),
                ))
            }
            .boxed()
        },
    );
}
