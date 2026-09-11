//! Common asset formats and runtime asset access used by XUI applications.

use std::{
    any::{Any, TypeId},
    convert::Infallible,
    fmt,
    string::FromUtf8Error,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use moka::sync::Cache;
use rustc_hash::{FxHashMap, FxHashSet};
use xui_interface::{ImageData, Size};
use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
use zune_image::{errors::ImageErrors, image::Image};

pub use xui_assets::*;

use crate::{IconData, SvgIconError};

/// How much parsed data nobody holds any more is kept around anyway.
///
/// Small on purpose. It exists for a widget that unmounts and comes straight
/// back -- a row scrolled out of a virtual list and back in -- not to hold a
/// gallery's worth of pixels after the gallery closed.
const RELEASED_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// A parsed value the asset runtime can hand to many holders at once.
///
/// The runtime keeps one value per parsed asset and gives every caller a clone
/// of it, so two widgets showing the same image hold the same pixels -- and,
/// for [`ImageData`], the same identity, which is what the renderer keys its
/// uploads on. It keeps that value for as long as anything else holds a
/// clone, and afterwards only while it fits in a small budget.
///
/// Implemented for [`ImageData`], [`IconData`] and every `Arc<T>`; a format
/// whose output is anything else can produce an `Arc` of it.
pub trait SharedAsset: Clone + Send + Sync + 'static {
    /// Whether a clone of this value is held anywhere besides the runtime.
    fn is_shared(&self) -> bool;

    /// Approximately how many bytes this value keeps resident, for the budget
    /// on values nothing holds any more.
    fn resident_size(&self) -> usize;
}

impl<T: ?Sized + Send + Sync + 'static> SharedAsset for Arc<T> {
    fn is_shared(&self) -> bool {
        Arc::strong_count(self) > 1
    }

    fn resident_size(&self) -> usize {
        std::mem::size_of_val(&**self)
    }
}

impl SharedAsset for ImageData {
    /// The pixels are what every clone shares, so they are what is counted.
    fn is_shared(&self) -> bool {
        Arc::strong_count(&self.pixels) > 1
    }

    fn resident_size(&self) -> usize {
        self.pixels.len()
    }
}

/// A parsed value's identity: which format parsed which bytes of which asset.
/// The hash is there so an edit under a development directory mount parses
/// again instead of handing out the old value.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct ParsedAssetKey {
    format: TypeId,
    id: AssetId,
    content_hash: [u8; 32],
}

/// [`SharedAsset`] with its type erased, so values of every format can live
/// in one table.
trait HeldAsset: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn is_shared(&self) -> bool;
    fn resident_size(&self) -> usize;
}

impl<T: SharedAsset> HeldAsset for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn is_shared(&self) -> bool {
        SharedAsset::is_shared(self)
    }

    fn resident_size(&self) -> usize {
        SharedAsset::resident_size(self)
    }
}

struct Interned {
    value: Box<dyn HeldAsset>,
    /// When it was last handed out, on the runtime's own clock. Released
    /// values over budget go oldest first.
    last_used: u64,
}

struct AssetRuntime {
    manager: Option<Arc<AssetManager>>,
    /// Bumped whenever the manager changes, so a value loaded from the old one
    /// can tell it is stale. See [`HookContext::use_asset`](crate::state::HookContext::use_asset).
    generation: u64,
    /// One value per parsed asset, held here alongside everyone else holding
    /// it. A value only this table holds has been released.
    interned: FxHashMap<ParsedAssetKey, Interned>,
    clock: u64,
    released_budget: usize,
    misses: Cache<AssetId, ()>,
    /// Failures already printed, so a widget that keeps asking for a broken
    /// asset reports it once instead of on every rebuild.
    reported: FxHashSet<String>,
}

impl Default for AssetRuntime {
    fn default() -> Self {
        Self {
            manager: None,
            generation: 0,
            interned: FxHashMap::default(),
            clock: 0,
            released_budget: RELEASED_BUDGET_BYTES,
            misses: Cache::builder()
                .max_capacity(256)
                .time_to_live(Duration::from_secs(2))
                .build(),
            reported: FxHashSet::default(),
        }
    }
}

impl AssetRuntime {
    fn get<T: SharedAsset>(&mut self, key: &ParsedAssetKey) -> Option<T> {
        self.clock += 1;
        let entry = self.interned.get_mut(key)?;
        entry.last_used = self.clock;
        entry.value.as_any().downcast_ref::<T>().cloned()
    }

    /// Adds a freshly parsed value, unless another thread parsed the same
    /// bytes first -- in which case that one is returned, so the asset keeps
    /// a single identity.
    fn intern<T: SharedAsset>(&mut self, key: ParsedAssetKey, value: T) -> T {
        if let Some(existing) = self.get::<T>(&key) {
            return existing;
        }
        self.interned.insert(
            key,
            Interned {
                value: Box::new(value.clone()),
                last_used: self.clock,
            },
        );
        self.collect_released();
        value
    }

    /// Drops released values, oldest first, until those left fit the budget.
    ///
    /// A value something still holds is never dropped: the memory is in use
    /// either way, and dropping it here would only cost it its identity.
    fn collect_released(&mut self) {
        let mut released: Vec<_> = self
            .interned
            .iter()
            .filter(|(_, entry)| !entry.value.is_shared())
            .map(|(key, entry)| (entry.last_used, *key, entry.value.resident_size()))
            .collect();
        let mut resident: usize = released.iter().map(|(_, _, size)| size).sum();
        if resident <= self.released_budget {
            return;
        }
        released.sort_unstable_by_key(|(last_used, _, _)| *last_used);
        for (_, key, size) in released {
            if resident <= self.released_budget {
                break;
            }
            self.interned.remove(&key);
            resident -= size;
        }
    }
}

static ASSET_RUNTIME: LazyLock<Mutex<AssetRuntime>> =
    LazyLock::new(|| Mutex::new(AssetRuntime::default()));

fn runtime() -> std::sync::MutexGuard<'static, AssetRuntime> {
    ASSET_RUNTIME.lock().expect("asset runtime poisoned")
}

/// Installs the asset manager used by asset-backed widgets on the current UI thread.
///
/// Installing a new manager clears decoded values so development overlays and a new
/// application instance cannot observe stale image data.
pub fn install_asset_manager(manager: AssetManager) {
    replace_runtime(Some(Arc::new(manager)));
}

/// Removes the current UI thread's asset manager and decoded image cache.
pub fn clear_asset_manager() {
    replace_runtime(None);
}

fn replace_runtime(manager: Option<Arc<AssetManager>>) {
    let mut runtime = runtime();
    let generation = runtime.generation + 1;
    *runtime = AssetRuntime {
        manager,
        generation,
        ..AssetRuntime::default()
    };
}

/// Which manager the runtime is on. See [`AssetRuntime::generation`].
pub(crate) fn generation() -> u64 {
    runtime().generation
}

/// Drops parsed values nothing holds any more, beyond the small budget kept
/// for widgets that come straight back.
///
/// Called by the app once a frame that committed a rebuild has finished: by
/// then an unmounted widget and the frame that last drew it have both let go.
pub(crate) fn collect_released() {
    runtime().collect_released();
}

/// Loads and parses an asset with `T`, parsing again on every call.
///
/// Asset-backed widgets load from their builders, which run on every rebuild,
/// so a format that is costly to parse wants a shared entry point instead --
/// see [`load_shared_asset`] and [`HookContext::use_asset`](crate::state::HookContext::use_asset).
pub fn load_asset<T: AssetFormat>(id: AssetId) -> Option<T::Output> {
    let found = Found::lookup(id, None)?;
    found.parse::<T>()
}

/// Loads an asset and parses it with `F`, sharing one parsed value among
/// every caller that asks for the same bytes. See [`SharedAsset`].
///
/// Missing assets and parse failures return `None`; an image or icon without
/// data draws nothing rather than failing the frame.
pub fn load_shared_asset<F>(id: AssetId) -> Option<F::Output>
where
    F: AssetFormat + 'static,
    F::Output: SharedAsset,
{
    load_shared::<F>(id, None)
}

/// Loads and decodes an image asset. See [`load_shared_asset`].
pub fn load_image_asset(id: AssetId) -> Option<ImageData> {
    load_shared::<ImageAsset>(id, None)
}

/// Resolves a normalized asset path and decodes it as an image.
pub fn load_image_asset_path(path: &str) -> Option<ImageData> {
    match AssetId::from_path(path) {
        Ok(id) => load_shared::<ImageAsset>(id, Some(path)),
        Err(error) => {
            report_failure(format_args!("{error}"));
            None
        }
    }
}

/// Loads and parses an SVG icon asset. See [`load_shared_asset`].
pub fn load_icon_asset(id: AssetId) -> Option<IconData> {
    load_shared::<SvgAsset>(id, None)
}

fn load_shared<F>(id: AssetId, path: Option<&str>) -> Option<F::Output>
where
    F: AssetFormat + 'static,
    F::Output: SharedAsset,
{
    let found = Found::lookup(id, path)?;
    let key = ParsedAssetKey {
        format: TypeId::of::<F>(),
        id,
        content_hash: found.metadata.content_hash,
    };
    if let Some(shared) = runtime().get::<F::Output>(&key) {
        return Some(shared);
    }
    // Parsed without the lock: a large image takes long enough to decode that
    // holding it would stall every other thread loading anything.
    let value = found.parse::<F>()?;
    Some(runtime().intern(key, value))
}

/// An asset some mounted source has, and what is needed to parse it.
struct Found<'a> {
    id: AssetId,
    path: Option<&'a str>,
    manager: Arc<AssetManager>,
    misses: Cache<AssetId, ()>,
    metadata: AssetMetadata,
}

impl<'a> Found<'a> {
    /// `None` when there is nothing to load: no manager is installed, or the
    /// asset missed recently, or no source has it -- the last recorded as a
    /// miss and reported.
    fn lookup(id: AssetId, path: Option<&'a str>) -> Option<Self> {
        let (manager, misses) = {
            let runtime = runtime();
            (runtime.manager.clone()?, runtime.misses.clone())
        };
        if misses.get(&id).is_some() {
            return None;
        }
        let metadata = match manager.metadata(id) {
            Ok(Some(metadata)) => metadata,
            Ok(None) => {
                misses.insert(id, ());
                report_failure(format_args!(
                    "asset {} is not in any mounted source",
                    describe(id, path)
                ));
                return None;
            }
            Err(error) => {
                misses.insert(id, ());
                report_failure(format_args!(
                    "could not look up asset {}: {error}",
                    describe(id, path)
                ));
                return None;
            }
        };
        Some(Self {
            id,
            path,
            manager,
            misses,
            metadata,
        })
    }

    fn parse<F: AssetFormat>(&self) -> Option<F::Output> {
        let error = match self.manager.read::<F>(self.id) {
            Ok(Some(value)) => return Some(value),
            // The source had it a moment ago; it went away in between.
            Ok(None) => format!("asset {} disappeared while loading", self.describe()),
            Err(error) => format!("could not load asset {}: {error}", self.describe()),
        };
        self.misses.insert(self.id, ());
        report_failure(error);
        None
    }

    fn describe(&self) -> String {
        // The metadata knows the path even when the caller only had an id.
        describe(
            self.id,
            Some(self.path.unwrap_or(self.metadata.path.as_str())),
        )
    }
}

fn describe(id: AssetId, path: Option<&str>) -> String {
    match path {
        Some(path) => format!("`{path}`"),
        None => {
            let hex: String = id
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("with id {hex}")
        }
    }
}

/// Prints why an asset produced nothing, once per distinct failure.
///
/// Every failure still resolves to `None` -- an image or icon that cannot load
/// renders empty rather than failing the frame -- but a mistyped path, a
/// corrupt archive and a file that does not parse would otherwise all look
/// like the same blank space.
fn report_failure(message: impl fmt::Display) {
    let message = message.to_string();
    let Ok(mut runtime) = ASSET_RUNTIME.lock() else {
        return;
    };
    if runtime.reported.insert(message.clone()) {
        eprintln!("xui::assets: {message}");
    }
}

/// What the generated bootstrap module calls into -- the one `xui-build`
/// writes from a build script, or `cargo xui` writes before running Cargo.
/// Not a stable API: it changes together with them.
///
/// Where the development directory and an external package are is decided
/// when the application starts, from the environment, with a path built into
/// debug builds as the fallback. Release builds get no fallback, so a
/// shipped binary carries no paths from the machine that built it.
#[doc(hidden)]
pub mod bootstrap {
    use std::path::{Path, PathBuf};

    use super::{AssetError, AssetManager, DirectorySource, PakSource};

    /// A directory to mount over the bundled assets, so edits show without a
    /// rebuild. `cargo xui` sets it when `dev_directory` is on.
    pub const DIR_ENV: &str = "XUI_ASSETS_DIR";
    /// The external asset package to open, instead of the one beside the
    /// executable. `cargo xui` sets it, which is what lets a test binary --
    /// built somewhere under `deps/` -- find the package.
    pub const PAK_ENV: &str = "XUI_ASSETS_PAK";

    /// Mounts the directory [`DIR_ENV`] names, or else `debug_fallback`.
    ///
    /// A directory that cannot be mounted is reported and skipped: it is a
    /// development convenience, and losing it must not take the bundled
    /// assets underneath down with it.
    pub fn mount_overlay(manager: &mut AssetManager, debug_fallback: Option<&str>) {
        let directory = match std::env::var_os(DIR_ENV) {
            Some(directory) => PathBuf::from(directory),
            None => match debug_fallback {
                Some(directory) => PathBuf::from(directory),
                None => return,
            },
        };
        match DirectorySource::new(&directory) {
            Ok(source) => {
                manager.mount(source);
            }
            Err(error) => eprintln!(
                "xui::assets: not mounting the development directory {}: {error}",
                directory.display()
            ),
        }
    }

    /// Opens an external asset package: the one [`PAK_ENV`] names, or else
    /// `file_name` beside the executable, or else `debug_fallback`.
    pub fn external_pak(
        file_name: &str,
        debug_fallback: Option<&str>,
    ) -> Result<PakSource, AssetError> {
        let path = match std::env::var_os(PAK_ENV) {
            Some(path) => PathBuf::from(path),
            None => {
                let executable = std::env::current_exe()?;
                let directory = executable.parent().ok_or_else(|| {
                    AssetError::InvalidPath(executable.display().to_string())
                })?;
                let beside = directory.join(file_name);
                match debug_fallback {
                    Some(fallback) if !beside.is_file() => PathBuf::from(fallback),
                    _ => beside,
                }
            }
        };
        open_named(&path)
    }

    fn open_named(path: &Path) -> Result<PakSource, AssetError> {
        PakSource::open(path).map_err(|error| match error {
            // Keep the kind, but say which file: "No such file or directory"
            // alone does not tell anyone where the package was expected.
            AssetError::Io(io) => AssetError::Io(std::io::Error::new(
                io.kind(),
                format!("{}: {io}", path.display()),
            )),
            other => AssetError::InvalidPak(format!("{}: {other}", path.display())),
        })
    }
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

    /// The runtime is process-global, so tests that install a manager take
    /// turns rather than clearing each other's out from under them.
    static RUNTIME: Mutex<()> = Mutex::new(());

    fn exclusive_runtime() -> std::sync::MutexGuard<'static, ()> {
        RUNTIME
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

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

    /// One asset in memory, counting how often the runtime looks it up and
    /// how often it actually reads it.
    #[derive(Clone)]
    struct MemorySource {
        data: AssetData,
        lookups: Arc<AtomicUsize>,
        loads: Arc<AtomicUsize>,
    }

    impl AssetSource for MemorySource {
        fn metadata(&self, id: AssetId) -> Result<Option<AssetMetadata>, AssetError> {
            self.lookups.fetch_add(1, Ordering::Relaxed);
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

    struct Mounted {
        id: AssetId,
        lookups: Arc<AtomicUsize>,
        loads: Arc<AtomicUsize>,
    }

    impl Mounted {
        fn lookups(&self) -> usize {
            self.lookups.load(Ordering::Relaxed)
        }

        fn loads(&self) -> usize {
            self.loads.load(Ordering::Relaxed)
        }
    }

    /// Installs a manager holding nothing but `path`.
    fn install_one(path: &str, bytes: impl AsRef<[u8]>) -> Mounted {
        let data = data(path, bytes);
        let mounted = Mounted {
            id: data.id,
            lookups: Arc::default(),
            loads: Arc::default(),
        };
        let mut manager = AssetManager::new();
        manager.mount(MemorySource {
            data,
            lookups: Arc::clone(&mounted.lookups),
            loads: Arc::clone(&mounted.loads),
        });
        install_asset_manager(manager);
        mounted
    }

    fn pixel_png() -> Vec<u8> {
        Image::from_u8(&[10, 20, 30, 255], 1, 1, ColorSpace::RGBA)
            .write_to_vec(zune_image::codecs::ImageFormat::PNG)
            .unwrap()
    }

    const TRIANGLE_SVG: &[u8] =
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0 0H10V10Z"/></svg>"#;

    #[test]
    fn runtime_shares_decoded_images_and_keeps_failures_empty() {
        let _runtime = exclusive_runtime();
        let asset = install_one("images/pixel.png", pixel_png());

        let first = load_image_asset(asset.id).unwrap();
        let second = load_image_asset_path("images/pixel.png").unwrap();
        // One decode and one identity, so the renderer uploads it once.
        assert_eq!(first, second);
        assert_eq!(first.size, Size::new(1, 1));
        let id = asset.id;
        let threaded = std::thread::spawn(move || load_image_asset(id))
            .join()
            .unwrap()
            .unwrap();
        assert_eq!(threaded, first);
        assert_eq!(asset.loads(), 1);
        assert!(load_image_asset_path("images/missing.png").is_none());

        clear_asset_manager();
        assert!(load_image_asset(asset.id).is_none());
    }

    #[test]
    fn runtime_shares_parsed_icons() {
        let _runtime = exclusive_runtime();
        let asset = install_one("icons/triangle.svg", TRIANGLE_SVG);

        let first = load_icon_asset(asset.id).unwrap();
        let second = load_icon_asset(asset.id).unwrap();
        assert_eq!(first, second);
        assert_eq!(asset.loads(), 1, "the second call parsed again");

        clear_asset_manager();
    }

    #[test]
    fn released_values_within_budget_are_kept() {
        let _runtime = exclusive_runtime();
        let asset = install_one("icons/triangle.svg", TRIANGLE_SVG);

        drop(load_icon_asset(asset.id).unwrap());
        collect_released();
        assert!(load_icon_asset(asset.id).is_some());
        assert_eq!(
            asset.loads(),
            1,
            "a released icon within budget was dropped"
        );

        clear_asset_manager();
    }

    #[test]
    fn only_released_values_are_dropped_over_budget() {
        let _runtime = exclusive_runtime();
        let asset = install_one("images/pixel.png", pixel_png());
        runtime().released_budget = 0;

        let held = load_image_asset(asset.id).unwrap();
        collect_released();
        assert_eq!(
            load_image_asset(asset.id).unwrap(),
            held,
            "an image still held was dropped"
        );

        let old_identity = held.id();
        drop(held);
        collect_released();
        let reloaded = load_image_asset(asset.id).unwrap();
        // Decoded again, so a new identity. (The manager's own byte cache
        // means the source is not read again, so `loads` cannot show this.)
        assert_ne!(
            reloaded.id(),
            old_identity,
            "a released image over budget was kept"
        );

        clear_asset_manager();
    }

    /// Two formats over the same bytes, each counting its parses.
    static FIRST_PARSES: AtomicUsize = AtomicUsize::new(0);
    static SECOND_PARSES: AtomicUsize = AtomicUsize::new(0);

    struct FirstFormat;
    struct SecondFormat;

    impl AssetFormat for FirstFormat {
        type Output = Arc<[u8]>;
        type Error = Infallible;

        fn parse(data: AssetData) -> Result<Self::Output, Self::Error> {
            FIRST_PARSES.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::from(data.bytes.as_ref()))
        }
    }

    impl AssetFormat for SecondFormat {
        type Output = Arc<usize>;
        type Error = Infallible;

        fn parse(data: AssetData) -> Result<Self::Output, Self::Error> {
            SECOND_PARSES.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(data.bytes.len()))
        }
    }

    #[test]
    fn each_format_keeps_its_own_value() {
        let _runtime = exclusive_runtime();
        let asset = install_one("data/value.bin", b"value");
        FIRST_PARSES.store(0, Ordering::Relaxed);
        SECOND_PARSES.store(0, Ordering::Relaxed);

        // Interleaved, so a table keyed without the format would have each
        // load evict the other's value and parse again.
        let bytes = load_shared_asset::<FirstFormat>(asset.id).unwrap();
        let len = load_shared_asset::<SecondFormat>(asset.id).unwrap();
        assert_eq!(load_shared_asset::<FirstFormat>(asset.id).unwrap(), bytes);
        assert_eq!(load_shared_asset::<SecondFormat>(asset.id).unwrap(), len);
        assert_eq!(&*bytes, b"value");
        assert_eq!(*len, 5);
        assert_eq!(FIRST_PARSES.load(Ordering::Relaxed), 1);
        assert_eq!(SECOND_PARSES.load(Ordering::Relaxed), 1);

        clear_asset_manager();
    }

    #[test]
    fn use_asset_looks_up_once_per_manager() {
        use crate::fiber::FiberArena;
        use crate::lanes::SYNC_LANE;
        use crate::state::{HookContext, HookStorage, Scheduler};

        let _runtime = exclusive_runtime();
        let asset = install_one("images/pixel.png", pixel_png());
        let mut storage = HookStorage::default();
        let scheduler = Scheduler::default();
        let owner = FiberArena::new().root();
        scheduler.set_root(owner);
        let render = |storage: &mut HookStorage, id| {
            let mut cx = HookContext::new(storage, owner, scheduler.clone(), SYNC_LANE);
            cx.use_asset::<ImageAsset>(id)
        };

        let first = render(&mut storage, asset.id).unwrap();
        let lookups = asset.lookups();
        let second = render(&mut storage, asset.id).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            asset.lookups(),
            lookups,
            "a rebuild looked the asset up again"
        );

        let replacement = install_one("images/pixel.png", pixel_png());
        let third = render(&mut storage, asset.id).unwrap();
        assert_ne!(third, first, "a new manager kept serving the old value");
        assert_eq!(replacement.loads(), 1);

        clear_asset_manager();
    }

    #[test]
    fn a_missing_external_package_is_named_in_the_error() {
        let error = bootstrap::external_pak("no-such-bundle.xpak", None)
            .err()
            .expect("the package does not exist")
            .to_string();
        assert!(error.contains("no-such-bundle.xpak"), "{error}");
    }

    #[test]
    fn failures_are_reported_once_with_the_asset_path() {
        let _runtime = exclusive_runtime();
        let asset = install_one("icons/broken.svg", b"<svg");

        assert!(load_icon_asset(asset.id).is_none());
        // The miss cache answers this one without loading again.
        assert!(load_icon_asset(asset.id).is_none());
        assert!(load_image_asset_path("images/missing.png").is_none());

        let reported = runtime().reported.clone();
        assert_eq!(reported.len(), 2, "{reported:?}");
        for path in ["`icons/broken.svg`", "`images/missing.png`"] {
            assert!(
                reported.iter().any(|message| message.contains(path)),
                "{path} not in {reported:?}"
            );
        }

        clear_asset_manager();
    }
}
