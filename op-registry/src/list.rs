//! List pairs: one page function becomes a follow-all op and a paged op.

use crate::registry::{words, TypedUnary};
use crate::{
    derive_name, CallOptions, FieldKind, FieldMeta, OpKind, OpParams, OpSpec, ParseCx, RefSlot,
    Registry, Resolve,
};
use anyhow::Result;
use coded_error::{CodedError, ErrorClass};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::Any;
use std::collections::HashSet;
use std::sync::Arc;

/// What a list op's page function is asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageRequest {
    /// The most items to return.
    pub limit: u32,
    /// Where to resume, from an earlier page's `next_cursor`.
    pub cursor: Option<String>,
}

/// One page of a list.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    /// The items of the page; always written, empty or not.
    pub items: Vec<T>,
    /// Where the next page starts, or `None` (`null`) on the last page.
    pub next_cursor: Option<String>,
}

/// A follow-all list passed its cap.
#[derive(Debug, thiserror::Error)]
#[error("{op} has more than {cap} items; page with --limit/--cursor")]
pub struct TooMany {
    code: &'static str,
    cap: usize,
    op: String,
}

impl CodedError for TooMany {
    fn code(&self) -> &'static str {
        self.code
    }
    fn class(&self) -> ErrorClass {
        ErrorClass::Logical
    }
}

/// The item type of a list: serialisable, and with a schema when the
/// `schema` feature is on (so `Vec<T>` and `Page<T>` have one too).
#[cfg(feature = "schema")]
pub trait ListItem: Serialize + schemars::JsonSchema {}
#[cfg(feature = "schema")]
impl<T: Serialize + schemars::JsonSchema> ListItem for T {}
/// The item type of a list: serialisable, and with a schema when the
/// `schema` feature is on (so `Vec<T>` and `Page<T>` have one too).
#[cfg(not(feature = "schema"))]
pub trait ListItem: Serialize {}
#[cfg(not(feature = "schema"))]
impl<T: Serialize> ListItem for T {}

/// The parameters of a `_page` op: the list's own, plus `limit` and `cursor`.
#[derive(Clone, Debug, PartialEq)]
pub struct PageParams<P> {
    /// The list's own parameters.
    pub inner: P,
    /// The most items to return.
    pub limit: u32,
    /// Where to resume.
    pub cursor: Option<String>,
}

const LIMIT_HELP: &str = "The most items to return.";
const CURSOR_HELP: &str = "Where to resume, from an earlier page's nextCursor.";

fn paging_field(name: &'static str, required: bool, help: &str) -> FieldMeta {
    FieldMeta {
        rust_name: name,
        flag: name.to_owned(),
        key: name.to_owned(),
        help: help.to_owned(),
        kind: FieldKind::Scalar,
        required,
        secret: false,
        from_group: None,
    }
}

impl<P: OpParams> OpParams for PageParams<P> {
    fn fields() -> Vec<FieldMeta> {
        let mut fields = P::fields();
        fields.push(paging_field("limit", true, LIMIT_HELP));
        fields.push(paging_field("cursor", false, CURSOR_HELP));
        fields
    }

    fn input_schema() -> Value {
        let mut schema = P::input_schema();
        schema["properties"]["limit"] =
            serde_json::json!({ "type": "integer", "minimum": 0, "description": LIMIT_HELP });
        schema["properties"]["cursor"] =
            serde_json::json!({ "type": "string", "description": CURSOR_HELP });
        match schema.get_mut("required").and_then(Value::as_array_mut) {
            Some(required) => required.push(Value::String("limit".to_owned())),
            None => schema["required"] = serde_json::json!(["limit"]),
        }
        schema
    }

    fn add_args(cmd: clap::Command) -> clap::Command {
        P::add_args(cmd)
            .arg(
                clap::Arg::new("limit")
                    .long("limit")
                    .help(LIMIT_HELP)
                    .value_parser(clap::value_parser!(u32)),
            )
            .arg(clap::Arg::new("cursor").long("cursor").help(CURSOR_HELP))
    }

    fn from_matches(m: &clap::ArgMatches, cx: &mut ParseCx) -> Result<Self> {
        let limit = crate::__private::rt::require(m.get_one::<u32>("limit").copied(), "limit")?;
        let cursor = m.get_one::<String>("cursor").cloned();
        Ok(Self {
            inner: P::from_matches(m, cx)?,
            limit,
            cursor,
        })
    }

    fn from_json(v: Value, cx: &mut ParseCx) -> Result<Self> {
        use crate::__private::rt;
        // Not an object: P's decoder gives the "expected object" error.
        let Value::Object(mut object) = v else {
            return Err(match P::from_json(v, cx) {
                Err(err) => err,
                Ok(_) => crate::usage(crate::UsageError::InvalidField {
                    name: "params".to_owned(),
                    reason: "expected object".to_owned(),
                }),
            });
        };
        let limit_value = object.remove("limit");
        let cursor_value = object.remove("cursor");
        let mut paging = serde_json::Map::new();
        if let Some(v) = limit_value {
            paging.insert("limit".to_owned(), v);
        }
        if let Some(v) = cursor_value {
            paging.insert("cursor".to_owned(), v);
        }
        let limit = rt::require(rt::get::<u32>(&paging, "limit")?, "limit")?;
        let cursor = rt::get::<String>(&paging, "cursor")?;
        Ok(Self {
            inner: P::from_json(Value::Object(object), cx)?,
            limit,
            cursor,
        })
    }

    fn validate(&self) -> Result<()> {
        self.inner.validate()
    }

    fn ref_slots(&mut self) -> Vec<RefSlot<'_>> {
        self.inner.ref_slots()
    }

    fn secrets(&self) -> Vec<Vec<u8>> {
        self.inner.secrets()
    }
}

fn coerce<C, P, O>(
    f: impl for<'a> Fn(&'a C, P, CallOptions) -> BoxFuture<'a, Result<O>> + Send + Sync + 'static,
) -> TypedUnary<C, P, O> {
    Arc::new(f)
}

/// Follows pages of `limit` items until the cursor ends, up to `cap` items.
async fn follow_all<C, P, T, F>(
    f: &F,
    ctx: &C,
    params: P,
    limit: u32,
    cap: usize,
    too_many: (&'static str, &str),
) -> Result<Vec<T>>
where
    P: Clone,
    F: for<'a> Fn(&'a C, P, PageRequest) -> BoxFuture<'a, Result<Page<T>>>,
{
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    loop {
        let page = f(ctx, params.clone(), PageRequest { limit, cursor }).await?;
        items.extend(page.items);
        if items.len() > cap {
            return Err(coded_error::into_anyhow(TooMany {
                code: too_many.0,
                cap,
                op: too_many.1.to_owned(),
            }));
        }
        match page.next_cursor {
            Some(next) if !next.is_empty() => {
                if !seen.insert(next.clone()) {
                    return Err(anyhow::anyhow!("page cursor repeated"));
                }
                cursor = Some(next);
            }
            _ => return Ok(items),
        }
    }
}

impl<C: Resolve + 'static> Registry<C> {
    /// Adds a list pair from one page function: `<name>` follows every page
    /// and returns a bare array (failing with `<pillar>.<noun>.too_many` past
    /// `spec.cap` items), and `<name>_page` returns one `Page<T>`. Both
    /// share the spec's metadata and name each other in `list_pair`.
    ///
    /// # Panics
    ///
    /// Panics when the path has no noun word ("list op needs a noun"), when
    /// no pillar is set ("set_pillar before push_list"), and on the build-time
    /// checks of [`Registry::push_op`].
    pub fn push_list<P, T, F>(&mut self, spec: OpSpec<P, T>, f: F)
    where
        P: OpParams + Clone,
        T: ListItem + Send + 'static,
        F: for<'a> Fn(&'a C, P, PageRequest) -> BoxFuture<'a, Result<Page<T>>>
            + Send
            + Sync
            + 'static,
    {
        let path = words(spec.path);
        assert!(
            path.len() >= 2,
            "list op needs a noun (path {:?})",
            spec.path
        );
        assert!(!self.pillar().is_empty(), "set_pillar before push_list");
        let name = derive_name(&path);
        let noun = path[path.len() - 2].replace('-', "_");
        let code: &'static str =
            Box::leak(format!("{}.{noun}.too_many", self.pillar()).into_boxed_str());

        let f = Arc::new(f);
        let (cap, page_size) = (spec.cap, spec.page_size);

        let all_spec = spec.respec::<P, Vec<T>>(spec.fields);
        let all_name = name.clone();
        let all_f = f.clone();
        let all = coerce(move |ctx: &C, params: P, _opts| {
            let f = all_f.clone();
            let op = all_name.clone();
            Box::pin(async move { follow_all(&*f, ctx, params, page_size, cap, (code, &op)).await })
        });
        let mut all_entry = Self::unary_entry(&all_spec, OpKind::ListAll, all);
        all_entry.descriptor.list_pair = Some(format!("{name}_page"));

        let mut page_spec = spec.respec::<PageParams<P>, Page<T>>(None);
        page_spec.group_default = false;
        let page_f = f.clone();
        let page = coerce(move |ctx: &C, params: PageParams<P>, _opts| {
            page_f(
                ctx,
                params.inner,
                PageRequest {
                    limit: params.limit,
                    cursor: params.cursor,
                },
            )
        });
        let mut page_entry = Self::unary_entry(&page_spec, OpKind::ListPage, page);
        page_entry.descriptor.name = format!("{name}_page");
        page_entry.descriptor.list_pair = Some(name);
        page_entry.fields = spec.fields.map(|fields| {
            Box::new(move |params: &(dyn Any + Send)| {
                fields(
                    &params
                        .downcast_ref::<PageParams<P>>()
                        .expect("the entry's fields function and parameters share one type")
                        .inner,
                )
            }) as _
        });

        self.push_entry(all_entry);
        self.push_entry(page_entry);
    }
}
