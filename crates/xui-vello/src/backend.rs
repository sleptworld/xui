use std::sync::Arc;

use vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene, AaSupport, peniko::color::AlphaColor};
use xui_core::render::{BuiltFrame, RenderBackend};
use xui_core::text::TextHost;
use xui_interface::{Color, NodeLifecycleEvent, Size, TextBackend};
use xui_shell::SurfaceTarget;

use crate::{
    VelloBackendError, WgpuContext,
    encode::{Unsupported, encode_frame},
};

/// Vello renders into this; the blit pass reads it.
const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelloBackendOptions {
    pub clear_color: Color,
}

impl Default for VelloBackendOptions {
    fn default() -> Self {
        Self {
            clear_color: Color::rgba(0.08, 0.09, 0.11, 1.0),
        }
    }
}

struct SceneTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

pub struct VelloBackend<T: TextBackend> {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    context: WgpuContext,
    renderer: Renderer,
    blit_pipeline: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    target: Option<SceneTarget>,
    scene: Scene,
    options: VelloBackendOptions,
    scale_factor: f32,
    reported: Unsupported,
    presented: bool,
    _text: std::marker::PhantomData<fn() -> T>,
}

impl<T: TextBackend> VelloBackend<T> {
    pub fn new(window: Arc<dyn SurfaceTarget>) -> Result<Self, VelloBackendError> {
        Self::new_with_options(window, VelloBackendOptions::default())
    }

    pub fn new_with_options(
        window: Arc<dyn SurfaceTarget>,
        options: VelloBackendOptions,
    ) -> Result<Self, VelloBackendError> {
        pollster::block_on(Self::new_async(window, options))
    }

    async fn new_async(
        window: Arc<dyn SurfaceTarget>,
        options: VelloBackendOptions,
    ) -> Result<Self, VelloBackendError> {
        let size = window.surface_size();
        let scale_factor = window.scale_factor() as f32;
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await?;
        let features = adapter.get_texture_format_features(SCENE_FORMAT);
        if !features
            .allowed_usages
            .contains(wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(VelloBackendError::UnsupportedVectorTarget);
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("xui-vello device"),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await?;

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or(VelloBackendError::UnsupportedSurface)?;
        // Vello's output is already sRGB-encoded, so present through a
        // non-sRGB format and let the blit copy bytes through unchanged.
        let formats = surface.get_capabilities(&adapter).formats;
        let linear = config.format.remove_srgb_suffix();
        config.format = if formats.contains(&linear) {
            linear
        } else {
            formats
                .iter()
                .copied()
                .find(|format| !format.is_srgb())
                .unwrap_or(config.format)
        };
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        surface.configure(&device, &config);

        let renderer = Renderer::new(
            &device,
            RendererOptions {
                antialiasing_support: AaSupport::area_only(),
                ..RendererOptions::default()
            },
        )?;
        let (blit_pipeline, blit_layout) = create_blit_pipeline(&device, config.format);

        Ok(Self {
            surface,
            config,
            context: WgpuContext { device, queue },
            renderer,
            blit_pipeline,
            blit_layout,
            target: None,
            scene: Scene::new(),
            options,
            scale_factor,
            reported: Unsupported::default(),
            presented: false,
            _text: std::marker::PhantomData,
        })
    }

    /// The device and queue this backend renders on. A canvas GPU painter must
    /// create its textures here.
    pub fn wgpu_context(&self) -> WgpuContext {
        self.context.clone()
    }

    fn ensure_target(&mut self) {
        let size = (self.config.width, self.config.height);
        if self.target.as_ref().is_some_and(|target| target.size == size) {
            return;
        }
        let device = &self.context.device;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xui-vello scene"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui-vello blit"),
            layout: &self.blit_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        self.target = Some(SceneTarget {
            _texture: texture,
            view,
            bind_group,
            size,
        });
    }

    fn acquire(&mut self) -> Result<Option<wgpu::SurfaceTexture>, VelloBackendError> {
        for attempt in 0..2 {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => return Ok(Some(frame)),
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost
                    if attempt == 0 =>
                {
                    self.surface.configure(&self.context.device, &self.config);
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(None);
                }
                _ => return Err(VelloBackendError::AcquireSurface),
            }
        }
        Err(VelloBackendError::AcquireSurface)
    }

    fn report(&mut self, found: Unsupported) {
        macro_rules! once {
            ($($field:ident),*) => {$(
                if found.$field && !self.reported.$field {
                    log::warn!("xui-vello: `{}` is not drawn yet", stringify!($field));
                }
            )*};
        }
        once!(text, image, nested_layer, gradient, shadow, texture, path_clip);
        self.reported.merge(found);
    }
}

impl<T: TextBackend> RenderBackend<TextHost<T>> for VelloBackend<T> {
    type Error = VelloBackendError;

    fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Self::Error> {
        self.presented = false;
        let width = (size.width * self.scale_factor).round().max(1.0) as u32;
        let height = (size.height * self.scale_factor).round().max(1.0) as u32;
        if (width, height) != (self.config.width, self.config.height) {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.context.device, &self.config);
        }
        self.ensure_target();
        Ok(())
    }

    fn submit(&mut self, frame: &BuiltFrame, _text: &mut TextHost<T>) -> Result<(), Self::Error> {
        self.scene.reset();
        let found = encode_frame(frame, self.scale_factor, &mut self.scene);
        self.report(found);

        let target = self.target.as_ref().expect("begin_frame ensures a target");
        let clear = self.options.clear_color;
        self.renderer.render_to_texture(
            &self.context.device,
            &self.context.queue,
            &self.scene,
            &target.view,
            &RenderParams {
                base_color: AlphaColor::new([clear.r, clear.g, clear.b, clear.a]),
                width: target.size.0,
                height: target.size.1,
                antialiasing_method: AaConfig::Area,
            },
        )?;
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        let Some(frame) = self.acquire()? else {
            return Ok(());
        };
        let target = self.target.as_ref().expect("begin_frame ensures a target");
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xui-vello blit"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui-vello blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &target.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.context.queue.submit([encoder.finish()]);
        frame.present();
        self.presented = true;
        Ok(())
    }

    fn did_present(&self) -> bool {
        self.presented
    }

    fn resize(&mut self, _size: Size<f32>) -> Result<(), Self::Error> {
        // `begin_frame` reconfigures from the size it is given.
        Ok(())
    }

    fn set_factor(&mut self, factor: f32) -> Result<(), Self::Error> {
        self.scale_factor = factor;
        Ok(())
    }

    fn handle_node_lifecycle(&mut self, _event: &NodeLifecycleEvent) {}
}

fn create_blit_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("xui-vello blit layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("xui-vello blit"),
        source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("xui-vello blit"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("xui-vello blit"),
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
            targets: &[Some(format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, layout)
}

#[cfg(test)]
mod tests {
    #[test]
    fn blit_shader_parses_as_wgsl() {
        naga::front::wgsl::parse_str(include_str!("blit.wgsl")).expect("blit.wgsl");
    }
}
