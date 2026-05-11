use std::sync::Arc;

use etagere::{Allocation, AllocatorOptions};
use etagere::{BucketedAtlasAllocator, Size};
use glam::{Vec2, Vec3};
use xui_interface::RenderBackend;
use xui_text::atlas::FontRenderBackend;
use xui_text::atlas::RendedGlyphBitmap;

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
    atlas: Atlas,
}

impl WGPUBackend {
    pub fn new(window: Arc<winit::window::Window>) -> Self {
        pollster::block_on(Self::new_(window))
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

        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui sdf shader"),
            source: wgpu::ShaderSource::Wgsl(UI_SHADER_WGSL.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui sdf pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui sdf render pipeline"),
            layout: Some(&pipeline_layout),

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

        Self {
            instance,
            adapter,
            surface,
            device,
            queue,
            config,
            render_pipeline,
            atlas,
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

impl RenderBackend for WGPUBackend {
    type Error = WgpuBackendError;

    fn begin_frame(&mut self, size: xui_interface::Size) -> Result<(), Self::Error> {
        let width = size.width.max(1.0) as u32;
        let height = size.height.max(1.0) as u32;
        if self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn paint(
        &mut self,
        commands: &[xui_interface::PaintCommand],
        damage: &xui_interface::DamageRegion,
    ) -> Result<(), Self::Error> {
        let _ = (commands, damage, &self.instance, &self.adapter);

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
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

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xui sdf encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui sdf render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.08,
                            g: 0.09,
                            b: 0.11,
                            a: 1.0,
                        }),
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
            pass.draw(0..3, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

pub struct AllocInfo {
    pub total_size: Vec3,
    pub layer: u32,
    pub origin: Vec2,
}

impl FontRenderBackend for WGPUBackend {
    type Error = crate::error::Error;
    type Allocation = AllocInfo;

    fn write_bitmap(
        &mut self,
        bitmap: &xui_text::atlas::RendedGlyphBitmap,
    ) -> Result<Self::Allocation, Self::Error> {
        return self.atlas.handle_allocation(&self.queue, bitmap);
    }
}
