use std::sync::Arc;

use etagere::{Allocation, AllocatorOptions};
use etagere::{BucketedAtlasAllocator, Size};
use glam::{Vec2, Vec3};
use wgpu::util::DeviceExt;
use xui_interface::{
    Color, DamageRegion, PaintCommand, Point, Rect, RenderBackend, TextLayoutConstraints,
    TextPaintCommand,
};
use xui_text::atlas::{FontRenderBackend, GlyphAtlas, RendedGlyphBitmap};
use xui_text::engine::TextLayouter;
use xui_text::typ::TextRunStyle;

use crate::sdf::UI_SHADER_WGSL;

pub type WgpuBackendError = Box<dyn std::error::Error + Send + Sync>;

pub struct WGPUBackend {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    ui_uniform_buffer: wgpu::Buffer,
    ui_bind_group: wgpu::BindGroup,
    atlas: Atlas,
    glyph_atlas: GlyphAtlas<AllocInfo>,
    last_text_glyph_records: Vec<TextGlyphRecord>,
    scene: SceneTexture,
    scene_needs_clear: bool,
    presented_frame: bool,
}

const SHAPE_RECT: f32 = 0.0;
const SHAPE_ROUNDED_RECT: f32 = 1.0;
const SHAPE_LINE: f32 = 2.0;
const STROKE_CENTER: f32 = 0.0;

const UI_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 10] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x4,
    2 => Float32x4,
    3 => Float32x4,
    4 => Float32x4,
    5 => Float32x4,
    6 => Float32x4,
    7 => Float32x4,
    8 => Float32x4,
    9 => Float32x4,
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct UiUniforms {
    viewport_size: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct UiInstance {
    bounds: [f32; 4],
    shape: [f32; 4],
    clip: [f32; 4],
    fill_color: [f32; 4],
    stroke_color: [f32; 4],
    params: [f32; 4],
    stroke_params: [f32; 4],
    projection_color: [f32; 4],
    projection_params: [f32; 4],
    extra: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGlyphRecord {
    pub screen_rect: Rect,
    pub clip: Rect,
    pub color: Color,
    pub atlas_origin: Vec2,
    pub atlas_layer: u32,
    pub atlas_size: Vec3,
    pub atlas_rect: Rect,
}

impl UiInstance {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &UI_INSTANCE_ATTRIBUTES,
        }
    }
}

impl WGPUBackend {
    pub fn new(window: Arc<winit::window::Window>) -> Self {
        pollster::block_on(Self::new_(window))
    }

    pub fn last_text_glyph_records(&self) -> &[TextGlyphRecord] {
        &self.last_text_glyph_records
    }

    async fn new_(window: Arc<winit::window::Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(Arc::clone(&window))
            .expect("failed to create surface");

        // 3. 选择 Adapter，可以理解为选择一个合适的 GPU / 后端
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

        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST;

        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui sdf shader"),
            source: wgpu::ShaderSource::Wgsl(UI_SHADER_WGSL.into()),
        });

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
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let ui_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui sdf bind group"),
            layout: &ui_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ui_uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui sdf pipeline layout"),
            bind_group_layouts: &[Some(&ui_bind_group_layout)],
            immediate_size: 0,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui sdf render pipeline"),
            layout: Some(&pipeline_layout),

            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[UiInstance::layout()],
            },

            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),

            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },

            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let atlas = Atlas::new(&device);
        let glyph_atlas = GlyphAtlas::new();
        let scene = SceneTexture::new(&device, &config);

        Self {
            instance,
            adapter,
            surface,
            device,
            queue,
            config,
            render_pipeline,
            ui_uniform_buffer,
            ui_bind_group,
            atlas,
            glyph_atlas,
            last_text_glyph_records: Vec::new(),
            scene,
            scene_needs_clear: true,
            presented_frame: false,
        }
    }
}

struct SceneTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
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
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }
}

struct Atlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    depth: u32,
    current_layer: u32,
    sampler: wgpu::Sampler,
    allocator: BucketedAtlasAllocator,
    size: Size,
    total_size: Vec3,
}

impl Atlas {
    fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("GlyphAtlas3D"),
            size: wgpu::Extent3d {
                width: 1024,
                height: 1024,
                depth_or_array_layers: 128,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let allocator = BucketedAtlasAllocator::with_options(
            Size::new(1024, 1024),
            &AllocatorOptions::default(),
        );

        Self {
            texture,
            current_layer: 0,
            sampler,
            view,
            allocator,
            depth: 128,
            size: Size::new(1024, 1024),
            total_size: Vec3::new(1024.0, 1024.0, 128.0),
        }
    }

    fn handle_allocation(
        &mut self,
        queue: &wgpu::Queue,
        bitmap: &RendedGlyphBitmap,
    ) -> Result<AllocInfo, crate::error::Error> {
        if let Some(alloc) = self
            .allocator
            .allocate(Size::new(bitmap.width as i32, bitmap.height as i32))
        {
            let layer = self.current_layer;
            self.write_glyph_to_texture(queue, &bitmap, alloc);
            return Ok(AllocInfo {
                total_size: self.total_size,
                layer,
                origin: Vec2::new(alloc.rectangle.min.x as f32, alloc.rectangle.min.y as f32),
            });
        }

        if self.current_layer + 1 < self.depth {
            self.current_layer += 1;
            self.allocator =
                BucketedAtlasAllocator::with_options(self.size, &AllocatorOptions::default());

            if let Some(alloc) = self
                .allocator
                .allocate(Size::new(bitmap.width as i32, bitmap.height as i32))
            {
                let layer = self.current_layer;
                self.write_glyph_to_texture(queue, &bitmap, alloc);

                return Ok(AllocInfo {
                    total_size: self.total_size,
                    layer,
                    origin: Vec2::new(alloc.rectangle.min.x as f32, alloc.rectangle.min.y as f32),
                });
            }
        }

        Err(crate::error::Error::Other(
            "Failed to allocate glyph".into(),
        ))
    }

    fn write_glyph_to_texture(
        &self,
        queue: &wgpu::Queue,
        bitmap: &RendedGlyphBitmap,
        alloc: Allocation,
    ) {
        let layer = self.current_layer;

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: alloc.rectangle.min.x as u32,
                    y: alloc.rectangle.min.y as u32,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bitmap.data.as_slice(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bitmap.width * 4),
                rows_per_image: Some(bitmap.height),
            },
            wgpu::Extent3d {
                width: bitmap.width,
                height: bitmap.height,
                depth_or_array_layers: 1,
            },
        );
    }
}

impl<T: TextLayouter> RenderBackend<T> for WGPUBackend {
    type Error = WgpuBackendError;

    fn begin_frame(&mut self, size: xui_interface::Size) -> Result<(), Self::Error> {
        let width = size.width.max(1.0) as u32;
        let height = size.height.max(1.0) as u32;
        if self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.scene = SceneTexture::new(&self.device, &self.config);
            self.scene_needs_clear = true;
        }
        self.queue.write_buffer(
            &self.ui_uniform_buffer,
            0,
            bytemuck::bytes_of(&UiUniforms {
                viewport_size: [width as f32, height as f32, 0.0, 0.0],
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
        text: &mut T,
    ) -> Result<(), Self::Error> {
        let _ = (&self.instance, &self.adapter);
        self.presented_frame = false;
        let scene_clip = damage.bounds().unwrap_or(Rect::new(
            0.0,
            0.0,
            self.config.width as f32,
            self.config.height as f32,
        ));
        let (instances, text_glyph_records) =
            self.build_ui_instances(commands, scene_clip, text)?;
        self.last_text_glyph_records = text_glyph_records;

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.scene = SceneTexture::new(&self.device, &self.config);
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

        let instance_buffer = (!instances.is_empty()).then(|| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("xui sdf instances"),
                    contents: bytemuck::cast_slice(&instances),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });

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
            pass.set_pipeline(&self.render_pipeline);
            pass.set_bind_group(0, &self.ui_bind_group, &[]);

            if let Some(instance_buffer) = &instance_buffer {
                if let Some(scissor) =
                    scissor_rect(scene_clip, self.config.width, self.config.height)
                {
                    pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
                }
                pass.set_vertex_buffer(0, instance_buffer.slice(..));
                pass.draw(0..6, 0..instances.len() as u32);
            }
        }
        self.scene_needs_clear = false;

        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.scene.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &frame.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.scene.width,
                height: self.scene.height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        self.presented_frame = true;
        Ok(())
    }
}

impl WGPUBackend {
    fn build_ui_instances<T: TextLayouter>(
        &mut self,
        commands: &[PaintCommand],
        viewport_clip: Rect,
        text: &mut T,
    ) -> Result<(Vec<UiInstance>, Vec<TextGlyphRecord>), WgpuBackendError> {
        let mut instances = Vec::new();
        let mut text_glyph_records = Vec::new();
        let mut transform_stack = vec![Point::new(0.0, 0.0)];
        let mut clip_stack = vec![viewport_clip];

        for command in commands {
            match command {
                PaintCommand::FillRect { rect, color } => {
                    let rect = translate_rect(*rect, current_transform(&transform_stack));
                    push_rect_instance(
                        &mut instances,
                        rect,
                        0.0,
                        *color,
                        Color::TRANSPARENT,
                        0.0,
                        current_clip(&clip_stack),
                    );
                }
                PaintCommand::StrokeRect { rect, color, width } => {
                    let rect = translate_rect(*rect, current_transform(&transform_stack));
                    push_rect_instance(
                        &mut instances,
                        rect,
                        0.0,
                        Color::TRANSPARENT,
                        *color,
                        *width,
                        current_clip(&clip_stack),
                    );
                }
                PaintCommand::FillRoundedRect {
                    rect,
                    radius,
                    color,
                } => {
                    let rect = translate_rect(*rect, current_transform(&transform_stack));
                    push_rect_instance(
                        &mut instances,
                        rect,
                        *radius,
                        *color,
                        Color::TRANSPARENT,
                        0.0,
                        current_clip(&clip_stack),
                    );
                }
                PaintCommand::StrokeRoundedRect {
                    rect,
                    radius,
                    color,
                    width,
                } => {
                    let rect = translate_rect(*rect, current_transform(&transform_stack));
                    push_rect_instance(
                        &mut instances,
                        rect,
                        *radius,
                        Color::TRANSPARENT,
                        *color,
                        *width,
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
                    push_line_instance(
                        &mut instances,
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
                    self.push_text_glyph_records(
                        command,
                        rect,
                        clip,
                        text,
                        &mut text_glyph_records,
                    )?;
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
                    push_rect_instance(
                        &mut instances,
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

        Ok((instances, text_glyph_records))
    }

    fn push_text_glyph_records<T: TextLayouter>(
        &mut self,
        command: &TextPaintCommand,
        rect: Rect,
        clip: Rect,
        text: &mut T,
        records: &mut Vec<TextGlyphRecord>,
    ) -> Result<(), WgpuBackendError> {
        if rect.width <= 0.0
            || rect.height <= 0.0
            || clip.width <= 0.0
            || clip.height <= 0.0
            || command.props.style.color.a <= 0.0
            || command.props.text.as_str().is_empty()
        {
            return Ok(());
        }

        let par = text.layout_text(&command.props, TextLayoutConstraints::max_width(rect.width));
        for line in par.lines() {
            let baseline_y = rect.y + line.baseline();
            let mut pen_x = rect.x + line.offset();

            for run in line.runs() {
                let style = TextRunStyle {
                    font: run.font().as_ref(),
                    font_coords: run.normalized_coords(),
                    font_size: run.font_size(),
                    baseline: baseline_y,
                    advance: run.advance(),
                };
                let mut writer = WgpuGlyphWriter {
                    queue: &self.queue,
                    atlas: &mut self.atlas,
                };
                let mut session = self.glyph_atlas.session(&style, &mut writer);

                for cluster in run.visual_clusters() {
                    for glyph in cluster.glyphs() {
                        let origin_x = pen_x + glyph.x;
                        let origin_y = baseline_y + glyph.y;
                        let Some((alloc, placement)) = session.get(glyph.id, origin_x, origin_y)
                        else {
                            continue;
                        };
                        if placement.width == 0 || placement.height == 0 {
                            continue;
                        }

                        let screen_rect = Rect::new(
                            origin_x + placement.left as f32,
                            origin_y + placement.top as f32,
                            placement.width as f32,
                            placement.height as f32,
                        );
                        if intersect_rect(clip, screen_rect).is_none() {
                            continue;
                        }

                        records.push(TextGlyphRecord {
                            screen_rect,
                            clip,
                            color: command.props.style.color,
                            atlas_origin: alloc.origin,
                            atlas_layer: alloc.layer,
                            atlas_size: alloc.total_size,
                            atlas_rect: Rect::new(
                                alloc.origin.x,
                                alloc.origin.y,
                                placement.width as f32,
                                placement.height as f32,
                            ),
                        });
                    }
                    pen_x += cluster.advance();
                }
            }
        }

        Ok(())
    }
}

fn push_rect_instance(
    instances: &mut Vec<UiInstance>,
    rect: Rect,
    radius: f32,
    fill_color: Color,
    stroke_color: Color,
    stroke_width: f32,
    clip: Rect,
) {
    if rect.width <= 0.0
        || rect.height <= 0.0
        || clip.width <= 0.0
        || clip.height <= 0.0
        || (fill_color.a <= 0.0 && stroke_color.a <= 0.0)
    {
        return;
    }

    let stroke_direction = STROKE_CENTER;
    let projection_color = Color::TRANSPARENT;
    let projection_offset = Point::new(0.0, 0.0);
    let projection_blur: f32 = 0.0;
    let projection_spread: f32 = 0.0;

    let stroke_outset = stroke_outset(stroke_width.max(0.0), stroke_direction) + 1.0;
    let projection_outset = projection_blur.max(0.0) + projection_spread.max(0.0);
    let projection_bounds = inflate_rect(
        translate_rect(rect, projection_offset),
        projection_outset + 1.0,
    );
    let bounds = inflate_rect(rect, stroke_outset).union(projection_bounds);
    let kind = if radius > 0.0 {
        SHAPE_ROUNDED_RECT
    } else {
        SHAPE_RECT
    };

    instances.push(UiInstance {
        bounds: rect_to_array(bounds),
        shape: rect_to_array(rect),
        clip: rect_to_array(clip),
        fill_color: color_to_array(fill_color),
        stroke_color: color_to_array(stroke_color),
        params: [kind, radius.max(0.0), 0.0, 0.0],
        stroke_params: [stroke_width.max(0.0), stroke_direction, 0.0, 0.0],
        projection_color: color_to_array(projection_color),
        projection_params: [
            projection_offset.x,
            projection_offset.y,
            projection_blur.max(0.0),
            projection_spread,
        ],
        extra: [0.0; 4],
    });
}

fn push_line_instance(
    instances: &mut Vec<UiInstance>,
    from: Point,
    to: Point,
    color: Color,
    width: f32,
    clip: Rect,
) {
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

    instances.push(UiInstance {
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

fn scissor_rect(rect: Rect, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let x0 = rect.x.floor().max(0.0).min(width as f32) as u32;
    let y0 = rect.y.floor().max(0.0).min(height as f32) as u32;
    let x1 = (rect.x + rect.width).ceil().max(0.0).min(width as f32) as u32;
    let y1 = (rect.y + rect.height).ceil().max(0.0).min(height as f32) as u32;

    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
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

struct WgpuGlyphWriter<'a> {
    queue: &'a wgpu::Queue,
    atlas: &'a mut Atlas,
}

impl FontRenderBackend for WgpuGlyphWriter<'_> {
    type Error = crate::error::Error;
    type Allocation = AllocInfo;

    fn write_bitmap(
        &mut self,
        bitmap: &xui_text::atlas::RendedGlyphBitmap,
    ) -> Result<Self::Allocation, Self::Error> {
        self.atlas.handle_allocation(self.queue, bitmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_shader_is_valid_wgsl() {
        let module =
            wgpu::naga::front::wgsl::parse_str(UI_SHADER_WGSL).expect("ui.wgsl should parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("ui.wgsl should validate");
    }
}
