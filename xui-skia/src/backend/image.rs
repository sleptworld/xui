//! Image data: decoding and orienting it, the caches that keep it, and drawing
//! it into a canvas.

use moka::sync::Cache;
use skia_safe::{
    AlphaType, Canvas, ClipOp, ColorSpace, ColorType, Data, Image, ImageInfo, Paint,
    SamplingOptions, images,
};
use xui_interface::{
    Affine, Alignment, Bounds, ImageData, ImageFit, ImageKey, ImageRepeat, ImageRotation,
    ImageStyle, ImageTransform, Sampling, Size, TextBackend,
};

use super::{
    SkiaBackend,
    convert::{sk_bounds, sk_matrix},
    surface::configure_canvas,
};
use crate::SkiaBackendError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct CachedImageKey {
    pub(super) data: u64,
    pub(super) transform: ImageTransform,
    pub(super) bytes: u32,
}

#[derive(Clone)]
pub(super) struct RasterImage {
    pub(super) image: Image,
    pub(super) bounds: Bounds,
}

#[derive(Clone)]
pub(super) struct CachedSourceImage {
    pub(super) data_id: u64,
    pub(super) image: Image,
    pub(super) bytes: u32,
}

// Source and transformed images share the overall 256 MiB backend budget.
const IMAGE_CACHE_POOL_BUDGET_BYTES: u64 = 128 * 1024 * 1024;

pub(super) fn image_cache() -> Cache<CachedImageKey, Image> {
    Cache::builder()
        .max_capacity(IMAGE_CACHE_POOL_BUDGET_BYTES)
        .weigher(|key: &CachedImageKey, _image: &Image| key.bytes)
        .build()
}

pub(super) fn source_image_cache() -> Cache<ImageKey, CachedSourceImage> {
    Cache::builder()
        .max_capacity(IMAGE_CACHE_POOL_BUDGET_BYTES)
        .weigher(|_key: &ImageKey, image: &CachedSourceImage| image.bytes)
        .build()
}

pub(super) fn image_bytes(data: &ImageData) -> u32 {
    u32::try_from(data.pixels.len()).unwrap_or(u32::MAX)
}

impl<T: TextBackend> SkiaBackend<T> {
    pub(super) fn draw_image(
        &mut self,
        canvas: &Canvas,
        primitive: &xui::render::ImagePrimitive,
        transform: Affine,
        opacity: f32,
    ) -> Result<(), SkiaBackendError> {
        if primitive.opacity <= 0.0
            || primitive.data.size.width == 0
            || primitive.data.size.height == 0
        {
            return Ok(());
        }
        // `analyze_frame` has already uploaded this, but a mask resolved
        // mid-frame can reach a primitive it never walked.
        self.prepare_image(primitive)?;
        let source = self
            .source_images
            .get(&primitive.image)
            .expect("source image was just prepared")
            .image;
        let key = CachedImageKey {
            data: primitive.data.id().raw(),
            transform: primitive.variant.transform,
            bytes: image_bytes(&primitive.data),
        };
        let image = if primitive.variant.transform == ImageTransform::default() {
            source
        } else if let Some(image) = self.image_cache.get(&key) {
            image
        } else {
            let image = make_image(&primitive.data, primitive.variant.transform)?;
            self.image_cache.insert(key, image.clone());
            image
        };
        let oriented_size = Size::new(image.width() as u32, image.height() as u32);
        let Some(tile) = fitted_image_rect(primitive.bounds, oriented_size, primitive.style) else {
            return Ok(());
        };
        let save = canvas.save();
        canvas.concat(&sk_matrix(transform));
        canvas.clip_rect(sk_bounds(primitive.bounds), ClipOp::Intersect, true);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_alpha_f((primitive.opacity * opacity).clamp(0.0, 1.0));
        let sampling = sampling_options(primitive.style.sampling);
        for rect in image_tiles(primitive.bounds, tile, primitive.style.repeat) {
            canvas.draw_image_rect_with_sampling_options(
                &image,
                None,
                sk_bounds(rect),
                sampling,
                &paint,
            );
        }
        canvas.restore_to_count(save);
        Ok(())
    }
}

pub(super) fn draw_raster_image(
    canvas: &Canvas,
    target_bounds: Bounds,
    scale: f32,
    source: &RasterImage,
    transform: Affine,
    paint: &Paint,
) {
    let save = canvas.save();
    configure_canvas(canvas, target_bounds, scale);
    draw_image_logical(canvas, source, transform, paint);
    canvas.restore_to_count(save);
}

pub(super) fn draw_image_logical(
    canvas: &Canvas,
    source: &RasterImage,
    transform: Affine,
    paint: &Paint,
) {
    let save = canvas.save();
    canvas.concat(&sk_matrix(transform));
    canvas.draw_image_rect_with_sampling_options(
        &source.image,
        None,
        sk_bounds(source.bounds),
        SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::None),
        paint,
    );
    canvas.restore_to_count(save);
}

pub(super) fn make_image(
    data: &ImageData,
    transform: ImageTransform,
) -> Result<Image, SkiaBackendError> {
    if transform == ImageTransform::default() {
        return make_image_from_pixels(data.pixels.as_ref(), data.size.width, data.size.height);
    }
    let (pixels, width, height) = transform_image_pixels(data, transform);
    make_image_from_pixels(&pixels, width, height)
}

pub(super) fn make_image_from_pixels(
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<Image, SkiaBackendError> {
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        ColorSpace::new_srgb(),
    );
    images::raster_from_data(&info, Data::new_copy(pixels), width as usize * 4)
        .ok_or(SkiaBackendError::SurfaceAllocation { width, height })
}

pub(super) fn transform_image_pixels(
    data: &ImageData,
    transform: ImageTransform,
) -> (Vec<u8>, u32, u32) {
    let source_width = data.size.width;
    let source_height = data.size.height;
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

fn fitted_image_rect(container: Bounds, image: Size<u32>, style: ImageStyle) -> Option<Bounds> {
    if container.width() <= 0.0
        || container.height() <= 0.0
        || image.width == 0
        || image.height == 0
    {
        return None;
    }
    let iw = image.width as f32;
    let ih = image.height as f32;
    let sx = container.width() / iw;
    let sy = container.height() / ih;
    let scale = match style.fit {
        ImageFit::Fill => return Some(container),
        ImageFit::Contain => sx.min(sy),
        ImageFit::Cover => sx.max(sy),
        ImageFit::None => 1.0,
        ImageFit::ScaleDown => sx.min(sy).min(1.0),
    };
    let size = Size::new(iw * scale, ih * scale);
    Some(aligned_rect(container, size, style.alignment))
}

fn aligned_rect(container: Bounds, size: Size<f32>, alignment: Alignment) -> Bounds {
    Bounds::from_origin_size(
        (
            container.x() + (container.width() - size.width) * alignment.x,
            container.y() + (container.height() - size.height) * alignment.y,
        ),
        size,
    )
}

fn image_tiles(container: Bounds, tile: Bounds, repeat: ImageRepeat) -> Vec<Bounds> {
    let repeat_x = matches!(repeat, ImageRepeat::Repeat | ImageRepeat::RepeatX);
    let repeat_y = matches!(repeat, ImageRepeat::Repeat | ImageRepeat::RepeatY);
    if !repeat_x && !repeat_y {
        return vec![tile];
    }
    let start_x = if repeat_x {
        container.x() + (tile.x() - container.x()).rem_euclid(tile.width()) - tile.width()
    } else {
        tile.x()
    };
    let start_y = if repeat_y {
        container.y() + (tile.y() - container.y()).rem_euclid(tile.height()) - tile.height()
    } else {
        tile.y()
    };
    let end_x = if repeat_x {
        container.x() + container.width()
    } else {
        tile.x() + tile.width()
    };
    let end_y = if repeat_y {
        container.y() + container.height()
    } else {
        tile.y() + tile.height()
    };
    let mut result = Vec::new();
    let mut y = start_y;
    while y < end_y {
        let mut x = start_x;
        while x < end_x {
            result.push(Bounds::from_origin_size(
                (x, y),
                (tile.width(), tile.height()),
            ));
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

fn sampling_options(value: Sampling) -> SamplingOptions {
    match value {
        Sampling::Nearest => {
            SamplingOptions::new(skia_safe::FilterMode::Nearest, skia_safe::MipmapMode::None)
        }
        Sampling::Linear => {
            SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::None)
        }
        Sampling::Cubic => SamplingOptions::from(skia_safe::CubicResampler::catmull_rom()),
    }
}
