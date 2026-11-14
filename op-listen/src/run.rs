//! Construction and execution of authorized listen streams.

use crate::{ListenCursor, ListenError, ListenFrame, Listener, Opened, OverflowPolicy};
use futures_util::{future::BoxFuture, stream::BoxStream, StreamExt};
use op_registry::Caller;
use std::{collections::VecDeque, time::Duration};
use tokio::time::{self, Instant, MissedTickBehavior};

/// Options for [`listen_stream`]. Reserved for future use.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ListenOptions {}

type Frame<C, L> = ListenFrame<<L as Listener<C>>::Snapshot, <L as Listener<C>>::Item>;
type Source<'a, C, L> = BoxStream<'a, anyhow::Result<(u64, <L as Listener<C>>::Item)>>;

/// Everything the listen loop needs between frames.
///
/// The check future borrows the caller and topic, so both are stored behind
/// `Arc`s and the check future is created with cloned handles owned by the
/// future itself. That keeps `RunState` free of self-references.
struct RunState<'a, C, L: Listener<C>> {
    listener: &'a L,
    ctx: &'a C,
    params: L::Params,
    caller: std::sync::Arc<Caller>,
    topic: std::sync::Arc<String>,
    source: Source<'a, C, L>,
    interval: Option<time::Interval>,
    pending_check: Option<BoxFuture<'a, bool>>,
    queue: VecDeque<anyhow::Result<Frame<C, L>>>,
    last_seq: Option<u64>,
    floor: Option<ListenCursor>,
    done: bool,
}

fn out_of_order(previous: u64, next: u64) -> anyhow::Error {
    coded_error::into_anyhow(ListenError::OutOfOrder { previous, next })
}

fn resnapshot_failed() -> anyhow::Error {
    coded_error::into_anyhow(ListenError::ResnapshotFailed)
}

/// What one turn of the select loop produced.
enum Event<I> {
    /// A source item, or the end of the source when `None`.
    Source(Option<anyhow::Result<(u64, I)>>),
    /// The pending authorization check resolved.
    Check(bool),
    /// The recheck interval ticked.
    Tick,
}

impl<'a, C, L> RunState<'a, C, L>
where
    C: Sync + 'a,
    L: Listener<C>,
{
    fn cursor(&self, seq: u64) -> String {
        ListenCursor::new(self.listener.epoch(), seq).to_string()
    }

    /// Records `seq` as the newest sequence, or reports it as out of order.
    /// Items must strictly increase; a snapshot may repeat the last seq.
    fn advance(&mut self, seq: u64, snapshot: bool) -> anyhow::Result<()> {
        if let Some(previous) = self.last_seq {
            if seq < previous || (seq == previous && !snapshot) {
                return Err(out_of_order(previous, seq));
            }
        }
        self.last_seq = Some(seq);
        Ok(())
    }
}

/// Builds the Stop frame for a revoked topic.
fn stop_frame<C, L>(listener: &L, topic: &str) -> Frame<C, L>
where
    C: Sync,
    L: Listener<C>,
{
    let error = listener.revoked_error(topic);
    let code = coded_error::facts_anyhow(&error).code.to_owned();
    ListenFrame::Stop {
        code,
        message: error.to_string(),
    }
}

/// Starts a `still_allowed` check whose future owns its arguments.
fn start_check<'a, C, L>(state: &RunState<'a, C, L>) -> BoxFuture<'a, bool>
where
    C: Sync + 'a,
    L: Listener<C>,
{
    let listener = state.listener;
    let ctx = state.ctx;
    let caller = state.caller.clone();
    let topic = state.topic.clone();
    Box::pin(async move { listener.still_allowed(ctx, &caller, &topic).await })
}

async fn next_run<'a, C, L>(
    mut state: RunState<'a, C, L>,
) -> Option<(anyhow::Result<Frame<C, L>>, RunState<'a, C, L>)>
where
    C: Sync + 'a,
    L: Listener<C>,
{
    if let Some(frame) = state.queue.pop_front() {
        return Some((frame, state));
    }
    if state.done {
        return None;
    }
    loop {
        let event = {
            let check_pending = state.pending_check.is_some();
            let tick_enabled = state.interval.is_some() && !check_pending;
            let RunState {
                source,
                pending_check,
                interval,
                ..
            } = &mut state;
            tokio::select! {
                item = source.next() => Event::Source(item),
                allowed = async {
                    match pending_check.as_mut() {
                        Some(check) => check.await,
                        None => std::future::pending::<bool>().await,
                    }
                }, if check_pending => Event::Check(allowed),
                _ = async {
                    match interval.as_mut() {
                        Some(interval) => interval.tick().await,
                        None => std::future::pending::<Instant>().await,
                    }
                }, if tick_enabled => Event::Tick,
            }
        };

        match event {
            Event::Check(true) => {
                state.pending_check = None;
            }
            Event::Check(false) => {
                state.pending_check = None;
                state.done = true;
                let frame = stop_frame(state.listener, &state.topic);
                return Some((Ok(frame), state));
            }
            Event::Tick => {
                let check = start_check(&state);
                state.pending_check = Some(check);
            }
            Event::Source(None) => {
                state.done = true;
                return None;
            }
            Event::Source(Some(Ok((seq, item)))) => {
                if state.floor.is_some_and(|floor| seq <= floor.seq) {
                    continue;
                }
                if let Err(error) = state.advance(seq, false) {
                    state.done = true;
                    return Some((Err(error), state));
                }
                let cursor = state.cursor(seq);
                return Some((Ok(ListenFrame::Item { item, cursor }), state));
            }
            Event::Source(Some(Err(error))) => {
                let lagged = coded_error::facts_anyhow(&error).code == "op_listen.lagged";
                if !lagged || state.listener.overflow() == OverflowPolicy::End {
                    state.done = true;
                    return Some((Err(error), state));
                }
                let opened = state
                    .listener
                    .open(state.ctx, state.params.clone(), None, state.floor)
                    .await;
                match opened {
                    Ok(Opened::Snapshot {
                        snapshot,
                        seq,
                        stream,
                        ..
                    }) => {
                        if let Err(error) = state.advance(seq, true) {
                            state.done = true;
                            return Some((Err(error), state));
                        }
                        state.source = stream;
                        let cursor = state.cursor(seq);
                        let frame = ListenFrame::Snapshot {
                            snapshot,
                            resync: true,
                            cursor,
                        };
                        return Some((Ok(frame), state));
                    }
                    Ok(_) => {
                        state.done = true;
                        return Some((Err(resnapshot_failed()), state));
                    }
                    Err(open_error) => {
                        state.done = true;
                        return Some((Err(open_error), state));
                    }
                }
            }
        }
    }
}

/// Authorizes a caller, opens the listener and returns the frame stream.
///
/// Authorization and the first `open` run before this returns; their errors
/// are returned directly. The returned stream only polls the listener's source
/// and the recheck machinery.
pub async fn listen_stream<'a, C, L>(
    listener: &'a L,
    ctx: &'a C,
    params: L::Params,
    cursor: Option<ListenCursor>,
    caller: Caller,
    _opts: ListenOptions,
) -> anyhow::Result<BoxStream<'a, anyhow::Result<ListenFrame<L::Snapshot, L::Item>>>>
where
    C: Sync + 'a,
    L: Listener<C>,
{
    let topic = listener.topic(&params);
    let grant = listener.authorize(ctx, &caller, &topic).await?;
    let effective = match (cursor, grant.not_before) {
        (Some(requested), Some(floor)) => Some(requested.max(floor)),
        (Some(requested), None) => Some(requested),
        (None, floor) => floor,
    };
    let opened = listener
        .open(ctx, params.clone(), effective, grant.not_before)
        .await?;
    let mut queue = VecDeque::new();
    let (source, last_seq) = match opened {
        Opened::Replay(source) => (source, None),
        Opened::Snapshot {
            snapshot,
            resync,
            seq,
            stream,
        } => {
            let cursor = ListenCursor::new(listener.epoch(), seq).to_string();
            queue.push_back(Ok(ListenFrame::Snapshot {
                snapshot,
                resync,
                cursor,
            }));
            (stream, Some(seq))
        }
    };
    let interval = grant.recheck_every.map(|duration: Duration| {
        let mut interval = time::interval_at(Instant::now() + duration, duration);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        interval
    });
    let state = RunState {
        listener,
        ctx,
        params,
        caller: std::sync::Arc::new(caller),
        topic: std::sync::Arc::new(topic),
        source,
        interval,
        pending_check: None,
        queue,
        last_seq,
        floor: grant.not_before,
        done: false,
    };
    Ok(Box::pin(
        futures_util::stream::unfold(state, next_run).fuse(),
    ))
}
