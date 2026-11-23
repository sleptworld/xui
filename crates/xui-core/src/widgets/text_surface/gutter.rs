//! A fixed-width margin sharing the surface's shaped visual lines.
use super::{BlockId, handle_revision};
use crate::render::{Primitive, Shape, ShapePrimitive, TextPrimitive};
use crate::text::{TextHost, TextLayoutSlot};
use std::{fmt, rc::Rc};
use xui_interface::events::PointerButton;
use xui_interface::{
    Bounds, ComputedColorStyle, ComputedTextStyle, NodeId, TextBackend, TextLayoutConstraints,
    TextLayoutInput, TextPaintProps, TextPaintStyle, TextVerticalAlign,
};
use xui_interface::{Color, Modifiers, Point, Rect, TextContent};

/// A visible visual line in gutter-local logical pixels. Indices are zero-based.
/// A partially visible line can have a negative top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GutterLine {
    pub block: BlockId,
    pub block_index: usize,
    pub visual_line: usize,
    pub rect: Rect,
    pub baseline: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct GutterPress {
    pub line: GutterLine,
    pub point: Point,
    pub button: PointerButton,
    pub modifiers: Modifiers,
}

/// Commands use gutter-local coordinates. Labels inherit the surface typography.
pub enum GutterItem {
    Fill(Rect, Color),
    Vector(crate::render::VectorPrimitive),
    Label {
        x: f32,
        baseline: f32,
        text: TextContent,
        color: Color,
    },
}

pub trait GutterRenderer {
    fn draw(&self, line: GutterLine, items: &mut Vec<GutterItem>);
    /// Runs in the event phase. The surface repaints after the press.
    fn press(&self, _press: GutterPress) {}
}

/// Per-view gutter configuration. Keep a custom renderer across renders.
#[derive(Clone)]
pub struct SurfaceGutter {
    pub width: f32,
    pub(super) renderer: Rc<dyn GutterRenderer>,
}

impl SurfaceGutter {
    pub fn new(width: f32, renderer: impl GutterRenderer + 'static) -> Self {
        Self {
            width,
            renderer: Rc::new(renderer),
        }
    }
    pub fn line_numbers(width: f32, color: Color) -> Self {
        struct Numbers(Color);
        impl GutterRenderer for Numbers {
            fn draw(&self, line: GutterLine, items: &mut Vec<GutterItem>) {
                if line.visual_line == 0 {
                    items.push(GutterItem::Label {
                        x: 2.0,
                        baseline: line.baseline,
                        text: TextContent::from((line.block_index + 1).to_string()),
                        color: self.0,
                    });
                }
            }
        }
        Self::new(width, Numbers(color))
    }
}

impl PartialEq for SurfaceGutter {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && Rc::ptr_eq(&self.renderer, &other.renderer)
    }
}
impl fmt::Debug for SurfaceGutter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SurfaceGutter")
            .field("width", &self.width)
            .finish_non_exhaustive()
    }
}

/// Prepare labels with the same text backend as the document. Reuse slots by
/// visible label index so scrolling does not retain one slot per visited line.
pub(super) fn prepare_paint<T: TextBackend>(
    owner: NodeId,
    measurer: &mut TextHost<T>,
    text_style: &ComputedTextStyle,
    gutter_items: Vec<GutterItem>,
    paint: &mut Vec<Primitive>,
) {
    if gutter_items.is_empty() && paint.is_empty() {
        return;
    }
    let font_context = measurer.font_epoch();
    let paint_style = TextPaintStyle::from_computed(text_style);
    paint.clear();
    let mut slots = Vec::new();
    for item in gutter_items {
        let primitive = match item {
            GutterItem::Vector(vector) => Primitive::Vector(vector),
            GutterItem::Fill(rect, color) => Primitive::Shape(ShapePrimitive {
                bounds: Bounds::from_origin_size((rect.x, rect.y), (rect.width, rect.height)),
                shape: Shape::Rect,
                fill: Some(ComputedColorStyle::Solid(color)),
                stroke: None,
                shadow: None,
            }),
            GutterItem::Label {
                x,
                baseline,
                text,
                color,
            } => {
                let slot = TextLayoutSlot::new(slots.len() as u32 + 1);
                slots.push(slot);
                let input = TextLayoutInput::new(
                    text,
                    TextLayoutConstraints::UNBOUNDED,
                    text_style.clone(),
                    Default::default(),
                    Default::default(),
                    font_context,
                );
                let handle = measurer.activate_slot(owner, slot, input);
                let Some(layout) = measurer.layout(handle) else {
                    continue;
                };
                let metrics = layout.metrics();
                Primitive::Text(TextPrimitive {
                    node_id: owner,
                    bounds: Bounds::from_origin_size(
                        (x, baseline - metrics.first_baseline.unwrap_or(0.0)),
                        metrics.size,
                    ),
                    unit: slot.into(),
                    layout_revision: handle_revision(handle),
                    vertical_align: TextVerticalAlign::Top,
                    paint: TextPaintProps::new(TextPaintStyle {
                        color,
                        ..paint_style
                    }),
                })
            }
        };
        paint.push(primitive);
    }
    measurer.retain_direct_slots(owner, slots);
}
