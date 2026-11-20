//! Derive macros for op-registry.

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
#[proc_macro]
pub fn ops(input: TokenStream) -> TokenStream {
    ops::expand(input.into()).into()
}
