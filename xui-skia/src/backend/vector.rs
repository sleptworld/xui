//! Shapes, paths, vector scenes and mask geometry.

use skia_safe::{
    Canvas, ClipOp, Matrix, Paint, PaintStyle, Path, PathBuilder, RRect, Rect as SkRect, Shader,
    paint::{Cap as SkCap, Join as SkJoin},
};
use std::sync::Arc;
use xui::render::{ClipShape, Shape};
use xui_interface::{
    Affine, Bounds, LineCap, LineJoin, PathData, PathFill, PathSegment, PathStroke, TextBackend,
    VectorCommand, VectorScene,
};
use xui_render_graph::MaskShape;

use super::{
    SkiaBackend,
    convert::{sk_bounds, sk_matrix},
    lru::LocalLru,
    paint::{GradientKey, alpha_color, solid_paint, style_paint},
};

#[derive(Clone)]
pub(super) enum CompiledVectorCommand {
    FillPath {
        path: Path,
        transform: Affine,
        fill: PathFill,
    },
    StrokePath {
        path: Path,
        transform: Affine,
        stroke: PathStroke,
    },
}

impl<T: TextBackend> SkiaBackend<T> {
    pub(super) fn compiled_vector_scene(
        &mut self,
        scene: &VectorScene,
    ) -> Arc<[CompiledVectorCommand]> {
        if let Some(compiled) = self.vector_scenes.get(&scene.id()) {
            return compiled;
        }
        let compiled: Arc<[CompiledVectorCommand]> = scene
            .commands()
            .iter()
            .filter_map(|command| match command {
                VectorCommand::FillPath {
                    path,
                    transform,
                    fill,
                } => Some(CompiledVectorCommand::FillPath {
                    path: self.compiled_vector_path(path),
                    transform: *transform,
                    fill: *fill,
                }),
                VectorCommand::StrokePath {
                    path,
                    transform,
                    stroke,
                } => Some(CompiledVectorCommand::StrokePath {
                    path: self.compiled_vector_path(path),
                    transform: *transform,
                    stroke: *stroke,
                }),
                // See the vello backend: a vector scene only ever holds paths.
                VectorCommand::Shape { .. } | VectorCommand::TextBox { .. } => None,
            })
            .collect::<Vec<_>>()
            .into();
        self.vector_scenes.insert(scene.id(), Arc::clone(&compiled));
        compiled
    }

    fn compiled_vector_path(&mut self, path: &PathData) -> Path {
        if let Some(compiled) = self.vector_paths.get(&path.id()) {
            return compiled;
        }
        let compiled = sk_path(path);
        self.vector_paths.insert(path.id(), compiled.clone());
        compiled
    }
}

/// Clips `canvas` to one chain node, already transformed by `matrix`.
///
/// A rect or rounded-rect clip is pushed as such whenever `matrix` keeps it
/// axis-aligned, which lets Skia scissor (or take its rrect fast path) instead
/// of rasterizing a clip mask from a path. Rotated and skewed clips, and path
/// clips, still go through the path form.
pub(super) fn apply_clip_shape(canvas: &Canvas, clip: &ClipShape, matrix: &Matrix) {
    if matrix.rect_stays_rect() {
        match clip {
            ClipShape::Rect(rect) => {
                let (mapped, _) = matrix.map_rect(sk_bounds(*rect));
                canvas.clip_rect(mapped, ClipOp::Intersect, true);
                return;
            }
            ClipShape::RoundedRect { rect, radius } => {
                let rrect = RRect::new_rect_xy(sk_bounds(*rect), *radius, *radius);
                if let Some(mapped) = rrect.transform(matrix) {
                    canvas.clip_rrect(mapped, ClipOp::Intersect, true);
                    return;
                }
            }
            ClipShape::Path { .. } => {}
        }
    }
    let mut builder = PathBuilder::new();
    match clip {
        ClipShape::Rect(rect) => {
            builder.add_rect(sk_bounds(*rect), None, None);
        }
        ClipShape::RoundedRect { rect, radius } => {
            builder.add_rrect(
                RRect::new_rect_xy(sk_bounds(*rect), *radius, *radius),
                None,
                None,
            );
        }
        ClipShape::Path { path, .. } => {
            append_path(&mut builder, path);
        }
    }
    builder.transform(matrix);
    canvas.clip_path(&builder.detach(), ClipOp::Intersect, true);
}

pub(super) fn draw_mask_shape(canvas: &Canvas, shape: MaskShape, paint: &Paint) {
    let unit = SkRect::from_xywh(0.0, 0.0, 1.0, 1.0);
    match shape {
        MaskShape::Rect => {
            canvas.draw_rect(unit, paint);
        }
        MaskShape::RoundedRect(radius) => {
            canvas.draw_round_rect(unit, radius.clamp(0.0, 0.5), radius.clamp(0.0, 0.5), paint);
        }
        MaskShape::Circle => {
            canvas.draw_circle((0.5, 0.5), 0.5, paint);
        }
        MaskShape::Ellipse => {
            canvas.draw_oval(unit, paint);
        }
        MaskShape::Line { from, to } => {
            let mut line = paint.clone();
            line.set_style(PaintStyle::Stroke);
            line.set_stroke_width(1.0);
            canvas.draw_line((from.x, from.y), (to.x, to.y), &line);
        }
    }
}

fn append_path(builder: &mut PathBuilder, path: &PathData) {
    for segment in path.segments() {
        match *segment {
            PathSegment::MoveTo(p) => {
                builder.move_to((p.x, p.y));
            }
            PathSegment::LineTo(p) => {
                builder.line_to((p.x, p.y));
            }
            PathSegment::QuadraticTo { control, to } => {
                builder.quad_to((control.x, control.y), (to.x, to.y));
            }
            PathSegment::CubicTo {
                control1,
                control2,
                to,
            } => {
                builder.cubic_to(
                    (control1.x, control1.y),
                    (control2.x, control2.y),
                    (to.x, to.y),
                );
            }
            PathSegment::Close => {
                builder.close();
            }
        }
    }
}

fn sk_path(path: &PathData) -> Path {
    let mut builder = PathBuilder::new();
    append_path(&mut builder, path);
    builder.detach()
}

pub(super) fn draw_shape(
    canvas: &Canvas,
    gradients: &mut LocalLru<GradientKey, Shader>,
    primitive: &xui::render::ShapePrimitive,
    transform: Affine,
    opacity: f32,
) {
    let save = canvas.save();
    canvas.concat(&sk_matrix(transform));
    if let Some(shadow) = primitive.shadow.filter(|s| s.color.a > 0.0) {
        let mut paint = solid_paint(alpha_color(shadow.color, opacity));
        paint.set_mask_filter(skia_safe::MaskFilter::blur(
            skia_safe::BlurStyle::Normal,
            shadow.blur.max(0.0),
            false,
        ));
        draw_shape_geometry(
            canvas,
            primitive.shape,
            primitive.bounds.expand(shadow.spread),
            shadow.offset,
            &paint,
        );
    }
    if let Some(fill) = primitive.fill {
        let paint = style_paint(gradients, fill, primitive.bounds, opacity);
        draw_shape_geometry(
            canvas,
            primitive.shape,
            primitive.bounds,
            xui_interface::Point::new(0.0, 0.0),
            &paint,
        );
    }
    if let Some(stroke) = primitive.stroke.filter(|s| s.width > 0.0) {
        let mut paint = style_paint(gradients, stroke.color, primitive.bounds, opacity);
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(stroke.width);
        draw_shape_geometry(
            canvas,
            primitive.shape,
            primitive.bounds,
            xui_interface::Point::new(0.0, 0.0),
            &paint,
        );
    }
    canvas.restore_to_count(save);
}

fn draw_shape_geometry(
    canvas: &Canvas,
    shape: Shape,
    rect: Bounds,
    offset: xui_interface::Point,
    paint: &Paint,
) {
    let rect = rect.translate(offset);
    match shape {
        Shape::Rect => {
            canvas.draw_rect(sk_bounds(rect), paint);
        }
        Shape::RoundedRect(radius) => {
            canvas.draw_round_rect(sk_bounds(rect), radius, radius, paint);
        }
        Shape::Circle => {
            canvas.draw_circle(
                (
                    rect.x() + rect.width() * 0.5,
                    rect.y() + rect.height() * 0.5,
                ),
                rect.width().min(rect.height()) * 0.5,
                paint,
            );
        }
        Shape::Ellipse => {
            canvas.draw_oval(sk_bounds(rect), paint);
        }
        Shape::Line { from, to } => {
            canvas.draw_line(
                (from.x + offset.x, from.y + offset.y),
                (to.x + offset.x, to.y + offset.y),
                paint,
            );
        }
    }
}

pub(super) fn draw_vector(
    canvas: &Canvas,
    commands: &[CompiledVectorCommand],
    primitive_transform: Affine,
    transform: Affine,
    opacity: f32,
) {
    let outer = primitive_transform.then(transform);
    for command in commands {
        match command {
            CompiledVectorCommand::FillPath {
                path,
                transform,
                fill,
            } => {
                let save = canvas.save();
                canvas.concat(&sk_matrix(transform.then(outer)));
                let mut path = path.clone();
                path.set_fill_type(match fill.rule {
                    xui_interface::FillRule::NonZero => skia_safe::PathFillType::Winding,
                    xui_interface::FillRule::EvenOdd => skia_safe::PathFillType::EvenOdd,
                });
                canvas.draw_path(&path, &solid_paint(alpha_color(fill.color, opacity)));
                canvas.restore_to_count(save);
            }
            CompiledVectorCommand::StrokePath {
                path,
                transform,
                stroke,
            } if stroke.width > 0.0 => {
                let save = canvas.save();
                canvas.concat(&sk_matrix(transform.then(outer)));
                let mut paint = solid_paint(alpha_color(stroke.color, opacity));
                paint.set_style(PaintStyle::Stroke);
                paint.set_stroke_width(stroke.width);
                paint.set_stroke_cap(match stroke.cap {
                    LineCap::Butt => SkCap::Butt,
                    LineCap::Square => SkCap::Square,
                    LineCap::Round => SkCap::Round,
                });
                paint.set_stroke_join(match stroke.join {
                    LineJoin::Miter => SkJoin::Miter,
                    LineJoin::Bevel => SkJoin::Bevel,
                    LineJoin::Round => SkJoin::Round,
                });
                if let Some(dash) = stroke.effective_dash() {
                    // Skia needs an even interval count; an odd pattern repeats
                    // to close the cycle, which is what SVG does too.
                    let mut intervals = dash.intervals().to_vec();
                    if intervals.len() % 2 == 1 {
                        intervals.extend_from_within(..);
                    }
                    if let Some(effect) = skia_safe::PathEffect::dash(&intervals, dash.offset) {
                        paint.set_path_effect(effect);
                    }
                }
                canvas.draw_path(path, &paint);
                canvas.restore_to_count(save);
            }
            CompiledVectorCommand::StrokePath { .. } => {}
        }
    }
}
