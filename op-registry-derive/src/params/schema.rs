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
        Base::String | Base::Path | Base::Ref | Base::SecretFile => quote!(#json::json!({"type": "string"})),
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
        Base::Enum(ty) => quote!(#json::json!(::op_registry::__private::rt::enum_name::<#ty>(&(#e)))),
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
        let any_json = matches!(&f.shape, Shape::Plain(Base::Json) | Shape::Optional(Base::Json))
            && f.help.is_empty();
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
    let groups = one_of_groups(fields).into_iter().map(|(_, optional, members)| {
        let keys = members.iter().map(|f| &f.key);
        quote!(builder.one_of(&[#(#keys),*], #optional);)
    });
    let depends = fields.iter().filter_map(|f| {
        let other = other_of(fields, f)?;
        let (other, key) = (&other.key, &f.key);
        Some(quote!(builder.depends(#other, #key);))
    });
    quote! {
        fn input_schema() -> #json::Value {
            let mut builder = ::op_registry::__private::rt::SchemaBuilder::default();
            #(#steps)*
            #(#groups)*
            #(#depends)*
            builder.build()
        }
    }
}

/// The groups named by `one_of` / `one_of_optional`: `(name, optional, members)`.
pub fn one_of_groups(fields: &[Field]) -> Vec<(String, bool, Vec<&Field>)> {
    let mut groups: Vec<(String, bool, Vec<&Field>)> = Vec::new();
    for f in fields {
        let Some((name, optional)) = &f.one_of else { continue };
        match groups.iter_mut().find(|g| &g.0 == name) {
            Some(g) => g.2.push(f),
            None => groups.push((name.clone(), *optional, vec![f])),
        }
    }
    groups
}

/// The field a `requires_if` field depends on.
pub fn other_of<'a>(fields: &'a [Field], f: &Field) -> Option<&'a Field> {
    let name = f.requires_if.as_ref()?;
    fields.iter().find(|o| &o.rust_name == name)
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
        let kind = match &f.shape {
            _ if f.reference.is_some() => quote!(Ref),
            Shape::Plain(Base::Json) | Shape::Optional(Base::Json) => quote!(Json),
            Shape::Plain(Base::Input) | Shape::Optional(Base::Input) => quote!(Input),
            Shape::Plain(Base::SecretFile) | Shape::Optional(Base::SecretFile) => {
                quote!(SecretFile)
            }
            Shape::Plain(_) | Shape::Optional(_) | Shape::Patch(_) => quote!(Scalar),
            Shape::Switch => quote!(Switch),
            Shape::TriState => quote!(TriState),
            Shape::Repeated(_) => quote!(Repeated),
            Shape::Map => quote!(Map),
            Shape::Flatten(_) => unreachable!(),
        };
        let (required, secret) = (f.required(), f.secret);
        quote! {
            fields.push(::op_registry::FieldMeta {
                rust_name: #rust_name,
                flag: #flag.to_owned(),
                key: #key.to_owned(),
                help: #help.to_owned(),
                kind: ::op_registry::FieldKind::#kind,
                required: #required,
                secret: #secret,
                from_group: ::std::option::Option::None,
            });
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
    let groups = one_of_groups(fields).into_iter().map(|(name, optional, members)| {
        let present = members.iter().filter_map(|f| f.present());
        quote! {
            #rt::check_one_of(
                #name,
                [#(#present),*].into_iter().filter(|p| *p).count(),
                #optional,
            )?;
        }
    });
    let requires = fields.iter().filter_map(|f| {
        let other = other_of(fields, f)?;
        let (present, other_present) = (f.present()?, other.present()?);
        let (key, other_key) = (&f.key, &other.key);
        Some(quote!(#rt::check_requires_if(#present, #other_present, #key, #other_key)?;))
    });
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
            Shape::Flatten(_) => Some(quote!(out.extend(::op_registry::OpParams::secrets(&self.#ident));)),
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
