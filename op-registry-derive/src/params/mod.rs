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

/// Checks that every relation names fields the derive can see.
fn check_relations(fields: &[Field]) -> syn::Result<()> {
    for (name, optional, members) in schema::one_of_groups(fields) {
        if members.len() < 2 {
            return Err(syn::Error::new(
                members[0].ident.span(),
                format!("one_of group `{name}` needs at least two `Option` fields"),
            ));
        }
        if members
            .iter()
            .any(|f| f.one_of.as_ref().map(|g| g.1) != Some(optional))
        {
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

/// One subject at most, and every scope name is a field of this struct or a
/// dotted path starting at one of its flattened groups.
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
        if let Some(r) = &f.reference {
            check_scope(fields, f, &r.scope)?;
        }
        check_scope(fields, f, &f.group_scope)?;
    }
    Ok(())
}

/// Checks one `scope = [..]` list: names resolve, and keys are distinct.
fn check_scope(fields: &[Field], owner: &Field, scope: &[attrs::ScopeName]) -> syn::Result<()> {
    let mut keys: Vec<(String, &str)> = Vec::new();
    for name in scope {
        let segments = name.segments();
        if segments.iter().any(|s| s.is_empty()) {
            return Err(syn::Error::new(
                name.span,
                format!("scope names `{}`, which is not a field path", name.path),
            ));
        }
        let head = fields.iter().find(|o| o.rust_name == segments[0]);
        let ok = if segments.len() == 1 {
            head.is_some_and(|o| {
                o.rust_name != owner.rust_name && !matches!(o.shape, types::Shape::Flatten(_))
            })
        } else {
            head.is_some_and(|o| matches!(o.shape, types::Shape::Flatten(_)))
        };
        if !ok {
            let span = if segments.len() == 1 {
                owner.ident.span()
            } else {
                name.span
            };
            let what = if segments.len() == 1 {
                "which is not another field of this struct"
            } else {
                "whose first segment is not a flattened group of this struct"
            };
            return Err(syn::Error::new(
                span,
                format!("scope names `{}`, {what}", name.path),
            ));
        }
        let key = name.key();
        if let Some((_, first)) = keys.iter().find(|(k, _)| *k == key) {
            return Err(syn::Error::new(
                name.span,
                format!(
                    "scope names `{first}` and `{}`, which share the scope key `{key}`",
                    name.path
                ),
            ));
        }
        keys.push((key, &name.path));
    }
    Ok(())
}
