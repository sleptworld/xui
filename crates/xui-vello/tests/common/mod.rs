//! Scenes and rendering helpers shared by the Vello pixel tests and the
//! Skia comparison.
#![allow(dead_code, unused_imports)]

use xui_core::{
    app::AppRoot,
    prelude::*,
    text::TextHost,
    widgets::{CANVAS_GPU_FORMAT, CANVAS_GPU_USAGES, CanvasController, CanvasGpuContext},
};
use xui_cosmic::CosmicEngine;
use xui_interface::{
    ColorStyle, Effect, FilterQuality, FlexDirectionStyle, ImageData, ImageFit, Point, PositionStyle,
    Sampling, ShadowStyle,
};
use xui_macros::component;
use xui_vello::{Unsupported, VelloBackend, VelloBackendOptions};

pub const SIZE: f32 = 100.0;
pub const CLEAR: Color = Color::rgb(0.0, 0.0, 0.0);
pub const RED: Color = Color::rgb(1.0, 0.0, 0.0);
pub const BLUE: Color = Color::rgb(0.0, 0.0, 1.0);

pub struct Frame {
    pub unsupported: Unsupported,
    pub pixels: Vec<u8>,
    pub width: usize,
}

impl Frame {
    pub fn at(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        self.pixels[i..i + 4].try_into().unwrap()
    }
}

/// Renders `root` at scale 1 into a `SIZE` x `SIZE` frame.
pub fn render(root: AppRoot) -> Option<Frame> {
    render_with(root, |_| {})
}

/// [`render`], with a chance to use the backend (to register textures, say)
/// before the frame is drawn.
pub fn render_with(
    root: AppRoot,
    setup: impl FnOnce(&mut VelloBackend<CosmicEngine>),
) -> Option<Frame> {
    render_scaled(root, 1.0, setup)
}

/// [`render_with`] on a display with `scale` physical pixels per logical one.
pub fn render_scaled(
    root: AppRoot,
    scale: f32,
    setup: impl FnOnce(&mut VelloBackend<CosmicEngine>),
) -> Option<Frame> {
    render_options(
        root,
        scale,
        VelloBackendOptions {
            clear_color: CLEAR,
            ..VelloBackendOptions::default()
        },
        setup,
    )
    .map(|(frame, _)| frame)
}

/// [`render_scaled`] with `options`, returning the backend's resource stats
/// after the frame as well.
pub fn render_options(
    root: AppRoot,
    scale: f32,
    options: VelloBackendOptions,
    setup: impl FnOnce(&mut VelloBackend<CosmicEngine>),
) -> Option<(Frame, xui_vello::ResourceStats)> {
    let mut backend = match VelloBackend::<CosmicEngine>::headless(scale, options) {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("skipping: no wgpu adapter ({error})");
            return None;
        }
    };
    setup(&mut backend);
    let mut app = App::new(root);
    // What a runner does at startup: GPU painters draw on the backend's device.
    let context = backend.wgpu_context();
    app.set_gpu_context(CanvasGpuContext::new(
        context.device().clone(),
        context.queue().clone(),
    ));
    app.set_scale_factor(scale);
    app.resize(Size::new(SIZE, SIZE));
    let mut text = TextHost::new(CosmicEngine::new(scale));
    app.render(&mut backend, &mut text).expect("render");
    let (width, height) = backend.frame_size_px();
    assert_eq!((width, height), ((SIZE * scale) as u32, (SIZE * scale) as u32));
    let pixels = backend.read_pixels_rgba8().expect("read pixels");
    assert_eq!(pixels.len(), width as usize * height as usize * 4);
    let frame = Frame {
        unsupported: backend.unsupported(),
        pixels,
        width: width as usize,
    };
    Some((frame, backend.resource_stats()))
}

pub fn is_close(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (*a as i32 - e as i32).abs() <= 2)
}

/// A box with no children: `container` with only its attributes.
pub fn solid_root(app: &mut App) {
    app.mount::<Solid>(SolidProps {});
}

#[component]
fn solid() {
    view! { container(width: SIZE, height: SIZE, background: RED) }
}

pub fn rounded_clip_root(app: &mut App) {
    app.mount::<RoundedClip>(RoundedClipProps {});
}

#[component]
fn rounded_clip() {
    // A blue child that overflows its parent: only the rounded parent's
    // interior may show it.
    view! {
        container(width: SIZE, height: SIZE, background: RED, border_radius: 40.0, clip: true) {
            container(width: SIZE, height: SIZE, background: BLUE)
        }
    }
}

/// A 40x40 red box 30px in from the corner of the white root surface, with
/// `effects` on it. Each effect makes it an isolated, filtered layer.
#[component]
fn effect_box(effects: &Vec<Effect>) {
    view! {
        container(width: SIZE, height: SIZE) {
            container(
                width: 40.0,
                height: 40.0,
                margin: EdgeInsets::all(30.0),
                background: RED,
                effects: effects.clone(),
            )
        }
    }
}

pub fn effect_root(app: &mut App, effects: Vec<Effect>) {
    app.mount::<EffectBox>(EffectBoxProps { effects });
}

pub fn blur_root(app: &mut App) {
    effect_root(app, vec![Effect::blur(4.0)]);
}

pub fn swap_root(app: &mut App) {
    // Red and blue trade places; green and alpha are left alone.
    #[rustfmt::skip]
    let matrix = [
        0.0, 0.0, 1.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0, 0.0,
        1.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0, 0.0,
    ];
    effect_root(app, vec![Effect::ColorMatrix(matrix)]);
}

pub fn shadow_effect_root(app: &mut App) {
    effect_root(
        app,
        vec![Effect::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 1.0).into(),
            offset_x: 0.0.into(),
            offset_y: 25.0.into(),
            sigma_x: 3.0.into(),
            sigma_y: 3.0.into(),
            spread: 0.0.into(),
            quality: FilterQuality::default(),
        }],
    );
}

pub fn text_root(app: &mut App) {
    app.mount::<HelloText>(HelloTextProps {});
}

#[component]
fn hello_text() {
    view! { text("Hello xui", width: SIZE, height: SIZE, color: RED, font_size: 32.0) }
}

pub fn count(frame: &Frame, matches: impl Fn([u8; 4]) -> bool) -> usize {
    frame
        .pixels
        .chunks_exact(4)
        .filter(|p| matches([p[0], p[1], p[2], p[3]]))
        .count()
}

/// 2x2: red, green / blue, white, fully opaque.
pub fn quad() -> ImageData {
    ImageData::rgba8(
        Size::new(2, 2),
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 255, 255,
        ],
    )
}

pub fn image_root(app: &mut App) {
    app.mount::<QuadImage>(QuadImageProps {});
}

#[component]
fn quad_image() {
    // Nearest keeps the four quadrants hard-edged.
    view! {
        image(
            source: (ImageKey::from("quad"), quad()),
            fit: ImageFit::Fill,
            sampling: Sampling::Nearest,
            width: SIZE,
            height: SIZE,
        )
    }
}

pub fn contained_image_root(app: &mut App) {
    app.mount::<ContainedImage>(ContainedImageProps {});
}

#[component]
fn contained_image() {
    // A square image in a wide box: `Contain` leaves bands on both sides.
    view! {
        image(
            source: (ImageKey::from("quad-contain"), quad()),
            fit: ImageFit::Contain,
            sampling: Sampling::Nearest,
            width: SIZE,
            height: 50.0,
            background: BLUE,
        )
    }
}

pub fn shadow_root(app: &mut App) {
    app.mount::<ShadowBox>(ShadowBoxProps {});
}

#[component]
fn shadow_box() {
    // The root surface is white. A 40x40 box at the top-left of the frame casts
    // a black shadow 20px straight down, blurred with sigma 4.
    view! {
        container(width: SIZE, height: SIZE) {
            container(
                width: 40.0,
                height: 40.0,
                margin: EdgeInsets::all(20.0),
                background: RED,
                shadow: ShadowStyle::new()
                    .color(Color::rgba(0.0, 0.0, 0.0, 1.0))
                    .offset(Point::new(0.0, 20.0))
                    .blur(4.0),
            )
        }
    }
}

pub fn luma(p: [u8; 4]) -> u32 {
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

pub fn linear_gradient_root(app: &mut App) {
    app.mount::<LinearGradient>(LinearGradientProps {});
}

#[component]
fn linear_gradient() {
    // Left edge red, right edge blue, in fractions of the box.
    view! {
        container(
            width: SIZE,
            height: SIZE,
            background: ColorStyle::linear_gradient(Point::new(0.0, 0.5), Point::new(1.0, 0.5), RED, BLUE),
        )
    }
}

pub fn radial_gradient_root(app: &mut App) {
    app.mount::<RadialGradient>(RadialGradientProps {});
}

#[component]
fn radial_gradient() {
    view! {
        container(
            width: SIZE,
            height: SIZE,
            background: ColorStyle::radial_gradient(Point::new(0.5, 0.5), 0.5, RED, BLUE),
        )
    }
}

pub fn clear(device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::TextureView, color: wgpu::Color) {
    let mut encoder = device.create_command_encoder(&Default::default());
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    }));
    queue.submit([encoder.finish()]);
}

/// A full-size canvas drawn by `controller`.
#[component]
pub fn painted(controller: &CanvasController) {
    view! { canvas(controller: controller.clone(), width: SIZE, height: SIZE) }
}

/// Mounts a full-size canvas drawn by `controller`.
pub fn mount_canvas(app: &mut App, controller: CanvasController) {
    app.mount::<Painted>(PaintedProps { controller });
}

pub fn gpu_canvas_root(app: &mut App) {
    // The painter fills its target with blue; the framework composites it.
    let controller = CanvasController::with_gpu_painter::<()>(|_, gpu| {
        clear(
            gpu.device(),
            gpu.queue(),
            gpu.target(),
            wgpu::Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 },
        );
    });
    mount_canvas(app, controller);
}

pub const REGISTERED: xui_interface::ExternalTextureId = xui_interface::ExternalTextureId(7);

pub fn registered_texture_root(app: &mut App) {
    let controller = CanvasController::with_painter(|painter| {
        // Left half of the canvas only.
        painter.texture(
            REGISTERED,
            1,
            xui_interface::Bounds::from_origin_size((0.0, 0.0), (50.0, 100.0)),
        );
    });
    mount_canvas(app, controller);
}

/// Red left half, blue right half, and a glass panel over the middle with a
/// backdrop blur: the seam between the colours is hard outside the panel and
/// soft inside it.
pub fn glass_root(app: &mut App) {
    app.mount::<Glass>(GlassProps {});
}

#[component]
fn glass() {
    view! {
        container(width: SIZE, height: SIZE, flex_direction: FlexDirectionStyle::Row) {
            container(width: 50.0, height: SIZE, background: RED)
            container(width: 50.0, height: SIZE, background: BLUE)
            container(
                width: 50.0,
                height: SIZE,
                position: PositionStyle::Absolute,
                inset: EdgeInsets::new(25.0, 0.0, 0.0, 0.0),
                backdrop_blur: 8.0,
            )
        }
    }
}

pub fn glass_with_content_root(app: &mut App) {
    app.mount::<GlassWithContent>(GlassWithContentProps {});
}

#[component]
fn glass_with_content() {
    view! {
        container(width: SIZE, height: SIZE, background: RED) {
            container(
                width: 50.0,
                height: SIZE,
                position: PositionStyle::Absolute,
                inset: EdgeInsets::new(25.0, 0.0, 0.0, 0.0),
                backdrop_blur: 8.0,
            ) {
                container(width: 10.0, height: 10.0, background: Color::rgb(0.0, 1.0, 0.0))
            }
        }
    }
}


/// A canvas-format texture on the backend's device, cleared to `color`.
pub fn texture_of(
    backend: &VelloBackend<CosmicEngine>,
    size: u32,
    color: wgpu::Color,
) -> wgpu::Texture {
    let context = backend.wgpu_context();
    let texture = context.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("test texture"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: CANVAS_GPU_FORMAT,
        usage: CANVAS_GPU_USAGES,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    clear(context.device(), context.queue(), &view, color);
    texture
}
