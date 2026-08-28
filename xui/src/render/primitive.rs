use xui_interface::{
    Affine, Bounds, ComputedColorStyle, ComputedShadowStyle, ComputedStrokeStyle, ImageData,
    ImageKey, ImageStyle, ImageVariant, NodeId, Point, TextPaintProps, TextVerticalAlign,
    VectorScene,
};

use crate::text::TextLayoutSlot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrimitiveChange {
    pub geometry: bool,
    pub paint: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    Shape(ShapePrimitive),
    Vector(VectorPrimitive),
    Image(ImagePrimitive),
    Text(TextPrimitive),
}

impl Primitive {
    pub fn local_bounds(&self) -> Bounds {
        match self {
            Self::Shape(value) => value.bounds,
            Self::Vector(value) => value.transform.transform_bounds(value.scene.bounds()),
            Self::Image(value) => value.bounds,
            Self::Text(value) => value.bounds,
        }
    }

    pub fn paint_bounds(&self) -> Bounds {
        match self {
            Self::Shape(value) => shape_paint_bounds(value),
            Self::Vector(value) => value.transform.transform_bounds(value.scene.bounds()),
            Self::Image(value) => value.bounds,
            Self::Text(value) => value.bounds,
        }
    }

    pub fn diff(&self, next: &Self) -> PrimitiveChange {
        if self == next {
            return PrimitiveChange::default();
        }
        match (self, next) {
            (Self::Shape(a), Self::Shape(b)) => PrimitiveChange {
                geometry: a.bounds != b.bounds || a.shape != b.shape,
                paint: a.fill != b.fill || a.stroke != b.stroke || a.shadow != b.shadow,
            },
            (Self::Vector(a), Self::Vector(b)) => {
                let scene = a.scene.diff(&b.scene);
                PrimitiveChange {
                    geometry: a.transform != b.transform || scene.geometry,
                    paint: scene.paint,
                }
            }
            (Self::Image(a), Self::Image(b)) => PrimitiveChange {
                geometry: a.bounds != b.bounds,
                paint: a.image != b.image
                    || a.data != b.data
                    || a.variant != b.variant
                    || a.style != b.style
                    || a.opacity != b.opacity,
            },
            (Self::Text(a), Self::Text(b)) => PrimitiveChange {
                geometry: a.bounds != b.bounds
                    || a.slot != b.slot
                    || a.layout_revision != b.layout_revision
                    || a.vertical_align != b.vertical_align,
                paint: a.node_id != b.node_id
                    || a.slot != b.slot
                    || a.layout_revision != b.layout_revision
                    || a.paint != b.paint,
            },
            _ => PrimitiveChange {
                geometry: true,
                paint: true,
            },
        }
    }
}

fn shape_paint_bounds(shape: &ShapePrimitive) -> Bounds {
    let stroke_bounds = shape.bounds.expand(
        shape
            .stroke
            .as_ref()
            .map(|stroke| stroke.width * 0.5)
            .unwrap_or(0.0),
    );
    let Some(shadow) = shape.shadow else {
        return stroke_bounds;
    };
    let shadow_bounds = shape
        .bounds
        .expand(shadow.spread.max(0.0) + shadow.blur.max(0.0) * 3.0)
        .translate(shadow.offset);
    stroke_bounds.union(shadow_bounds)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapePrimitive {
    pub bounds: Bounds,
    pub shape: Shape,
    pub fill: Option<ComputedColorStyle>,
    pub stroke: Option<ComputedStrokeStyle>,
    pub shadow: Option<ComputedShadowStyle>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Rect,
    RoundedRect(f32),
    Circle,
    Ellipse,
    Line { from: Point, to: Point },
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorPrimitive {
    pub scene: VectorScene,
    pub transform: Affine,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImagePrimitive {
    pub bounds: Bounds,
    pub image: ImageKey,
    pub data: ImageData,
    pub variant: ImageVariant,
    pub style: ImageStyle,
    pub opacity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextPrimitive {
    pub bounds: Bounds,
    pub node_id: NodeId,
    pub slot: TextLayoutSlot,
    pub layout_revision: u64,
    pub vertical_align: TextVerticalAlign,
    pub paint: TextPaintProps,
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{Color, ComputedShadowStyle, Size};

    #[test]
    fn paint_bounds_include_asymmetric_shadow_and_stroke() {
        let primitive = Primitive::Shape(ShapePrimitive {
            bounds: Bounds::from_origin_size(Point::new(10.0, 10.0), Size::new(20.0, 20.0)),
            shape: Shape::Rect,
            fill: None,
            stroke: Some(ComputedStrokeStyle {
                color: ComputedColorStyle::Solid(Color::BLACK),
                width: 4.0,
                line_style: xui_interface::StrokeLineStyle::Solid,
            }),
            shadow: Some(ComputedShadowStyle {
                color: Color::BLACK,
                offset: Point::new(8.0, -3.0),
                blur: 2.0,
                spread: 1.0,
            }),
        });
        assert_eq!(
            primitive.paint_bounds(),
            Bounds::from_origin_size((8.0, 0.0), (37.0, 34.0))
        );
    }
}
