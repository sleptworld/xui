use crate::event_system::EventContext;
use xui_interface::{
    Bounds, Color, ComputedStyle, EventRef, EventResult, Key, Size, Style, TextContent, TextProps,
    WidgetType, WidgetUpdateFlags, core::Sizing,
};

use super::props_hash;
use crate::render::{Primitive, RenderTreeWriter, Shape, ShapePrimitive};

#[derive(Debug)]
pub struct RootWidget {
    pub style: Style,
}

impl Default for RootWidget {
    fn default() -> Self {
        Self {
            style: Style::new(),
        }
    }
}

impl RootWidget {
    pub fn set_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl RootWidget {
    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    pub(super) fn props_hash(&self) -> u64 {
        props_hash(&self.style)
    }

    pub(super) fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        if self.style != next.style {
            self.style = next.style.clone();
            WidgetUpdateFlags::STYLE_TARGET
        } else {
            WidgetUpdateFlags::empty()
        }
    }

    pub(super) fn default_style(&self) -> Style {
        Style::new().size(Size::<Sizing>::new(
            Sizing::Percent(1.0.try_into().unwrap()),
            Sizing::Percent(1.0.try_into().unwrap()),
        ))
    }

    pub(super) fn get_key(&self) -> Option<&Key> {
        None
    }

    pub(super) fn current_style(&self) -> &Style {
        &self.style
    }

    pub(super) fn render(
        &self,
        _node_id: xui_interface::NodeId,
        rect: Bounds,
        _style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        writer
            .primitive(Primitive::Shape(ShapePrimitive {
                bounds: rect,
                shape: Shape::Rect,
                fill: Some(xui_interface::ComputedColorStyle::Solid(Color::WHITE)),
                stroke: None,
                shadow: None,
            }))
            .expect("widget render tree must remain valid");
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

    no_event_handler_methods!();
}
