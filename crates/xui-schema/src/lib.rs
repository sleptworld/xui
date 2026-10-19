//! The host vocabulary that `xui-macros` and `xui-core` must agree on.
//!
//! `view!` / `xui!` build a lowercase host tag at a fixed call site, so the
//! macro can split its attributes into the part that never changes and the
//! part that does (`docs/template-slots.md` §12). To do that it has to know a
//! little about the vocabulary: which tags are hosts, and which attributes are
//! plain style setters. That knowledge lives here, in data only, so the macro
//! never reads the runtime's source and the runtime never depends on the macro.
//! `xui-core`'s tests check this list against the code it describes.

#![forbid(unsafe_code)]

/// What a host tag's body means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostBody {
    /// Children, given as `{ .. }` or as one `IntoChildren` content value.
    Children,
    /// `text("..")`: the content is the text.
    Text,
}

/// A lowercase tag whose elements can be templated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostTag {
    /// The constructor's name, `row` for `row()`.
    pub name: &'static str,
    /// The widget the constructor returns, under `xui_core::widgets`.
    pub widget: &'static str,
    pub body: HostBody,
}

/// The host tags the macro templates. Others (`canvas`, `image`, `icon`,
/// `text_input`, and any third-party lowercase tag) are built as before; they
/// either have per-instance state in their constructor or inherent attributes
/// that shadow style properties with a different meaning.
pub const HOST_TAGS: &[HostTag] = &[
    HostTag {
        name: "container",
        widget: "ContainerWidget",
        body: HostBody::Children,
    },
    HostTag {
        name: "row",
        widget: "ContainerWidget",
        body: HostBody::Children,
    },
    HostTag {
        name: "column",
        widget: "ContainerWidget",
        body: HostBody::Children,
    },
    HostTag {
        name: "center",
        widget: "ContainerWidget",
        body: HostBody::Children,
    },
    HostTag {
        name: "z_stack",
        widget: "ZStackWidget",
        body: HostBody::Children,
    },
    HostTag {
        name: "grid",
        widget: "GridWidget",
        body: HostBody::Children,
    },
    HostTag {
        name: "text",
        widget: "TextWidget",
        body: HostBody::Text,
    },
];

/// Style attributes whose setter only assigns fields (`StyleValue::Value`), so
/// applying several of them to an empty `Style` and merging that over the
/// widget's style is the same as calling them on the widget in order.
///
/// Every entry of `xui_core::dsl`'s `style_props!` table except `scrollbar`,
/// which assigns a whole `ScrollbarStylePatch` (unset fields included) and so
/// is not equivalent to a merge.
pub const STYLE_PROPS: &[&str] = &[
    // text
    "color",
    "font_family",
    "font_size",
    "font_weight",
    "font_style",
    "line_height",
    "letter_spacing",
    "decoration",
    // layout
    "gap",
    "size",
    "width",
    "height",
    "min_size",
    "min_width",
    "min_height",
    "max_size",
    "max_width",
    "max_height",
    "margin",
    "padding",
    "align",
    "justify",
    "inset",
    "position",
    "display",
    // transform
    "transform",
    "translate",
    "translate_x",
    "translate_y",
    "scale",
    "scale_xy",
    "rotate",
    "transform_origin",
    // paint
    "background",
    "border_color",
    "border_width",
    "border_radius",
    "stroke",
    "shadow",
    "clip",
    "visibility",
    "cursor",
    "backdrop_blur",
    "backdrop_style",
    "effects",
    // scroll
    "scroll_direction",
    "scrollbar_style",
    "scrollbar_width",
    "scrollbar_track_color",
    "scrollbar_thumb_color",
    "scrollbar_radius",
    "scrollbar_visibility",
    // animation
    "transition",
];

/// Attributes that rewrite a widget's style wholesale rather than set fields:
/// `style` merges a whole `Style` (rules included), `scrollbar` assigns a whole
/// patch. Their place among the other attributes matters, so a templated
/// element only takes them as static values written before every changing
/// style attribute.
pub const ORDERED_STYLE_ATTRS: &[&str] = &["style", "scrollbar"];

/// Looks a host tag up by its constructor name.
pub fn host_tag(name: &str) -> Option<&'static HostTag> {
    HOST_TAGS.iter().find(|tag| tag.name == name)
}
