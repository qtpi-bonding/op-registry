//! `assert_local_ops_offline`: a local op never touches a connection.

use super::refs::args_for;
use super::{fail_on, Fixture};
use crate::{CallOptions, OpKind, Paths, Registry, Resolve};
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;

/// Runs each `Local` op with `sample_params` (or the override for it) on
/// `ctx`. Whether the op returns `Ok` or `Err` does not matter; a panic, as
/// the app's connector raises when used, fails the check and names the op.
///
/// # Panics
///
/// Panics with every op that panicked.
pub async fn assert_local_ops_offline<C: Resolve>(
    registry: &Registry<C>,
    ctx: &C,
    overrides: &[Fixture<'_>],
) {
    let mut problems = Vec::new();
    for d in registry.descriptors() {
        if d.kind != OpKind::Local || d.paths == Paths::TypedOnly {
            continue;
        }
        let args = args_for(d, overrides);
        let call = registry.call(&d.name, args, ctx, CallOptions::default());
        if let Err(payload) = AssertUnwindSafe(call).catch_unwind().await {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "a panic".to_owned());
            problems.push(format!("{} touched a connection: {message}", d.name));
        }
    }
    fail_on("assert_local_ops_offline", &problems);
}
