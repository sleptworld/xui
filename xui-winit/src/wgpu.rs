use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use etagere::{Allocation, AllocatorOptions};
use etagere::{BucketedAtlasAllocator, Size};
use glam::{Vec2, Vec3};
use wgpu::util::DeviceExt;
use xui_interface::{
    Color, ComputedColorStyle, ComputedShadowStyle, ComputedStrokeStyle, DamageRegion, GlyphBitmap,
    GlyphPlacement, PaintCommand, Point, Rect, RenderBackend, TextLayoutBackend, TextPaintCommand,
};

use crate::sdf::UI_SHADER_WGSL;
use crate::text_cache::WinitTextEngine;

pub type WgpuBackendError = Box<dyn std::error::Error + Send + Sync>;

pub struct WGPUBackend<T: TextLayoutBackend = WinitTextEngine> {
    // Instances
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    glyph_pipelines: [wgpu::RenderPipeline; 3],
    // Composite State
    composite_pipeline: wgpu::RenderPipeline,
    composite_bind_group_layout: wgpu::BindGroupLayout,
    composite_sampler: wgpu::Sampler,
    composite_bind_group: wgpu::BindGroup,
    // Common Tools
    ui_uniform_buffer: wgpu::Buffer,
    ui_bind_group: wgpu::BindGroup,
    glyph_bind_group: wgpu::BindGroup,
    atlas: Atlas,
    glyph_cache: GlyphTextureCache<T::GlyphKey>,
    last_text_glyph_records: Vec<TextGlyphRecord>,
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
    scale_factor: [f32; 4],
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlyphInstance {
    ptype: u32,
    bounds: [f32; 4],
    layer: f32,
    padding: [f32; 3],
    uv: [f32; 4],
    color: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PType {
    Mask,
    SubPixelMask,
    Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGlyphRecord {
    pub ptype: PType,
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

impl<T: TextLayoutBackend> WGPUBackend<T> {
    pub fn new(window: Arc<winit::window::Window>) -> Self {
        pollster::block_on(Self::new_(window))
    }

    pub fn last_text_glyph_records(&self) -> &[TextGlyphRecord] {
        &self.last_text_glyph_records
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui sdf shader"),
            source: wgpu::ShaderSource::Wgsl(UI_SHADER_WGSL.into()),
        });

        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui composite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/composite.wgsl").into()),
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
                scale_factor: [scale_factor as f32; 4],
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

        let atlas = Atlas::new(&device);

        let composite_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("xui composite bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xui composite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui sdf pipeline layout"),
            bind_group_layouts: &[Some(&ui_bind_group_layout)],
            immediate_size: 0,
        });

        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("xui composite pipeline layout"),
                bind_group_layouts: &[Some(&composite_bind_group_layout)],
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
                    format: SCENE_FORMAT,
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
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui composite render pipeline"),
            layout: Some(&composite_pipeline_layout),

            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },

            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
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

        let glyph_cache = GlyphTextureCache::new();
        let scene = SceneTexture::new(&device, &config);
        let composite_bind_group = create_composite_bind_group(
            &device,
            &composite_bind_group_layout,
            &composite_sampler,
            &scene.view,
        );

        Self {
            instance,
            adapter,
            surface,
            device,
            queue,
            config,
            render_pipeline,
            glyph_pipelines,
            composite_pipeline,
            composite_bind_group_layout,
            composite_sampler,
            composite_bind_group,
            ui_uniform_buffer,
            ui_bind_group,
            glyph_bind_group,
            atlas,
            glyph_cache,
            last_text_glyph_records: Vec::new(),
            scene,
            scene_needs_clear: true,
            presented_frame: false,
            scale_factor: scale_factor as f32,
            _text: PhantomData,
        }
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

fn create_composite_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    scene_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("xui composite bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(scene_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

struct GlyphTextureCache<K> {
    glyphs: HashMap<K, Option<(AllocInfo, GlyphPlacement, u32)>>,
}

impl<K> GlyphTextureCache<K> {
    fn new() -> Self {
        Self {
            glyphs: HashMap::new(),
        }
    }
}

impl<K: Eq + std::hash::Hash> GlyphTextureCache<K> {
    fn get(&self, key: &K) -> Option<&Option<(AllocInfo, GlyphPlacement, u32)>> {
        self.glyphs.get(key)
    }

    fn insert(&mut self, key: K, value: Option<(AllocInfo, GlyphPlacement, u32)>) {
        self.glyphs.insert(key, value);
    }
}

impl<T: TextLayoutBackend> RenderBackend<T> for WGPUBackend<T> {
    type Error = WgpuBackendError;

    fn begin_frame(&mut self, size: xui_interface::Size<f32>) -> Result<(), Self::Error> {
        let width = (size.width * self.scale_factor).max(1.0).ceil() as u32;
        let height = (size.height * self.scale_factor).max(1.0).ceil() as u32;
        if self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.scene = SceneTexture::new(&self.device, &self.config);
            self.composite_bind_group = create_composite_bind_group(
                &self.device,
                &self.composite_bind_group_layout,
                &self.composite_sampler,
                &self.scene.view,
            );
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
        text: &mut T,
    ) -> Result<(), Self::Error> {
        let _ = (&self.instance, &self.adapter);
        self.presented_frame = false;
        let logical_scene_size = xui_interface::Size::<f32>::new(
            self.config.width as f32 / self.scale_factor,
            self.config.height as f32 / self.scale_factor,
        );
        let scene_clip = damage.bounds().unwrap_or(Rect::new(
            0.0,
            0.0,
            logical_scene_size.width,
            logical_scene_size.height,
        ));
        let (instances, glyph_instances, text_glyph_records) =
            self.build_ui_instances(commands, scene_clip, text)?;
        self.last_text_glyph_records = text_glyph_records;

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.scene = SceneTexture::new(&self.device, &self.config);
                self.composite_bind_group = create_composite_bind_group(
                    &self.device,
                    &self.composite_bind_group_layout,
                    &self.composite_sampler,
                    &self.scene.view,
                );
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
        let glyph_instance_buffer = (!glyph_instances.is_empty()).then(|| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("xui glyph instances"),
                    contents: bytemuck::cast_slice(&glyph_instances),
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

            if let Some(instance_buffer) = &instance_buffer {
                pass.set_pipeline(&self.render_pipeline);
                pass.set_bind_group(0, &self.ui_bind_group, &[]);
                pass.set_vertex_buffer(0, instance_buffer.slice(..));
                pass.draw(0..6, 0..instances.len() as u32);
            }

            if let Some(glyph_instance_buffer) = &glyph_instance_buffer {
                for pipeline in &self.glyph_pipelines {
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &self.ui_bind_group, &[]);
                    pass.set_bind_group(1, &self.glyph_bind_group, &[]);
                    pass.set_vertex_buffer(0, glyph_instance_buffer.slice(..));
                    pass.draw(0..4, 0..glyph_instances.len() as u32);
                }
            }
        }
        self.scene_needs_clear = false;

        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui composite render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &self.composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
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

impl<T: TextLayoutBackend> WGPUBackend<T> {
    fn build_ui_instances(
        &mut self,
        commands: &[PaintCommand],
        viewport_clip: Rect,
        text: &mut T,
    ) -> Result<(Vec<UiInstance>, Vec<GlyphInstance>, Vec<TextGlyphRecord>), WgpuBackendError> {
        let mut instances = Vec::new();
        let mut glyph_instances = Vec::new();
        let mut text_glyph_records = Vec::new();
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
                    push_paint_rect_instance(
                        &mut instances,
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
                    push_paint_rect_instance(
                        &mut instances,
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
                        &mut glyph_instances,
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

        Ok((instances, glyph_instances, text_glyph_records))
    }

    fn push_text_glyph_records(
        &mut self,
        command: &TextPaintCommand,
        rect: Rect,
        clip: Rect,
        text: &mut T,
        records: &mut Vec<TextGlyphRecord>,
        glyph_instances: &mut Vec<GlyphInstance>,
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

        let layout = if let Some(layout) = text.get_cached_layout(command.node_id) {
            layout
        } else {
            return Ok(());
        };

        let scale = 1. / self.scale_factor;
        let mut positioned_glyphs = Vec::new();
        text.visit_layout_glyphs(
            &layout,
            Point::new(rect.x, rect.y),
            self.scale_factor,
            &mut |glyph| positioned_glyphs.push(glyph),
        );

        for glyph in positioned_glyphs {
            let Some((alloc, placement, ptype)) = self.glyph_allocation(text, &glyph.key)? else {
                continue;
            };
            if placement.width == 0 || placement.height == 0 {
                continue;
            }

            let screen_rect = Rect::new(
                (glyph.physical_x + placement.left) as f32 * scale,
                (glyph.physical_y - placement.top) as f32 * scale,
                placement.width as f32 * scale,
                placement.height as f32 * scale,
            );
            if intersect_rect(clip, screen_rect).is_none() {
                continue;
            }

            let record = TextGlyphRecord {
                ptype,
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
            };
            push_glyph_instance(glyph_instances, &record);
            records.push(record);
        }
        Ok(())
    }

    fn glyph_allocation(
        &mut self,
        text: &mut T,
        key: &T::GlyphKey,
    ) -> Result<Option<(AllocInfo, GlyphPlacement, u32)>, WgpuBackendError> {
        if let Some(cached) = self.glyph_cache.get(key) {
            return Ok(*cached);
        }

        let value = if let Some(bitmap) = text.rasterize_glyph(key) {
            if bitmap.width == 0 || bitmap.height == 0 {
                None
            } else {
                Some((
                    self.atlas.handle_allocation(&self.queue, &bitmap)?,
                    bitmap.placement,
                    bitmap.ptype,
                ))
            }
        } else {
            None
        };
        self.glyph_cache.insert(key.clone(), value);
        Ok(value)
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
    push_projected_rect_instance(
        instances,
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

fn push_paint_rect_instance(
    instances: &mut Vec<UiInstance>,
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
        push_shadow_instance(
            instances,
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
        push_fill_style_instance(instances, rect, radius, fill, clip);
    }

    if let Some(stroke) = stroke.filter(|stroke| stroke.width > 0.0 && stroke.color.is_visible()) {
        push_stroke_style_instance(instances, rect, radius, stroke.color, stroke.width, clip);
    }
}

fn push_fill_style_instance(
    instances: &mut Vec<UiInstance>,
    rect: Rect,
    radius: f32,
    style: ComputedColorStyle,
    clip: Rect,
) {
    let style = InstanceColorStyle::new(style, rect);
    push_projected_rect_instance(
        instances,
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
    instances: &mut Vec<UiInstance>,
    rect: Rect,
    radius: f32,
    style: ComputedColorStyle,
    stroke_width: f32,
    clip: Rect,
) {
    let style = InstanceColorStyle::new(style, rect);
    push_projected_rect_instance(
        instances,
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
    instances: &mut Vec<UiInstance>,
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

    instances.push(UiInstance {
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
}

fn push_projected_rect_instance(
    instances: &mut Vec<UiInstance>,
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

    instances.push(UiInstance {
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

fn push_glyph_instance(instances: &mut Vec<GlyphInstance>, record: &TextGlyphRecord) {
    if record.screen_rect.width <= 0.0
        || record.screen_rect.height <= 0.0
        || record.clip.width <= 0.0
        || record.clip.height <= 0.0
        || record.color.a <= 0.0
        || record.atlas_size.x <= 0.0
        || record.atlas_size.y <= 0.0
        || record.atlas_size.z <= 0.0
    {
        return;
    }

    instances.push(GlyphInstance {
        ptype: record.ptype,
        bounds: rect_to_array(record.screen_rect),
        layer: record.atlas_layer as f32 / record.atlas_size.z,
        padding: [0.0; 3],
        uv: [
            record.atlas_rect.x / record.atlas_size.x,
            record.atlas_rect.y / record.atlas_size.y,
            record.atlas_rect.width / record.atlas_size.x,
            record.atlas_rect.height / record.atlas_size.y,
        ],
        color: color_to_array(record.color),
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
    use crate::{CosmicTextEngine, WinitTextEngine};
    use xui_interface::RenderBackend;

    fn assert_array_near(actual: [f32; 4], expected: [f32; 4]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 0.001,
                "expected {actual} to be near {expected}"
            );
        }
    }

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

    #[test]
    fn glyph_shader_is_valid_wgsl() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("shaders/glyph.wgsl"))
            .expect("glyph.wgsl should parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("glyph.wgsl should validate");
    }

    #[test]
    fn composite_shader_is_valid_wgsl() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("shaders/composite.wgsl"))
            .expect("composite.wgsl should parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("composite.wgsl should validate");
    }

    #[test]
    fn cosmic_text_engine_matches_wgpu_backend_bounds() {
        fn assert_backend<T, B>()
        where
            T: TextLayoutBackend,
            B: RenderBackend<T>,
        {
        }

        assert_backend::<
            WinitTextEngine<CosmicTextEngine>,
            WGPUBackend<WinitTextEngine<CosmicTextEngine>>,
        >();
    }

    #[test]
    fn rect_with_fill_stroke_and_shadow_generates_ordered_instances() {
        let mut instances = Vec::new();
        let rect = Rect::new(0.0, 0.0, 100.0, 80.0);
        let shadow = ComputedShadowStyle {
            color: Color::rgba(0.0, 0.0, 0.0, 0.4),
            offset: Point::new(1.0, -1.0),
            blur: 2.0,
            spread: 2.0,
        };
        let stroke = ComputedStrokeStyle {
            color: ComputedColorStyle::Solid(Color::WHITE),
            width: 2.0,
            line_style: xui_interface::StrokeLineStyle::Solid,
        };

        push_paint_rect_instance(
            &mut instances,
            rect,
            0.0,
            ComputedColorStyle::Solid(Color::BLACK),
            Some(stroke),
            Some(shadow),
            Rect::new(-20.0, -20.0, 140.0, 120.0),
        );

        assert_eq!(instances.len(), 3);
        assert_eq!(instances[0].params[3], 1.0);
        assert_array_near(instances[0].bounds, [-7.0, -9.0, 116.0, 96.0]);
        assert_array_near(instances[0].shape, [0.0, 0.0, 100.0, 80.0]);
        assert_eq!(instances[1].params[3], 0.0);
        assert_array_near(instances[1].shape, [0.0, 0.0, 100.0, 80.0]);
        assert_eq!(instances[2].stroke_params[0], 2.0);
    }

    #[test]
    fn rounded_rect_shadow_expands_bounds_without_moving_shape() {
        let mut instances = Vec::new();
        let rect = Rect::new(10.0, 20.0, 90.0, 70.0);
        let shadow = ComputedShadowStyle {
            color: Color::rgba(0.0, 0.0, 0.0, 0.5),
            offset: Point::new(-2.0, 3.0),
            blur: 4.0,
            spread: 1.0,
        };

        push_paint_rect_instance(
            &mut instances,
            rect,
            8.0,
            ComputedColorStyle::Solid(Color::TRANSPARENT),
            None,
            Some(shadow),
            Rect::new(-10.0, 0.0, 130.0, 120.0),
        );

        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].params[0], SHAPE_ROUNDED_RECT);
        assert_eq!(instances[0].params[1], 8.0);
        assert_array_near(instances[0].bounds, [-5.0, 10.0, 116.0, 96.0]);
        assert_array_near(instances[0].shape, [10.0, 20.0, 90.0, 70.0]);
    }

    #[test]
    fn shadow_bounds_tracks_large_shadow_offset_separately_from_layout_rect() {
        let rect = Rect::new(10.0, 0.0, 100.0, 50.0);

        let bounds = shadow_bounds(rect, Point::new(30.0, -25.0), 2.0, 4.0);

        assert_eq!(bounds, Rect::new(30.0, -35.0, 120.0, 70.0));
    }

    #[test]
    fn transparent_shadow_and_invisible_shape_emit_no_instances() {
        let mut instances = Vec::new();
        let rect = Rect::new(0.0, 0.0, 100.0, 80.0);
        let shadow = ComputedShadowStyle {
            color: Color::TRANSPARENT,
            offset: Point::new(0.0, 0.0),
            blur: 8.0,
            spread: 2.0,
        };

        push_paint_rect_instance(
            &mut instances,
            rect,
            0.0,
            ComputedColorStyle::Solid(Color::TRANSPARENT),
            None,
            Some(shadow),
            rect,
        );

        assert!(instances.is_empty());
    }
}
