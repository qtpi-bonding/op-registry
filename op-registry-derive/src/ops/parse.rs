//! Parsing of the `ops!` block into a checked model.

use proc_macro2::Span;
use syn::parse::{Parse, ParseStream};
use syn::{
    braced, bracketed, Attribute, Expr, ExprLit, Ident, Lit, LitBool, LitInt, LitStr, Meta, Path,
    Result, Token, Type,
};

pub struct Ops {
    pub ctx: Type,
    pub globals: Option<Type>,
    pub pillar: Option<LitStr>,
    pub bin: Option<Bin>,
    pub resolve_server: Vec<LitStr>,
    pub groups: Vec<(LitStr, LitStr)>,
    pub entries: Vec<Entry>,
}

pub struct Bin {
    pub bin: LitStr,
    pub version: Expr,
    pub about: LitStr,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Op,
    Local,
    Stream,
    List,
}

pub struct Entry {
    pub path: LitStr,
    pub summary: String,
    pub long_help: String,
    pub kind: Kind,
    pub func: Path,
    pub params: Type,
    /// The output type, or the item type of a list.
    pub output: Type,
    pub access: Vec<Ident>,
    pub audience: Ident,
    pub errors: Vec<Type>,
    pub fields: Option<Path>,
    pub exits: Vec<(LitInt, LitStr)>,
    pub text: Option<LitBool>,
    pub hidden: Option<LitBool>,
    pub default: Option<LitBool>,
    pub typed_only: bool,
    pub cursor: Option<LitBool>,
    pub cap: Option<LitInt>,
    pub page_size: Option<LitInt>,
    pub unpaged: Option<LitBool>,
}

const ACCESS_WORDS: [&str; 5] = ["reads", "writes", "network", "spawns", "destructive"];
const AUDIENCES: [&str; 3] = ["agent", "owner", "dev"];

impl Parse for Ops {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut ctx = None;
        let mut globals = None;
        let mut pillar = None;
        let (mut bin, mut version, mut about) = (None, None, None);
        let mut resolve_server = Vec::new();
        let mut groups = Vec::new();
        let header_span = input.span();

        while !input.is_empty() && !input.peek(LitStr) && !input.peek(Token![#]) {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "Ctx" => once(&mut ctx, &key, input.parse::<Type>()?)?,
                "globals" => once(&mut globals, &key, input.parse::<Type>()?)?,
                "pillar" => once(&mut pillar, &key, input.parse::<LitStr>()?)?,
                "bin" => once(&mut bin, &key, input.parse::<LitStr>()?)?,
                "version" => once(&mut version, &key, input.parse::<Expr>()?)?,
                "about" => once(&mut about, &key, input.parse::<LitStr>()?)?,
                "resolve_server" => {
                    let inner;
                    bracketed!(inner in input);
                    resolve_server = inner
                        .parse_terminated(<LitStr as Parse>::parse, Token![,])?
                        .into_iter()
                        .collect();
                }
                "groups" => {
                    let inner;
                    braced!(inner in input);
                    while !inner.is_empty() {
                        let path: LitStr = inner.parse()?;
                        inner.parse::<Token![=>]>()?;
                        let summary: LitStr = inner.parse()?;
                        groups.push((path, summary));
                        if inner.is_empty() {
                            break;
                        }
                        inner.parse::<Token![,]>()?;
                    }
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown header key `{other}`: expected Ctx, globals, pillar, bin, \
                             version, about, resolve_server or groups"
                        ),
                    ))
                }
            }
            input.parse::<Token![;]>()?;
        }

        let ctx = ctx.ok_or_else(|| {
            syn::Error::new(header_span, "missing `Ctx = <type>;` at the top of `ops!`")
        })?;
        let bin = match (bin, version, about) {
            (Some(bin), Some(version), Some(about)) => Some(Bin { bin, version, about }),
            (None, None, None) => None,
            _ => {
                return Err(syn::Error::new(
                    header_span,
                    "`bin`, `version` and `about` go together: give all three or none",
                ))
            }
        };

        let mut entries = Vec::new();
        while !input.is_empty() {
            entries.push(input.parse::<Entry>()?);
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(Ops {
            ctx,
            globals,
            pillar,
            bin,
            resolve_server,
            groups,
            entries,
        })
    }
}

fn once<T>(slot: &mut Option<T>, key: &Ident, value: T) -> Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new(key.span(), format!("`{key}` is given twice")));
    }
    *slot = Some(value);
    Ok(())
}

/// The doc comment as `(summary, long_help)`: the first paragraph is the
/// summary, the rest is the long help.
fn docs(attrs: &[Attribute], at: Span) -> Result<(String, String)> {
    let mut lines = Vec::new();
    for attr in attrs {
        let text = match &attr.meta {
            Meta::NameValue(nv) if nv.path.is_ident("doc") => match &nv.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) => s.value(),
                _ => return Err(syn::Error::new_spanned(attr, "unsupported doc comment")),
            },
            _ => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "only doc comments are allowed on an op entry",
                ))
            }
        };
        lines.push(text.strip_prefix(' ').unwrap_or(&text).trim_end().to_owned());
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let first = lines.iter().position(|l| !l.is_empty()).ok_or_else(|| {
        syn::Error::new(at, "an op needs a doc comment: its first paragraph is the summary")
    })?;
    let lines = &lines[first..];
    let end = lines.iter().position(String::is_empty).unwrap_or(lines.len());
    let summary = lines[..end].join(" ");
    let rest = lines[end..]
        .iter()
        .skip_while(|l| l.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    Ok((summary, rest))
}

impl Parse for Entry {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let path: LitStr = input.parse()?;
        let (summary, long_help) = docs(&attrs, path.span())?;
        input.parse::<Token![=>]>()?;
        let body;
        braced!(body in input);

        let mut params = None;
        let mut output: Option<(Ident, Type)> = None;
        let mut item: Option<(Ident, Type)> = None;
        let mut kinds: Vec<(Ident, Kind, Path)> = Vec::new();
        let mut access: Option<Vec<Ident>> = None;
        let mut audience: Option<Ident> = None;
        let mut errors = Vec::new();
        let mut fields = None;
        let mut exits = Vec::new();
        let mut exits_key: Option<Ident> = None;
        let (mut text, mut hidden, mut default) = (None, None, None);
        let mut text_key: Option<Ident> = None;
        let mut default_key: Option<Ident> = None;
        let mut typed_only = false;
        let (mut cursor, mut cap, mut page_size, mut unpaged) = (None, None, None, None);
        let mut cursor_key = None;
        let mut list_only: Vec<Ident> = Vec::new();
        let mut seen: Vec<String> = Vec::new();

        while !body.is_empty() {
            let key: Ident = body.parse()?;
            if seen.contains(&key.to_string()) {
                return Err(syn::Error::new(key.span(), format!("`{key}` is given twice")));
            }
            seen.push(key.to_string());
            body.parse::<Token![:]>()?;
            match key.to_string().as_str() {
                "params" => params = Some(body.parse::<Type>()?),
                "output" => output = Some((key.clone(), body.parse::<Type>()?)),
                "item" => item = Some((key.clone(), body.parse::<Type>()?)),
                "op" | "local" | "stream" | "list" => {
                    let kind = match key.to_string().as_str() {
                        "op" => Kind::Op,
                        "local" => Kind::Local,
                        "stream" => Kind::Stream,
                        _ => Kind::List,
                    };
                    kinds.push((key.clone(), kind, body.parse::<Path>()?));
                }
                "access" => {
                    let mut words = Vec::new();
                    if body.peek(syn::token::Bracket) {
                        let inner;
                        bracketed!(inner in body);
                        words.extend(inner.parse_terminated(Ident::parse, Token![,])?);
                    } else {
                        words.push(body.parse::<Ident>()?);
                    }
                    for word in &words {
                        if !ACCESS_WORDS.contains(&word.to_string().as_str()) {
                            return Err(syn::Error::new(
                                word.span(),
                                format!("unknown access `{word}`: expected one of {ACCESS_WORDS:?}"),
                            ));
                        }
                    }
                    access = Some(words);
                }
                "audience" => {
                    let word: Ident = body.parse()?;
                    if !AUDIENCES.contains(&word.to_string().as_str()) {
                        return Err(syn::Error::new(
                            word.span(),
                            format!("unknown audience `{word}`: expected agent, owner or dev"),
                        ));
                    }
                    audience = Some(word);
                }
                "errors" => {
                    let inner;
                    bracketed!(inner in body);
                    errors = inner
                        .parse_terminated(Type::parse, Token![,])?
                        .into_iter()
                        .collect();
                }
                "fields" => fields = Some(body.parse::<Path>()?),
                "exits" => {
                    exits_key = Some(key.clone());
                    let inner;
                    braced!(inner in body);
                    while !inner.is_empty() {
                        let code: LitInt = inner.parse()?;
                        let n: u8 = code.base10_parse().map_err(|_| {
                            syn::Error::new(code.span(), "an exit code is a number from 3 to 63")
                        })?;
                        if !(3..=63).contains(&n) {
                            return Err(syn::Error::new(
                                code.span(),
                                "an exit code must be in 3..=63 (0, 1 and 2 are reserved; 64 and up are the framework's)",
                            ));
                        }
                        inner.parse::<Token![=>]>()?;
                        exits.push((code, inner.parse::<LitStr>()?));
                        if inner.is_empty() {
                            break;
                        }
                        inner.parse::<Token![,]>()?;
                    }
                }
                "text" => {
                    text_key = Some(key.clone());
                    text = Some(body.parse::<LitBool>()?);
                }
                "hidden" => hidden = Some(body.parse::<LitBool>()?),
                "default" => {
                    default_key = Some(key.clone());
                    default = Some(body.parse::<LitBool>()?);
                }
                "paths" => {
                    let inner;
                    bracketed!(inner in body);
                    for word in inner.parse_terminated(Ident::parse, Token![,])? {
                        if word == "typed" {
                            typed_only = true;
                        } else {
                            return Err(syn::Error::new(
                                word.span(),
                                "`paths` takes `[typed]` (the typed path only)",
                            ));
                        }
                    }
                }
                "cursor" => {
                    cursor_key = Some(key.clone());
                    cursor = Some(body.parse::<LitBool>()?);
                }
                "cap" => {
                    list_only.push(key.clone());
                    cap = Some(body.parse::<LitInt>()?);
                }
                "page_size" => {
                    list_only.push(key.clone());
                    page_size = Some(body.parse::<LitInt>()?);
                }
                "unpaged" => {
                    list_only.push(key.clone());
                    unpaged = Some(body.parse::<LitBool>()?);
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown key `{other}`: expected params, output, item, op, local, \
                             stream, list, access, audience, errors, fields, exits, text, hidden, \
                             default, paths, cursor, cap, page_size or unpaged"
                        ),
                    ))
                }
            }
            if body.is_empty() {
                break;
            }
            body.parse::<Token![,]>()?;
        }

        let at = path.span();
        let params = params
            .ok_or_else(|| syn::Error::new(at, "missing `params: <type>` (required on every op)"))?;
        if kinds.len() > 1 {
            return Err(syn::Error::new(
                kinds[1].0.span(),
                "an op has exactly one kind: use one of `op:`, `local:`, `stream:` or `list:`",
            ));
        }
        let (_, kind, func) = kinds.pop().ok_or_else(|| {
            syn::Error::new(at, "missing the kind: give one of `op:`, `local:`, `stream:` or `list:`")
        })?;
        let access = access
            .ok_or_else(|| syn::Error::new(at, "missing `access:` (reads, writes, network, spawns or destructive)"))?;
        let audience = audience
            .ok_or_else(|| syn::Error::new(at, "missing `audience:` (agent, owner or dev)"))?;

        let output = if kind == Kind::List {
            if let Some((key, _)) = output {
                return Err(syn::Error::new(
                    key.span(),
                    "a `list:` entry names its element type with `item:`, not `output:`",
                ));
            }
            item.map(|(_, t)| t)
                .ok_or_else(|| syn::Error::new(at, "a `list:` entry needs `item: <type>`"))?
        } else {
            if let Some((key, _)) = item {
                return Err(syn::Error::new(key.span(), "`item:` goes only with `list:`; use `output:`"));
            }
            output
                .map(|(_, t)| t)
                .ok_or_else(|| syn::Error::new(at, "missing `output: <type>`"))?
        };

        if kind == Kind::List {
            for key in [&text_key, &exits_key].into_iter().flatten() {
                return Err(syn::Error::new(
                    key.span(),
                    format!("`{key}:` is not allowed on a `list:` entry: a list pair has no text or extra exits"),
                ));
            }
        } else if let Some(key) = list_only.first() {
            return Err(syn::Error::new(key.span(), format!("`{key}:` goes only with `list:`")));
        }
        if kind != Kind::Stream {
            if let Some(key) = cursor_key {
                return Err(syn::Error::new(key.span(), "`cursor:` goes only with `stream:`"));
            }
        }
        if let Some(key) = default_key {
            if path.value().split_whitespace().count() < 2 {
                return Err(syn::Error::new(
                    key.span(),
                    "`default:` needs a path with a group word: a one-word path is already top level",
                ));
            }
        }

        Ok(Entry {
            path,
            summary,
            long_help,
            kind,
            func,
            params,
            output,
            access,
            audience,
            errors,
            fields,
            exits,
            text,
            hidden,
            default,
            typed_only,
            cursor,
            cap,
            page_size,
            unpaged,
        })
    }
}
