//! `xui!` — the element DSL.
//!
//! This module is deliberately ignorant. It knows the *shape* of the syntax and
//! nothing else: no tag names, no attribute names, no widget methods. Every
//! `<tag attr={value}>` becomes, in effect,
//!
//! ```text
//! let node = tag();
//! let node = node.__with(node.__properties().attr(), value);
//! ElementBody::build(<children>, node)
//! ```
//!
//! `tag()` returns a `Draft` of the node, and the draft's property table —
//! one method per property the node has, its capabilities' included — names
//! the property. A closure given to an `on_*` attribute goes to `__with_on`,
//! which spells out the handler's `Fn` bound so the closure infers its
//! argument types; a
//! `value.share()` goes to `__with_shared`, which projects a forwarded prop.
//! `key` is the draft's key. The type system decides whether the rest is
//! legal. Consequences:
//!
//! - A new host widget or component needs no change here — it only needs a
//!   constructor function in scope.
//! - A new capability property needs no change here — every node selecting
//!   the capability has it in its table.
//! - A misspelled tag or attribute is reported by rustc against the node's
//!   table, with its own "did you mean" suggestion, pointed at the offending
//!   token (see [`Element::expand`]'s use of `quote_spanned!`).
//!
//! The one thing this module *does* enforce is what it can see without types:
//! an attribute must not be written twice.
//!
//! [`Element`] is also the tree `view!` parses into (see [`crate::view`]), so
//! the two syntaxes share one expansion and cannot drift apart. The control
//! flow children ([`Child::If`], [`Child::For`], [`Child::Match`],
//! [`Child::Let`]) only have a `view!` spelling today.

use std::collections::HashMap;

use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Error, Expr, Ident, LitStr, Pat, Path, Result, Stmt, Token, braced};

use crate::errors::Errors;

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

fn path_name(path: &Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// How an attribute's value reaches its property.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Setter {
    /// A value the property accepts.
    Value,
    /// A closure given to an `on_*` attribute: a handler, whose argument
    /// types come from the property it is set to.
    Handler,
    /// `x.share()`: a forwarded prop, projected rather than copied.
    Shared,
}

impl Setter {
    pub(crate) fn of(name: &Ident, value: &TokenStream2) -> Self {
        fn returns_closure(expr: &Expr) -> bool {
            match expr {
                Expr::Closure(_) => true,
                Expr::Paren(expr) => returns_closure(&expr.expr),
                Expr::Group(expr) => returns_closure(&expr.expr),
                Expr::Block(expr) => {
                    matches!(expr.block.stmts.last(), Some(Stmt::Expr(expr, None)) if returns_closure(expr))
                }
                _ => false,
            }
        }
        match syn::parse2::<Expr>(value.clone()) {
            Ok(expr) if name.to_string().starts_with("on_") && returns_closure(&expr) => {
                Self::Handler
            }
            Ok(Expr::MethodCall(call)) if call.method == "share" && call.args.is_empty() => {
                Self::Shared
            }
            _ => Self::Value,
        }
    }
}

/// Keep closure inference at the handler call, even when local bindings
/// precede the closure in an attribute block.
pub(crate) fn map_handler(
    value: TokenStream2,
    call: impl FnOnce(TokenStream2) -> TokenStream2,
) -> TokenStream2 {
    if let Ok(Expr::Block(mut block)) = syn::parse2::<Expr>(value.clone()) {
        if let Some(Stmt::Expr(tail, None)) = block.block.stmts.pop() {
            let statements = block.block.stmts;
            let mapped = map_handler(quote!(#tail), call);
            return quote!({ #(#statements)* #mapped });
        }
    }
    call(value)
}

impl Element {
    pub fn expand(&self, xui: &TokenStream2) -> Result<TokenStream2> {
        let mut errors = Errors::default();
        let tokens = crate::template::lower(self, xui, &mut errors);
        errors.into_result()?;
        Ok(tokens)
    }

    /// The element built now, as an expression: its draft with every
    /// attribute set, given its body. What a component (or any tag a
    /// template does not hold) lowers to.
    pub(crate) fn expand_eager(
        &self,
        xui_core: &TokenStream2,
        errors: &mut Errors,
    ) -> TokenStream2 {
        self.check_duplicate_attrs(errors);

        let tag = &self.tag;
        // The constructor call carries the tag's span, so `cannot find function`
        // underlines the tag rather than the whole macro invocation.
        let constructor = quote_spanned!(tag.span()=> #tag());
        let attrs = self.attrs.iter().map(|attr| {
            let name = &attr.name;
            let value = &attr.value;
            if name == "key" {
                return quote_spanned!(name.span()=> let __xui_node = __xui_node.key(#value););
            }
            let kind = Setter::of(name, value);
            let setter = match kind {
                Setter::Handler => quote_spanned!(name.span()=> __with_on),
                Setter::Shared => quote_spanned!(name.span()=> __with_shared),
                Setter::Value => quote_spanned!(name.span()=> __with),
            };
            // The property comes from the node's own table, so `no method
            // named ..` points at the attribute and lists the node's
            // properties with a similar name.
            let invoke =
                |value| quote_spanned! {name.span()=> __xui_node.#setter(__xui_property, #value) };
            let assigned = if kind == Setter::Handler {
                map_handler(value.clone(), invoke)
            } else {
                invoke(value.clone())
            };
            quote_spanned! {name.span()=>
                let __xui_property = __xui_node.__properties().#name();
                let __xui_node = #assigned;
            }
        });

        let children = self.expand_children(xui_core, errors);
        // The body's span, so that "does not accept this kind of body"
        // underlines the body rather than the whole invocation.
        let span = self.body_span().unwrap_or_else(|| tag.span());
        let build = quote_spanned! {span=>
            #xui_core::dsl::ElementBody::build(#children, __xui_node)
        };
        quote! {
            {
                #[allow(unused_imports)]
                use #xui_core::caps::tables::*;
                let __xui_node = #constructor;
                #(#attrs)*
                #build
            }
        }
    }

    fn body_span(&self) -> Option<Span> {
        match &self.body {
            Body::None => None,
            Body::Content(expr) => Some(expr.span()),
            Body::Children(children) => children.first().map(Child::span),
        }
    }

    pub(crate) fn check_duplicate_attrs(&self, errors: &mut Errors) {
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
    fn expand_children(&self, xui_core: &TokenStream2, errors: &mut Errors) -> TokenStream2 {
        match &self.body {
            Body::None => quote!(#xui_core::dsl::NoChildren),
            Body::Content(expr) => {
                quote_spanned!(expr.span()=> #xui_core::dsl::Content(#expr))
            }
            Body::Children(children) => {
                let statements = expand_statements(children, xui_core, errors);
                quote! {
                    #xui_core::dsl::Children({
                        let mut __xui_children = ::std::vec::Vec::new();
                        #statements
                        __xui_children
                    })
                }
            }
        }
    }
}

impl Child {
    fn span(&self) -> Span {
        match self {
            Child::Element(element) => element.tag.span(),
            Child::Expr(expr) => expr.span(),
            Child::If(child) => child.if_token.span,
            Child::For(child) => child.for_token.span,
            Child::Match(child) => child.match_token.span,
            Child::Let(stmt) => stmt.span(),
        }
    }
}

/// Lowers a child list to statements that push onto `__xui_children`. Control
/// flow children become the same control flow around their own pushes, so a
/// branch that is not taken contributes nothing and a loop contributes one
/// entry per iteration.
pub(crate) fn expand_statements(
    children: &[Child],
    xui_core: &TokenStream2,
    errors: &mut Errors,
) -> TokenStream2 {
    let statements = children.iter().map(|child| match child {
        Child::Element(element) => {
            let element = crate::template::lower(element, xui_core, errors);
            quote!(__xui_children.push(#element);)
        }
        Child::Expr(expr) => quote_spanned! {expr.span()=>
            #xui_core::IntoChildren::append_children(#expr, &mut __xui_children);
        },
        Child::If(child) => expand_if(child, xui_core, errors),
        Child::For(ForChild {
            for_token,
            pat,
            iter,
            body,
            ..
        }) => {
            let body = expand_statements(body, xui_core, errors);
            quote!(#for_token #pat in #iter { #body })
        }
        Child::Match(MatchChild {
            match_token,
            scrutinee,
            arms,
        }) => {
            let arms = arms.iter().map(|arm| {
                let pat = &arm.pat;
                let guard = arm.guard.as_ref().map(|guard| quote!(if #guard));
                let body = expand_statements(&arm.body, xui_core, errors);
                quote!(#pat #guard => { #body })
            });
            let arms: Vec<_> = arms.collect();
            quote!(#match_token #scrutinee { #(#arms)* })
        }
        Child::Let(stmt) => stmt.to_token_stream(),
    });
    let statements: Vec<_> = statements.collect();
    quote!(#(#statements)*)
}

fn expand_if(child: &IfChild, xui_core: &TokenStream2, errors: &mut Errors) -> TokenStream2 {
    let IfChild {
        if_token,
        cond,
        then,
        otherwise,
    } = child;
    let then = expand_statements(then, xui_core, errors);
    let otherwise = otherwise.as_ref().map(|otherwise| {
        // `else if` arrives as an `else` holding a single `if` child; emitting
        // it without the extra block keeps the expansion readable.
        match otherwise.as_slice() {
            [Child::If(nested)] => {
                let nested = expand_if(nested, xui_core, errors);
                quote!(else #nested)
            }
            otherwise => {
                let otherwise = expand_statements(otherwise, xui_core, errors);
                quote!(else { #otherwise })
            }
        }
    });
    quote!(#if_token #cond { #then } #otherwise)
}
