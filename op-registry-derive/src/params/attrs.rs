//! The one `Field` model: parsed once, read by every generator.

use syn::ext::IdentExt;
use syn::{Expr, ExprArray, ExprLit, Ident, Lit};

use super::types::{classify, Base, Shape};

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
    /// A `SecretFile` that also takes `{"inline": base64}` on JSON paths.
    pub secret_inline: bool,
    /// `(group, optional)` from `one_of` / `one_of_optional`.
    pub one_of: Option<(String, bool)>,
    /// The Rust name of the field this one is required with.
    pub requires_if: Option<String>,
    /// Set on a `Ref` field.
    pub reference: Option<RefInfo>,
}

/// What `subject`, `kind`, `id_only` and `scope` say about a `Ref` field.
pub struct RefInfo {
    pub kind: String,
    pub subject: bool,
    pub id_only: bool,
    pub scope: Vec<String>,
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
        let mut secret_inline = false;
        let mut one_of: Option<(String, bool)> = None;
        let mut requires_if = None;
        let mut clearable = false;
        let mut flatten = false;
        let mut subject: Option<String> = None;
        let mut kind: Option<String> = None;
        let mut id_only = false;
        let mut scope: Option<Vec<String>> = None;
        for attr in f.attrs.iter().filter(|a| a.path().is_ident("op")) {
            attr.parse_nested_meta(|m| {
                let key = m.path.get_ident().map(Ident::to_string).unwrap_or_default();
                match key.as_str() {
                    "default" => default = Some(m.value()?.parse::<Expr>()?),
                    "value_enum" => value_enum = true,
                    "secret" => secret = true,
                    "secret_inline" => secret_inline = true,
                    "clearable" => clearable = true,
                    "flatten" => flatten = true,
                    "id_only" => id_only = true,
                    "subject" => subject = Some(m.value()?.parse::<syn::LitStr>()?.value()),
                    "kind" => kind = Some(m.value()?.parse::<syn::LitStr>()?.value()),
                    "scope" => {
                        let array: ExprArray = m.value()?.parse()?;
                        let mut names = Vec::new();
                        for e in &array.elems {
                            match e {
                                Expr::Lit(ExprLit {
                                    lit: Lit::Str(s), ..
                                }) => names.push(s.value()),
                                _ => {
                                    return Err(syn::Error::new_spanned(
                                        e,
                                        "scope takes string literals",
                                    ))
                                }
                            }
                        }
                        scope = Some(names);
                    }
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
            return Self::flatten(f, ident, rust_name, secret || secret_inline || default.is_some() || value_enum
                || keys.is_some() || clearable || one_of.is_some() || requires_if.is_some()
                || subject.is_some() || kind.is_some() || id_only || scope.is_some());
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
        let is_secret_file = matches!(
            shape,
            Shape::Plain(Base::SecretFile) | Shape::Optional(Base::SecretFile)
        );
        if secret_inline && !is_secret_file {
            return Err(syn::Error::new_spanned(
                &f.ty,
                format!("field `{rust_name}`: secret_inline applies only to a `SecretFile` field"),
            ));
        }
        if secret && is_enum_shape_any(&shape) {
            return Err(syn::Error::new_spanned(
                &f.ty,
                format!("field `{rust_name}`: secret does not apply to a value_enum field"),
            ));
        }
        let reference = ref_info(&f.ty, &rust_name, &shape, subject, kind, id_only, scope, default.is_some())?;
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

        let (key, flag) = match &reference {
            Some(r) if r.subject => ("id".to_owned(), "id".to_owned()),
            _ => (camel(&rust_name), rust_name.replace('_', "-")),
        };
        Ok(Field {
            key,
            flag,
            help: doc(f),
            ident,
            rust_name,
            shape,
            default,
            keys,
            secret,
            secret_inline,
            one_of,
            requires_if,
            reference,
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
            secret_inline: false,
            one_of: None,
            requires_if: None,
            reference: None,
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

/// Checks the ref attributes against the field's type.
#[allow(clippy::too_many_arguments)]
fn ref_info(
    ty: &syn::Type,
    rust_name: &str,
    shape: &Shape,
    subject: Option<String>,
    kind: Option<String>,
    id_only: bool,
    scope: Option<Vec<String>>,
    has_default: bool,
) -> syn::Result<Option<RefInfo>> {
    let is_ref = matches!(
        shape,
        Shape::Plain(Base::Ref) | Shape::Optional(Base::Ref) | Shape::Repeated(Base::Ref)
    );
    let err = |msg: String| Err(syn::Error::new_spanned(ty, format!("field `{rust_name}`: {msg}")));
    if !is_ref {
        if subject.is_some() || kind.is_some() || id_only || scope.is_some() {
            return err("subject, kind, id_only and scope apply only to a `Ref` field".into());
        }
        return Ok(None);
    }
    if has_default {
        return err("a `Ref` field takes no default".into());
    }
    match (subject, kind) {
        (Some(_), Some(_)) => err("subject and kind are exclusive".into()),
        (None, None) => err("a `Ref` field needs `#[op(subject = \"..\")]` or `#[op(kind = \"..\")]`".into()),
        (Some(kind), None) => {
            if !matches!(shape, Shape::Plain(_)) {
                return err("the subject is a plain `Ref`".into());
            }
            Ok(Some(RefInfo { kind, subject: true, id_only, scope: scope.unwrap_or_default() }))
        }
        (None, Some(kind)) => {
            if id_only || scope.is_some() {
                return err("id_only and scope belong to the subject".into());
            }
            Ok(Some(RefInfo { kind, subject: false, id_only: false, scope: Vec::new() }))
        }
    }
}

fn is_enum_shape_any(shape: &Shape) -> bool {
    matches!(
        shape,
        Shape::Plain(Base::Enum(_))
            | Shape::Optional(Base::Enum(_))
            | Shape::Repeated(Base::Enum(_))
            | Shape::Patch(Base::Enum(_))
    )
}

fn is_enum_shape(shape: &Shape) -> bool {
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
