use std::{collections::HashMap, sync::Arc};

use xui::{ImageFormat, ImageKey, ImageResource, Rect, Size};

use crate::wgpu::{SCENE_FORMAT, WgpuBackendError};

const IMAGE_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x4,
    2 => Float32x4,
];

pub struct ImageRender {
    image_pipeline: wgpu::RenderPipeline,
    image_bind_group_layout: wgpu::BindGroupLayout,
    image_sampler: wgpu::Sampler,
    image_resources: HashMap<ImageKey, RegisteredImageResource>,
    image_textures: HashMap<ImageKey, CachedImageTexture>,
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

        Self {
            image_pipeline,
            image_bind_group_layout,
            image_sampler,
            image_resources: HashMap::new(),
            image_textures: HashMap::new(),
        }
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
        self.image_textures.remove(key);
    }

    pub fn clear_image_resources(&mut self) {
        self.image_resources.clear();
        self.image_textures.clear();
    }
}

fn validate_image_resource(resource: &ImageResource) -> Result<(), WgpuBackendError> {
    if resource.key.0.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "image resource key cannot be empty",
        )
        .into());
    }
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

struct GpuImage {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}
