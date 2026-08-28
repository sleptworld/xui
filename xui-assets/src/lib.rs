//! Source mounting and caching for arbitrary XPAK assets.
//!
//! The runtime side of the asset system. An `AssetManager` mounts one or more
//! `AssetSource`s and resolves assets by `AssetId`, caching decompressed
//! immutable bytes and parsing them through pluggable `AssetFormat`s. Re-exports
//! the container types from `xui-pak`.
//!
//! Sources are searched in insertion order; the first match wins, so mount
//! high-priority overlays first. `CachePolicy::Immutable` (paks) cache
//! decompressed bytes and stay zero-copy where possible; `CachePolicy::Volatile`
//! (`DirectorySource`) never caches so live edits take effect immediately.

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use camino::Utf8PathBuf;
use moka::sync::Cache;
pub use xui_pak::{
    AssetBytes, AssetData, AssetError, AssetId, AssetMetadata, AssetSource, CachePolicy,
    Compression, EmbeddedPak, PakOpenOptions, PakSource, normalize_asset_path,
};

/// Parses the raw data returned by an [`AssetSource`] into a runtime value.
///
/// Formats are represented by zero-sized types in most cases. This keeps parsing policy
/// separate from both the asset source and the value being produced, and allows downstream
/// crates to provide formats for types they do not own.
pub trait AssetFormat {
    type Output;
    type Error: Error + Send + Sync + 'static;

    fn parse(data: AssetData) -> Result<Self::Output, Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum AssetReadError {
    #[error(transparent)]
    Load(#[from] AssetError),
    #[error("failed to parse asset `{path}`: {source}")]
    Parse {
        path: Utf8PathBuf,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    mount: usize,
    id: AssetId,
    content_hash: [u8; 32],
}

pub struct AssetManager {
    /// Sources are searched in insertion order; the first match wins.
    sources: Vec<Arc<dyn AssetSource>>,
    decompressed: Cache<CacheKey, Arc<[u8]>>,
}

impl Default for AssetManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetManager {
    pub fn new() -> Self {
        Self::with_cache_capacity(256)
    }

    pub fn with_cache_capacity(capacity: u64) -> Self {
        Self {
            sources: Vec::new(),
            decompressed: Cache::new(capacity),
        }
    }

    /// Adds a source at the lowest priority. Mount high-priority overlays first.
    pub fn mount(&mut self, source: impl AssetSource + 'static) -> &mut Self {
        self.sources.push(Arc::new(source));
        self
    }

    pub fn mount_arc(&mut self, source: Arc<dyn AssetSource>) -> &mut Self {
        self.sources.push(source);
        self
    }

    pub fn metadata(&self, id: AssetId) -> Result<Option<AssetMetadata>, AssetError> {
        for source in &self.sources {
            if let Some(metadata) = source.metadata(id)? {
                return Ok(Some(metadata));
            }
        }
        Ok(None)
    }

    pub fn metadata_path(&self, path: &str) -> Result<Option<AssetMetadata>, AssetError> {
        self.metadata(AssetId::from_path(path)?)
    }

    pub fn load(&self, id: AssetId) -> Result<Option<AssetData>, AssetError> {
        for (mount, source) in self.sources.iter().enumerate() {
            let metadata = match source.metadata(id)? {
                Some(metadata) => metadata,
                None => continue,
            };
            let key = CacheKey {
                mount,
                id,
                content_hash: metadata.content_hash,
            };
            if source.cache_policy() == CachePolicy::Immutable
                && let Some(bytes) = self.decompressed.get(&key)
            {
                return Ok(Some(AssetData {
                    id,
                    metadata,
                    bytes: AssetBytes::Owned(bytes),
                }));
            }
            let Some(data) = source.load(id)? else {
                continue;
            };
            if source.cache_policy() == CachePolicy::Immutable
                && let AssetBytes::Owned(bytes) = &data.bytes
            {
                self.decompressed.insert(key, Arc::clone(bytes));
            }
            return Ok(Some(data));
        }
        Ok(None)
    }

    pub fn load_path(&self, path: &str) -> Result<Option<AssetData>, AssetError> {
        self.load(AssetId::from_path(path)?)
    }

    /// Loads an asset and parses it using `F`.
    pub fn read<F: AssetFormat>(&self, id: AssetId) -> Result<Option<F::Output>, AssetReadError> {
        let Some(data) = self.load(id)? else {
            return Ok(None);
        };
        let path = data.metadata.path.clone();
        F::parse(data)
            .map(Some)
            .map_err(|source| AssetReadError::Parse {
                path,
                source: Box::new(source),
            })
    }

    /// Resolves an asset path and parses the matching asset using `F`.
    pub fn read_path<F: AssetFormat>(
        &self,
        path: &str,
    ) -> Result<Option<F::Output>, AssetReadError> {
        self.read::<F>(AssetId::from_path(path)?)
    }

    pub fn invalidate_cache(&self) {
        self.decompressed.invalidate_all();
    }
}

pub struct DirectorySource {
    root: PathBuf,
}

impl DirectorySource {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, AssetError> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err(AssetError::InvalidPath(root.display().to_string()));
        }
        Ok(Self { root })
    }

    fn find(&self, id: AssetId) -> Result<Option<(Utf8PathBuf, PathBuf)>, AssetError> {
        // Directory mounts favor correctness and live edits over lookup speed. They are intended
        // for development; immutable release content should use an indexed PAK.
        let mut builder = ignore::WalkBuilder::new(&self.root);
        builder.follow_links(false).hidden(false);
        for item in builder.build() {
            let entry =
                item.map_err(|error| AssetError::Io(std::io::Error::other(error.to_string())))?;
            let Some(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_file()
                || kind.is_symlink()
                || entry.path().symlink_metadata()?.file_type().is_symlink()
            {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .map_err(|_| AssetError::InvalidPath(entry.path().display().to_string()))?;
            let relative = relative
                .to_str()
                .ok_or_else(|| AssetError::InvalidPath(relative.display().to_string()))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            let logical = normalize_asset_path(&relative)?;
            if AssetId::from_normalized_path(logical.as_str()) == id {
                return Ok(Some((logical, entry.path().to_owned())));
            }
        }
        Ok(None)
    }

    fn read_data(&self, id: AssetId) -> Result<Option<AssetData>, AssetError> {
        let Some((logical, path)) = self.find(id)? else {
            return Ok(None);
        };
        let bytes: Arc<[u8]> = Arc::from(fs::read(path)?);
        let metadata = AssetMetadata {
            id,
            path: logical,
            content_hash: *blake3::hash(&bytes).as_bytes(),
            stored_len: bytes.len() as u64,
            original_len: bytes.len() as u64,
            compression: Compression::None,
            alignment: 1,
        };
        Ok(Some(AssetData {
            id,
            metadata,
            bytes: AssetBytes::Owned(bytes),
        }))
    }
}

impl AssetSource for DirectorySource {
    fn metadata(&self, id: AssetId) -> Result<Option<AssetMetadata>, AssetError> {
        Ok(self.read_data(id)?.map(|data| data.metadata))
    }

    fn load(&self, id: AssetId) -> Result<Option<AssetData>, AssetError> {
        self.read_data(id)
    }

    fn cache_policy(&self) -> CachePolicy {
        CachePolicy::Volatile
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::fs;
    use xui_pak_build::{BuildConfig, build_to};

    #[test]
    fn directory_overrides_pak_and_observes_edits() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let overlay = temp.path().join("overlay");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&overlay).unwrap();
        fs::write(base.join("value.bin"), b"base").unwrap();
        fs::write(overlay.join("value.bin"), b"overlay").unwrap();
        let pak_path = temp.path().join("base.xpak");
        build_to(
            &BuildConfig::default(),
            &base,
            &pak_path,
            &temp.path().join("assets.rs"),
        )
        .unwrap();

        let mut assets = AssetManager::new();
        assets.mount(DirectorySource::new(&overlay).unwrap());
        assets.mount(PakSource::open(&pak_path).unwrap());
        assert_eq!(
            &*assets.load_path("value.bin").unwrap().unwrap().bytes,
            b"overlay"
        );
        fs::write(overlay.join("value.bin"), b"changed").unwrap();
        assert_eq!(
            &*assets.load_path("value.bin").unwrap().unwrap().bytes,
            b"changed"
        );
    }

    #[test]
    fn invalid_paths_are_rejected() {
        let assets = AssetManager::new();
        assert!(assets.load_path("../secret").is_err());
    }

    struct Length;

    impl AssetFormat for Length {
        type Output = usize;
        type Error = Infallible;

        fn parse(data: AssetData) -> Result<Self::Output, Self::Error> {
            Ok(data.bytes.len())
        }
    }

    #[test]
    fn read_parses_loaded_asset() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("value.bin"), b"value").unwrap();
        let mut assets = AssetManager::new();
        assets.mount(DirectorySource::new(temp.path()).unwrap());

        assert_eq!(assets.read_path::<Length>("value.bin").unwrap(), Some(5));
        assert_eq!(assets.read_path::<Length>("missing.bin").unwrap(), None);
    }

    struct AlwaysFails;

    impl AssetFormat for AlwaysFails {
        type Output = ();
        type Error = std::io::Error;

        fn parse(_data: AssetData) -> Result<Self::Output, Self::Error> {
            Err(std::io::Error::other("bad format"))
        }
    }

    #[test]
    fn read_error_contains_asset_path() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("value.bin"), b"value").unwrap();
        let mut assets = AssetManager::new();
        assets.mount(DirectorySource::new(temp.path()).unwrap());

        let error = assets.read_path::<AlwaysFails>("value.bin").unwrap_err();
        assert!(error.to_string().contains("value.bin"));
        assert!(error.to_string().contains("bad format"));
    }
}
