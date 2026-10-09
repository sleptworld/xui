//! Scenes and rendering helpers shared by the Vello pixel tests and the
//! Skia comparison.
#![allow(dead_code, unused_imports)]

use xui_core::{
    app::ComponentFn,
    prelude::*,
    text::TextHost,
    widgets::{
        CANVAS_GPU_FORMAT, CANVAS_GPU_USAGES, CanvasController, CanvasGpuContext, CanvasWidget,
        ContainerWidget, ImageWidget, TextWidget,
    },
};
use xui_cosmic::CosmicEngine;
use xui_interface::{
    ColorStyle, Effect, FilterQuality, FlexDirectionStyle, ImageData, ImageFit, Point, Sampling, ShadowStyle,
};
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
pub fn render(root: ComponentFn) -> Option<Frame> {
    render_with(root, |_| {})
}

/// [`render`], with a chance to use the backend (to register textures, say)
/// before the frame is drawn.
pub fn render_with(
    root: ComponentFn,
    setup: impl FnOnce(&mut VelloBackend<CosmicEngine>),
) -> Option<Frame> {
    render_scaled(root, 1.0, setup)
}

/// [`render_with`] on a display with `scale` physical pixels per logical one.
pub fn render_scaled(
    root: ComponentFn,
    scale: f32,
    setup: impl FnOnce(&mut VelloBackend<CosmicEngine>),
) -> Option<Frame> {
    let mut backend = match VelloBackend::<CosmicEngine>::headless(
        scale,
        VelloBackendOptions {
            clear_color: CLEAR,
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
    Some(Frame {
        unsupported: backend.unsupported(),
        pixels,
        width: width as usize,
    })
}

pub fn is_close(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (*a as i32 - e as i32).abs() <= 2)
}

pub fn block(style: Style, children: Vec<ElementDesc>) -> ElementDesc {
    ContainerWidget::new().style(style).into_element_desc(children)
}

pub fn solid_root(_: &mut HookContext<'_>) -> ElementDesc {
    block(
        Style::new().width(SIZE).height(SIZE).background(RED),
        vec![],
    )
}

pub fn rounded_clip_root(_: &mut HookContext<'_>) -> ElementDesc {
    // A blue child that overflows its parent: only the rounded parent's
    // interior may show it.
    let child = block(
        Style::new().width(SIZE).height(SIZE).background(BLUE),
        vec![],
    );
    block(
        Style::new()
            .width(SIZE)
            .height(SIZE)
            .background(RED)
            .border_radius(40.0)
            .clip(true),
        vec![child],
    )
}

/// A 40x40 red box 30px in from the corner of the white root surface, with
/// `effects` on it. Each effect makes it an isolated, filtered layer.
pub fn effect_root(effects: Vec<Effect>) -> impl Fn(&mut HookContext<'_>) -> ElementDesc {
    move |_| {
        block(
            Style::new().width(SIZE).height(SIZE),
            vec![block(
                Style::new()
                    .width(40.0)
                    .height(40.0)
                    .margin(EdgeInsets::all(30.0))
                    .background(RED)
                    .effects(effects.clone()),
                vec![],
            )],
        )
    }
}

pub fn blur_root(cx: &mut HookContext<'_>) -> ElementDesc {
    effect_root(vec![Effect::blur(4.0)])(cx)
}

pub fn swap_root(cx: &mut HookContext<'_>) -> ElementDesc {
    // Red and blue trade places; green and alpha are left alone.
    #[rustfmt::skip]
    let matrix = [
        0.0, 0.0, 1.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0, 0.0,
        1.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0, 0.0,
    ];
    effect_root(vec![Effect::ColorMatrix(matrix)])(cx)
}

pub fn shadow_effect_root(cx: &mut HookContext<'_>) -> ElementDesc {
    effect_root(vec![Effect::DropShadow {
        color: Color::rgba(0.0, 0.0, 0.0, 1.0).into(),
        offset_x: 0.0.into(),
        offset_y: 25.0.into(),
        sigma_x: 3.0.into(),
        sigma_y: 3.0.into(),
        spread: 0.0.into(),
        quality: FilterQuality::default(),
    }])(cx)
}

pub fn text_root(_: &mut HookContext<'_>) -> ElementDesc {
    TextWidget::new("Hello xui")
        .style(
            Style::new()
                .width(SIZE)
                .height(SIZE)
                .color(RED)
                .font_size(32.0),
        )
        .into_element_desc()
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

pub fn image_root(_: &mut HookContext<'_>) -> ElementDesc {
    ImageWidget::new()
        .image_data("quad", quad())
        .fit(ImageFit::Fill)
        // Nearest keeps the four quadrants hard-edged.
        .sampling(Sampling::Nearest)
        .style(Style::new().width(SIZE).height(SIZE))
        .into_element_desc(vec![])
}

pub fn contained_image_root(_: &mut HookContext<'_>) -> ElementDesc {
    // A square image in a wide box: `Contain` leaves bands on both sides.
    ImageWidget::new()
        .image_data("quad-contain", quad())
        .fit(ImageFit::Contain)
        .sampling(Sampling::Nearest)
        .style(Style::new().width(SIZE).height(50.0).background(BLUE))
        .into_element_desc(vec![])
}

pub fn shadow_root(_: &mut HookContext<'_>) -> ElementDesc {
    // The root surface is white. A 40x40 box at the top-left of the frame casts
    // a black shadow 20px straight down, blurred with sigma 4.
    block(
        Style::new().width(SIZE).height(SIZE),
        vec![block(
            Style::new()
                .width(40.0)
                .height(40.0)
                .margin(EdgeInsets::all(20.0))
                .background(RED)
                .shadow(
                    ShadowStyle::new()
                        .color(Color::rgba(0.0, 0.0, 0.0, 1.0))
                        .offset(Point::new(0.0, 20.0))
                        .blur(4.0),
                ),
            vec![],
        )],
    )
}

pub fn luma(p: [u8; 4]) -> u32 {
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

pub fn linear_gradient_root(_: &mut HookContext<'_>) -> ElementDesc {
    // Left edge red, right edge blue, in fractions of the box.
    block(
        Style::new().width(SIZE).height(SIZE).background(ColorStyle::linear_gradient(
            Point::new(0.0, 0.5),
            Point::new(1.0, 0.5),
            RED,
            BLUE,
        )),
        vec![],
    )
}

pub fn radial_gradient_root(_: &mut HookContext<'_>) -> ElementDesc {
    block(
        Style::new().width(SIZE).height(SIZE).background(ColorStyle::radial_gradient(
            Point::new(0.5, 0.5),
            0.5,
            RED,
            BLUE,
        )),
        vec![],
    )
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

pub fn gpu_canvas_root(_: &mut HookContext<'_>) -> ElementDesc {
    // The painter fills its target with blue; the framework composites it.
    let controller = CanvasController::with_gpu_painter::<()>(|_, gpu| {
        clear(
            gpu.device(),
            gpu.queue(),
            gpu.target(),
            wgpu::Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 },
        );
    });
    CanvasWidget::new(controller)
        .style(Style::new().width(SIZE).height(SIZE))
        .into_element_desc()
}

pub const REGISTERED: xui_interface::ExternalTextureId = xui_interface::ExternalTextureId(7);

pub fn registered_texture_root(_: &mut HookContext<'_>) -> ElementDesc {
    let controller = CanvasController::with_painter(|painter| {
        // Left half of the canvas only.
        painter.texture(
            REGISTERED,
            1,
            xui_interface::Bounds::from_origin_size((0.0, 0.0), (50.0, 100.0)),
        );
    });
    CanvasWidget::new(controller)
        .style(Style::new().width(SIZE).height(SIZE))
        .into_element_desc()
}

/// Red left half, blue right half, and a glass panel over the middle with a
/// backdrop blur: the seam between the colours is hard outside the panel and
/// soft inside it.
pub fn glass_root(_: &mut HookContext<'_>) -> ElementDesc {
    let half = |color| block(Style::new().width(50.0).height(SIZE).background(color), vec![]);
    let pane = block(
        Style::new()
            .width(50.0)
            .height(SIZE)
            .absolute()
            .inset(EdgeInsets::new(25.0, 0.0, 0.0, 0.0))
            .backdrop_blur(8.0),
        vec![],
    );
    ContainerWidget::new()
        .style(Style::new().width(SIZE).height(SIZE))
        .flex_direction(FlexDirectionStyle::Row)
        .into_element_desc(vec![half(RED), half(BLUE), pane])
}

pub fn glass_with_content_root(_: &mut HookContext<'_>) -> ElementDesc {
    let green = block(
        Style::new().width(10.0).height(10.0).background(Color::rgb(0.0, 1.0, 0.0)),
        vec![],
    );
    let pane = ContainerWidget::new()
        .style(
            Style::new()
                .width(50.0)
                .height(SIZE)
                .absolute()
                .inset(EdgeInsets::new(25.0, 0.0, 0.0, 0.0))
                .backdrop_blur(8.0),
        )
        .into_element_desc(vec![green]);
    block(
        Style::new().width(SIZE).height(SIZE).background(RED),
        vec![pane],
    )
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
