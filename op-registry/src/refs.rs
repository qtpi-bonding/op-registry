//! References to other objects, and the resolver that turns them into ids.

use futures_util::future::BoxFuture;
use std::collections::BTreeMap;

/// A reference to an object, given as text (an id or a name).
///
/// The library resolves it before the op runs, so inside an op
/// [`Ref::resolved`] and [`Ref::id`] always work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    raw: String,
    resolved: Option<Resolved>,
}

/// What a ref resolved to.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// The object's id.
    Id(String),
    /// The raw text, passed through for the server to resolve (a `resolve: server`
    /// kind).
    Raw(String),
}

impl Ref {
    /// An unresolved ref holding `raw`.
    pub fn new(raw: impl Into<String>) -> Self {
        Self {
            raw: raw.into(),
            resolved: None,
        }
    }

    /// The text as given.
    pub fn as_raw(&self) -> &str {
        &self.raw
    }

    /// The resolution.
    ///
    /// # Panics
    ///
    /// Panics with "ref not resolved" when read before resolution, which
    /// never happens inside an op.
    pub fn resolved(&self) -> &Resolved {
        self.resolved.as_ref().expect("ref not resolved")
    }

    /// The resolved id (or the raw text of a pass-through ref).
    ///
    /// # Panics
    ///
    /// Panics like [`Ref::resolved`].
    pub fn id(&self) -> &str {
        match self.resolved() {
            Resolved::Id(s) | Resolved::Raw(s) => s,
        }
    }

    pub(crate) fn resolved_opt(&self) -> Option<&Resolved> {
        self.resolved.as_ref()
    }

    pub(crate) fn set_resolved(&mut self, resolved: Resolved) {
        self.resolved = Some(resolved);
    }
}

impl serde::Serialize for Ref {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.raw)
    }
}

impl<'de> serde::Deserialize<'de> for Ref {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Ref::new)
    }
}

/// The field values a resolver needs, by camelCase field key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope(BTreeMap<String, String>);

impl Scope {
    /// The value of the named sibling, if it was given.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    pub(crate) fn merged(&self, over: &Scope) -> Scope {
        let mut map = self.0.clone();
        map.extend(over.0.iter().map(|(k, v)| (k.clone(), v.clone())));
        Scope(map)
    }

    pub(crate) fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self(pairs.into_iter().collect())
    }
}

/// The context's way to turn a ref into an object id.
pub trait Resolve: Send + Sync {
    /// Resolves `r` of `kind`. An unknown ref is the app's coded
    /// `<pillar>.<kind>.not_found`, a ref matching several objects
    /// `.ambiguous`.
    ///
    /// The default is an uncoded error ("context has no resolver for kind");
    /// the not-found test flags it, so an app with refs cannot forget this.
    fn resolve<'a>(
        &'a self,
        kind: &'a str,
        r: &'a Ref,
        scope: &'a Scope,
    ) -> BoxFuture<'a, anyhow::Result<Resolved>> {
        let _ = (r, scope);
        Box::pin(async move { Err(anyhow::anyhow!("context has no resolver for kind `{kind}`")) })
    }
}

/// One ref of a parameter struct, as the derive hands it to the pipeline.
pub struct RefSlot<'a> {
    /// The object kind.
    pub kind: &'static str,
    /// Whether this is the op's subject (`--id`).
    pub subject: bool,
    /// Whether the slot takes ids only and skips name resolution.
    pub id_only: bool,
    /// The sibling values the resolver may need.
    pub scope: Scope,
    /// The ref to fill.
    pub slot: &'a mut Ref,
}
