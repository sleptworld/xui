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
        "position" => Some(quote! {
            __xui_style = __xui_style.position_type(#value);
        }),
        "inset" => Some(quote! {
            __xui_style = __xui_style.inset(#value);
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
        "backdrop_blur" => Some(quote! {
            __xui_style = __xui_style.backdrop_blur(#value);
        }),
        _ => None,
    }
}

pub fn parse_transform_style_attr<T: quote::ToTokens + ?Sized>(
    name: &str,
    value: &T,
) -> Option<TokenStream2> {
    match name {
        "transform" => Some(quote! {
            __xui_style = __xui_style.transform(#value);
        }),
        "translate" => Some(quote! {
            __xui_style = __xui_style.translate(#value);
        }),
        "translate_x" => Some(quote! {
            __xui_style = __xui_style.translate_x(#value);
        }),
        "translate_y" => Some(quote! {
            __xui_style = __xui_style.translate_y(#value);
        }),
        "scale" => Some(quote! {
            __xui_style = __xui_style.scale(#value);
        }),
        "scale_xy" => Some(quote! {
            __xui_style = __xui_style.scale_xy(#value);
        }),
        "rotate" => Some(quote! {
            __xui_style = __xui_style.rotate(#value);
        }),
        "transform_origin" => Some(quote! {
            __xui_style = __xui_style.transform_origin(#value);
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
    parse_event_attr(attr.name.to_string().as_str(), &attr.value)
}

pub fn parse_event_attr<T: quote::ToTokens + ?Sized>(
    name: &str,
    value: &T,
) -> Option<TokenStream2> {
    match name {
        "on_event" => Some(quote! { __xui_element = __xui_element.on_event(#value); }),
        "on_event_capture" => {
            Some(quote! { __xui_element = __xui_element.on_event_capture(#value); })
        }
        "on_click" => Some(quote! { __xui_element = __xui_element.on_click(#value); }),
        "on_click_capture" => {
            Some(quote! { __xui_element = __xui_element.on_click_capture(#value); })
        }
        "on_double_click" => {
            Some(quote! { __xui_element = __xui_element.on_double_click(#value); })
        }
        "on_double_click_capture" => {
            Some(quote! { __xui_element = __xui_element.on_double_click_capture(#value); })
        }
        "on_context_menu" => {
            Some(quote! { __xui_element = __xui_element.on_context_menu(#value); })
        }
        "on_context_menu_capture" => {
            Some(quote! { __xui_element = __xui_element.on_context_menu_capture(#value); })
        }
        "on_hover_enter" => Some(quote! { __xui_element = __xui_element.on_hover_enter(#value); }),
        "on_hover_leave" => Some(quote! { __xui_element = __xui_element.on_hover_leave(#value); }),
        "on_hover_change" => {
            Some(quote! { __xui_element = __xui_element.on_hover_change(#value); })
        }
        "on_press_start" => Some(quote! { __xui_element = __xui_element.on_press_start(#value); }),
        "on_press_start_capture" => {
            Some(quote! { __xui_element = __xui_element.on_press_start_capture(#value); })
        }
        "on_press_end" => Some(quote! { __xui_element = __xui_element.on_press_end(#value); }),
        "on_press_end_capture" => {
            Some(quote! { __xui_element = __xui_element.on_press_end_capture(#value); })
        }
        "on_press_cancel" => {
            Some(quote! { __xui_element = __xui_element.on_press_cancel(#value); })
        }
        "on_press_cancel_capture" => {
            Some(quote! { __xui_element = __xui_element.on_press_cancel_capture(#value); })
        }
        "on_focus" => Some(quote! { __xui_element = __xui_element.on_focus(#value); }),
        "on_blur" => Some(quote! { __xui_element = __xui_element.on_blur(#value); }),
        "on_focus_in" => Some(quote! { __xui_element = __xui_element.on_focus_in(#value); }),
        "on_focus_in_capture" => {
            Some(quote! { __xui_element = __xui_element.on_focus_in_capture(#value); })
        }
        "on_focus_out" => Some(quote! { __xui_element = __xui_element.on_focus_out(#value); }),
        "on_focus_out_capture" => {
            Some(quote! { __xui_element = __xui_element.on_focus_out_capture(#value); })
        }
        "on_drag_start" => Some(quote! { __xui_element = __xui_element.on_drag_start(#value); }),
        "on_drag_start_capture" => {
            Some(quote! { __xui_element = __xui_element.on_drag_start_capture(#value); })
        }
        "on_drag_move" => Some(quote! { __xui_element = __xui_element.on_drag_move(#value); }),
        "on_drag_move_capture" => {
            Some(quote! { __xui_element = __xui_element.on_drag_move_capture(#value); })
        }
        "on_drag_end" => Some(quote! { __xui_element = __xui_element.on_drag_end(#value); }),
        "on_drag_end_capture" => {
            Some(quote! { __xui_element = __xui_element.on_drag_end_capture(#value); })
        }
        "on_drag_cancel" => Some(quote! { __xui_element = __xui_element.on_drag_cancel(#value); }),
        "on_drag_cancel_capture" => {
            Some(quote! { __xui_element = __xui_element.on_drag_cancel_capture(#value); })
        }
        "on_scroll" => Some(quote! { __xui_element = __xui_element.on_scroll(#value); }),
        "on_scroll_capture" => {
            Some(quote! { __xui_element = __xui_element.on_scroll_capture(#value); })
        }
        _ => None,
    }
}

pub fn unsupported_attr<T>(attr: &XuiAttribute, tag: &str, attr_name: &str) -> Result<T> {
    Err(Error::new(
        attr.name.span(),
        format!("unsupported attribute `{attr_name}` on <{tag}>"),
    ))
}
