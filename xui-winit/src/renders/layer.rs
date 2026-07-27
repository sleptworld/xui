use std::{ops::Range, sync::Arc};

use wgpu::util::DeviceExt;
use xui::render::LayerEffect;
use xui_interface::{Affine, ImageData, ImageDataId, ImageFormat, Point, Rect};

use crate::wgpu::{SCENE_FORMAT, SCENE_SAMPLE_COUNT, WgpuBackendError, physical_scissor};

const TILE_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileVertex {
    position: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
}

impl TileVertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TILE_VERTEX_ATTRIBUTES,
        }
    }
}

pub struct LayerTileDrawRecord {
    vertices: [TileVertex; 6],
    pub bind_group: Arc<wgpu::BindGroup>,
    pub clip: Rect,
}

impl std::fmt::Debug for LayerTileDrawRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerTileDrawRecord")
            .field("clip", &self.clip)
            .finish_non_exhaustive()
    }
}

pub struct LayerTileRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
}

impl LayerTileRenderer {
    pub fn new(device: &wgpu::Device, ui_layout: &wgpu::BindGroupLayout) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xui layer tile bind group layout"),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xui layer tile sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui layer tile shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/layer_tile.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui layer tile pipeline layout"),
            bind_group_layouts: &[Some(ui_layout), Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui layer tile pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[TileVertex::layout()],
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
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: SCENE_SAMPLE_COUNT,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xui empty layer tile vertices"),
            size: 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bind_group_layout,
            sampler,
            vertex_buffer,
            vertex_capacity: 0,
        }
    }

    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
    ) -> Arc<wgpu::BindGroup> {
        Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui layer tile bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }))
    }

    pub fn record(
        &self,
        bind_group: Arc<wgpu::BindGroup>,
        source_bounds: Rect,
        uv: Rect,
        transform: Affine,
        opacity: f32,
        clip: Rect,
    ) -> LayerTileDrawRecord {
        let p0 = transform.transform_point(Point::new(source_bounds.x, source_bounds.y));
        let p1 = transform.transform_point(Point::new(
            source_bounds.x + source_bounds.width,
            source_bounds.y,
        ));
        let p2 = transform.transform_point(Point::new(
            source_bounds.x,
            source_bounds.y + source_bounds.height,
        ));
        let p3 = transform.transform_point(Point::new(
            source_bounds.x + source_bounds.width,
            source_bounds.y + source_bounds.height,
        ));
        let u0 = [uv.x, uv.y];
        let u1 = [uv.x + uv.width, uv.y];
        let u2 = [uv.x, uv.y + uv.height];
        let u3 = [uv.x + uv.width, uv.y + uv.height];
        let opacity = opacity.clamp(0.0, 1.0);
        let vertex = |point: Point, uv| TileVertex {
            position: [point.x, point.y],
            uv,
            opacity,
        };
        LayerTileDrawRecord {
            vertices: [
                vertex(p0, u0),
                vertex(p1, u1),
                vertex(p2, u2),
                vertex(p2, u2),
                vertex(p1, u1),
                vertex(p3, u3),
            ],
            bind_group,
            clip,
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        records: &[LayerTileDrawRecord],
    ) {
        let vertices: Vec<_> = records.iter().flat_map(|record| record.vertices).collect();
        if vertices.is_empty() {
            return;
        }
        let bytes = bytemuck::cast_slice(&vertices);
        if bytes.len() > self.vertex_capacity {
            self.vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("xui layer tile vertices"),
                contents: bytes,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            self.vertex_capacity = bytes.len();
        } else {
            queue.write_buffer(&self.vertex_buffer, 0, bytes);
        }
    }

    pub fn render_range(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        ui_bind_group: &wgpu::BindGroup,
        records: &[LayerTileDrawRecord],
        range: Range<usize>,
        scale_factor: f32,
        target_size: (u32, u32),
    ) {
        if range.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, ui_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        for index in range {
            let record = &records[index];
            let Some((x, y, width, height)) =
                physical_scissor(record.clip, scale_factor, target_size)
            else {
                continue;
            };
            pass.set_scissor_rect(x, y, width, height);
            pass.set_bind_group(1, record.bind_group.as_ref(), &[]);
            let start = (index * 6) as u32;
            pass.draw(start..start + 6, 0..1);
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EffectUniform {
    params0: [f32; 4],
    params1: [f32; 4],
    color: [f32; 4],
    texture_size: [f32; 4],
    target_origin_scale: [f32; 4],
    mask_bounds: [f32; 4],
    matrix: [[f32; 4]; 5],
}

struct MaskTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

pub struct LayerEffectRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white_mask: MaskTexture,
    masks: std::collections::HashMap<ImageDataId, MaskTexture>,
}

impl LayerEffectRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xui layer effect bind group layout"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xui layer effect sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui layer effect shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/layer_effect.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui layer effect pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui layer effect pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: SCENE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let white_mask = create_mask_texture(device, None, "xui white effect mask")
            .expect("the built-in white mask is valid");
        Self {
            pipeline,
            bind_group_layout,
            sampler,
            white_mask,
            masks: std::collections::HashMap::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        effect: &LayerEffect,
        blur_direction: Option<[f32; 2]>,
        target_size: (u32, u32),
        target_origin: Point,
        scale_factor: f32,
    ) -> Result<(), WgpuBackendError> {
        let (kind, sigma, params1, color, mask, mask_bounds, matrix) = match effect {
            LayerEffect::Blur { sigma } => (
                0.0,
                sigma.max(0.0) * scale_factor,
                [0.0; 4],
                [0.0; 4],
                None,
                [0.0; 4],
                identity_color_matrix(),
            ),
            LayerEffect::DropShadow(shadow) => (
                1.0,
                shadow.blur.max(0.0) * scale_factor,
                [
                    shadow.offset.x * scale_factor,
                    shadow.offset.y * scale_factor,
                    shadow.spread * scale_factor,
                    0.0,
                ],
                [
                    shadow.color.r,
                    shadow.color.g,
                    shadow.color.b,
                    shadow.color.a,
                ],
                None,
                [0.0; 4],
                identity_color_matrix(),
            ),
            LayerEffect::ColorMatrix { matrix } => (
                2.0,
                0.0,
                [0.0; 4],
                [0.0; 4],
                None,
                [0.0; 4],
                matrix_to_rows(*matrix),
            ),
            LayerEffect::Mask { data, bounds, .. } => (
                3.0,
                0.0,
                [0.0; 4],
                [0.0; 4],
                Some(data),
                [bounds.x, bounds.y, bounds.width, bounds.height],
                identity_color_matrix(),
            ),
        };
        let mask_view = if let Some(data) = mask {
            if !self.masks.contains_key(&data.id()) {
                let texture = create_mask_texture(device, Some((queue, data)), "xui layer mask")?;
                self.masks.insert(data.id(), texture);
            }
            &self.masks[&data.id()].view
        } else {
            &self.white_mask.view
        };
        let direction = blur_direction.unwrap_or([0.0, 0.0]);
        let uniform = EffectUniform {
            params0: [kind, sigma, direction[0], direction[1]],
            params1,
            color,
            texture_size: [target_size.0 as f32, target_size.1 as f32, 0.0, 0.0],
            target_origin_scale: [target_origin.x, target_origin.y, scale_factor, 0.0],
            mask_bounds,
            matrix,
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("xui layer effect uniform"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui layer effect bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(mask_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xui layer effect pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn create_mask_texture(
    device: &wgpu::Device,
    source: Option<(&wgpu::Queue, &ImageData)>,
    label: &str,
) -> Result<MaskTexture, WgpuBackendError> {
    let (width, height, pixels, format) = match source {
        Some((_, data)) => {
            if data.size.width == 0
                || data.size.height == 0
                || data.pixels.len() != data.size.width as usize * data.size.height as usize * 4
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "invalid layer mask image data",
                )
                .into());
            }
            let format = match data.format {
                ImageFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            };
            (
                data.size.width,
                data.size.height,
                data.pixels.as_ref(),
                format,
            )
        }
        None => (
            1,
            1,
            &[255, 255, 255, 255][..],
            wgpu::TextureFormat::Rgba8Unorm,
        ),
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if let Some((queue, _)) = source {
        queue.write_texture(
            texture.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    } else {
        // The built-in mask is only sampled as opaque white. Initializing it is
        // deferred to a render-independent clear by using an RGBA8 texture.
        // wgpu zero-initializes resources, so upload through a temporary queue is
        // unavailable here; the shader ignores the mask for non-mask effects.
        let _ = pixels;
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(MaskTexture {
        _texture: texture,
        view,
    })
}

fn identity_color_matrix() -> [[f32; 4]; 5] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
        [0.0; 4],
    ]
}

fn matrix_to_rows(matrix: [f32; 20]) -> [[f32; 4]; 5] {
    [
        [matrix[0], matrix[1], matrix[2], matrix[3]],
        [matrix[5], matrix[6], matrix[7], matrix[8]],
        [matrix[10], matrix[11], matrix[12], matrix[13]],
        [matrix[15], matrix[16], matrix[17], matrix[18]],
        [matrix[4], matrix[9], matrix[14], matrix[19]],
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn layer_shaders_parse_as_wgsl() {
        for source in [
            include_str!("../shaders/layer_tile.wgsl"),
            include_str!("../shaders/layer_effect.wgsl"),
        ] {
            let module =
                naga::front::wgsl::parse_str(source).expect("layer shader must parse as WGSL");
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("layer shader must pass naga validation");
        }
    }
}
