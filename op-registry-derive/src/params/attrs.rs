//! The one `Field` model: parsed once, read by every generator.

use syn::ext::IdentExt;
use syn::{Expr, ExprArray, ExprLit, Ident, Lit};

use super::types::{classify, Shape};

pub struct Field {
    pub ident: Ident,
    pub rust_name: String,
    /// The long flag without dashes.
    pub flag: String,
    /// The camelCase JSON key.
    pub key: String,
    pub help: String,
    pub shape: Shape,
    pub default: Option<Expr>,
    /// A closed key set for map fields.
    pub keys: Option<Vec<String>>,
    pub secret: bool,
    /// `(group, optional)` from `one_of` / `one_of_optional`.
    pub one_of: Option<(String, bool)>,
    /// The Rust name of the field this one is required with.
    pub requires_if: Option<String>,
}

impl Field {
    pub fn parse(f: &syn::Field) -> syn::Result<Field> {
        let ident = f
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new_spanned(f, "OpParams requires named fields"))?;
        let rust_name = ident.unraw().to_string();

        let mut default = None;
        let mut value_enum = false;
        let mut keys = None;
        let mut secret = false;
        let mut one_of: Option<(String, bool)> = None;
        let mut requires_if = None;
        let mut clearable = false;
        let mut flatten = false;
        for attr in f.attrs.iter().filter(|a| a.path().is_ident("op")) {
            attr.parse_nested_meta(|m| {
                let key = m.path.get_ident().map(Ident::to_string).unwrap_or_default();
                match key.as_str() {
                    "default" => default = Some(m.value()?.parse::<Expr>()?),
                    "value_enum" => value_enum = true,
                    "secret" => secret = true,
                    "clearable" => clearable = true,
                    "flatten" => flatten = true,
                    "one_of" | "one_of_optional" => {
                        let group: syn::LitStr = m.value()?.parse()?;
                        if one_of.is_some() {
                            return Err(m.error("one field takes one one_of group"));
                        }
                        one_of = Some((group.value(), key == "one_of_optional"));
                    }
                    "requires_if" => {
                        let other: syn::LitStr = m.value()?.parse()?;
                        requires_if = Some(other.value());
                    }
                    "keys" => {
                        let array: ExprArray = m.value()?.parse()?;
                        let mut out = Vec::new();
                        for e in &array.elems {
                            match e {
                                Expr::Lit(ExprLit {
                                    lit: Lit::Str(s), ..
                                }) => out.push(s.value()),
                                _ => {
                                    return Err(syn::Error::new_spanned(
                                        e,
                                        "keys takes string literals",
                                    ))
                                }
                            }
                        }
                        keys = Some(out);
                    }
                    "long" | "short" | "positional" | "env" => {
                        return Err(m.error(format!(
                            "unknown #[op] key `{key}`: flags are the long kebab-case field name only"
                        )))
                    }
                    _ => return Err(m.error(format!("unknown #[op] key `{key}`"))),
                }
                Ok(())
            })?;
        }

        if flatten {
            return Self::flatten(f, ident, rust_name, secret || default.is_some() || value_enum
                || keys.is_some() || clearable || one_of.is_some() || requires_if.is_some());
        }
        let shape = classify(&f.ty, value_enum).ok_or_else(|| {
            let ty = &f.ty;
            syn::Error::new_spanned(
                ty,
                format!(
                    "unsupported OpParams field `{rust_name}` of type `{}`",
                    quote::quote!(#ty).to_string().replace(' ', "")
                ),
            )
        })?;
        if matches!(shape, Shape::Patch(_)) != clearable {
            return Err(syn::Error::new_spanned(
                &f.ty,
                format!(
                    "field `{rust_name}`: `#[op(clearable)]` and a `Patch<T>` field type go together"
                ),
            ));
        }
        if one_of.is_some() && !matches!(shape, Shape::Optional(_)) {
            return Err(syn::Error::new_spanned(
                &f.ty,
                format!("field `{rust_name}`: one_of applies only to an `Option<T>` field"),
            ));
        }
        if requires_if.is_some() && !matches!(shape, Shape::Optional(_)) {
            return Err(syn::Error::new_spanned(
                &f.ty,
                format!("field `{rust_name}`: requires_if applies only to an `Option<T>` field"),
            ));
        }
        if let Some(d) = &default {
            if !matches!(shape, Shape::Plain(_)) {
                return Err(syn::Error::new_spanned(
                    d,
                    format!("field `{rust_name}`: default applies only to a plain scalar field"),
                ));
            }
        }
        if keys.is_some() && !matches!(shape, Shape::Map) {
            return Err(syn::Error::new_spanned(
                &f.ty,
                format!("field `{rust_name}`: keys applies only to BTreeMap<String, String>"),
            ));
        }
        if value_enum && !is_enum_shape(&shape) {
            return Err(syn::Error::new_spanned(
                &f.ty,
                format!("field `{rust_name}`: value_enum on a built-in type"),
            ));
        }

        Ok(Field {
            key: camel(&rust_name),
            flag: rust_name.replace('_', "-"),
            help: doc(f),
            ident,
            rust_name,
            shape,
            default,
            keys,
            secret,
            one_of,
            requires_if,
        })
    }

    fn flatten(
        f: &syn::Field,
        ident: Ident,
        rust_name: String,
        other_attrs: bool,
    ) -> syn::Result<Field> {
        if other_attrs {
            return Err(syn::Error::new_spanned(
                f,
                format!("field `{rust_name}`: flatten takes no other #[op] key"),
            ));
        }
        if !matches!(f.ty, syn::Type::Path(_)) {
            return Err(syn::Error::new_spanned(
                &f.ty,
                format!("field `{rust_name}`: flatten needs a struct type that derives OpParams"),
            ));
        }
        Ok(Field {
            key: camel(&rust_name),
            flag: rust_name.replace('_', "-"),
            help: doc(f),
            ident,
            rust_name,
            shape: Shape::Flatten(f.ty.clone()),
            default: None,
            keys: None,
            secret: false,
            one_of: None,
            requires_if: None,
        })
    }

    /// Whether the field was given, as an expression over `self`.
    pub fn present(&self) -> Option<proc_macro2::TokenStream> {
        use quote::quote;
        let ident = &self.ident;
        Some(match &self.shape {
            Shape::Optional(_) | Shape::TriState => quote!(self.#ident.is_some()),
            Shape::Repeated(_) | Shape::Map => quote!(!self.#ident.is_empty()),
            Shape::Switch => quote!(self.#ident),
            Shape::Patch(_) => {
                quote!(!::std::matches!(self.#ident, ::op_registry::Patch::Keep))
            }
            Shape::Plain(_) | Shape::Flatten(_) => return None,
        })
    }

    pub fn required(&self) -> bool {
        matches!(self.shape, Shape::Plain(_)) && self.default.is_none()
    }
}

fn is_enum_shape(shape: &Shape) -> bool {
    use super::types::Base;
    matches!(
        shape,
        Shape::Plain(Base::Enum(_)) | Shape::Optional(Base::Enum(_)) | Shape::Repeated(Base::Enum(_))
    )
}

fn camel(snake: &str) -> String {
    let mut out = String::new();
    for (i, part) in snake.split('_').filter(|p| !p.is_empty()).enumerate() {
        if i == 0 {
            out.push_str(part);
        } else {
            let mut chars = part.chars();
            if let Some(c) = chars.next() {
                out.extend(c.to_uppercase());
                out.push_str(chars.as_str());
            }
        }
    }
    out
}

fn doc(f: &syn::Field) -> String {
    f.attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .filter_map(|a| match &a.meta {
            syn::Meta::NameValue(v) => match &v.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) => Some(s.value().trim().to_owned()),
                _ => None,
            },
            _ => None,
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Field {
    /// The declared default as an expression of the field's type.
    pub fn default_value(&self) -> Option<proc_macro2::TokenStream> {
        use super::types::Base;
        use quote::quote;
        let e = self.default.as_ref()?;
        let name = &self.rust_name;
        Some(match &self.shape {
            Shape::Plain(Base::Enum(ty)) => quote!({ let d: #ty = #e; d }),
            Shape::Plain(b) => {
                let ty = b.ty();
                quote!(::op_registry::__private::rt::default_of::<#ty>(#name, &(#e)))
            }
            _ => return None,
        })
    }
}
