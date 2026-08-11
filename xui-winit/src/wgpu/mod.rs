mod cache;
mod job;
mod layer;
mod snapshot;
mod surface;
mod tex;
pub mod texture_pool;
mod utils;
use crate::{
    renders::*,
    wgpu::{
        cache::{LayerCacheBook, SurfaceCache},
        snapshot::LayerSnapshot,
        surface::{CompositePrefixPlan, CompositePrefixStep},
        tex::{
            FrameTarget, SharedTileTarget, SurfaceKey, TemporarySurface, TileCoord,
            intersect_pixel_rect, logical_rect,
        },
        utils::choose_srgb_surface_format,
    },
};
pub use cache::LayerCacheStats;
use glam::{Vec2, Vec3};
use std::{collections::HashSet, marker::PhantomData, sync::Arc};
pub use texture_pool::{
    TextureLease, TexturePool, TexturePoolError, TexturePoolOptions, TexturePoolStats,
    TextureRequest,
};
use wgpu::util::DeviceExt;
use xui::render::{
    BuiltDraw, BuiltFrame, BuiltItem, BuiltLayerId, ImagePrimitive, RenderBackend, Shape,
    TextPrimitive,
};
use xui::text::TextHost;
use xui_interface::*;
use xui_render_graph::{
    ExternalAliasing, LayerPlanContext, LayerRenderPlan, PlanLimits, TextureClass,
};
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

#[derive(Debug, thiserror::Error)]
pub enum WgpuBackendInitError {
    #[error("failed to create wgpu surface")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("failed to find a compatible wgpu adapter")]
    RequestAdapter(#[from] wgpu::RequestAdapterError),
    #[error("failed to create wgpu device")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error("surface is not supported by the selected wgpu adapter")]
    UnsupportedSurface,
    #[error("surface does not support an sRGB format")]
    UnsupportedSrgbSurface,
    #[error("adapter cannot use Rgba8Unorm as a sampled Vello storage target")]
    UnsupportedVectorTarget,
    #[error("failed to initialize Vello")]
    Vello(#[from] vello::Error),
    #[error("failed to allocate shared tile target")]
    TexturePool(#[from] TexturePoolError),
}

#[derive(Debug, Clone)]
pub struct WgpuBackendOptions {
    pub layer_cache_budget_bytes: u64,
    pub layer_cache_tile_size: u32,
    /// Number of physical tile rings retained around each visible surface demand.
    pub surface_viewport_guard_tiles: u32,
    pub texture_pool: TexturePoolOptions,
}

impl Default for WgpuBackendOptions {
    fn default() -> Self {
        Self {
            layer_cache_budget_bytes: 128 * 1024 * 1024,
            layer_cache_tile_size: 256,
            surface_viewport_guard_tiles: 1,
            texture_pool: TexturePoolOptions::default(),
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
    vector_render: VectorRenderer,
    // Sdfs
    sdf_render: SdfRenderer,
    // Composite
    compositor: Compositor,
    texture_pool: TexturePool,
    render_graph_renderer: RenderGraphRenderer,
    // Common Tools
    shared_tile_ui_uniform_buffer: wgpu::Buffer,
    shared_tile_ui_bind_group: wgpu::BindGroup,
    root_needs_clear: bool,
    presented_frame: bool,
    options: WgpuBackendOptions,
    layer_cache: LayerCacheBook,
    surfaces: SurfaceCache,
    shared_tile_target: SharedTileTarget,
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
    pub fn new(window: Arc<winit::window::Window>) -> Result<Self, WgpuBackendInitError> {
        Self::new_with_options(window, WgpuBackendOptions::default())
    }

    pub fn new_with_options(
        window: Arc<winit::window::Window>,
        options: WgpuBackendOptions,
    ) -> Result<Self, WgpuBackendInitError> {
        pollster::block_on(Self::new_(window, options))
    }

    async fn new_(
        window: Arc<winit::window::Window>,
        options: WgpuBackendOptions,
    ) -> Result<Self, WgpuBackendInitError> {
        let size = window.inner_size();
        let scale_factor = window.scale_factor();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(Arc::clone(&window))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await?;
        let vector_features = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba8Unorm);
        let required_vector_usages =
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        if !vector_features
            .allowed_usages
            .contains(required_vector_usages)
        {
            return Err(WgpuBackendInitError::UnsupportedVectorTarget);
        }

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await?;

        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .ok_or(WgpuBackendInitError::UnsupportedSurface)?;
        let surface_capabilities = surface.get_capabilities(&adapter);
        config.format = choose_srgb_surface_format(config.format, &surface_capabilities.formats)
            .ok_or(WgpuBackendInitError::UnsupportedSrgbSurface)?;

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

        let atlas = Atlas::new(&device);
        let glyph_render = GlyphRender::new(&device, &atlas, &ui_bind_group_layout);
        let image_render = ImageRender::new(&device, &ui_bind_group_layout);
        let vector_render = VectorRenderer::new(&device, &ui_bind_group_layout)?;
        let compositor = Compositor::new(&device, config.format);
        let sdf_render = SdfRenderer::new(&device, &ui_bind_group_layout);
        let texture_pool = TexturePool::new(&device, options.texture_pool);
        let render_graph_renderer = RenderGraphRenderer::new(&device, texture_pool.clone());
        let shared_tile_target =
            SharedTileTarget::new(&device, &texture_pool, options.layer_cache_tile_size)?;
        let shared_tile_ui_uniform_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("xui shared tile ui uniforms"),
                contents: bytemuck::bytes_of(&UiUniforms {
                    viewport_size: [
                        shared_tile_target.extent.0 as f32,
                        shared_tile_target.extent.1 as f32,
                        0.0,
                        0.0,
                    ],
                    scale_factor: [scale_factor as f32; 4],
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let shared_tile_ui_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui shared tile ui bind group"),
            layout: &ui_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shared_tile_ui_uniform_buffer.as_entire_binding(),
            }],
        });

        Ok(Self {
            instance,
            adapter,
            surface,
            device,
            queue,
            config,
            image_render,
            vector_render,
            compositor,
            texture_pool,
            render_graph_renderer,
            sdf_render,
            shared_tile_ui_uniform_buffer,
            shared_tile_ui_bind_group,
            glyph_render,
            atlas,
            root_needs_clear: true,
            presented_frame: false,
            layer_cache: LayerCacheBook::default(),
            surfaces: SurfaceCache::default(),
            shared_tile_target,
            options,
            scale_factor: scale_factor as f32,
            _text: PhantomData,
        })
    }

    fn logical_scene_size(&self) -> xui_interface::Size<f32> {
        xui_interface::Size::<f32>::new(
            self.config.width as f32 / self.scale_factor,
            self.config.height as f32 / self.scale_factor,
        )
    }

    pub fn layer_cache_stats(&self) -> LayerCacheStats {
        let mut stats = self.layer_cache.stats();
        stats.resident_bytes = self.surfaces.resident_bytes();
        stats.entries = self.surfaces.surfaces.len();
        stats.resident_tiles = self.surfaces.tile_count();
        stats
    }

    pub fn texture_pool_stats(&self) -> TexturePoolStats {
        self.texture_pool.stats()
    }

    /// Acquires a pool-backed texture. Holding the returned lease across frames
    /// pins the allocation, which is suitable for transition snapshots.
    pub fn acquire_pooled_texture(
        &self,
        request: TextureRequest,
    ) -> Result<TextureLease, TexturePoolError> {
        self.texture_pool.acquire(&self.device, request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TileJobId {
    surface: SurfaceKey,
    coord: TileCoord,
}

#[derive(Debug, Clone)]
struct TileRenderJob {
    id: TileJobId,
    layer: BuiltLayerId,
    runs: std::ops::Range<usize>,
}

#[derive(Debug, Clone)]
struct LayerGraphDrawRecord {
    parent_surface: SurfaceKey,
    parent_coord: TileCoord,
    surface: SurfaceKey,
    coords: Box<[TileCoord]>,
    program: xui::render::render_graph::BuiltLayerProgram,
    plan: LayerRenderPlan,
    prefix_plan: Option<CompositePrefixPlan>,
}

#[derive(Debug, Clone, Copy)]
struct PrefixStageDemand {
    prefix: xui::render::SurfacePrefix,
    demand: Rect,
    placement: Option<xui::render::BuiltLayerInstanceId>,
}

enum MaterializedBackdrop {
    Shared {
        physical_bounds: xui_render_graph::PixelRect,
    },
    Temporary(TemporarySurface),
}

struct FrameRenderData {
    prepared: PrepareResult,
    tile_jobs: Vec<TileRenderJob>,
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
            self.root_needs_clear = true;
            self.layer_cache.clear();
            self.surfaces.clear();
        }
        self.queue.write_buffer(
            &self.shared_tile_ui_uniform_buffer,
            0,
            bytemuck::bytes_of(&UiUniforms {
                viewport_size: [
                    self.shared_tile_target.extent.0 as f32,
                    self.shared_tile_target.extent.1 as f32,
                    0.0,
                    0.0,
                ],
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
        let logical_scene_size = self.logical_scene_size();
        let root_dirty = self.layer_cache.update(
            built,
            self.scale_factor,
            self.options.layer_cache_budget_bytes,
            self.options.layer_cache_tile_size,
        );

        self.presented_frame = false;
        let full_scene = Rect::new(
            0.0,
            0.0,
            logical_scene_size.width,
            logical_scene_size.height,
        );
        let mut scene_clip = if self.root_needs_clear {
            full_scene
        } else if let Some(bounds) = root_dirty.bounds() {
            let Some(bounds) = intersect_rect(bounds, full_scene) else {
                self.presented_frame = true;
                return Ok(());
            };
            bounds
        } else {
            self.presented_frame = true;
            return Ok(());
        };

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.root_needs_clear = true;
                self.surfaces.clear();
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

        self.vector_render.begin_frame();
        let mut data = self.build_frame_render_data(built, scene_clip, text)?;
        self.prepare_vector_runs(&mut data.prepared)?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xui sdf encoder"),
            });

        self.sdf_render
            .deal_instances(&self.device, &self.queue, &data.prepared.sdf_records);
        self.image_render
            .deal_records(&self.device, &self.queue, &data.prepared.image_records)?;
        self.vector_render.prepare(&self.device, &self.queue);
        self.glyph_render
            .deal_glyphs(&self.device, &self.queue, &data.prepared.glyph_records);
        self.encode_tile_jobs(&mut encoder, built, &data.prepared, &data.tile_jobs)?;

        self.root_needs_clear = false;
        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let frame_target = FrameTarget {
            size: (self.config.width, self.config.height),
            view: &frame_view,
        };
        let root = &self.surfaces.surfaces[&SurfaceKey::Root];
        let mut root_coords: Vec<_> = root.tiles.keys().copied().collect();
        root_coords.sort_unstable();
        let root_tiles: Vec<_> = root_coords
            .iter()
            .filter_map(|coord| root.tiles.get(coord).map(|tile| (*coord, tile)))
            .filter(|(coord, tile)| {
                tile.valid
                    || data.tile_jobs.iter().any(|job| {
                        job.id
                            == TileJobId {
                                surface: SurfaceKey::Root,
                                coord: *coord,
                            }
                    })
            })
            .map(|(_, tile)| {
                let allocation = tile.texture.allocation_extent();
                CompositeTile {
                    view: tile.texture.view(),
                    origin: (tile.physical_bounds.x, tile.physical_bounds.y),
                    valid_extent: (tile.physical_bounds.width, tile.physical_bounds.height),
                    allocation_extent: (allocation.width, allocation.height),
                }
            })
            .collect();
        self.compositor.composite_tiles(
            &self.device,
            &mut encoder,
            frame_target.view,
            frame_target.size,
            &root_tiles,
        );

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        for job in &data.tile_jobs {
            if let Some(tile) = self
                .surfaces
                .surfaces
                .get_mut(&job.id.surface)
                .and_then(|surface| surface.tiles.get_mut(&job.id.coord))
            {
                tile.valid = true;
            }
        }
        self.surfaces
            .finish_frame(self.options.layer_cache_budget_bytes);
        self.texture_pool.trim();
        self.presented_frame = true;
        Ok(())
    }

    fn set_factor(&mut self, factor: f32) -> Result<(), Self::Error> {
        if self.scale_factor.to_bits() != factor.to_bits() {
            self.layer_cache.clear();
            self.surfaces.clear();
        }
        self.scale_factor = factor;
        Ok(())
    }
}

impl<T: TextBackend> WGPUBackend<T> {
    fn prepare_vector_runs(
        &mut self,
        prepared: &mut PrepareResult,
    ) -> Result<(), WgpuBackendError> {
        for run in &mut prepared.runs {
            let PreparedRun::Vector { records, composite } = run else {
                continue;
            };
            *composite = self.vector_render.rasterize_run(
                &self.device,
                &self.queue,
                &prepared.vector_records,
                records.clone(),
                self.scale_factor,
            )?;
        }
        Ok(())
    }

    fn build_frame_render_data(
        &mut self,
        frame: &BuiltFrame,
        viewport_clip: Rect,
        text: &mut TextHost<T>,
    ) -> Result<FrameRenderData, WgpuBackendError> {
        debug_assert!(
            self.shared_tile_target
                .matches(self.options.layer_cache_tile_size)
        );
        self.surfaces
            .begin_frame(frame, self.scale_factor, self.options.layer_cache_tile_size);
        let mut tile_jobs = Vec::new();
        let mut scheduled = HashSet::new();
        let full_viewport = Rect::new(
            0.0,
            0.0,
            self.config.width as f32 / self.scale_factor,
            self.config.height as f32 / self.scale_factor,
        );
        let root_coords = self.surfaces.surfaces[&SurfaceKey::Root]
            .geometry
            .coords_for_rect(full_viewport, 0);
        for coord in root_coords {
            let created = loop {
                match self.surfaces.ensure_tile(
                    &self.device,
                    &self.texture_pool,
                    SurfaceKey::Root,
                    coord,
                    true,
                ) {
                    Ok(created) => break created,
                    Err(TexturePoolError::HardBudgetExceeded { .. })
                        if self.surfaces.evict_lru_auto_unpinned() =>
                    {
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            let tile = &self.surfaces.surfaces[&SurfaceKey::Root].tiles[&coord];
            let tile_bounds = tile.logical_bounds(self.scale_factor);
            if !created && tile.valid && !tile_bounds.intersects(viewport_clip) {
                continue;
            }
            let id = TileJobId {
                surface: SurfaceKey::Root,
                coord,
            };
            if !scheduled.insert(id) {
                continue;
            }
            self.collect_layer_dependencies(
                frame,
                frame.root_layer,
                tile_bounds,
                &mut scheduled,
                &mut tile_jobs,
            )?;
            tile_jobs.push(TileRenderJob {
                id,
                layer: frame.root_layer,
                runs: 0..0,
            });
        }

        let mut prepared = PrepareResult::default();
        let mut job_index = 0;
        while job_index < tile_jobs.len() {
            let id = tile_jobs[job_index].id;
            let layer = tile_jobs[job_index].layer;
            let tile = &self.surfaces.surfaces[&id.surface].tiles[&id.coord];
            let origin = Point::new(
                tile.physical_bounds.x as f32 / self.scale_factor,
                tile.physical_bounds.y as f32 / self.scale_factor,
            );
            let viewport = Rect::new(
                0.0,
                0.0,
                tile.physical_bounds.width as f32 / self.scale_factor,
                tile.physical_bounds.height as f32 / self.scale_factor,
            );
            let start = prepared.begin_run_group();
            let graph_start = prepared.graph_records.len();
            self.push_compiled_layer(
                frame,
                layer,
                id.surface,
                id.coord,
                viewport,
                Affine::translate(-origin.x, -origin.y),
                1.0,
                text,
                &mut prepared,
            )?;
            tile_jobs[job_index].runs = start..prepared.runs.len();
            let graph_end = prepared.graph_records.len();
            for graph_index in graph_start..graph_end {
                self.collect_prefix_dependencies(
                    frame,
                    &prepared.graph_records[graph_index],
                    &mut scheduled,
                    &mut tile_jobs,
                )?;
            }
            job_index += 1;
        }

        Ok(FrameRenderData {
            prepared,
            tile_jobs,
        })
    }

    fn collect_prefix_dependencies(
        &mut self,
        frame: &BuiltFrame,
        record: &LayerGraphDrawRecord,
        scheduled: &mut HashSet<TileJobId>,
        jobs: &mut Vec<TileRenderJob>,
    ) -> Result<(), WgpuBackendError> {
        let Some(prefix_plan) = record
            .prefix_plan
            .as_ref()
            .filter(|plan| plan.crosses_surface())
        else {
            return Ok(());
        };
        let Some(backdrop_id) = record.plan.backdrop() else {
            return Ok(());
        };
        let bounds = record.plan.resources()[backdrop_id.index()].physical_bounds;
        if bounds.width == 0 || bounds.height == 0 {
            return Ok(());
        }
        let tail_tile = &self.surfaces.surfaces[&record.parent_surface].tiles[&record.parent_coord];
        let tail_world = xui_render_graph::PixelRect {
            x: tail_tile
                .physical_bounds
                .x
                .checked_add(bounds.x)
                .ok_or_else(|| std::io::Error::other("prefix x coordinate overflow"))?,
            y: tail_tile
                .physical_bounds
                .y
                .checked_add(bounds.y)
                .ok_or_else(|| std::io::Error::other("prefix y coordinate overflow"))?,
            width: bounds.width,
            height: bounds.height,
        };
        let stages = composite_prefix_stage_demands(
            frame,
            prefix_plan,
            logical_rect(tail_world, self.scale_factor),
        )?;
        for stage in stages {
            let key = surface_key_for_layer(frame, stage.prefix.layer)?;
            let geometry = self.surfaces.surfaces[&key].geometry;
            let required: HashSet<_> = geometry
                .coords_for_rect(stage.demand, 0)
                .into_iter()
                .collect();
            let mut coords: Vec<_> = required.iter().copied().collect();
            coords.extend(
                geometry
                    .coords_for_rect(stage.demand, self.options.surface_viewport_guard_tiles)
                    .into_iter()
                    .filter(|coord| !required.contains(coord)),
            );
            'tiles: for coord in coords {
                let required_tile = required.contains(&coord);
                if !required_tile
                    && !self
                        .surfaces
                        .can_allocate_guard(key, self.options.layer_cache_budget_bytes)
                {
                    continue;
                }
                let created = loop {
                    match self.surfaces.ensure_tile(
                        &self.device,
                        &self.texture_pool,
                        key,
                        coord,
                        true,
                    ) {
                        Ok(created) => break created,
                        Err(TexturePoolError::HardBudgetExceeded { .. }) if !required_tile => {
                            continue 'tiles;
                        }
                        Err(TexturePoolError::HardBudgetExceeded { .. })
                            if self.surfaces.evict_lru_auto_unpinned() =>
                        {
                            continue;
                        }
                        Err(error) => return Err(error.into()),
                    }
                };
                let tile = &self.surfaces.surfaces[&key].tiles[&coord];
                let layer = &frame.layers[stage.prefix.layer.0];
                let dirty = self
                    .layer_cache
                    .last_dirty
                    .get(&layer.source)
                    .is_some_and(|dirty| dirty.intersects(tile.logical_bounds(self.scale_factor)));
                let needs_render = required_tile || created || !tile.valid || dirty;
                let id = TileJobId {
                    surface: key,
                    coord,
                };
                if !needs_render || !scheduled.insert(id) {
                    continue;
                }
                self.collect_layer_dependencies(
                    frame,
                    stage.prefix.layer,
                    tile.logical_bounds(self.scale_factor),
                    scheduled,
                    jobs,
                )?;
                jobs.push(TileRenderJob {
                    id,
                    layer: stage.prefix.layer,
                    runs: 0..0,
                });
            }
        }
        Ok(())
    }

    fn collect_layer_dependencies(
        &mut self,
        frame: &BuiltFrame,
        parent: BuiltLayerId,
        parent_region: Rect,
        scheduled: &mut HashSet<TileJobId>,
        jobs: &mut Vec<TileRenderJob>,
    ) -> Result<(), WgpuBackendError> {
        for item in &frame.layers[parent.0].items {
            let BuiltItem::Layer(instance_id) = item else {
                continue;
            };
            let instance = frame.layer_instance(*instance_id).expect("built instance");
            let Some(destination_clip) = intersect_rect(parent_region, instance.world_bounds)
            else {
                continue;
            };
            let child = &frame.layers[instance.layer.0];
            let plan = instance
                .render_program
                .program()
                .instantiate(&LayerPlanContext {
                    backdrop_source_bounds: parent_region,
                    parent_destination_bounds: parent_region,
                    composite_clip_bounds: Some(destination_clip),
                    layer_content_bounds: child.content_bounds,
                    backdrop_bounds: Some(destination_clip),
                    composite: instance.composite,
                    scale_factor: self.scale_factor,
                    color_texture_class: TextureClass::LINEAR_COLOR,
                    external_aliasing: ExternalAliasing::Distinct,
                    limits: PlanLimits {
                        max_texture_dimension_2d: self.device.limits().max_texture_dimension_2d,
                    },
                })?;
            let requested = plan.resources()[plan.layer_content().index()].logical_bounds;
            if requested.width <= 0.0 || requested.height <= 0.0 {
                continue;
            }
            let cache = child
                .cache_id
                .expect("every isolated built layer has a cache identity");
            let surface_key = SurfaceKey::Layer(cache);
            let Some(geometry) = self
                .surfaces
                .surfaces
                .get(&surface_key)
                .map(|surface| surface.geometry)
            else {
                continue;
            };
            let required: HashSet<_> = geometry.coords_for_rect(requested, 0).into_iter().collect();
            let mut coords: Vec<_> = required.iter().copied().collect();
            let guard =
                geometry.coords_for_rect(requested, self.options.surface_viewport_guard_tiles);
            coords.extend(guard.into_iter().filter(|coord| !required.contains(coord)));
            'tiles: for coord in coords {
                let required_tile = required.contains(&coord);
                if !required_tile
                    && !self
                        .surfaces
                        .can_allocate_guard(surface_key, self.options.layer_cache_budget_bytes)
                {
                    continue;
                }
                let created = loop {
                    match self.surfaces.ensure_tile(
                        &self.device,
                        &self.texture_pool,
                        surface_key,
                        coord,
                        true,
                    ) {
                        Ok(created) => break created,
                        Err(TexturePoolError::HardBudgetExceeded { .. }) if !required_tile => {
                            continue 'tiles;
                        }
                        Err(TexturePoolError::HardBudgetExceeded { .. })
                            if self.surfaces.evict_lru_auto_unpinned() =>
                        {
                            continue;
                        }
                        Err(error) => return Err(error.into()),
                    }
                };
                let Some(tile) = self
                    .surfaces
                    .surfaces
                    .get(&surface_key)
                    .and_then(|surface| surface.tiles.get(&coord))
                else {
                    continue;
                };
                let dirty = self
                    .layer_cache
                    .last_dirty
                    .get(&child.source)
                    .is_some_and(|dirty| dirty.intersects(tile.logical_bounds(self.scale_factor)));
                let needs_render = created || !tile.valid || dirty;
                let id = TileJobId {
                    surface: surface_key,
                    coord,
                };
                if !needs_render || !scheduled.insert(id) {
                    continue;
                }
                let target_region = tile.logical_bounds(self.scale_factor);
                self.collect_layer_dependencies(
                    frame,
                    instance.layer,
                    target_region,
                    scheduled,
                    jobs,
                )?;
                jobs.push(TileRenderJob {
                    id,
                    layer: instance.layer,
                    runs: 0..0,
                });
            }
        }
        Ok(())
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
                PreparedRun::Vector { composite, .. } => self.vector_render.render(
                    pass,
                    ui_bind_group,
                    *composite,
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
                PreparedRun::Graph { .. } => {}
            }
        }
    }

    fn encode_tile_jobs(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &BuiltFrame,
        prepared: &PrepareResult,
        jobs: &[TileRenderJob],
    ) -> Result<(), WgpuBackendError> {
        let mut groups = std::collections::HashMap::<SurfaceKey, Vec<usize>>::new();
        for (index, job) in jobs.iter().enumerate() {
            groups.entry(job.id.surface).or_default().push(index);
        }
        let mut completed = HashSet::new();
        let mut active = Vec::new();
        self.encode_surface_tile_jobs(
            encoder,
            frame,
            prepared,
            jobs,
            &groups,
            SurfaceKey::Root,
            &mut completed,
            &mut active,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_surface_tile_jobs(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &BuiltFrame,
        prepared: &PrepareResult,
        all_jobs: &[TileRenderJob],
        groups: &std::collections::HashMap<SurfaceKey, Vec<usize>>,
        surface: SurfaceKey,
        completed: &mut HashSet<SurfaceKey>,
        active: &mut Vec<SurfaceKey>,
    ) -> Result<(), WgpuBackendError> {
        if completed.contains(&surface) {
            return Ok(());
        }
        if active.contains(&surface) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "surface render dependency contains a cycle",
            )
            .into());
        }
        let Some(indices) = groups.get(&surface) else {
            completed.insert(surface);
            return Ok(());
        };
        let jobs: Vec<_> = indices.iter().map(|index| &all_jobs[*index]).collect();
        active.push(surface);
        let segmented: Vec<_> = jobs
            .iter()
            .map(|job| prepared_run_segments(&prepared.runs, job.runs.clone()))
            .collect();
        let barrier_count = segmented
            .first()
            .map_or(0, |segments| segments.len().saturating_sub(1));
        debug_assert!(segmented.iter().all(|segments| {
            segments.len() == barrier_count + 1
                && segments[..barrier_count]
                    .iter()
                    .all(|segment| segment.item_index.is_some())
        }));

        for barrier in 0..barrier_count {
            let item_index = segmented[0][barrier]
                .item_index
                .expect("non-trailing segments end at a placement barrier");
            debug_assert!(
                segmented
                    .iter()
                    .all(|segments| segments[barrier].item_index == Some(item_index))
            );

            for (job, segments) in jobs.iter().zip(&segmented) {
                let runs = segments[barrier].runs.clone();
                if barrier == 0 || !runs.is_empty() {
                    self.encode_tile_segment(encoder, prepared, job, runs, barrier == 0);
                }
            }

            // Every tile of this surface is now resolved at the exact same
            // paint-order prefix, so a backdrop footprint can stitch adjacent
            // tiles without observing later content from another tile.
            let mut children = Vec::new();
            let mut seen_children = HashSet::new();
            for segments in &segmented {
                let Some(record) = segments[barrier].graph else {
                    continue;
                };
                let child = prepared.graph_records[record].surface;
                if seen_children.insert(child) {
                    children.push(child);
                }
            }
            for child in children {
                self.encode_surface_tile_jobs(
                    encoder, frame, prepared, all_jobs, groups, child, completed, active,
                )?;
            }
            for (job, segments) in jobs.iter().zip(&segmented) {
                let Some(record) = segments[barrier].graph else {
                    continue;
                };
                self.restore_tile_target(encoder, job);
                self.encode_graph_record(encoder, frame, prepared, job, record, active)?;
            }
        }

        for (job, segments) in jobs.iter().zip(&segmented) {
            let trailing = segments[barrier_count].runs.clone();
            if barrier_count == 0 || !trailing.is_empty() {
                self.encode_tile_segment(encoder, prepared, job, trailing, barrier_count == 0);
            }
        }
        let popped = active.pop();
        debug_assert_eq!(popped, Some(surface));
        completed.insert(surface);
        Ok(())
    }

    fn encode_tile_segment(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        prepared: &PrepareResult,
        job: &TileRenderJob,
        runs: std::ops::Range<usize>,
        first_pass: bool,
    ) {
        if !first_pass {
            self.restore_tile_target(encoder, job);
        }
        let resident = &self.surfaces.surfaces[&job.id.surface].tiles[&job.id.coord];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xui layer tile content pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.shared_tile_target.msaa.view(),
                resolve_target: Some(resident.texture.view()),
                ops: wgpu::Operations {
                    load: if first_pass {
                        wgpu::LoadOp::Clear(if job.id.surface == SurfaceKey::Root {
                            wgpu::Color {
                                r: 0.08,
                                g: 0.09,
                                b: 0.11,
                                a: 1.0,
                            }
                        } else {
                            wgpu::Color::TRANSPARENT
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
            prepared,
            runs,
            &self.shared_tile_ui_bind_group,
            self.shared_tile_target.extent,
        );
    }

    fn restore_tile_target(&self, encoder: &mut wgpu::CommandEncoder, job: &TileRenderJob) {
        let tile = &self.surfaces.surfaces[&job.id.surface].tiles[&job.id.coord];
        let allocation = tile.texture.allocation_extent();
        self.compositor.restore_tile_msaa(
            &self.device,
            encoder,
            self.shared_tile_target.msaa.view(),
            self.shared_tile_target.extent,
            CompositeTile {
                view: tile.texture.view(),
                origin: (0, 0),
                valid_extent: (tile.physical_bounds.width, tile.physical_bounds.height),
                allocation_extent: (allocation.width, allocation.height),
            },
        );
    }

    fn encode_graph_record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &BuiltFrame,
        prepared: &PrepareResult,
        job: &TileRenderJob,
        record_index: usize,
        active_surfaces: &[SurfaceKey],
    ) -> Result<(), WgpuBackendError> {
        let record = &prepared.graph_records[record_index];
        let content_bounds =
            record.plan.resources()[record.plan.layer_content().index()].physical_bounds;
        let content = self.materialize_surface_tiles(
            encoder,
            record.surface,
            &record.coords,
            content_bounds,
            "xui stitched layer content",
        )?;
        let destination_bounds =
            record.plan.resources()[record.plan.parent_destination().index()].physical_bounds;
        let parent_destination = self.snapshot_tile_destination(
            encoder,
            record.parent_surface,
            record.parent_coord,
            destination_bounds,
            false,
            "xui parent destination snapshot",
        )?;
        let backdrop = record
            .plan
            .backdrop()
            .map(|id| record.plan.resources()[id.index()].physical_bounds)
            .filter(|bounds| bounds.width > 0 && bounds.height > 0)
            .map(|bounds| {
                if let Some(tail) = record.prefix_plan.as_ref().and_then(|plan| plan.tail()) {
                    let expected_surface = if tail.layer == frame.root_layer {
                        SurfaceKey::Root
                    } else {
                        SurfaceKey::Layer(
                            frame.layers[tail.layer.0]
                                .cache_id
                                .expect("isolated prefix surface has a cache identity"),
                        )
                    };
                    if expected_surface != record.parent_surface {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "composite-prefix tail does not match the active parent surface",
                        )
                        .into());
                    }
                }
                let crosses_surface = record
                    .prefix_plan
                    .as_ref()
                    .is_some_and(CompositePrefixPlan::crosses_surface);
                if crosses_surface {
                    self.materialize_composite_prefix(
                        encoder,
                        frame,
                        record
                            .prefix_plan
                            .as_ref()
                            .expect("cross-surface prefix has a plan"),
                        active_surfaces,
                        record.parent_surface,
                        record.parent_coord,
                        bounds,
                    )
                } else {
                    self.snapshot_tile_destination(
                        encoder,
                        record.parent_surface,
                        record.parent_coord,
                        bounds,
                        true,
                        "xui local composite-prefix backdrop",
                    )
                }
            })
            .transpose()?;
        let target = &self.surfaces.surfaces[&job.id.surface].tiles[&job.id.coord];
        self.render_graph_renderer.encode(
            &self.device,
            &self.queue,
            encoder,
            &record.plan,
            &record.program,
            GraphTarget {
                view: target.texture.view(),
                msaa_view: self.shared_tile_target.msaa.view(),
                extent: self.shared_tile_target.extent,
                logical_bounds: Rect::new(
                    0.0,
                    0.0,
                    target.physical_bounds.width as f32 / self.scale_factor,
                    target.physical_bounds.height as f32 / self.scale_factor,
                ),
            },
            GraphTexture {
                view: content.texture.view(),
                extent: content.extent(),
                logical_bounds: content.logical_bounds,
            },
            match &parent_destination {
                MaterializedBackdrop::Shared { physical_bounds } => GraphTexture {
                    view: self.shared_tile_target.resolve_scratch.view(),
                    extent: (physical_bounds.width, physical_bounds.height),
                    logical_bounds: logical_rect(*physical_bounds, self.scale_factor),
                },
                MaterializedBackdrop::Temporary(destination) => GraphTexture {
                    view: destination.texture.view(),
                    extent: destination.extent(),
                    logical_bounds: destination.logical_bounds,
                },
            },
            backdrop.as_ref().map(|backdrop| match backdrop {
                MaterializedBackdrop::Shared { physical_bounds } => GraphTexture {
                    view: self.shared_tile_target.resolve_scratch.view(),
                    extent: (physical_bounds.width, physical_bounds.height),
                    logical_bounds: logical_rect(*physical_bounds, self.scale_factor),
                },
                MaterializedBackdrop::Temporary(backdrop) => GraphTexture {
                    view: backdrop.texture.view(),
                    extent: backdrop.extent(),
                    logical_bounds: backdrop.logical_bounds,
                },
            }),
            &self.image_render,
            self.scale_factor,
        )?;
        Ok(())
    }

    fn materialize_composite_prefix(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &BuiltFrame,
        plan: &CompositePrefixPlan,
        active_surfaces: &[SurfaceKey],
        tail_surface: SurfaceKey,
        tail_coord: TileCoord,
        physical_bounds: xui_render_graph::PixelRect,
    ) -> Result<MaterializedBackdrop, WgpuBackendError> {
        let tail_tile = &self.surfaces.surfaces[&tail_surface].tiles[&tail_coord];
        let tail_world_physical = xui_render_graph::PixelRect {
            x: tail_tile
                .physical_bounds
                .x
                .checked_add(physical_bounds.x)
                .ok_or_else(|| std::io::Error::other("prefix x coordinate overflow"))?,
            y: tail_tile
                .physical_bounds
                .y
                .checked_add(physical_bounds.y)
                .ok_or_else(|| std::io::Error::other("prefix y coordinate overflow"))?,
            width: physical_bounds.width,
            height: physical_bounds.height,
        };
        let stages = composite_prefix_stage_demands(
            frame,
            plan,
            logical_rect(tail_world_physical, self.scale_factor),
        )?;
        let mut current: Option<TemporarySurface> = None;
        for stage in stages {
            let key = surface_key_for_layer(frame, stage.prefix.layer)?;
            if !active_surfaces.contains(&key) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "composite-prefix source surface is not paused at its requested prefix",
                )
                .into());
            }
            let stage_physical =
                crate::wgpu::tex::physical_rect(stage.demand, self.scale_factor)
                    .ok_or_else(|| std::io::Error::other("empty composite-prefix demand"))?;
            let surface = &self.surfaces.surfaces[&key];
            let mut coords = surface.geometry.coords_for_rect(stage.demand, 0);
            coords.retain(|coord| surface.tiles.contains_key(coord));
            coords.sort_unstable();
            let local = self.materialize_surface_tiles(
                encoder,
                key,
                &coords,
                stage_physical,
                "xui composite-prefix replay surface",
            )?;

            current = Some(match (current, stage.placement) {
                (None, None) => local,
                (Some(parent), Some(instance_id)) => {
                    let placement = frame
                        .layer_instance(instance_id)
                        .ok_or_else(|| std::io::Error::other("invalid prefix placement"))?;
                    let parent =
                        self.apply_backdrop_only_to_prefix(encoder, frame, instance_id, parent)?;
                    let child = TemporarySurface::new(
                        &self.device,
                        &self.texture_pool,
                        stage_physical,
                        self.scale_factor,
                        "xui composite-prefix placement temporary",
                    )?;
                    let parent_allocation = parent.texture.allocation_extent();
                    self.compositor.blit_scene(
                        &self.device,
                        encoder,
                        child.texture.view(),
                        child.extent(),
                        stage.demand,
                        SceneBlitSource {
                            view: parent.texture.view(),
                            allocation_extent: (parent_allocation.width, parent_allocation.height),
                            logical_bounds: parent.logical_bounds,
                        },
                        placement.composite.transform,
                        1,
                        SceneBlitBlend::Replace,
                        true,
                    );
                    let local_allocation = local.texture.allocation_extent();
                    self.compositor.blit_scene(
                        &self.device,
                        encoder,
                        child.texture.view(),
                        child.extent(),
                        stage.demand,
                        SceneBlitSource {
                            view: local.texture.view(),
                            allocation_extent: (local_allocation.width, local_allocation.height),
                            logical_bounds: local.logical_bounds,
                        },
                        Affine::IDENTITY,
                        1,
                        SceneBlitBlend::SrcOver,
                        false,
                    );
                    child
                }
                (Some(_), None) | (None, Some(_)) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "malformed composite-prefix placement chain",
                    )
                    .into());
                }
            });
        }
        let mut current = current.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "composite-prefix plan has no replay stage",
            )
        })?;
        current.logical_bounds = logical_rect(physical_bounds, self.scale_factor);
        Ok(MaterializedBackdrop::Temporary(current))
    }

    fn apply_backdrop_only_to_prefix(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &BuiltFrame,
        instance_id: xui::render::BuiltLayerInstanceId,
        parent: TemporarySurface,
    ) -> Result<TemporarySurface, WgpuBackendError> {
        let instance = frame
            .layer_instance(instance_id)
            .ok_or_else(|| std::io::Error::other("invalid prefix placement"))?;
        if instance
            .render_program
            .program()
            .external_resource(xui_render_graph::ExternalResourceKind::Backdrop)
            .is_none()
        {
            return Ok(parent);
        }

        let output = TemporarySurface::new(
            &self.device,
            &self.texture_pool,
            parent.physical_bounds,
            self.scale_factor,
            "xui composite-prefix backdrop-only output",
        )?;
        let backdrop_copy = TemporarySurface::new(
            &self.device,
            &self.texture_pool,
            parent.physical_bounds,
            self.scale_factor,
            "xui composite-prefix distinct backdrop input",
        )?;
        let parent_allocation = parent.texture.allocation_extent();
        self.compositor.blit_scene(
            &self.device,
            encoder,
            backdrop_copy.texture.view(),
            backdrop_copy.extent(),
            parent.logical_bounds,
            SceneBlitSource {
                view: parent.texture.view(),
                allocation_extent: (parent_allocation.width, parent_allocation.height),
                logical_bounds: parent.logical_bounds,
            },
            Affine::IDENTITY,
            1,
            SceneBlitBlend::Replace,
            true,
        );
        let dummy = TemporarySurface::new(
            &self.device,
            &self.texture_pool,
            xui_render_graph::PixelRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            self.scale_factor,
            "xui backdrop-only unused layer content",
        )?;
        let chunk_width = self.shared_tile_target.extent.0.max(1);
        let chunk_height = self.shared_tile_target.extent.1.max(1);
        let full = parent.physical_bounds;
        let right = i64::from(full.x) + i64::from(full.width);
        let bottom = i64::from(full.y) + i64::from(full.height);
        let mut y = i64::from(full.y);
        while y < bottom {
            let height = u32::try_from((bottom - y).min(i64::from(chunk_height)))?;
            let mut x = i64::from(full.x);
            while x < right {
                let width = u32::try_from((right - x).min(i64::from(chunk_width)))?;
                let chunk = xui_render_graph::PixelRect {
                    x: i32::try_from(x)?,
                    y: i32::try_from(y)?,
                    width,
                    height,
                };
                let chunk_logical = logical_rect(chunk, self.scale_factor);
                let backdrop_bounds = intersect_rect(chunk_logical, instance.world_bounds);
                let plan = instance.render_program.program().instantiate_entry(
                    xui_render_graph::LayerProgramEntry::BackdropOnly,
                    &LayerPlanContext {
                        backdrop_source_bounds: parent.logical_bounds,
                        parent_destination_bounds: chunk_logical,
                        composite_clip_bounds: backdrop_bounds,
                        layer_content_bounds: frame.layers[instance.layer.0].content_bounds,
                        backdrop_bounds,
                        composite: instance.composite,
                        scale_factor: self.scale_factor,
                        color_texture_class: TextureClass::LINEAR_COLOR,
                        external_aliasing: ExternalAliasing::Distinct,
                        limits: PlanLimits {
                            max_texture_dimension_2d: self.device.limits().max_texture_dimension_2d,
                        },
                    },
                )?;

                if plan.is_noop() {
                    let source_x = u32::try_from(x - i64::from(full.x))?;
                    let source_y = u32::try_from(y - i64::from(full.y))?;
                    encoder.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: parent.texture.texture(),
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: source_x,
                                y: source_y,
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: output.texture.texture(),
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: source_x,
                                y: source_y,
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                    );
                    x += i64::from(width);
                    continue;
                }

                self.compositor.blit_scene(
                    &self.device,
                    encoder,
                    self.shared_tile_target.msaa.view(),
                    (width, height),
                    chunk_logical,
                    SceneBlitSource {
                        view: parent.texture.view(),
                        allocation_extent: (parent_allocation.width, parent_allocation.height),
                        logical_bounds: parent.logical_bounds,
                    },
                    Affine::IDENTITY,
                    SCENE_SAMPLE_COUNT,
                    SceneBlitBlend::Replace,
                    true,
                );
                self.render_graph_renderer.encode(
                    &self.device,
                    &self.queue,
                    encoder,
                    &plan,
                    &instance.render_program,
                    GraphTarget {
                        view: self.shared_tile_target.resolve_scratch.view(),
                        msaa_view: self.shared_tile_target.msaa.view(),
                        extent: self.shared_tile_target.extent,
                        logical_bounds: chunk_logical,
                    },
                    GraphTexture {
                        view: dummy.texture.view(),
                        extent: dummy.extent(),
                        logical_bounds: dummy.logical_bounds,
                    },
                    GraphTexture {
                        view: parent.texture.view(),
                        extent: parent.extent(),
                        logical_bounds: parent.logical_bounds,
                    },
                    Some(GraphTexture {
                        view: backdrop_copy.texture.view(),
                        extent: backdrop_copy.extent(),
                        logical_bounds: backdrop_copy.logical_bounds,
                    }),
                    &self.image_render,
                    self.scale_factor,
                )?;
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: self.shared_tile_target.resolve_scratch.texture(),
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: output.texture.texture(),
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: u32::try_from(x - i64::from(full.x))?,
                            y: u32::try_from(y - i64::from(full.y))?,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
                x += i64::from(width);
            }
            y += i64::from(height);
        }
        Ok(output)
    }

    fn materialize_surface_tiles(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        surface_key: SurfaceKey,
        coords: &[TileCoord],
        physical_bounds: xui_render_graph::PixelRect,
        label: &'static str,
    ) -> Result<TemporarySurface, WgpuBackendError> {
        let temporary = TemporarySurface::new(
            &self.device,
            &self.texture_pool,
            physical_bounds,
            self.scale_factor,
            label,
        )?;
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui temporary surface clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: temporary.texture.view(),
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
        }
        let surface = &self.surfaces.surfaces[&surface_key];
        for coord in coords {
            let Some(tile) = surface.tiles.get(coord) else {
                continue;
            };
            let Some(overlap) = intersect_pixel_rect(tile.physical_bounds, physical_bounds) else {
                continue;
            };
            let source_x = u32::try_from(overlap.x - tile.physical_bounds.x)
                .expect("overlap is inside source tile");
            let source_y = u32::try_from(overlap.y - tile.physical_bounds.y)
                .expect("overlap is inside source tile");
            let destination_x = u32::try_from(overlap.x - physical_bounds.x)
                .expect("overlap is inside temporary surface");
            let destination_y = u32::try_from(overlap.y - physical_bounds.y)
                .expect("overlap is inside temporary surface");
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tile.texture.texture(),
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: source_x,
                        y: source_y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: temporary.texture.texture(),
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: destination_x,
                        y: destination_y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: overlap.width,
                    height: overlap.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        Ok(temporary)
    }

    fn snapshot_tile_destination(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        surface_key: SurfaceKey,
        current_coord: TileCoord,
        physical_bounds: xui_render_graph::PixelRect,
        allow_shared: bool,
        label: &'static str,
    ) -> Result<MaterializedBackdrop, WgpuBackendError> {
        let uses_shared = allow_shared
            && physical_bounds.width <= self.shared_tile_target.extent.0
            && physical_bounds.height <= self.shared_tile_target.extent.1;
        let temporary = (!uses_shared)
            .then(|| {
                TemporarySurface::new(
                    &self.device,
                    &self.texture_pool,
                    physical_bounds,
                    self.scale_factor,
                    label,
                )
            })
            .transpose()?;
        let destination_view = temporary
            .as_ref()
            .map(|surface| surface.texture.view())
            .unwrap_or_else(|| self.shared_tile_target.resolve_scratch.view());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui backdrop temporary clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: destination_view,
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
        }
        let destination_texture = temporary
            .as_ref()
            .map(|surface| surface.texture.texture())
            .unwrap_or_else(|| self.shared_tile_target.resolve_scratch.texture());
        let surface = &self.surfaces.surfaces[&surface_key];
        let current = &surface.tiles[&current_coord];
        let mut coords: Vec<_> = surface.tiles.keys().copied().collect();
        coords.sort_unstable_by_key(|coord| (*coord == current_coord, *coord));
        for coord in coords {
            let tile = &surface.tiles[&coord];
            let tile_bounds = xui_render_graph::PixelRect {
                x: tile.physical_bounds.x - current.physical_bounds.x,
                y: tile.physical_bounds.y - current.physical_bounds.y,
                width: tile.physical_bounds.width,
                height: tile.physical_bounds.height,
            };
            let Some(overlap) = intersect_pixel_rect(tile_bounds, physical_bounds) else {
                continue;
            };
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tile.texture.texture(),
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: u32::try_from(overlap.x - tile_bounds.x)
                            .expect("overlap is inside backdrop source tile"),
                        y: u32::try_from(overlap.y - tile_bounds.y)
                            .expect("overlap is inside backdrop source tile"),
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: destination_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: u32::try_from(overlap.x - physical_bounds.x)
                            .expect("overlap is inside backdrop temporary"),
                        y: u32::try_from(overlap.y - physical_bounds.y)
                            .expect("overlap is inside backdrop temporary"),
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: overlap.width,
                    height: overlap.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        Ok(match temporary {
            Some(surface) => MaterializedBackdrop::Temporary(surface),
            None => MaterializedBackdrop::Shared { physical_bounds },
        })
    }

    fn push_compiled_layer(
        &mut self,
        frame: &BuiltFrame,
        layer_id: BuiltLayerId,
        target_surface: SurfaceKey,
        target_coord: TileCoord,
        viewport_clip: Rect,
        placement_transform: Affine,
        placement_opacity: f32,
        text: &mut TextHost<T>,
        result: &mut PrepareResult,
    ) -> Result<(), WgpuBackendError> {
        for (item_index, item) in frame.layers[layer_id.0].items.iter().enumerate() {
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
                        BuiltDraw::Vector(value) => result.vector_records.push(VectorDrawRecord {
                            scene: value.primitive.scene.clone(),
                            transform: value.primitive.transform.then(world_transform),
                            opacity: placement_opacity,
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
                BuiltItem::Layer(instance_id) => {
                    let graph_barrier = result.push_graph_barrier(item_index);
                    let instance = frame.layer_instance(*instance_id).expect("built instance");
                    if instance
                        .render_program
                        .program()
                        .external_resource(xui_render_graph::ExternalResourceKind::Backdrop)
                        .is_some()
                    {
                        let prefix = instance.destination_prefix.ok_or_else(|| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "backdrop program is missing its composite-prefix binding",
                            )
                        })?;
                        let node = frame.composite_prefix(prefix).ok_or_else(|| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "backdrop program references an invalid composite prefix",
                            )
                        })?;
                        if node.local.layer != layer_id || node.local.item_count != item_index {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "backdrop prefix does not match the active surface version",
                            )
                            .into());
                        }
                    }
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
                    let surface_key = SurfaceKey::Layer(cache);
                    let Some(resident) = self.surfaces.surfaces.get(&surface_key) else {
                        continue;
                    };
                    let transform = instance.composite.transform.then(placement_transform);
                    let opacity = placement_opacity * instance.composite.opacity.clamp(0.0, 1.0);
                    let placed_bounds = placement_transform.transform_rect(instance.world_bounds);
                    let backdrop_bounds = intersect_rect(instance_clip, placed_bounds);
                    let destination_clip =
                        intersect_rect(instance_clip, placed_bounds).unwrap_or(instance_clip);
                    let parent_surface_bounds =
                        placement_transform.transform_rect(frame.layers[layer_id.0].render_bounds);
                    let plan =
                        instance
                            .render_program
                            .program()
                            .instantiate(&LayerPlanContext {
                                backdrop_source_bounds: parent_surface_bounds,
                                parent_destination_bounds: viewport_clip,
                                composite_clip_bounds: Some(destination_clip),
                                layer_content_bounds: layer.content_bounds,
                                backdrop_bounds,
                                composite: xui_render_graph::CompositeInstance {
                                    opacity,
                                    transform,
                                },
                                scale_factor: self.scale_factor,
                                color_texture_class: TextureClass::LINEAR_COLOR,
                                external_aliasing: ExternalAliasing::Distinct,
                                limits: PlanLimits {
                                    max_texture_dimension_2d: self
                                        .device
                                        .limits()
                                        .max_texture_dimension_2d,
                                },
                            })?;
                    let content_demand =
                        plan.resources()[plan.layer_content().index()].logical_bounds;
                    let mut coords = resident.geometry.coords_for_rect(content_demand, 0);
                    coords.retain(|coord| resident.tiles.contains_key(coord));
                    coords.sort_unstable();
                    result.fill_graph_barrier(
                        graph_barrier,
                        LayerGraphDrawRecord {
                            parent_surface: target_surface,
                            parent_coord: target_coord,
                            surface: surface_key,
                            coords: coords.into_boxed_slice(),
                            program: instance.render_program.clone(),
                            plan,
                            prefix_plan: instance
                                .destination_prefix
                                .and_then(|prefix| CompositePrefixPlan::from_tail(frame, prefix)),
                        },
                    );
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

        let handle = text
            .active_slot(command.node_id, command.slot)
            .expect("text layout must be prepared before paint");
        let layout = text
            .layout(handle)
            .expect("active text layout must be resident before paint");
        let vertical_offset =
            text_vertical_offset(command.vertical_align, rect.height, layout.size().height);
        let text_rect = Rect::new(
            rect.x,
            rect.y + vertical_offset,
            rect.width,
            rect.height - vertical_offset,
        );

        let layout_query = text.query(handle);

        if let (Some(selection), Some(query)) = (command.paint.selection, layout_query) {
            if selection.color.a > 0.0 {
                for selection_rect in query.selection_rects(selection.range) {
                    let screen_rect = Rect::new(
                        text_rect.x + selection_rect.x,
                        text_rect.y + selection_rect.y,
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
                    text_rect.x + glyph.draw_pos.x + bitmap.left as f32 * scale,
                    text_rect.y + glyph.draw_pos.y - bitmap.top as f32 * scale,
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
                text_rect,
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

fn text_vertical_offset(align: TextVerticalAlign, box_height: f32, layout_height: f32) -> f32 {
    match align {
        TextVerticalAlign::Top | TextVerticalAlign::Baseline => 0.0,
        TextVerticalAlign::Middle => ((box_height - layout_height) * 0.5).max(0.0),
        TextVerticalAlign::Bottom => (box_height - layout_height).max(0.0),
    }
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
    pub vector_records: Vec<VectorDrawRecord>,
    pub glyph_records: Vec<TextGlyphRecord>,
    pub graph_records: Vec<LayerGraphDrawRecord>,
    runs: Vec<PreparedRun>,
    merge_floor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreparedRun {
    Sdf(std::ops::Range<usize>),
    Vector {
        records: std::ops::Range<usize>,
        composite: Option<usize>,
    },
    Image(std::ops::Range<usize>),
    Glyph(std::ops::Range<usize>),
    Graph {
        item_index: usize,
        graph: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedRunSegment {
    runs: std::ops::Range<usize>,
    item_index: Option<usize>,
    graph: Option<usize>,
}

fn prepared_run_segments(
    prepared: &[PreparedRun],
    runs: std::ops::Range<usize>,
) -> Vec<PreparedRunSegment> {
    let mut result = Vec::new();
    let mut start = runs.start;
    for index in runs.clone() {
        if let PreparedRun::Graph { item_index, graph } = prepared[index] {
            result.push(PreparedRunSegment {
                runs: start..index,
                item_index: Some(item_index),
                graph,
            });
            start = index + 1;
        }
    }
    result.push(PreparedRunSegment {
        runs: start..runs.end,
        item_index: None,
        graph: None,
    });
    result
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
    fn begin_run_group(&mut self) -> usize {
        self.merge_floor = self.runs.len();
        self.merge_floor
    }

    fn record_lengths(&self) -> [usize; 4] {
        [
            self.sdf_records.len(),
            self.vector_records.len(),
            self.image_records.len(),
            self.glyph_records.len(),
        ]
    }

    fn push_graph_barrier(&mut self, item_index: usize) -> usize {
        let index = self.runs.len();
        self.runs.push(PreparedRun::Graph {
            item_index,
            graph: None,
        });
        self.merge_floor = self.runs.len();
        index
    }

    fn fill_graph_barrier(&mut self, barrier: usize, record: LayerGraphDrawRecord) {
        let index = self.graph_records.len();
        self.graph_records.push(record);
        let PreparedRun::Graph { graph, .. } = &mut self.runs[barrier] else {
            unreachable!("graph barrier index must refer to a graph run")
        };
        *graph = Some(index);
    }

    fn finish_draw_run(&mut self, draw: &BuiltDraw, start: [usize; 4]) {
        let end = self.record_lengths();
        match draw {
            BuiltDraw::Shape(_) => self.push_run(PreparedRun::Sdf(start[0]..end[0])),
            BuiltDraw::Vector(_) => self.push_run(PreparedRun::Vector {
                records: start[1]..end[1],
                composite: None,
            }),
            BuiltDraw::Image(_) => self.push_run(PreparedRun::Image(start[2]..end[2])),
            BuiltDraw::Text(_) => {
                self.push_run(PreparedRun::Sdf(start[0]..end[0]));
                self.push_run(PreparedRun::Glyph(start[3]..end[3]));
            }
        }
    }

    fn push_run(&mut self, run: PreparedRun) {
        let range = match &run {
            PreparedRun::Sdf(range) | PreparedRun::Image(range) | PreparedRun::Glyph(range) => {
                range
            }
            PreparedRun::Vector { records, .. } => records,
            PreparedRun::Graph { .. } => {
                self.runs.push(run);
                self.merge_floor = self.runs.len();
                return;
            }
        };
        if range.is_empty() {
            return;
        }
        let merged = self.runs.len() > self.merge_floor
            && match (self.runs.last_mut(), &run) {
                (Some(PreparedRun::Sdf(previous)), PreparedRun::Sdf(next))
                | (Some(PreparedRun::Image(previous)), PreparedRun::Image(next))
                | (Some(PreparedRun::Glyph(previous)), PreparedRun::Glyph(next))
                    if previous.end == next.start =>
                {
                    previous.end = next.end;
                    true
                }
                (
                    Some(PreparedRun::Vector {
                        records: previous,
                        composite: None,
                    }),
                    PreparedRun::Vector {
                        records: next,
                        composite: None,
                    },
                ) if previous.end == next.start => {
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

#[cfg(test)]
mod prepared_run_tests {
    use super::{PrepareResult, PreparedRun, text_vertical_offset};
    use xui_interface::TextVerticalAlign;

    #[test]
    fn adjacent_vectors_merge_only_within_one_render_target() {
        let mut prepared = PrepareResult::default();
        prepared.begin_run_group();
        prepared.push_run(PreparedRun::Vector {
            records: 0..1,
            composite: None,
        });
        prepared.push_run(PreparedRun::Vector {
            records: 1..2,
            composite: None,
        });
        assert_eq!(
            prepared.runs,
            [PreparedRun::Vector {
                records: 0..2,
                composite: None,
            }]
        );

        prepared.begin_run_group();
        prepared.push_run(PreparedRun::Vector {
            records: 2..3,
            composite: None,
        });
        assert_eq!(prepared.runs.len(), 2);
    }

    #[test]
    fn text_vertical_alignment_offsets_inside_the_box() {
        assert_eq!(
            text_vertical_offset(TextVerticalAlign::Top, 100.0, 40.0),
            0.0
        );
        assert_eq!(
            text_vertical_offset(TextVerticalAlign::Baseline, 100.0, 40.0),
            0.0
        );
        assert_eq!(
            text_vertical_offset(TextVerticalAlign::Middle, 100.0, 40.0),
            30.0
        );
        assert_eq!(
            text_vertical_offset(TextVerticalAlign::Bottom, 100.0, 40.0),
            60.0
        );
        assert_eq!(
            text_vertical_offset(TextVerticalAlign::Bottom, 20.0, 40.0),
            0.0
        );
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

fn expand_sample_rect(rect: Rect, expansion: xui_render_graph::SampleExpansion) -> Rect {
    Rect::new(
        rect.x - expansion.left,
        rect.y - expansion.top,
        rect.width + expansion.left + expansion.right,
        rect.height + expansion.top + expansion.bottom,
    )
}

fn composite_prefix_stage_demands(
    frame: &BuiltFrame,
    plan: &CompositePrefixPlan,
    tail_demand: Rect,
) -> Result<Vec<PrefixStageDemand>, WgpuBackendError> {
    let mut demand = tail_demand;
    let mut reverse_demands = Vec::new();
    for step in plan.steps.iter().rev() {
        match *step {
            CompositePrefixStep::Replay(prefix) => reverse_demands.push((prefix, demand)),
            CompositePrefixStep::TraversePlacement { instance, entry } => {
                if entry != xui_render_graph::LayerProgramEntry::BackdropOnly {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "composite-prefix traversal must use BackdropOnly",
                    )
                    .into());
                }
                let placement = frame
                    .layer_instance(instance)
                    .ok_or_else(|| std::io::Error::other("invalid prefix placement"))?;
                demand = placement.composite.transform.transform_rect(demand);
                demand = expand_sample_rect(
                    demand,
                    placement
                        .render_program
                        .program()
                        .backdrop_input_expansion(),
                );
            }
        }
    }
    reverse_demands.reverse();

    let mut pending_placement = None;
    let mut demand_index = 0;
    let mut stages = Vec::with_capacity(reverse_demands.len());
    for step in &plan.steps {
        match *step {
            CompositePrefixStep::TraversePlacement { instance, .. } => {
                if pending_placement.replace(instance).is_some() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "composite-prefix contains adjacent placements",
                    )
                    .into());
                }
            }
            CompositePrefixStep::Replay(prefix) => {
                let Some((expected, demand)) = reverse_demands.get(demand_index).copied() else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "composite-prefix demand count does not match replay count",
                    )
                    .into());
                };
                if expected != prefix {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "composite-prefix replay order changed during demand mapping",
                    )
                    .into());
                }
                demand_index += 1;
                stages.push(PrefixStageDemand {
                    prefix,
                    demand,
                    placement: pending_placement.take(),
                });
            }
        }
    }
    if pending_placement.is_some() || demand_index != reverse_demands.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "composite-prefix terminates with an unmatched placement",
        )
        .into());
    }
    Ok(stages)
}

fn surface_key_for_layer(
    frame: &BuiltFrame,
    layer: BuiltLayerId,
) -> Result<SurfaceKey, WgpuBackendError> {
    if layer == frame.root_layer {
        Ok(SurfaceKey::Root)
    } else {
        frame.layers[layer.0]
            .cache_id
            .map(SurfaceKey::Layer)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "isolated prefix layer has no cache identity",
                )
                .into()
            })
    }
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
mod backdrop_run_tests {
    use super::*;

    #[test]
    fn graph_barriers_split_runs_immediately_before_layer_execution() {
        let runs = vec![
            PreparedRun::Sdf(0..1),
            PreparedRun::Graph {
                item_index: 1,
                graph: Some(0),
            },
            PreparedRun::Image(0..1),
            PreparedRun::Graph {
                item_index: 3,
                graph: Some(1),
            },
            PreparedRun::Image(1..2),
            PreparedRun::Glyph(0..1),
        ];

        assert_eq!(
            prepared_run_segments(&runs, 0..runs.len()),
            vec![
                PreparedRunSegment {
                    runs: 0..1,
                    item_index: Some(1),
                    graph: Some(0),
                },
                PreparedRunSegment {
                    runs: 2..3,
                    item_index: Some(3),
                    graph: Some(1),
                },
                PreparedRunSegment {
                    runs: 4..6,
                    item_index: None,
                    graph: None,
                },
            ]
        );
    }
}
