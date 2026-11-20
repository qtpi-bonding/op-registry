//! `ref_slots()`: the refs of a struct, non-subjects first.

use proc_macro2::TokenStream;
use quote::quote;

use super::attrs::{Field, ScopeName};
use super::types::Shape;

/// `scope(vec![(key, value), ..])?` over the named fields of `self`.
fn scope_expr(names: &[ScopeName]) -> TokenStream {
    let pairs = names.iter().map(|name| {
        let key = name.key();
        let mut access = quote!(self);
        for seg in name.segments() {
            let mut ident = syn::parse_str::<syn::Ident>(seg)
                .unwrap_or_else(|_| syn::Ident::new_raw(seg, name.span));
            ident.set_span(name.span);
            access = quote!(#access.#ident);
        }
        quote!((#key, ::op_registry::__private::rt::ScopeValue::scope_value(&#access)))
    });
    quote!(::op_registry::__private::rt::scope(vec![#(#pairs),*])?)
}

pub fn ref_slots_fn(fields: &[Field]) -> TokenStream {
    let mut subject_scope = TokenStream::new();
    let mut group_scopes = TokenStream::new();
    let mut pushes = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        let ident = &f.ident;
        let Some(r) = &f.reference else {
            if matches!(f.shape, Shape::Flatten(_)) {
                if f.group_scope.is_empty() {
                    pushes.push(
                        quote!(slots.extend(::op_registry::OpParams::ref_slots(&mut self.#ident)?);),
                    );
                } else {
                    let var = quote::format_ident!("group_scope_{i}");
                    let expr = scope_expr(&f.group_scope);
                    group_scopes.extend(quote!(let #var = #expr;));
                    pushes.push(quote! {
                        slots.extend(
                            ::op_registry::OpParams::ref_slots(&mut self.#ident)?
                                .into_iter()
                                .map(|mut s| {
                                    if s.subject {
                                        s.scope = ::op_registry::__private::rt::merge_scope(&s.scope, &#var);
                                    }
                                    s
                                }),
                        );
                    });
                }
            }
            continue;
        };
        let (kind, subject, id_only, absent_ok) = (&r.kind, r.subject, r.id_only, r.absent_ok);
        let scope = if r.subject {
            let expr = scope_expr(&r.scope);
            subject_scope = quote!(let scope = #expr;);
            quote!(scope)
        } else {
            quote!(::op_registry::Scope::default())
        };
        let slot = |target: TokenStream| {
            quote! {
                ::op_registry::RefSlot {
                    kind: #kind,
                    subject: #subject,
                    id_only: #id_only,
                    absent_ok: #absent_ok,
                    scope: #scope.clone(),
                    slot: #target,
                }
            }
        };
        pushes.push(match &f.shape {
            Shape::Plain(_) => {
                let s = slot(quote!(&mut self.#ident));
                quote!(slots.push(#s);)
            }
            Shape::Optional(_) => {
                let s = slot(quote!(r));
                quote!(if let ::std::option::Option::Some(r) = &mut self.#ident { slots.push(#s); })
            }
            _ => {
                let s = slot(quote!(r));
                quote!(for r in self.#ident.iter_mut() { slots.push(#s); })
            }
        });
    }
    quote! {
        fn ref_slots(&mut self) -> ::op_registry::__private::anyhow::Result<::std::vec::Vec<::op_registry::RefSlot<'_>>> {
            #subject_scope
            #group_scopes
            let mut slots: ::std::vec::Vec<::op_registry::RefSlot<'_>> = ::std::vec::Vec::new();
            #(#pushes)*
            slots.sort_by_key(|s| s.subject);
            ::std::result::Result::Ok(slots)
        }
    }
}
