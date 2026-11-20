//! The `OpParams` derive: parse the struct once into `Field`s, then emit one
//! trait method per generator module.

mod attrs;
mod clap_gen;
mod json_gen;
mod schema;
mod types;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

use attrs::Field;

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(name, "OpParams requires a struct"));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new_spanned(
            name,
            "OpParams requires named fields",
        ));
    };
    let fields = named
        .named
        .iter()
        .map(Field::parse)
        .collect::<syn::Result<Vec<_>>>()?;

    check_relations(&fields)?;
    let fields_fn = schema::fields_fn(&fields);
    let schema_fn = schema::input_schema_fn(&fields);
    let add_args = clap_gen::add_args_fn(&fields);
    let from_matches = clap_gen::from_matches_fn(&fields);
    let from_json = json_gen::from_json_fn(&fields);
    let validate = schema::validate_fn(&fields);
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::op_registry::OpParams for #name #ty_generics #where_clause {
            #fields_fn
            #schema_fn
            #add_args
            #from_matches
            #from_json
            #validate
        }
    })
}

/// Checks that every relation names fields the derive can see.
fn check_relations(fields: &[Field]) -> syn::Result<()> {
    for (name, optional, members) in schema::one_of_groups(fields) {
        if members.len() < 2 {
            return Err(syn::Error::new(
                members[0].ident.span(),
                format!("one_of group `{name}` needs at least two `Option` fields"),
            ));
        }
        if members.iter().any(|f| f.one_of.as_ref().map(|g| g.1) != Some(optional)) {
            return Err(syn::Error::new(
                members[0].ident.span(),
                format!("group `{name}` mixes one_of and one_of_optional"),
            ));
        }
    }
    for f in fields {
        if let Some(other) = &f.requires_if {
            let target = fields.iter().find(|o| &o.rust_name == other);
            if target.and_then(|t| t.present()).is_none() {
                return Err(syn::Error::new(
                    f.ident.span(),
                    format!(
                        "requires_if names `{other}`, which must be an optional field of this struct"
                    ),
                ));
            }
        }
    }
    Ok(())
}
