//! `add_args()` and `from_matches()`.

use proc_macro2::TokenStream;
use quote::quote;

use super::attrs::Field;
use super::types::{Base, Shape};

fn clap() -> TokenStream {
    quote!(::op_registry::__private::clap)
}

fn keys_expr(f: &Field) -> TokenStream {
    match &f.keys {
        Some(keys) => quote!(::std::option::Option::Some(&[#(#keys),*][..])),
        None => quote!(::std::option::Option::None::<&[&str]>),
    }
}

/// The clap arguments of one field (two for a tri-state).
fn args(f: &Field) -> Vec<TokenStream> {
    let clap = clap();
    let (id, flag) = (&f.rust_name, &f.flag);
    let help = (!f.help.is_empty()).then(|| {
        let h = &f.help;
        quote!(.help(#h))
    });
    let valued = |base: &Base, action: Option<TokenStream>, required: bool| {
        let ty = base.ty();
        let negative = base
            .allows_negative()
            .then(|| quote!(.allow_negative_numbers(true)));
        let required = required.then(|| quote!(.required(true)));
        quote! {
            #clap::Arg::new(#id).long(#flag) #help #required #action
                .value_parser(#clap::value_parser!(#ty)) #negative
        }
    };
    let append = Some(quote!(.action(#clap::ArgAction::Append)));
    match &f.shape {
        Shape::Plain(b) => vec![valued(b, None, f.required())],
        Shape::Optional(b) => vec![valued(b, None, false)],
        Shape::Repeated(b) => vec![valued(b, append, false)],
        Shape::Switch => vec![quote! {
            #clap::Arg::new(#id).long(#flag) #help.action(#clap::ArgAction::SetTrue)
        }],
        Shape::TriState => {
            let no_id = format!("no_{}", f.rust_name);
            let no_flag = format!("no-{}", f.flag);
            let no_help = format!("The opposite of --{flag}.");
            vec![
                quote! {
                    #clap::Arg::new(#id).long(#flag) #help
                        .action(#clap::ArgAction::SetTrue).conflicts_with(#no_id)
                },
                quote! {
                    #clap::Arg::new(#no_id).long(#no_flag).help(#no_help)
                        .action(#clap::ArgAction::SetTrue).conflicts_with(#id)
                },
            ]
        }
        Shape::Map => vec![quote! {
            #clap::Arg::new(#id).long(#flag) #help
                .action(#clap::ArgAction::Append)
                .value_name("KEY=VALUE")
                .value_parser(#clap::value_parser!(::std::string::String))
        }],
    }
}

pub fn add_args_fn(fields: &[Field]) -> TokenStream {
    let clap = clap();
    let args = fields.iter().flat_map(args);
    quote! {
        fn add_args(cmd: #clap::Command) -> #clap::Command {
            cmd #(.arg(#args))*
        }
    }
}

fn decode(f: &Field) -> TokenStream {
    let rt = quote!(::op_registry::__private::rt);
    let (id, key, flag) = (&f.rust_name, &f.key, &f.flag);
    let get_one = |b: &Base| {
        let ty = b.ty();
        quote!(m.get_one::<#ty>(#id).cloned())
    };
    match &f.shape {
        Shape::Plain(b) => match f.default_value() {
            Some(d) => {
                let get = get_one(b);
                quote!(#get.unwrap_or_else(|| #d))
            }
            None => {
                let get = get_one(b);
                quote!(#rt::require(#get, #key)?)
            }
        },
        Shape::Optional(b) => get_one(b),
        Shape::Repeated(b) => {
            let ty = b.ty();
            quote!(m.get_many::<#ty>(#id).map(|v| v.cloned().collect()).unwrap_or_default())
        }
        Shape::Switch => quote!(m.get_flag(#id)),
        Shape::TriState => {
            let no_id = format!("no_{}", f.rust_name);
            quote! {
                if m.get_flag(#no_id) {
                    ::std::option::Option::Some(false)
                } else if m.get_flag(#id) {
                    ::std::option::Option::Some(true)
                } else {
                    ::std::option::Option::None
                }
            }
        }
        Shape::Map => {
            let keys = keys_expr(f);
            quote!(#rt::map_from_pairs(#flag, m.get_many::<::std::string::String>(#id).into_iter().flatten(), #keys)?)
        }
    }
}

pub fn from_matches_fn(fields: &[Field]) -> TokenStream {
    let clap = clap();
    let lets = fields.iter().map(|f| {
        let (ident, value) = (&f.ident, decode(f));
        quote!(let #ident = #value;)
    });
    let idents = fields.iter().map(|f| &f.ident);
    quote! {
        fn from_matches(
            m: &#clap::ArgMatches,
            _cx: &mut ::op_registry::ParseCx,
        ) -> ::op_registry::__private::anyhow::Result<Self> {
            #(#lets)*
            ::std::result::Result::Ok(Self { #(#idents),* })
        }
    }
}
