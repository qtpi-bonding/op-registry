//! Shared fixtures for the registry tests.
#![allow(dead_code)]

use futures_util::future::BoxFuture;
use op_registry::{CallOptions, OpParams, OpSpec, Registry};
use serde::Serialize;

/// The registry context used by the tests.
pub struct Ctx;
impl op_registry::Resolve for Ctx {}

/// A small parameter fixture.
#[derive(Clone, Debug, PartialEq, OpParams)]
pub struct Echo {
    pub value: Option<String>,
}

/// A small output fixture.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Out {
    pub ok: bool,
}

/// Registers a unary op at `path` that returns `Out { ok: true }`.
pub fn push_ok(registry: &mut Registry<Ctx>, spec: OpSpec<Echo, Out>) {
    registry.push_op(
        spec,
        |_ctx: &Ctx, _params: Echo, _opts: CallOptions| -> BoxFuture<'_, anyhow::Result<Out>> {
            Box::pin(async { Ok(Out { ok: true }) })
        },
    );
}

/// A registry holding one unary op per path, each with a group summary for
/// its noun path so the build-time checks pass.
pub fn registry_of(paths: &[&'static str]) -> Registry<Ctx> {
    let mut registry = Registry::new();
    let mut seen: Vec<String> = Vec::new();
    for path in paths {
        let words: Vec<&str> = path.split_whitespace().collect();
        for end in 1..words.len() {
            let group = words[..end].join(" ");
            if !seen.contains(&group) {
                registry.group(&group, "A group.");
                seen.push(group);
            }
        }
        push_ok(&mut registry, OpSpec::new(path, "An op."));
    }
    registry
}
