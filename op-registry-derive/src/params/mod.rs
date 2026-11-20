//! The `OpParams` derive: parse the struct once into `Field`s, then emit one
//! trait method per generator module.

pub(crate) mod attrs;
pub(crate) mod clap_gen;
mod json_gen;
mod ref_gen;
mod relations;
pub(crate) mod schema;
pub(crate) mod types;

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

    relations::check_relations(&fields)?;
    let fields_fn = schema::fields_fn(&fields);
    let schema_fn = schema::input_schema_fn(&fields);
    let add_args = clap_gen::add_args_fn(&fields);
    let from_matches = clap_gen::from_matches_fn(&fields);
    let from_json = json_gen::from_json_fn(&fields);
    let validator = struct_validator(&input.attrs)?;
    let validate = schema::validate_fn(&fields, validator.as_ref());
    let ref_slots = ref_gen::ref_slots_fn(&fields);
    let secrets = schema::secrets_fn(&fields);
    let needs_secrets = fields.iter().any(|f| {
        matches!(
            f.shape,
            types::Shape::Plain(types::Base::SecretFile)
                | types::Shape::Optional(types::Base::SecretFile)
        )
    });
    let require_secrets = needs_secrets.then(|| {
        quote!(
            const _: () = {
                ::op_registry::__require_secrets!();
            };
        )
    });
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        #require_secrets
        impl #impl_generics ::op_registry::OpParams for #name #ty_generics #where_clause {
            #fields_fn
            #schema_fn
            #add_args
            #from_matches
            #from_json
            #validate
            #ref_slots
            #secrets
        }
    })
}

/// The struct-level `#[op(validate = path)]`: the keys of a struct-level
/// `#[op(..)]` are `validate` only (more arrive with their changes).
fn struct_validator(attrs: &[syn::Attribute]) -> syn::Result<Option<syn::Path>> {
    let mut validator = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("op")) {
        attr.parse_nested_meta(|m| {
            let key = m
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            match key.as_str() {
                "validate" => {
                    let bad =
                        || m.error("`validate` takes a function path: #[op(validate = check)]");
                    if !m.input.peek(syn::Token![=]) {
                        return Err(bad());
                    }
                    validator = Some(m.value()?.parse::<syn::Path>().map_err(|_| bad())?);
                    Ok(())
                }
                _ => Err(m.error(format!("unknown #[op] key `{key}`"))),
            }
        })?;
    }
    Ok(validator)
}
