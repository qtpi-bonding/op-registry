//! `fields()` and `input_schema()`.

use proc_macro2::TokenStream;
use quote::quote;

use super::attrs::Field;
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
        let schema = if in_relation(fields, f) && !matches!(f.shape, Shape::Patch(_)) {
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
        quote! {
            let mut property = #schema;
            if let Some(object) = property.as_object_mut() {
                #description
                #default
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
    let requires = fields
        .iter()
        .filter(|f| !f.requires_if.is_empty())
        .map(|f| {
            let field = member(f);
            let others = others_of(fields, f).into_iter().map(member);
            quote!(builder.requires_if(#field, &[#(#others),*]);)
        });
    quote! {
        fn input_schema() -> #json::Value {
            let mut builder = ::op_registry::__private::rt::SchemaBuilder::default();
            #(#steps)*
            #(#groups)*
            #(#requires)*
            builder.build()
        }
    }
}

/// The groups named by `one_of` / `one_of_optional`: `(name, optional, members)`.
pub fn one_of_groups(fields: &[Field]) -> Vec<(String, bool, Vec<&Field>)> {
    let mut groups: Vec<(String, bool, Vec<&Field>)> = Vec::new();
    for f in fields {
        let Some((name, optional)) = &f.one_of else {
            continue;
        };
        match groups.iter_mut().find(|g| &g.0 == name) {
            Some(g) => g.2.push(f),
            None => groups.push((name.clone(), *optional, vec![f])),
        }
    }
    groups
}

/// The fields a `requires_if` field depends on.
pub fn others_of<'a>(fields: &'a [Field], f: &Field) -> Vec<&'a Field> {
    f.requires_if
        .iter()
        .filter_map(|name| fields.iter().find(|o| &o.rust_name == name))
        .collect()
}

/// The `FieldKind` variant name of a field (never a flattened group).
fn kind_name(f: &Field) -> TokenStream {
    match &f.shape {
        Shape::Plain(Base::Json) | Shape::Optional(Base::Json) => quote!(Json),
        Shape::Plain(Base::Input) | Shape::Optional(Base::Input) => quote!(Input),
        Shape::Plain(Base::SecretFile) | Shape::Optional(Base::SecretFile) => quote!(SecretFile),
        Shape::Plain(Base::Ref) | Shape::Optional(Base::Ref) => quote!(Ref),
        Shape::Plain(Base::Path) | Shape::Optional(Base::Path) => quote!(Path),
        Shape::Plain(_) | Shape::Optional(_) => quote!(Scalar),
        Shape::Patch(_) => quote!(Patch),
        Shape::Switch => quote!(Switch),
        Shape::TriState => quote!(TriState),
        Shape::Repeated(_) => quote!(Repeated),
        Shape::Map => quote!(Map),
        Shape::Flatten(_) => unreachable!("flattened groups have no kind"),
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
    f.one_of.is_some()
        || !f.requires_if.is_empty()
        || fields.iter().any(|o| o.requires_if.contains(&f.rust_name))
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

/// `validate()`: the relations, then each flattened group's own.
pub fn validate_fn(fields: &[Field]) -> TokenStream {
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
    let mut requires = Vec::new();
    for f in fields {
        for other in others_of(fields, f) {
            let (Some(present), Some(other_present)) = (f.present(), other.present()) else {
                continue;
            };
            let (key, other_key) = (&f.key, &other.key);
            requires
                .push(quote!(#rt::check_requires_if(#present, #other_present, #key, #other_key)?;));
        }
    }
    let nested = fields.iter().filter_map(|f| match &f.shape {
        Shape::Flatten(_) => {
            let ident = &f.ident;
            Some(quote!(::op_registry::OpParams::validate(&self.#ident)?;))
        }
        _ => None,
    });
    quote! {
        fn validate(&self) -> ::op_registry::__private::anyhow::Result<()> {
            #(#groups)*
            #(#requires)*
            #(#nested)*
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
