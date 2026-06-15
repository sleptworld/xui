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

pub fn parse_animation_attr<T: quote::ToTokens + ?Sized>(
    name: &str,
    value: &T,
) -> Option<TokenStream2> {
    match name {
        "animated_style" => Some(quote! {
            __xui_animated_style = #value;
            __xui_has_animated_style = true;
        }),
        "animation" => Some(quote! {
            let (__xui_animation_trigger, __xui_animation_style, __xui_animation_transition) = #value;
            __xui_animated_style = __xui_animated_style.animation(
                __xui_animation_trigger,
                __xui_animation_style,
                __xui_animation_transition,
            );
            __xui_has_animated_style = true;
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
            __xui_style =  __xui_style.color(#value);
        }),
        "font_family" => Some(quote! {
            __xui_style =  __xui_style.font_family(#value);
        }),
        "font_size" => Some(quote! {
            __xui_style =  __xui_style.font_size(#value);
        }),
        "font_weight" => Some(quote! {
            __xui_style =  __xui_style.font_weight(#value);
        }),
        "font_style" => Some(quote! {
            __xui_style =  __xui_style.font_style(#value);
        }),
        "line_height" => Some(quote! {
            __xui_style =  __xui_style.line_height(#value);
        }),
        "letter_spacing" => Some(quote! {
            __xui_style =  __xui_style.letter_spacing(#value);
        }),
        "decoration" => Some(quote! {
            __xui_style =  __xui_style.decoration(#value);
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
            __xui_style =  __xui_style.flex_direction(#value);
        }),
        "gap" => Some(quote! {
            __xui_style =  __xui_style.gap(#value);
        }),
        "padding" => Some(quote! {
            __xui_style =  __xui_style.padding(#value);
        }),
        "size" => Some(quote! {
            __xui_style = __xui_style.size(#value);
        }),
        "min_size" => Some(quote! {
            __xui_style =  __xui_style.min_size(#value);
        }),
        "max_size" => Some(quote! {
            __xui_style =  __xui_style.max_size(#value);
        }),
        "margin" => Some(quote! {
            __xui_style =  __xui_style.margin(#value);
        }),
        "align" => Some(quote! {
            __xui_style =  __xui_style.align(#value);
        }),
        "justify" => Some(quote! {
            __xui_style =  __xui_style.justify(#value);
        }),
        "width" => Some(quote! {
            __xui_style = __xui_style.width(#value);

        }),
        "height" => Some(quote! {
            __xui_style = __xui_style.height(#value);
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
            __xui_style = __xui_style.background(#value);
        }),
        "border_color" => Some(quote! {
            __xui_style =  __xui_style.border_color(#value);
        }),
        "border_width" => Some(quote! {
            __xui_style =  __xui_style.border_width(#value);
        }),
        "border_radius" => Some(quote! {
            __xui_style =  __xui_style.border_radius(#value);
        }),
        "stroke" => Some(quote! {
            __xui_style =  __xui_style.stroke(#value);
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
                __xui_style =  __xui_style.no_stroke();
            }
        }),
        "shadow" => Some(quote! {
            __xui_style =  __xui_style.shadow(#value);
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
                __xui_style =  __xui_style.no_shadow();
            }
        }),
        "clip" => Some(quote! {
            __xui_style =  __xui_style.clip(#value);
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
            __xui_style =  __xui_style.scroll_direction(#value);
        }),
        "scrollbar" => Some(quote! {
            __xui_style =  __xui_style.scrollbar(#value);
        }),
        "scrollbar_width" => Some(quote! {
            __xui_style =  __xui_style.scrollbar_width(#value);
        }),
        "scrollbar_track_color" => Some(quote! {
            __xui_style =  __xui_style.scrollbar_track_color(#value);
        }),
        "scrollbar_thumb_color" => Some(quote! {
            __xui_style =  __xui_style.scrollbar_thumb_color(#value);
        }),
        "scrollbar_radius" => Some(quote! {
            __xui_style =  __xui_style.scrollbar_radius(#value);
        }),
        "scrollbar_visibility" => Some(quote! {
            __xui_style =  __xui_style.scrollbar_visibility(#value);
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
            __xui_style =  __xui_style.gap(#value);
        }),
        _ => None,
    }
}
