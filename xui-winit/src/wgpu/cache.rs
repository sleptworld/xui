use crate::wgpu::{
    LayerSnapshot, TexturePool, TexturePoolError, intersect_rect,
    layer::diff_layer,
    snapshot::layer_snapshot,
    tex::{SurfaceGeometry, SurfaceKey, TileCoord, TiledSurface},
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{HashMap, HashSet};
use xui::render::{
    BuiltFrame, BuiltItem, BuiltLayerId, CachePolicy, ContentVersion, LayerCacheId, RenderNodeId,
};
use xui_interface::{Affine, Rect};
use xui_render_graph::SampleExpansion;

fn layer_effect_expansions(frame: &BuiltFrame) -> HashMap<RenderNodeId, SampleExpansion> {
    let mut expansions = HashMap::<RenderNodeId, SampleExpansion>::new();
    for parent in &frame.layers {
        for item in &parent.items {
            let BuiltItem::Layer(instance_id) = item else {
                continue;
            };
            let instance = frame.layer_instance(*instance_id).expect("built instance");
            let child = &frame.layers[instance.layer.0];
            let expansion = instance.render_program.program().layer_visual_expansion();
            expansions
                .entry(child.source)
                .and_modify(|current| {
                    current.left = current.left.max(expansion.left);
                    current.top = current.top.max(expansion.top);
                    current.right = current.right.max(expansion.right);
                    current.bottom = current.bottom.max(expansion.bottom);
                })
                .or_insert(expansion);
        }
    }
    expansions
}

fn propagate_composite_prefix_damage(
    frame: &BuiltFrame,
    dirty_by_layer: &mut HashMap<RenderNodeId, BackendDirtyRegion>,
) {
    let mut additions = Vec::new();
    for parent in &frame.layers {
        for item in &parent.items {
            let BuiltItem::Layer(instance_id) = item else {
                continue;
            };
            let instance = frame.layer_instance(*instance_id).expect("built instance");
            let Some(mut prefix) = instance.destination_prefix else {
                continue;
            };
            let mut chain = Vec::new();
            while let Some(node) = frame.composite_prefix(prefix).copied() {
                chain.push(node);
                let Some(parent_prefix) = node.parent else {
                    break;
                };
                prefix = parent_prefix;
            }
            chain.reverse();

            let target_expansion = instance.render_program.program().backdrop_input_expansion();
            for ancestor_index in 0..chain.len().saturating_sub(1) {
                let ancestor = frame.layers[chain[ancestor_index].local.layer.0].source;
                let Some(mut dirty) = dirty_by_layer.get(&ancestor).cloned() else {
                    continue;
                };
                for node in &chain[ancestor_index + 1..] {
                    let Some(placement_id) = node.placement else {
                        dirty = BackendDirtyRegion::default();
                        break;
                    };
                    let placement = frame
                        .layer_instance(placement_id)
                        .expect("prefix placement exists");
                    dirty = dirty.through_prefix_placement(
                        placement.composite.transform,
                        placement.world_bounds,
                        placement
                            .render_program
                            .program()
                            .backdrop_input_expansion(),
                        frame.layers[node.local.layer.0].render_bounds,
                    );
                }
                let affected = dirty.backdrop_damage(instance.world_bounds, target_expansion);
                if !affected.rects.is_empty() {
                    additions.push((parent.source, affected));
                }
            }
        }
    }
    for (source, damage) in additions {
        dirty_by_layer.entry(source).or_default().extend(damage);
    }

    // Cross-surface backdrop damage becomes ordinary child output damage from
    // this point upward. Re-run the hierarchy in child-to-parent order so the
    // newly added regions reach the root in the same frame.
    for parent in frame.layers.iter().rev() {
        let mut propagated = BackendDirtyRegion::default();
        for item in &parent.items {
            let BuiltItem::Layer(instance_id) = item else {
                continue;
            };
            let instance = frame.layer_instance(*instance_id).expect("built instance");
            let child = &frame.layers[instance.layer.0];
            if let Some(dirty) = dirty_by_layer.get(&child.source) {
                propagated.add_transformed(
                    dirty,
                    instance.composite.transform,
                    instance.world_bounds,
                );
            }
        }
        dirty_by_layer
            .entry(parent.source)
            .or_default()
            .extend(propagated);
    }
}

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

    pub fn backdrop_damage(&self, output_bounds: Rect, expansion: SampleExpansion) -> Self {
        let mut damage = Self::default();
        for rect in &self.rects {
            let expanded = Rect::new(
                rect.x - expansion.left,
                rect.y - expansion.top,
                rect.width + expansion.left + expansion.right,
                rect.height + expansion.top + expansion.bottom,
            );
            if let Some(affected) = intersect_rect(expanded, output_bounds) {
                damage.add(affected);
            }
        }
        damage
    }

    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().copied().reduce(Rect::union)
    }

    pub fn intersects(&self, bounds: Rect) -> bool {
        self.rects.iter().any(|rect| rect.intersects(bounds))
    }

    pub fn expand_sample(&mut self, expansion: SampleExpansion) {
        for rect in &mut self.rects {
            *rect = Rect::new(
                rect.x - expansion.left,
                rect.y - expansion.top,
                rect.width + expansion.left + expansion.right,
                rect.height + expansion.top + expansion.bottom,
            );
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

    fn through_prefix_placement(
        &self,
        child_to_parent: Affine,
        parent_clip: Rect,
        expansion: SampleExpansion,
        child_clip: Rect,
    ) -> Self {
        let Some(parent_to_child) = inverse_affine(child_to_parent) else {
            return Self::default();
        };
        let mut result = Self::default();
        for rect in &self.rects {
            let expanded = Rect::new(
                rect.x - expansion.left,
                rect.y - expansion.top,
                rect.width + expansion.left + expansion.right,
                rect.height + expansion.top + expansion.bottom,
            );
            let Some(parent_visible) = intersect_rect(expanded, parent_clip) else {
                continue;
            };
            let child = parent_to_child.transform_rect(parent_visible);
            if let Some(child) = intersect_rect(child, child_clip) {
                result.add(child);
            }
        }
        result
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

fn inverse_affine(value: Affine) -> Option<Affine> {
    let determinant = value.xx * value.yy - value.xy * value.yx;
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    let inverse = determinant.recip();
    let xx = value.yy * inverse;
    let yx = -value.yx * inverse;
    let xy = -value.xy * inverse;
    let yy = value.xx * inverse;
    let dx = -(xx * value.dx + xy * value.dy);
    let dy = -(yx * value.dx + yy * value.dy);
    Some(Affine::new(xx, yx, xy, yy, dx, dy))
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
        let effect_expansions = layer_effect_expansions(frame);

        for layer in frame.layers.iter().rev() {
            let next_snapshot = layer_snapshot(frame, layer);
            next_snapshots.insert(layer.source, next_snapshot);

            let mut dirty = match self.snapshots.get(&layer.source) {
                Some(previous) => {
                    diff_layer(previous, &next_snapshots[&layer.source], &dirty_by_layer)
                }
                None => BackendDirtyRegion::full(layer.render_bounds),
            };

            dirty.expand_sample(
                effect_expansions
                    .get(&layer.source)
                    .copied()
                    .unwrap_or(SampleExpansion::ZERO),
            );
            if !dirty.rects.is_empty() && self.snapshots.contains_key(&layer.source) {
                self.partial_updates += 1;
            }
            dirty_by_layer.insert(layer.source, dirty);
        }
        propagate_composite_prefix_damage(frame, &mut dirty_by_layer);
        self.dirty_regions = dirty_by_layer.values().map(|dirty| dirty.rects.len()).sum();
        self.dirty_tiles = dirty_by_layer
            .values()
            .map(|dirty| dirty.tiles(scale, tile_size).len())
            .sum();
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
pub(super) struct SurfaceCache {
    pub(super) surfaces: FxHashMap<SurfaceKey, TiledSurface>,
    frame: u64,
}

impl SurfaceCache {
    pub fn clear(&mut self) {
        self.surfaces.clear();
    }

    pub fn begin_frame(&mut self, frame: &BuiltFrame, scale_factor: f32, tile_size: u32) {
        self.frame = self.frame.wrapping_add(1).max(1);
        let live: FxHashSet<_> = frame
            .layers
            .iter()
            .filter_map(|layer| layer.cache_id.map(SurfaceKey::Layer))
            .chain([SurfaceKey::Root])
            .collect();
        self.surfaces.retain(|key, _| live.contains(key));

        for (index, layer) in frame.layers.iter().enumerate() {
            let layer_id = BuiltLayerId(index);
            let key = if layer_id == frame.root_layer {
                SurfaceKey::Root
            } else {
                SurfaceKey::Layer(
                    layer
                        .cache_id
                        .expect("every isolated built layer has a cache identity"),
                )
            };
            let Some(geometry) = SurfaceGeometry::new(layer.render_bounds, scale_factor, tile_size)
            else {
                self.surfaces.remove(&key);
                continue;
            };
            let policy = if key == SurfaceKey::Root {
                CachePolicy::None
            } else {
                layer.cache_policy
            };
            match self.surfaces.get_mut(&key) {
                Some(surface) if surface.source == layer.source && surface.geometry == geometry => {
                    surface.layer = layer_id;
                    surface.policy = policy;
                }
                _ => {
                    self.surfaces.insert(
                        key,
                        TiledSurface::new(layer_id, layer.source, policy, geometry),
                    );
                }
            }
        }
    }

    pub fn ensure_tile(
        &mut self,
        device: &wgpu::Device,
        pool: &TexturePool,
        key: SurfaceKey,
        coord: TileCoord,
        keepalive: bool,
    ) -> Result<bool, TexturePoolError> {
        self.surfaces
            .get_mut(&key)
            .expect("surface cache initialized from BuiltFrame")
            .ensure_tile(device, pool, coord, self.frame, keepalive)
    }

    pub fn can_allocate_guard(&self, key: SurfaceKey, auto_budget: u64) -> bool {
        let Some(surface) = self.surfaces.get(&key) else {
            return false;
        };
        if surface.policy != CachePolicy::Auto {
            return true;
        }
        let tile_bytes = surface
            .tiles
            .values()
            .next()
            .map(|tile| tile.bytes())
            .unwrap_or_else(|| {
                let side = u64::from(surface.geometry.tile_size);
                side.saturating_mul(side).saturating_mul(8)
            });
        self.auto_bytes().saturating_add(tile_bytes) <= auto_budget
    }

    pub fn evict_lru_auto_unpinned(&mut self) -> bool {
        let now = self.frame;
        let candidate = self
            .surfaces
            .iter()
            .filter(|(_, surface)| surface.policy == CachePolicy::Auto)
            .flat_map(|(key, surface)| {
                surface
                    .tiles
                    .iter()
                    .filter(move |(_, tile)| tile.keepalive_frame != now)
                    .map(move |(coord, tile)| (*key, *coord, tile.last_used))
            })
            .min_by_key(|(_, _, last_used)| *last_used);
        let Some((key, coord, _)) = candidate else {
            return false;
        };
        self.surfaces
            .get_mut(&key)
            .and_then(|surface| surface.tiles.remove(&coord))
            .is_some()
    }

    pub(super) fn finish_frame(&mut self, auto_budget: u64) {
        while self.auto_bytes() > auto_budget {
            if !self.evict_lru_auto_unpinned() {
                break;
            }
        }

        let now = self.frame;
        for (key, surface) in &mut self.surfaces {
            if *key == SurfaceKey::Root || surface.policy != CachePolicy::None {
                continue;
            }
            surface.tiles.retain(|_, tile| tile.keepalive_frame == now);
        }
    }

    fn auto_bytes(&self) -> u64 {
        self.surfaces
            .values()
            .filter(|surface| surface.policy == CachePolicy::Auto)
            .flat_map(|surface| surface.tiles.values())
            .map(|tile| tile.bytes())
            .sum()
    }

    pub(super) fn resident_bytes(&self) -> u64 {
        self.surfaces
            .values()
            .flat_map(|surface| surface.tiles.values())
            .map(|tile| tile.bytes())
            .sum()
    }

    pub(super) fn tile_count(&self) -> usize {
        self.surfaces
            .values()
            .map(|surface| surface.tiles.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_damage_is_expanded_then_mapped_into_child_space() {
        let dirty = BackendDirtyRegion::full(Rect::new(14.0, 20.0, 4.0, 6.0));
        let mapped = dirty.through_prefix_placement(
            Affine::new(2.0, 0.0, 0.0, 2.0, 10.0, 10.0),
            Rect::new(0.0, 0.0, 100.0, 100.0),
            SampleExpansion {
                left: 2.0,
                top: 4.0,
                right: 6.0,
                bottom: 8.0,
            },
            Rect::new(-100.0, -100.0, 200.0, 200.0),
        );
        assert_eq!(mapped.rects, vec![Rect::new(1.0, 3.0, 6.0, 9.0)]);
    }
}
