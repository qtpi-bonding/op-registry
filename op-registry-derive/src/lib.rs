//! Derive macros for op-registry.

use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{parse_macro_input, Attribute, Data, DeriveInput, Expr, LitStr, Type};

struct Field {
    ident: syn::Ident,
    ty: Type,
    name: String,
    long: String,
    short: Option<char>,
    positional: bool,
    env: Option<String>,
    default: Option<Expr>,
    value_enum: bool,
    delimiter: Option<char>,
    required_unless: Option<String>,
    doc: String,
    schema: Option<syn::Path>,
}

fn meta(attrs: &[Attribute], key: &str) -> Option<syn::Meta> {
    attrs.iter().find_map(|a| {
        if !a.path().is_ident("op") {
            return None;
        }
        a.parse_args_with(Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
            .ok()?
            .into_iter()
            .find(|meta| meta.path().is_ident(key))
    })
}

fn validate_attrs(attrs: &[Attribute]) -> Result<(), syn::Error> {
    const KEYS: &[&str] = &[
        "doc",
        "long",
        "short",
        "positional",
        "env",
        "default",
        "value_enum",
        "value_delimiter",
        "required_unless_present",
        "schema",
    ];
    for attr in attrs.iter().filter(|a| a.path().is_ident("op")) {
        let metas = attr
            .parse_args_with(Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
            .map_err(|e| syn::Error::new_spanned(attr, format!("invalid #[op] attribute: {e}")))?;
        for m in metas {
            let key = m
                .path()
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            if !KEYS.contains(&key.as_str()) {
                return Err(syn::Error::new_spanned(
                    m,
                    format!("unknown #[op] key `{key}`"),
                ));
            }
            let bad = |msg: &str| syn::Error::new_spanned(&m, msg);
            match key.as_str() {
                "positional" | "value_enum" => {
                    if !matches!(m, syn::Meta::Path(_)) {
                        return Err(bad("this #[op] option takes no value"));
                    }
                }
                "long" | "env" | "required_unless_present" | "doc" | "schema" => {
                    if !matches!(&m, syn::Meta::NameValue(v) if matches!(&v.value, Expr::Lit(x) if matches!(x.lit, syn::Lit::Str(_))))
                    {
                        return Err(bad("this #[op] option requires a string literal"));
                    }
                }
                "short" | "value_delimiter" => {
                    if !matches!(&m, syn::Meta::NameValue(v) if matches!(&v.value, Expr::Lit(x) if matches!(x.lit, syn::Lit::Char(_))))
                    {
                        return Err(bad("this #[op] option requires a character literal"));
                    }
                }
                "default" if !matches!(m, syn::Meta::NameValue(_)) => {
                    return Err(bad("default requires a value"));
                }
                _ => {}
            }
        }
    }
    Ok(())
}
fn string_meta(attrs: &[Attribute], key: &str) -> Option<String> {
    match meta(attrs, key) {
        Some(syn::Meta::NameValue(v)) => match v.value {
            Expr::Lit(x) => {
                if let syn::Lit::Str(s) = x.lit {
                    Some(s.value())
                } else {
                    None
                }
            }
            _ => None,
        },
        _ => None,
    }
}
fn char_meta(attrs: &[Attribute], key: &str) -> Option<char> {
    match meta(attrs, key) {
        Some(syn::Meta::NameValue(v)) => match v.value {
            Expr::Lit(x) => {
                if let syn::Lit::Char(c) = x.lit {
                    Some(c.value())
                } else {
                    None
                }
            }
            _ => None,
        },
        _ => None,
    }
}
fn has(attrs: &[Attribute], key: &str) -> bool {
    meta(attrs, key).is_some()
}
fn type_info(ty: &Type) -> (bool, bool, Type) {
    if let Type::Path(p) = ty {
        if let Some(s) = p.path.segments.last() {
            if s.ident == "Option" || s.ident == "Vec" {
                if let syn::PathArguments::AngleBracketed(a) = &s.arguments {
                    if let Some(syn::GenericArgument::Type(t)) = a.args.first() {
                        return (s.ident == "Option", s.ident == "Vec", t.clone());
                    }
                }
            }
        }
    }
    (false, false, ty.clone())
}
fn type_name(t: &Type) -> String {
    quote!(#t).to_string().replace(' ', "")
}

#[proc_macro_derive(OpParams, attributes(op))]
pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let fields = match input.data {
        Data::Struct(s) => match s.fields {
            syn::Fields::Named(n) => n.named,
            _ => {
                return syn::Error::new_spanned(name, "OpParams requires named fields")
                    .to_compile_error()
                    .into()
            }
        },
        _ => {
            return syn::Error::new_spanned(name, "OpParams requires a struct")
                .to_compile_error()
                .into()
        }
    };
    let mut fs = Vec::new();
    for f in fields {
        if let Err(error) = validate_attrs(&f.attrs) {
            return error.to_compile_error().into();
        }
        let ident = f.ident.unwrap();
        let fname = ident.to_string();
        let doc = f
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("doc"))
            .filter_map(|a| {
                if let syn::Meta::NameValue(v) = &a.meta {
                    if let Expr::Lit(x) = &v.value {
                        if let syn::Lit::Str(s) = &x.lit {
                            return Some(s.value().trim().to_string());
                        }
                    }
                }
                None
            })
            .collect::<Vec<_>>()
            .join(" ");
        let doc = string_meta(&f.attrs, "doc").unwrap_or(doc);
        let default = match meta(&f.attrs, "default") {
            Some(syn::Meta::NameValue(value)) => Some(value.value),
            _ => None,
        };
        let schema = match string_meta(&f.attrs, "schema") {
            Some(s) => match syn::parse_str(&s) {
                Ok(path) => Some(path),
                Err(_) => {
                    return syn::Error::new_spanned(&f.ty, format!("unparsable schema path `{s}`"))
                        .to_compile_error()
                        .into()
                }
            },
            None => None,
        };
        let (_, _, inner) = type_info(&f.ty);
        let inner_name = type_name(&inner);
        let supported = matches!(
            inner_name.as_str(),
            "String" | "bool" | "i32" | "i64" | "u16" | "u32" | "u64" | "usize" | "f64"
        ) || has(&f.attrs, "value_enum");
        if !supported {
            return syn::Error::new_spanned(
                &f.ty,
                format!(
                    "unsupported OpParams field `{}` of type `{}`",
                    fname, inner_name
                ),
            )
            .to_compile_error()
            .into();
        }
        let (opt, _, _) = type_info(&f.ty);
        let required_unless = string_meta(&f.attrs, "required_unless_present");
        if required_unless.is_some() && !opt {
            return syn::Error::new_spanned(
                &f.ty,
                format!("field `{fname}` with required_unless_present must be Option<T>"),
            )
            .to_compile_error()
            .into();
        }
        fs.push(Field {
            ident,
            ty: f.ty,
            name: fname.clone(),
            long: string_meta(&f.attrs, "long").unwrap_or_else(|| fname.replace('_', "-")),
            short: char_meta(&f.attrs, "short"),
            positional: has(&f.attrs, "positional"),
            env: string_meta(&f.attrs, "env"),
            default,
            value_enum: has(&f.attrs, "value_enum"),
            delimiter: char_meta(&f.attrs, "value_delimiter"),
            required_unless: string_meta(&f.attrs, "required_unless_present"),
            doc,
            schema,
        });
    }
    let names = fs.iter().map(|f| &f.name).collect::<Vec<_>>();
    let mut props = Vec::new();
    let mut req = Vec::new();
    let mut jsons = Vec::new();
    let mut args = Vec::new();
    let mut gets = Vec::new();
    let mut positional_index = 0usize;
    for f in &fs {
        let id = &f.ident;
        let key = &f.name;
        let (opt, vec, inner) = type_info(&f.ty);
        let tn = type_name(&inner);
        let desc = &f.doc;
        let enum_schema = quote!({let n=<#inner as ::op_registry::__private::clap::ValueEnum>::value_variants().iter().filter_map(|v|v.to_possible_value()).map(|v|v.get_name().to_string()).collect::<Vec<_>>();::op_registry::__private::serde_json::json!({"type":"string","enum":n})});
        let frag = if let Some(p) = &f.schema {
            quote!(#p())
        } else if f.value_enum {
            if vec {
                quote!({let item = #enum_schema; let mut array = ::op_registry::__private::serde_json::json!({"type":"array"}); array["items"] = item; array})
            } else {
                enum_schema.clone()
            }
        } else if vec {
            let item = if tn == "String" {
                quote!(::op_registry::__private::serde_json::json!({"type":"string"}))
            } else if tn == "bool" {
                quote!(::op_registry::__private::serde_json::json!({"type":"boolean"}))
            } else if tn == "f64" {
                quote!(::op_registry::__private::serde_json::json!({"type":"number"}))
            } else {
                quote!(::op_registry::__private::serde_json::json!({"type":"integer"}))
            };
            quote!({ let item = #item; let mut array = ::op_registry::__private::serde_json::json!({"type":"array"}); array["items"] = item; array })
        } else if tn == "String" {
            quote!(::op_registry::__private::serde_json::json!({"type":"string"}))
        } else if tn == "bool" {
            quote!(::op_registry::__private::serde_json::json!({"type":"boolean"}))
        } else if tn == "f64" {
            quote!(::op_registry::__private::serde_json::json!({"type":"number"}))
        } else {
            quote!(::op_registry::__private::serde_json::json!({"type":"integer"}))
        };
        let def=f.default.as_ref().map(|e|quote!(o.insert("default".into(),::op_registry::__private::serde_json::to_value(&(#e)).unwrap());)).unwrap_or_default();
        props.push(quote! { properties.insert(#key.to_string(), { let mut x = #frag; let o = x.as_object_mut().unwrap(); o.insert("description".into(), ::op_registry::__private::serde_json::json!(#desc)); #def x }); });
        if !opt && !vec && tn != "bool" && f.default.is_none() && f.required_unless.is_none() {
            req.push(quote!(#key));
        }
        let invalid_expected = quote!(::op_registry::__private::usage(::op_registry::UsageError::InvalidField{name:#key.to_string(),reason:"expected enum".to_string()}));
        let invalid_enum = quote!(::op_registry::__private::usage(::op_registry::UsageError::InvalidField{name:#key.to_string(),reason:"invalid enum".to_string()}));
        let invalid_json = quote!(::op_registry::__private::usage(::op_registry::UsageError::InvalidField{name:#key.to_string(),reason:e.to_string()}));
        let missing = quote!(::op_registry::__private::usage(::op_registry::UsageError::MissingField{name:#key.to_string()}));
        let decode = if f.value_enum && vec {
            quote!({let values:Vec<String>=::op_registry::__private::serde_json::from_value(v.clone()).map_err(|_|#invalid_expected)?; values.into_iter().map(|s| <#inner as ::op_registry::__private::clap::ValueEnum>::from_str(&s,true).map_err(|_|#invalid_enum)).collect::<::std::result::Result<Vec<_>,_>>()?})
        } else if f.value_enum && opt {
            quote!({let value:Option<String>=::op_registry::__private::serde_json::from_value(v.clone()).map_err(|_|#invalid_expected)?; value.map(|s| <#inner as ::op_registry::__private::clap::ValueEnum>::from_str(&s,true).map_err(|_|#invalid_enum)).transpose()?})
        } else if f.value_enum {
            quote!({let s:String=::op_registry::__private::serde_json::from_value(v.clone()).map_err(|_|#invalid_expected)?;<#inner as ::op_registry::__private::clap::ValueEnum>::from_str(&s,true).map_err(|_|#invalid_enum)?})
        } else {
            quote!(::op_registry::__private::serde_json::from_value(v.clone()).map_err(|e|#invalid_json)?)
        };
        let absent = if opt && f.required_unless.is_none() {
            quote!(None)
        } else if let Some(other) = &f.required_unless {
            quote!(if object.contains_key(#other) { None } else { return Err(#missing) })
        } else if vec {
            quote!(Vec::new())
        } else if let Some(e) = &f.default {
            quote!(::op_registry::__private::serde_json::from_value(::op_registry::__private::serde_json::to_value(&(#e)).unwrap()).unwrap())
        } else {
            quote!(return Err(#missing))
        };
        jsons.push(quote!(let #id=if let Some(v)=object.get(#key){#decode}else{#absent};));
        let mut arg = quote!(::op_registry::__private::clap::Arg::new(#key));
        if f.positional {
            positional_index += 1;
            let index = positional_index;
            arg = quote!(#arg.index(#index));
        } else {
            let l = &f.long;
            arg = quote!(#arg.long(#l));
        }
        if let Some(c) = f.short {
            arg = quote!(#arg.short(#c));
        }
        if let Some(e) = &f.env {
            arg = quote!(#arg.env(#e));
        }
        if let Some(d) = f.delimiter {
            arg = quote!(#arg.value_delimiter(#d));
        }
        if let Some(u) = &f.required_unless {
            let other_is_positional = fs.iter().any(|other| other.name == *u && other.positional);
            if !other_is_positional {
                arg = quote!(#arg.required_unless_present(#u));
            }
        }
        if !opt && !vec && tn != "bool" && f.default.is_none() && f.required_unless.is_none() {
            let conditional_target = fs
                .iter()
                .any(|other| other.required_unless.as_deref() == Some(key));
            if !conditional_target || f.positional {
                arg = quote!(#arg.required(true));
            }
        }
        if f.value_enum {
            arg = quote!(#arg.value_parser(::op_registry::__private::clap::builder::PossibleValuesParser::new(<#inner as ::op_registry::__private::clap::ValueEnum>::value_variants().iter().filter_map(|v|v.to_possible_value()))));
        } else if tn != "String" && tn != "bool" {
            arg = quote!(#arg.value_parser(::op_registry::__private::clap::value_parser!(#inner)));
        }
        if vec {
            arg = quote!(#arg.action(::op_registry::__private::clap::ArgAction::Append));
        } else if tn == "bool" {
            arg = quote!(#arg.action(::op_registry::__private::clap::ArgAction::SetTrue));
        }
        if let Some(e) = &f.default {
            let literal = match e {
                Expr::Lit(value) => match &value.lit {
                    syn::Lit::Str(value) => LitStr::new(&value.value(), value.span()),
                    syn::Lit::Char(value) => LitStr::new(&value.value().to_string(), value.span()),
                    other => LitStr::new(
                        &other.to_token_stream().to_string().replace(' ', ""),
                        other.span(),
                    ),
                },
                Expr::Unary(value) => LitStr::new(
                    &value.to_token_stream().to_string().replace(' ', ""),
                    value.span(),
                ),
                _ => {
                    return syn::Error::new_spanned(e, "default must be a literal")
                        .to_compile_error()
                        .into()
                }
            };
            arg = quote!(#arg.default_value(#literal));
        }
        args.push(quote!(cmd=cmd.arg(#arg);));
        let get = if f.value_enum && vec {
            quote!(matches.get_many::<String>(#key).map(|values| values.map(|value| <#inner as ::op_registry::__private::clap::ValueEnum>::from_str(value, true).map_err(|_| #invalid_enum)).collect::<::std::result::Result<Vec<_>,_>>()).transpose()?.unwrap_or_default())
        } else if f.value_enum && opt {
            quote!(matches.get_one::<String>(#key).map(|value| <#inner as ::op_registry::__private::clap::ValueEnum>::from_str(value, true).map_err(|_| #invalid_enum)).transpose()?)
        } else if f.value_enum {
            quote!(matches.get_one::<String>(#key).cloned().ok_or_else(||#missing).and_then(|value| <#inner as ::op_registry::__private::clap::ValueEnum>::from_str(&value, true).map_err(|_| #invalid_enum))?)
        } else if vec {
            quote!(matches.get_many::<#inner>(#key).map(|x|x.cloned().collect()).unwrap_or_default())
        } else if opt {
            quote!(matches.get_one::<#inner>(#key).cloned())
        } else if tn == "bool" {
            quote!(matches.get_flag(#key))
        } else {
            quote!(matches.get_one::<#inner>(#key).cloned().ok_or_else(||#missing)?)
        };
        gets.push(quote!(let #id=#get;));
    }
    let ids = fs.iter().map(|f| &f.ident).collect::<Vec<_>>();
    quote!{impl ::op_registry::OpParams for #name{fn schema()->::op_registry::__private::serde_json::Value{let mut properties=::std::collections::BTreeMap::new();#(#props)*let required: Vec<&str>=vec![#(#req),*];::op_registry::__private::serde_json::json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})}fn from_json(value: ::op_registry::__private::serde_json::Value)->::op_registry::__private::anyhow::Result<Self>{let object=value.as_object().ok_or_else(||::op_registry::__private::usage(::op_registry::UsageError::InvalidField{name:"params".into(),reason:"expected object".into()}))?;for k in object.keys(){if ![#(#names),*].contains(&k.as_str()){return Err(::op_registry::__private::usage(::op_registry::UsageError::InvalidField{name:k.clone(),reason:"unknown field".into()}));}}#(#jsons)*Ok(Self{#(#ids),*})}fn clap_args(mut cmd: ::op_registry::__private::clap::Command)->::op_registry::__private::clap::Command{#(#args)*cmd}fn from_matches(matches:&::op_registry::__private::clap::ArgMatches)->::op_registry::__private::anyhow::Result<Self>{#(#gets)*Ok(Self{#(#ids),*})}}}.into()
}
