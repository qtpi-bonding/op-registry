//! `add_args()` and `from_matches()`.

use proc_macro2::TokenStream;
use quote::quote;

use super::attrs::Field;
use super::relations::group_ids;
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
pub(crate) fn args(f: &Field) -> Vec<TokenStream> {
    let clap = clap();
    let (id, flag) = (&f.rust_name, &f.flag);
    let help = (!f.help.is_empty()).then(|| {
        let h = &f.help;
        quote!(.help(#h))
    });
    let valued = |base: &Base, action: Option<TokenStream>, required: bool| {
        let ty = base.clap_ty();
        let negative = base
            .allows_negative()
            .then(|| quote!(.allow_negative_numbers(true)));
        let required = required.then(|| quote!(.required(true)));
        let ignore_case =
            (f.ignore_case && matches!(base, Base::Enum(_))).then(|| quote!(.ignore_case(true)));
        quote! {
            #clap::Arg::new(#id).long(#flag) #help #required #action
                .value_parser(#clap::value_parser!(#ty)) #negative #ignore_case
        }
    };
    let append = Some(quote!(.action(#clap::ArgAction::Append)));
    match &f.shape {
        Shape::Flatten(_) => Vec::new(),
        Shape::Patch(b) => {
            let clear_id = format!("clear_{}", f.rust_name);
            let clear_flag = format!("clear-{}", f.flag);
            let clear_help = format!("Remove the stored value of --{flag}.");
            let set = valued(b, None, false);
            vec![
                quote!(#set.conflicts_with(#clear_id)),
                quote! {
                    #clap::Arg::new(#clear_id).long(#clear_flag).help(#clear_help)
                        .action(#clap::ArgAction::SetTrue).conflicts_with(#id)
                },
            ]
        }
        Shape::Plain(b) => vec![valued(b, None, f.required())],
        Shape::Optional(b) => vec![valued(b, None, f.explicit_required)],
        Shape::Repeated(b) => vec![valued(b, append, f.explicit_required)],
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

/// A `Vec<String>` expression: the clap arg ids of every target, then `extra`.
fn target_ids(
    fields: &[Field],
    targets: &[super::attrs::ScopeName],
    extra: &[String],
) -> TokenStream {
    let ids = targets
        .iter()
        .map(|t| super::relations::resolve(fields, t).arg_ids());
    quote! {{
        let mut ids: ::std::vec::Vec<::std::string::String> = ::std::vec::Vec::new();
        #(ids.extend(#ids);)*
        #(ids.push(#extra.to_owned());)*
        ids
    }}
}

pub fn add_args_fn(fields: &[Field]) -> TokenStream {
    let clap = clap();
    let steps = fields.iter().map(|f| match &f.shape {
        Shape::Flatten(ty) => quote!(cmd = <#ty as ::op_registry::OpParams>::add_args(cmd);),
        _ => {
            let conflicts = (!f.conflicts_with.is_empty()).then(|| {
                let ids = target_ids(fields, &f.conflicts_with, &[]);
                quote!(.conflicts_with_all(#ids))
            });
            let own_extra: Vec<String> = group_ids(f).into_iter().skip(1).collect();
            let unless = (!f.required_unless.is_empty()).then(|| {
                let ids = target_ids(fields, &f.required_unless, &own_extra);
                quote!(.required_unless_present_any(#ids))
            });
            let args = args(f).into_iter().enumerate().map(|(i, arg)| {
                let unless = if i == 0 { unless.clone() } else { None };
                quote!(cmd = cmd.arg(#arg #conflicts #unless);)
            });
            quote!(#(#args)*)
        }
    });
    // A required tri-state: one of `--x` / `--no-x`.
    let required_tri = fields
        .iter()
        .filter(|f| f.explicit_required && matches!(f.shape, Shape::TriState))
        .map(|f| {
            let name = format!("{}:required", f.rust_name);
            let ids = group_ids(f);
            quote! {
                cmd = cmd.group(
                    #clap::ArgGroup::new(#name).args([#(#ids),*]).required(true).multiple(false),
                );
            }
        });
    let at_least_one = super::relations::at_least_one_groups(fields)
        .into_iter()
        .map(|(name, members)| {
            let ids = members.iter().flat_map(|f| group_ids(f));
            quote! {
                cmd = cmd.group(
                    #clap::ArgGroup::new(#name).args([#(#ids),*]).required(true).multiple(true),
                );
            }
        });
    let groups = super::relations::one_of_groups(fields).into_iter().map(|(name, optional, members)| {
        let ids = members.iter().flat_map(|f| group_ids(f));
        let required = !optional;
        quote! {
            cmd = cmd.group(
                #clap::ArgGroup::new(#name).args([#(#ids),*]).required(#required).multiple(false),
            );
        }
    });
    quote! {
        fn add_args(mut cmd: #clap::Command) -> #clap::Command {
            #(#steps)*
            #(#groups)*
            #(#required_tri)*
            #(#at_least_one)*
            cmd
        }
    }
}

pub(crate) fn decode(f: &Field) -> TokenStream {
    let rt = quote!(::op_registry::__private::rt);
    let (id, key, flag) = (&f.rust_name, &f.key, &f.flag);
    let get_one = |b: &Base| {
        let ty = b.clap_ty();
        match b {
            Base::Json => quote! {
                m.get_one::<#ty>(#id)
                    .map(|s| #rt::json_arg(_cx, #flag, s))
                    .transpose()?
            },
            Base::Input => quote! {
                m.get_one::<#ty>(#id)
                    .map(|s| #rt::input_arg(_cx, #flag, s))
                    .transpose()?
            },
            Base::SecretFile => {
                let hide = f.secret;
                quote! {
                    m.get_one::<#ty>(#id)
                        .map(|s| #rt::secret_file_arg(_cx, #flag, s, #hide))
                        .transpose()?
                }
            }
            Base::Ref => quote!(m.get_one::<#ty>(#id).cloned().map(::op_registry::Ref::new)),
            Base::FromStr(t) => {
                let hide = f.secret;
                quote! {
                    m.get_one::<#ty>(#id)
                        .map(|s| #rt::from_str_arg::<#t>(#flag, s, #hide))
                        .transpose()?
                }
            }
            _ => quote!(m.get_one::<#ty>(#id).cloned()),
        }
    };
    match &f.shape {
        Shape::Flatten(ty) => quote!(<#ty as ::op_registry::OpParams>::from_matches(m, _cx)?),
        Shape::Patch(b) => {
            let clear_id = format!("clear_{}", f.rust_name);
            let get = get_one(b);
            quote! {
                if m.get_flag(#clear_id) {
                    ::op_registry::Patch::Clear
                } else {
                    match #get {
                        ::std::option::Option::Some(v) => ::op_registry::Patch::Set(v),
                        ::std::option::Option::None => ::op_registry::Patch::Keep,
                    }
                }
            }
        }
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
        Shape::Repeated(Base::Json) => quote! {
            m.get_many::<::std::string::String>(#id)
                .into_iter()
                .flatten()
                .map(|s| #rt::json_arg(_cx, #flag, s))
                .collect::<::op_registry::__private::anyhow::Result<::std::vec::Vec<_>>>()?
        },
        Shape::Repeated(Base::Ref) => quote! {
            m.get_many::<::std::string::String>(#id)
                .into_iter()
                .flatten()
                .cloned()
                .map(::op_registry::Ref::new)
                .collect::<::std::vec::Vec<_>>()
        },
        Shape::Repeated(Base::FromStr(t)) => {
            let hide = f.secret;
            quote! {
                m.get_many::<::std::string::String>(#id)
                    .into_iter()
                    .flatten()
                    .map(|s| #rt::from_str_arg::<#t>(#flag, s, #hide))
                    .collect::<::op_registry::__private::anyhow::Result<::std::vec::Vec<_>>>()?
            }
        }
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
            let this = Self { #(#idents),* };
            ::op_registry::OpParams::validate(&this)?;
            ::std::result::Result::Ok(this)
        }
    }
}
