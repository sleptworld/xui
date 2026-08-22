use crate::render::{Primitive, RenderTreeWriter, Shape, ShapePrimitive};
use xui_interface::{Bounds, ComputedStyle};

pub(super) fn render_box(rect: Bounds, style: &ComputedStyle, writer: &mut RenderTreeWriter<'_>) {
    let paint = style.paint;
    let shape = if paint.border_radius > 0.0 {
        Shape::RoundedRect(paint.border_radius)
    } else {
        Shape::Rect
    };
    writer
        .primitive(Primitive::Shape(ShapePrimitive {
            bounds: rect,
            shape,
            fill: Some(paint.background),
            stroke: paint.stroke,
            shadow: paint.shadow,
        }))
        .expect("widget render tree must remain valid");
}
