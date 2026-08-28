//! `xui!` — the element DSL.
//!
//! This module is deliberately ignorant. It knows the *shape* of the syntax and
//! nothing else: no tag names, no attribute names, no widget methods. Every
//! `<tag attr={value}>` becomes
//!
//! ```text
//! IntoElement::into_element(tag().attr(value), <children>)
//! ```
//!
//! and the type system decides whether that is legal. Consequences:
//!
//! - A new host widget or component needs no change here — it only needs a
//!   constructor function in scope.
//! - A new style property needs no change here — `xui::dsl::StyleProps` picks it
//!   up for every widget at once.
//! - A misspelled tag or attribute is reported by rustc against the real method
//!   set, with its own "did you mean" suggestion, pointed at the offending
//!   token (see [`Element::expand`]'s use of `quote_spanned!`).
//!
//! The one thing this module *does* enforce is what it can see without types:
//! an attribute must not be written twice.

use std::collections::HashMap;

use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Error, Expr, Ident, LitStr, Path, Result, Token, braced};

use crate::errors::Errors;

pub struct Element {
    tag: Path,
    attrs: Vec<Attribute>,
    children: Vec<Child>,
}

struct Attribute {
    name: Ident,
    value: TokenStream2,
}

enum Child {
    Element(Element),
    Expr(Expr),
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
                children: Vec::new(),
            });
        }

        input.parse::<Token![>]>()?;
        let children = parse_children(input, &tag)?;

        Ok(Self {
            tag,
            attrs,
            children,
        })
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

fn path_name(path: &Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

impl Element {
    pub fn expand(&self, xui: &TokenStream2) -> Result<TokenStream2> {
        let mut errors = Errors::default();
        let tokens = self.expand_inner(xui, &mut errors);
        errors.into_result()?;
        Ok(tokens)
    }

    fn expand_inner(&self, xui: &TokenStream2, errors: &mut Errors) -> TokenStream2 {
        self.check_duplicate_attrs(errors);

        let tag = &self.tag;
        // The constructor call carries the tag's span, so `cannot find function`
        // underlines the tag rather than the whole macro invocation.
        let mut builder = quote_spanned!(tag.span()=> #tag());
        for attr in &self.attrs {
            let name = &attr.name;
            let value = &attr.value;
            // Likewise for `no method named ...`: it points at the attribute.
            builder = quote_spanned!(name.span()=> #builder.#name(#value));
        }

        let children = self.expand_children(xui, errors);
        // The body's span, so that "does not accept this kind of body"
        // underlines the body rather than the whole invocation.
        let span = self.body_span().unwrap_or_else(|| tag.span());
        quote_spanned! {span=>
            #xui::dsl::ElementBody::build(#children, #builder)
        }
    }

    fn body_span(&self) -> Option<Span> {
        match self.children.as_slice() {
            [] => None,
            [Child::Expr(expr)] => Some(expr.span()),
            children => children.first().map(|child| match child {
                Child::Element(element) => element.tag.span(),
                Child::Expr(expr) => expr.span(),
            }),
        }
    }

    fn check_duplicate_attrs(&self, errors: &mut Errors) {
        let mut seen: HashMap<String, Span> = HashMap::new();
        for attr in &self.attrs {
            let name = attr.name.to_string();
            if let Some(first) = seen.get(&name) {
                let mut error = Error::new(
                    attr.name.span(),
                    format!("attribute `{name}` is set more than once"),
                );
                error.combine(Error::new(*first, format!("`{name}` was first set here")));
                errors.push(error);
            } else {
                seen.insert(name, attr.name.span());
            }
        }
    }

    /// Children are handed to the widget as one of three marker types; which of
    /// them a widget accepts is a property of its `IntoElement` impls, not of
    /// this macro. That is what makes `<canvas>{x}</canvas>` a type error and
    /// `<text>{x}</text>` a text assignment without either being special-cased.
    fn expand_children(&self, xui: &TokenStream2, errors: &mut Errors) -> TokenStream2 {
        match self.children.as_slice() {
            [] => quote!(#xui::dsl::NoChildren),
            // A lone braced expression is intentionally left ambiguous here and
            // resolved by the receiving type.
            [Child::Expr(expr)] => {
                quote_spanned!(expr.span()=> #xui::dsl::Content(#expr))
            }
            children => {
                let pushes = children.iter().map(|child| match child {
                    Child::Element(element) => {
                        let element = element.expand_inner(xui, errors);
                        quote!(__xui_children.push(#element);)
                    }
                    Child::Expr(expr) => quote_spanned! {expr.span()=>
                        #xui::IntoChildren::append_children(#expr, &mut __xui_children);
                    },
                });
                quote! {
                    #xui::dsl::Children({
                        let mut __xui_children = ::std::vec::Vec::new();
                        #(#pushes)*
                        __xui_children
                    })
                }
            }
        }
    }
}
