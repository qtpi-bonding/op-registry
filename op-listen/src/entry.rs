//! Registers a listener as a streaming registry operation.

use crate::{ListenCursor, ListenFrame, Listener};
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
    spec: OpSpec<L::Params, ListenFrame<L::Snapshot, L::Item>>,
) where
    C: Sync + Send + 'static,
    L: Listener<C>,
{
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
