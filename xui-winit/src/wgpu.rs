use crate::renders::*;
use glam::{Vec2, Vec3};
use std::{marker::PhantomData, sync::Arc};
use wgpu::util::DeviceExt;
use xui::text::TextHost;
use xui_interface::*;
use xui_text_engine::CosmicEngine;

pub type WgpuBackendError = Box<dyn std::error::Error + Send + Sync>;

pub struct WGPUBackend<T: TextBackend = CosmicEngine> {
    // Instances
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    // Glyph
    glyph_render: GlyphRender,
    atlas: Atlas,
    // Images
    image_render: ImageRender,
    // Sdfs
    sdf_render: SdfRenderer,
    // Composite
    compositor: Compositor,
    // Common Tools
    ui_uniform_buffer: wgpu::Buffer,
    ui_bind_group: wgpu::BindGroup,
    scene: SceneTexture,
    scene_needs_clear: bool,
    presented_frame: bool,
    scale_factor: f32,
    _text: PhantomData<fn() -> T>,
}

const SHAPE_RECT: f32 = 0.0;
const SHAPE_ROUNDED_RECT: f32 = 1.0;
const SHAPE_LINE: f32 = 2.0;
const COLOR_SOLID: f32 = 0.0;
const COLOR_LINEAR_GRADIENT: f32 = 1.0;
const COLOR_RADIAL_GRADIENT: f32 = 2.0;
const STROKE_CENTER: f32 = 0.0;
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct UiUniforms {
    viewport_size: [f32; 4],
    scale_factor: [f32; 4],
}

impl<T: TextBackend> WGPUBackend<T> {
    pub fn new(window: Arc<winit::window::Window>) -> Self {
        pollster::block_on(Self::new_(window))
    }

    async fn new_(window: Arc<winit::window::Window>) -> Self {
        let size = window.inner_size();
        let scale_factor = window.scale_factor();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(Arc::clone(&window))
            .expect("failed to create surface");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("failed to find adapter");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await
            .expect("failed to create device");

        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .expect("surface not supported by adapter");
        let surface_capabilities = surface.get_capabilities(&adapter);
        config.format = choose_srgb_surface_format(config.format, &surface_capabilities.formats)
            .expect("surface does not support an sRGB format");

        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        surface.configure(&device, &config);

        let ui_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("xui sdf bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let ui_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("xui sdf uniforms"),
            contents: bytemuck::bytes_of(&UiUniforms {
                viewport_size: [size.width as f32, size.height as f32, 0.0, 0.0],
                scale_factor: [scale_factor as f32; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let ui_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui bind group"),
            layout: &ui_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ui_uniform_buffer.as_entire_binding(),
            }],
        });

        let scene = SceneTexture::new(&device, &config);
        let atlas = Atlas::new(&device);
        let glyph_render = GlyphRender::new(&device, &atlas, &ui_bind_group_layout);
        let image_render = ImageRender::new(&device, &ui_bind_group_layout);
        let compositor = Compositor::new(&device, config.format, &scene.view);
        let sdf_render = SdfRenderer::new(&device, &ui_bind_group_layout);

        Self {
            instance,
            adapter,
            surface,
            device,
            queue,
            config,
            image_render,
            compositor,
            sdf_render,
            ui_uniform_buffer,
            ui_bind_group,
            glyph_render,
            atlas,
            scene,
            scene_needs_clear: true,
            presented_frame: false,
            scale_factor: scale_factor as f32,
            _text: PhantomData,
        }
    }

    fn logical_scene_size(&self) -> xui_interface::Size<f32> {
        xui_interface::Size::<f32>::new(
            self.config.width as f32 / self.scale_factor,
            self.config.height as f32 / self.scale_factor,
        )
    }
}

struct SceneTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl SceneTexture {
    fn new(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> Self {
        let width = config.width.max(1);
        let height = config.height.max(1);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xui scene cache"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
        }
    }
}

fn choose_srgb_surface_format(
    default: wgpu::TextureFormat,
    supported: &[wgpu::TextureFormat],
) -> Option<wgpu::TextureFormat> {
    let default_srgb = default.add_srgb_suffix();
    if supported.contains(&default_srgb) {
        return Some(default_srgb);
    }

    if default.is_srgb() {
        return Some(default);
    }

    supported.iter().copied().find(wgpu::TextureFormat::is_srgb)
}

impl<T: TextBackend> RenderBackend<TextHost<T>> for WGPUBackend<T> {
    type Error = WgpuBackendError;

    fn begin_frame(&mut self, size: xui_interface::Size<f32>) -> Result<(), Self::Error> {
        let width = (size.width * self.scale_factor).max(1.0).ceil() as u32;
        let height = (size.height * self.scale_factor).max(1.0).ceil() as u32;
        if self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.scene = SceneTexture::new(&self.device, &self.config);
            self.compositor.reset_view(&self.device, &self.scene.view);
            self.scene_needs_clear = true;
        }
        self.queue.write_buffer(
            &self.ui_uniform_buffer,
            0,
            bytemuck::bytes_of(&UiUniforms {
                viewport_size: [width as f32, height as f32, 0.0, 0.0],
                scale_factor: [self.scale_factor; 4],
            }),
        );
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn did_present(&self) -> bool {
        self.presented_frame
    }

    fn paint(
        &mut self,
        commands: &[PaintCommand],
        damage: &DamageRegion,
        text: &mut TextHost<T>,
    ) -> Result<(), Self::Error> {
        let _ = (&self.instance, &self.adapter);
        self.presented_frame = false;
        let logical_scene_size = self.logical_scene_size();
        let scene_clip = damage.bounds().unwrap_or(Rect::new(
            0.0,
            0.0,
            logical_scene_size.width,
            logical_scene_size.height,
        ));

        let result = self.build_ui_instances(commands, scene_clip, text)?;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.scene = SceneTexture::new(&self.device, &self.config);
                self.compositor.reset_view(&self.device, &self.scene.view);
                self.scene_needs_clear = true;
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => {
                        return Ok(());
                    }
                    wgpu::CurrentSurfaceTexture::Outdated
                    | wgpu::CurrentSurfaceTexture::Lost
                    | wgpu::CurrentSurfaceTexture::Validation => {
                        return Err(std::io::Error::other(
                            "failed to acquire current wgpu surface texture after reconfigure",
                        )
                        .into());
                    }
                }
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(std::io::Error::other("wgpu surface texture validation error").into());
            }
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xui sdf encoder"),
            });

        self.sdf_render
            .deal_instances(&self.device, &self.queue, &result.sdf_records);
        self.image_render
            .deal_records(&self.device, &self.queue, &result.image_records)?;
        self.glyph_render
            .deal_glyphs(&self.device, &self.queue, &result.glyph_records);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui scene cache render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.scene.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if self.scene_needs_clear {
                            wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.08,
                                g: 0.09,
                                b: 0.11,
                                a: 1.0,
                            })
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            let target_size = (self.config.width, self.config.height);
            self.sdf_render.render(
                &mut pass,
                &self.ui_bind_group,
                &result.sdf_scissors,
                self.scale_factor,
                target_size,
            );
            self.image_render.render(
                &mut pass,
                &self.ui_bind_group,
                &result.image_records,
                &result.image_scissors,
                self.scale_factor,
                target_size,
            );
            self.glyph_render.render(
                &mut pass,
                &self.ui_bind_group,
                self.scale_factor,
                target_size,
            );
        }

        self.scene_needs_clear = false;
        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            self.compositor
                .composite(&mut encoder, &frame_view, self.scene_needs_clear);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        self.presented_frame = true;
        Ok(())
    }

    fn set_factor(&mut self, factor: f32) -> Result<(), Self::Error> {
        self.scale_factor = factor;
        Ok(())
    }
}

impl<T: TextBackend> WGPUBackend<T> {
    fn build_ui_instances(
        &mut self,
        commands: &[PaintCommand],
        viewport_clip: Rect,
        text: &mut TextHost<T>,
    ) -> Result<PrepareResult, WgpuBackendError> {
        let mut result = PrepareResult::default();
        let mut transform_stack = vec![Point::new(0.0, 0.0)];
        let mut clip_stack = vec![viewport_clip];

        for command in commands {
            match command {
                PaintCommand::Rect {
                    rect,
                    color,
                    stroke,
                    shadow,
                } => {
                    let rect = translate_rect(*rect, current_transform(&transform_stack));
                    result.push_paint_rect_instance(
                        rect,
                        0.0,
                        *color,
                        *stroke,
                        *shadow,
                        current_clip(&clip_stack),
                    );
                }
                PaintCommand::RoundedRect {
                    rect,
                    radius,
                    color,
                    stroke,
                    shadow,
                } => {
                    let rect = translate_rect(*rect, current_transform(&transform_stack));
                    result.push_paint_rect_instance(
                        rect,
                        *radius,
                        *color,
                        *stroke,
                        *shadow,
                        current_clip(&clip_stack),
                    );
                }
                PaintCommand::Line {
                    from,
                    to,
                    color,
                    width,
                } => {
                    let offset = current_transform(&transform_stack);
                    result.push_line_instance(
                        translate_point(*from, offset),
                        translate_point(*to, offset),
                        *color,
                        *width,
                        current_clip(&clip_stack),
                    );
                }
                PaintCommand::Text(command) => {
                    let rect = translate_rect(command.rect, current_transform(&transform_stack));
                    let Some(clip) = intersect_rect(current_clip(&clip_stack), rect) else {
                        continue;
                    };
                    self.push_text_glyph_records(command, rect, clip, text, &mut result)?;
                }
                PaintCommand::Image(command) => {
                    let rect = translate_rect(command.rect, current_transform(&transform_stack));
                    result.push_image_record(command, rect, current_clip(&clip_stack));
                }
                PaintCommand::PushClip(rect) => {
                    let rect = translate_rect(*rect, current_transform(&transform_stack));
                    let clip =
                        intersect_rect(current_clip(&clip_stack), rect).unwrap_or(Rect::ZERO);
                    clip_stack.push(clip);
                }
                PaintCommand::PopClip => {
                    if clip_stack.len() > 1 {
                        clip_stack.pop();
                    }
                }
                PaintCommand::PushTransform { translate } => {
                    let current = current_transform(&transform_stack);
                    transform_stack
                        .push(Point::new(current.x + translate.x, current.y + translate.y));
                }
                PaintCommand::PopTransform => {
                    if transform_stack.len() > 1 {
                        transform_stack.pop();
                    }
                }

                PaintCommand::Clear(color) => {
                    result.push_rect_instance(
                        viewport_clip,
                        0.0,
                        *color,
                        Color::TRANSPARENT,
                        0.0,
                        viewport_clip,
                    );
                }
            }
        }

        Ok(result)
    }

    fn push_text_glyph_records(
        &mut self,
        command: &TextPaintCommand,
        rect: Rect,
        clip: Rect,
        text: &mut TextHost<T>,
        records: &mut PrepareResult,
    ) -> Result<(), WgpuBackendError> {
        if rect.width <= 0.0 || rect.height <= 0.0 || clip.width <= 0.0 || clip.height <= 0.0 {
            return Ok(());
        }

        let layout = text
            .simple_layout(command.node_id)
            .expect("text layout must be prepared before paint");

        let layout_query = text.layout_query(command.node_id);

        if let (Some(selection), Some(query)) = (command.paint.selection, layout_query) {
            if selection.color.a > 0.0 {
                for selection_rect in query.selection_rects(selection.range) {
                    let screen_rect = Rect::new(
                        rect.x + selection_rect.x,
                        rect.y + selection_rect.y,
                        selection_rect.width,
                        selection_rect.height,
                    );
                    records.push_paint_rect_instance(
                        screen_rect,
                        0.0,
                        ComputedColorStyle::Solid(selection.color),
                        None,
                        None,
                        clip,
                    );
                }
            }
        }

        let caret_rect = command
            .paint
            .caret
            .and_then(|caret| layout_query.and_then(|query| query.caret_rect(caret.char_index)));

        let backend = text.backend_mut();

        if command.paint.style.color.a > 0.0 {
            let scale = 1. / self.scale_factor;

            for glyph in &layout.glyphs {
                let Some((alloc, bitmap)) = self.glyph_allocation(backend, glyph.key.clone())?
                else {
                    continue;
                };
                if bitmap.width == 0 || bitmap.height == 0 {
                    continue;
                }

                let screen_rect = Rect::new(
                    rect.x + glyph.draw_pos.x + bitmap.left as f32 * scale,
                    rect.y + glyph.draw_pos.y - bitmap.top as f32 * scale,
                    bitmap.width as f32 * scale,
                    bitmap.height as f32 * scale,
                );
                if intersect_rect(clip, screen_rect).is_none() {
                    continue;
                }

                let record = TextGlyphRecord {
                    ptype: bitmap.format,
                    screen_rect,
                    clip,
                    color: command.paint.style.color,
                    atlas_origin: alloc.origin,
                    atlas_layer: alloc.layer,
                    atlas_size: alloc.total_size,
                    atlas_rect: Rect::new(
                        alloc.origin.x,
                        alloc.origin.y,
                        bitmap.width as f32,
                        bitmap.height as f32,
                    ),
                };
                records.glyph_records.push(record);
            }
        }

        if let Some(caret) = command.paint.caret {
            push_text_caret(
                command,
                rect,
                clip,
                Some(layout.size()),
                records,
                caret,
                caret_rect,
            );
        }
        Ok(())
    }

    fn glyph_allocation(
        &mut self,
        text: &mut T,
        key: <T as Shaper>::GlyphKey,
    ) -> Result<Option<(AllocInfo, RasterizedGlyph)>, WgpuBackendError> {
        let value = if let Some(bitmap) = text.rasterize(key) {
            if bitmap.width == 0 || bitmap.height == 0 {
                None
            } else {
                Some((self.atlas.handle_allocation(&self.queue, &bitmap)?, bitmap))
            }
        } else {
            None
        };
        Ok(value)
    }
}

fn push_text_caret(
    command: &TextPaintCommand,
    rect: Rect,
    clip: Rect,
    layout_size: Option<Size<f32>>,
    records: &mut PrepareResult,
    caret: TextCaret,
    caret_rect: Option<Rect>,
) {
    if caret.color.a <= 0.0 || caret.width <= 0.0 {
        return;
    }

    let (caret_x, top, height) = if let Some(caret_rect) = caret_rect {
        (
            rect.x + caret_rect.x,
            rect.y + caret_rect.y,
            caret_rect.height.min(rect.height).max(1.0),
        )
    } else {
        let caret_x = rect.x + layout_size.map(|size| size.width).unwrap_or(0.0);
        let height = line_height_for_caret(
            command.paint.style.line_height,
            command.paint.style.font_size,
        )
        .min(rect.height)
        .max(1.0);
        let top = rect.y + ((rect.height - height) * 0.5).max(0.0);
        (caret_x, top, height)
    };
    records.push_line_instance(
        Point::new(caret_x, top),
        Point::new(caret_x, top + height),
        caret.color,
        caret.width,
        clip,
    );
}

fn line_height_for_caret(line_height: LineHeight, font_size: f32) -> f32 {
    match line_height {
        LineHeight::Normal => font_size * 1.2,
        LineHeight::Px(px) => px,
        LineHeight::Em(em) => em * font_size,
    }
}

#[derive(Default)]
struct PrepareResult {
    pub sdf_records: Vec<SdfInstance>,
    pub sdf_scissors: Vec<Rect>,
    pub image_records: Vec<ImageDrawRecord>,
    pub image_scissors: Vec<Rect>,
    pub glyph_records: Vec<TextGlyphRecord>,
}

pub(crate) fn physical_scissor(
    rect: Rect,
    scale_factor: f32,
    target_size: (u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let x0 = (rect.x * scale_factor)
        .floor()
        .max(0.0)
        .min(target_size.0 as f32) as u32;
    let y0 = (rect.y * scale_factor)
        .floor()
        .max(0.0)
        .min(target_size.1 as f32) as u32;
    let x1 = ((rect.x + rect.width) * scale_factor)
        .ceil()
        .max(0.0)
        .min(target_size.0 as f32) as u32;
    let y1 = ((rect.y + rect.height) * scale_factor)
        .ceil()
        .max(0.0)
        .min(target_size.1 as f32) as u32;
    if x1 > x0 && y1 > y0 {
        Some((x0, y0, x1 - x0, y1 - y0))
    } else {
        None
    }
}

impl PrepareResult {
    fn push_paint_rect_instance(
        &mut self,
        rect: Rect,
        radius: f32,
        fill: ComputedColorStyle,
        stroke: Option<ComputedStrokeStyle>,
        shadow: Option<ComputedShadowStyle>,
        clip: Rect,
    ) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }

        let visible_shadow = shadow.filter(|shadow| shadow.color.a > 0.0);

        if let Some(shadow) = visible_shadow {
            self.push_shadow_instance(
                rect,
                radius,
                shadow.color,
                shadow.offset,
                shadow.blur,
                shadow.spread,
                clip,
            );
        }

        if fill.is_visible() {
            self.push_fill_style_instance(rect, radius, fill, clip);
        }

        if let Some(stroke) =
            stroke.filter(|stroke| stroke.width > 0.0 && stroke.color.is_visible())
        {
            self.push_stroke_style_instance(rect, radius, stroke.color, stroke.width, clip);
        }
    }

    fn push_fill_style_instance(
        &mut self,
        rect: Rect,
        radius: f32,
        style: ComputedColorStyle,
        clip: Rect,
    ) {
        let style = InstanceColorStyle::new(style, rect);
        self.push_projected_rect_instance(
            rect,
            radius,
            style.from,
            Color::TRANSPARENT,
            0.0,
            style.to,
            Point::new(0.0, 0.0),
            0.0,
            0.0,
            style.kind,
            style.geometry,
            false,
            clip,
        );
    }

    fn push_stroke_style_instance(
        &mut self,
        rect: Rect,
        radius: f32,
        style: ComputedColorStyle,
        stroke_width: f32,
        clip: Rect,
    ) {
        let style = InstanceColorStyle::new(style, rect);
        self.push_projected_rect_instance(
            rect,
            radius,
            Color::TRANSPARENT,
            style.from,
            stroke_width,
            style.to,
            Point::new(0.0, 0.0),
            0.0,
            0.0,
            style.kind,
            style.geometry,
            false,
            clip,
        );
    }

    fn push_shadow_instance(
        &mut self,
        shape: Rect,
        radius: f32,
        color: Color,
        offset: Point,
        blur: f32,
        spread: f32,
        clip: Rect,
    ) {
        let bounds = shadow_bounds(shape, offset, blur, spread);
        if shape.width <= 0.0
            || shape.height <= 0.0
            || bounds.width <= 0.0
            || bounds.height <= 0.0
            || clip.width <= 0.0
            || clip.height <= 0.0
            || color.a <= 0.0
        {
            return;
        }

        let kind = if radius > 0.0 {
            SHAPE_ROUNDED_RECT
        } else {
            SHAPE_RECT
        };

        self.sdf_records.push(SdfInstance {
            bounds: rect_to_array(bounds),
            shape: rect_to_array(shape),
            clip: rect_to_array(clip),
            fill_color: [0.0; 4],
            stroke_color: [0.0; 4],
            params: [kind, radius.max(0.0), COLOR_SOLID, 1.0],
            stroke_params: [0.0, STROKE_CENTER, 0.0, 0.0],
            projection_color: color_to_array(color),
            projection_params: [offset.x, offset.y, blur.max(0.0), spread],
            extra: [0.0; 4],
        });
        self.sdf_scissors.push(clip);
    }

    fn push_projected_rect_instance(
        &mut self,
        rect: Rect,
        radius: f32,
        fill_color: Color,
        stroke_color: Color,
        stroke_width: f32,
        projection_color: Color,
        projection_offset: Point,
        projection_blur: f32,
        projection_spread: f32,
        color_kind: f32,
        color_geometry: [f32; 4],
        projection_enabled: bool,
        clip: Rect,
    ) {
        if rect.width <= 0.0
            || rect.height <= 0.0
            || clip.width <= 0.0
            || clip.height <= 0.0
            || (fill_color.a <= 0.0 && stroke_color.a <= 0.0 && projection_color.a <= 0.0)
        {
            return;
        }

        let stroke_direction = STROKE_CENTER;
        let stroke_outset = stroke_outset(stroke_width.max(0.0), stroke_direction) + 1.0;
        let projection_outset = projection_blur.max(0.0) + projection_spread.max(0.0);
        let projection_bounds = inflate_rect(
            translate_rect(rect, projection_offset),
            projection_outset + 1.0,
        );
        let mut bounds = inflate_rect(rect, stroke_outset);
        if projection_enabled {
            bounds = bounds.union(projection_bounds);
        }
        let kind = if radius > 0.0 {
            SHAPE_ROUNDED_RECT
        } else {
            SHAPE_RECT
        };

        self.sdf_records.push(SdfInstance {
            bounds: rect_to_array(bounds),
            shape: rect_to_array(rect),
            clip: rect_to_array(clip),
            fill_color: color_to_array(fill_color),
            stroke_color: color_to_array(stroke_color),
            params: [
                kind,
                radius.max(0.0),
                color_kind,
                if projection_enabled { 1.0 } else { 0.0 },
            ],
            stroke_params: [stroke_width.max(0.0), stroke_direction, 0.0, 0.0],
            projection_color: color_to_array(projection_color),
            projection_params: [
                projection_offset.x,
                projection_offset.y,
                projection_blur.max(0.0),
                projection_spread,
            ],
            extra: color_geometry,
        });
        self.sdf_scissors.push(clip);
    }

    fn push_image_record(&mut self, command: &ImagePaintCommand, rect: Rect, clip: Rect) {
        if rect.width <= 0.0
            || rect.height <= 0.0
            || clip.width <= 0.0
            || clip.height <= 0.0
            || command.opacity <= 0.0
            || command.key == ImageKey::default()
        {
            return;
        }

        let scissor = clip;
        let Some(container_clip) = intersect_rect(clip, rect) else {
            return;
        };
        let Some(tile) = fitted_image_rect(rect, command.data.size, command.style) else {
            return;
        };
        let draw_rect = repeated_image_bounds(rect, tile, command.style.repeat);
        let Some(clip) = intersect_rect(container_clip, draw_rect) else {
            return;
        };
        let mut variant = command.variant.clone();
        variant.sampling = command.style.sampling;

        self.image_records.push(ImageDrawRecord {
            key: command.key.clone(),
            data: command.data.clone(),
            rect: draw_rect,
            clip,
            tile,
            repeat: command.style.repeat,
            opacity: command.opacity.clamp(0.0, 1.0),
            variant,
        });
        self.image_scissors.push(scissor);
    }

    fn push_rect_instance(
        &mut self,
        rect: Rect,
        radius: f32,
        fill_color: Color,
        stroke_color: Color,
        stroke_width: f32,
        clip: Rect,
    ) {
        self.push_projected_rect_instance(
            rect,
            radius,
            fill_color,
            stroke_color,
            stroke_width,
            Color::TRANSPARENT,
            Point::new(0.0, 0.0),
            0.0,
            0.0,
            COLOR_SOLID,
            [0.0; 4],
            false,
            clip,
        );
    }

    fn push_line_instance(&mut self, from: Point, to: Point, color: Color, width: f32, clip: Rect) {
        if color.a <= 0.0 || width <= 0.0 || clip.width <= 0.0 || clip.height <= 0.0 {
            return;
        }

        let min_x = from.x.min(to.x);
        let min_y = from.y.min(to.y);
        let max_x = from.x.max(to.x);
        let max_y = from.y.max(to.y);
        let bounds = inflate_rect(
            Rect::new(
                min_x,
                min_y,
                (max_x - min_x).max(1.0),
                (max_y - min_y).max(1.0),
            ),
            width * 0.5 + 1.0,
        );

        self.sdf_records.push(SdfInstance {
            bounds: rect_to_array(bounds),
            shape: rect_to_array(bounds),
            clip: rect_to_array(clip),
            fill_color: color_to_array(color),
            stroke_color: [0.0; 4],
            params: [SHAPE_LINE, 0.0, 0.0, 0.0],
            stroke_params: [width, STROKE_CENTER, 0.0, 0.0],
            projection_color: [0.0; 4],
            projection_params: [0.0; 4],
            extra: [from.x, from.y, to.x, to.y],
        });
        self.sdf_scissors.push(clip);
    }
}

struct InstanceColorStyle {
    kind: f32,
    from: Color,
    to: Color,
    geometry: [f32; 4],
}

impl InstanceColorStyle {
    fn new(style: ComputedColorStyle, rect: Rect) -> Self {
        match style {
            ComputedColorStyle::Solid(color) => Self {
                kind: COLOR_SOLID,
                from: color,
                to: Color::TRANSPARENT,
                geometry: [0.0; 4],
            },
            ComputedColorStyle::LinearGradient(gradient) => {
                let start = relative_point_in_rect(rect, gradient.start);
                let end = relative_point_in_rect(rect, gradient.end);
                Self {
                    kind: COLOR_LINEAR_GRADIENT,
                    from: gradient.from,
                    to: gradient.to,
                    geometry: [start.x, start.y, end.x, end.y],
                }
            }
            ComputedColorStyle::RadialGradient(gradient) => {
                let center = relative_point_in_rect(rect, gradient.center);
                Self {
                    kind: COLOR_RADIAL_GRADIENT,
                    from: gradient.from,
                    to: gradient.to,
                    geometry: [center.x, center.y, gradient.radius.max(0.0), 0.0],
                }
            }
        }
    }
}

fn current_transform(stack: &[Point]) -> Point {
    stack.last().copied().unwrap_or_default()
}

fn current_clip(stack: &[Rect]) -> Rect {
    stack.last().copied().unwrap_or_default()
}

fn translate_point(point: Point, offset: Point) -> Point {
    Point::new(point.x + offset.x, point.y + offset.y)
}

fn translate_rect(rect: Rect, offset: Point) -> Rect {
    Rect::new(
        rect.x + offset.x,
        rect.y + offset.y,
        rect.width,
        rect.height,
    )
}

fn fitted_image_rect(
    container: Rect,
    image_size: Size<u32>,
    image_style: ImageStyle,
) -> Option<Rect> {
    if container.width <= 0.0
        || container.height <= 0.0
        || image_size.width == 0
        || image_size.height == 0
    {
        return None;
    }

    let image_width = image_size.width as f32;
    let image_height = image_size.height as f32;
    let scale_x = container.width / image_width;
    let scale_y = container.height / image_height;
    let (draw_width, draw_height) = match image_style.fit {
        ImageFit::Fill => (container.width, container.height),
        ImageFit::Contain => scaled_size(image_width, image_height, scale_x.min(scale_y)),
        ImageFit::Cover => scaled_size(image_width, image_height, scale_x.max(scale_y)),
        ImageFit::None => (image_width, image_height),
        ImageFit::ScaleDown => {
            scaled_size(image_width, image_height, scale_x.min(scale_y).min(1.0))
        }
    };

    Some(aligned_rect(
        container,
        Size::new(draw_width, draw_height),
        image_style.alignment,
    ))
}

fn scaled_size(width: f32, height: f32, scale: f32) -> (f32, f32) {
    (width * scale, height * scale)
}

fn aligned_rect(container: Rect, size: Size<f32>, alignment: Alignment) -> Rect {
    Rect::new(
        container.x + (container.width - size.width) * alignment.x,
        container.y + (container.height - size.height) * alignment.y,
        size.width,
        size.height,
    )
}

fn repeated_image_bounds(container: Rect, tile: Rect, repeat: ImageRepeat) -> Rect {
    let repeat_x = matches!(repeat, ImageRepeat::Repeat | ImageRepeat::RepeatX);
    let repeat_y = matches!(repeat, ImageRepeat::Repeat | ImageRepeat::RepeatY);
    Rect::new(
        if repeat_x { container.x } else { tile.x },
        if repeat_y { container.y } else { tile.y },
        if repeat_x {
            container.width
        } else {
            tile.width
        },
        if repeat_y {
            container.height
        } else {
            tile.height
        },
    )
}

fn shadow_bounds(shape: Rect, offset: Point, blur: f32, spread: f32) -> Rect {
    let center = Point::new(
        shape.x + shape.width * 0.5 + offset.x,
        shape.y + shape.height * 0.5 + offset.y,
    );
    let half_width = (shape.width * 0.5 + spread).max(0.0) + blur.max(0.0) * 3.0;
    let half_height = (shape.height * 0.5 + spread).max(0.0) + blur.max(0.0) * 3.0;
    Rect::new(
        center.x - half_width,
        center.y - half_height,
        half_width * 2.0,
        half_height * 2.0,
    )
}

fn relative_point_in_rect(rect: Rect, point: Point) -> Point {
    Point::new(
        rect.x + rect.width * point.x,
        rect.y + rect.height * point.y,
    )
}

fn inflate_rect(rect: Rect, amount: f32) -> Rect {
    Rect::new(
        rect.x - amount,
        rect.y - amount,
        rect.width + amount * 2.0,
        rect.height + amount * 2.0,
    )
}

fn stroke_outset(width: f32, direction: f32) -> f32 {
    let width = width.max(0.0);
    if direction > 0.0 {
        width
    } else if direction < 0.0 {
        0.0
    } else {
        width * 0.5
    }
}

fn intersect_rect(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);

    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

fn rect_to_array(rect: Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}

fn color_to_array(color: Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AllocInfo {
    pub total_size: Vec3,
    pub layer: u32,
    pub origin: Vec2,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(width: u32, height: u32) -> ImageData {
        ImageData::rgba8(
            Size::new(width, height),
            vec![255; width as usize * height as usize * 4],
        )
    }

    #[test]
    fn physical_scissor_scales_expands_and_clamps_to_target() {
        assert_eq!(
            physical_scissor(Rect::new(-0.25, 1.25, 10.5, 4.5), 2.0, (20, 20)),
            Some((0, 2, 20, 10))
        );
        assert_eq!(
            physical_scissor(Rect::new(30.0, 0.0, 5.0, 5.0), 1.0, (20, 20)),
            None
        );
    }

    fn image_command(data: ImageData, style: ImageStyle) -> ImagePaintCommand {
        ImagePaintCommand {
            key: ImageKey::UserProvided(1),
            data,
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            opacity: 1.0,
            variant: ImageVariant::default(),
            style,
        }
    }

    #[test]
    fn image_record_contain_fit_uses_fitted_tile_rect() {
        let mut result = PrepareResult::default();
        let command = image_command(
            pixels(2, 1),
            ImageStyle {
                fit: ImageFit::Contain,
                sampling: Sampling::Nearest,
                ..ImageStyle::default()
            },
        );

        result.push_image_record(&command, command.rect, Rect::new(0.0, 0.0, 100.0, 100.0));

        assert_eq!(result.image_records.len(), 1);
        let record = &result.image_records[0];
        assert_eq!(record.rect, Rect::new(0.0, 25.0, 100.0, 50.0));
        assert_eq!(record.clip, Rect::new(0.0, 25.0, 100.0, 50.0));
        assert_eq!(record.tile, Rect::new(0.0, 25.0, 100.0, 50.0));
        assert_eq!(record.repeat, ImageRepeat::NoRepeat);
        assert_eq!(record.variant.sampling, Sampling::Nearest);
    }

    #[test]
    fn image_record_cover_fit_clips_to_container() {
        let mut result = PrepareResult::default();
        let command = image_command(
            pixels(2, 1),
            ImageStyle {
                fit: ImageFit::Cover,
                ..ImageStyle::default()
            },
        );

        result.push_image_record(&command, command.rect, Rect::new(0.0, 0.0, 100.0, 100.0));

        assert_eq!(result.image_records.len(), 1);
        let record = &result.image_records[0];
        assert_eq!(record.rect, Rect::new(-50.0, 0.0, 200.0, 100.0));
        assert_eq!(record.clip, Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(record.tile, Rect::new(-50.0, 0.0, 200.0, 100.0));
    }

    #[test]
    fn image_record_repeat_x_draws_container_axis_with_tile_uvs() {
        let mut result = PrepareResult::default();
        let command = image_command(
            pixels(10, 10),
            ImageStyle {
                fit: ImageFit::None,
                alignment: Alignment::START,
                repeat: ImageRepeat::RepeatX,
                ..ImageStyle::default()
            },
        );
        let rect = Rect::new(0.0, 0.0, 25.0, 10.0);

        result.push_image_record(&command, rect, rect);

        assert_eq!(result.image_records.len(), 1);
        let record = &result.image_records[0];
        assert_eq!(record.rect, Rect::new(0.0, 0.0, 25.0, 10.0));
        assert_eq!(record.clip, Rect::new(0.0, 0.0, 25.0, 10.0));
        assert_eq!(record.tile, Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(record.repeat, ImageRepeat::RepeatX);
    }

    #[test]
    fn scale_down_uses_natural_size_until_image_would_overflow() {
        let small = fitted_image_rect(
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Size::new(20, 10),
            ImageStyle {
                fit: ImageFit::ScaleDown,
                ..ImageStyle::default()
            },
        )
        .unwrap();
        assert_eq!(small, Rect::new(40.0, 45.0, 20.0, 10.0));

        let large = fitted_image_rect(
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Size::new(200, 100),
            ImageStyle {
                fit: ImageFit::ScaleDown,
                ..ImageStyle::default()
            },
        )
        .unwrap();
        assert_eq!(large, Rect::new(0.0, 25.0, 100.0, 50.0));
    }
}
