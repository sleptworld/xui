use crate::{ElementNode, XuiAttribute};
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Error, Result};

pub fn parse_attrs_helper<F>(
    node: &ElementNode,
    f: F,
    attrs_stmts: &mut Vec<TokenStream2>,
) -> Result<()>
where
    F: Fn(&str, &TokenStream2) -> Option<TokenStream2>,
{
    for attr in &node.attrs {
        let name = attr.name.to_string();
        let value = &attr.value;

        if let Some(stmt) = f(name.as_str(), value) {
            attrs_stmts.push(stmt);
            continue;
        }
    }
    Ok(())
}

pub fn parse_base_attr<T: quote::ToTokens + ?Sized>(name: &str, value: &T) -> Option<TokenStream2> {
    match name {
        "key" => Some(quote! {
            __xui_element = __xui_element.key(#value);
        }),
        "style" => Some(quote! {
            __xui_style.merge(&#value);
        }),
        _ => None,
    }
}

pub fn parse_text_style_attr<T: quote::ToTokens + ?Sized>(
    name: &str,
    value: &T,
) -> Option<TokenStream2> {
    match name {
        "color" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().color(#value));
        }),
        "font_family" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().font_family(#value));
        }),
        "font_size" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().font_size(#value));
        }),
        "font_weight" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().font_weight(#value));
        }),
        "font_style" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().font_style(#value));
        }),
        "line_height" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().line_height(#value));
        }),
        "letter_spacing" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().letter_spacing(#value));
        }),
        "decoration" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().decoration(#value));
        }),
        _ => None,
    }
}

pub fn parse_layout_style_attr<T: quote::ToTokens + ?Sized>(
    name: &str,
    value: &T,
) -> Option<TokenStream2> {
    match name {
        "flex_direction" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().flex_direction(#value));
        }),
        "gap" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().gap(#value));
        }),
        "padding" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().padding(#value));
        }),
        "size" => Some(quote! {
            if let Some(__xui_size) = #value {
                __xui_style.merge(&::xui::Style::new().size(__xui_size));
            }
        }),
        "min_size" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().min_size(#value));
        }),
        "max_size" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().max_size(#value));
        }),
        "margin" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().margin(#value));
        }),
        "align" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().align(#value));
        }),
        "justify" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().justify(#value));
        }),
        "width" => Some(quote! {
            let mut __xui_size = match __xui_style.layout.size {
                ::xui::StyleValue::Value(size) => size,
                _ => ::xui::Size::hug(),
            };
            __xui_size.width = #value;
            __xui_style.layout.size = ::xui::StyleValue::Value(__xui_size);
        }),
        "height" => Some(quote! {
            let mut __xui_size = match __xui_style.layout.size {
                ::xui::StyleValue::Value(size) => size,
                _ => ::xui::Size::hug(),
            };
            __xui_size.height = #value;
            __xui_style.layout.size = ::xui::StyleValue::Value(__xui_size);
        }),

        _ => None,
    }
}

pub fn parse_paint_style_attr<T: quote::ToTokens + ?Sized>(
    name: &str,
    value: &T,
) -> Option<TokenStream2> {
    match name {
        "background" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().background(#value));
        }),
        "border_color" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().border_color(#value));
        }),
        "border_width" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().border_width(#value));
        }),
        "border_radius" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().border_radius(#value));
        }),
        "stroke" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().stroke(#value));
        }),
        "stroke_style" => Some(quote! {
            let (__xui_stroke_color, __xui_stroke_width, __xui_stroke_line_style) = #value;

            __xui_style.merge(&::xui::Style::new().stroke_style(
                __xui_stroke_color,
                __xui_stroke_width,
                __xui_stroke_line_style,
            ));
        }),
        "no_stroke" => Some(quote! {
            if #value {
                __xui_style.merge(&::xui::Style::new().no_stroke());
            }
        }),
        "shadow" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().shadow(#value));
        }),
        "box_shadow" => Some(quote! {
            let (
                __xui_shadow_color,
                __xui_shadow_offset,
                __xui_shadow_blur,
                __xui_shadow_spread,
            ) = #value;

            __xui_style.merge(&::xui::Style::new().box_shadow(
                __xui_shadow_color,
                __xui_shadow_offset,
                __xui_shadow_blur,
                __xui_shadow_spread,
            ));
        }),
        "no_shadow" => Some(quote! {
            if #value {
                __xui_style.merge(&::xui::Style::new().no_shadow());
            }
        }),
        "clip" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().clip(#value));
        }),
        _ => None,
    }
}

pub fn parse_scroll_style_attr<T: quote::ToTokens + ?Sized>(
    name: &str,
    value: &T,
) -> Option<TokenStream2> {
    match name {
        "scroll_direction" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().scroll_direction(#value));
        }),
        "scrollbar" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().scrollbar(#value));
        }),
        "scrollbar_width" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().scrollbar_width(#value));
        }),
        "scrollbar_track_color" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().scrollbar_track_color(#value));
        }),
        "scrollbar_thumb_color" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().scrollbar_thumb_color(#value));
        }),
        "scrollbar_radius" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().scrollbar_radius(#value));
        }),
        "scrollbar_visibility" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().scrollbar_visibility(#value));
        }),
        _ => None,
    }
}

pub fn event_attr_stmt(attr: &XuiAttribute) -> Option<TokenStream2> {
    let value = &attr.value;
    match attr.name.to_string().as_str() {
        "on_event" => Some(quote! { __xui_element = __xui_element.on_event(#value); }),
        "on_click" => Some(quote! { __xui_element = __xui_element.on_click(#value); }),
        "on_hover_change" => {
            Some(quote! { __xui_element = __xui_element.on_hover_change(#value); })
        }
        "on_pointer_down" => {
            Some(quote! { __xui_element = __xui_element.on_pointer_down(#value); })
        }
        "on_pointer_up" => Some(quote! { __xui_element = __xui_element.on_pointer_up(#value); }),
        "on_pointer_move" => {
            Some(quote! { __xui_element = __xui_element.on_pointer_move(#value); })
        }
        "on_key_down" => Some(quote! { __xui_element = __xui_element.on_key_down(#value); }),
        "on_key_up" => Some(quote! { __xui_element = __xui_element.on_key_up(#value); }),
        _ => None,
    }
}

pub fn unsupported_attr<T>(attr: &XuiAttribute, tag: &str, attr_name: &str) -> Result<T> {
    Err(Error::new(
        attr.name.span(),
        format!("unsupported attribute `{attr_name}` on <{tag}>"),
    ))
}

pub fn parse_stack_attr<T: quote::ToTokens + ?Sized>(
    name: &str,
    value: &T,
) -> Option<TokenStream2> {
    match name {
        "gap" => Some(quote! {
            __xui_style.merge(&::xui::Style::new().gap(#value));
        }),
        _ => None,
    }
}
