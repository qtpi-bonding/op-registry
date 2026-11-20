//! Classifying a field type into the one shape the generators work from.

use syn::{GenericArgument, PathArguments, Type};

/// The element type of a field.
#[derive(Clone)]
pub enum Base {
    String,
    Path,
    /// An integer type; `signed` allows negative values.
    Int {
        ty: Type,
        signed: bool,
    },
    Float,
    Bool,
    /// An `#[op(value_enum)]` type.
    Enum(Type),
    /// `op_registry::Json`: any JSON value.
    Json,
    /// `op_registry::Input`: bytes from a path or stdin.
    Input,
    /// `op_registry::Ref`: a reference resolved before the op runs.
    Ref,
    /// `op_registry::SecretFile`: secret bytes from a path or stdin.
    SecretFile,
}

impl Base {
    pub fn ty(&self) -> proc_macro2::TokenStream {
        use quote::quote;
        match self {
            Base::String => quote!(::std::string::String),
            Base::Path => quote!(::std::path::PathBuf),
            Base::Int { ty, .. } | Base::Enum(ty) => quote!(#ty),
            Base::Float => quote!(f64),
            Base::Bool => quote!(bool),
            Base::Json => quote!(::op_registry::Json),
            Base::Input => quote!(::op_registry::Input),
            Base::Ref => quote!(::op_registry::Ref),
            Base::SecretFile => quote!(::op_registry::SecretFile),
        }
    }

    /// The type clap parses: `Json` and `Input` arrive as text and are
    /// resolved (file, stdin) at decode time.
    pub fn clap_ty(&self) -> proc_macro2::TokenStream {
        match self {
            Base::Json | Base::Input | Base::Ref | Base::SecretFile => {
                quote::quote!(::std::string::String)
            }
            b => b.ty(),
        }
    }

    pub fn allows_negative(&self) -> bool {
        matches!(self, Base::Int { signed: true, .. } | Base::Float)
    }
}

/// How the field is wrapped.
#[derive(Clone)]
pub enum Shape {
    Plain(Base),
    Optional(Base),
    Repeated(Base),
    Switch,
    TriState,
    Map,
    /// A clearable `Patch<T>`.
    Patch(Base),
    /// A flattened `OpParams` group.
    Flatten(Type),
}

/// Splits `Wrapper<T>` into the wrapper's name and `T`.
fn wrapped(ty: &Type) -> Option<(String, &Type)> {
    let Type::Path(p) = ty else { return None };
    let seg = p.path.segments.last()?;
    let PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    let mut types = args.args.iter().filter_map(|a| match a {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    });
    let first = types.next()?;
    if seg.ident == "BTreeMap" {
        return Some((seg.ident.to_string(), ty));
    }
    if types.next().is_some() {
        return None;
    }
    Some((seg.ident.to_string(), first))
}

fn name_of(ty: &Type) -> String {
    quote::quote!(#ty).to_string().replace(' ', "")
}

fn base(ty: &Type, value_enum: bool) -> Option<Base> {
    let name = name_of(ty);
    let name = name.strip_prefix("std::path::").unwrap_or(&name);
    let name = name.strip_prefix("std::string::").unwrap_or(name);
    let name = name.strip_prefix("op_registry::").unwrap_or(name);
    Some(match name {
        "String" => Base::String,
        "PathBuf" => Base::Path,
        "bool" => Base::Bool,
        "f64" => Base::Float,
        "Json" => Base::Json,
        "Input" => Base::Input,
        "Ref" => Base::Ref,
        "SecretFile" => Base::SecretFile,
        "i32" | "i64" => Base::Int {
            ty: ty.clone(),
            signed: true,
        },
        "u8" | "u16" | "u32" | "u64" | "usize" => Base::Int {
            ty: ty.clone(),
            signed: false,
        },
        _ if value_enum => Base::Enum(ty.clone()),
        _ => return None,
    })
}

/// `None` means the type is unsupported.
pub fn classify(ty: &Type, value_enum: bool) -> Option<Shape> {
    if let Some((wrapper, inner)) = wrapped(ty) {
        match wrapper.as_str() {
            "Option" => {
                return match base(inner, value_enum)? {
                    Base::Bool => Some(Shape::TriState),
                    b => Some(Shape::Optional(b)),
                }
            }
            "Vec" => {
                return match base(inner, value_enum)? {
                    Base::Bool | Base::Input | Base::SecretFile => None,
                    b => Some(Shape::Repeated(b)),
                }
            }
            "Patch" => {
                return match base(inner, value_enum)? {
                    Base::Bool | Base::Json | Base::Input | Base::Ref | Base::SecretFile => None,
                    b => Some(Shape::Patch(b)),
                }
            }
            "BTreeMap" => {
                return (name_of(inner) == "BTreeMap<String,String>").then_some(Shape::Map)
            }
            _ => return None,
        }
    }
    match base(ty, value_enum)? {
        Base::Bool => Some(Shape::Switch),
        b => Some(Shape::Plain(b)),
    }
}
