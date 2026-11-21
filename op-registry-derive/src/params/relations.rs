//! Relation targets and groups: what `one_of`, `requires_if`, `conflicts_with`,
//! `required_unless` and `at_least_one` name, and the derive-time checks on them.

use proc_macro2::TokenStream;
use quote::quote;

use super::attrs::{Field, ScopeName};
use super::types::{self, Base, Shape};

/// The groups named by `one_of` / `one_of_optional`: `(name, optional, members)`.
pub fn one_of_groups(fields: &[Field]) -> Vec<(String, bool, Vec<&Field>)> {
    let mut groups: Vec<(String, bool, Vec<&Field>)> = Vec::new();
    for f in fields {
        let Some((name, optional)) = &f.one_of else {
            continue;
        };
        match groups.iter_mut().find(|g| &g.0 == name) {
            Some(g) => g.2.push(f),
            None => groups.push((name.clone(), *optional, vec![f])),
        }
    }
    groups
}

/// The groups named by `at_least_one`: `(name, members)`.
pub fn at_least_one_groups(fields: &[Field]) -> Vec<(String, Vec<&Field>)> {
    let mut groups: Vec<(String, Vec<&Field>)> = Vec::new();
    for f in fields {
        let Some(name) = &f.at_least_one else {
            continue;
        };
        match groups.iter_mut().find(|g| &g.0 == name) {
            Some(g) => g.1.push(f),
            None => groups.push((name.clone(), vec![f])),
        }
    }
    groups
}

/// A relation target: a field of this struct, or a leaf behind a flatten.
pub enum Target<'a> {
    Local(&'a Field),
    Dotted {
        /// The field path from `self` to the leaf.
        path: Vec<syn::Ident>,
        /// The type of the flatten field the path starts at.
        group: &'a syn::Type,
        /// The leaf's rust name.
        leaf: String,
    },
}

/// Resolves a target that `check_relations` already accepted.
pub fn resolve<'a>(fields: &'a [Field], name: &ScopeName) -> Target<'a> {
    let segments = name.segments();
    let head = fields
        .iter()
        .find(|f| f.rust_name == segments[0])
        .expect("relation target checked by the derive");
    if segments.len() == 1 {
        return Target::Local(head);
    }
    let Shape::Flatten(group) = &head.shape else {
        unreachable!("relation target checked by the derive")
    };
    let path = segments
        .iter()
        .map(|seg| {
            // A keyword field is a raw identifier.
            syn::parse_str::<syn::Ident>(seg)
                .unwrap_or_else(|_| syn::Ident::new_raw(seg, name.span))
        })
        .collect();
    Target::Dotted {
        path,
        group,
        leaf: name.leaf().to_owned(),
    }
}

impl Target<'_> {
    /// Whether the target is given, as an expression over `self`.
    pub fn present(&self) -> TokenStream {
        let rt = quote!(::op_registry::__private::rt);
        match self {
            Target::Local(f) => {
                let ident = &f.ident;
                quote!(#rt::Presence::present(&self.#ident))
            }
            Target::Dotted { path, .. } => quote!(#rt::Presence::present(&self.#(#path).*)),
        }
    }

    /// The target's JSON key, as a `String` expression.
    pub fn key(&self) -> TokenStream {
        let rt = quote!(::op_registry::__private::rt);
        match self {
            Target::Local(f) => {
                let key = &f.key;
                quote!(#key.to_owned())
            }
            Target::Dotted { group, leaf, .. } => quote!(#rt::leaf_of::<#group>(#leaf).0),
        }
    }

    /// `(key, kind)` as a `(String, FieldKind)` expression.
    pub fn keyed(&self) -> TokenStream {
        let rt = quote!(::op_registry::__private::rt);
        match self {
            Target::Local(f) => {
                let (key, kind) = (&f.key, kind_name(f));
                quote!((#key.to_owned(), ::op_registry::FieldKind::#kind))
            }
            Target::Dotted { group, leaf, .. } => quote!(#rt::leaf_of::<#group>(#leaf)),
        }
    }

    /// The target's flag with its dashes, as a `String` expression.
    pub fn flag(&self) -> TokenStream {
        let rt = quote!(::op_registry::__private::rt);
        match self {
            Target::Local(f) => {
                let flag = spelled(f);
                quote!(#flag.to_owned())
            }
            Target::Dotted { group, leaf, .. } => quote!(#rt::leaf_flag::<#group>(#leaf)),
        }
    }

    /// The clap arg ids of the target, as a `Vec<String>` expression.
    pub fn arg_ids(&self) -> TokenStream {
        let rt = quote!(::op_registry::__private::rt);
        match self {
            Target::Local(f) => {
                let ids = group_ids(f);
                quote!(::std::vec![#(#ids.to_owned()),*])
            }
            Target::Dotted { group, leaf, .. } => {
                quote!(#rt::arg_ids(#leaf, #rt::leaf_of::<#group>(#leaf).1))
            }
        }
    }
}

/// A field's flag spellings as a relation line names them: a tri-state is
/// `--x or --no-x`, a `Patch` `--x or --clear-x`.
fn spelled(f: &Field) -> String {
    match f.shape {
        Shape::TriState => format!("--{} or --no-{}", f.flag, f.flag),
        Shape::Patch(_) => format!("--{} or --clear-{}", f.flag, f.flag),
        _ => format!("--{}", f.flag),
    }
}

/// `relations()`: one plain line per relation, then each flattened group's.
pub fn relations_fn(fields: &[Field]) -> TokenStream {
    let mut lines: Vec<TokenStream> = Vec::new();
    let flags = |members: &[&Field]| {
        let flags: Vec<String> = members.iter().map(|f| spelled(f)).collect();
        flags.join(", ")
    };
    for (_, optional, members) in one_of_groups(fields) {
        let flags = flags(&members);
        lines.push(if optional {
            quote!(out.push(::std::format!("give at most one of {}", #flags));)
        } else {
            quote!(out.push(::std::format!("give exactly one of {}", #flags));)
        });
    }
    for (_, members) in at_least_one_groups(fields) {
        let flags = flags(&members);
        lines.push(quote!(out.push(::std::format!("give at least one of {}", #flags));))
    }
    for f in fields {
        let own = spelled(f);
        let join = |names: &[ScopeName], word: &str| {
            let flags = names.iter().map(|t| resolve(fields, t).flag());
            quote!(::std::vec![#(#flags),*].join(#word))
        };
        if !f.requires_if.is_empty() {
            let others = join(&f.requires_if, " or ");
            lines.push(
                quote!(out.push(::std::format!("{} is required when {} is given", #own, #others));),
            );
        }
        if !f.conflicts_with.is_empty() {
            let others = join(&f.conflicts_with, ", ");
            lines.push(quote!(out.push(::std::format!("{} conflicts with {}", #own, #others));));
        }
        if !f.required_unless.is_empty() {
            let others = join(&f.required_unless, " or ");
            lines.push(
                quote!(out.push(::std::format!("{} is required unless {} is given", #own, #others));),
            );
        }
    }
    let nested = fields.iter().filter_map(|f| match &f.shape {
        Shape::Flatten(ty) => {
            Some(quote!(out.extend(<#ty as ::op_registry::OpParams>::relations());))
        }
        _ => None,
    });
    quote! {
        fn relations() -> ::std::vec::Vec<::std::string::String> {
            let mut out = ::std::vec::Vec::new();
            #(#lines)*
            #(#nested)*
            out
        }
    }
}

/// `let when = [(String, FieldKind) ...]` as `(&str, FieldKind)` pairs.
pub fn keyed_list(fields: &[Field], names: &[ScopeName]) -> TokenStream {
    let keyed = names.iter().map(|t| resolve(fields, t).keyed());
    quote! {
        let owned = [#(#keyed),*];
        let when: ::std::vec::Vec<(&str, ::op_registry::FieldKind)> =
            owned.iter().map(|t| (t.0.as_str(), t.1)).collect();
    }
}

/// The `FieldKind` variant name of a field (never a flattened group).
pub fn kind_name(f: &Field) -> TokenStream {
    match &f.shape {
        Shape::Plain(Base::Json) | Shape::Optional(Base::Json) => quote!(Json),
        Shape::Plain(Base::Input) | Shape::Optional(Base::Input) => quote!(Input),
        Shape::Plain(Base::SecretFile) | Shape::Optional(Base::SecretFile) => quote!(SecretFile),
        Shape::Plain(Base::Ref) | Shape::Optional(Base::Ref) => quote!(Ref),
        Shape::Plain(Base::Path) | Shape::Optional(Base::Path) => quote!(Path),
        Shape::Plain(_) | Shape::Optional(_) => quote!(Scalar),
        Shape::Patch(_) => quote!(Patch),
        Shape::Switch => quote!(Switch),
        Shape::TriState => quote!(TriState),
        Shape::Repeated(_) => quote!(Repeated),
        Shape::Map => quote!(Map),
        Shape::Flatten(_) => unreachable!("flattened groups have no kind"),
    }
}

/// Checks that every relation names fields the derive can see.
pub(super) fn check_relations(fields: &[Field]) -> syn::Result<()> {
    for (name, optional, members) in one_of_groups(fields) {
        if members.len() < 2 {
            return Err(syn::Error::new(
                members[0].ident.span(),
                format!("one_of group `{name}` needs two members"),
            ));
        }
        if members
            .iter()
            .any(|f| f.one_of.as_ref().map(|g| g.1) != Some(optional))
        {
            return Err(syn::Error::new(
                members[0].ident.span(),
                format!("group `{name}` mixes one_of and one_of_optional"),
            ));
        }
    }
    check_refs(fields)?;
    for (name, members) in at_least_one_groups(fields) {
        if members.len() < 2 {
            return Err(syn::Error::new(
                members[0].ident.span(),
                format!("at_least_one group `{name}` needs two members"),
            ));
        }
        if let Some((_, _, one_of)) = one_of_groups(fields).into_iter().find(|g| g.0 == name) {
            return Err(syn::Error::new(
                one_of[0].ident.span(),
                format!("group `{name}` is used with two kinds (one_of and at_least_one)"),
            ));
        }
    }
    for f in fields {
        for list in [&f.requires_if, &f.conflicts_with, &f.required_unless] {
            for target in list {
                check_target(fields, f, target)?;
            }
        }
        if let Some(both) = f
            .conflicts_with
            .iter()
            .find(|c| f.required_unless.iter().any(|u| u.path == c.path))
        {
            return Err(syn::Error::new(
                both.span,
                format!(
                    "field `{}`: conflicts_with and required_unless both name `{}`",
                    f.rust_name, both.path
                ),
            ));
        }
    }
    Ok(())
}

/// A relation target is another field of this struct that can be absent, or
/// a dotted path starting at one of its flattened groups.
fn check_target(fields: &[Field], f: &Field, target: &ScopeName) -> syn::Result<()> {
    let err = |msg: String| {
        Err(syn::Error::new(
            target.span,
            format!("field `{}`: {msg}", f.rust_name),
        ))
    };
    let path = &target.path;
    let segments = target.segments();
    if segments.iter().any(|s| s.is_empty()) {
        return err(format!("`{path}` has an empty segment"));
    }
    let head = fields.iter().find(|o| o.rust_name == segments[0]);
    if segments.len() > 1 {
        return match head {
            Some(h) if matches!(h.shape, types::Shape::Flatten(_)) => Ok(()),
            _ => err(format!(
                "`{}` in `{path}` is not a #[op(flatten)] field",
                segments[0]
            )),
        };
    }
    match head {
        None => err(format!("unknown target `{path}`")),
        Some(h) if h.rust_name == f.rust_name => {
            err("a relation cannot name the field itself".into())
        }
        Some(h) if h.present().is_none() => err(format!(
            "target `{path}` cannot be absent, so it cannot be a relation target"
        )),
        Some(_) => Ok(()),
    }
}

/// One subject at most, and every scope name is a field of this struct or a
/// dotted path starting at one of its flattened groups.
fn check_refs(fields: &[Field]) -> syn::Result<()> {
    let mut subjects = fields
        .iter()
        .filter(|f| f.reference.as_ref().is_some_and(|r| r.subject));
    subjects.next();
    if let Some(second) = subjects.next() {
        return Err(syn::Error::new(
            second.ident.span(),
            "an op has one subject: only one field takes `#[op(subject = ..)]`",
        ));
    }
    for f in fields {
        if let Some(r) = &f.reference {
            check_scope(fields, f, &r.scope)?;
            if !r.subject {
                check_non_subject_scope(fields, f, &r.scope)?;
            }
        }
        check_scope(fields, f, &f.group_scope)?;
    }
    Ok(())
}

/// A non-subject's scope may not name a ref: non-subjects resolve first, so
/// the ref would still be unresolved.
fn check_non_subject_scope(
    fields: &[Field],
    owner: &Field,
    scope: &[ScopeName],
) -> syn::Result<()> {
    for name in scope {
        let segments = name.segments();
        if segments.len() != 1 {
            continue;
        }
        if fields
            .iter()
            .any(|o| o.rust_name == segments[0] && o.reference.is_some())
        {
            return Err(syn::Error::new(
                name.span,
                format!(
                    "the scope of non-subject ref `{}` names the ref `{}`; refs resolve non-subjects first, so only the subject's scope may name a ref",
                    owner.rust_name, name.path
                ),
            ));
        }
    }
    Ok(())
}

/// Checks one `scope = [..]` list: names resolve, and keys are distinct.
fn check_scope(fields: &[Field], owner: &Field, scope: &[ScopeName]) -> syn::Result<()> {
    let mut keys: Vec<(String, &str)> = Vec::new();
    for name in scope {
        let segments = name.segments();
        if segments.iter().any(|s| s.is_empty()) {
            return Err(syn::Error::new(
                name.span,
                format!("scope names `{}`, which is not a field path", name.path),
            ));
        }
        let head = fields.iter().find(|o| o.rust_name == segments[0]);
        let ok = if segments.len() == 1 {
            head.is_some_and(|o| {
                o.rust_name != owner.rust_name && !matches!(o.shape, types::Shape::Flatten(_))
            })
        } else {
            head.is_some_and(|o| matches!(o.shape, types::Shape::Flatten(_)))
        };
        if !ok {
            let span = if segments.len() == 1 {
                owner.ident.span()
            } else {
                name.span
            };
            let what = if segments.len() == 1 {
                "which is not another field of this struct"
            } else {
                "whose first segment is not a flattened group of this struct"
            };
            return Err(syn::Error::new(
                span,
                format!("scope names `{}`, {what}", name.path),
            ));
        }
        let key = name.key();
        if let Some((_, first)) = keys.iter().find(|(k, _)| *k == key) {
            return Err(syn::Error::new(
                name.span,
                format!(
                    "scope names `{first}` and `{}`, which share the scope key `{key}`",
                    name.path
                ),
            ));
        }
        keys.push((key, &name.path));
    }
    Ok(())
}

/// The clap arg ids of a relation member: a tri-state also has `no_x`, a
/// `Patch` also `clear_x`.
pub(crate) fn group_ids(f: &Field) -> Vec<String> {
    let mut ids = vec![f.rust_name.clone()];
    match f.shape {
        Shape::TriState => ids.push(format!("no_{}", f.rust_name)),
        Shape::Patch(_) => ids.push(format!("clear_{}", f.rust_name)),
        _ => {}
    }
    ids
}
