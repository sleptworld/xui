//! Versioned, deterministic containers for arbitrary application bytes.
//!
//! Defines the on-disk `.xpak` format, the `AssetSource` trait, and readers for
//! mapped (file) and embedded (static slice) archives. No build logic lives
//! here — see `xui-pak-build`.
//!
//! # Format
//!
//! A fixed 56-byte header (`MAGIC = *b"XPAK"`, `FORMAT_VERSION = 1`) is
//! followed by payload blobs (raw or `zstd`-compressed, aligned to a power of
//! two) and a `postcard`-encoded index protected by its own `blake3` hash.
//! Entries are strictly sorted by `AssetId`.
//!
//! # Readers
//!
//! - `PakSource` — memory-mapped file reader.
//! - `EmbeddedPak` — zero-copy reader over a `&'static [u8]`.
//! - `PakOpenOptions` — hard limits on index and decompressed-entry sizes.
//!
//! `load` re-verifies each entry's content hash; `verify_all` checks every entry.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    ops::{Deref, Range},
    path::Path,
    sync::Arc,
};

use camino::Utf8PathBuf;
use memmap2::{Mmap, MmapOptions};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAGIC: [u8; 4] = *b"XPAK";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 56;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct AssetId([u8; 16]);

impl AssetId {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub fn from_path(path: &str) -> Result<Self, AssetError> {
        let path = normalize_asset_path(path)?;
        Ok(Self::from_normalized_path(path.as_str()))
    }

    pub fn from_normalized_path(path: &str) -> Self {
        let digest = blake3::hash(path.as_bytes());
        let mut id = [0; 16];
        id.copy_from_slice(&digest.as_bytes()[..16]);
        Self(id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Compression {
    None,
    Zstd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetMetadata {
    pub id: AssetId,
    pub path: Utf8PathBuf,
    pub content_hash: [u8; 32],
    pub stored_len: u64,
    pub original_len: u64,
    pub compression: Compression,
    pub alignment: u32,
}

#[derive(Clone)]
pub enum AssetBytes {
    Static {
        bytes: &'static [u8],
        range: Range<usize>,
    },
    Mapped {
        map: Arc<Mmap>,
        range: Range<usize>,
    },
    Owned(Arc<[u8]>),
}

impl std::fmt::Debug for AssetBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetBytes")
            .field(
                "storage",
                &match self {
                    Self::Static { .. } => "static",
                    Self::Mapped { .. } => "mapped",
                    Self::Owned(_) => "owned",
                },
            )
            .field("len", &self.len())
            .finish()
    }
}

impl AsRef<[u8]> for AssetBytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Static { bytes, range } => &bytes[range.clone()],
            Self::Mapped { map, range } => &map[range.clone()],
            Self::Owned(bytes) => bytes,
        }
    }
}

impl Deref for AssetBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

#[derive(Clone, Debug)]
pub struct AssetData {
    pub id: AssetId,
    pub metadata: AssetMetadata,
    pub bytes: AssetBytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CachePolicy {
    Immutable,
    Volatile,
}

pub trait AssetSource: Send + Sync {
    fn metadata(&self, id: AssetId) -> Result<Option<AssetMetadata>, AssetError>;
    fn load(&self, id: AssetId) -> Result<Option<AssetData>, AssetError>;

    fn cache_policy(&self) -> CachePolicy {
        CachePolicy::Immutable
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PakOpenOptions {
    pub max_index_size: u64,
    pub max_decompressed_entry_size: u64,
}

impl Default for PakOpenOptions {
    fn default() -> Self {
        Self {
            max_index_size: 64 * 1024 * 1024,
            max_decompressed_entry_size: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("invalid asset path `{0}`")]
    InvalidPath(String),
    #[error("invalid pak: {0}")]
    InvalidPak(String),
    #[error("unsupported pak version {0}")]
    UnsupportedVersion(u16),
    #[error("asset `{path}` exceeds configured limit ({size} > {limit})")]
    LimitExceeded { path: String, size: u64, limit: u64 },
    #[error("asset content hash mismatch for `{0}`")]
    HashMismatch(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("index decode failed: {0}")]
    IndexDecode(#[from] postcard::Error),
}

pub fn normalize_asset_path(path: &str) -> Result<Utf8PathBuf, AssetError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(AssetError::InvalidPath(path.to_owned()));
    }
    Ok(Utf8PathBuf::from(path))
}

pub struct PakSource {
    reader: PakReader<MappedStorage>,
}

impl PakSource {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AssetError> {
        Self::open_with_options(path, PakOpenOptions::default())
    }

    pub fn open_with_options(
        path: impl AsRef<Path>,
        options: PakOpenOptions,
    ) -> Result<Self, AssetError> {
        let file = File::open(path)?;
        // SAFETY: the mapping is immutable and retained for the lifetime of every returned slice.
        let map = unsafe { MmapOptions::new().map(&file)? };
        Ok(Self {
            reader: PakReader::new(MappedStorage(Arc::new(map)), options)?,
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = AssetMetadata> + '_ {
        self.reader.entries()
    }

    pub fn verify_all(&self) -> Result<(), AssetError> {
        self.reader.verify_all()
    }
}

impl AssetSource for PakSource {
    fn metadata(&self, id: AssetId) -> Result<Option<AssetMetadata>, AssetError> {
        Ok(self.reader.metadata(id))
    }

    fn load(&self, id: AssetId) -> Result<Option<AssetData>, AssetError> {
        self.reader.load(id)
    }
}

pub struct EmbeddedPak {
    reader: PakReader<StaticStorage>,
}

impl EmbeddedPak {
    pub fn new(bytes: &'static [u8]) -> Result<Self, AssetError> {
        Self::new_with_options(bytes, PakOpenOptions::default())
    }

    pub fn new_with_options(
        bytes: &'static [u8],
        options: PakOpenOptions,
    ) -> Result<Self, AssetError> {
        Ok(Self {
            reader: PakReader::new(StaticStorage(bytes), options)?,
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = AssetMetadata> + '_ {
        self.reader.entries()
    }

    pub fn verify_all(&self) -> Result<(), AssetError> {
        self.reader.verify_all()
    }
}

impl AssetSource for EmbeddedPak {
    fn metadata(&self, id: AssetId) -> Result<Option<AssetMetadata>, AssetError> {
        Ok(self.reader.metadata(id))
    }

    fn load(&self, id: AssetId) -> Result<Option<AssetData>, AssetError> {
        self.reader.load(id)
    }
}

trait Storage: Send + Sync {
    fn bytes(&self) -> &[u8];
    fn view(&self, range: Range<usize>) -> AssetBytes;
}

struct MappedStorage(Arc<Mmap>);

impl Storage for MappedStorage {
    fn bytes(&self) -> &[u8] {
        &self.0
    }

    fn view(&self, range: Range<usize>) -> AssetBytes {
        AssetBytes::Mapped {
            map: Arc::clone(&self.0),
            range,
        }
    }
}

struct StaticStorage(&'static [u8]);

impl Storage for StaticStorage {
    fn bytes(&self) -> &[u8] {
        self.0
    }

    fn view(&self, range: Range<usize>) -> AssetBytes {
        AssetBytes::Static {
            bytes: self.0,
            range,
        }
    }
}

struct PakReader<S> {
    storage: S,
    index: format::PakIndex,
    by_id: HashMap<AssetId, usize>,
    options: PakOpenOptions,
}

impl<S: Storage> PakReader<S> {
    fn new(storage: S, options: PakOpenOptions) -> Result<Self, AssetError> {
        let bytes = storage.bytes();
        let header = format::decode_header(bytes)?;
        if header.version != FORMAT_VERSION {
            return Err(AssetError::UnsupportedVersion(header.version));
        }
        if header.index_len > options.max_index_size {
            return Err(AssetError::LimitExceeded {
                path: "<index>".into(),
                size: header.index_len,
                limit: options.max_index_size,
            });
        }
        if header.index_offset < HEADER_LEN as u64 {
            return Err(AssetError::InvalidPak(
                "index overlaps the fixed header".into(),
            ));
        }
        let index_range = checked_range(header.index_offset, header.index_len, bytes.len())?;
        let index_bytes = &bytes[index_range.clone()];
        if blake3::hash(index_bytes).as_bytes() != &header.index_hash {
            return Err(AssetError::HashMismatch("<index>".into()));
        }
        let index: format::PakIndex = postcard::from_bytes(index_bytes)?;
        let mut by_id = HashMap::with_capacity(index.entries.len());
        let mut paths = HashSet::with_capacity(index.entries.len());
        let mut previous = None;
        for (position, entry) in index.entries.iter().enumerate() {
            validate_entry(entry, header.index_offset, bytes.len())?;
            if previous.is_some_and(|id| id >= entry.id) {
                return Err(AssetError::InvalidPak(
                    "index is not strictly sorted by AssetId".into(),
                ));
            }
            previous = Some(entry.id);
            if !paths.insert(entry.path.clone()) || by_id.insert(entry.id, position).is_some() {
                return Err(AssetError::InvalidPak("duplicate asset path or id".into()));
            }
        }
        Ok(Self {
            storage,
            index,
            by_id,
            options,
        })
    }

    fn metadata(&self, id: AssetId) -> Option<AssetMetadata> {
        self.entry(id).map(metadata_from_entry)
    }

    fn entries(&self) -> impl Iterator<Item = AssetMetadata> + '_ {
        self.index.entries.iter().map(metadata_from_entry)
    }

    fn load(&self, id: AssetId) -> Result<Option<AssetData>, AssetError> {
        let Some(entry) = self.entry(id) else {
            return Ok(None);
        };
        let range = checked_range(entry.offset, entry.stored_len, self.storage.bytes().len())?;
        let stored = &self.storage.bytes()[range.clone()];
        let bytes = match entry.compression {
            Compression::None => self.storage.view(range),
            Compression::Zstd => {
                if entry.original_len > self.options.max_decompressed_entry_size {
                    return Err(AssetError::LimitExceeded {
                        path: entry.path.clone(),
                        size: entry.original_len,
                        limit: self.options.max_decompressed_entry_size,
                    });
                }
                let decoder = zstd::stream::read::Decoder::new(stored)?;
                let mut decoded = Vec::with_capacity(
                    usize::try_from(entry.original_len.min(16 * 1024 * 1024)).unwrap(),
                );
                decoder
                    .take(self.options.max_decompressed_entry_size.saturating_add(1))
                    .read_to_end(&mut decoded)?;
                if decoded.len() as u64 > self.options.max_decompressed_entry_size {
                    return Err(AssetError::LimitExceeded {
                        path: entry.path.clone(),
                        size: decoded.len() as u64,
                        limit: self.options.max_decompressed_entry_size,
                    });
                }
                if decoded.len() as u64 != entry.original_len {
                    return Err(AssetError::InvalidPak(format!(
                        "decoded length mismatch for `{}`",
                        entry.path
                    )));
                }
                AssetBytes::Owned(Arc::from(decoded))
            }
        };
        if blake3::hash(bytes.as_ref()).as_bytes() != &entry.content_hash {
            return Err(AssetError::HashMismatch(entry.path.clone()));
        }
        let metadata = metadata_from_entry(entry);
        Ok(Some(AssetData {
            id,
            metadata,
            bytes,
        }))
    }

    fn verify_all(&self) -> Result<(), AssetError> {
        for entry in &self.index.entries {
            self.load(entry.id)?
                .ok_or_else(|| AssetError::InvalidPak("missing indexed asset".into()))?;
        }
        Ok(())
    }

    fn entry(&self, id: AssetId) -> Option<&format::PakIndexEntry> {
        self.by_id.get(&id).map(|index| &self.index.entries[*index])
    }
}

fn metadata_from_entry(entry: &format::PakIndexEntry) -> AssetMetadata {
    AssetMetadata {
        id: entry.id,
        path: Utf8PathBuf::from(&entry.path),
        content_hash: entry.content_hash,
        stored_len: entry.stored_len,
        original_len: entry.original_len,
        compression: entry.compression,
        alignment: entry.alignment,
    }
}

fn validate_entry(
    entry: &format::PakIndexEntry,
    data_end: u64,
    file_len: usize,
) -> Result<(), AssetError> {
    let normalized = normalize_asset_path(&entry.path)?;
    if AssetId::from_normalized_path(normalized.as_str()) != entry.id {
        return Err(AssetError::InvalidPak(format!(
            "asset id mismatch for `{}`",
            entry.path
        )));
    }
    if entry.alignment == 0 || !entry.alignment.is_power_of_two() {
        return Err(AssetError::InvalidPak(format!(
            "invalid alignment for `{}`",
            entry.path
        )));
    }
    if entry.offset < HEADER_LEN as u64 {
        return Err(AssetError::InvalidPak(format!(
            "payload overlaps header for `{}`",
            entry.path
        )));
    }
    if !entry.offset.is_multiple_of(u64::from(entry.alignment)) {
        return Err(AssetError::InvalidPak(format!(
            "misaligned payload for `{}`",
            entry.path
        )));
    }
    let range = checked_range(entry.offset, entry.stored_len, file_len)?;
    if range.end as u64 > data_end {
        return Err(AssetError::InvalidPak(format!(
            "payload overlaps index for `{}`",
            entry.path
        )));
    }
    if entry.compression == Compression::None && entry.stored_len != entry.original_len {
        return Err(AssetError::InvalidPak(format!(
            "uncompressed length mismatch for `{}`",
            entry.path
        )));
    }
    Ok(())
}

fn checked_range(offset: u64, len: u64, total: usize) -> Result<Range<usize>, AssetError> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| AssetError::InvalidPak("range overflow".into()))?;
    let start =
        usize::try_from(offset).map_err(|_| AssetError::InvalidPak("offset too large".into()))?;
    let end = usize::try_from(end).map_err(|_| AssetError::InvalidPak("range too large".into()))?;
    if end > total {
        return Err(AssetError::InvalidPak("range extends beyond file".into()));
    }
    Ok(start..end)
}

#[doc(hidden)]
pub mod format {
    use super::*;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct PakIndex {
        pub entries: Vec<PakIndexEntry>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct PakIndexEntry {
        pub id: AssetId,
        pub path: String,
        pub content_hash: [u8; 32],
        pub offset: u64,
        pub stored_len: u64,
        pub original_len: u64,
        pub compression: Compression,
        pub alignment: u32,
    }

    pub struct Header {
        pub version: u16,
        pub index_offset: u64,
        pub index_len: u64,
        pub index_hash: [u8; 32],
    }

    pub fn encode_header(
        index_offset: u64,
        index_len: u64,
        index_hash: [u8; 32],
    ) -> [u8; HEADER_LEN] {
        let mut bytes = [0; HEADER_LEN];
        bytes[0..4].copy_from_slice(&MAGIC);
        bytes[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes[6..8].copy_from_slice(&0u16.to_le_bytes());
        bytes[8..16].copy_from_slice(&index_offset.to_le_bytes());
        bytes[16..24].copy_from_slice(&index_len.to_le_bytes());
        bytes[24..56].copy_from_slice(&index_hash);
        bytes
    }

    pub fn decode_header(bytes: &[u8]) -> Result<Header, AssetError> {
        if bytes.len() < HEADER_LEN || bytes[0..4] != MAGIC {
            return Err(AssetError::InvalidPak("missing or invalid header".into()));
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        let flags = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
        if flags != 0 {
            return Err(AssetError::InvalidPak("unsupported header flags".into()));
        }
        Ok(Header {
            version,
            index_offset: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            index_len: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            index_hash: bytes[24..56].try_into().unwrap(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_canonical_and_ids_are_stable() {
        assert_eq!(
            normalize_asset_path("data/app.bin").unwrap().as_str(),
            "data/app.bin"
        );
        assert_eq!(
            AssetId::from_path("data/app.bin").unwrap(),
            AssetId::from_path("data/app.bin").unwrap()
        );
        for invalid in ["", "/a", "a\\b", "a//b", "a/./b", "a/../b"] {
            assert!(normalize_asset_path(invalid).is_err(), "{invalid}");
        }
    }
}
