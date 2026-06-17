use etagere::{Allocation, AllocatorOptions, BucketedAtlasAllocator, Size};
use glam::{Vec2, Vec3};
use xui_interface::GlyphBitmap;

use crate::wgpu::AllocInfo;

pub struct Atlas {
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
    pub fn new(device: &wgpu::Device) -> Self {
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

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn handle_allocation(
        &mut self,
        queue: &wgpu::Queue,
        bitmap: &GlyphBitmap,
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

    fn write_glyph_to_texture(&self, queue: &wgpu::Queue, bitmap: &GlyphBitmap, alloc: Allocation) {
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
