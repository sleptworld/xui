use crate::wgpu::{SCENE_FORMAT, WgpuBackendError};
use moka::sync::Cache;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use xui_interface::{
    ImageData, ImageDataId, ImageFormat, ImageKey, ImageRepeat, ImageRotation, ImageVariant, Rect,
    Sampling,
};

const IMAGE_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x4,
    2 => Float32x4,
    3 => Float32x4,
    4 => Float32x4,
];

const DEFAULT_IMAGE_TEXTURE_CACHE_CAPACITY: u64 = 256;

pub struct ImageRender {
    image_pipeline: wgpu::RenderPipeline,
    image_bind_group_layout: wgpu::BindGroupLayout,
    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,
    image_textures: Cache<ImageKey, Arc<CachedImageTexture>>,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ImageInstance {
    bounds: [f32; 4],
    clip: [f32; 4],
    params: [f32; 4],
    tile: [f32; 4],
    repeat: [f32; 4],
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

struct CachedImageTexture {
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    linear_bind_group: wgpu::BindGroup,
    nearest_bind_group: wgpu::BindGroup,
    data_id: ImageDataId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageDrawRecord {
    pub key: ImageKey,
    pub data: ImageData,
    pub rect: Rect,
    pub clip: Rect,
    pub tile: Rect,
    pub repeat: ImageRepeat,
    pub opacity: f32,
    pub variant: ImageVariant,
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
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xui linear image sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xui nearest image sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
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
            linear_sampler,
            nearest_sampler,
            image_textures: Cache::new(DEFAULT_IMAGE_TEXTURE_CACHE_CAPACITY),
            instance_buffer,
            instance_capacity: 0,
        }
    }

    pub fn deal_records(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        records: &[ImageDrawRecord],
    ) -> Result<(), WgpuBackendError> {
        for record in records {
            let cached = self.image_textures.get(&record.key);
            if cached
                .as_ref()
                .is_some_and(|texture| texture.data_id == record.data.id())
            {
                continue;
            }

            let texture = Arc::new(create_cached_image_texture(
                device,
                queue,
                &self.image_bind_group_layout,
                &self.linear_sampler,
                &self.nearest_sampler,
                &record.data,
            )?);
            self.image_textures.insert(record.key.clone(), texture);
        }

        let instances: Vec<ImageInstance> = records
            .iter()
            .map(|record| ImageInstance {
                bounds: rect_to_array(record.rect),
                clip: rect_to_array(record.clip),
                params: image_params(record),
                tile: rect_to_array(record.tile),
                repeat: image_repeat(record.repeat),
            })
            .collect();

        if instances.is_empty() {
            return Ok(());
        }
        if instances.len() > self.instance_capacity {
            self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("xui image instances"),
                contents: bytemuck::cast_slice(&instances),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            self.instance_capacity = instances.len();
        } else {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }

        Ok(())
    }

    pub fn render(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        tool_bind_group: &wgpu::BindGroup,
        records: &[ImageDrawRecord],
        scissors: &[Rect],
        scale_factor: f32,
        target_size: (u32, u32),
    ) {
        if records.is_empty() {
            return;
        }

        pass.set_pipeline(&self.image_pipeline);
        pass.set_bind_group(0, tool_bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        for (index, record) in records.iter().enumerate() {
            let Some(texture) = self.image_textures.get(&record.key) else {
                continue;
            };
            let bind_group = match record.variant.sampling {
                Sampling::Nearest => &texture.nearest_bind_group,
                Sampling::Linear | Sampling::Cubic => &texture.linear_bind_group,
            };
            pass.set_bind_group(1, bind_group, &[]);
            let Some((x, y, width, height)) =
                crate::wgpu::physical_scissor(scissors[index], scale_factor, target_size)
            else {
                continue;
            };
            pass.set_scissor_rect(x, y, width, height);
            pass.draw(0..6, index as u32..index as u32 + 1);
        }
    }
}

fn validate_image_data(data: &ImageData) -> Result<(), WgpuBackendError> {
    if data.size.width == 0 || data.size.height == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "image resource size cannot be zero",
        )
        .into());
    }

    let bytes_per_pixel = match data.format {
        ImageFormat::Rgba8UnormSrgb => 4usize,
    };
    let expected_len = data.size.width as usize * data.size.height as usize * bytes_per_pixel;
    if data.pixels.len() != expected_len {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "image resource pixel length mismatch: expected {expected_len}, got {}",
                data.pixels.len()
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
    linear_sampler: &wgpu::Sampler,
    nearest_sampler: &wgpu::Sampler,
    data: &ImageData,
) -> Result<CachedImageTexture, WgpuBackendError> {
    validate_image_data(data)?;

    let format = match data.format {
        ImageFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("xui image texture"),
        size: wgpu::Extent3d {
            width: data.size.width,
            height: data.size.height,
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
        data.pixels.as_ref(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(data.size.width * 4),
            rows_per_image: Some(data.size.height),
        },
        wgpu::Extent3d {
            width: data.size.width,
            height: data.size.height,
            depth_or_array_layers: 1,
        },
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let create_bind_group = |label, sampler| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
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
        })
    };
    let linear_bind_group = create_bind_group("xui linear image bind group", linear_sampler);
    let nearest_bind_group = create_bind_group("xui nearest image bind group", nearest_sampler);

    Ok(CachedImageTexture {
        _texture: texture,
        _view: view,
        linear_bind_group,
        nearest_bind_group,
        data_id: data.id(),
    })
}

fn image_params(record: &ImageDrawRecord) -> [f32; 4] {
    let transform = record.variant.transform;
    let rotation = match transform.rotate {
        ImageRotation::Deg0 => 0.0,
        ImageRotation::Deg90 => 1.0,
        ImageRotation::Deg180 => 2.0,
        ImageRotation::Deg270 => 3.0,
    };
    [
        record.opacity.clamp(0.0, 1.0),
        if transform.flip_x { 1.0 } else { 0.0 },
        if transform.flip_y { 1.0 } else { 0.0 },
        rotation,
    ]
}

fn image_repeat(repeat: ImageRepeat) -> [f32; 4] {
    [
        if matches!(repeat, ImageRepeat::Repeat | ImageRepeat::RepeatX) {
            1.0
        } else {
            0.0
        },
        if matches!(repeat, ImageRepeat::Repeat | ImageRepeat::RepeatY) {
            1.0
        } else {
            0.0
        },
        0.0,
        0.0,
    ]
}

fn rect_to_array(rect: Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{ImageTransform, Size};

    fn record(variant: ImageVariant, opacity: f32) -> ImageDrawRecord {
        ImageDrawRecord {
            key: ImageKey::UserProvided(1),
            data: ImageData::rgba8(Size::new(1, 1), [255, 0, 0, 255]),
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            clip: Rect::new(0.0, 0.0, 10.0, 10.0),
            tile: Rect::new(0.0, 0.0, 10.0, 10.0),
            repeat: ImageRepeat::NoRepeat,
            opacity,
            variant,
        }
    }

    #[test]
    fn image_params_encode_opacity_flip_and_rotation() {
        let variant = ImageVariant {
            transform: ImageTransform {
                flip_x: true,
                flip_y: false,
                rotate: ImageRotation::Deg270,
            },
            ..ImageVariant::default()
        };

        assert_eq!(image_params(&record(variant, 1.5)), [1.0, 1.0, 0.0, 3.0]);
    }

    #[test]
    fn image_repeat_encodes_repeated_axes() {
        assert_eq!(image_repeat(ImageRepeat::NoRepeat), [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(image_repeat(ImageRepeat::Repeat), [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(image_repeat(ImageRepeat::RepeatX), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(image_repeat(ImageRepeat::RepeatY), [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn image_data_validation_rejects_invalid_dimensions_and_pixel_length() {
        let valid = ImageData::rgba8(Size::new(1, 2), [0; 8]);
        assert!(validate_image_data(&valid).is_ok());

        let empty = ImageData::rgba8(Size::new(0, 1), []);
        assert!(validate_image_data(&empty).is_err());

        let truncated = ImageData::rgba8(Size::new(2, 2), [0; 4]);
        assert!(validate_image_data(&truncated).is_err());
    }
}
