//! `from_json()`: camelCase keys only.

use proc_macro2::TokenStream;
use quote::quote;

use super::attrs::Field;
use super::types::{Base, Shape};

fn decode(f: &Field) -> TokenStream {
    let rt = quote!(::op_registry::__private::rt);
    let key = &f.key;
    let ignore_case = f.ignore_case;
    // One optional scalar of a base type; an enum goes through its names.
    let get = |b: &Base| match b {
        Base::SecretFile => {
            let (inline, hide) = (f.secret_inline, f.secret);
            quote!(#rt::secret_file_json(&o, #key, #inline, #hide)?)
        }
        Base::Enum(ty) => quote! {
            #rt::get::<::std::string::String>(&o, #key)?
                .map(|s| #rt::enum_value::<#ty>(#key, &s, #ignore_case))
                .transpose()?
        },
        Base::FromStr(ty) => {
            let hide = f.secret;
            quote! {
                #rt::get::<::std::string::String>(&o, #key)?
                    .map(|s| #rt::from_str_arg::<#ty>(#key, &s, #hide))
                    .transpose()?
            }
        }
        b => {
            let ty = b.ty();
            quote!(#rt::get::<#ty>(&o, #key)?)
        }
    };
    match &f.shape {
        Shape::Flatten(ty) => quote! {
            <#ty as ::op_registry::OpParams>::from_json(
                #rt::sub_object(&o, &<#ty as ::op_registry::OpParams>::fields()),
                _cx,
            )?
        },
        Shape::Patch(Base::Enum(ty)) => quote!(#rt::patch_enum::<#ty>(&o, #key, #ignore_case)?),
        Shape::Patch(b) => {
            let ty = b.ty();
            quote!(#rt::patch::<#ty>(&o, #key)?)
        }
        Shape::Plain(b) => {
            let get = get(b);
            match f.default_value() {
                Some(d) => quote!(#get.unwrap_or_else(|| #d)),
                None => quote!(#rt::require(#get, #key)?),
            }
        }
        Shape::Optional(b) => get(b),
        Shape::Repeated(Base::Enum(ty)) => quote! {
            #rt::get::<::std::vec::Vec<::std::string::String>>(&o, #key)?
                .unwrap_or_default()
                .iter()
                .map(|s| #rt::enum_value::<#ty>(#key, s, #ignore_case))
                .collect::<::std::result::Result<::std::vec::Vec<_>, _>>()?
        },
        Shape::Repeated(Base::FromStr(ty)) => {
            let hide = f.secret;
            quote! {
                #rt::get::<::std::vec::Vec<::std::string::String>>(&o, #key)?
                    .unwrap_or_default()
                    .iter()
                    .map(|s| #rt::from_str_arg::<#ty>(#key, s, #hide))
                    .collect::<::op_registry::__private::anyhow::Result<::std::vec::Vec<_>>>()?
            }
        }
        Shape::Repeated(b) => {
            let ty = b.ty();
            quote!(#rt::get::<::std::vec::Vec<#ty>>(&o, #key)?.unwrap_or_default())
        }
        Shape::Switch => quote!(#rt::get::<bool>(&o, #key)?.unwrap_or(false)),
        Shape::TriState => quote!(#rt::get::<bool>(&o, #key)?),
        Shape::Map => {
            let keys = match &f.keys {
                Some(keys) => quote!(::std::option::Option::Some(&[#(#keys),*][..])),
                None => quote!(::std::option::Option::None::<&[&str]>),
            };
            quote!(#rt::map_from_json(&o, #key, #keys)?)
        }
    }
}

/// A `SecretFile` field decodes its own secret-safe errors.
fn is_secret_file(shape: &Shape) -> bool {
    matches!(
        shape,
        Shape::Plain(Base::SecretFile) | Shape::Optional(Base::SecretFile)
    )
}

pub fn from_json_fn(fields: &[Field]) -> TokenStream {
    let known = fields.iter().map(|f| match &f.shape {
        Shape::Flatten(ty) => quote! {
            known.extend(<#ty as ::op_registry::OpParams>::fields().into_iter().map(|f| f.key));
        },
        _ => {
            let key = &f.key;
            quote!(known.push(#key.to_owned());)
        }
    });
    let lets = fields.iter().map(|f| {
        let (ident, value) = (&f.ident, decode(f));
        if f.secret && !is_secret_file(&f.shape) {
            // serde quotes the value it rejects: say nothing of a secret one.
            let key = &f.key;
            quote! {
                #[allow(clippy::needless_question_mark)]
                let #ident = ::op_registry::__private::rt::hide_value(
                    #key,
                    (|| -> ::op_registry::__private::anyhow::Result<_> {
                        ::std::result::Result::Ok(#value)
                    })(),
                )?;
            }
        } else {
            quote!(let #ident = #value;)
        }
    });
    let idents = fields.iter().map(|f| &f.ident);
    quote! {
        fn from_json(
            v: ::op_registry::__private::serde_json::Value,
            _cx: &mut ::op_registry::ParseCx,
        ) -> ::op_registry::__private::anyhow::Result<Self> {
            let mut known: ::std::vec::Vec<::std::string::String> = ::std::vec::Vec::new();
            #(#known)*
            let o = ::op_registry::__private::rt::object(v, &known)?;
            #(#lets)*
            let this = Self { #(#idents),* };
            ::op_registry::OpParams::validate(&this)?;
            ::std::result::Result::Ok(this)
        }
    }
}
