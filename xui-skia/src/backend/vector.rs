//! Shapes, paths, vector scenes and mask geometry.

use rustc_hash::FxHashMap;
use skia_safe::{
    Canvas, ClipOp, Image, Matrix, Paint, PaintStyle, Path, PathBuilder, RRect, Rect as SkRect,
    SamplingOptions, Shader,
    paint::{Cap as SkCap, Join as SkJoin},
};
use std::sync::Arc;
use xui::render::{ClipShape, Shape};
use xui_interface::{
    Affine, Bounds, ExternalTextureId, LineCap, LineJoin, PathData, PathFill, PathSegment,
    PathStroke, Sampling, TextBackend, VectorCommand, VectorScene,
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
    /// Resolved against [`ExternalTextures`] at draw time, not here: the id is
    /// stable across frames and the compiled scene is cached, so binding the
    /// image now would pin the contents this scene was first compiled with.
    Texture {
        id: ExternalTextureId,
        bounds: Bounds,
        opacity: f32,
        sampling: Sampling,
    },
}

/// GPU textures the application registered, drawn by id from a canvas.
///
/// The `Image` borrows a platform handle it does not own; `_owner` is whatever
/// keeps that handle alive -- a `wgpu::Texture` today -- and is never read.
pub(crate) struct ExternalTexture {
    pub(super) image: Image,
    pub(super) _owner: Box<dyn std::any::Any>,
}

pub(super) type ExternalTextures = FxHashMap<ExternalTextureId, ExternalTexture>;

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
                VectorCommand::Texture {
                    id,
                    bounds,
                    opacity,
                    sampling,
                    // Only there to make a content change visible to scene
                    // diffing; by the time a scene is compiled the diff has
                    // already happened and the image is looked up live.
                    revision: _,
                } => Some(CompiledVectorCommand::Texture {
                    id: *id,
                    bounds: *bounds,
                    opacity: *opacity,
                    sampling: *sampling,
                }),
                // A vector scene otherwise only ever holds paths; shapes and
                // text are lowered into batches of their own.
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
    textures: &ExternalTextures,
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
            CompiledVectorCommand::Texture {
                id,
                bounds,
                opacity: texture_opacity,
                sampling,
            } => {
                // An unregistered id draws nothing. A texture that is not ready
                // yet is a normal startup state, and losing the rest of the
                // canvas over it would be the worse failure.
                let Some(texture) = textures.get(id) else {
                    continue;
                };
                let save = canvas.save();
                canvas.concat(&sk_matrix(outer));
                let mut paint = Paint::default();
                paint.set_alpha_f(opacity * texture_opacity);
                canvas.draw_image_rect_with_sampling_options(
                    &texture.image,
                    None,
                    sk_bounds(*bounds),
                    sk_sampling(*sampling),
                    &paint,
                );
                canvas.restore_to_count(save);
            }
        }
    }
}

fn sk_sampling(sampling: Sampling) -> SamplingOptions {
    match sampling {
        Sampling::Nearest => {
            SamplingOptions::new(skia_safe::FilterMode::Nearest, skia_safe::MipmapMode::None)
        }
        Sampling::Linear => {
            SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::None)
        }
        // Mitchell, the same cubic Skia uses for high-quality image scaling.
        Sampling::Cubic => SamplingOptions::from(skia_safe::CubicResampler::mitchell()),
    }
}

#[cfg(test)]
mod texture_tests {
    use skia_safe::{AlphaType, ColorSpace, ColorType, ImageInfo, surfaces};
    use xui_cosmic::CosmicEngine;
    use xui_interface::VectorScene;

    use super::*;
    use crate::{SkiaBackendOptions, backend::SkiaBackend};

    type TestBackend = SkiaBackend<CosmicEngine>;

    const SIZE: i32 = 8;

    /// A solid-red raster image standing in for an imported GPU texture.
    ///
    /// The registry does not care where an `Image` came from -- the wgpu import
    /// is covered by `crate::wgpu_surface`'s own test -- so this exercises the
    /// half that runs on every backend: compile, look up, draw.
    fn red_image() -> Image {
        let info = ImageInfo::new(
            (SIZE, SIZE),
            ColorType::RGBA8888,
            AlphaType::Premul,
            ColorSpace::new_srgb(),
        );
        let mut surface = surfaces::raster(&info, None, None).unwrap();
        surface.canvas().clear(skia_safe::Color::RED);
        surface.image_snapshot()
    }

    fn texture_scene(id: ExternalTextureId, revision: u64) -> VectorScene {
        VectorScene::new(vec![VectorCommand::Texture {
            id,
            revision,
            bounds: Bounds::new(
                xui_interface::Point::new(0.0, 0.0),
                xui_interface::Point::new(SIZE as f32, SIZE as f32),
            ),
            opacity: 1.0,
            sampling: Sampling::Nearest,
        }])
    }

    fn draw_scene(backend: &mut TestBackend, scene: &VectorScene) -> Vec<u8> {
        let info = ImageInfo::new(
            (SIZE, SIZE),
            ColorType::RGBA8888,
            AlphaType::Premul,
            ColorSpace::new_srgb(),
        );
        let mut surface = surfaces::raster(&info, None, None).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        let commands = backend.compiled_vector_scene(scene);
        draw_vector(
            surface.canvas(),
            &commands,
            &backend.external_textures,
            Affine::IDENTITY,
            Affine::IDENTITY,
            1.0,
        );
        let row_bytes = SIZE as usize * 4;
        let mut pixels = vec![0u8; row_bytes * SIZE as usize];
        assert!(surface.read_pixels(&info, &mut pixels, row_bytes, (0, 0)));
        pixels
    }

    #[test]
    fn a_registered_texture_is_drawn_by_id() {
        let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
        let id = ExternalTextureId::next();
        backend.external_textures.insert(
            id,
            ExternalTexture {
                image: red_image(),
                _owner: Box::new(()),
            },
        );

        let pixels = draw_scene(&mut backend, &texture_scene(id, 1));
        assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
    }

    /// A canvas may name a texture that is not registered yet -- during
    /// startup, or after the application dropped it. That has to leave the rest
    /// of the drawing intact rather than fail the frame.
    #[test]
    fn an_unregistered_texture_draws_nothing() {
        let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
        let pixels = draw_scene(&mut backend, &texture_scene(ExternalTextureId::next(), 1));
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
    }

    /// The id is stable across frames so the backend keeps one registration, so
    /// nothing about a redraw of the same texture is visible to scene diffing.
    /// `revision` is what makes it visible.
    #[test]
    fn a_new_revision_reports_a_paint_change() {
        let id = ExternalTextureId::next();
        let before = texture_scene(id, 1);
        let after = texture_scene(id, 2);
        let change = before.diff(&after);
        assert!(change.paint, "new contents must repaint");
        assert!(
            !change.geometry,
            "new contents in the same rectangle must not re-emit geometry"
        );
    }
}
