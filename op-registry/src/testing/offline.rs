//! `assert_local_ops_offline`: a local op never touches a connection.

use super::refs::args_for;
use super::{
    decode_problem, fail_on_noting, kit_options, kit_path, typed_only_line, Fixture, KitDir,
};
use crate::{OpKind, Registry, Resolve};
use futures_util::FutureExt;
use serde_json::Map;
use std::panic::AssertUnwindSafe;

/// Runs each `Local` op, on the first surface it declares, with `sample_params` (or the override for it) on
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
    let mut skipped = Vec::new();
    let dir = KitDir::new();
    for d in registry.descriptors() {
        if d.kind != OpKind::Local {
            continue;
        }
        let Some(path) = kit_path(d) else {
            skipped.push(typed_only_line("assert_local_ops_offline", d));
            continue;
        };
        let (args, sampled) = args_for(registry, d, overrides, Map::new(), &dir);
        let call = registry.call_json(&d.name, args, ctx, kit_options(), path, false);
        match AssertUnwindSafe(call).catch_unwind().await {
            Ok(outcome) => {
                if let (true, Err(err)) = (sampled, outcome.result) {
                    problems.extend(decode_problem(&d.name, &[err]));
                }
            }
            Err(payload) => {
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_else(|| "a panic".to_owned());
                problems.push(format!("{} touched a connection: {message}", d.name));
            }
        }
    }
    fail_on_noting("assert_local_ops_offline", &problems, &skipped);
}
