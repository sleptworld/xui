mod cache;
mod job;
mod layer;
mod scene;
mod snapshot;
mod utils;
use crate::{
    renders::*,
    wgpu::{
        cache::{BackendDirtyRegion, LayerCacheBook, LayerTileCache},
        scene::SceneTexture,
        snapshot::LayerSnapshot,
        utils::{affine_inverse, choose_srgb_surface_format},
    },
};
pub use cache::LayerCacheStats;
use glam::{Vec2, Vec3};
use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    sync::Arc,
};
use wgpu::util::DeviceExt;
use xui::render::{
    BuiltDraw, BuiltFrame, BuiltItem, BuiltLayerId, CachePolicy, ImagePrimitive, LayerCacheId,
    LayerEffect, RenderBackend, RenderNodeId, Shape, TextPrimitive,
};
use xui::text::TextHost;
use xui_interface::*;
use xui_text_engine::CosmicEngine;

const SHAPE_RECT: f32 = 0.0;
const SHAPE_ROUNDED_RECT: f32 = 1.0;
const SHAPE_LINE: f32 = 2.0;
const COLOR_SOLID: f32 = 0.0;
const COLOR_LINEAR_GRADIENT: f32 = 1.0;
const COLOR_RADIAL_GRADIENT: f32 = 2.0;
const STROKE_CENTER: f32 = 0.0;
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const SCENE_SAMPLE_COUNT: u32 = 4;

pub type WgpuBackendError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone)]
pub struct WgpuBackendOptions {
    pub layer_cache_budget_bytes: u64,
    pub layer_cache_tile_size: u32,
}

impl Default for WgpuBackendOptions {
    fn default() -> Self {
        Self {
            layer_cache_budget_bytes: 128 * 1024 * 1024,
            layer_cache_tile_size: 256,
        }
    }
}

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
    path_render: PathRenderer,
    // Sdfs
    sdf_render: SdfRenderer,
    // Composite
    compositor: Compositor,
    layer_tile_renderer: LayerTileRenderer,
    layer_effect_renderer: LayerEffectRenderer,
    // Common Tools
    ui_uniform_buffer: wgpu::Buffer,
    ui_bind_group_layout: wgpu::BindGroupLayout,
    ui_bind_group: wgpu::BindGroup,
    scene: SceneTexture,
    scene_needs_clear: bool,
    presented_frame: bool,
    options: WgpuBackendOptions,
    layer_cache: LayerCacheBook,
    layer_tiles: LayerTileCache,
    scale_factor: f32,
    _text: PhantomData<fn() -> T>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct UiUniforms {
    viewport_size: [f32; 4],
    scale_factor: [f32; 4],
}

impl<T: TextBackend> WGPUBackend<T> {
    pub fn new(window: Arc<winit::window::Window>) -> Self {
        Self::new_with_options(window, WgpuBackendOptions::default())
    }

    pub fn new_with_options(
        window: Arc<winit::window::Window>,
        options: WgpuBackendOptions,
    ) -> Self {
        pollster::block_on(Self::new_(window, options))
    }

    async fn new_(window: Arc<winit::window::Window>, options: WgpuBackendOptions) -> Self {
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
        let path_render = PathRenderer::new(&device, &ui_bind_group_layout);
        let compositor = Compositor::new(&device, config.format, &scene.view);
        let sdf_render = SdfRenderer::new(&device, &ui_bind_group_layout);
        let layer_tile_renderer = LayerTileRenderer::new(&device, &ui_bind_group_layout);
        let layer_effect_renderer = LayerEffectRenderer::new(&device);

        Self {
            instance,
            adapter,
            surface,
            device,
            queue,
            config,
            image_render,
            path_render,
            compositor,
            layer_tile_renderer,
            layer_effect_renderer,
            sdf_render,
            ui_uniform_buffer,
            ui_bind_group_layout,
            ui_bind_group,
            glyph_render,
            atlas,
            scene,
            scene_needs_clear: true,
            presented_frame: false,
            layer_cache: LayerCacheBook::default(),
            layer_tiles: LayerTileCache::default(),
            options,
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

    pub fn layer_cache_stats(&self) -> LayerCacheStats {
        let mut stats = self.layer_cache.stats();
        stats.resident_bytes = self.layer_tiles.resident_bytes();
        stats.entries = self.layer_tiles.layers.len();
        stats.resident_tiles = self.layer_tiles.tile_count();
        stats
    }
}

#[derive(Debug, Clone, PartialEq)]
struct LayerTileStorageVersion {
    render_bounds: Rect,
    effects: Arc<[LayerEffect]>,
    scale_bits: u32,
    tile_size: u32,
}

struct ResidentTile {
    _textures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    _msaa_texture: wgpu::Texture,
    msaa_view: wgpu::TextureView,
    _ui_uniform_buffer: wgpu::Buffer,
    ui_bind_group: wgpu::BindGroup,
    composite_bind_group: Arc<wgpu::BindGroup>,
    inner_bounds: Rect,
    target_origin: Point,
    target_size: (u32, u32),
    inner_uv: Rect,
    final_index: usize,
    bytes: u64,
    last_used: u64,
    valid: bool,
}

struct ResidentLayer {
    source: RenderNodeId,
    policy: CachePolicy,
    storage: LayerTileStorageVersion,
    tiles: HashMap<(i32, i32), ResidentTile>,
}

fn layer_effect_final_index(effects: &[LayerEffect]) -> usize {
    effects.iter().fold(0, |index, effect| match effect {
        LayerEffect::Blur { .. } => index,
        LayerEffect::DropShadow(_) | LayerEffect::ColorMatrix { .. } | LayerEffect::Mask { .. } => {
            1 - index
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TileJobId {
    cache: LayerCacheId,
    coord: (i32, i32),
}

#[derive(Debug, Clone)]
struct TileRenderJob {
    id: TileJobId,
    layer: BuiltLayerId,
    runs: std::ops::Range<usize>,
}

struct FrameRenderData {
    prepared: PrepareResult,
    tile_jobs: Vec<TileRenderJob>,
    root_runs: std::ops::Range<usize>,
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
            self.layer_cache.clear();
            self.layer_tiles.clear();
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

    fn submit(&mut self, built: &BuiltFrame, text: &mut TextHost<T>) -> Result<(), Self::Error> {
        let _ = (&self.instance, &self.adapter);
        let mut next_layer_cache = self.layer_cache.clone();
        let root_dirty = next_layer_cache.update(
            built,
            self.scale_factor,
            self.options.layer_cache_budget_bytes,
            self.options.layer_cache_tile_size,
        );
        self.presented_frame = false;
        let logical_scene_size = self.logical_scene_size();
        let full_scene = Rect::new(
            0.0,
            0.0,
            logical_scene_size.width,
            logical_scene_size.height,
        );
        let mut scene_clip = if self.scene_needs_clear {
            full_scene
        } else if let Some(bounds) = root_dirty.bounds() {
            let Some(bounds) = intersect_rect(bounds, full_scene) else {
                self.layer_cache = next_layer_cache;
                self.presented_frame = true;
                return Ok(());
            };
            bounds
        } else {
            self.layer_cache = next_layer_cache;
            self.presented_frame = true;
            return Ok(());
        };

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.scene = SceneTexture::new(&self.device, &self.config);
                self.compositor.reset_view(&self.device, &self.scene.view);
                self.scene_needs_clear = true;
                self.layer_tiles.clear();
                scene_clip = full_scene;
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

        let data = self.build_frame_render_data(built, scene_clip, text)?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xui sdf encoder"),
            });

        self.sdf_render
            .deal_instances(&self.device, &self.queue, &data.prepared.sdf_records);
        self.image_render
            .deal_records(&self.device, &self.queue, &data.prepared.image_records)?;
        self.path_render
            .prepare(&self.device, &self.queue, &data.prepared.path_records);
        self.glyph_render
            .deal_glyphs(&self.device, &self.queue, &data.prepared.glyph_records);
        self.layer_tile_renderer
            .prepare(&self.device, &self.queue, &data.prepared.layer_records);

        for job in &data.tile_jobs {
            self.encode_tile_job(&mut encoder, built, &data.prepared, job)?;
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui scene cache render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.scene.msaa_view,
                    resolve_target: Some(&self.scene.view),
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

            self.render_prepared_runs(
                &mut pass,
                &data.prepared,
                data.root_runs.clone(),
                &self.ui_bind_group,
                (self.config.width, self.config.height),
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
        for job in &data.tile_jobs {
            if let Some(tile) = self
                .layer_tiles
                .layers
                .get_mut(&job.id.cache)
                .and_then(|layer| layer.tiles.get_mut(&job.id.coord))
            {
                tile.valid = true;
            }
        }
        self.layer_tiles
            .finish_frame(self.options.layer_cache_budget_bytes);
        self.layer_cache = next_layer_cache;
        self.presented_frame = true;
        Ok(())
    }

    fn set_factor(&mut self, factor: f32) -> Result<(), Self::Error> {
        if self.scale_factor.to_bits() != factor.to_bits() {
            self.layer_cache.clear();
            self.layer_tiles.clear();
        }
        self.scale_factor = factor;
        Ok(())
    }
}

impl<T: TextBackend> WGPUBackend<T> {
    fn build_frame_render_data(
        &mut self,
        frame: &BuiltFrame,
        viewport_clip: Rect,
        text: &mut TextHost<T>,
    ) -> Result<FrameRenderData, WgpuBackendError> {
        self.layer_tiles.begin_frame(frame);
        let mut tile_jobs = Vec::new();
        let mut scheduled = HashSet::new();
        self.collect_layer_dependencies(
            frame,
            frame.root_layer,
            viewport_clip,
            &mut scheduled,
            &mut tile_jobs,
        );

        let mut prepared = PrepareResult::default();
        for job in &mut tile_jobs {
            let tile = &self.layer_tiles.layers[&job.id.cache].tiles[&job.id.coord];
            let origin = tile.target_origin;
            let size = tile.target_size;
            let viewport = Rect::new(
                0.0,
                0.0,
                size.0 as f32 / self.scale_factor,
                size.1 as f32 / self.scale_factor,
            );
            let start = prepared.runs.len();
            self.push_compiled_layer(
                frame,
                job.layer,
                viewport,
                Affine::translate(-origin.x, -origin.y),
                1.0,
                text,
                &mut prepared,
            )?;
            job.runs = start..prepared.runs.len();
        }

        let root_start = prepared.runs.len();
        self.push_compiled_layer(
            frame,
            frame.root_layer,
            viewport_clip,
            Affine::IDENTITY,
            1.0,
            text,
            &mut prepared,
        )?;
        let root_runs = root_start..prepared.runs.len();
        Ok(FrameRenderData {
            prepared,
            tile_jobs,
            root_runs,
        })
    }

    fn collect_layer_dependencies(
        &mut self,
        frame: &BuiltFrame,
        parent: BuiltLayerId,
        parent_region: Rect,
        scheduled: &mut HashSet<TileJobId>,
        jobs: &mut Vec<TileRenderJob>,
    ) {
        for item in &frame.layers[parent.0].items {
            let BuiltItem::Layer(instance) = item else {
                continue;
            };
            let Some(visible) = intersect_rect(parent_region, instance.world_bounds) else {
                continue;
            };
            let child = &frame.layers[instance.layer.0];
            let requested = affine_inverse(instance.composite.transform)
                .map(|inverse| inverse.transform_rect(visible))
                .unwrap_or(child.render_bounds);
            let Some(requested) = intersect_rect(requested, child.render_bounds) else {
                continue;
            };
            let coords = BackendDirtyRegion::full(requested)
                .tiles(self.scale_factor, self.options.layer_cache_tile_size);
            let dirty_coords = self
                .layer_cache
                .last_dirty
                .get(&child.source)
                .map(|dirty| dirty.tiles(self.scale_factor, self.options.layer_cache_tile_size))
                .unwrap_or_default();
            let cache = child
                .cache_id
                .expect("every isolated built layer has a cache identity");
            for coord in coords {
                let created = self.layer_tiles.ensure_tile(
                    &self.device,
                    &self.ui_bind_group_layout,
                    &self.layer_tile_renderer,
                    child,
                    coord,
                    self.scale_factor,
                    self.options.layer_cache_tile_size,
                );
                let Some(tile) = self
                    .layer_tiles
                    .layers
                    .get(&cache)
                    .and_then(|layer| layer.tiles.get(&coord))
                else {
                    continue;
                };
                let needs_render = created || !tile.valid || dirty_coords.contains(&coord);
                let id = TileJobId { cache, coord };
                if !needs_render || !scheduled.insert(id) {
                    continue;
                }
                let target_region = Rect::new(
                    tile.target_origin.x,
                    tile.target_origin.y,
                    tile.target_size.0 as f32 / self.scale_factor,
                    tile.target_size.1 as f32 / self.scale_factor,
                );
                self.collect_layer_dependencies(
                    frame,
                    instance.layer,
                    target_region,
                    scheduled,
                    jobs,
                );
                jobs.push(TileRenderJob {
                    id,
                    layer: instance.layer,
                    runs: 0..0,
                });
            }
        }
    }

    fn render_prepared_runs<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        prepared: &'a PrepareResult,
        runs: std::ops::Range<usize>,
        ui_bind_group: &'a wgpu::BindGroup,
        target_size: (u32, u32),
    ) {
        for run in &prepared.runs[runs] {
            match run {
                PreparedRun::Sdf(range) => self.sdf_render.render_range(
                    pass,
                    ui_bind_group,
                    &prepared.sdf_scissors,
                    range.clone(),
                    self.scale_factor,
                    target_size,
                ),
                PreparedRun::Path(range) => self.path_render.render_range(
                    pass,
                    ui_bind_group,
                    range.clone(),
                    self.scale_factor,
                    target_size,
                ),
                PreparedRun::Image(range) => self.image_render.render_range(
                    pass,
                    ui_bind_group,
                    &prepared.image_records,
                    &prepared.image_scissors,
                    range.clone(),
                    self.scale_factor,
                    target_size,
                ),
                PreparedRun::Glyph(range) => self.glyph_render.render_range(
                    pass,
                    ui_bind_group,
                    range.clone(),
                    self.scale_factor,
                    target_size,
                ),
                PreparedRun::Layer(range) => self.layer_tile_renderer.render_range(
                    pass,
                    ui_bind_group,
                    &prepared.layer_records,
                    range.clone(),
                    self.scale_factor,
                    target_size,
                ),
            }
        }
    }

    fn encode_tile_job(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &BuiltFrame,
        prepared: &PrepareResult,
        job: &TileRenderJob,
    ) -> Result<(), WgpuBackendError> {
        let layer = &frame.layers[job.layer.0];
        let resident = &self.layer_tiles.layers[&job.id.cache].tiles[&job.id.coord];
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui layer tile content pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &resident.msaa_view,
                    resolve_target: Some(&resident.views[0]),
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
            self.render_prepared_runs(
                &mut pass,
                prepared,
                job.runs.clone(),
                &resident.ui_bind_group,
                resident.target_size,
            );
        }

        let mut current = 0usize;
        for effect in layer.effects.iter() {
            match effect {
                LayerEffect::Blur { .. } => {
                    let resident = &self.layer_tiles.layers[&job.id.cache].tiles[&job.id.coord];
                    self.layer_effect_renderer.encode(
                        &self.device,
                        &self.queue,
                        encoder,
                        &resident.views[current],
                        &resident.views[1 - current],
                        effect,
                        Some([1.0, 0.0]),
                        resident.target_size,
                        resident.target_origin,
                        self.scale_factor,
                    )?;
                    let resident = &self.layer_tiles.layers[&job.id.cache].tiles[&job.id.coord];
                    self.layer_effect_renderer.encode(
                        &self.device,
                        &self.queue,
                        encoder,
                        &resident.views[1 - current],
                        &resident.views[current],
                        effect,
                        Some([0.0, 1.0]),
                        resident.target_size,
                        resident.target_origin,
                        self.scale_factor,
                    )?;
                }
                LayerEffect::DropShadow(_)
                | LayerEffect::ColorMatrix { .. }
                | LayerEffect::Mask { .. } => {
                    let resident = &self.layer_tiles.layers[&job.id.cache].tiles[&job.id.coord];
                    self.layer_effect_renderer.encode(
                        &self.device,
                        &self.queue,
                        encoder,
                        &resident.views[current],
                        &resident.views[1 - current],
                        effect,
                        None,
                        resident.target_size,
                        resident.target_origin,
                        self.scale_factor,
                    )?;
                    current = 1 - current;
                }
            }
        }
        debug_assert_eq!(current, resident.final_index);
        Ok(())
    }

    fn push_compiled_layer(
        &mut self,
        frame: &BuiltFrame,
        layer_id: BuiltLayerId,
        viewport_clip: Rect,
        placement_transform: Affine,
        placement_opacity: f32,
        text: &mut TextHost<T>,
        result: &mut PrepareResult,
    ) -> Result<(), WgpuBackendError> {
        for item in &frame.layers[layer_id.0].items {
            match item {
                BuiltItem::Draw(draw) => {
                    let starts = result.record_lengths();
                    let common = draw.common();
                    let chain_clip = common
                        .clip_chain
                        .map(|id| {
                            placement_transform.transform_rect(frame.clip_chains[id.0].world_bounds)
                        })
                        .unwrap_or(viewport_clip);
                    let Some(clip) = intersect_rect(viewport_clip, chain_clip) else {
                        continue;
                    };
                    let world_transform = common.world_transform.then(placement_transform);
                    match draw {
                        BuiltDraw::Shape(value) => {
                            let primitive = &value.primitive;
                            match primitive.shape {
                                Shape::Line { from, to } => {
                                    let Some(stroke) = primitive.stroke else {
                                        continue;
                                    };
                                    let Some(color) = stroke.color.solid_color() else {
                                        continue;
                                    };
                                    result.push_line_instance(
                                        world_transform.transform_point(from),
                                        world_transform.transform_point(to),
                                        alpha_color(color, placement_opacity),
                                        stroke.width,
                                        clip,
                                    );
                                }
                                shape => {
                                    let rect = world_transform.transform_rect(primitive.bounds);
                                    let radius = match shape {
                                        Shape::RoundedRect(radius) => {
                                            radius * world_transform.xx.abs()
                                        }
                                        Shape::Circle | Shape::Ellipse => {
                                            rect.width.min(rect.height) * 0.5
                                        }
                                        Shape::Rect | Shape::Line { .. } => 0.0,
                                    };
                                    result.push_paint_rect_instance(
                                        rect,
                                        radius,
                                        alpha_style(
                                            primitive.fill.unwrap_or_default(),
                                            placement_opacity,
                                        ),
                                        primitive.stroke.map(|mut stroke| {
                                            stroke.color =
                                                alpha_style(stroke.color, placement_opacity);
                                            stroke
                                        }),
                                        primitive.shadow.map(|mut shadow| {
                                            shadow.color =
                                                alpha_color(shadow.color, placement_opacity);
                                            shadow
                                        }),
                                        clip,
                                    );
                                }
                            }
                        }
                        BuiltDraw::Path(value) => result.path_records.push(PathDrawRecord {
                            path: value.primitive.path.clone(),
                            transform: value.primitive.transform.then(world_transform),
                            fill: value.primitive.fill.map(|mut fill| {
                                fill.color = alpha_color(fill.color, placement_opacity);
                                fill
                            }),
                            stroke: value.primitive.stroke.map(|mut stroke| {
                                stroke.color = alpha_color(stroke.color, placement_opacity);
                                stroke
                            }),
                            clip,
                        }),
                        BuiltDraw::Image(value) => result.push_image_record(
                            &ImagePrimitive {
                                opacity: value.primitive.opacity * placement_opacity,
                                ..value.primitive.clone()
                            },
                            world_transform.transform_rect(value.primitive.bounds),
                            clip,
                        ),
                        BuiltDraw::Text(value) => {
                            let rect = world_transform.transform_rect(value.primitive.bounds);
                            let Some(clip) = intersect_rect(clip, rect) else {
                                continue;
                            };
                            self.push_text_glyph_records(
                                &TextPrimitive {
                                    paint: alpha_text_paint(
                                        value.primitive.paint,
                                        placement_opacity,
                                    ),
                                    ..value.primitive.clone()
                                },
                                rect,
                                clip,
                                text,
                                result,
                            )?;
                        }
                    }
                    result.finish_draw_run(draw, starts);
                }
                BuiltItem::Layer(instance) => {
                    let instance_clip = instance
                        .clip_chain
                        .map(|id| {
                            placement_transform.transform_rect(frame.clip_chains[id.0].world_bounds)
                        })
                        .unwrap_or(viewport_clip);
                    let Some(instance_clip) = intersect_rect(viewport_clip, instance_clip) else {
                        continue;
                    };
                    let layer = &frame.layers[instance.layer.0];
                    let cache = layer
                        .cache_id
                        .expect("every isolated built layer has a cache identity");
                    let Some(resident) = self.layer_tiles.layers.get(&cache) else {
                        continue;
                    };
                    let transform = instance.composite.transform.then(placement_transform);
                    let opacity = placement_opacity * instance.composite.opacity.clamp(0.0, 1.0);
                    for tile in resident.tiles.values() {
                        let placed = transform.transform_rect(tile.inner_bounds);
                        if !placed.intersects(instance_clip) {
                            continue;
                        }
                        result.push_layer_record(self.layer_tile_renderer.record(
                            Arc::clone(&tile.composite_bind_group),
                            tile.inner_bounds,
                            tile.inner_uv,
                            transform,
                            opacity,
                            instance_clip,
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn push_text_glyph_records(
        &mut self,
        command: &TextPrimitive,
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

fn alpha_color(mut color: Color, opacity: f32) -> Color {
    color.a *= opacity.clamp(0.0, 1.0);
    color
}

fn alpha_style(style: ComputedColorStyle, opacity: f32) -> ComputedColorStyle {
    match style {
        ComputedColorStyle::Solid(color) => ComputedColorStyle::Solid(alpha_color(color, opacity)),
        ComputedColorStyle::LinearGradient(mut gradient) => {
            gradient.from = alpha_color(gradient.from, opacity);
            gradient.to = alpha_color(gradient.to, opacity);
            ComputedColorStyle::LinearGradient(gradient)
        }
        ComputedColorStyle::RadialGradient(mut gradient) => {
            gradient.from = alpha_color(gradient.from, opacity);
            gradient.to = alpha_color(gradient.to, opacity);
            ComputedColorStyle::RadialGradient(gradient)
        }
    }
}

fn alpha_text_paint(mut paint: TextPaintProps, opacity: f32) -> TextPaintProps {
    paint.style.color = alpha_color(paint.style.color, opacity);
    if let Some(caret) = &mut paint.caret {
        caret.color = alpha_color(caret.color, opacity);
    }
    if let Some(selection) = &mut paint.selection {
        selection.color = alpha_color(selection.color, opacity);
    }
    if let Some(ime) = &mut paint.ime {
        ime.underline_color = alpha_color(ime.underline_color, opacity);
    }
    paint
}

fn push_text_caret(
    command: &TextPrimitive,
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
    pub path_records: Vec<PathDrawRecord>,
    pub glyph_records: Vec<TextGlyphRecord>,
    pub layer_records: Vec<LayerTileDrawRecord>,
    runs: Vec<PreparedRun>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreparedRun {
    Sdf(std::ops::Range<usize>),
    Path(std::ops::Range<usize>),
    Image(std::ops::Range<usize>),
    Glyph(std::ops::Range<usize>),
    Layer(std::ops::Range<usize>),
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
    fn record_lengths(&self) -> [usize; 4] {
        [
            self.sdf_records.len(),
            self.path_records.len(),
            self.image_records.len(),
            self.glyph_records.len(),
        ]
    }

    fn push_layer_record(&mut self, record: LayerTileDrawRecord) {
        let start = self.layer_records.len();
        self.layer_records.push(record);
        self.push_run(PreparedRun::Layer(start..start + 1));
    }

    fn finish_draw_run(&mut self, draw: &BuiltDraw, start: [usize; 4]) {
        let end = self.record_lengths();
        match draw {
            BuiltDraw::Shape(_) => self.push_run(PreparedRun::Sdf(start[0]..end[0])),
            BuiltDraw::Path(_) => self.push_run(PreparedRun::Path(start[1]..end[1])),
            BuiltDraw::Image(_) => self.push_run(PreparedRun::Image(start[2]..end[2])),
            BuiltDraw::Text(_) => {
                self.push_run(PreparedRun::Sdf(start[0]..end[0]));
                self.push_run(PreparedRun::Glyph(start[3]..end[3]));
            }
        }
    }

    fn push_run(&mut self, run: PreparedRun) {
        let range = match &run {
            PreparedRun::Sdf(range)
            | PreparedRun::Path(range)
            | PreparedRun::Image(range)
            | PreparedRun::Glyph(range)
            | PreparedRun::Layer(range) => range,
        };
        if range.is_empty() {
            return;
        }
        let merged = match (self.runs.last_mut(), &run) {
            (Some(PreparedRun::Sdf(previous)), PreparedRun::Sdf(next))
            | (Some(PreparedRun::Path(previous)), PreparedRun::Path(next))
            | (Some(PreparedRun::Image(previous)), PreparedRun::Image(next))
            | (Some(PreparedRun::Glyph(previous)), PreparedRun::Glyph(next))
            | (Some(PreparedRun::Layer(previous)), PreparedRun::Layer(next))
                if previous.end == next.start =>
            {
                previous.end = next.end;
                true
            }
            _ => false,
        };
        if !merged {
            self.runs.push(run);
        }
    }

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

    fn push_image_record(&mut self, command: &ImagePrimitive, rect: Rect, clip: Rect) {
        if rect.width <= 0.0
            || rect.height <= 0.0
            || clip.width <= 0.0
            || clip.height <= 0.0
            || command.opacity <= 0.0
            || command.image == ImageKey::default()
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
            key: command.image.clone(),
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
