//! Templated host elements for `xui!` / `view!` (`docs/template-slots.md` §12).
//!
//! A lowercase tag is a host, and `xui_schema` says which hosts and which of
//! their attributes this module understands. For such an element the
//! attributes that never change are applied once per call site, into a
//! `TemplateBase`; on every render only the rest are evaluated:
//!
//! - style attributes from the first changing one on, chained in source order
//!   onto an empty `Style` through `dsl::fragment`, whose functions take the
//!   same parameters as the builder methods, so values are typed as before;
//! - `on_*` handlers, each passed straight to `dsl::handlers::on_*`, whose own
//!   bound gives an unannotated closure its higher-ranked signature;
//! - `key`, and `text(..)`'s content.
//!
//! Anything else that changes, and `style` / `scrollbar` written after a
//! changing style attribute (they rewrite the style wholesale, so their place
//! matters), leaves the element to the ordinary expansion.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Expr, Ident};
use xui_schema::{HostBody, ORDERED_STYLE_ATTRS, STYLE_PROPS, host_tag};

use crate::element::{Body, Element};
use crate::errors::Errors;
use crate::hoist::is_static_expr;

/// One attribute evaluated on every render, in source order.
enum Slot<'a> {
    Style(&'a Ident, &'a TokenStream2),
    Handler(&'a Ident, &'a TokenStream2),
    Key(&'a Ident, &'a TokenStream2),
}

/// The templated expansion of `element`, or `None` when it has to be built
/// the ordinary way.
pub fn expand(
    element: &Element,
    xui_core: &TokenStream2,
    errors: &mut Errors,
) -> Option<TokenStream2> {
    let tag = &element.tag;
    if tag
        .segments
        .iter()
        .any(|segment| !segment.arguments.is_none())
    {
        return None;
    }
    let host = host_tag(&tag.segments.last()?.ident.to_string())?;

    let mut base_attrs = Vec::new();
    let mut slots = Vec::new();
    // Set once a style attribute's value changes: style attributes after it
    // are chained behind it rather than applied up front, keeping their order.
    let mut style_changes = false;
    for attr in &element.attrs {
        let name = attr.name.to_string();
        let fixed =
            syn::parse2::<Expr>(attr.value.clone()).is_ok_and(|value| is_static_expr(&value));
        if name == "key" {
            if fixed {
                base_attrs.push(attr);
            } else {
                slots.push(Slot::Key(&attr.name, &attr.value));
            }
        } else if name.starts_with("on_") {
            slots.push(Slot::Handler(&attr.name, &attr.value));
        } else if STYLE_PROPS.contains(&name.as_str()) {
            if fixed && !style_changes {
                base_attrs.push(attr);
            } else {
                style_changes |= !fixed;
                slots.push(Slot::Style(&attr.name, &attr.value));
            }
        } else if fixed && !(style_changes && ORDERED_STYLE_ATTRS.contains(&name.as_str())) {
            base_attrs.push(attr);
        } else {
            return None;
        }
    }

    let mut base_content = None;
    let mut content_slot = None;
    let body = match (&element.body, host.body) {
        (Body::None, _) => quote!(#xui_core::dsl::NoChildren),
        (Body::Content(content), HostBody::Text) if is_static_expr(content) => {
            base_content = Some(content);
            quote!(#xui_core::dsl::NoChildren)
        }
        (Body::Content(content), HostBody::Text) => {
            content_slot = Some(content);
            quote!(#xui_core::dsl::NoChildren)
        }
        // Text takes no children: leave the type error to the ordinary path.
        (Body::Children(_), HostBody::Text) => return None,
        (_, HostBody::Children) => element.expand_children(xui_core, errors, true),
    };

    element.check_duplicate_attrs(errors);

    // The static part, built on first use in each thread. Spans as in
    // `Element::expand_inner`, so a misspelled static attribute still gets
    // rustc's own `no method named` against the attribute.
    let widget = format_ident!("{}", host.widget);
    let mut base = quote_spanned!(tag.span()=> #tag());
    for attr in base_attrs {
        let name = &attr.name;
        let value = &attr.value;
        base = quote_spanned!(name.span()=> #base.#name(#value));
    }
    if let Some(content) = base_content {
        base = quote_spanned!(content.span()=> #base.text(#content));
    }

    let content = match content_slot {
        Some(content) => quote_spanned! {content.span()=>
            ::core::option::Option::Some(#xui_core::dsl::text_content(#content))
        },
        None => quote!(::core::option::Option::None),
    };

    let mut statements = Vec::new();
    let mut handlers = Vec::new();
    let mut key = quote!(::core::option::Option::None);
    let mut style = quote!(::core::option::Option::None);
    let mut style_started = false;
    for (index, slot) in slots.iter().enumerate() {
        match slot {
            Slot::Style(name, value) => {
                if !style_started {
                    style_started = true;
                    statements.push(quote!(let __xui_style = #xui_core::dsl::empty_style();));
                    style = quote!(::core::option::Option::Some(__xui_style));
                }
                statements.push(quote_spanned! {name.span()=>
                    let __xui_style = #xui_core::dsl::fragment::#name(__xui_style, #value);
                });
            }
            Slot::Handler(name, value) => {
                let binding = format_ident!("__xui_handler_{index}");
                statements.push(quote_spanned! {name.span()=>
                    let #binding = #xui_core::dsl::handlers::#name(#value);
                });
                handlers.push(binding);
            }
            Slot::Key(name, value) => {
                statements.push(quote_spanned! {name.span()=>
                    let __xui_key = #xui_core::dsl::key(#value);
                });
                key = quote!(::core::option::Option::Some(__xui_key));
            }
        }
    }

    // Body first, then the attributes in order: the order today's builder
    // chain evaluates them in.
    Some(quote! {{
        let __xui_body = #body;
        let __xui_content = #content;
        ::std::thread_local! {
            static __XUI_TEMPLATE: ::std::rc::Rc<#xui_core::dsl::TemplateBase> =
                #xui_core::dsl::TemplateBase::new(
                    #xui_core::dsl::host::<#xui_core::widgets::#widget>(#base)
                );
        }
        #(#statements)*
        #xui_core::dsl::templated(
            &__XUI_TEMPLATE,
            #key,
            #style,
            __xui_content,
            [#(#handlers),*],
            __xui_body,
        )
    }})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn templates(source: &str) -> bool {
        let element: Element = syn::parse_str(source).unwrap();
        expand(&element, &quote!(::xui_core), &mut Errors::default()).is_some()
    }

    #[test]
    fn lowercase_vocabulary_tags_are_templated() {
        assert!(templates("<row gap={8.0} background={color} />"));
        assert!(templates("<text color={color}>{label}</text>"));
        assert!(templates("<container on_click={handler} key={id} />"));
        assert!(templates("<xui_core::widgets::column gap={gap} />"));
    }

    #[test]
    fn other_tags_are_built_as_before() {
        // Components, hosts outside the vocabulary, and generic tags.
        assert!(!templates("<Button label={label} />"));
        assert!(!templates("<image src={src} />"));
        assert!(!templates("<my_widget gap={gap} />"));
        assert!(!templates("<row::<T> gap={gap} />"));
    }

    #[test]
    fn a_changing_attribute_outside_the_vocabulary_builds_the_element_as_before() {
        assert!(!templates("<row flex_wrap={wrap} />"));
        assert!(!templates("<container focusable={focusable} />"));
        assert!(!templates("<row style={style} />"));
        // Static, it goes into the base.
        assert!(templates("<row flex_wrap={true} gap={gap} />"));
    }

    #[test]
    fn style_and_scrollbar_must_come_before_every_changing_style_attribute() {
        assert!(templates("<row style={CARD} background={color} />"));
        assert!(!templates("<row background={color} style={CARD} />"));
        assert!(templates("<row scrollbar={BAR} width={width} />"));
        assert!(!templates("<row width={width} scrollbar={BAR} />"));
        // A changing handler or key does not order the style.
        assert!(templates("<row on_click={handler} style={CARD} />"));
    }

    #[test]
    fn text_takes_content_not_children() {
        assert!(!templates("<text color={color}><row /></text>"));
    }
}
