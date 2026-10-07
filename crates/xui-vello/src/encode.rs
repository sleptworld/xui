//! `BuiltFrame` -> `vello::Scene`.
//!
//! Vello draws everything the frame describes, not just paths. This is the
//! piece that grows: each `BuiltDraw` arm below is a place where the scene
//! gains a primitive, and each `Unsupported` is a known gap.

use vello::{
    Scene,
    kurbo::{Affine as KurboAffine, BezPath, Cap, Circle, Ellipse, Join, Line, Rect as KurboRect, RoundedRect, Shape as _, Stroke},
    peniko::{
        Fill,
        color::{AlphaColor, Srgb},
    },
};
use xui_core::render::{
    BuiltDraw, BuiltFrame, BuiltItem, BuiltLayerId, ShapePrimitive, VectorPrimitive,
};
use xui_interface::{
    Affine, Bounds, Color, ComputedColorStyle, FillRule, LineCap, LineJoin, PathData, PathSegment,
    Shape, VectorCommand,
};

/// Things the frame asked for that this backend does not draw yet, so the
/// caller can report each once instead of every frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported {
    pub text: bool,
    pub image: bool,
    pub nested_layer: bool,
    pub gradient: bool,
    pub shadow: bool,
    pub texture: bool,
    pub path_clip: bool,
}

impl Unsupported {
    pub fn merge(&mut self, other: Self) {
        self.text |= other.text;
        self.image |= other.image;
        self.nested_layer |= other.nested_layer;
        self.gradient |= other.gradient;
        self.shadow |= other.shadow;
        self.texture |= other.texture;
        self.path_clip |= other.path_clip;
    }
}

/// Encodes the root layer of `frame` into `scene`, in physical pixels.
pub fn encode_frame(frame: &BuiltFrame, scale_factor: f32, scene: &mut Scene) -> Unsupported {
    let mut unsupported = Unsupported::default();
    let physical = KurboAffine::scale(scale_factor as f64);
    encode_layer(frame, frame.root_layer, physical, scene, &mut unsupported);
    unsupported
}

fn encode_layer(
    frame: &BuiltFrame,
    layer: BuiltLayerId,
    physical: KurboAffine,
    scene: &mut Scene,
    unsupported: &mut Unsupported,
) {
    for item in &frame.layers[layer.0].items {
        match item {
            BuiltItem::Draw(draw) => {
                let common = draw.common();
                let clip = common.clip_chain.map(|id| frame.clip_chains[id.0].world_bounds);
                if let Some(clip) = clip {
                    scene.push_clip_layer(Fill::NonZero, physical, &kurbo_bounds(clip));
                }
                let world = physical * kurbo_affine(common.world_transform);
                match draw {
                    BuiltDraw::Shape(value) => encode_shape(scene, world, &value.primitive, unsupported),
                    BuiltDraw::Vector(value) => encode_vector(scene, world, &value.primitive, unsupported),
                    BuiltDraw::Text(_) => unsupported.text = true,
                    BuiltDraw::Image(_) => unsupported.image = true,
                }
                if clip.is_some() {
                    scene.pop_layer();
                }
            }
            // A nested layer carries its own effect program (blur, backdrop,
            // masks, ...). Those are render-graph passes, which run outside
            // Vello; see the module docs in `lib.rs`.
            BuiltItem::Layer(_) => unsupported.nested_layer = true,
        }
    }
}

fn encode_shape(scene: &mut Scene, world: KurboAffine, shape: &ShapePrimitive, unsupported: &mut Unsupported) {
    if shape.shadow.is_some() {
        unsupported.shadow = true;
    }
    let bounds = kurbo_bounds(shape.bounds);
    let path = match shape.shape {
        Shape::Rect => bounds.to_path(0.1),
        Shape::RoundedRect(radius) => RoundedRect::from_rect(bounds, radius as f64).to_path(0.1),
        Shape::Circle => {
            let radius = bounds.width().min(bounds.height()) * 0.5;
            Circle::new(bounds.center(), radius).to_path(0.1)
        }
        Shape::Ellipse => Ellipse::new(
            bounds.center(),
            (bounds.width() * 0.5, bounds.height() * 0.5),
            0.0,
        )
        .to_path(0.1),
        Shape::Line { from, to } => {
            Line::new((from.x as f64, from.y as f64), (to.x as f64, to.y as f64)).to_path(0.1)
        }
    };
    if !matches!(shape.shape, Shape::Line { .. })
        && let Some(fill) = shape.fill
    {
        if let Some(color) = solid(fill, unsupported) {
            scene.fill(Fill::NonZero, world, color, None, &path);
        }
    }
    if let Some(stroke) = shape.stroke
        && stroke.width > 0.0
        && let Some(color) = solid(stroke.color, unsupported)
    {
        // Dashes and dots (`StrokeLineStyle`) are not lowered yet.
        scene.stroke(&Stroke::new(stroke.width as f64), world, color, None, &path);
    }
}

fn encode_vector(scene: &mut Scene, world: KurboAffine, vector: &VectorPrimitive, unsupported: &mut Unsupported) {
    let world = world * kurbo_affine(vector.transform);
    for command in vector.scene.commands() {
        match command {
            VectorCommand::FillPath { path, transform, fill } => {
                let rule = match fill.rule {
                    FillRule::NonZero => Fill::NonZero,
                    FillRule::EvenOdd => Fill::EvenOdd,
                };
                scene.fill(rule, world * kurbo_affine(*transform), color(fill.color, 1.0), None, &kurbo_path(path));
            }
            VectorCommand::StrokePath { path, transform, stroke } if stroke.width > 0.0 => {
                let mut style = Stroke::new(stroke.width as f64)
                    .with_caps(match stroke.cap {
                        LineCap::Butt => Cap::Butt,
                        LineCap::Square => Cap::Square,
                        LineCap::Round => Cap::Round,
                    })
                    .with_join(match stroke.join {
                        LineJoin::Miter => Join::Miter,
                        LineJoin::Bevel => Join::Bevel,
                        LineJoin::Round => Join::Round,
                    });
                if let Some(dash) = stroke.effective_dash() {
                    style = style.with_dashes(dash.offset as f64, dash.intervals().iter().map(|v| *v as f64));
                }
                scene.stroke(&style, world * kurbo_affine(*transform), color(stroke.color, 1.0), None, &kurbo_path(path));
            }
            VectorCommand::StrokePath { .. } => {}
            VectorCommand::Shape { .. } | VectorCommand::TextBox { .. } => unsupported.text = true,
            VectorCommand::Texture { .. } => unsupported.texture = true,
        }
    }
}

fn solid(style: ComputedColorStyle, unsupported: &mut Unsupported) -> Option<AlphaColor<Srgb>> {
    match style.solid_color() {
        Some(value) => Some(color(value, 1.0)),
        None => {
            unsupported.gradient = true;
            None
        }
    }
}

fn color(value: Color, opacity: f32) -> AlphaColor<Srgb> {
    AlphaColor::new([value.r, value.g, value.b, value.a * opacity.clamp(0.0, 1.0)])
}

fn kurbo_bounds(bounds: Bounds) -> KurboRect {
    KurboRect::new(
        bounds.min.x as f64,
        bounds.min.y as f64,
        bounds.max.x as f64,
        bounds.max.y as f64,
    )
}

fn kurbo_affine(value: Affine) -> KurboAffine {
    KurboAffine::new([
        value.xx as f64,
        value.yx as f64,
        value.xy as f64,
        value.yy as f64,
        value.dx as f64,
        value.dy as f64,
    ])
}

fn kurbo_path(path: &PathData) -> BezPath {
    let mut out = BezPath::new();
    for segment in path.segments() {
        match *segment {
            PathSegment::MoveTo(p) => out.move_to((p.x as f64, p.y as f64)),
            PathSegment::LineTo(p) => out.line_to((p.x as f64, p.y as f64)),
            PathSegment::QuadraticTo { control, to } => {
                out.quad_to((control.x as f64, control.y as f64), (to.x as f64, to.y as f64))
            }
            PathSegment::CubicTo { control1, control2, to } => out.curve_to(
                (control1.x as f64, control1.y as f64),
                (control2.x as f64, control2.y as f64),
                (to.x as f64, to.y as f64),
            ),
            PathSegment::Close => out.close_path(),
        }
    }
    out
}
