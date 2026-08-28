use std::collections::{HashMap, HashSet};

use skia_safe::{
    AlphaType, ColorSpace, ColorType, ImageInfo, Surface,
    gpu::{self, DirectContext},
    surfaces,
};
use xui::render::{BuiltLayer, CachePolicy, LayerCacheId, RenderNodeId};
use xui_interface::Bounds;

use crate::SkiaBackendError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkiaLayerCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub resident_bytes: u64,
    pub partial_updates: u64,
    pub full_updates: u64,
    pub dirty_regions: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SurfaceGeometry {
    bounds: Bounds,
    width: u32,
    height: u32,
    scale_bits: u32,
}

impl SurfaceGeometry {
    fn new(bounds: Bounds, scale: f32) -> Self {
        Self {
            bounds,
            width: (bounds.width().max(0.0) * scale).ceil().max(1.0) as u32,
            height: (bounds.height().max(0.0) * scale).ceil().max(1.0) as u32,
            scale_bits: scale.to_bits(),
        }
    }

    fn bytes(self) -> u64 {
        u64::from(self.width)
            .saturating_mul(u64::from(self.height))
            .saturating_mul(4)
    }
}

struct CachedSurface {
    source: RenderNodeId,
    geometry: SurfaceGeometry,
    policy: CachePolicy,
    surface: Surface,
    last_used: u64,
}

pub(crate) struct SurfaceLease {
    pub(crate) surface: Surface,
    pub(crate) reused: bool,
    pub(crate) cache_id: Option<LayerCacheId>,
}

#[derive(Default)]
pub(crate) struct LayerSurfaceCache {
    entries: HashMap<LayerCacheId, CachedSurface>,
    frame: u64,
    hits: u64,
    misses: u64,
    partial_updates: u64,
    full_updates: u64,
    dirty_regions: usize,
}

impl LayerSurfaceCache {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn begin_frame(&mut self, dirty_regions: usize) {
        self.frame = self.frame.wrapping_add(1).max(1);
        self.dirty_regions = dirty_regions;
    }

    pub(crate) fn acquire(
        &mut self,
        layer: &BuiltLayer,
        scale: f32,
        gpu_context: Option<&mut DirectContext>,
    ) -> Result<SurfaceLease, SkiaBackendError> {
        let geometry = SurfaceGeometry::new(layer.render_bounds, scale);
        let cache_id = (layer.cache_policy != CachePolicy::None)
            .then_some(layer.cache_id)
            .flatten();
        if let Some(id) = cache_id {
            if let Some(entry) = self.entries.remove(&id)
                && entry.source == layer.source
                && entry.geometry == geometry
            {
                self.hits += 1;
                return Ok(SurfaceLease {
                    surface: entry.surface,
                    reused: true,
                    cache_id: Some(id),
                });
            }
            self.misses += 1;
        }
        let surface = if let Some(context) = gpu_context {
            let info = ImageInfo::new(
                (geometry.width as i32, geometry.height as i32),
                ColorType::BGRA8888,
                AlphaType::Premul,
                ColorSpace::new_srgb(),
            );
            gpu::surfaces::render_target(
                context,
                gpu::Budgeted::Yes,
                &info,
                None,
                gpu::SurfaceOrigin::TopLeft,
                None,
                false,
                false,
            )
        } else {
            surfaces::raster_n32_premul((geometry.width as i32, geometry.height as i32))
        }
        .ok_or(SkiaBackendError::SurfaceAllocation {
            width: geometry.width,
            height: geometry.height,
        })?;
        Ok(SurfaceLease {
            surface,
            reused: false,
            cache_id,
        })
    }

    pub(crate) fn release(&mut self, layer: &BuiltLayer, scale: f32, lease: SurfaceLease) {
        let Some(id) = lease.cache_id else {
            return;
        };
        self.entries.insert(
            id,
            CachedSurface {
                source: layer.source,
                geometry: SurfaceGeometry::new(layer.render_bounds, scale),
                policy: layer.cache_policy,
                surface: lease.surface,
                last_used: self.frame,
            },
        );
    }

    pub(crate) fn record_update(&mut self, partial: bool) {
        if partial {
            self.partial_updates += 1;
        } else {
            self.full_updates += 1;
        }
    }

    pub(crate) fn finish_frame(&mut self, live: &[LayerCacheId], auto_budget: u64) {
        let live: HashSet<_> = live.iter().copied().collect();
        self.entries.retain(|id, _| live.contains(id));
        while self.auto_bytes() > auto_budget {
            let candidate = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.policy == CachePolicy::Auto)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(id, _)| *id);
            let Some(id) = candidate else {
                break;
            };
            self.entries.remove(&id);
        }
    }

    pub(crate) fn stats(&self) -> SkiaLayerCacheStats {
        SkiaLayerCacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.entries.len(),
            resident_bytes: self
                .entries
                .values()
                .map(|entry| entry.geometry.bytes())
                .sum(),
            partial_updates: self.partial_updates,
            full_updates: self.full_updates,
            dirty_regions: self.dirty_regions,
        }
    }

    fn auto_bytes(&self) -> u64 {
        self.entries
            .values()
            .filter(|entry| entry.policy == CachePolicy::Auto)
            .map(|entry| entry.geometry.bytes())
            .sum()
    }
}
