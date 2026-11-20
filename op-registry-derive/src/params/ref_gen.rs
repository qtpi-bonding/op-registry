//! `ref_slots()`: the refs of a struct, non-subjects first.

use proc_macro2::TokenStream;
use quote::quote;

use super::attrs::Field;
use super::types::Shape;

pub fn ref_slots_fn(fields: &[Field]) -> TokenStream {
    let mut subject_scope = TokenStream::new();
    let mut pushes = Vec::new();
    for f in fields {
        let ident = &f.ident;
        let Some(r) = &f.reference else {
            if matches!(f.shape, Shape::Flatten(_)) {
                pushes.push(quote!(slots.extend(::op_registry::OpParams::ref_slots(&mut self.#ident));));
            }
            continue;
        };
        let (kind, subject, id_only) = (&r.kind, r.subject, r.id_only);
        let scope = if r.subject {
            let pairs = r.scope.iter().map(|name| {
                let sibling = fields
                    .iter()
                    .find(|o| &o.rust_name == name)
                    .map(|o| &o.ident);
                quote!((#name, ::op_registry::__private::rt::ScopeValue::scope_value(&self.#sibling)))
            });
            subject_scope = quote!(let scope = ::op_registry::__private::rt::scope(vec![#(#pairs),*]););
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
        fn ref_slots(&mut self) -> ::std::vec::Vec<::op_registry::RefSlot<'_>> {
            #subject_scope
            let mut slots: ::std::vec::Vec<::op_registry::RefSlot<'_>> = ::std::vec::Vec::new();
            #(#pushes)*
            slots.sort_by_key(|s| s.subject);
            slots
        }
    }
}
