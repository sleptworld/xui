//! Raster images: orientation, fitting, tiling, and the cache that keeps
//! Vello's atlas entries alive.
//!
//! Vello keys its image atlas on the identity of the pixel `Blob`, so an image
//! must keep one `peniko::ImageData` for as long as it is drawn. Rebuilding it
//! from the source pixels each frame would upload it again each frame.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use moka::sync::Cache;
use rustc_hash::FxBuildHasher;

use vello::{
    Scene,
    kurbo::{Affine as KurboAffine, Rect as KurboRect},
    peniko::{
        Blob, Fill, ImageAlphaType, ImageBrush, ImageData as PenikoImage, ImageFormat,
        ImageQuality,
    },
};
use xui_core::render::ImagePrimitive;
use xui_interface::{
    Alignment, Bounds, ImageData, ImageFit, ImageRepeat, ImageRotation, ImageStyle,
    ImageTransform, Sampling, Size,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ImageKey {
    data: u64,
    transform: ImageTransform,
}

/// Oriented images as Vello holds them, kept between frames.
///
/// Vello keys its atlas on the identity of the pixel `Blob`, so an image has to
/// keep one `peniko::ImageData` for as long as it is drawn, and a *new* one
/// means uploading it again. The cache therefore outlives frames: an image
/// scrolled out of view and back keeps its blob, until the byte budget (counted
/// in pixels) pushes out the least recently used.
pub(crate) struct ImageCache {
    cache: Cache<ImageKey, PenikoImage, FxBuildHasher>,
    misses: AtomicU64,
}

impl ImageCache {
    pub(crate) fn new(budget_bytes: u64) -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(budget_bytes)
                .weigher(|_: &ImageKey, image: &PenikoImage| {
                    u32::try_from(image.data.len()).unwrap_or(u32::MAX).max(1)
                })
                .build_with_hasher(FxBuildHasher),
            misses: AtomicU64::new(0),
        }
    }

    fn get(&self, data: &ImageData, transform: ImageTransform) -> PenikoImage {
        let key = ImageKey {
            data: data.id().raw(),
            transform,
        };
        self.cache.get_with(key, || {
            self.misses.fetch_add(1, Ordering::Relaxed);
            let (pixels, width, height) = if transform == ImageTransform::default() {
                (data.pixels.to_vec(), data.size.width, data.size.height)
            } else {
                oriented_pixels(data, transform)
            };
            PenikoImage {
                data: Blob::new(Arc::new(pixels)),
                format: ImageFormat::Rgba8,
                // `ImageData` is sRGB with straight alpha.
                alpha_type: ImageAlphaType::Alpha,
                width,
                height,
            }
        })
    }

    /// Applies pending evictions, so the budget holds before the next frame.
    pub(crate) fn settle(&self) {
        self.cache.run_pending_tasks();
    }

    pub(crate) fn entries(&self) -> u64 {
        self.cache.entry_count()
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.cache.weighted_size()
    }

    /// Images built (not found) since the backend started.
    pub(crate) fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
}

pub(crate) fn encode_image(
    scene: &mut Scene,
    world: KurboAffine,
    primitive: &ImagePrimitive,
    cache: &ImageCache,
) {
    let size = primitive.data.size;
    if primitive.opacity <= 0.0 || size.width == 0 || size.height == 0 {
        return;
    }
    let image = cache.get(&primitive.data, primitive.variant.transform);
    let Some(tile) = fitted_rect(
        primitive.bounds,
        Size::new(image.width, image.height),
        primitive.style,
    ) else {
        return;
    };
    let brush = ImageBrush::new(image.clone())
        .with_quality(quality(primitive.style.sampling))
        .with_alpha(primitive.opacity.clamp(0.0, 1.0));

    let bounds = primitive.bounds;
    scene.push_clip_layer(
        Fill::NonZero,
        world,
        &KurboRect::new(
            bounds.min.x as f64,
            bounds.min.y as f64,
            bounds.max.x as f64,
            bounds.max.y as f64,
        ),
    );
    for rect in tiles(bounds, tile, primitive.style.repeat) {
        // `draw_image` places the image's own pixel rectangle at the origin,
        // so scale and move that onto the tile.
        let place = KurboAffine::translate((rect.x() as f64, rect.y() as f64))
            * KurboAffine::scale_non_uniform(
                rect.width() as f64 / image.width as f64,
                rect.height() as f64 / image.height as f64,
            );
        scene.draw_image(&brush, world * place);
    }
    scene.pop_layer();
}

pub(crate) fn quality(sampling: Sampling) -> ImageQuality {
    match sampling {
        Sampling::Nearest => ImageQuality::Low,
        Sampling::Linear => ImageQuality::Medium,
        Sampling::Cubic => ImageQuality::High,
    }
}

/// The rectangle one copy of the image occupies inside `container`.
fn fitted_rect(container: Bounds, image: Size<u32>, style: ImageStyle) -> Option<Bounds> {
    if container.width() <= 0.0 || container.height() <= 0.0 || image.width == 0 || image.height == 0
    {
        return None;
    }
    let (iw, ih) = (image.width as f32, image.height as f32);
    let (sx, sy) = (container.width() / iw, container.height() / ih);
    let scale = match style.fit {
        ImageFit::Fill => return Some(container),
        ImageFit::Contain => sx.min(sy),
        ImageFit::Cover => sx.max(sy),
        ImageFit::None => 1.0,
        ImageFit::ScaleDown => sx.min(sy).min(1.0),
    };
    Some(aligned(
        container,
        Size::new(iw * scale, ih * scale),
        style.alignment,
    ))
}

fn aligned(container: Bounds, size: Size<f32>, alignment: Alignment) -> Bounds {
    Bounds::from_origin_size(
        (
            container.x() + (container.width() - size.width) * alignment.x,
            container.y() + (container.height() - size.height) * alignment.y,
        ),
        size,
    )
}

/// Every copy of `tile` needed to cover `container` under `repeat`.
fn tiles(container: Bounds, tile: Bounds, repeat: ImageRepeat) -> Vec<Bounds> {
    let repeat_x = matches!(repeat, ImageRepeat::Repeat | ImageRepeat::RepeatX);
    let repeat_y = matches!(repeat, ImageRepeat::Repeat | ImageRepeat::RepeatY);
    if !repeat_x && !repeat_y {
        return vec![tile];
    }
    let start = |repeat: bool, container: f32, tile: f32, extent: f32| {
        if repeat {
            container + (tile - container).rem_euclid(extent) - extent
        } else {
            tile
        }
    };
    let end = |repeat: bool, container: f32, extent: f32, tile: f32| {
        if repeat { container + extent } else { tile }
    };
    let start_x = start(repeat_x, container.x(), tile.x(), tile.width());
    let start_y = start(repeat_y, container.y(), tile.y(), tile.height());
    let end_x = end(repeat_x, container.x(), container.width(), tile.x() + tile.width());
    let end_y = end(repeat_y, container.y(), container.height(), tile.y() + tile.height());

    let mut result = Vec::new();
    let mut y = start_y;
    while y < end_y {
        let mut x = start_x;
        while x < end_x {
            result.push(Bounds::from_origin_size((x, y), (tile.width(), tile.height())));
            if !repeat_x {
                break;
            }
            x += tile.width();
        }
        if !repeat_y {
            break;
        }
        y += tile.height();
    }
    result
}

/// Pixels rotated and flipped by `transform`, with the resulting size.
fn oriented_pixels(data: &ImageData, transform: ImageTransform) -> (Vec<u8>, u32, u32) {
    let (source_width, source_height) = (data.size.width, data.size.height);
    let (width, height) = match transform.rotate {
        ImageRotation::Deg0 | ImageRotation::Deg180 => (source_width, source_height),
        ImageRotation::Deg90 | ImageRotation::Deg270 => (source_height, source_width),
    };
    let mut output = vec![0; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let mut u = x;
            let mut v = y;
            if transform.flip_x {
                u = width - 1 - u;
            }
            if transform.flip_y {
                v = height - 1 - v;
            }
            let (sx, sy) = match transform.rotate {
                ImageRotation::Deg0 => (u, v),
                ImageRotation::Deg90 => (v, source_height - 1 - u),
                ImageRotation::Deg180 => (source_width - 1 - u, source_height - 1 - v),
                ImageRotation::Deg270 => (source_width - 1 - v, u),
            };
            let src = (sy as usize * source_width as usize + sx as usize) * 4;
            let dst = (y as usize * width as usize + x as usize) * 4;
            output[dst..dst + 4].copy_from_slice(&data.pixels[src..src + 4]);
        }
    }
    (output, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(fit: ImageFit) -> ImageStyle {
        ImageStyle {
            fit,
            ..ImageStyle::default()
        }
    }

    #[test]
    fn contain_keeps_aspect_and_centres() {
        let container = Bounds::from_origin_size((0.0, 0.0), (100.0, 50.0));
        let tile = fitted_rect(container, Size::new(10, 10), style(ImageFit::Contain)).unwrap();
        assert_eq!((tile.width(), tile.height()), (50.0, 50.0));
        assert_eq!(tile.x(), 25.0);
    }

    #[test]
    fn cover_overflows_the_short_side() {
        let container = Bounds::from_origin_size((0.0, 0.0), (100.0, 50.0));
        let tile = fitted_rect(container, Size::new(10, 10), style(ImageFit::Cover)).unwrap();
        assert_eq!((tile.width(), tile.height()), (100.0, 100.0));
    }

    #[test]
    fn repeat_covers_the_container() {
        let container = Bounds::from_origin_size((0.0, 0.0), (100.0, 30.0));
        let tile = Bounds::from_origin_size((0.0, 0.0), (40.0, 30.0));
        assert_eq!(tiles(container, tile, ImageRepeat::NoRepeat).len(), 1);
        // 0..40, 40..80, 80..120: the last one is clipped by the container.
        assert_eq!(tiles(container, tile, ImageRepeat::RepeatX).len(), 4);
    }

    #[test]
    fn rotating_swaps_the_dimensions() {
        let data = ImageData::rgba8(Size::new(2, 1), vec![1, 2, 3, 4, 5, 6, 7, 8]);
        let transform = ImageTransform {
            rotate: ImageRotation::Deg90,
            ..ImageTransform::default()
        };
        let (pixels, width, height) = oriented_pixels(&data, transform);
        assert_eq!((width, height), (1, 2));
        // 90 degrees clockwise: the left pixel ends up on top.
        assert_eq!(&pixels[..4], &[1, 2, 3, 4]);
    }
}
