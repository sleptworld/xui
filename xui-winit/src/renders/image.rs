use std::{collections::HashMap, sync::Arc};

use moka::sync::Cache;
use wgpu::util::DeviceExt;
use xui::{ImageFormat, ImageKey, ImageResource, Rect, Size};

use crate::wgpu::{SCENE_FORMAT, WgpuBackendError};

const IMAGE_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x4,
    2 => Float32x4,
];

const DEFAULT_IMAGE_TEXTURE_CACHE_CAPACITY: u64 = 256;

pub struct ImageRender {
    image_pipeline: wgpu::RenderPipeline,
    image_bind_group_layout: wgpu::BindGroupLayout,
    image_sampler: wgpu::Sampler,
    image_resources: HashMap<ImageKey, RegisteredImageResource>,
    image_textures: Cache<ImageKey, Arc<CachedImageTexture>>,

    instance_buffer: wgpu::Buffer,
    instance_count: u32,
    instance_size: usize,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ImageInstance {
    bounds: [f32; 4],
    clip: [f32; 4],
    params: [f32; 4],
}

impl ImageInstance {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &IMAGE_INSTANCE_ATTRIBUTES,
        }
    }
}

struct RegisteredImageResource {
    resource: ImageResource,
    version: u64,
}

struct CachedImageTexture {
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    version: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageDrawRecord {
    pub key: ImageKey,
    pub rect: Rect,
    pub clip: Rect,
    pub opacity: f32,
}

impl ImageRender {
    pub fn new(device: &wgpu::Device, tool_layout: &wgpu::BindGroupLayout) -> Self {
        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui image shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/image.wgsl").into()),
        });

        let image_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("xui image bind group layout"),
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
        let image_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xui image sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let image_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("xui image pipeline layout"),
                bind_group_layouts: &[Some(&tool_layout), Some(&image_bind_group_layout)],
                immediate_size: 0,
            });

        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui image render pipeline"),
            layout: Some(&image_pipeline_layout),

            vertex: wgpu::VertexState {
                module: &image_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[ImageInstance::layout()],
            },

            fragment: Some(wgpu::FragmentState {
                module: &image_shader,
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

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: 0,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            image_pipeline,
            image_bind_group_layout,
            image_sampler,
            image_resources: HashMap::new(),
            image_textures: Cache::new(DEFAULT_IMAGE_TEXTURE_CACHE_CAPACITY),
            instance_buffer,
            instance_count: 0,
            instance_size: 0,
        }
    }

    pub fn deal_records(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        records: &[ImageResource],
    ) -> Result<(), WgpuBackendError> {
        if records.is_empty() {
            self.instance_count = 0;
            return Ok(());
        }

        let mut instances: Vec<ImageInstance> = vec![];

        for record in records {}

        let instance_size = std::mem::size_of::<ImageInstance>();
        let instance_count = records.len() as u32;

        if instance_count > self.instance_count {
            self.instance_size = instance_size;
            self.instance_count = instance_count;
            self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("instance_buffer"),
                contents: unsafe { bytemuck::cast_slice(&instances) },
                usage: wgpu::BufferUsages::VERTEX,
            });
        }

        Ok(())

        // self.instance_size = instance_size;
        // self.instance_count = instance_count;
    }

    pub fn set_image_resource(&mut self, resource: ImageResource) -> Result<(), WgpuBackendError> {
        validate_image_resource(&resource)?;
        match self.image_resources.get_mut(&resource.key) {
            Some(existing) if existing.resource == resource => {}
            Some(existing) => {
                existing.resource = resource;
                existing.version = existing.version.saturating_add(1);
            }
            None => {
                self.image_resources.insert(
                    resource.key.clone(),
                    RegisteredImageResource {
                        resource,
                        version: 1,
                    },
                );
            }
        }
        Ok(())
    }

    pub fn set_rgba_image(
        &mut self,
        key: impl Into<ImageKey>,
        size: Size<u32>,
        pixels: impl Into<Arc<[u8]>>,
    ) -> Result<(), WgpuBackendError> {
        self.set_image_resource(ImageResource::rgba8(key, size, pixels))
    }

    pub fn remove_image_resource(&mut self, key: &ImageKey) {
        self.image_resources.remove(key);
        self.image_textures.invalidate(key);
    }

    pub fn clear_image_resources(&mut self) {
        self.image_resources.clear();
        self.image_textures.invalidate_all();
    }

    pub fn prepare_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        records: &[ImageDrawRecord],
    ) -> Result<(), WgpuBackendError> {
        for record in records {
            let Some(registered) = self.image_resources.get(&record.key) else {
                continue;
            };

            let cached = self.image_textures.get(&record.key);
            if cached
                .as_ref()
                .is_some_and(|texture| texture.version == registered.version)
            {
                continue;
            }

            let texture = Arc::new(create_cached_image_texture(
                device,
                queue,
                &self.image_bind_group_layout,
                &self.image_sampler,
                &registered.resource,
                registered.version,
            )?);
            self.image_textures.insert(record.key.clone(), texture);
        }

        Ok(())
    }

    pub fn render(
        &self,
        device: &wgpu::Device,
        pass: &mut wgpu::RenderPass<'_>,
        tool_bind_group: &wgpu::BindGroup,
        records: &[ImageDrawRecord],
    ) {
        let image_instances: Vec<ImageInstance> = records
            .iter()
            .map(|record| ImageInstance {
                bounds: rect_to_array(record.rect),
                clip: rect_to_array(record.clip),
                params: [record.opacity, 0.0, 0.0, 0.0],
            })
            .collect();

        if image_instances.is_empty() {
            return;
        }

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("xui image instances"),
            contents: bytemuck::cast_slice(&image_instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        pass.set_pipeline(&self.image_pipeline);
        pass.set_bind_group(0, tool_bind_group, &[]);
        pass.set_vertex_buffer(0, instance_buffer.slice(..));
        for (index, record) in records.iter().enumerate() {
            let Some(texture) = self.image_textures.get(&record.key) else {
                continue;
            };
            pass.set_bind_group(1, &texture.bind_group, &[]);
            pass.draw(0..6, index as u32..index as u32 + 1);
        }
    }
}

fn validate_image_resource(resource: &ImageResource) -> Result<(), WgpuBackendError> {
    if resource.size.width == 0 || resource.size.height == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "image resource size cannot be zero",
        )
        .into());
    }

    let bytes_per_pixel = match resource.format {
        ImageFormat::Rgba8UnormSrgb => 4usize,
    };
    let expected_len =
        resource.size.width as usize * resource.size.height as usize * bytes_per_pixel;
    if resource.pixels.len() != expected_len {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "image resource pixel length mismatch: expected {expected_len}, got {}",
                resource.pixels.len()
            ),
        )
        .into());
    }

    Ok(())
}

fn create_cached_image_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    resource: &ImageResource,
    version: u64,
) -> Result<CachedImageTexture, WgpuBackendError> {
    validate_image_resource(resource)?;

    let format = match resource.format {
        ImageFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("xui image texture"),
        size: wgpu::Extent3d {
            width: resource.size.width,
            height: resource.size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        resource.pixels.as_ref(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(resource.size.width * 4),
            rows_per_image: Some(resource.size.height),
        },
        wgpu::Extent3d {
            width: resource.size.width,
            height: resource.size.height,
            depth_or_array_layers: 1,
        },
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("xui image bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });

    Ok(CachedImageTexture {
        _texture: texture,
        _view: view,
        bind_group,
        version,
    })
}

fn rect_to_array(rect: Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}
