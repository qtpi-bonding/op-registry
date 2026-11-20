//! The `OpGlobals` derive: the params field model, with `#[op(env = "...")]`,
//! emitted as global clap arguments only.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

use crate::params::attrs::Field;
use crate::params::types::{Base, Shape};
use crate::params::{clap_gen, schema};

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(name, "OpGlobals requires a struct"));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new_spanned(
            name,
            "OpGlobals requires named fields",
        ));
    };
    let fields = named
        .named
        .iter()
        .map(|f| Field::parse_with(f, true))
        .collect::<syn::Result<Vec<_>>>()?;
    for (f, raw) in fields.iter().zip(&named.named) {
        check(f, raw)?;
    }

    let fields_fn = schema::fields_fn(&fields);
    let steps = fields.iter().map(|f| {
        let args = clap_gen::args(f);
        let env = global_env(f);
        // Clap forbids a required global argument; `from_matches` reports a
        // missing one as `usage.missing` instead.
        // The env fallback belongs to the first (positive) argument only.
        let first = args
            .first()
            .map(|a| quote!(cmd = cmd.arg(#a.required(false).global(true) #env);));
        let rest = args
            .iter()
            .skip(1)
            .map(|a| quote!(cmd = cmd.arg(#a.required(false).global(true));));
        quote!(#first #(#rest)*)
    });
    let lets = fields.iter().map(|f| {
        let (ident, value) = (&f.ident, clap_gen::decode(f));
        quote!(let #ident = #value;)
    });
    let idents = fields.iter().map(|f| &f.ident);
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::op_registry::OpGlobals for #name #ty_generics #where_clause {
            #fields_fn
            fn add_args(
                mut cmd: ::op_registry::__private::clap::Command,
            ) -> ::op_registry::__private::clap::Command {
                #(#steps)*
                cmd
            }
            fn from_matches(
                m: &::op_registry::__private::clap::ArgMatches,
            ) -> ::op_registry::__private::anyhow::Result<Self> {
                #(#lets)*
                ::std::result::Result::Ok(Self { #(#idents),* })
            }
        }
    })
}

/// `.env(..)` for a field with an env fallback; the value stays out of help.
fn global_env(f: &Field) -> Option<TokenStream> {
    let var = f.env.as_ref()?;
    let hide = if f.secret {
        quote!(.hide_env(true))
    } else {
        quote!(.hide_env_values(true))
    };
    Some(quote!(.env(#var) #hide))
}

/// The field rules of globals: no Ref, Json, Input, SecretFile or flatten, no
/// relations, no clearable patch; `env` only on a scalar or switch.
fn check(f: &Field, raw: &syn::Field) -> syn::Result<()> {
    let err = |msg: &str| {
        Err(syn::Error::new_spanned(
            &raw.ty,
            format!("global `{}`: {msg}", f.rust_name),
        ))
    };
    let bad_base = |b: &Base| matches!(b, Base::Json | Base::Input | Base::Ref | Base::SecretFile);
    match &f.shape {
        Shape::Flatten(_) => return err("a global cannot be flattened"),
        Shape::Patch(_) => return err("a global cannot be clearable"),
        Shape::Plain(b) | Shape::Optional(b) | Shape::Repeated(b) if bad_base(b) => {
            return err("a global cannot be a Ref, Json, Input or SecretFile")
        }
        _ => {}
    }
    if f.one_of.is_some()
        || f.at_least_one.is_some()
        || f.explicit_required
        || !f.requires_if.is_empty()
        || !f.conflicts_with.is_empty()
        || !f.required_unless.is_empty()
    {
        return err("a global takes no relation and no required");
    }
    if f.env.is_some()
        && !matches!(
            f.shape,
            Shape::Plain(_) | Shape::Optional(_) | Shape::Switch
        )
    {
        return err("env applies only to a scalar, an optional scalar or a bool");
    }
    Ok(())
}
