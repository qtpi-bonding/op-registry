//! Expansion of the checked `ops!` model into `registry()` and `ops`.

use super::parse::{Entry, Kind, Ops};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::LitStr;

pub fn expand(ops: &Ops) -> TokenStream {
    let ctx = &ops.ctx;
    let pillar = ops.pillar.as_ref().map(|p| quote!(__r.set_pillar(#p);));
    let bin = ops.bin.as_ref().map(|b| {
        let (bin, version, about) = (&b.bin, &b.version, &b.about);
        quote!(__r.set_bin(#bin, #version, #about);)
    });
    let servers = ops
        .resolve_server
        .iter()
        .map(|k| quote!(__r.resolve_server(#k);));
    let groups = ops.groups.iter().map(|(p, s)| quote!(__r.group(#p, #s);));
    let pushes = ops.entries.iter().map(|e| push(ctx, e));
    let globals = ops.globals.as_ref().map(|g| {
        // Fails to compile unless the context's `CliContext::Globals` is `g`.
        quote! {
            const _: () = {
                fn __check_globals<C: ::op_registry::CliContext<Globals = #g>>() {}
                let _ = __check_globals::<#ctx>;
            };
        }
    });
    let markers = ops.entries.iter().enumerate().map(|(i, e)| markers(i, e));
    let aliases = ops.entries.iter().enumerate().map(|(i, e)| aliases(i, e));

    quote! {
        /// The registry of this binary's operations.
        pub fn registry() -> ::op_registry::Registry<#ctx> {
            let mut __r = ::op_registry::Registry::<#ctx>::new();
            #pillar
            #bin
            #(#servers)*
            #(#groups)*
            #(#pushes)*
            __r
        }

        // The user's types are named here, outside `mod ops`, so a marker
        // named like one of them cannot shadow it.
        #[doc(hidden)]
        type __OpCtx = #ctx;
        #globals
        #(#aliases)*

        /// One marker type per operation, for `Registry::call_typed`.
        pub mod ops {
            #(#markers)*
        }
    }
}

/// `concept get` becomes `ConceptGet`; `-` and `_` split words too.
fn pascal(path: &str, suffix: &str) -> syn::Ident {
    let name: String = path
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
        .replace('-', "_");
    let mut out = String::new();
    for word in name
        .split('_')
        .chain(suffix.split('_'))
        .filter(|w| !w.is_empty())
    {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    syn::Ident::new(&out, Span::call_site())
}

fn push(ctx: &syn::Type, e: &Entry) -> TokenStream {
    let (path, summary, long_help) = (
        &e.path,
        LitStr::new(&e.summary, e.path.span()),
        &e.long_help,
    );
    let (params, output, func) = (&e.params, &e.output, &e.func);
    let audience = match e.audience.to_string().as_str() {
        "agent" => quote!(Agent),
        "owner" => quote!(Owner),
        _ => quote!(Dev),
    };
    let words = e
        .access
        .iter()
        .map(|w| LitStr::new(&w.to_string(), w.span()));
    let mut sets = vec![
        quote!(__spec.long_help = #long_help;),
        quote!(__spec.access = ::op_registry::Access::from_words(&[#(#words),*]);),
        quote!(__spec.audience = ::op_registry::Audience::#audience;),
    ];
    let mut set = |field: &str, value: &Option<syn::LitBool>| {
        if let Some(v) = value {
            let field = format_ident!("{field}");
            sets.push(quote!(__spec.#field = #v;));
        }
    };
    set("hidden", &e.hidden);
    set("group_default", &e.default);
    set("cursor", &e.cursor);
    set("unpaged", &e.unpaged);
    if let Some(cap) = &e.cap {
        sets.push(quote!(__spec.cap = #cap;));
    }
    if let Some(n) = &e.page_size {
        sets.push(quote!(__spec.page_size = #n;));
    }
    if let Some(n) = &e.follow_page {
        sets.push(quote!(__spec.follow_page = ::core::option::Option::Some(#n);));
    }
    if let Some(words) = &e.paths {
        let surfaces = words.iter().map(|w| {
            let v = match w.to_string().as_str() {
                "cli" => "Cli",
                "typed" => "Typed",
                "mcp" => "Mcp",
                _ => "Json",
            };
            let v = syn::Ident::new(v, w.span());
            quote!(::op_registry::Surface::#v)
        });
        sets.push(quote!(__spec.paths = ::op_registry::Paths::of(&[#(#surfaces),*]);));
    }
    if let Some(fields) = &e.fields {
        sets.push(quote!(__spec.fields = Some(#fields);));
    }
    if !e.exits.is_empty() {
        let exits = e.exits.iter().map(|(c, m)| quote!((#c, #m)));
        sets.push(quote!(__spec.exits = vec![#(#exits),*];));
        sets.push(quote! {
            __spec.ok_exit = Some(|o: &#output| <#output as ::op_registry::OkExit>::exit_code(o));
        });
    }
    if e.text.as_ref().is_some_and(|t| t.value) {
        sets.push(quote! {
            __spec.render_text =
                Some(|o: &#output| <#output as ::op_registry::RenderText>::render_text(o));
        });
    }
    if !e.errors.is_empty() {
        let errors = e
            .errors
            .iter()
            .map(|t| quote!(<#t as ::op_registry::__private::coded_error::CodeCatalog>::codes()));
        sets.push(quote!(__spec.errors = [#(#errors),*].concat();));
    }

    let future = quote!(::op_registry::__private::futures_util::future::BoxFuture);
    let anyhow = quote!(::op_registry::__private::anyhow);
    let (push, call) = match e.kind {
        Kind::Op | Kind::Local => {
            let method = format_ident!(
                "{}",
                if e.kind == Kind::Op {
                    "push_op"
                } else {
                    "push_local"
                }
            );
            (
                method,
                quote! {
                    |__c: &#ctx, __p: #params, __o: ::op_registry::CallOptions|
                        -> #future<'_, #anyhow::Result<#output>> {
                        ::std::boxed::Box::pin(#func(__c, __p, __o))
                    }
                },
            )
        }
        Kind::Stream => (format_ident!("push_stream"), quote!(#func)),
        Kind::List => (
            format_ident!("push_list"),
            quote! {
                |__c: &#ctx, __p: #params, __r: ::op_registry::PageRequest|
                    -> #future<'_, #anyhow::Result<::op_registry::Page<#output>>> {
                    ::std::boxed::Box::pin(#func(__c, __p, __r))
                }
            },
        ),
    };
    quote! {
        {
            let mut __spec = ::op_registry::OpSpec::<#params, #output>::new(#path, #summary);
            #(#sets)*
            __r.#push(__spec, #call);
        }
    }
}

fn alias_names(i: usize) -> (syn::Ident, syn::Ident) {
    (
        format_ident!("__OpParams{i}"),
        format_ident!("__OpOutput{i}"),
    )
}

fn aliases(i: usize, e: &Entry) -> TokenStream {
    let (p, o) = alias_names(i);
    let (params, output) = (&e.params, &e.output);
    quote! {
        #[doc(hidden)]
        type #p = #params;
        #[doc(hidden)]
        type #o = #output;
    }
}

fn markers(i: usize, e: &Entry) -> TokenStream {
    let path = e.path.value();
    let (p, o) = alias_names(i);
    let ctx = quote!(super::__OpCtx);
    let (params, output) = (quote!(super::#p), quote!(super::#o));
    let one = |ident: syn::Ident, name: String, params: TokenStream, output: TokenStream| {
        quote! {
            pub struct #ident;
            impl ::op_registry::Op for #ident {
                type Ctx = #ctx;
                type Params = #params;
                type Output = #output;
                const NAME: &'static str = #name;
            }
        }
    };
    let name: String = path
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
        .replace('-', "_");
    match e.kind {
        Kind::List => {
            let list = one(
                pascal(&path, ""),
                name.clone(),
                quote!(#params),
                quote!(::std::vec::Vec<#output>),
            );
            let page = one(
                pascal(&path, "page"),
                format!("{name}_page"),
                quote!(::op_registry::PageParams<#params>),
                quote!(::op_registry::Page<#output>),
            );
            quote!(#list #page)
        }
        _ => one(pascal(&path, ""), name, quote!(#params), quote!(#output)),
    }
}
