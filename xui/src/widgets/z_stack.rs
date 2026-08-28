use crate::element::ElementDesc;
use crate::event_system::EventContext;
use crate::event_system::callbacks::EventHandlers;
use crate::event_system::interaction::InteractionProperties;
use crate::render::RenderTreeWriter;
use xui_interface::{
    Alignment, Bounds, ComputedStyle, EventRef, EventResult, Key, Style, TextContent,
    TextProps, WidgetType, WidgetUpdateFlags,
};

use super::utils::render_box;
use super::{props_hash, widget_element_desc};

/// A SwiftUI-style overlay container. Children share one Taffy grid cell and
/// are painted in declaration order, with later children appearing on top.
pub struct ZStackWidget {
    pub key: Option<Key>,
    pub style: Style,
    pub alignment: Alignment,
    pub event_handlers: EventHandlers,
    pub interaction: InteractionProperties,
}

impl std::fmt::Debug for ZStackWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZStackWidget")
            .field("key", &self.key)
            .field("alignment", &self.alignment)
            .finish()
    }
}

impl ZStackWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            style: Style::default(),
            alignment: Alignment::CENTER,
            event_handlers: EventHandlers::default(),
            interaction: InteractionProperties::default(),
        }
    }

    /// Merges a style in, rather than replacing what is already there.
    ///
    /// On a fresh builder — how nearly every call site uses it — merging into an
    /// all-unset style is indistinguishable from assignment. The difference
    /// shows in the `xui!` macro, where `style={..}` is one attribute among
    /// many: assignment made `<column padding={..} style={..} />` silently
    /// discard the padding, and whether an attribute survived depended on
    /// whether it was written before or after `style`.
    pub fn style(mut self, style: impl xui_interface::StyleMerge) -> Self {
        self.style.merge(&style);
        self
    }

    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn into_element_desc(self, children: Vec<ElementDesc>) -> ElementDesc {
        widget_element_desc(self, children)
    }

    event_handler_methods!();

    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::ZStack
    }

    pub(super) fn get_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    pub(super) fn props_hash(&self) -> u64 {
        props_hash(&(&self.style, self.alignment))
    }

    pub(super) fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();
        if self.style != next.style {
            self.style = next.style.clone();
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }
        if self.alignment != next.alignment {
            self.alignment = next.alignment;
            flags |= WidgetUpdateFlags::LAYOUT_INPUT;
        }
        flags
    }

    pub(super) fn default_style(&self) -> Style {
        Style::new()
    }

    pub(super) fn current_style(&self) -> &Style {
        &self.style
    }

    pub(super) fn render(
        &self,
        _node_id: xui_interface::NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        render_box(rect, style, writer);
    }

    pub(super) fn handle_event(
        &mut self,
        _event: EventRef<'_>,
        _cx: &mut EventContext<'_>,
    ) -> EventResult {
        EventResult::Ignored
    }

    pub(super) fn text_content(&self) -> Option<TextContent> {
        None
    }

    pub(super) fn text_layout_props(&self, _style: &ComputedStyle) -> Option<TextProps> {
        None
    }
}

impl Default for ZStackWidget {
    fn default() -> Self {
        Self::new()
    }
}
