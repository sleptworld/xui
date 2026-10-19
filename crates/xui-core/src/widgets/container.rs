use crate::element::ElementDesc;
use crate::event_system::callbacks::EventHandlers;
use crate::event_system::interaction::InteractionProperties;
use crate::widgets::utils::render_box;
use xui_interface::Bounds;
use xui_interface::style::FlexDirectionStyle;
use xui_interface::{
    ColorStyle, ComputedStyle, Key, LengthValue, ScrollDirectionStyle, Style, WidgetType,
    WidgetUpdateFlags, style::ScrollbarStylePatch,
};

use super::{HostWidget, props_hash, sync, widget_element_desc};
use crate::render::RenderTreeWriter;

pub struct ContainerWidget {
    pub key: Option<Key>,
    pub style: Style,
    pub flex_direction: Option<FlexDirectionStyle>,
    /// Whether a `row`/`column` wraps children onto new lines when they
    /// overflow, like CSS `flex-wrap: wrap`. Ignored without a flex direction.
    pub flex_wrap: bool,
    pub event_handlers: EventHandlers,
    pub interaction: InteractionProperties,
}

impl std::fmt::Debug for ContainerWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerWidget")
            .field("key", &self.key)
            .field("flex_direction", &self.flex_direction)
            .field("flex_wrap", &self.flex_wrap)
            .finish()
    }
}

impl ContainerWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            style: Style::default(),
            flex_direction: None,
            flex_wrap: false,
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

    pub fn flex_direction(mut self, direction: FlexDirectionStyle) -> Self {
        self.flex_direction = Some(direction);
        self
    }

    /// Wraps overflowing children onto new lines; `gap` then spaces the lines
    /// as well as the items within a line.
    pub fn flex_wrap(mut self, wrap: bool) -> Self {
        self.flex_wrap = wrap;
        self
    }

    pub fn scrollable(mut self) -> Self {
        let style = self.style.scroll_vertical();
        self.style = style;
        self
    }

    pub fn scroll_direction(mut self, direction: ScrollDirectionStyle) -> Self {
        let style = self.style.scroll_direction(direction);
        self.style = style;
        self
    }

    pub fn scrollbar(mut self, scrollbar: ScrollbarStylePatch) -> Self {
        let style = self.style.scrollbar(scrollbar);
        self.style = style;
        self
    }

    pub fn scrollbar_width(mut self, width: impl Into<LengthValue>) -> Self {
        let style = self.style.scrollbar_width(width);
        self.style = style;
        self
    }

    pub fn scrollbar_track_color(mut self, color: impl Into<ColorStyle>) -> Self {
        let style = self.style.scrollbar_track_color(color);
        self.style = style;
        self
    }

    pub fn scrollbar_thumb_color(mut self, color: impl Into<ColorStyle>) -> Self {
        let style = self.style.scrollbar_thumb_color(color);
        self.style = style;
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
}

impl Default for ContainerWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl HostWidget for ContainerWidget {
    const TYPE: WidgetType = WidgetType::Container;

    fn get_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn current_style(&self) -> &Style {
        &self.style
    }

    fn style_mut(&mut self) -> Option<&mut Style> {
        Some(&mut self.style)
    }

    fn set_key(&mut self, key: Option<Key>) {
        self.key = key;
    }

    fn props_hash(&self) -> u64 {
        props_hash(&(&self.style, self.flex_direction, self.flex_wrap))
    }

    fn instantiate(&self) -> Self {
        Self {
            key: self.key.clone(),
            style: self.style.clone(),
            flex_direction: self.flex_direction,
            flex_wrap: self.flex_wrap,
            event_handlers: EventHandlers::default(),
            interaction: InteractionProperties::default(),
        }
    }

    fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        sync(
            &mut self.style,
            &next.style,
            WidgetUpdateFlags::STYLE_TARGET,
        ) | sync(
            &mut self.flex_direction,
            &next.flex_direction,
            WidgetUpdateFlags::LAYOUT_INPUT,
        ) | sync(
            &mut self.flex_wrap,
            &next.flex_wrap,
            WidgetUpdateFlags::LAYOUT_INPUT,
        )
    }

    fn render(
        &self,
        _node_id: xui_interface::NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        render_box(rect, style, writer);
    }

    fn interaction(&self) -> Option<(&InteractionProperties, &EventHandlers)> {
        Some((&self.interaction, &self.event_handlers))
    }

    fn interaction_mut(&mut self) -> Option<(&mut InteractionProperties, &mut EventHandlers)> {
        Some((&mut self.interaction, &mut self.event_handlers))
    }
}
