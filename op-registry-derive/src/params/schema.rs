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
        Base::String | Base::Path => quote!(#json::json!({"type": "string"})),
        Base::Int { signed: true, .. } => quote!(#json::json!({"type": "integer"})),
        Base::Int { signed: false, .. } => {
            quote!(#json::json!({"type": "integer", "minimum": 0}))
        }
        Base::Float => quote!(#json::json!({"type": "number"})),
        Base::Bool => quote!(#json::json!({"type": "boolean"})),
        Base::Enum(ty) => quote!(#json::json!({
            "type": "string",
            "enum": ::op_registry::__private::rt::enum_names::<#ty>(),
        })),
    }
}

fn shape_schema(f: &Field) -> TokenStream {
    let json = json_path();
    match &f.shape {
        Shape::Plain(b) | Shape::Optional(b) => base_schema(b),
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
    let value = match base {
        Base::Enum(ty) => quote!(#json::json!(::op_registry::__private::rt::enum_name::<#ty>(&(#e)))),
        _ => quote!(#json::to_value(&(#e)).expect("op-registry: default is not serialisable")),
    };
    quote!(object.insert("default".to_owned(), #value);)
}

pub fn input_schema_fn(fields: &[Field]) -> TokenStream {
    let json = json_path();
    let props = fields.iter().map(|f| {
        let key = &f.key;
        let schema = shape_schema(f);
        let description = if f.help.is_empty() {
            quote!()
        } else {
            let help = &f.help;
            quote!(object.insert("description".to_owned(), #json::json!(#help));)
        };
        let default = default_schema(f);
        quote! {
            let mut property = #schema;
            if let Some(object) = property.as_object_mut() {
                #description
                #default
            }
            properties.insert(#key.to_owned(), property);
        }
    });
    let required = fields.iter().filter(|f| f.required()).map(|f| &f.key);
    quote! {
        fn input_schema() -> #json::Value {
            let mut properties = #json::Map::new();
            #(#props)*
            let mut schema = #json::json!({
                "type": "object",
                "properties": properties,
                "additionalProperties": false,
            });
            let required: ::std::vec::Vec<&str> = ::std::vec![#(#required),*];
            if !required.is_empty() {
                schema["required"] = #json::json!(required);
            }
            schema
        }
    }
}

pub fn fields_fn(fields: &[Field]) -> TokenStream {
    let metas = fields.iter().map(|f| {
        let (rust_name, flag, key, help) = (&f.rust_name, &f.flag, &f.key, &f.help);
        let kind = match f.shape {
            Shape::Plain(_) | Shape::Optional(_) => quote!(Scalar),
            Shape::Switch => quote!(Switch),
            Shape::TriState => quote!(TriState),
            Shape::Repeated(_) => quote!(Repeated),
            Shape::Map => quote!(Map),
        };
        let (required, secret) = (f.required(), f.secret);
        quote! {
            ::op_registry::FieldMeta {
                rust_name: #rust_name,
                flag: #flag.to_owned(),
                key: #key.to_owned(),
                help: #help.to_owned(),
                kind: ::op_registry::FieldKind::#kind,
                required: #required,
                secret: #secret,
                from_group: ::std::option::Option::None,
            }
        }
    });
    quote! {
        fn fields() -> ::std::vec::Vec<::op_registry::FieldMeta> {
            ::std::vec![#(#metas),*]
        }
    }
}
