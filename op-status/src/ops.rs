//! The `status get` operation, registered with op-registry.
//!
//! The operation reads through a [`StatusSource`] and checks every component
//! with [`check_component`]. Debug builds refuse a violation with the coded
//! error. Release builds drop the offending attributes and never pass them on.

use anyhow::Result;
use futures_util::future::BoxFuture;
use op_registry::{Access, Audience, CallOptions, CliHooks, Descriptor, OpParams, Registry};
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
}

impl Default for StatusOptions {
    fn default() -> Self {
        Self {
            audience: Audience::Agent,
            max_components: 1000,
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
}

/// Parameters of `status get`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, OpParams)]
pub struct GetParams {
    /// Only components of this kind.
    #[op(long = "kind")]
    pub kind: Option<String>,
    /// Only components with this id.
    #[op(long = "id")]
    pub id: Option<String>,
    /// Maximum number of components to return.
    #[op(long = "limit")]
    pub limit: Option<u32>,
}

fn get_op<'a, C: StatusSource + Sync>(
    source: &'a C,
    params: GetParams,
    max_components: usize,
) -> BoxFuture<'a, Result<StatusSnapshot>> {
    Box::pin(async move {
        let limit = match params.limit {
            Some(requested) => usize::try_from(requested)
                .unwrap_or(usize::MAX)
                .min(max_components),
            None => max_components,
        };
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
    let descriptor = Descriptor::new("status", "get")
        .description("Reads component status snapshots")
        .access(Access::reads())
        .audience(opts.audience)
        .streaming(false)
        .input_schema(<GetParams as OpParams>::schema())
        .output_schema(op_registry::__private::output_schema_of::<StatusSnapshot>());
    registry.push_unary::<GetParams, StatusSnapshot, _>(
        descriptor,
        move |ctx: &C, params: GetParams, _opts: CallOptions| get_op(ctx, params, max),
        CliHooks::for_params::<GetParams>(),
    );
    registry
}
