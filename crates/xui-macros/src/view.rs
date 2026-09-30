//! `view!` — the element DSL, spelled like SwiftUI.
//!
//! ```text
//! view! {
//!     column(gap: 12.0) {
//!         text("Inbox").font_size(20.0)
//!         if messages.is_empty() {
//!             text("Nothing here")
//!         } else {
//!             for message in &messages {
//!                 message_row(message: message.clone()).key(message.id.clone())
//!             }
//!         }
//!         ..footer
//!     }
//!     .padding(EdgeInsets::all(16.0))
//! }
//! ```
//!
//! This is only a second parser. It produces the same [`Element`] tree as
//! `xui!` and shares its expansion, so the runtime contract is identical:
//!
//! | `view!`                  | `xui!`                       | lowers to                     |
//! |--------------------------|------------------------------|-------------------------------|
//! | `tag()`                  | `<tag />`                    | body `NoChildren`             |
//! | `tag(name: v)`           | `<tag name={v} />`           | `tag().name(v)`               |
//! | `tag().name(v)`          | `<tag name={v} />`           | `tag().name(v)`               |
//! | `tag().name(a, b)`       | `<tag name={(a, b)} />`      | `tag().name((a, b))`          |
//! | `tag(expr)`              | `<tag>{expr}</tag>`          | body `Content(expr)`          |
//! | `tag { a() b() }`        | `<tag><a /><b /></tag>`      | body `Children(vec)`          |
//! | `..expr` as a child      | `{expr}` among siblings      | `IntoChildren` splice         |
//!
//! Named arguments and trailing modifiers are the same thing — both become a
//! builder method call — so which to use is a matter of reading order: what
//! the element *is* in the parentheses, how it looks after them. Setting one
//! name twice across the two is still an error.
//!
//! A splice is `..expr` rather than `{expr}` because a `{` after an element
//! is always that element's body: `text("a") {rows}` would otherwise be
//! ambiguous across a line break.
//!
//! Inside a `{ ... }` body, `if`/`else`, `if let`, `for`, `match` and `let`
//! work like they do in a SwiftUI result builder: their bodies are child
//! lists, and they contribute whatever children the taken branch or each
//! iteration produces.

use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Error, Expr, ExprPath, ExprRange, Ident, Pat, RangeLimits, Result, Stmt, Token, braced,
    parenthesized,
};

use quote::{ToTokens, quote};

use crate::element::{Attribute, Body, Child, Element, ForChild, IfChild, MatchArm, MatchChild};

/// The root of a `view!` invocation: exactly one element.
pub struct View(pub Element);

impl Parse for View {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let element = parse_element(input)?;
        if !input.is_empty() {
            return Err(Error::new(
                input.span(),
                "`view!` takes exactly one root element; wrap siblings in a container such as `column { .. }`",
            ));
        }
        Ok(Self(element))
    }
}

enum Arg {
    Named(Ident, Expr),
    Positional(Expr),
}

impl Parse for Arg {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        // `name: value`, but not `path::to::value`.
        if input.peek(Ident) && input.peek2(Token![:]) && !input.peek2(Token![::]) {
            let name: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Named(name, input.parse()?))
        } else {
            Ok(Self::Positional(input.parse()?))
        }
    }
}

fn parse_element(input: ParseStream<'_>) -> Result<Element> {
    // An expression path, so `tag::<T>()` works and `(` ends the path instead
    // of being read as `Fn(..)` sugar.
    let ExprPath { path, qself, .. } = input.parse()?;
    if let Some(qself) = qself {
        return Err(Error::new(
            qself.lt_token.span,
            "a qualified path cannot be used as an element",
        ));
    }

    if !(input.peek(syn::token::Paren) || input.peek(syn::token::Brace)) {
        let name = path.to_token_stream();
        return Err(Error::new(
            path.span(),
            format!(
                "expected `(` or `{{` after `{name}`: write `{name}()` for an element, \
                 or `..{name}` to insert an expression"
            ),
        ));
    }

    let mut attrs = Vec::new();
    let mut content = None;
    if input.peek(syn::token::Paren) {
        let args;
        parenthesized!(args in input);
        for arg in Punctuated::<Arg, Token![,]>::parse_terminated(&args)? {
            match arg {
                Arg::Named(name, value) => attrs.push(Attribute {
                    name,
                    value: value.into_token_stream(),
                }),
                Arg::Positional(expr) if content.is_some() => {
                    return Err(Error::new(
                        expr.span(),
                        "an element takes at most one unnamed argument (its content); \
                         name the others, as in `name: value`",
                    ));
                }
                Arg::Positional(expr) if !attrs.is_empty() => {
                    return Err(Error::new(
                        expr.span(),
                        "the unnamed content argument must come before named arguments",
                    ));
                }
                Arg::Positional(expr) => content = Some(expr),
            }
        }
    }

    let body = if input.peek(syn::token::Brace) {
        let block;
        let brace = braced!(block in input);
        if let Some(content) = &content {
            let mut error = Error::new(
                brace.span.join(),
                "an element takes either an unnamed content argument or a `{ .. }` body, not both",
            );
            error.combine(Error::new(
                content.span(),
                "content argument given here; if the `{ .. }` was meant as the next \
                 sibling, insert it with `..expr` instead",
            ));
            return Err(error);
        }
        Body::Children(parse_children(&block)?)
    } else {
        match content {
            Some(expr) => Body::Content(expr),
            None => Body::None,
        }
    };

    // `.` but not the `..` of a following splice.
    while input.peek(Token![.]) && !input.peek(Token![..]) {
        input.parse::<Token![.]>()?;
        let name: Ident = input.parse()?;
        let args;
        let paren = parenthesized!(args in input);
        let values = Punctuated::<Expr, Token![,]>::parse_terminated(&args)?;
        // Builder methods take exactly one value, and multi-argument setters
        // take a tuple (see `xui_core::dsl::StyleProps`), so `.m(a, b)` is
        // sugar for `.m((a, b))`.
        let value = match values.len() {
            0 => {
                return Err(Error::new(
                    paren.span.join(),
                    format!("modifier `.{name}()` needs a value, as in `.{name}(true)`"),
                ));
            }
            1 => values.into_token_stream(),
            _ => quote!((#values)),
        };
        attrs.push(Attribute { name, value });
    }

    Ok(Element {
        tag: path,
        attrs,
        body,
    })
}

fn parse_children(input: ParseStream<'_>) -> Result<Vec<Child>> {
    let mut children = Vec::new();
    while !input.is_empty() {
        parse_child(input, &mut children)?;
    }
    Ok(children)
}

fn parse_child(input: ParseStream<'_>, children: &mut Vec<Child>) -> Result<()> {
    let child = parse_single_child(input, children)?;
    children.push(child);
    Ok(())
}

fn parse_single_child(input: ParseStream<'_>, children: &mut Vec<Child>) -> Result<Child> {
    if input.peek(Token![if]) {
        Ok(Child::If(parse_if(input)?))
    } else if input.peek(Token![for]) {
        let for_token = input.parse()?;
        let pat = Pat::parse_multi_with_leading_vert(input)?;
        input.parse::<Token![in]>()?;
        let iter = Expr::parse_without_eager_brace(input)?;
        let body = parse_block(input)?;
        Ok(Child::For(ForChild {
            for_token,
            pat,
            iter,
            body,
        }))
    } else if input.peek(Token![match]) {
        Ok(Child::Match(parse_match(input)?))
    } else if input.peek(Token![let]) {
        match input.parse::<Stmt>()? {
            stmt @ Stmt::Local(_) => Ok(Child::Let(stmt)),
            other => Err(Error::new(other.span(), "expected a `let` statement")),
        }
    } else if input.peek(Token![..]) {
        input.parse::<Token![..]>()?;
        match input.parse()? {
            // `..a ..b` parses as the range `a..b`: range is the one binary
            // operator a sibling can start with. Rust ranges do not chain, so
            // the parser stops after one and there are always exactly two.
            // Splicing a range is not meaningful; `..(a..b)` still says so.
            Expr::Range(ExprRange {
                attrs,
                start: Some(start),
                limits: RangeLimits::HalfOpen(_),
                end: Some(end),
            }) if attrs.is_empty() => {
                children.push(Child::Expr(*start));
                Ok(Child::Expr(*end))
            }
            expr => Ok(Child::Expr(expr)),
        }
    } else if input.peek(syn::token::Brace) {
        // `{expr}` would be ambiguous with the body of the element before it.
        Err(Error::new(
            input.span(),
            "a `{ .. }` here would be the body of the previous element; \
             insert an expression with `..expr`",
        ))
    } else if input.peek(Ident)
        || input.peek(Token![::])
        || input.peek(Token![crate])
        || input.peek(Token![self])
        || input.peek(Token![super])
    {
        Ok(Child::Element(parse_element(input)?))
    } else {
        Err(Error::new(
            input.span(),
            "expected an element such as `text(..)`, `..expr` to insert children, \
             or `if`/`for`/`match`/`let`",
        ))
    }
}

fn parse_block(input: ParseStream<'_>) -> Result<Vec<Child>> {
    let block;
    braced!(block in input);
    parse_children(&block)
}

fn parse_if(input: ParseStream<'_>) -> Result<IfChild> {
    let if_token = input.parse()?;
    let cond = Expr::parse_without_eager_brace(input)?;
    let then = parse_block(input)?;
    let otherwise = if input.peek(Token![else]) {
        input.parse::<Token![else]>()?;
        if input.peek(Token![if]) {
            Some(vec![Child::If(parse_if(input)?)])
        } else {
            Some(parse_block(input)?)
        }
    } else {
        None
    };
    Ok(IfChild {
        if_token,
        cond,
        then,
        otherwise,
    })
}

fn parse_match(input: ParseStream<'_>) -> Result<MatchChild> {
    let match_token = input.parse()?;
    let scrutinee = Expr::parse_without_eager_brace(input)?;
    let content;
    braced!(content in input);
    let mut arms = Vec::new();
    while !content.is_empty() {
        let pat = Pat::parse_multi_with_leading_vert(&content)?;
        let guard = if content.peek(Token![if]) {
            content.parse::<Token![if]>()?;
            Some(content.parse()?)
        } else {
            None
        };
        content.parse::<Token![=>]>()?;
        // `pat => { children }` or `pat => single_child`.
        let body = if content.peek(syn::token::Brace) {
            parse_block(&content)?
        } else {
            let mut body = Vec::new();
            parse_child(&content, &mut body)?;
            body
        };
        if !content.is_empty() && content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
        arms.push(MatchArm { pat, guard, body });
    }
    Ok(MatchChild {
        match_token,
        scrutinee,
        arms,
    })
}
