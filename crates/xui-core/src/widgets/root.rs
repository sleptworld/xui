use xui_interface::{
    Bounds, Color, ComputedStyle, Size, Style, WidgetType, WidgetUpdateFlags, core::Sizing,
};

use super::{HostWidget, props_hash, sync};
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

impl HostWidget for RootWidget {
    const TYPE: WidgetType = WidgetType::Container;

    fn current_style(&self) -> &Style {
        &self.style
    }

    fn style_mut(&mut self) -> Option<&mut Style> {
        Some(&mut self.style)
    }

    fn default_style(&self) -> Style {
        Style::new().size(Size::<Sizing>::new(
            Sizing::Percent(1.0.try_into().unwrap()),
            Sizing::Percent(1.0.try_into().unwrap()),
        ))
    }

    fn props_hash(&self) -> u64 {
        props_hash(&self.style)
    }

    fn instantiate(&self) -> Self {
        Self {
            style: self.style.clone(),
        }
    }

    fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        sync(
            &mut self.style,
            &next.style,
            WidgetUpdateFlags::STYLE_TARGET,
        )
    }

    fn render(
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
}
