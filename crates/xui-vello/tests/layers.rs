//! Layer trees built directly as render scenes: what the encoder isolates,
//! what it draws inline, and how the compositor puts them together. Vello only
//! draws; every composite here is this crate's.

mod common;

use std::sync::Arc;

use xui_core::render::{
    BlendMode, CachePolicy, ClipShape, CompositeStyle, FrameBuilder, FrameProperties, LayerDescriptor,
    Primitive, RenderBackend, RenderNodeId, RenderScene, SceneCompiler, Shape, ShapePrimitive,
};
use xui_core::text::TextHost;
use xui_cosmic::CosmicEngine;
use xui_core::render::VectorPrimitive;
use xui_interface::{
    Affine, Bounds, Color, ComputedBackdropFilter, ComputedBackdropMask, ComputedBackdropStyle,
    ComputedColorStyle, ComputedEffect, ExternalTextureId, FilterQuality, PathBuilder, Point,
    Sampling, Size, VectorCommand, VectorScene,
};
use xui_vello::{VelloBackend, VelloBackendOptions};

const SIZE: f32 = 100.0;
const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
const RED: Color = Color::rgb(1.0, 0.0, 0.0);
const BLUE: Color = Color::rgb(0.0, 0.0, 1.0);

struct Frame {
    pixels: Vec<u8>,
    unsupported: xui_vello::Unsupported,
}

impl Frame {
    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * SIZE as usize + x) * 4;
        self.pixels[i..i + 4].try_into().unwrap()
    }
}

/// Builds a scene under the root with `build`, and draws it on black.
fn draw(build: impl FnOnce(&mut RenderScene, RenderNodeId)) -> Option<Frame> {
    draw_with(|_| {}, build)
}

/// [`draw`], with a chance to use the backend first (to register a texture).
fn draw_with(
    setup: impl FnOnce(&mut VelloBackend<CosmicEngine>),
    build: impl FnOnce(&mut RenderScene, RenderNodeId),
) -> Option<Frame> {
    let mut backend = match VelloBackend::<CosmicEngine>::headless(
        1.0,
        VelloBackendOptions {
            clear_color: Color::rgb(0.0, 0.0, 0.0),
            ..VelloBackendOptions::default()
        },
    ) {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("skipping: no wgpu adapter ({error})");
            return None;
        }
    };
    setup(&mut backend);
    let mut source = RenderScene::new();
    let root = source.root();
    build(&mut source, root);
    let mut compiler = SceneCompiler::new();
    let scene = compiler
        .compile(&source, &source.dirty_snapshot())
        .expect("compile");
    let frame = FrameBuilder::new()
        .build(scene, rect(0.0, 0.0, SIZE, SIZE), &FrameProperties::default())
        .expect("build");
    let mut text = TextHost::new(CosmicEngine::new(1.0));
    backend.begin_frame(Size::new(SIZE, SIZE)).unwrap();
    backend.submit(&frame, &mut text).unwrap();
    backend.end_frame().unwrap();
    Some(Frame {
        pixels: backend.read_pixels_rgba8().unwrap(),
        unsupported: backend.unsupported(),
    })
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds {
    Bounds::from_origin_size(Point::new(x, y), Size::new(width, height))
}

fn fill(source: &mut RenderScene, parent: RenderNodeId, bounds: Bounds, color: Color) {
    let shape = source.insert_primitive(Primitive::Shape(ShapePrimitive {
        bounds,
        shape: Shape::Rect,
        fill: Some(ComputedColorStyle::Solid(color)),
        stroke: None,
        shadow: None,
    }));
    source.append_child(parent, shape).unwrap();
}

/// A layer under `parent` whose content is a group, returned.
fn layer(source: &mut RenderScene, parent: RenderNodeId, descriptor: LayerDescriptor) -> RenderNodeId {
    let layer = source.insert_layer(descriptor);
    let content = source.insert_group();
    source.set_child(layer, Some(content)).unwrap();
    source.append_child(parent, layer).unwrap();
    content
}

/// A clip under `parent` whose content is a group, returned.
fn clip(source: &mut RenderScene, parent: RenderNodeId, shape: ClipShape) -> RenderNodeId {
    let clip = source.insert_clip(shape);
    let content = source.insert_group();
    source.set_child(clip, Some(content)).unwrap();
    source.append_child(parent, clip).unwrap();
    content
}

fn translucent(opacity: f32) -> LayerDescriptor {
    LayerDescriptor {
        composite: CompositeStyle {
            opacity,
            ..CompositeStyle::default()
        },
        ..LayerDescriptor::default()
    }
}

fn glass(bounds: Bounds) -> LayerDescriptor {
    LayerDescriptor {
        bounds: Some(bounds),
        backdrop_style: Some(ComputedBackdropStyle {
            filters: Arc::from([ComputedBackdropFilter::Blur {
                sigma_x: 8.0,
                sigma_y: 8.0,
                quality: FilterQuality::Medium,
            }]),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: ComputedBackdropMask::None,
        }),
        ..LayerDescriptor::default()
    }
}

fn close(actual: u8, expected: u8) -> bool {
    actual.abs_diff(expected) <= 2
}

/// Half of white over black: half of the encoded value, as every blend is.
const HALF: u8 = 128;

#[test]
fn an_opacity_group_is_composed_as_one() {
    let Some(frame) = draw(|source, root| {
        let group = layer(source, root, translucent(0.5));
        fill(source, group, rect(0.0, 0.0, 60.0, SIZE), WHITE);
        fill(source, group, rect(40.0, 0.0, 60.0, SIZE), WHITE);
    }) else {
        return;
    };
    // Where the two squares overlap is no brighter than where they do not:
    // the group is made opaque first, then faded once.
    for x in [20, 50, 80] {
        let pixel = frame.at(x, 50);
        assert!(close(pixel[0], HALF) && pixel[3] == 255, "x = {x}: {pixel:?}");
    }
    assert_eq!(frame.unsupported, Default::default());
}

#[test]
fn an_opaque_layer_is_drawn_inline_and_unchanged() {
    let Some(frame) = draw(|source, root| {
        let group = layer(source, root, translucent(1.0));
        fill(source, group, rect(10.0, 10.0, 30.0, 30.0), RED);
    }) else {
        return;
    };
    assert_eq!(frame.at(20, 20), [255, 0, 0, 255]);
    assert_eq!(frame.at(50, 50), [0, 0, 0, 255]);
}

/// Red left half, blue right half, both under `parent`.
fn halves(source: &mut RenderScene, parent: RenderNodeId) {
    fill(source, parent, rect(0.0, 0.0, 50.0, SIZE), RED);
    fill(source, parent, rect(50.0, 0.0, 50.0, SIZE), BLUE);
}

/// The seam between red and blue, sampled across x 25..75 at `y`: hard
/// (fully red then fully blue) or soft (mixed somewhere).
fn seam_is_soft(frame: &Frame, y: usize) -> bool {
    (25..75).any(|x| {
        let p = frame.at(x, y);
        p[0] > 30 && p[2] > 30
    })
}

#[test]
fn a_backdrop_inside_a_clip_is_blurred_and_cut_by_it() {
    let Some(frame) = draw(|source, root| {
        halves(source, root);
        // The clip covers the top half only; the glass, inside it, all of the
        // middle.
        let clipped = clip(source, root, ClipShape::Rect(rect(0.0, 0.0, SIZE, 50.0)));
        layer(source, clipped, glass(rect(25.0, 0.0, 50.0, SIZE)));
    }) else {
        return;
    };
    assert!(seam_is_soft(&frame, 25), "the glass does not blur inside the clip");
    assert!(!seam_is_soft(&frame, 75), "the glass blurs outside its clip");
    assert_eq!(frame.unsupported, Default::default());
}

/// What the group drew before the glass is what it blurs (the white beneath
/// the group shows only through the group's own 0.1%).
#[test]
fn a_backdrop_inside_a_layer_reads_what_the_layer_drew_before_it() {
    let Some(frame) = draw(|source, root| {
        // White outside the group; red and blue inside it, with the glass.
        fill(source, root, rect(0.0, 0.0, SIZE, SIZE), WHITE);
        let group = layer(source, root, translucent(0.999));
        halves(source, group);
        layer(source, group, glass(rect(25.0, 0.0, 50.0, SIZE)));
    }) else {
        return;
    };
    assert!(seam_is_soft(&frame, 50), "the nested glass does not blur");
    // Outside the glass, the halves are as drawn, with 0.1% of the white
    // behind the group showing through: 3 once encoded, at the steep low end
    // of the sRGB curve.
    let (red, blue) = (frame.at(10, 50), frame.at(90, 50));
    assert!(red[0] == 255 && red[2] < 8, "{red:?}");
    assert!(blue[2] == 255 && blue[0] < 8, "{blue:?}");
    assert_eq!(frame.unsupported, Default::default());
}

fn moved(dx: f32, dy: f32) -> LayerDescriptor {
    LayerDescriptor {
        composite: CompositeStyle {
            transform: Affine::translate(dx, dy),
            ..CompositeStyle::default()
        },
        ..LayerDescriptor::default()
    }
}

fn max_difference(a: &Frame, b: &Frame) -> u8 {
    a.pixels
        .iter()
        .zip(&b.pixels)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap()
}

/// The glass at the root, over the halves: what every passthrough case below
/// must come out as.
fn glass_at_root() -> Option<Frame> {
    draw(|source, root| {
        halves(source, root);
        layer(source, root, glass(rect(25.0, 0.0, 50.0, SIZE)));
    })
}

/// A layer lets what is beneath it through to a backdrop inside it (the
/// default): the glass blurs the halves under the group, placed where the
/// group places it, as if it were at the root.
#[test]
fn a_backdrop_inside_a_passthrough_layer_sees_beneath_it() {
    let Some(reference) = glass_at_root() else { return };
    let Some(frame) = draw(|source, root| {
        halves(source, root);
        // Isolated (moved), letting through; the glass is at 15..65 in it,
        // 25..75 in the root.
        let group = layer(source, root, moved(10.0, 0.0));
        layer(source, group, glass(rect(15.0, 0.0, 50.0, SIZE)));
    }) else {
        return;
    };
    assert!(seam_is_soft(&frame, 50), "the glass does not see beneath its layer");
    let worst = max_difference(&frame, &reference);
    assert!(worst <= 1, "off by {worst} from the glass at the root");
}

/// A clip cutting through the layer: the glass near the cut still sees what
/// is beneath it past the cut, as the glass at the root, in the same clip,
/// does. The layer is drawn whole, not only where it shows.
#[test]
fn a_backdrop_in_a_clipped_passthrough_layer_sees_past_the_clip() {
    let top = ClipShape::Rect(rect(0.0, 0.0, SIZE, 60.0));
    let Some(reference) = draw(|source, root| {
        halves(source, root);
        let clipped = clip(source, root, top.clone());
        layer(source, clipped, glass(rect(25.0, 0.0, 50.0, SIZE)));
    }) else {
        return;
    };
    let Some(frame) = draw(|source, root| {
        halves(source, root);
        let clipped = clip(source, root, top.clone());
        let group = layer(source, clipped, moved(10.0, 0.0));
        layer(source, group, glass(rect(15.0, 0.0, 50.0, SIZE)));
    }) else {
        return;
    };
    let worst = max_difference(&frame, &reference);
    assert!(worst <= 1, "off by {worst} from the glass at the root");
}

/// Through two passthrough layers, each moved: still as at the root.
#[test]
fn a_backdrop_sees_through_every_passthrough_layer_around_it() {
    let Some(reference) = glass_at_root() else { return };
    let Some(frame) = draw(|source, root| {
        halves(source, root);
        let outer = layer(source, root, moved(10.0, 0.0));
        let inner = layer(source, outer, moved(-4.0, 3.0));
        layer(source, inner, glass(rect(19.0, -3.0, 50.0, SIZE)));
    }) else {
        return;
    };
    let worst = max_difference(&frame, &reference);
    assert!(worst <= 1, "off by {worst} from the glass at the root");
}

/// A glass inside a glass sees beneath it with the outer glass applied: as
/// two glasses one after the other at the root.
///
/// What a backdrop inside a layer sees beneath it reaches as far as the layer
/// does, as in the Skia backend: so the outer glass covers what the inner
/// one reads (its bounds grown by three sigmas).
#[test]
fn a_backdrop_inside_a_glass_sees_the_glass_applied() {
    let outer_bounds = rect(0.0, 0.0, SIZE, SIZE);
    let inner_bounds = rect(40.0, 30.0, 20.0, 40.0);
    let Some(reference) = draw(|source, root| {
        halves(source, root);
        layer(source, root, glass(outer_bounds));
        layer(source, root, glass(inner_bounds));
    }) else {
        return;
    };
    let Some(frame) = draw(|source, root| {
        halves(source, root);
        let outer = layer(source, root, glass(outer_bounds));
        layer(source, outer, glass(inner_bounds));
    }) else {
        return;
    };
    let worst = max_difference(&frame, &reference);
    assert!(worst <= 1, "off by {worst} from two glasses at the root");
}

/// A layer that isolates its backdrop, and does nothing else: a backdrop
/// inside it sees only what is drawn in it, never what is beneath it.
#[test]
fn a_backdrop_inside_a_layer_that_isolates_sees_nothing_beneath_it() {
    let Some(frame) = draw(|source, root| {
        halves(source, root);
        let isolated = layer(
            source,
            root,
            LayerDescriptor {
                backdrop_isolation: xui_core::render::BackdropIsolation::Isolate,
                ..LayerDescriptor::default()
            },
        );
        layer(source, isolated, glass(rect(25.0, 0.0, 50.0, SIZE)));
    }) else {
        return;
    };
    assert!(!seam_is_soft(&frame, 50), "the glass read beneath the layer");
    assert_eq!(frame.at(49, 50), [255, 0, 0, 255]);
    assert_eq!(frame.at(50, 50), [0, 0, 255, 255]);
}

#[test]
fn a_filtered_layer_is_cut_by_a_rounded_clip_around_it() {
    let Some(frame) = draw(|source, root| {
        let clipped = clip(
            source,
            root,
            ClipShape::RoundedRect {
                rect: rect(0.0, 0.0, SIZE, SIZE),
                radius: 40.0,
            },
        );
        let blurred = layer(
            source,
            clipped,
            LayerDescriptor {
                effects: Arc::from([ComputedEffect::Blur {
                    sigma_x: 1.0,
                    sigma_y: 1.0,
                    quality: FilterQuality::Medium,
                }]),
                ..LayerDescriptor::default()
            },
        );
        fill(source, blurred, rect(-20.0, -20.0, 140.0, 140.0), WHITE);
    }) else {
        return;
    };
    assert_eq!(frame.at(50, 50), [255, 255, 255, 255]);
    assert_eq!(frame.at(50, 1)[0], 255, "the edge midpoint is inside the clip");
    assert_eq!(frame.at(1, 1), [0, 0, 0, 255], "the corner is outside the clip");
    assert_eq!(frame.unsupported, Default::default());
}

fn blurred(source: &mut RenderScene, parent: RenderNodeId) -> RenderNodeId {
    layer(
        source,
        parent,
        LayerDescriptor {
            effects: Arc::from([ComputedEffect::Blur {
                sigma_x: 1.0,
                sigma_y: 1.0,
                quality: FilterQuality::Medium,
            }]),
            ..LayerDescriptor::default()
        },
    )
}

/// A clip edge on a pixel boundary covers the pixel inside it fully: the
/// compositor's antialiasing is one pixel wide, as Vello's is.
#[test]
fn a_clip_on_a_pixel_boundary_leaves_its_edge_pixels_whole() {
    let Some(frame) = draw(|source, root| {
        let clipped = clip(
            source,
            root,
            ClipShape::RoundedRect {
                rect: rect(10.0, 10.0, 80.0, 80.0),
                radius: 4.0,
            },
        );
        let content = blurred(source, clipped);
        fill(source, content, rect(-20.0, -20.0, 140.0, 140.0), WHITE);
    }) else {
        return;
    };
    for (x, y) in [(10, 50), (89, 50), (50, 10), (50, 89)] {
        assert_eq!(frame.at(x, y), [255, 255, 255, 255], "({x}, {y}) is inside");
    }
    for (x, y) in [(9, 50), (90, 50), (50, 9), (50, 90)] {
        assert_eq!(frame.at(x, y), [0, 0, 0, 255], "({x}, {y}) is outside");
    }
}

/// The triangle (0, 0), (100, 0), (0, 100): its long edge crosses pixels at
/// every angle of coverage.
fn triangle() -> ClipShape {
    let mut builder = PathBuilder::new();
    builder
        .move_to(Point::new(0.0, 0.0))
        .line_to(Point::new(100.0, 0.0))
        .line_to(Point::new(0.0, 100.0))
        .close();
    let path = builder.build();
    ClipShape::Path {
        bounds: path.bounds(),
        path,
    }
}

/// A path clip around an isolated layer is Vello's coverage of the path, pixel
/// for pixel: the same as Vello clipping a fill drawn in place, since the
/// composite blends on the same encoded values Vello does.
#[test]
fn a_path_clip_on_a_layer_is_the_shape_vello_fills() {
    let Some(inline) = draw(|source, root| {
        let clipped = clip(source, root, triangle());
        fill(source, clipped, rect(-20.0, -20.0, 140.0, 140.0), WHITE);
    }) else {
        return;
    };
    let Some(layered) = draw(|source, root| {
        let clipped = clip(source, root, triangle());
        let content = blurred(source, clipped);
        fill(source, content, rect(-20.0, -20.0, 140.0, 140.0), WHITE);
    }) else {
        return;
    };
    let mut edge = 0;
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let coverage = inline.at(x, y)[0];
            edge += (coverage != 0 && coverage != 255) as usize;
            let expected = coverage;
            let got = layered.at(x, y)[0];
            assert!(
                got.abs_diff(expected) <= 1,
                "({x}, {y}): {got}, want {expected} for coverage {coverage}"
            );
        }
    }
    assert!(edge > 50, "the diagonal is antialiased: {edge} partial pixels");
    assert_eq!(layered.unsupported, Default::default());
}

/// Six rounded clips, each cutting a little more off the left: more than the
/// shader evaluates, so the innermost go through the mask. All of them cut.
#[test]
fn every_clip_cuts_however_many_there_are() {
    let Some(frame) = draw(|source, root| {
        let mut parent = root;
        for step in 0..6 {
            let left = 5.0 + step as f32 * 5.0;
            parent = clip(
                source,
                parent,
                ClipShape::RoundedRect {
                    rect: rect(left, 0.0, SIZE - left, SIZE),
                    radius: 2.0,
                },
            );
        }
        let content = blurred(source, parent);
        fill(source, content, rect(-20.0, -20.0, 140.0, 140.0), WHITE);
    }) else {
        return;
    };
    // The innermost clip starts at 30.
    assert_eq!(frame.at(29, 50), [0, 0, 0, 255], "the sixth clip does not cut");
    assert_eq!(frame.at(30, 50), [255, 255, 255, 255]);
    assert_eq!(frame.at(60, 50), [255, 255, 255, 255]);
}

const TEXTURE: ExternalTextureId = ExternalTextureId(11);

#[test]
fn a_texture_is_cut_by_a_path_clip() {
    let Some(frame) = draw_with(
        |backend| {
            let green = wgpu::Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
            let texture = common::texture_of(backend, 4, green);
            backend.set_wgpu_texture(TEXTURE, &texture);
        },
        |source, root| {
            let clipped = clip(source, root, triangle());
            let canvas = source.insert_primitive(Primitive::Vector(VectorPrimitive {
                scene: VectorScene::new(vec![VectorCommand::Texture {
                    id: TEXTURE,
                    source: None,
                    revision: 1,
                    bounds: rect(0.0, 0.0, SIZE, SIZE),
                    opacity: 1.0,
                    sampling: Sampling::Linear,
                }]),
                transform: Affine::IDENTITY,
            }));
            source.append_child(clipped, canvas).unwrap();
        },
    ) else {
        return;
    };
    assert_eq!(frame.at(10, 10), [0, 255, 0, 255], "inside the triangle");
    assert_eq!(frame.at(80, 80), [0, 0, 0, 255], "inside its bounds, outside it");
    assert_eq!(frame.at(60, 20), [0, 255, 0, 255]);
    assert_eq!(frame.at(20, 85), [0, 0, 0, 255]);
}

// --- Cached layers, across frames ------------------------------------------

/// A scene drawn frame after frame by one backend, as an app would.
struct Session {
    backend: VelloBackend<CosmicEngine>,
    source: RenderScene,
    compiler: SceneCompiler,
    text: TextHost<CosmicEngine>,
}

impl Session {
    fn new() -> Option<Self> {
        let backend = VelloBackend::<CosmicEngine>::headless(
            1.0,
            VelloBackendOptions {
                clear_color: Color::rgb(0.0, 0.0, 0.0),
                ..VelloBackendOptions::default()
            },
        )
        .ok()?;
        Some(Self {
            backend,
            source: RenderScene::new(),
            compiler: SceneCompiler::new(),
            text: TextHost::new(CosmicEngine::new(1.0)),
        })
    }

    fn frame(&mut self) -> Vec<u8> {
        let snapshot = self.source.dirty_snapshot();
        let scene = self.compiler.compile(&self.source, &snapshot).expect("compile");
        let frame = FrameBuilder::new()
            .build(scene, rect(0.0, 0.0, SIZE, SIZE), &FrameProperties::default())
            .expect("build");
        self.source.acknowledge(&snapshot);
        self.backend.begin_frame(Size::new(SIZE, SIZE)).unwrap();
        self.backend.submit(&frame, &mut self.text).unwrap();
        self.backend.end_frame().unwrap();
        self.backend.read_pixels_rgba8().unwrap()
    }

    /// The same scene, drawn by a backend that has seen nothing before.
    fn fresh(&self) -> Vec<u8> {
        let mut other = Session::new().expect("a second backend");
        let mut compiler = SceneCompiler::new();
        let scene = compiler
            .compile(&self.source, &self.source.dirty_snapshot())
            .expect("compile");
        let frame = FrameBuilder::new()
            .build(scene, rect(0.0, 0.0, SIZE, SIZE), &FrameProperties::default())
            .expect("build");
        other.backend.begin_frame(Size::new(SIZE, SIZE)).unwrap();
        other.backend.submit(&frame, &mut other.text).unwrap();
        other.backend.end_frame().unwrap();
        other.backend.read_pixels_rgba8().unwrap()
    }

    fn stats(&self) -> xui_vello::ResourceStats {
        self.backend.resource_stats()
    }

    /// The frame drawn now is what a fresh backend draws.
    fn assert_fresh(&mut self, what: &str) {
        let drawn = self.frame();
        let fresh = self.fresh();
        let worst = drawn
            .iter()
            .zip(&fresh)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(worst <= 1, "{what}: off by {worst} from a fresh render");
    }
}

fn square(bounds: Bounds, color: Color) -> Primitive {
    Primitive::Shape(ShapePrimitive {
        bounds,
        shape: Shape::Rect,
        fill: Some(ComputedColorStyle::Solid(color)),
        stroke: None,
        shadow: None,
    })
}

/// A cached layer (as a scroll page is) holding a red and a blue square, and
/// a white bar drawn over its middle after it. Returns the layer, the red
/// square and the bar.
fn cached_page(session: &mut Session) -> (RenderNodeId, RenderNodeId, RenderNodeId) {
    let source = &mut session.source;
    let root = source.root();
    let layer = source.insert_layer(LayerDescriptor {
        cache_policy: CachePolicy::Always,
        ..LayerDescriptor::default()
    });
    let content = source.insert_group();
    source.set_child(layer, Some(content)).unwrap();
    source.append_child(root, layer).unwrap();
    let red = source.insert_primitive(square(rect(10.0, 10.0, 30.0, 30.0), RED));
    source.append_child(content, red).unwrap();
    let blue = source.insert_primitive(square(rect(60.0, 60.0, 30.0, 30.0), BLUE));
    source.append_child(content, blue).unwrap();
    let bar = source.insert_primitive(square(rect(0.0, 45.0, SIZE, 10.0), WHITE));
    source.append_child(root, bar).unwrap();
    (layer, red, bar)
}

#[test]
fn a_cached_layer_is_drawn_once_while_it_does_not_change() {
    let Some(mut session) = Session::new() else { return };
    let (_, _, bar) = cached_page(&mut session);
    session.assert_fresh("first frame");
    assert_eq!(session.stats().layer_cache_misses, 1);
    assert_eq!(session.stats().layer_cache_entries, 1);
    // The bar over the layer changes: the layer is composited again, from
    // its texture, and not drawn.
    for shade in [0.5, 0.25] {
        session
            .source
            .update_primitive(bar, square(rect(0.0, 45.0, SIZE, 10.0), Color::rgb(shade, shade, shade)))
            .unwrap();
        session.assert_fresh("the bar changed");
    }
    let stats = session.stats();
    assert_eq!((stats.layer_cache_misses, stats.layer_cache_patches), (1, 0));
    assert_eq!(stats.layer_cache_hits, 2);
}

#[test]
fn a_change_inside_a_cached_layer_redraws_only_that() {
    let Some(mut session) = Session::new() else { return };
    let (_, red, _) = cached_page(&mut session);
    session.frame();
    session
        .source
        .update_primitive(red, square(rect(10.0, 10.0, 30.0, 30.0), Color::rgb(0.0, 1.0, 0.0)))
        .unwrap();
    session.assert_fresh("the red square turned green");
    let stats = session.stats();
    assert_eq!((stats.layer_cache_misses, stats.layer_cache_patches), (1, 1));
}

#[test]
fn moving_a_cached_layer_composites_it_again_without_drawing_it() {
    let Some(mut session) = Session::new() else { return };
    let (layer, _, _) = cached_page(&mut session);
    session.frame();
    for dy in [-7.0, -19.0] {
        session
            .source
            .update_layer_composite(
                layer,
                CompositeStyle {
                    transform: Affine::translate(0.0, dy),
                    ..CompositeStyle::default()
                },
            )
            .unwrap();
        session.assert_fresh("the layer scrolled");
    }
    let stats = session.stats();
    assert_eq!((stats.layer_cache_misses, stats.layer_cache_patches), (1, 0));
}

/// A cached layer that changes while it is not drawn cannot be patched next
/// time: the damage then says only what changed since. It is drawn afresh.
#[test]
fn a_cached_layer_changed_off_screen_is_drawn_again() {
    let Some(mut session) = Session::new() else { return };
    let (layer, red, _) = cached_page(&mut session);
    session.frame();
    let at = |dx| CompositeStyle {
        transform: Affine::translate(dx, 0.0),
        ..CompositeStyle::default()
    };
    session.source.update_layer_composite(layer, at(500.0)).unwrap();
    session
        .source
        .update_primitive(red, square(rect(10.0, 10.0, 30.0, 30.0), Color::rgb(0.0, 1.0, 0.0)))
        .unwrap();
    session.frame();
    session.source.update_layer_composite(layer, at(0.0)).unwrap();
    session.assert_fresh("back on screen");
    assert_eq!(session.stats().layer_cache_misses, 2);
}

#[test]
fn a_removed_cached_layer_gives_its_texture_back() {
    let Some(mut session) = Session::new() else { return };
    let (layer, _, _) = cached_page(&mut session);
    session.frame();
    assert_eq!(session.stats().layer_cache_entries, 1);
    assert!(session.stats().layer_cache_bytes > 0);
    session.source.detach(layer).unwrap();
    session.assert_fresh("the layer is gone");
    assert_eq!(session.stats().layer_cache_entries, 0);
    assert_eq!(session.stats().layer_cache_bytes, 0);
}

// --- Cached layers with effects ----------------------------------------------

fn effects(sigma: f32) -> Arc<[ComputedEffect]> {
    effects_with_shadow(sigma, Color::rgba(1.0, 1.0, 0.0, 1.0))
}

fn effects_with_shadow(sigma: f32, shadow: Color) -> Arc<[ComputedEffect]> {
    Arc::from([
        ComputedEffect::Blur {
            sigma_x: sigma,
            sigma_y: sigma,
            quality: FilterQuality::Medium,
        },
        // Offset, so the effects reach further one way than the other.
        ComputedEffect::DropShadow {
            color: shadow,
            offset: Point::new(9.0, 6.0),
            sigma_x: 2.0,
            sigma_y: 2.0,
            spread: 1.0,
            quality: FilterQuality::Medium,
        },
    ])
}

fn filtered_page_descriptor(sigma: f32) -> LayerDescriptor {
    LayerDescriptor {
        cache_policy: CachePolicy::Always,
        effects: effects(sigma),
        ..LayerDescriptor::default()
    }
}

/// [`cached_page`], with a blur and an offset shadow on the layer.
fn filtered_page(session: &mut Session) -> (RenderNodeId, RenderNodeId, RenderNodeId) {
    let (layer, red, bar) = cached_page(session);
    session
        .source
        .update_layer_descriptor(layer, filtered_page_descriptor(2.0))
        .unwrap();
    (layer, red, bar)
}

fn filtered_counts(session: &Session) -> (u64, u64, u64) {
    let stats = session.stats();
    (
        stats.layer_cache_filtered_misses,
        stats.layer_cache_filtered_patches,
        stats.layer_cache_filtered_hits,
    )
}

#[test]
fn a_cached_layers_effects_are_applied_once_while_it_does_not_change() {
    let Some(mut session) = Session::new() else { return };
    let (_, _, bar) = filtered_page(&mut session);
    session.assert_fresh("first frame");
    assert_eq!(filtered_counts(&session), (1, 0, 0));
    for shade in [0.5, 0.25] {
        session
            .source
            .update_primitive(bar, square(rect(0.0, 45.0, SIZE, 10.0), Color::rgb(shade, shade, shade)))
            .unwrap();
        session.assert_fresh("the bar over the layer changed");
    }
    assert_eq!(filtered_counts(&session), (1, 0, 2));
}

#[test]
fn a_change_inside_a_cached_layer_reapplies_its_effects_only_around_it() {
    let Some(mut session) = Session::new() else { return };
    let (_, red, _) = filtered_page(&mut session);
    session.frame();
    // The square shrinks from its far corner and changes colour; its near
    // corner, an edge of the layer's content bounds, stays where it was.
    for (side, color) in [(24.0, Color::rgb(0.0, 1.0, 0.0)), (18.0, Color::rgb(1.0, 0.0, 1.0))] {
        session
            .source
            .update_primitive(red, square(rect(10.0, 10.0, side, side), color))
            .unwrap();
        session.assert_fresh("a square inside shrank and changed colour");
    }
    let stats = session.stats();
    assert_eq!((stats.layer_cache_misses, stats.layer_cache_patches), (1, 2));
    assert_eq!(filtered_counts(&session), (1, 2, 2));
}

#[test]
fn scrolling_a_cached_layer_with_effects_applies_none_of_them() {
    let Some(mut session) = Session::new() else { return };
    let (layer, _, _) = filtered_page(&mut session);
    session.frame();
    for dy in [-7.0, -19.0] {
        session
            .source
            .update_layer_composite(
                layer,
                CompositeStyle {
                    transform: Affine::translate(0.0, dy),
                    ..CompositeStyle::default()
                },
            )
            .unwrap();
        session.assert_fresh("the layer scrolled");
    }
    assert_eq!(filtered_counts(&session), (1, 0, 2));
    assert_eq!(session.stats().layer_cache_patches, 0);
}

/// New effects on unchanged content: the content texture is kept as it is
/// (a layer's effects have a version of their own, apart from its content),
/// and only the effects are applied to it afresh.
#[test]
fn changing_a_cached_layers_effects_keeps_its_content() {
    let Some(mut session) = Session::new() else { return };
    let (layer, _, _) = filtered_page(&mut session);
    session.frame();
    session
        .source
        .update_layer_descriptor(layer, filtered_page_descriptor(5.0))
        .unwrap();
    session.assert_fresh("the blur grew");
    let stats = session.stats();
    assert_eq!((stats.layer_cache_misses, stats.layer_cache_patches), (1, 0));
    assert_eq!(filtered_counts(&session).0, 2);
    // Only the shadow's colour: the effects reach as far as before, so their
    // result covers the same pixels, and only the program says it is stale.
    session
        .source
        .update_layer_descriptor(
            layer,
            LayerDescriptor {
                effects: effects_with_shadow(5.0, Color::rgba(0.0, 1.0, 1.0, 1.0)),
                ..filtered_page_descriptor(5.0)
            },
        )
        .unwrap();
    session.assert_fresh("the shadow changed colour");
    assert_eq!(filtered_counts(&session).0, 3);
    let stats = session.stats();
    assert_eq!((stats.layer_cache_misses, stats.layer_cache_patches), (1, 0));
}

// --- Many draws in the same clips ---------------------------------------------

/// A scroll view's worth of rows inside three clips, on a large target: each
/// clip is pushed once for all the rows. Pushed once a row, the clips' tiles
/// overflow Vello's buffers and the whole frame comes out empty.
#[test]
fn many_draws_in_the_same_clips_are_all_drawn() {
    const WIDTH: f32 = 1600.0;
    const HEIGHT: f32 = 1000.0;
    let Ok(mut backend) = VelloBackend::<CosmicEngine>::headless(
        1.0,
        VelloBackendOptions {
            clear_color: Color::rgb(0.0, 0.0, 0.0),
            ..VelloBackendOptions::default()
        },
    ) else {
        return;
    };
    let mut source = RenderScene::new();
    let mut parent = source.root();
    for depth in 0..3 {
        let inset = 10.0 + depth as f32 * 10.0;
        parent = clip(
            &mut source,
            parent,
            ClipShape::RoundedRect {
                rect: rect(inset, inset, WIDTH - 2.0 * inset, HEIGHT - 2.0 * inset),
                radius: 8.0,
            },
        );
    }
    for index in 0..1600 {
        let (column, row) = ((index % 40) as f32, (index / 40) as f32);
        fill(&mut source, parent, rect(column * 40.0, row * 25.0, 36.0, 21.0), RED);
    }
    let mut compiler = SceneCompiler::new();
    let scene = compiler.compile(&source, &source.dirty_snapshot()).unwrap();
    let frame = FrameBuilder::new()
        .build(scene, rect(0.0, 0.0, WIDTH, HEIGHT), &FrameProperties::default())
        .unwrap();
    let mut text = TextHost::new(CosmicEngine::new(1.0));
    backend.begin_frame(Size::new(WIDTH, HEIGHT)).unwrap();
    backend.submit(&frame, &mut text).unwrap();
    backend.end_frame().unwrap();
    let pixels = backend.read_pixels_rgba8().unwrap();
    let at = |x: usize, y: usize| -> [u8; 4] {
        pixels[(y * WIDTH as usize + x) * 4..][..4].try_into().unwrap()
    };
    // A row well inside the clips, the gap between two rows, and a row cut
    // off by the outermost clip.
    assert_eq!(at(50, 50), [255, 0, 0, 255], "a row inside the clips");
    assert_eq!(at(78, 50), [0, 0, 0, 255], "the gap between two rows");
    assert_eq!(at(5, 5), [0, 0, 0, 255], "outside the clips");
}

/// A cached layer with a backdrop inside that sees beneath it is not kept: what
/// it shows changes with what is beneath it, which its damage does not track.
#[test]
fn a_cached_layer_that_lets_a_backdrop_see_beneath_it_is_drawn_every_frame() {
    let Some(mut session) = Session::new() else { return };
    let root = session.source.root();
    let red = session.source.insert_primitive(square(rect(0.0, 0.0, 50.0, SIZE), RED));
    session.source.append_child(root, red).unwrap();
    let blue = session.source.insert_primitive(square(rect(50.0, 0.0, 50.0, SIZE), BLUE));
    session.source.append_child(root, blue).unwrap();
    let page = layer(
        &mut session.source,
        root,
        LayerDescriptor {
            cache_policy: CachePolicy::Always,
            ..LayerDescriptor::default()
        },
    );
    layer(&mut session.source, page, glass(rect(25.0, 0.0, 50.0, SIZE)));
    session.assert_fresh("first frame");
    session
        .source
        .update_primitive(red, square(rect(0.0, 0.0, 50.0, SIZE), Color::rgb(0.0, 1.0, 0.0)))
        .unwrap();
    session.assert_fresh("what is beneath the page changed");
    assert_eq!(session.stats().layer_cache_entries, 0);
}
