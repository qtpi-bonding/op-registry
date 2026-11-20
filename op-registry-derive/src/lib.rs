//! Derive macros for op-registry.

mod globals;
mod ops;
mod params;

use proc_macro::TokenStream;

/// Derives `op_registry::OpParams`: one struct gives the clap arguments, the
/// camelCase JSON decoder and the input schema.
#[proc_macro_derive(OpParams, attributes(op))]
pub fn derive_op_params(input: TokenStream) -> TokenStream {
    params::derive(syn::parse_macro_input!(input as syn::DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Declares every operation of a binary in one block: the registry builder
/// `registry()` and a typed marker per op in `ops`.
///
/// See the op-registry design, section 2.1, for the grammar.
///
/// The markers are `pub` items of `pub mod ops` and name the op's param and
/// output types in their `Op` impls, so those types must be at least as
/// visible as the markers need (a private type gives error E0446).
#[proc_macro]
pub fn ops(input: TokenStream) -> TokenStream {
    ops::expand(input.into()).into()
}

/// Derives `op_registry::OpGlobals`: the process settings of one binary, as
/// global clap arguments with an optional `#[op(env = "...")]` fallback.
///
/// The field rules are those of `OpParams`, except `Ref`, `Json`, `Input`,
/// `SecretFile` and `flatten` fields are rejected.
#[proc_macro_derive(OpGlobals, attributes(op))]
pub fn derive_op_globals(input: TokenStream) -> TokenStream {
    globals::derive(syn::parse_macro_input!(input as syn::DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
