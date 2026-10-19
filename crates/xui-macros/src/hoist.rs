//! Static subtree hoisting for `xui!` / `view!`.
//!
//! An element whose tag, attributes and children are all *static* — decided
//! from the syntax alone, see [`is_static_element`] — is built once per call
//! site and thread and handed out as the same allocation on every render. The
//! runtime then skips it by pointer, the way it skips a memoized element
//! (`docs/template-slots.md` §5).
//!
//! The judgement stays syntactic, so this module still knows no tag or
//! attribute name. It is deliberately conservative: an expression it cannot
//! prove free of local variables, outer generics and side effects is dynamic.
//! Anything that slips through the rules and does refer to a local or a
//! generic parameter fails to compile (E0434 / E0401) inside the hoisted
//! initializer rather than producing a UI that stops updating.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Expr, Path, PathArguments, Type, UnOp};

use crate::element::{Body, Child, Element};

/// Whether `element` and everything under it can be built once and shared.
pub fn is_static_element(element: &Element) -> bool {
    is_static_tag(&element.tag)
        && element.attrs.iter().all(|attr| {
            syn::parse2::<Expr>(attr.value.clone()).is_ok_and(|value| is_static_expr(&value))
        })
        && match &element.body {
            Body::None => true,
            Body::Content(content) => is_static_expr(content),
            Body::Children(children) => children.iter().all(|child| match child {
                Child::Element(element) => is_static_element(element),
                Child::Expr(expr) => is_static_expr(expr),
                Child::If(_) | Child::For(_) | Child::Match(_) | Child::Let(_) => false,
            }),
        }
}

/// Wraps an element expansion so that it runs once per thread.
pub fn hoist(element: TokenStream2, xui_core: &TokenStream2) -> TokenStream2 {
    quote! {{
        ::std::thread_local! {
            static __XUI_HOISTED: ::std::rc::Rc<#xui_core::dsl::Hoisted> =
                #xui_core::dsl::Hoisted::new(#element);
        }
        #xui_core::dsl::hoisted(&__XUI_HOISTED)
    }}
}

/// A tag constructor is a function path; a generic one (`tag::<T>()`) or one
/// reached through `Self` could name the enclosing item's generics.
fn is_static_tag(tag: &Path) -> bool {
    plain_path(tag)
        && tag
            .segments
            .first()
            .is_none_or(|first| !is_generic_like(&first.ident))
}

/// The static expressions: literals and constants, and tuples, arrays,
/// references, casts and negations of them. Calls of any kind are dynamic —
/// `Color::hex("#fff")` is pure, but `Instant::now()` has the same shape.
pub fn is_static_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(lit) => lit.attrs.is_empty(),
        Expr::Unary(unary) => matches!(unary.op, UnOp::Neg(_)) && is_static_expr(&unary.expr),
        Expr::Paren(paren) => is_static_expr(&paren.expr),
        Expr::Group(group) => is_static_expr(&group.expr),
        Expr::Tuple(tuple) => tuple.elems.iter().all(is_static_expr),
        Expr::Array(array) => array.elems.iter().all(is_static_expr),
        Expr::Reference(reference) => {
            reference.mutability.is_none() && is_static_expr(&reference.expr)
        }
        Expr::Cast(cast) => is_static_expr(&cast.expr) && is_primitive(&cast.ty),
        Expr::Path(path) => {
            path.attrs.is_empty() && path.qself.is_none() && is_constant_path(&path.path)
        }
        _ => false,
    }
}

/// A path that names a constant or an enum variant rather than a local:
/// `Color::BLACK`, `AlignStyle::Center`, `GAP`, `None`.
fn is_constant_path(path: &Path) -> bool {
    let (Some(first), Some(last)) = (path.segments.first(), path.segments.last()) else {
        return false;
    };
    // A lowercase last segment is a local, a function or a module; a single
    // capital letter or `Self` up front may be the enclosing item's generics.
    plain_path(path)
        && !is_generic_like(&first.ident)
        && last
            .ident
            .to_string()
            .starts_with(|c: char| c.is_ascii_uppercase())
}

/// `Self`, or a single capital letter: the usual spelling of a type parameter.
fn is_generic_like(ident: &syn::Ident) -> bool {
    let name = ident.to_string();
    name == "Self" || (name.len() == 1 && name.starts_with(|c: char| c.is_ascii_uppercase()))
}

fn plain_path(path: &Path) -> bool {
    path.segments
        .iter()
        .all(|segment| matches!(segment.arguments, PathArguments::None))
}

fn is_primitive(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.qself.is_none()
        && path.path.get_ident().is_some_and(|ident| {
            matches!(
                ident.to_string().as_str(),
                "f32"
                    | "f64"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "isize"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "usize"
                    | "bool"
                    | "char"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn expr(tokens: TokenStream2) -> Expr {
        syn::parse2(tokens).unwrap()
    }

    #[test]
    fn static_expressions() {
        for tokens in [
            quote!(12.0),
            quote!("Inbox"),
            quote!(-1),
            quote!(true),
            quote!((8.0, 4.0)),
            quote!([1, 2]),
            quote!(&"a"),
            quote!(1 as f32),
            quote!(Color::BLACK),
            quote!(AlignStyle::Center),
            quote!(crate::theme::GAP),
            quote!(GAP),
            quote!(None),
        ] {
            assert!(is_static_expr(&expr(tokens.clone())), "{tokens} is static");
        }
    }

    #[test]
    fn dynamic_expressions() {
        for tokens in [
            quote!(count),
            quote!(self),
            quote!(T::DEFAULT),
            quote!(Self::GAP),
            quote!(N),
            quote!(Self),
            quote!(Color::hex("#fff")),
            quote!(EdgeInsets::all(16.0)),
            quote!(Style::new().width(16.0)),
            quote!(format!("{}", 1)),
            quote!(move |_| {}),
            quote!({ 1 }),
            quote!(&mut X),
            quote!(1 as T),
            quote!(Vec::<u8>::NEW),
            quote!(theme.gap),
            quote!(-count),
        ] {
            assert!(
                !is_static_expr(&expr(tokens.clone())),
                "{tokens} is dynamic"
            );
        }
    }

    #[test]
    fn static_tags() {
        assert!(is_static_tag(&parse_quote!(text)));
        assert!(is_static_tag(&parse_quote!(widgets::text)));
        assert!(!is_static_tag(&parse_quote!(list::<T>)));
        assert!(!is_static_tag(&parse_quote!(Self::row)));
        assert!(!is_static_tag(&parse_quote!(T::make)));
    }
}
