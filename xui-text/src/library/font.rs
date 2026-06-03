use memmap2::Mmap;
use std::{ops::Deref, path::Path, sync::Arc};
pub use swash::{Attributes, CacheKey, FontRef};

/// Identifier for a cached font group.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FontGroupId(pub u64);

#[derive(Clone)]
pub struct Font {
    pub(crate) data: SharedFontData,
    pub(crate) offset: u32,
    pub(crate) attributes: Attributes,
    pub(crate) key: CacheKey,
}

impl Font {
    pub fn attributes(&self) -> Attributes {
        self.as_ref().attributes()
    }

    pub fn localized_strings<'a>(&'a self) -> swash::iter::LocalizedStrings<'a> {
        self.as_ref().localized_strings()
    }

    pub fn variations<'a>(&'a self) -> swash::iter::Variations<'a> {
        self.as_ref().variations()
    }

    pub fn instances<'a>(&'a self) -> swash::iter::Instances<'a> {
        self.as_ref().instances()
    }

    pub fn writing_systems<'a>(&'a self) -> swash::iter::WritingSystems<'a> {
        self.as_ref().writing_systems()
    }

    pub fn features<'a>(&'a self) -> swash::iter::Features<'a> {
        self.as_ref().features()
    }

    pub fn metrics(&self, coords: &[swash::NormalizedCoord]) -> swash::Metrics {
        self.as_ref().metrics(coords)
    }

    pub fn glyph_metrics<'a>(
        &'a self,
        coords: &'a [swash::NormalizedCoord],
    ) -> swash::GlyphMetrics<'a> {
        self.as_ref().glyph_metrics(coords)
    }

    pub fn charmap<'a>(&'a self) -> swash::Charmap<'a> {
        self.as_ref().charmap()
    }

    pub fn color_palettes<'a>(&'a self) -> swash::iter::ColorPalettes<'a> {
        self.as_ref().color_palettes()
    }

    pub fn cache_key(&self) -> CacheKey {
        self.key
    }

    pub fn as_ref<'a>(&'a self) -> FontRef<'a> {
        FontRef {
            data: &self.data,
            offset: self.offset,
            key: self.key,
        }
    }
}

enum FontExp {
    Mapped(Mmap),
    Memory(Vec<u8>),
}

impl FontExp {
    fn data(&self) -> &[u8] {
        match self {
            FontExp::Mapped(mmap) => &*mmap,
            FontExp::Memory(vec) => &vec,
        }
    }
}

#[derive(Clone)]
#[repr(transparent)]
pub struct SharedFontData {
    inner: Arc<FontExp>,
}

impl SharedFontData {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(FontExp::Memory(data)),
        }
    }

    pub fn from_file(path: impl AsRef<Path>, mmap: bool) -> std::io::Result<Self> {
        let path = path.as_ref();

        if mmap {
            let file = std::fs::File::open(path)?;
            let mmap = unsafe { Mmap::map(&file)? };
            Ok(Self {
                inner: Arc::new(FontExp::Mapped(mmap)),
            })
        } else {
            let data = std::fs::read(path)?;
            Ok(Self {
                inner: Arc::new(FontExp::Memory(data)),
            })
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.inner.data()
    }

    pub fn downgrade(&self) -> ShareFontDataWeak {
        ShareFontDataWeak {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

pub struct ShareFontDataWeak {
    inner: std::sync::Weak<FontExp>,
}

impl ShareFontDataWeak {
    pub fn upgrade(&self) -> Option<SharedFontData> {
        self.inner.upgrade().map(|inner| SharedFontData { inner })
    }
}

impl Deref for SharedFontData {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_bytes()
    }
}

impl AsRef<[u8]> for SharedFontData {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl PartialEq for Font {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for Font {}

impl<'a> From<&'a Font> for FontRef<'a> {
    fn from(f: &'a Font) -> FontRef<'a> {
        f.as_ref()
    }
}
