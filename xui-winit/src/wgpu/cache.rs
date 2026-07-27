use crate::{
    renders::LayerTileRenderer,
    wgpu::{
        LayerSnapshot, LayerTileStorageVersion, ResidentLayer, ResidentTile, SCENE_FORMAT,
        SCENE_SAMPLE_COUNT, UiUniforms, intersect_rect, layer::diff_layer,
        layer_effect_final_index, snapshot::layer_snapshot,
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use wgpu::util::DeviceExt;
use xui::render::{
    BuiltFrame, CachePolicy, ContentVersion, LayerCacheId, LayerEffect, RenderNodeId,
};
use xui_interface::Point;
use xui_interface::{Affine, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LayerCacheVersion {
    content: ContentVersion,
    width: u32,
    height: u32,
    scale_bits: u32,
}

#[derive(Debug, Clone)]
struct LayerCacheEntry {
    version: LayerCacheVersion,
    bytes: u64,
    last_used: u64,
    policy: CachePolicy,
    dirty: BackendDirtyRegion,
    dirty_tiles: HashSet<(i32, i32)>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct BackendDirtyRegion {
    rects: Vec<Rect>,
}

impl BackendDirtyRegion {
    pub fn full(rect: Rect) -> Self {
        let mut region = Self::default();
        region.add(rect);
        region
    }

    pub fn add(&mut self, rect: Rect) {
        if rect.width > 0.0 && rect.height > 0.0 {
            self.rects.push(rect);
        }
    }

    pub fn extend(&mut self, other: Self) {
        self.rects.extend(other.rects);
    }

    pub fn backdrop_damage(&self, output_bounds: Rect, expansion: f32) -> Self {
        let mut damage = Self::default();
        for rect in &self.rects {
            if let Some(affected) = intersect_rect(rect.expand(expansion), output_bounds) {
                damage.add(affected);
            }
        }
        damage
    }

    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().copied().reduce(Rect::union)
    }

    pub fn expand(&mut self, amount: f32) {
        if amount > 0.0 {
            for rect in &mut self.rects {
                *rect = rect.expand(amount);
            }
        }
    }

    pub fn add_transformed(&mut self, other: &Self, transform: Affine, clip: Rect) {
        for rect in &other.rects {
            let transformed = transform.transform_rect(*rect);
            if let Some(clipped) = intersect_rect(transformed, clip) {
                self.add(clipped);
            }
        }
    }

    pub fn tiles(&self, scale: f32, tile_size: u32) -> HashSet<(i32, i32)> {
        let tile_size = tile_size.max(1) as i32;
        let mut tiles = HashSet::new();
        for rect in &self.rects {
            let left = (rect.x * scale).floor() as i32;
            let top = (rect.y * scale).floor() as i32;
            let right = ((rect.x + rect.width) * scale).ceil() as i32;
            let bottom = ((rect.y + rect.height) * scale).ceil() as i32;
            if right <= left || bottom <= top {
                continue;
            }
            let x0 = left.div_euclid(tile_size);
            let y0 = top.div_euclid(tile_size);
            let x1 = (right - 1).div_euclid(tile_size);
            let y1 = (bottom - 1).div_euclid(tile_size);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    tiles.insert((x, y));
                }
            }
        }
        tiles
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayerCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub resident_bytes: u64,
    pub entries: usize,
    pub resident_tiles: usize,
    pub partial_updates: u64,
    pub dirty_regions: usize,
    pub dirty_tiles: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct LayerCacheBook {
    entries: HashMap<LayerCacheId, LayerCacheEntry>,
    snapshots: HashMap<RenderNodeId, LayerSnapshot>,
    frame: u64,
    hits: u64,
    misses: u64,
    partial_updates: u64,
    dirty_regions: usize,
    dirty_tiles: usize,
    pub(super) last_dirty: HashMap<RenderNodeId, BackendDirtyRegion>,
}

impl LayerCacheBook {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.snapshots.clear();
        self.last_dirty.clear();
    }

    pub fn update(
        &mut self,
        frame: &BuiltFrame,
        scale: f32,
        _auto_budget: u64,
        tile_size: u32,
    ) -> BackendDirtyRegion {
        self.frame = self.frame.wrapping_add(1).max(1);
        self.dirty_regions = 0;
        self.dirty_tiles = 0;
        self.entries
            .retain(|id, _| frame.live_layer_caches.contains(id));

        let mut next_snapshots = HashMap::new();
        let mut dirty_by_layer = HashMap::new();

        for layer in frame.layers.iter().rev() {
            let next_snapshot = layer_snapshot(frame, layer);
            next_snapshots.insert(layer.source, next_snapshot);

            let mut dirty = match self.snapshots.get(&layer.source) {
                Some(previous) => {
                    diff_layer(previous, &next_snapshots[&layer.source], &dirty_by_layer)
                }
                None => BackendDirtyRegion::full(layer.render_bounds),
            };

            let expansion = layer
                .effects
                .iter()
                .map(LayerEffect::visual_expansion)
                .sum();
            dirty.expand(expansion);
            if !dirty.rects.is_empty() {
                self.dirty_regions += dirty.rects.len();
                self.dirty_tiles += dirty.tiles(scale, tile_size).len();
                if self.snapshots.contains_key(&layer.source) {
                    self.partial_updates += 1;
                }
            }
            dirty_by_layer.insert(layer.source, dirty);
        }
        self.snapshots = next_snapshots;
        self.last_dirty = dirty_by_layer.clone();

        for layer in &frame.layers {
            if layer.cache_policy == CachePolicy::None {
                continue;
            }
            let Some(id) = layer.cache_id else { continue };
            let width = (layer.render_bounds.width.max(0.0) * scale).ceil() as u32;
            let height = (layer.render_bounds.height.max(0.0) * scale).ceil() as u32;
            let version = LayerCacheVersion {
                content: layer.content_version,
                width,
                height,
                scale_bits: scale.to_bits(),
            };
            let bytes = width as u64 * height as u64 * 8;
            let dirty = dirty_by_layer
                .get(&layer.source)
                .cloned()
                .unwrap_or_default();
            let dirty_tiles = dirty.tiles(scale, tile_size);
            match self.entries.get_mut(&id) {
                Some(entry) if entry.version == version => {
                    self.hits += 1;
                    entry.last_used = self.frame;
                    entry.policy = layer.cache_policy;
                    entry.dirty = dirty;
                    entry.dirty_tiles = dirty_tiles;
                }
                Some(entry) => {
                    self.misses += 1;
                    *entry = LayerCacheEntry {
                        version,
                        bytes,
                        last_used: self.frame,
                        policy: layer.cache_policy,
                        dirty,
                        dirty_tiles,
                    };
                }
                None => {
                    self.misses += 1;
                    self.entries.insert(
                        id,
                        LayerCacheEntry {
                            version,
                            bytes,
                            last_used: self.frame,
                            policy: layer.cache_policy,
                            dirty,
                            dirty_tiles,
                        },
                    );
                }
            }
        }
        dirty_by_layer
            .remove(&frame.layers[frame.root_layer.0].source)
            .unwrap_or_default()
    }

    pub fn stats(&self) -> LayerCacheStats {
        LayerCacheStats {
            hits: self.hits,
            misses: self.misses,
            resident_bytes: self.entries.values().map(|entry| entry.bytes).sum(),
            entries: self.entries.len(),
            resident_tiles: 0,
            partial_updates: self.partial_updates,
            dirty_regions: self.dirty_regions,
            dirty_tiles: self.dirty_tiles,
        }
    }
}

#[derive(Default)]
pub(super) struct LayerTileCache {
    pub(super) layers: HashMap<LayerCacheId, ResidentLayer>,
    frame: u64,
}

impl LayerTileCache {
    pub fn clear(&mut self) {
        self.layers.clear();
    }

    pub fn begin_frame(&mut self, frame: &BuiltFrame) {
        self.frame = self.frame.wrapping_add(1).max(1);
        self.layers.retain(|key, layer| {
            layer.policy == CachePolicy::None || frame.live_layer_caches.contains(key)
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ensure_tile(
        &mut self,
        device: &wgpu::Device,
        ui_layout: &wgpu::BindGroupLayout,
        tile_renderer: &LayerTileRenderer,
        layer: &xui::render::BuiltLayer,
        coord: (i32, i32),
        scale_factor: f32,
        tile_size: u32,
    ) -> bool {
        let key = layer
            .cache_id
            .expect("every isolated built layer has a cache identity");
        let storage = LayerTileStorageVersion {
            render_bounds: layer.render_bounds,
            effects: Arc::clone(&layer.effects),
            scale_bits: scale_factor.to_bits(),
            tile_size,
        };
        let resident = self.layers.entry(key).or_insert_with(|| ResidentLayer {
            source: layer.source,
            policy: layer.cache_policy,
            storage: storage.clone(),
            tiles: HashMap::new(),
        });
        if resident.storage != storage || resident.source != layer.source {
            resident.tiles.clear();
            resident.storage = storage;
            resident.source = layer.source;
        }
        resident.policy = layer.cache_policy;
        if let Some(tile) = resident.tiles.get_mut(&coord) {
            tile.last_used = self.frame;
            return false;
        }

        let logical_tile_size = tile_size.max(1) as f32 / scale_factor;
        let grid_bounds = Rect::new(
            coord.0 as f32 * logical_tile_size,
            coord.1 as f32 * logical_tile_size,
            logical_tile_size,
            logical_tile_size,
        );
        let Some(inner_bounds) = intersect_rect(grid_bounds, layer.render_bounds) else {
            return false;
        };
        let padding = layer
            .effects
            .iter()
            .map(LayerEffect::visual_expansion)
            .sum::<f32>();
        let padding_px = (padding * scale_factor).ceil().max(0.0) as u32;
        let inner_width = (inner_bounds.width * scale_factor).ceil().max(1.0) as u32;
        let inner_height = (inner_bounds.height * scale_factor).ceil().max(1.0) as u32;
        let target_size = (
            inner_width.saturating_add(padding_px.saturating_mul(2)),
            inner_height.saturating_add(padding_px.saturating_mul(2)),
        );
        let logical_padding = padding_px as f32 / scale_factor;
        let target_origin = Point::new(
            inner_bounds.x - logical_padding,
            inner_bounds.y - logical_padding,
        );
        let texture = |label: &'static str, sample_count, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: target_size.0,
                    height: target_size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: SCENE_FORMAT,
                usage,
                view_formats: &[],
            })
        };
        let textures = [
            texture(
                "xui layer tile ping",
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
            texture(
                "xui layer tile pong",
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
        ];
        let views = [
            textures[0].create_view(&wgpu::TextureViewDescriptor::default()),
            textures[1].create_view(&wgpu::TextureViewDescriptor::default()),
        ];
        let msaa_texture = texture(
            "xui layer tile msaa",
            SCENE_SAMPLE_COUNT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let msaa_view = msaa_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let ui_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("xui layer tile ui uniforms"),
            contents: bytemuck::bytes_of(&UiUniforms {
                viewport_size: [target_size.0 as f32, target_size.1 as f32, 0.0, 0.0],
                scale_factor: [scale_factor; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let ui_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui layer tile ui bind group"),
            layout: ui_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ui_uniform_buffer.as_entire_binding(),
            }],
        });
        let final_index = layer_effect_final_index(&layer.effects);
        let composite_bind_group = tile_renderer.create_bind_group(device, &views[final_index]);
        let inner_uv = Rect::new(
            padding_px as f32 / target_size.0 as f32,
            padding_px as f32 / target_size.1 as f32,
            inner_width as f32 / target_size.0 as f32,
            inner_height as f32 / target_size.1 as f32,
        );
        let pixels = target_size.0 as u64 * target_size.1 as u64;
        resident.tiles.insert(
            coord,
            ResidentTile {
                _textures: textures,
                views,
                _msaa_texture: msaa_texture,
                msaa_view,
                _ui_uniform_buffer: ui_uniform_buffer,
                ui_bind_group,
                composite_bind_group,
                inner_bounds,
                target_origin,
                target_size,
                inner_uv,
                final_index,
                bytes: pixels * 8 * (2 + SCENE_SAMPLE_COUNT as u64),
                last_used: self.frame,
                valid: false,
            },
        );
        true
    }

    pub(super) fn finish_frame(&mut self, auto_budget: u64) {
        self.layers
            .retain(|_, layer| layer.policy != CachePolicy::None);
        while self.auto_bytes() > auto_budget {
            let candidate = self
                .layers
                .iter()
                .filter(|(_, layer)| layer.policy == CachePolicy::Auto)
                .flat_map(|(key, layer)| {
                    layer
                        .tiles
                        .iter()
                        .map(move |(coord, tile)| (*key, *coord, tile.last_used))
                })
                .min_by_key(|(_, _, last_used)| *last_used);
            let Some((key, coord, _)) = candidate else {
                break;
            };
            if let Some(layer) = self.layers.get_mut(&key) {
                layer.tiles.remove(&coord);
            }
        }
        self.layers.retain(|_, layer| !layer.tiles.is_empty());
    }

    fn auto_bytes(&self) -> u64 {
        self.layers
            .values()
            .filter(|layer| layer.policy == CachePolicy::Auto)
            .flat_map(|layer| layer.tiles.values())
            .map(|tile| tile.bytes)
            .sum()
    }

    pub(super) fn resident_bytes(&self) -> u64 {
        self.layers
            .values()
            .flat_map(|layer| layer.tiles.values())
            .map(|tile| tile.bytes)
            .sum()
    }

    pub(super) fn tile_count(&self) -> usize {
        self.layers.values().map(|layer| layer.tiles.len()).sum()
    }
}
