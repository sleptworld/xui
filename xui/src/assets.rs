//! Common asset formats and runtime asset access used by XUI applications.

use std::{
    convert::Infallible,
    string::FromUtf8Error,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use moka::sync::Cache;
use xui_interface::{ImageData, Size};
use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
use zune_image::{errors::ImageErrors, image::Image};

pub use xui_assets::*;

use crate::{IconData, SvgIconError};

const DECODED_IMAGE_CACHE_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct DecodedImageKey {
    id: AssetId,
    content_hash: [u8; 32],
}

struct AssetRuntime {
    manager: Option<Arc<AssetManager>>,
    images: Cache<DecodedImageKey, ImageData>,
    misses: Cache<AssetId, ()>,
}

impl Default for AssetRuntime {
    fn default() -> Self {
        Self {
            manager: None,
            images: Cache::builder()
                .max_capacity(DECODED_IMAGE_CACHE_BUDGET_BYTES)
                .weigher(|_key: &DecodedImageKey, image: &ImageData| {
                    u32::try_from(image.pixels.len()).unwrap_or(u32::MAX)
                })
                .build(),
            misses: Cache::builder()
                .max_capacity(256)
                .time_to_live(Duration::from_secs(2))
                .build(),
        }
    }
}

static ASSET_RUNTIME: LazyLock<Mutex<AssetRuntime>> =
    LazyLock::new(|| Mutex::new(AssetRuntime::default()));

/// Installs the asset manager used by asset-backed widgets on the current UI thread.
///
/// Installing a new manager clears decoded values so development overlays and a new
/// application instance cannot observe stale image data.
pub fn install_asset_manager(manager: AssetManager) {
    *ASSET_RUNTIME.lock().expect("asset runtime poisoned") = AssetRuntime {
        manager: Some(Arc::new(manager)),
        ..AssetRuntime::default()
    };
}

/// Removes the current UI thread's asset manager and decoded image cache.
pub fn clear_asset_manager() {
    *ASSET_RUNTIME.lock().expect("asset runtime poisoned") = AssetRuntime::default();
}

pub fn load_asset<T: AssetFormat>(id: AssetId) -> Option<T::Output> {
    let (manager, misses) = {
        let runtime = ASSET_RUNTIME.lock().expect("asset runtime poisoned");
        (runtime.manager.clone()?, runtime.misses.clone())
    };

    if misses.get(&id).is_some() {
        return None;
    }

    let _metadata = match manager.metadata(id).ok().flatten() {
        Some(metadata) => metadata,
        None => {
            misses.insert(id, ());
            return None;
        }
    };

    match manager.read::<T>(id).ok().flatten() {
        Some(data) => Some(data),
        None => {
            misses.insert(id, ());
            None
        }
    }
}

/// Loads and decodes an image asset, caching both successful and failed lookups.
///
/// Missing assets and decoding failures intentionally return `None`; an image widget
/// without decoded data participates in layout with a zero intrinsic size and emits no
/// image paint command.
pub fn load_image_asset(id: AssetId) -> Option<ImageData> {
    let (manager, images, misses) = {
        let runtime = ASSET_RUNTIME.lock().expect("asset runtime poisoned");
        (
            runtime.manager.clone()?,
            runtime.images.clone(),
            runtime.misses.clone(),
        )
    };
    if misses.get(&id).is_some() {
        return None;
    }
    let metadata = match manager.metadata(id).ok().flatten() {
        Some(metadata) => metadata,
        None => {
            misses.insert(id, ());
            return None;
        }
    };
    let key = DecodedImageKey {
        id,
        content_hash: metadata.content_hash,
    };
    if let Some(cached) = images.get(&key) {
        return Some(cached);
    }
    match manager.read::<ImageAsset>(id).ok().flatten() {
        Some(image) => {
            images.insert(key, image.clone());
            Some(image)
        }
        None => {
            misses.insert(id, ());
            None
        }
    }
}

/// Resolves a normalized asset path and decodes it as an image.
pub fn load_image_asset_path(path: &str) -> Option<ImageData> {
    AssetId::from_path(path).ok().and_then(load_image_asset)
}

/// Returns the source bytes without an additional copy.
#[derive(Clone, Copy, Debug, Default)]
pub struct BytesAsset;

impl AssetFormat for BytesAsset {
    type Output = AssetBytes;
    type Error = Infallible;

    fn parse(data: AssetData) -> Result<Self::Output, Self::Error> {
        Ok(data.bytes)
    }
}

/// Decodes a UTF-8 encoded asset into a [`String`].
#[derive(Clone, Copy, Debug, Default)]
pub struct TextAsset;

impl AssetFormat for TextAsset {
    type Output = String;
    type Error = FromUtf8Error;

    fn parse(data: AssetData) -> Result<Self::Output, Self::Error> {
        String::from_utf8(data.bytes.as_ref().to_vec())
    }
}

/// Decodes PNG and JPEG assets with zune into RGBA8 pixels.
#[derive(Clone, Copy, Debug, Default)]
pub struct ImageAsset;

impl AssetFormat for ImageAsset {
    type Output = ImageData;
    type Error = ImageErrors;

    fn parse(data: AssetData) -> Result<Self::Output, Self::Error> {
        let mut image = Image::read(ZCursor::new(data.bytes.as_ref()), DecoderOptions::default())?;
        image.convert_color(ColorSpace::RGBA)?;
        let (width, height) = image.dimensions();
        let width =
            u32::try_from(width).map_err(|_| ImageErrors::GenericStr("image width exceeds u32"))?;
        let height = u32::try_from(height)
            .map_err(|_| ImageErrors::GenericStr("image height exceeds u32"))?;
        let pixels = image
            .flatten_to_u8()
            .into_iter()
            .next()
            .ok_or(ImageErrors::NoImageBuffer)?;
        Ok(ImageData::rgba8(Size::new(width, height), pixels))
    }
}

pub struct SvgAsset;

impl AssetFormat for SvgAsset {
    type Output = IconData;
    type Error = SvgIconError;

    fn parse(data: AssetData) -> Result<Self::Output, Self::Error> {
        IconData::from_svg_bytes(&data.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn data(path: &str, bytes: impl AsRef<[u8]>) -> AssetData {
        let bytes: std::sync::Arc<[u8]> = bytes.as_ref().into();
        let id = AssetId::from_path(path).unwrap();
        AssetData {
            id,
            metadata: AssetMetadata {
                id,
                path: normalize_asset_path(path).unwrap(),
                content_hash: [0; 32],
                stored_len: bytes.len() as u64,
                original_len: bytes.len() as u64,
                compression: Compression::None,
                alignment: 1,
            },
            bytes: AssetBytes::Owned(bytes),
        }
    }

    #[test]
    fn basic_byte_and_text_formats_parse() {
        let bytes = BytesAsset::parse(data("value.bin", b"hello")).unwrap();
        assert_eq!(bytes.as_ref(), b"hello");

        let text = TextAsset::parse(data("value.txt", b"hello")).unwrap();
        assert_eq!(text, "hello");
        assert!(TextAsset::parse(data("bad.txt", b"\xff")).is_err());
    }

    #[test]
    fn image_format_decodes_to_rgba8() {
        let source = Image::from_u8(&[1, 2, 3, 4, 1, 2, 3, 4], 2, 1, ColorSpace::RGBA);
        let png = source
            .write_to_vec(zune_image::codecs::ImageFormat::PNG)
            .unwrap();

        let decoded = ImageAsset::parse(data("pixel.png", png)).unwrap();
        assert_eq!(decoded.size, Size::new(2, 1));
        assert_eq!(decoded.pixels.as_ref(), &[1, 2, 3, 4, 1, 2, 3, 4]);
    }

    #[derive(Clone)]
    struct MemorySource {
        data: AssetData,
        loads: Arc<AtomicUsize>,
    }

    impl AssetSource for MemorySource {
        fn metadata(&self, id: AssetId) -> Result<Option<AssetMetadata>, AssetError> {
            Ok((id == self.data.id).then(|| self.data.metadata.clone()))
        }

        fn load(&self, id: AssetId) -> Result<Option<AssetData>, AssetError> {
            if id != self.data.id {
                return Ok(None);
            }
            self.loads.fetch_add(1, Ordering::Relaxed);
            Ok(Some(self.data.clone()))
        }
    }

    #[test]
    fn runtime_caches_decoded_images_and_keeps_failures_empty() {
        let source = Image::from_u8(&[10, 20, 30, 255], 1, 1, ColorSpace::RGBA);
        let png = source
            .write_to_vec(zune_image::codecs::ImageFormat::PNG)
            .unwrap();
        let data = data("images/pixel.png", png);
        let id = data.id;
        let loads = Arc::new(AtomicUsize::new(0));
        let mut manager = AssetManager::new();
        manager.mount(MemorySource {
            data,
            loads: Arc::clone(&loads),
        });
        install_asset_manager(manager);

        let first = load_image_asset(id).unwrap();
        let second = load_image_asset_path("images/pixel.png").unwrap();
        // assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.size, Size::new(1, 1));
        assert_eq!(second.size, Size::new(1, 1));
        let threaded = std::thread::spawn(move || load_image_asset(id))
            .join()
            .unwrap()
            .unwrap();
        assert_eq!(threaded.size, Size::new(1, 1));
        assert_eq!(loads.load(Ordering::Relaxed), 1);
        assert!(load_image_asset_path("images/missing.png").is_none());

        clear_asset_manager();
        assert!(load_image_asset(id).is_none());
    }
}
