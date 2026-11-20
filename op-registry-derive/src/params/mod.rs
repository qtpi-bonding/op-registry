//! The `OpParams` derive: parse the struct once into `Field`s, then emit one
//! trait method per generator module.

pub(crate) mod attrs;
pub(crate) mod clap_gen;
mod json_gen;
mod ref_gen;
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

    check_relations(&fields)?;
    let fields_fn = schema::fields_fn(&fields);
    let schema_fn = schema::input_schema_fn(&fields);
    let add_args = clap_gen::add_args_fn(&fields);
    let from_matches = clap_gen::from_matches_fn(&fields);
    let from_json = json_gen::from_json_fn(&fields);
    let validate = schema::validate_fn(&fields);
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
        quote!(const _: () = { ::op_registry::__require_secrets!(); };)
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
    check_refs(fields)?;
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

/// One subject at most, and every scope name is a sibling field.
fn check_refs(fields: &[Field]) -> syn::Result<()> {
    let mut subjects = fields
        .iter()
        .filter(|f| f.reference.as_ref().is_some_and(|r| r.subject));
    subjects.next();
    if let Some(second) = subjects.next() {
        return Err(syn::Error::new(
            second.ident.span(),
            "an op has one subject: only one field takes `#[op(subject = ..)]`",
        ));
    }
    for f in fields {
        let Some(r) = &f.reference else { continue };
        for name in &r.scope {
            let sibling = fields
                .iter()
                .find(|o| &o.rust_name == name && o.rust_name != f.rust_name);
            if sibling.is_none_or(|o| matches!(o.shape, types::Shape::Flatten(_))) {
                return Err(syn::Error::new(
                    f.ident.span(),
                    format!("scope names `{name}`, which is not another field of this struct"),
                ));
            }
        }
    }
    Ok(())
}
