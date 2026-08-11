use rustc_hash::FxHashMap;
use xui::render::{BuiltLayerId, CachePolicy, LayerCacheId, RenderNodeId};
use xui_interface::Rect;
use xui_render_graph::PixelRect;

use super::{TextureLease, TexturePool, TexturePoolError, TextureRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct TileCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum SurfaceKey {
    Root,
    Layer(LayerCacheId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SurfaceGeometry {
    pub physical_bounds: PixelRect,
    pub tile_size: u32,
    pub scale_bits: u32,
}

impl SurfaceGeometry {
    pub fn new(bounds: Rect, scale_factor: f32, tile_size: u32) -> Option<Self> {
        let physical_bounds = physical_rect(bounds, scale_factor)?;
        Some(Self {
            physical_bounds,
            tile_size: tile_size.max(1),
            scale_bits: scale_factor.to_bits(),
        })
    }

    pub fn scale_factor(self) -> f32 {
        f32::from_bits(self.scale_bits)
    }

    pub fn tile_rect(self, coord: TileCoord) -> Option<PixelRect> {
        if coord.x < 0 || coord.y < 0 {
            return None;
        }
        let offset_x = u32::try_from(coord.x).ok()?.checked_mul(self.tile_size)?;
        let offset_y = u32::try_from(coord.y).ok()?.checked_mul(self.tile_size)?;
        if offset_x >= self.physical_bounds.width || offset_y >= self.physical_bounds.height {
            return None;
        }
        let x = self
            .physical_bounds
            .x
            .checked_add(i32::try_from(offset_x).ok()?)?;
        let y = self
            .physical_bounds
            .y
            .checked_add(i32::try_from(offset_y).ok()?)?;
        Some(PixelRect {
            x,
            y,
            width: self
                .tile_size
                .min(self.physical_bounds.width.saturating_sub(offset_x)),
            height: self
                .tile_size
                .min(self.physical_bounds.height.saturating_sub(offset_y)),
        })
    }

    pub fn coords_for_rect(self, bounds: Rect, guard_tiles: u32) -> Vec<TileCoord> {
        let Some(mut demand) = physical_rect(bounds, self.scale_factor()) else {
            return Vec::new();
        };
        let guard = guard_tiles.saturating_mul(self.tile_size);
        demand = expand_pixel_rect(demand, guard);
        let Some(demand) = intersect_pixel_rect(demand, self.physical_bounds) else {
            return Vec::new();
        };

        let local_left = i64::from(demand.x) - i64::from(self.physical_bounds.x);
        let local_top = i64::from(demand.y) - i64::from(self.physical_bounds.y);
        let local_right = local_left + i64::from(demand.width);
        let local_bottom = local_top + i64::from(demand.height);
        let tile = i64::from(self.tile_size);
        let x0 = local_left.div_euclid(tile);
        let y0 = local_top.div_euclid(tile);
        let x1 = (local_right - 1).div_euclid(tile);
        let y1 = (local_bottom - 1).div_euclid(tile);
        let mut result = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (Ok(x), Ok(y)) = (i32::try_from(x), i32::try_from(y)) else {
                    continue;
                };
                result.push(TileCoord { x, y });
            }
        }
        result
    }
}

pub(super) struct Tile {
    pub texture: TextureLease,
    pub physical_bounds: PixelRect,
    pub valid: bool,
    pub last_used: u64,
    pub keepalive_frame: u64,
}

impl Tile {
    pub fn logical_bounds(&self, scale_factor: f32) -> Rect {
        logical_rect(self.physical_bounds, scale_factor)
    }

    pub fn bytes(&self) -> u64 {
        self.texture.bytes()
    }
}

pub(super) struct TiledSurface {
    pub layer: BuiltLayerId,
    pub source: RenderNodeId,
    pub policy: CachePolicy,
    pub geometry: SurfaceGeometry,
    pub tiles: FxHashMap<TileCoord, Tile>,
}

impl TiledSurface {
    pub fn new(
        layer: BuiltLayerId,
        source: RenderNodeId,
        policy: CachePolicy,
        geometry: SurfaceGeometry,
    ) -> Self {
        Self {
            layer,
            source,
            policy,
            geometry,
            tiles: FxHashMap::default(),
        }
    }

    pub fn ensure_tile(
        &mut self,
        device: &wgpu::Device,
        pool: &TexturePool,
        coord: TileCoord,
        frame: u64,
        keepalive: bool,
    ) -> Result<bool, TexturePoolError> {
        if let Some(tile) = self.tiles.get_mut(&coord) {
            tile.last_used = frame;
            if keepalive {
                tile.keepalive_frame = frame;
            }
            return Ok(false);
        }
        let Some(physical_bounds) = self.geometry.tile_rect(coord) else {
            return Ok(false);
        };
        let texture = pool.acquire(
            device,
            TextureRequest::tile(self.geometry.tile_size, "xui tiled surface tile"),
        )?;
        self.tiles.insert(
            coord,
            Tile {
                texture,
                physical_bounds,
                valid: false,
                last_used: frame,
                keepalive_frame: keepalive.then_some(frame).unwrap_or(0),
            },
        );
        Ok(true)
    }
}

pub(super) struct TemporarySurface {
    pub texture: TextureLease,
    pub physical_bounds: PixelRect,
    pub logical_bounds: Rect,
}

impl TemporarySurface {
    pub fn new(
        device: &wgpu::Device,
        pool: &TexturePool,
        physical_bounds: PixelRect,
        scale_factor: f32,
        label: &'static str,
    ) -> Result<Self, TexturePoolError> {
        let texture = pool.acquire(
            device,
            TextureRequest::scene(
                (physical_bounds.width.max(1), physical_bounds.height.max(1)),
                label,
            ),
        )?;
        Ok(Self {
            texture,
            physical_bounds,
            logical_bounds: logical_rect(physical_bounds, scale_factor),
        })
    }

    pub fn extent(&self) -> (u32, u32) {
        (self.physical_bounds.width, self.physical_bounds.height)
    }
}

pub(super) struct SharedTileTarget {
    pub msaa: TextureLease,
    pub resolve_scratch: TextureLease,
    pub extent: (u32, u32),
    tile_size: u32,
}

impl SharedTileTarget {
    pub fn new(
        device: &wgpu::Device,
        pool: &TexturePool,
        tile_size: u32,
    ) -> Result<Self, TexturePoolError> {
        let tile_size = tile_size.max(1);
        let resolve_scratch = pool.acquire(
            device,
            TextureRequest::tile(tile_size, "xui shared tile resolve scratch"),
        )?;
        let msaa = pool.acquire(
            device,
            TextureRequest::tile_msaa(tile_size, "xui shared tile msaa"),
        )?;
        let allocation = resolve_scratch.allocation_extent();
        debug_assert_eq!(allocation.width, msaa.allocation_extent().width);
        debug_assert_eq!(allocation.height, msaa.allocation_extent().height);
        Ok(Self {
            msaa,
            resolve_scratch,
            extent: (allocation.width, allocation.height),
            tile_size,
        })
    }

    pub fn matches(&self, tile_size: u32) -> bool {
        self.tile_size == tile_size.max(1)
    }
}

pub(super) struct FrameTarget<'a> {
    pub size: (u32, u32),
    pub view: &'a wgpu::TextureView,
}

pub(super) fn physical_rect(bounds: Rect, scale_factor: f32) -> Option<PixelRect> {
    if !scale_factor.is_finite()
        || scale_factor <= 0.0
        || !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.width <= 0.0
        || bounds.height <= 0.0
    {
        return None;
    }
    let left = (bounds.x * scale_factor).floor() as f64;
    let top = (bounds.y * scale_factor).floor() as f64;
    let right = ((bounds.x + bounds.width) * scale_factor).ceil() as f64;
    let bottom = ((bounds.y + bounds.height) * scale_factor).ceil() as f64;
    if left < i32::MIN as f64
        || top < i32::MIN as f64
        || right > i32::MAX as f64
        || bottom > i32::MAX as f64
        || right <= left
        || bottom <= top
    {
        return None;
    }
    Some(PixelRect {
        x: left as i32,
        y: top as i32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

pub(super) fn logical_rect(bounds: PixelRect, scale_factor: f32) -> Rect {
    Rect::new(
        bounds.x as f32 / scale_factor,
        bounds.y as f32 / scale_factor,
        bounds.width as f32 / scale_factor,
        bounds.height as f32 / scale_factor,
    )
}

pub(super) fn intersect_pixel_rect(a: PixelRect, b: PixelRect) -> Option<PixelRect> {
    let left = i64::from(a.x).max(i64::from(b.x));
    let top = i64::from(a.y).max(i64::from(b.y));
    let right = (i64::from(a.x) + i64::from(a.width)).min(i64::from(b.x) + i64::from(b.width));
    let bottom = (i64::from(a.y) + i64::from(a.height)).min(i64::from(b.y) + i64::from(b.height));
    (right > left && bottom > top).then(|| PixelRect {
        x: left as i32,
        y: top as i32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

fn expand_pixel_rect(rect: PixelRect, amount: u32) -> PixelRect {
    let amount = i64::from(amount);
    let left = (i64::from(rect.x) - amount).max(i64::from(i32::MIN));
    let top = (i64::from(rect.y) - amount).max(i64::from(i32::MIN));
    let right = (i64::from(rect.x) + i64::from(rect.width) + amount).min(i64::from(i32::MAX));
    let bottom = (i64::from(rect.y) + i64::from(rect.height) + amount).min(i64::from(i32::MAX));
    PixelRect {
        x: left as i32,
        y: top as i32,
        width: (right - left).max(0) as u32,
        height: (bottom - top).max(0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_bounds_are_pixel_aligned_before_tiling() {
        let geometry =
            SurfaceGeometry::new(Rect::new(-1.25, 2.25, 520.5, 257.0), 2.0, 256).unwrap();
        assert_eq!(
            geometry.physical_bounds,
            PixelRect {
                x: -3,
                y: 4,
                width: 1042,
                height: 515,
            }
        );
        assert_eq!(
            geometry.tile_rect(TileCoord { x: 4, y: 2 }),
            Some(PixelRect {
                x: 1021,
                y: 516,
                width: 18,
                height: 3,
            })
        );
    }

    #[test]
    fn tile_grid_is_local_to_negative_picture_origin() {
        let geometry =
            SurfaceGeometry::new(Rect::new(-300.0, -20.0, 600.0, 40.0), 1.0, 256).unwrap();
        assert_eq!(
            geometry.coords_for_rect(Rect::new(-1.0, -1.0, 2.0, 2.0), 0),
            vec![TileCoord { x: 1, y: 0 }]
        );
        assert_eq!(
            geometry.coords_for_rect(Rect::new(-1.0, -1.0, 2.0, 2.0), 1),
            vec![
                TileCoord { x: 0, y: 0 },
                TileCoord { x: 1, y: 0 },
                TileCoord { x: 2, y: 0 },
            ]
        );
    }

    #[test]
    fn expansion_is_applied_to_the_whole_picture_not_each_tile() {
        let geometry = SurfaceGeometry::new(Rect::new(90.0, 80.0, 320.0, 290.0), 1.0, 128).unwrap();
        assert_eq!(
            geometry.tile_rect(TileCoord { x: 0, y: 0 }).unwrap().width,
            128
        );
        assert_eq!(
            geometry.tile_rect(TileCoord { x: 2, y: 2 }).unwrap().width,
            64
        );
        assert_eq!(
            geometry.tile_rect(TileCoord { x: 2, y: 2 }).unwrap().height,
            34
        );
    }
}
