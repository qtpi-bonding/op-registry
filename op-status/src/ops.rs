//! The `status get` operation, registered with op-registry.
//!
//! The operation reads through a [`StatusSource`] and checks every component
//! with [`check_component`]. Debug builds refuse a violation with the coded
//! error. Release builds drop the offending attributes and never pass them on.

use anyhow::Result;
use futures_util::future::BoxFuture;
use op_registry::{Access, Audience, CallOptions, OpParams, OpSpec, Registry};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::open::take_snapshot;
use crate::source::StatusSource;
use crate::types::StatusFilter;
use crate::types::StatusSnapshot;

/// Options for [`status_registry`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct StatusOptions {
    /// Audience for the `status get` operation.
    pub audience: Audience,
    /// Largest number of components one call may return.
    pub max_components: usize,
    /// Register `status watch` when the source provides watch.
    pub watch: bool,
}

impl Default for StatusOptions {
    fn default() -> Self {
        Self {
            audience: Audience::Agent,
            max_components: 1000,
            watch: true,
        }
    }
}

impl StatusOptions {
    /// Creates options with the defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the audience.
    pub fn audience(mut self, audience: Audience) -> Self {
        self.audience = audience;
        self
    }

    /// Sets the maximum number of components per call.
    pub fn max_components(mut self, max: usize) -> Self {
        self.max_components = max;
        self
    }

    /// Sets whether `status watch` is registered when the source provides watch.
    pub fn watch(mut self, watch: bool) -> Self {
        self.watch = watch;
        self
    }
}

/// Parameters of `status get`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, OpParams)]
pub struct GetParams {
    /// Only components of this kind.
    pub kind: Option<String>,
    /// Only components with this id.
    pub id: Option<String>,
    /// Maximum number of components to return.
    pub limit: Option<u32>,
}

/// Clamps a requested component limit to `max`. `None` means `max`.
pub(crate) fn clamp_limit(requested: Option<u32>, max: usize) -> usize {
    match requested {
        Some(requested) => usize::try_from(requested).unwrap_or(usize::MAX).min(max),
        None => max,
    }
}

fn get_op<'a, C: StatusSource + Sync>(
    source: &'a C,
    params: GetParams,
    max_components: usize,
) -> BoxFuture<'a, Result<StatusSnapshot>> {
    Box::pin(async move {
        let limit = clamp_limit(params.limit, max_components);
        let filter = StatusFilter {
            kind: params.kind,
            id: params.id,
        };
        take_snapshot(source, &filter, limit).await
    })
}

/// Builds a registry holding the `status get` operation for source `C`.
pub fn status_registry<C: StatusSource + Sync + 'static>(opts: StatusOptions) -> Registry<C> {
    let mut registry = Registry::<C>::new();
    let max = opts.max_components;
    let watch = opts.watch;
    let audience = opts.audience;
    registry.group("status", "Component status.");
    registry.push_op(
        OpSpec::<GetParams, StatusSnapshot> {
            access: Access::reads(),
            audience,
            ..OpSpec::new("status get", "Reads component status snapshots")
        },
        move |ctx: &C, params: GetParams, _opts: CallOptions| get_op(ctx, params, max),
    );
    #[cfg(feature = "listen")]
    if watch && C::provides_watch() {
        op_listen::listen_entry(
            &mut registry,
            std::sync::Arc::new(crate::listen::StatusListener::new(max)),
            OpSpec {
                access: Access::reads(),
                audience,
                ..OpSpec::new(
                    "status watch",
                    "Streams component status changes after a snapshot",
                )
            },
        );
    }
    #[cfg(not(feature = "listen"))]
    let _ = (watch, audience);
    registry
}
