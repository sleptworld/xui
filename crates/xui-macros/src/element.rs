//! The element tree `#[component]`'s view is written in, and its `xui!`
//! grammar: `<tag attr={expr} attr="lit">`, children that are elements or
//! `{expr}`, `<tag />`. `view!` (see [`crate::view`]) parses into the same
//! tree, so the two spellings cannot drift apart. What a tree compiles to is
//! [`crate::reactive`]'s business: this module knows the shape of the
//! syntax and nothing else.
//!
//! The control flow children ([`Child::If`], [`Child::For`],
//! [`Child::Match`], [`Child::Let`]) only have a `view!` spelling.

use proc_macro2::TokenStream as TokenStream2;
use quote::ToTokens;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Error, Expr, Ident, LitStr, Pat, Path, Result, Stmt, Token, braced};


pub struct Element {
    pub tag: Path,
    pub attrs: Vec<Attribute>,
    pub body: Body,
}

pub struct Attribute {
    pub name: Ident,
    pub value: TokenStream2,
}

/// What the element was given besides attributes. Each variant is one of the
/// `xui_core::dsl` body marker types.
pub enum Body {
    /// `NoChildren`
    None,
    /// `Content(expr)` — meaning is up to the receiving widget.
    Content(Expr),
    /// `Children(vec)`
    Children(Vec<Child>),
}

pub enum Child {
    Element(Element),
    /// Spliced through `IntoChildren`, so one element or any iterator of them.
    Expr(Expr),
    If(IfChild),
    For(ForChild),
    Match(MatchChild),
    /// A `let` statement, in scope for the siblings after it.
    Let(Stmt),
}

pub struct IfChild {
    pub if_token: Token![if],
    pub cond: Expr,
    pub then: Vec<Child>,
    pub otherwise: Option<Vec<Child>>,
}

pub struct ForChild {
    pub for_token: Token![for],
    pub pat: Pat,
    pub iter: Expr,
    pub body: Vec<Child>,
}

pub struct MatchChild {
    pub match_token: Token![match],
    pub scrutinee: Expr,
    pub arms: Vec<MatchArm>,
}

pub struct MatchArm {
    pub pat: Pat,
    pub guard: Option<Expr>,
    pub body: Vec<Child>,
}

impl Parse for Element {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        input.parse::<Token![<]>()?;
        let tag: Path = input.parse()?;
        let attrs = parse_attributes(input)?;

        if input.peek(Token![/]) {
            input.parse::<Token![/]>()?;
            input.parse::<Token![>]>()?;
            return Ok(Self {
                tag,
                attrs,
                body: Body::None,
            });
        }

        input.parse::<Token![>]>()?;
        let mut children = parse_children(input, &tag)?;
        // A lone braced expression is intentionally left ambiguous here and
        // resolved by the receiving type.
        let body = match children.as_slice() {
            [] => Body::None,
            [Child::Expr(_)] => match children.pop() {
                Some(Child::Expr(expr)) => Body::Content(expr),
                _ => unreachable!(),
            },
            _ => Body::Children(children),
        };

        Ok(Self { tag, attrs, body })
    }
}

fn parse_attributes(input: ParseStream<'_>) -> Result<Vec<Attribute>> {
    let mut attrs = Vec::new();
    while !(input.peek(Token![>]) || input.peek(Token![/])) {
        if input.is_empty() {
            return Err(Error::new(input.span(), "unterminated opening tag"));
        }
        let name: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        // `attr={expr}` is the general form; `attr="literal"` is sugar for it.
        let value = if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            content.parse::<Expr>()?.into_token_stream()
        } else {
            input.parse::<LitStr>()?.into_token_stream()
        };
        attrs.push(Attribute { name, value });
    }
    Ok(attrs)
}

fn parse_children(input: ParseStream<'_>, tag: &Path) -> Result<Vec<Child>> {
    let mut children = Vec::new();
    loop {
        if input.is_empty() {
            return Err(Error::new(
                tag.span(),
                format!("missing closing tag </{}>", path_name(tag)),
            ));
        }

        if starts_closing_tag(input) {
            input.parse::<Token![<]>()?;
            input.parse::<Token![/]>()?;
            let close: Path = input.parse()?;
            input.parse::<Token![>]>()?;
            if path_name(&close) != path_name(tag) {
                return Err(Error::new(
                    close.span(),
                    format!("expected closing tag </{}>", path_name(tag)),
                ));
            }
            return Ok(children);
        }

        if input.peek(Token![<]) {
            children.push(Child::Element(input.parse()?));
        } else if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            children.push(Child::Expr(content.parse()?));
        } else {
            return Err(Error::new(
                input.span(),
                "children must be nested tags or braced Rust expressions",
            ));
        }
    }
}

fn starts_closing_tag(input: ParseStream<'_>) -> bool {
    let fork = input.fork();
    fork.parse::<Token![<]>().is_ok() && fork.parse::<Token![/]>().is_ok()
}

pub(crate) fn path_name(path: &Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}
