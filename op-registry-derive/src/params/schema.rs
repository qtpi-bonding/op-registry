//! `fields()` and `input_schema()`.

use proc_macro2::TokenStream;
use quote::quote;

use super::attrs::{Field, ScopeName};
use super::relations::{
    at_least_one_groups, keyed_list, kind_name, one_of_groups, resolve, Target,
};
use super::types::{Base, Shape};

fn json_path() -> TokenStream {
    quote!(::op_registry::__private::serde_json)
}

fn base_schema(base: &Base) -> TokenStream {
    let json = json_path();
    match base {
        Base::String | Base::Path | Base::Ref | Base::SecretFile => {
            quote!(#json::json!({"type": "string"}))
        }
        Base::Int { signed: true, .. } => quote!(#json::json!({"type": "integer"})),
        Base::Int { signed: false, .. } => {
            quote!(#json::json!({"type": "integer", "minimum": 0}))
        }
        Base::Float => quote!(#json::json!({"type": "number"})),
        Base::Bool => quote!(#json::json!({"type": "boolean"})),
        Base::Json => quote!(#json::json!({})),
        Base::Input => quote!(#json::json!({"type": "string", "contentEncoding": "base64"})),
        Base::Enum(ty) => quote!(#json::json!({
            "type": "string",
            "enum": ::op_registry::__private::rt::enum_names::<#ty>(),
        })),
    }
}

fn shape_schema(f: &Field) -> TokenStream {
    let json = json_path();
    match &f.shape {
        Shape::Plain(Base::SecretFile) | Shape::Optional(Base::SecretFile) if f.secret_inline => {
            quote!(#json::json!({"oneOf": [
                {"type": "string"},
                {
                    "type": "object",
                    "properties": {"inline": {"type": "string", "contentEncoding": "base64"}},
                    "required": ["inline"],
                    "additionalProperties": false,
                },
            ]}))
        }
        Shape::Plain(b) | Shape::Optional(b) => base_schema(b),
        Shape::Patch(b) => {
            let base = base_schema(b);
            quote!(::op_registry::__private::rt::nullable(#base))
        }
        Shape::Flatten(_) => unreachable!("flattened groups have no property"),
        Shape::Repeated(b) => {
            let item = base_schema(b);
            quote!(#json::json!({"type": "array", "items": #item}))
        }
        Shape::Switch | Shape::TriState => quote!(#json::json!({"type": "boolean"})),
        Shape::Map => match &f.keys {
            Some(keys) => {
                let props = keys.iter().map(|k| quote!(#k: {"type": "string"}));
                quote!(#json::json!({
                    "type": "object",
                    "properties": { #(#props),* },
                    "additionalProperties": false,
                }))
            }
            None => quote!(#json::json!({
                "type": "object",
                "additionalProperties": {"type": "string"},
            })),
        },
    }
}

fn default_schema(f: &Field) -> TokenStream {
    let json = json_path();
    let (Some(e), Shape::Plain(base)) = (&f.default, &f.shape) else {
        return quote!();
    };
    if f.secret {
        return quote!();
    }
    let value = match base {
        Base::Enum(ty) => {
            quote!(#json::json!(::op_registry::__private::rt::enum_name::<#ty>(&(#e))))
        }
        _ => quote!(#json::to_value(&(#e)).expect("op-registry: default is not serialisable")),
    };
    quote!(object.insert("default".to_owned(), #value);)
}

pub fn input_schema_fn(fields: &[Field]) -> TokenStream {
    let json = json_path();
    let steps = fields.iter().map(|f| {
        if let Shape::Flatten(ty) = &f.shape {
            return quote!(builder.absorb(<#ty as ::op_registry::OpParams>::input_schema()););
        }
        let key = &f.key;
        let schema = shape_schema(f);
        let schema = if in_relation(fields, f)
            && !f.explicit_required
            && !matches!(f.shape, Shape::Patch(_))
        {
            quote!(::op_registry::__private::rt::nullable(#schema))
        } else {
            schema
        };
        let description = if f.help.is_empty() {
            quote!()
        } else {
            let help = &f.help;
            quote!(object.insert("description".to_owned(), #json::json!(#help));)
        };
        let default = default_schema(f);
        let default = if f.secret {
            quote!(object.insert("writeOnly".to_owned(), #json::json!(true));)
        } else {
            default
        };
        let any_json = matches!(
            &f.shape,
            Shape::Plain(Base::Json) | Shape::Optional(Base::Json)
        ) && f.help.is_empty();
        let description = if any_json {
            quote!(object.insert("description".to_owned(), #json::json!("Any JSON value."));)
        } else {
            description
        };
        let require = f.required().then(|| quote!(builder.require(#key);));
        // A required `Vec` is non-empty, a required any-JSON `Option` non-null.
        let required_bounds = match (&f.shape, f.explicit_required) {
            (Shape::Repeated(_), true) => {
                quote!(object.insert("minItems".to_owned(), #json::json!(1));)
            }
            (Shape::Optional(Base::Json), true) => {
                quote!(object.insert("not".to_owned(), #json::json!({"type": "null"}));)
            }
            _ => quote!(),
        };
        quote! {
            let mut property = #schema;
            if let Some(object) = property.as_object_mut() {
                #description
                #default
                #required_bounds
            }
            builder.property(#key, property);
            #require
        }
    });
    let groups = one_of_groups(fields)
        .into_iter()
        .map(|(_, optional, members)| {
            let members = members.iter().map(|f| member(f));
            quote!(builder.one_of(&[#(#members),*], #optional);)
        });
    let at_least_one = at_least_one_groups(fields).into_iter().map(|(_, members)| {
        let members = members.iter().map(|f| member(f));
        quote!(builder.at_least_one(&[#(#members),*]);)
    });
    let mut relations = Vec::new();
    for f in fields
        .iter()
        .filter(|f| !matches!(f.shape, Shape::Flatten(_)))
    {
        let own = member(f);
        if !f.requires_if.is_empty() {
            let when = keyed_list(fields, &f.requires_if);
            relations.push(quote!({
                #when
                builder.requires_if(#own, &when);
            }));
        }
        for t in &f.conflicts_with {
            let other = resolve(fields, t).keyed();
            relations.push(quote!({
                let other = #other;
                builder.conflicts_with(#own, (other.0.as_str(), other.1));
            }));
        }
        if !f.required_unless.is_empty() {
            let when = keyed_list(fields, &f.required_unless);
            relations.push(quote!({
                #when
                builder.required_unless(#own, &when);
            }));
        }
    }
    // A leaf of a flattened group that a relation names admits `null`.
    let dotted_nullable = fields
        .iter()
        .flat_map(|f| {
            f.requires_if
                .iter()
                .chain(&f.conflicts_with)
                .chain(&f.required_unless)
        })
        .filter(|t| t.segments().len() > 1)
        .map(|t| {
            let key = resolve(fields, t).key();
            quote!(builder.nullable(&#key);)
        });
    quote! {
        fn input_schema() -> #json::Value {
            let mut builder = ::op_registry::__private::rt::SchemaBuilder::default();
            #(#steps)*
            #(#dotted_nullable)*
            #(#groups)*
            #(#at_least_one)*
            #(#relations)*
            builder.build()
        }
    }
}

/// `(key, kind)` of a relation member, for the schema builder.
fn member(f: &Field) -> TokenStream {
    let (key, kind) = (&f.key, kind_name(f));
    quote!((#key, ::op_registry::FieldKind::#kind))
}

/// Whether a relation names the field, so its property must admit `null`
/// (which `from_json` reads as absent).
fn in_relation(fields: &[Field], f: &Field) -> bool {
    let names = |list: &[ScopeName]| list.iter().any(|t| t.path == f.rust_name);
    f.one_of.is_some()
        || f.at_least_one.is_some()
        || !f.requires_if.is_empty()
        || !f.conflicts_with.is_empty()
        || !f.required_unless.is_empty()
        || fields
            .iter()
            .any(|o| names(&o.requires_if) || names(&o.conflicts_with) || names(&o.required_unless))
}

pub fn fields_fn(fields: &[Field]) -> TokenStream {
    let pushes = fields.iter().map(|f| {
        if let Shape::Flatten(ty) = &f.shape {
            let group = match ty {
                syn::Type::Path(p) => p
                    .path
                    .segments
                    .last()
                    .map(|s| s.ident.to_string())
                    .unwrap_or_default(),
                _ => String::new(),
            };
            return quote! {
                for mut meta in <#ty as ::op_registry::OpParams>::fields() {
                    meta.from_group.get_or_insert(#group);
                    fields.push(meta);
                }
            };
        }
        let (rust_name, flag, key, help) = (&f.rust_name, &f.flag, &f.key, &f.help);
        let kind = kind_name(f);
        let (required, secret) = (f.required(), f.secret);
        let subject = f.reference.as_ref().filter(|r| r.subject).map(|r| {
            let (kind, id_only) = (&r.kind, r.id_only);
            quote!(.with_subject(::op_registry::SubjectMeta::new(#kind).id_only(#id_only)))
        });
        let env = f.env.as_ref().map(|var| quote!(.with_env(#var)));
        quote! {
            fields.push(
                ::op_registry::FieldMeta::new(#rust_name, ::op_registry::FieldKind::#kind)
                    .with_flag(#flag)
                    .with_key(#key)
                    .with_help(#help)
                    .required(#required)
                    .secret(#secret)
                    #subject
                    #env
            );
        }
    });
    quote! {
        fn fields() -> ::std::vec::Vec<::op_registry::FieldMeta> {
            let mut fields = ::std::vec::Vec::new();
            #(#pushes)*
            fields
        }
    }
}

/// `validate()`: the relations, then each flattened group's own, then the
/// struct's `#[op(validate = ..)]`, whose `Err` is a usage error.
pub fn validate_fn(fields: &[Field], validator: Option<&syn::Path>) -> TokenStream {
    let rt = quote!(::op_registry::__private::rt);
    let groups = one_of_groups(fields)
        .into_iter()
        .map(|(name, optional, members)| {
            let present = members.iter().filter_map(|f| f.present());
            quote! {
                #rt::check_one_of(
                    #name,
                    [#(#present),*].into_iter().filter(|p| *p).count(),
                    #optional,
                )?;
            }
        });
    let required = fields
        .iter()
        .filter(|f| f.explicit_required)
        .filter_map(|f| {
            let (present, key) = (f.present()?, &f.key);
            Some(quote!(#rt::check_required(#present, #key)?;))
        });
    let at_least_one = at_least_one_groups(fields)
        .into_iter()
        .map(|(name, members)| {
            let present = members.iter().filter_map(|f| f.present());
            quote! {
                #rt::check_at_least_one(
                    #name,
                    [#(#present),*].into_iter().filter(|p| *p).count(),
                )?;
            }
        });
    let mut relations = Vec::new();
    for f in fields {
        let Some(present) = f.present() else { continue };
        let key = &f.key;
        for t in &f.requires_if {
            let other = resolve(fields, t);
            let (other_present, other_key) = (other.present(), other.key());
            relations.push(quote! {
                if #other_present && !#present {
                    return Err(#rt::requires_if_error(#key, #other_key));
                }
            });
        }
        for t in &f.conflicts_with {
            let other = resolve(fields, t);
            let (other_present, other_key) = (other.present(), other.key());
            relations.push(quote! {
                if #present && #other_present {
                    return Err(#rt::conflicts_error(#key, #other_key));
                }
            });
        }
        if !f.required_unless.is_empty() {
            let others: Vec<_> = f
                .required_unless
                .iter()
                .map(|t| resolve(fields, t))
                .collect();
            let other_present = others.iter().map(Target::present);
            let other_keys = others.iter().map(Target::key);
            relations.push(quote! {
                if !(#present #(|| #other_present)*) {
                    return Err(#rt::missing_unless_error(#key, ::std::vec![#(#other_keys),*]));
                }
            });
        }
    }
    let nested = fields.iter().filter_map(|f| match &f.shape {
        Shape::Flatten(_) => {
            let ident = &f.ident;
            Some(quote!(::op_registry::OpParams::validate(&self.#ident)?;))
        }
        _ => None,
    });
    let validator = validator.map(|path| {
        quote! {
            if let ::std::result::Result::Err(error) = #path(self) {
                return ::std::result::Result::Err(::op_registry::usage(error));
            }
        }
    });
    quote! {
        fn validate(&self) -> ::op_registry::__private::anyhow::Result<()> {
            #(#required)*
            #(#groups)*
            #(#at_least_one)*
            #(#relations)*
            #(#nested)*
            #validator
            ::std::result::Result::Ok(())
        }
    }
}

/// `secrets()`: the bytes of every `SecretFile` and `#[op(secret)]` field,
/// then each flattened group's own.
pub fn secrets_fn(fields: &[Field]) -> TokenStream {
    let rt = quote!(::op_registry::__private::rt);
    let pushes = fields.iter().filter_map(|f| {
        let ident = &f.ident;
        match &f.shape {
            Shape::Flatten(_) => {
                Some(quote!(out.extend(::op_registry::OpParams::secrets(&self.#ident));))
            }
            Shape::Plain(Base::SecretFile) | Shape::Optional(Base::SecretFile) => {
                Some(quote!(#rt::SecretValue::push_secrets(&self.#ident, &mut out);))
            }
            _ if f.secret => Some(quote!(#rt::SecretValue::push_secrets(&self.#ident, &mut out);)),
            _ => None,
        }
    });
    quote! {
        fn secrets(&self) -> ::std::vec::Vec<::std::vec::Vec<u8>> {
            let mut out = ::std::vec::Vec::new();
            #(#pushes)*
            out
        }
    }
}
