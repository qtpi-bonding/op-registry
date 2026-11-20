//! The `ops!` declaration macro: parse the block, then expand it.

mod expand;
mod parse;

use proc_macro2::TokenStream;

pub fn expand(input: TokenStream) -> TokenStream {
    match syn::parse2::<parse::Ops>(input) {
        Ok(ops) => expand::expand(&ops),
        Err(error) => error.into_compile_error(),
    }
}
