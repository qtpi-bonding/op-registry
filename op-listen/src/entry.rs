//! Registers a listener as a streaming registry operation.

use crate::{ListenCursor, ListenFrame, ListenOptions, Listener};
use futures_util::{FutureExt, StreamExt};
use op_registry::{CallOptions, CliHooks, Cursor, Descriptor, Registry};
use std::sync::Arc;

/// Registers `listener` in `registry` as a CLI-only streaming operation.
///
/// The descriptor's `streaming` flag is forced on and its output schema is set
/// to the schema of [`ListenFrame`] for this listener. The operation is not
/// exposed through the MCP tool list.
///
/// A listen stream borrows its listener for as long as the stream lives, while
/// a registry entry only lends the stream the context. To make that borrow
/// sound without unsafe code, one strong reference to `listener` is leaked
/// (`Box::leak`) when the entry is registered; the listener therefore lives for
/// the rest of the process. Register each listener once.
pub fn listen_entry<C, L>(registry: &mut Registry<C>, listener: Arc<L>, mut descriptor: Descriptor)
where
    C: Sync + Send + 'static,
    L: Listener<C>,
{
    descriptor.streaming = true;
    descriptor.output_schema =
        serde_json::to_value(schemars::schema_for!(ListenFrame<L::Snapshot, L::Item>))
            .unwrap_or(serde_json::Value::Null);
    let leaked: &'static Arc<L> = Box::leak(Box::new(listener));
    let listener: &'static L = leaked.as_ref();
    registry.push_stream_borrowed::<L::Params, serde_json::Value, _>(
        descriptor,
        move |ctx: &C, params: L::Params, cursor: Option<Cursor>, opts: CallOptions| {
            async move {
                let Some(caller) = opts.caller else {
                    return Err(coded_error::into_anyhow(crate::ListenError::NoCaller));
                };
                let cursor = match cursor {
                    Some(cursor) => Some(ListenCursor::try_from(&cursor)?),
                    None => None,
                };
                let frames = crate::run::listen_stream(
                    listener,
                    ctx,
                    params,
                    cursor,
                    caller,
                    ListenOptions::default(),
                )
                .await?;
                Ok(frames
                    .map(|frame| frame.and_then(|f| serde_json::to_value(f).map_err(Into::into)))
                    .boxed())
            }
            .boxed()
        },
        CliHooks::for_params::<L::Params>(),
    );
}
