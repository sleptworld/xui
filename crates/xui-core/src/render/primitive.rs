use std::sync::Arc;

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
    Batch(BatchPrimitive),
}

impl Primitive {
    pub fn local_bounds(&self) -> Bounds {
        match self {
            Self::Shape(value) => value.bounds,
            Self::Vector(value) => value.transform.transform_bounds(value.scene.bounds()),
            Self::Image(value) => value.bounds,
            Self::Text(value) => value.bounds,
            Self::Batch(value) => value.paint_bounds(),
        }
    }

    pub fn paint_bounds(&self) -> Bounds {
        match self {
            Self::Shape(value) => shape_paint_bounds(value),
            Self::Vector(value) => value.transform.transform_bounds(value.scene.bounds()),
            Self::Image(value) => value.bounds,
            Self::Text(value) => value.bounds,
            Self::Batch(value) => value.paint_bounds(),
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
            (Self::Batch(a), Self::Batch(b)) => a.diff(b),
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

// `Shape` itself now lives in `xui-interface`, next to `VectorCommand`, so a
// canvas scene can name the same analytic shapes this primitive draws.
pub use xui_interface::Shape;

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

/// An ordered run of primitives painted from one render node.
///
/// For content that draws many items but never needs them addressed one by
/// one -- a canvas is the case this exists for. Giving every item a node of
/// its own makes the scene grow with the picture, and every per-node cost in
/// the scene (dirty summaries, child scans, reconciliation) grows with it. A
/// batch keeps that at one leaf; the frame builder expands it into one draw per
/// item, so backends see exactly what separate nodes would have produced.
///
/// Items are in batch-local coordinates and placed at `origin` in the node's
/// space, so moving the node leaves the shared item list untouched.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchPrimitive {
    pub content: Arc<BatchContent>,
    pub origin: Point,
}

impl BatchPrimitive {
    pub fn new(content: Arc<BatchContent>, origin: Point) -> Self {
        Self { content, origin }
    }

    /// The transform from batch-local coordinates to the node's space.
    pub fn transform(&self) -> Affine {
        Affine::translate(self.origin.x, self.origin.y)
    }

    fn paint_bounds(&self) -> Bounds {
        self.content.paint_bounds.translate(self.origin)
    }

    fn diff(&self, next: &Self) -> PrimitiveChange {
        let moved = self.origin != next.origin;
        if Arc::ptr_eq(&self.content, &next.content) {
            return PrimitiveChange {
                geometry: moved,
                paint: false,
            };
        }
        let (a, b) = (&self.content.items, &next.content.items);
        if a.len() != b.len() {
            return PrimitiveChange {
                geometry: true,
                paint: true,
            };
        }
        a.iter().zip(b.iter()).fold(
            PrimitiveChange {
                geometry: moved,
                paint: false,
            },
            |mut change, (a, b)| {
                let item = a.primitive.diff(&b.primitive);
                change.geometry |= item.geometry || a.clip != b.clip;
                change.paint |= item.paint;
                change
            },
        )
    }
}

/// The items of a [`BatchPrimitive`], with their bounds summarized once.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchContent {
    items: Vec<BatchItem>,
    paint_bounds: Bounds,
}

impl BatchContent {
    pub fn new(items: Vec<BatchItem>) -> Self {
        debug_assert!(
            items
                .iter()
                .all(|item| !matches!(item.primitive, Primitive::Batch(_))),
            "batches do not nest"
        );
        let paint_bounds = items
            .iter()
            .map(BatchItem::paint_bounds)
            .reduce(Bounds::union)
            .unwrap_or(Bounds::ZERO);
        Self {
            items,
            paint_bounds,
        }
    }

    pub fn items(&self) -> &[BatchItem] {
        &self.items
    }
}

/// One primitive of a batch, optionally clipped to a rectangle of its own.
///
/// The clip is resolved by the frame builder into a clip chain under the
/// node's, so an item that needs one does not need a clip node -- and with it
/// a node -- of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchItem {
    pub primitive: Primitive,
    pub clip: Option<Bounds>,
}

impl BatchItem {
    pub fn new(primitive: Primitive) -> Self {
        Self {
            primitive,
            clip: None,
        }
    }

    pub fn clipped(primitive: Primitive, clip: Bounds) -> Self {
        Self {
            primitive,
            clip: Some(clip),
        }
    }

    fn paint_bounds(&self) -> Bounds {
        let bounds = self.primitive.paint_bounds();
        match self.clip {
            Some(clip) => (bounds & clip).unwrap_or(Bounds::ZERO),
            None => bounds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{Color, ComputedShadowStyle, Point, Size};

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
