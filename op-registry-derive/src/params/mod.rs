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
use types::Shape;

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
    let mut fields = named
        .named
        .iter()
        .map(Field::parse)
        .collect::<syn::Result<Vec<_>>>()?;
    let struct_attrs = struct_attrs(&input.attrs)?;
    if struct_attrs.allow_hyphen {
        for f in fields.iter_mut().filter(|f| {
            matches!(
                f.shape,
                Shape::Plain(_) | Shape::Optional(_) | Shape::Repeated(_) | Shape::Patch(_)
            )
        }) {
            f.allow_hyphen = true;
        }
    }

    relations::check_relations(&fields)?;
    let fields_fn = schema::fields_fn(&fields);
    let schema_fn = schema::input_schema_fn(&fields);
    let add_args = clap_gen::add_args_fn(&fields);
    let from_matches = clap_gen::from_matches_fn(&fields);
    let from_json = json_gen::from_json_fn(&fields);
    let validate = schema::validate_fn(&fields, struct_attrs.validator.as_ref());
    let ref_slots = ref_gen::ref_slots_fn(&fields);
    let secrets = schema::secrets_fn(&fields);
    let relations_fn = relations::relations_fn(&fields);
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
            #relations_fn
        }
    })
}

/// The struct-level `#[op(..)]` keys: `validate = path` and `allow_hyphen`.
struct StructAttrs {
    validator: Option<syn::Path>,
    allow_hyphen: bool,
}

fn struct_attrs(attrs: &[syn::Attribute]) -> syn::Result<StructAttrs> {
    let mut out = StructAttrs {
        validator: None,
        allow_hyphen: false,
    };
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
                    out.validator = Some(m.value()?.parse::<syn::Path>().map_err(|_| bad())?);
                    Ok(())
                }
                "allow_hyphen" => {
                    out.allow_hyphen = true;
                    Ok(())
                }
                _ => Err(m.error(format!("unknown #[op] key `{key}`"))),
            }
        })?;
    }
    Ok(out)
}
