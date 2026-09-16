//! Surfaces: allocating them, recycling them across frames, snapshotting them,
//! and getting the finished frame onto the window.

use skia_safe::{
    AlphaType, BlendMode as SkBlendMode, Canvas, ClipOp, ColorSpace, ColorType, IRect, ImageInfo,
    Paint, Region, SamplingOptions, Surface,
};
use std::{collections::HashMap, num::NonZeroU32};
use xui_interface::{Bounds, Rect, Size, TextBackend};

use super::{
    SkiaBackend,
    convert::physical_extent,
    image::{RasterImage, make_image_from_pixels},
};
use crate::{SkiaBackendError, damage::DamageRegion};

/// Offscreen surfaces recycled across frames, keyed by exact pixel extent.
///
/// Every filter pass in a render plan allocates a surface, uses it for one
/// frame and drops it. A plain effects list runs eight of those per frame, and
/// the extents repeat frame after frame because the scene geometry does. Skia's
/// resource cache recycles the underlying allocation, but each request still
/// builds a surface, a render target and a device on top of it.
///
/// A surface handed back here still holds the last frame's pixels, so `take`
/// clears it: a full-surface clear is a load-op on the GPU and a memset on the
/// CPU, both far cheaper than the allocation they replace.
/// How long an unwanted extent stays resident. Extents come and go as the
/// scene changes — a resized window, an effect that stopped being applied —
/// and without an upper bound the pool would hold every extent it ever saw
/// until it hit its budget.
const MAX_POOL_IDLE_FRAMES: u64 = 60;

struct PooledSurface {
    pub(super) surface: Surface,
    idle_since: u64,
}

#[derive(Default)]
pub(super) struct SurfacePool {
    idle: HashMap<(u32, u32), Vec<PooledSurface>>,
    pub(super) bytes: u64,
    frame: u64,
}

impl SurfacePool {
    pub(super) fn begin_frame(&mut self) {
        self.frame += 1;
        let Some(cutoff) = self.frame.checked_sub(MAX_POOL_IDLE_FRAMES) else {
            return;
        };
        let before = self.idle.len();
        self.idle.retain(|_, bucket| {
            bucket.retain(|entry| entry.idle_since >= cutoff);
            !bucket.is_empty()
        });
        if self.idle.len() != before {
            self.bytes = self
                .idle
                .values()
                .flatten()
                .map(|entry| surface_bytes(&entry.surface))
                .sum();
        }
    }

    pub(super) fn take(&mut self, width: u32, height: u32) -> Option<Surface> {
        let bucket = self.idle.get_mut(&(width, height))?;
        let mut surface = bucket.pop()?.surface;
        if bucket.is_empty() {
            self.idle.remove(&(width, height));
        }
        self.bytes = self.bytes.saturating_sub(surface_bytes(&surface));
        let canvas = surface.canvas();
        // Hand it back indistinguishable from a fresh allocation. Without
        // pooling, nothing that drew into an offscreen surface had to leave
        // the canvas tidy; a leftover clip would spare part of the previous
        // frame's pixels from the clear below, and a leftover transform would
        // displace whatever draws next.
        canvas.restore_to_count(1);
        canvas.reset_matrix();
        canvas.clear(skia_safe::Color::TRANSPARENT);
        Some(surface)
    }

    pub(super) fn put(&mut self, surface: Surface, budget: u64) {
        let bytes = surface_bytes(&surface);
        if self.bytes.saturating_add(bytes) > budget {
            return;
        }
        self.bytes += bytes;
        self.idle
            .entry((surface.width() as u32, surface.height() as u32))
            .or_default()
            .push(PooledSurface {
                surface,
                idle_since: self.frame,
            });
    }

    pub(super) fn clear(&mut self) {
        self.idle.clear();
        self.bytes = 0;
    }
}

fn surface_bytes(surface: &Surface) -> u64 {
    (surface.width() as u64).saturating_mul(surface.height() as u64) * 4
}

impl<T: TextBackend> SkiaBackend<T> {
    pub(super) fn ensure_surface(&mut self, logical: Size<f32>) -> Result<(), SkiaBackendError> {
        let width = (logical.width.max(0.0) * self.scale_factor).ceil().max(1.0) as u32;
        let height = (logical.height.max(0.0) * self.scale_factor)
            .ceil()
            .max(1.0) as u32;
        if self.gpu_context.is_some() {
            if self.frame_size_px != Size::new(width, height) {
                self.frame_size_px = Size::new(width, height);
                // Rebuilding a swapchain releases its images, so nothing may
                // still reference one. `raster` wraps the image acquired for
                // the current frame, and the layer cache holds offscreen
                // surfaces the presenter is about to free with the rest of the
                // context's GPU resources.
                self.raster = None;
                self.compositor = None;
                self.damage_tracker.clear();
                self.layer_cache.clear();
                self.surface_pool.clear();
                self.damage_history.clear();
                if let Some(presenter) = self.presenter.as_mut() {
                    presenter.resize(self.gpu_context.as_mut(), width, height)?;
                }
            }
            if self.compositor.is_none() {
                // Freshly allocated, so nothing on it is worth keeping and the
                // first frame after this has to repaint in full.
                self.compositor = Some(new_surface_px(width, height, self.gpu_context.as_mut())?);
                self.damage_tracker.clear();
            }
            return Ok(());
        }
        if self.frame_size_px != Size::new(width, height) || self.raster.is_none() {
            self.raster = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32));
            if self.raster.is_none() {
                return Err(SkiaBackendError::SurfaceAllocation { width, height });
            }
            self.frame_size_px = Size::new(width, height);
            self.damage_tracker.clear();
            self.layer_cache.clear();
            self.surface_pool.clear();
            self.damage_history.clear();
        }
        Ok(())
    }

    pub(super) fn new_surface(&mut self, bounds: Bounds) -> Result<Surface, SkiaBackendError> {
        let width = (bounds.width().max(0.0) * self.scale_factor)
            .ceil()
            .max(1.0) as u32;
        let height = (bounds.height().max(0.0) * self.scale_factor)
            .ceil()
            .max(1.0) as u32;
        self.new_surface_px(width, height)
    }

    pub(super) fn new_surface_px(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<Surface, SkiaBackendError> {
        if self.options.optimizations.surface_pool
            && let Some(surface) = self.surface_pool.take(width, height)
        {
            self.frame_stats.pooled_surface_reuses += 1;
            return Ok(surface);
        }
        self.frame_stats.offscreen_surface_allocations += 1;
        new_surface_px(width, height, self.gpu_context.as_mut())
    }

    /// Hands a surface back for reuse. Only safe once nothing holds a snapshot
    /// of it: drawing into a surface with a live snapshot makes Skia copy it.
    pub(super) fn recycle_surface(&mut self, surface: Surface) {
        if !self.options.optimizations.surface_pool {
            return;
        }
        self.surface_pool
            .put(surface, self.options.surface_pool_budget_bytes);
    }

    /// A fully transparent image covering `bounds`.
    ///
    /// Used for an empty mask and for the layer-content placeholder a
    /// backdrop-only pass never reads. Both used to allocate a full
    /// layer-sized surface, clear it and snapshot it just to express
    /// "nothing"; a 1x1 transparent pixel stretches to exactly the same
    /// result wherever it is drawn.
    pub(super) fn transparent_image(
        &mut self,
        bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        let image = match &self.empty_image {
            Some(image) => image.clone(),
            None => {
                let image = make_image_from_pixels(&[0, 0, 0, 0], 1, 1)?;
                self.empty_image = Some(image.clone());
                image
            }
        };
        Ok(RasterImage { image, bounds })
    }

    /// Copies the finished frame onto the acquired swapchain image.
    ///
    /// Whole-frame, because a swapchain image's previous contents are not
    /// guaranteed — only the *scene* drawing is incremental.
    pub(super) fn blit_to_swapchain(
        &mut self,
        source: &mut Surface,
    ) -> Result<(), SkiaBackendError> {
        // Make the scene's work visible to the read that follows it. Skia
        // orders this itself for a same-context read, but the presenters flush
        // only the swapchain surface, so state it rather than rely on it.
        if let Some(context) = self.gpu_context.as_mut() {
            context.flush_and_submit_surface(source, None);
        }
        let target = self.raster.as_mut().ok_or_else(|| {
            SkiaBackendError::InvalidFrame("no acquired swapchain image to present into".into())
        })?;
        blit_surface(source, target.canvas());
        Ok(())
    }

    pub(super) fn snapshot_target(&mut self, target: &mut Surface, bounds: Bounds) -> RasterImage {
        self.frame_stats.image_snapshots += 1;
        RasterImage {
            image: target.image_snapshot(),
            bounds,
        }
    }

    pub(super) fn snapshot_surface_output(
        &mut self,
        surface: &mut Surface,
        bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        let (width, height) = physical_extent(bounds, self.scale_factor);
        let image = surface
            .image_snapshot_with_bounds(IRect::new(0, 0, width as i32, height as i32))
            .ok_or(SkiaBackendError::SurfaceAllocation { width, height })?;
        self.frame_stats.image_snapshots += 1;
        Ok(RasterImage { image, bounds })
    }
}

pub(super) fn new_surface_px(
    width: u32,
    height: u32,
    gpu_context: Option<&mut skia_safe::gpu::DirectContext>,
) -> Result<Surface, SkiaBackendError> {
    if let Some(context) = gpu_context {
        let info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::BGRA8888,
            AlphaType::Premul,
            ColorSpace::new_srgb(),
        );
        return skia_safe::gpu::surfaces::render_target(
            context,
            skia_safe::gpu::Budgeted::Yes,
            &info,
            None,
            skia_safe::gpu::SurfaceOrigin::TopLeft,
            None,
            false,
            false,
        )
        .ok_or(SkiaBackendError::SurfaceAllocation { width, height });
    }
    skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
        .ok_or(SkiaBackendError::SurfaceAllocation { width, height })
}

pub(super) fn clear_surface_output(surface: &mut Surface, bounds: Bounds, scale: f32) {
    let (width, height) = physical_extent(bounds, scale);
    let canvas = surface.canvas();
    let save = canvas.save();
    canvas.reset_matrix();
    canvas.clip_irect(
        IRect::new(0, 0, width as i32, height as i32),
        ClipOp::Intersect,
    );
    canvas.clear(skia_safe::Color::TRANSPARENT);
    canvas.restore_to_count(save);
}

pub(super) fn damage_region(
    surface: &Surface,
    target_bounds: Bounds,
    scale: f32,
    damage: &DamageRegion,
) -> Region {
    let width = surface.width();
    let height = surface.height();
    let rects: Vec<_> = damage
        .rects()
        .iter()
        .filter_map(|rect| {
            let left = ((rect.x() - target_bounds.x()) * scale).floor() as i32;
            let top = ((rect.y() - target_bounds.y()) * scale).floor() as i32;
            let right = ((rect.x() + rect.width() - target_bounds.x()) * scale).ceil() as i32;
            let bottom = ((rect.y() + rect.height() - target_bounds.y()) * scale).ceil() as i32;
            let clipped = IRect::new(
                left.clamp(0, width),
                top.clamp(0, height),
                right.clamp(0, width),
                bottom.clamp(0, height),
            );
            (!clipped.is_empty()).then_some(clipped)
        })
        .collect();
    let mut region = Region::new();
    region.set_rects(&rects);
    region
}

pub(super) fn physical_damage_rects(
    damage: &DamageRegion,
    scale: f32,
    size: Size<u32>,
) -> Vec<softbuffer::Rect> {
    damage
        .rects()
        .iter()
        .filter_map(|rect| {
            let left = (rect.x() * scale).floor().max(0.0) as u32;
            let top = (rect.y() * scale).floor().max(0.0) as u32;
            let right = ((rect.x() + rect.width()) * scale).ceil().max(0.0) as u32;
            let bottom = ((rect.y() + rect.height()) * scale).ceil().max(0.0) as u32;
            let left = left.min(size.width);
            let top = top.min(size.height);
            let right = right.min(size.width);
            let bottom = bottom.min(size.height);
            Some(softbuffer::Rect {
                x: left,
                y: top,
                width: NonZeroU32::new(right.checked_sub(left)?)?,
                height: NonZeroU32::new(bottom.checked_sub(top)?)?,
            })
        })
        .collect()
}

pub(super) fn full_softbuffer_rect(size: Size<u32>) -> Result<softbuffer::Rect, SkiaBackendError> {
    Ok(softbuffer::Rect {
        x: 0,
        y: 0,
        width: NonZeroU32::new(size.width)
            .ok_or_else(|| SkiaBackendError::InvalidFrame("frame width is zero".into()))?,
        height: NonZeroU32::new(size.height)
            .ok_or_else(|| SkiaBackendError::InvalidFrame("frame height is zero".into()))?,
    })
}

pub(super) fn copy_surface_damage(
    surface: &mut Surface,
    destination: &mut [u32],
    frame_width: u32,
    damage: &[softbuffer::Rect],
) -> Result<(), SkiaBackendError> {
    for rect in damage {
        let width = rect.width.get();
        let height = rect.height.get();
        let mut rgba = vec![0; width as usize * height as usize * 4];
        let info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            ColorSpace::new_srgb(),
        );
        if !surface.read_pixels(
            &info,
            &mut rgba,
            width as usize * 4,
            (rect.x as i32, rect.y as i32),
        ) {
            return Err(SkiaBackendError::PixelRead);
        }
        for row in 0..height as usize {
            let dst_start = (rect.y as usize + row) * frame_width as usize + rect.x as usize;
            let src_start = row * width as usize * 4;
            for column in 0..width as usize {
                let src = src_start + column * 4;
                destination[dst_start + column] = (u32::from(rgba[src]) << 16)
                    | (u32::from(rgba[src + 1]) << 8)
                    | u32::from(rgba[src + 2]);
            }
        }
    }
    Ok(())
}

pub(super) fn non_empty_bounds(bounds: Rect) -> Rect {
    Rect::new(
        bounds.x,
        bounds.y,
        bounds.width.max(f32::EPSILON),
        bounds.height.max(f32::EPSILON),
    )
}

/// Copies `source` over `canvas` one-to-one in device pixels.
///
/// `Src` rather than `SrcOver`: the destination is a swapchain image whose
/// previous contents are undefined, so the frame has to replace them outright
/// rather than blend with them.
fn blit_surface(source: &mut Surface, canvas: &Canvas) {
    let save = canvas.save();
    canvas.reset_matrix();
    let mut paint = Paint::default();
    paint.set_blend_mode(SkBlendMode::Src);
    source.draw(
        canvas,
        (0, 0),
        SamplingOptions::new(skia_safe::FilterMode::Nearest, skia_safe::MipmapMode::None),
        Some(&paint),
    );
    canvas.restore_to_count(save);
}

pub(super) fn configure_canvas(canvas: &Canvas, bounds: Bounds, scale: f32) {
    canvas.scale((scale, scale));
    canvas.translate((-bounds.x(), -bounds.y()));
}

#[cfg(test)]
mod blit_tests {
    use super::*;
    use skia_safe::Rect as SkRect;

    fn raster(width: i32, height: i32) -> Surface {
        skia_safe::surfaces::raster_n32_premul((width, height)).expect("raster surface")
    }

    fn pixel(surface: &mut Surface, x: i32, y: i32) -> [u8; 4] {
        let info = ImageInfo::new(
            (1, 1),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            ColorSpace::new_srgb(),
        );
        let mut bytes = [0u8; 4];
        assert!(
            surface.read_pixels(&info, &mut bytes, 4, (x, y)),
            "pixel read at {x},{y}"
        );
        bytes
    }

    /// The GPU path draws the scene to a persistent surface and blits that to
    /// the acquired swapchain image. There is no GPU in a unit test, but the
    /// blit itself — alignment, and replacing rather than blending with
    /// whatever the destination held — is the part worth pinning down.
    #[test]
    fn blit_replaces_the_destination_one_to_one() {
        let mut source = raster(8, 8);
        source.canvas().clear(skia_safe::Color::TRANSPARENT);
        let mut red = Paint::default();
        red.set_color(skia_safe::Color::RED);
        source
            .canvas()
            .draw_rect(SkRect::from_xywh(2.0, 3.0, 1.0, 1.0), &red);

        // A destination holding something else entirely, as a swapchain image
        // recycled by the presentation engine would.
        let mut destination = raster(8, 8);
        destination.canvas().clear(skia_safe::Color::BLUE);
        blit_surface(&mut source, destination.canvas());

        assert_eq!(
            pixel(&mut destination, 2, 3),
            [255, 0, 0, 255],
            "the source pixel did not land at its own coordinates"
        );
        assert_eq!(
            pixel(&mut destination, 0, 0),
            [0, 0, 0, 0],
            "the destination's old contents showed through the transparent source"
        );
    }

    /// A blit must not inherit whatever transform the caller left on the canvas.
    #[test]
    fn blit_ignores_the_canvas_transform() {
        let mut source = raster(8, 8);
        source.canvas().clear(skia_safe::Color::GREEN);

        let mut destination = raster(8, 8);
        destination.canvas().clear(skia_safe::Color::TRANSPARENT);
        destination.canvas().translate((4.0, 4.0));
        destination.canvas().scale((2.0, 2.0));
        blit_surface(&mut source, destination.canvas());

        assert_eq!(pixel(&mut destination, 0, 0), [0, 255, 0, 255]);
        assert_eq!(pixel(&mut destination, 7, 7), [0, 255, 0, 255]);
    }
}

#[cfg(test)]
mod surface_pool_tests {
    use super::*;
    use skia_safe::Rect as SkRect;

    fn raster(width: i32, height: i32) -> Surface {
        skia_safe::surfaces::raster_n32_premul((width, height)).expect("raster surface")
    }

    fn pixel(surface: &mut Surface, x: i32, y: i32) -> [u8; 4] {
        let info = ImageInfo::new(
            (1, 1),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            ColorSpace::new_srgb(),
        );
        let mut bytes = [0u8; 4];
        assert!(surface.read_pixels(&info, &mut bytes, 4, (x, y)));
        bytes
    }

    /// A recycled surface must come back indistinguishable from a fresh one.
    ///
    /// Without pooling every offscreen surface was newly allocated, so no
    /// drawing code had to leave the canvas tidy. Pooling makes a stray clip or
    /// an unbalanced `save` outlive the frame that left it, which would corrupt
    /// whatever draws next at that size — a failure that only shows up once
    /// some unrelated code drifts out of balance.
    #[test]
    fn a_recycled_surface_carries_no_canvas_state() {
        let mut pool = SurfacePool::default();
        let mut dirty = raster(8, 8);
        {
            let canvas = dirty.canvas();
            canvas.clear(skia_safe::Color::RED);
            // Left behind by the frame that used this surface.
            canvas.save();
            canvas.clip_rect(
                SkRect::from_xywh(0.0, 0.0, 2.0, 2.0),
                ClipOp::Intersect,
                false,
            );
            canvas.translate((3.0, 3.0));
        }
        pool.put(dirty, u64::MAX);

        let mut reused = pool.take(8, 8).expect("the surface is in the pool");
        assert_eq!(
            pixel(&mut reused, 7, 7),
            [0, 0, 0, 0],
            "a leftover clip kept the recycled surface from being cleared"
        );
        let mut green = Paint::default();
        green.set_color(skia_safe::Color::GREEN);
        reused
            .canvas()
            .draw_rect(SkRect::from_xywh(0.0, 0.0, 1.0, 1.0), &green);
        assert_eq!(
            pixel(&mut reused, 0, 0),
            [0, 255, 0, 255],
            "a leftover transform displaced the first draw into the recycled surface"
        );
    }

    /// An extent nothing asks for again must not stay resident forever.
    #[test]
    fn the_pool_drops_extents_that_go_unused() {
        let mut pool = SurfacePool::default();
        pool.begin_frame();
        pool.put(raster(8, 8), u64::MAX);
        for _ in 0..MAX_POOL_IDLE_FRAMES {
            pool.begin_frame();
        }
        assert!(pool.take(8, 8).is_some(), "dropped while still fresh");

        pool.put(raster(8, 8), u64::MAX);
        for _ in 0..=MAX_POOL_IDLE_FRAMES {
            pool.begin_frame();
        }
        assert!(pool.take(8, 8).is_none(), "kept an extent nothing wanted");
        assert_eq!(pool.bytes, 0, "the byte count outlived the surfaces");
    }

    #[test]
    fn the_pool_stops_at_its_budget() {
        let mut pool = SurfacePool::default();
        let budget = surface_bytes(&raster(8, 8)) * 2;
        for _ in 0..4 {
            pool.put(raster(8, 8), budget);
        }
        assert!(pool.take(8, 8).is_some());
        assert!(pool.take(8, 8).is_some());
        assert!(pool.take(8, 8).is_none(), "the pool exceeded its budget");
    }
}
