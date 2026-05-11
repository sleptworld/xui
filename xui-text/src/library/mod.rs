use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::SystemTime,
};
mod fallback;
mod font;
mod index;
mod utils;
use crate::{
    library::index::{
        FamilyData, FamilyFontData, FileData, FileDataStatus, FontData, FontEntry, SourceData,
        SourceKind, StaticIndex, parse_families,
    },
    typ::*,
};
pub use font::*;
pub use index::FamilyList;
use swash::{
    FontDataRef, Stretch, StringId, Style, Synthesis, Weight,
    proxy::CharmapProxy,
    text::{
        Cjk, Language, Script,
        cluster::{CharCluster, Status},
    },
};
pub type Epoch = u64;
pub type FontGroupKey = (u64, Attributes);
const MAX_INLINE: usize = 6;

/// Hint for specifying whether font files should be memory mapped.
#[derive(Default, Copy, Clone, PartialEq, Eq, Debug)]
pub enum MmapHint {
    /// Never memory map.
    #[default]
    Never,
    /// Always memory map.
    Always,
    /// Memory map when file size is greater than or equal to a
    /// threshold value.
    Threshold(usize),
}

pub struct FontContext {
    library: Library,
    fonts: FontCache,
    groups: GroupCache,
}

impl Default for FontContext {
    fn default() -> Self {
        let library = Library::builder()
            .add_system_fonts()
            .add_user_fonts()
            .build();

        Self::new(library)
    }
}

impl FontContext {
    pub fn new(library: Library) -> Self {
        let index = library.index.clone();

        let fonts = FontCache {
            index,
            sources: HashMap::default(),
            epoch: 0,
        };

        Self {
            library,
            fonts,
            groups: GroupCache::default(),
        }
    }

    pub fn register_group(&mut self, families: &str, key: u64, attrs: Attributes) -> FontGroupId {
        self.groups.get(&self.fonts, families, key, attrs)
    }

    pub fn select_group(&mut self, descriptor: FontGroupId) {
        self.groups.select(descriptor);
    }

    pub fn reset_group_state(&mut self) {
        self.groups.reset();
    }

    pub fn map_cluster(
        &mut self,
        cluster: &mut CharCluster,
        synthesis: &mut Synthesis,
    ) -> Option<Font> {
        let mut best = None;
        let list = &self.groups.state.fonts;
        for entry in self.groups.fonts.get_mut(list.start..list.end)?.iter_mut() {
            match entry.map_cluster(&mut self.fonts, cluster, synthesis, best.is_none()) {
                Some((font, status)) => {
                    if status == Status::Complete {
                        return Some(font);
                    }
                    best = Some(font);
                }
                None => continue,
            }
        }
        let attrs = list.attributes;
        // We don't have a complete mapping at this point, so time to check
        // fallback fonts.
        if cluster.info().is_emoji() {
            if let Some(entry) = self.groups.emoji(&self.fonts, attrs) {
                match entry.map_cluster(&mut self.fonts, cluster, synthesis, best.is_none()) {
                    Some((font, status)) => {
                        if status == Status::Complete {
                            return Some(font);
                        }
                        best = Some(font);
                    }
                    None => {}
                }
            }
        }
        if !self.groups.state.fallbacks_ready {
            self.groups.fill_fallbacks(&self.fonts);
        }
        for &family in &self.groups.state.fallbacks {
            let entry = match self.groups.state.fallback_map.get_mut(&(family, attrs)) {
                Some(entry) => entry,
                _ => match self.fonts.query(family, attrs) {
                    Some(font) => {
                        self.groups
                            .state
                            .fallback_map
                            .insert((family, attrs), font.selector(attrs).into());
                        self.groups
                            .state
                            .fallback_map
                            .get_mut(&(family, attrs))
                            .unwrap()
                    }
                    _ => continue,
                },
            };
            match entry.map_cluster(&mut self.fonts, cluster, synthesis, best.is_none()) {
                Some((font, status)) => {
                    if status == Status::Complete {
                        return Some(font);
                    }
                    best = Some(font);
                }
                None => continue,
            }
        }
        best
    }

    pub fn select_fallbacks(&mut self, script: Script, language: Option<&Language>) {
        self.groups
            .select_fallbacks(script, language.map(|l| l.cjk()).unwrap_or(Cjk::Simplified))
    }
}

struct FontCache {
    index: Arc<StaticIndex>,
    sources: HashMap<SourceId, Option<(SharedFontData, Epoch)>>,
    epoch: Epoch,
}

impl FontCache {
    /// Returns a font entry that matches the specified family and
    /// attributes.
    pub fn query<'a>(
        &'a self,
        family: impl Into<FamilyKey<'a>>,
        attributes: impl Into<Attributes>,
    ) -> Option<FontEntry<'a>> {
        self.index.query(family, attributes)
    }

    /// Returns a font entry for the specified identifier.
    pub fn font_by_id<'a>(&'a self, id: FontId) -> Option<FontEntry<'a>> {
        self.index.font_by_id(id)
    }

    /// Returns a font matching the specified key.
    pub fn get<'k>(&mut self, key: impl Into<FontKey<'k>>) -> Option<Font> {
        let (source_id, offset, attributes, key) = match key.into() {
            FontKey::Id(id) => {
                let font = self.font_by_id(id)?;
                (
                    font.source().id(),
                    font.offset(),
                    font.attributes(),
                    font.cache_key(),
                )
            }
            FontKey::Descriptor(family, attrs) => {
                let font = self.query(family, attrs)?;
                (
                    font.source().id(),
                    font.offset(),
                    font.attributes(),
                    font.cache_key(),
                )
            }
        };
        let epoch = self.epoch;
        match self.sources.get_mut(&source_id) {
            Some(data) => {
                return data.as_mut().map(|d| {
                    d.1 = epoch;
                    Font {
                        data: d.0.clone(),
                        offset,
                        attributes,
                        key,
                    }
                });
            }
            _ => {}
        }
        let source = self.index.base.sources.get(source_id.to_usize())?;
        match source.get() {
            Some(data) => {
                self.sources.insert(source_id, Some((data.clone(), epoch)));
                Some(Font {
                    data,
                    offset,
                    attributes,
                    key,
                })
            }
            _ => {
                self.sources.insert(source_id, None);
                None
            }
        }
    }
}

/// Internal cache of font groups.
///
/// The strategy here uses a two layer caching system that maps user font
/// groups to a list of resolved font identifiers and a group
/// identifier. The group identifier is then mapped to a transient
/// list of cached fonts. This structure provides reasonably fast lookup
/// while also allowing group invalidation and eviction without the
/// need for notifying user code or for a messy observer/listener style
/// system. Essentially, this is more complex than desired, but the complexity
/// is entirely encapsulated here.
#[derive(Default)]
struct GroupCache {
    /// Maps from a user font descriptor key to a list of font
    /// identifiers.
    key_map: HashMap<FontGroupKey, CachedGroup>,
    /// Temporary storage for parsing a user font descriptor.
    tmp: Vec<(FontId, Attributes)>,
    /// Next descriptor identifier.
    next_id: u64,
    /// Map from descriptor identifier to the list of cached fonts. This
    /// is semi-transient: usually per layout session.
    font_map: HashMap<FontGroupId, CachedFontList>,
    /// Fonts referenced by the ranges in `font_map`.
    fonts: Vec<CachedFont>,
    /// Currently selected descriptor/script/language state for mapping
    /// clusters.
    state: GroupCacheState,
}

/// Current mapping state for a descriptor cache.
struct GroupCacheState {
    /// Selected identifier.
    id: FontGroupId,
    /// Selected font list.
    fonts: CachedFontList,
    /// Fallback state.
    fallback: Option<(Script, Cjk)>,
    /// True if the fallbacks list is current.
    fallbacks_ready: bool,
    /// Transient fallback cache to avoid excessive queries.
    fallback_map: HashMap<(FamilyId, Attributes), CachedFont>,
    /// Current list of fallback families.
    fallbacks: Vec<FamilyId>,
    /// True if we've attempted to load an emoji font.
    emoji_ready: bool,
    /// Cached emoji font.
    emoji: Option<CachedFont>,
}

impl Default for GroupCacheState {
    fn default() -> Self {
        Self {
            id: FontGroupId(!0),
            fonts: CachedFontList::default(),
            fallback: None,
            fallbacks_ready: true,
            fallback_map: HashMap::default(),
            fallbacks: Vec::new(),
            emoji_ready: false,
            emoji: None,
        }
    }
}

impl GroupCacheState {
    fn reset(&mut self) {
        self.id = FontGroupId(!0);
        self.fonts = CachedFontList::default();
        self.fallback = None;
        self.fallbacks_ready = true;
        self.fallback_map.clear();
        self.fallbacks.clear();
        self.emoji_ready = false;
        self.emoji = None;
    }
}

impl GroupCache {
    /// Returns a font group identifier for the specified families and attributes.
    fn get(&mut self, fonts: &FontCache, names: &str, key: u64, attrs: Attributes) -> FontGroupId {
        use std::collections::hash_map::Entry;
        let key = (key, attrs);
        // Fast path for a descriptor we've already seen.
        match self.key_map.get_mut(&key) {
            Some(item) => {
                item.epoch = fonts.epoch;
                match self.font_map.entry(item.id) {
                    Entry::Occupied(..) => {}
                    Entry::Vacant(e) => {
                        let start = self.fonts.len();
                        self.fonts.extend(
                            item.data
                                .get()
                                .iter()
                                .map(|&sel| (sel.0, sel.1, attrs).into()),
                        );
                        let end = self.fonts.len();
                        e.insert(CachedFontList {
                            attributes: attrs,
                            start,
                            end,
                        });
                    }
                }
                return item.id;
            }
            _ => {}
        }
        // Parse the descriptor and collect the font identifiers.
        self.tmp.clear();
        for family in parse_families(names) {
            match fonts.query(family, attrs).map(|f| f.selector(attrs)) {
                Some(sel) => self.tmp.push((sel.0, sel.1)),
                _ => {}
            }
        }
        // Slow path: linear search.
        for (item_key, item) in &self.key_map {
            if item_key.1 != attrs {
                continue;
            }
            let existing = item.data.get();
            if existing == &self.tmp {
                match self.font_map.entry(item.id) {
                    Entry::Occupied(..) => {}
                    Entry::Vacant(e) => {
                        let start = self.fonts.len();
                        self.fonts.extend(
                            item.data
                                .get()
                                .iter()
                                .map(|&sel| (sel.0, sel.1, attrs).into()),
                        );
                        let end = self.fonts.len();
                        e.insert(CachedFontList {
                            attributes: attrs,
                            start,
                            end,
                        });
                    }
                }
                return item.id;
            }
        }
        // Insert a new entry.
        let id = FontGroupId(self.next_id);
        self.next_id += 1;
        let mut data = GroupData::Inline(0, [(FontId(0), Attributes::default()); MAX_INLINE]);
        for font in &self.tmp {
            data.push(font.0, font.1);
        }
        let start = self.fonts.len();
        self.fonts
            .extend(self.tmp.iter().map(|&sel| (sel.0, sel.1, attrs).into()));
        let end = self.fonts.len();
        self.font_map.insert(
            id,
            CachedFontList {
                attributes: attrs,
                start,
                end,
            },
        );
        let desc = CachedGroup {
            id,
            epoch: fonts.epoch,
            data,
        };
        self.key_map.insert(key, desc);
        id
    }

    /// Selects a descriptor for mapping clusters.
    fn select(&mut self, id: FontGroupId) {
        if self.state.id == id {
            return;
        }
        match self.font_map.get(&id) {
            Some(fonts) => self.state.fonts = *fonts,
            _ => self.state.fonts = CachedFontList::default(),
        }
        self.state.id = id;
    }

    /// Selects a fallback state.
    fn select_fallbacks(&mut self, script: Script, cjk: Cjk) {
        if self.state.fallback != Some((script, cjk)) {
            self.state.fallback = Some((script, cjk));
            self.state.fallbacks_ready = false;
            self.state.fallbacks.clear();
        }
    }

    fn fill_fallbacks(&mut self, fonts: &FontCache) {
        self.state.fallbacks.clear();
        self.state.fallbacks_ready = true;
        match self.state.fallback {
            Some((script, cjk)) => {
                let familyids = fonts.index.fallbacks(script, cjk);
                self.state.fallbacks.extend_from_slice(familyids);
            }
            _ => {}
        }
    }

    fn emoji(&mut self, fonts: &FontCache, attrs: Attributes) -> Option<&mut CachedFont> {
        if !self.state.emoji_ready {
            self.state.emoji_ready = true;
            let family = fonts.index.emoji_family()?;
            let sel = fonts.query(family, ())?.selector(attrs);
            self.state.emoji = Some(sel.into());
        }
        self.state.emoji.as_mut()
    }

    /// Clears all transient state.
    fn reset(&mut self) {
        self.state.reset();
        self.font_map.clear();
        self.fonts.clear();
    }

    fn prune(&mut self, epoch: Epoch, target_size: usize) {
        if self.key_map.len() <= target_size {
            return;
        }
        let mut count = self.key_map.len() - target_size;
        self.key_map.retain(|_, v| {
            if count != 0 && v.epoch < epoch {
                count -= 1;
                false
            } else {
                true
            }
        });
        if count != 0 {
            self.key_map.retain(|_, _| {
                if count != 0 {
                    count -= 1;
                    false
                } else {
                    true
                }
            });
        }
    }
}

struct CachedGroup {
    id: FontGroupId,
    epoch: Epoch,
    data: GroupData,
}

#[derive(Clone)]
enum GroupData {
    Inline(u8, [(FontId, Attributes); MAX_INLINE]),
    Heap(Vec<(FontId, Attributes)>),
}

impl GroupData {
    fn clear(&mut self) {
        match self {
            Self::Inline(len, _) => {
                *len = 0;
            }
            Self::Heap(vec) => {
                vec.clear();
            }
        }
    }

    fn push(&mut self, font: FontId, attrs: Attributes) {
        match self {
            Self::Inline(len, ids) => {
                if *len as usize == ids.len() {
                    let mut vec = Vec::from(&ids[..]);
                    vec.push((font, attrs));
                    *self = Self::Heap(vec);
                } else {
                    ids[*len as usize] = (font, attrs);
                    *len += 1;
                }
            }
            Self::Heap(vec) => {
                vec.push((font, attrs));
            }
        }
    }

    fn get(&self) -> &[(FontId, Attributes)] {
        match self {
            Self::Inline(len, ids) => &ids[..*len as usize],
            Self::Heap(vec) => &vec,
        }
    }
}

#[derive(Copy, Clone, Default)]
struct CachedFontList {
    /// Attributes are necessary for fallback font selection.
    attributes: Attributes,
    /// Range of cached fonts.
    start: usize,
    /// ... ditto
    end: usize,
}

struct CachedFont {
    id: FontId,
    font: Option<Font>,
    charmap: CharmapProxy,
    synth: Synthesis,
    error: bool,
}

impl CachedFont {
    #[inline]
    fn map_cluster(
        &mut self,
        fonts: &mut FontCache,
        cluster: &mut CharCluster,
        synth: &mut Synthesis,
        first: bool,
    ) -> Option<(Font, Status)> {
        if self.error {
            return None;
        }
        let font = match &self.font {
            Some(font) => font,
            None => {
                let font = fonts.get(self.id);
                let font = match font {
                    Some(f) => f,
                    _ => {
                        self.error = true;
                        return None;
                    }
                };
                self.charmap = CharmapProxy::from_font(&font.as_ref());
                self.font = Some(font);
                self.font.as_ref().unwrap()
            }
        };
        let charmap = self.charmap.materialize(&font.as_ref());
        let status = cluster.map(|ch| charmap.map(ch));
        if status != Status::Discard || first {
            *synth = self.synth;
            Some((font.clone(), status))
        } else {
            None
        }
    }
}

impl From<(FontId, Attributes, Attributes)> for CachedFont {
    fn from(v: (FontId, Attributes, Attributes)) -> Self {
        let synth = v.1.synthesize(v.2);
        Self {
            id: v.0,
            font: None,
            charmap: CharmapProxy::default(),
            synth,
            error: false,
        }
    }
}

pub struct Library {
    index: Arc<StaticIndex>,
}

pub struct LibraryBuilder {
    scanner: Scanner,
    inner: Inner,
    generics: bool,
    fallbacks: bool,
}

pub struct Inner {
    path: PathBuf,
    mmap: bool,
    timestamp: SystemTime,
    source: SourceId,
    file_added: bool,
    mmap_hint: MmapHint,
    index: StaticIndex,
    lowercase_name: String,
}

impl Default for Inner {
    fn default() -> Self {
        Self::new()
    }
}

impl Inner {
    fn new() -> Self {
        Self {
            path: PathBuf::new(),
            mmap: false,
            timestamp: SystemTime::UNIX_EPOCH,
            source: SourceId(0),
            file_added: false,
            mmap_hint: MmapHint::Threshold(1024 * 1024),
            index: StaticIndex::default(),
            lowercase_name: String::default(),
        }
    }
}

impl ScannerSink for Inner {
    fn add_font(&mut self, font: &FontInfo) {
        self.lowercase_name.clear();
        self.lowercase_name.extend(font.name.to_lowercase().chars());
        let index = &mut self.index;

        let family =
            if let Some(family_id) = index.base.family_map.get(self.lowercase_name.as_str()) {
                let family = &mut index.families[family_id.to_usize()];
                if family.contains(font.stretch, font.weight, font.style) {
                    return;
                }
                family
            } else {
                let family_id = FamilyId(index.families.len() as u32);
                let family = FamilyData {
                    id: family_id,
                    name: SmallString::from(&font.name).unwrap(),
                    fonts: Vec::new(),
                    has_stretch: true,
                };
                index.families.push(family);
                index
                    .base
                    .family_map
                    .insert(SmallString::from(&self.lowercase_name).unwrap(), family_id);
                &mut index.families[family_id.to_usize()]
            };

        if !self.file_added {
            self.file_added = true;
            let mut path2 = PathBuf::new();
            core::mem::swap(&mut path2, &mut self.path);
            index.base.sources.push(SourceData {
                id: self.source,
                kind: SourceKind::File(FileData {
                    path: path2.into(),
                    mmap: self.mmap,
                    timestamp: self.timestamp,
                    status: RwLock::new(FileDataStatus::Empty),
                }),
            });
        }

        let font_id = FontId(index.base.fonts.len() as u32);
        let family_id = family.id;
        let font_data = FontData {
            id: font_id,
            family: family_id,
            source: self.source,
            index: font.index,
            offset: font.offset,
            attributes: font.attrs,
            key: CacheKey::new(),
        };
        index.base.fonts.push(font_data);
        family.fonts.push(FamilyFontData {
            id: font_id,
            stretch: font.stretch,
            weight: font.weight,
            style: font.style,
        });
        if font.stretch != Stretch::NORMAL {
            family.has_stretch = true;
        }
        for name in font.all_names() {
            if !index.base.family_map.contains_key(name.as_str()) {
                index
                    .base
                    .family_map
                    .insert(SmallString::from(name.as_str()).unwrap(), family_id);
            }
        }
    }

    fn enter_file(&mut self, path: PathBuf, timestamp: SystemTime, size: u64) {
        let mmap = match self.mmap_hint {
            MmapHint::Never => false,
            MmapHint::Always => true,
            MmapHint::Threshold(value) => (value as u64) < size,
        };
        self.path = path;
        self.mmap = mmap;
        self.timestamp = timestamp;
        self.source = SourceId(self.index.base.sources.len() as u32);
        self.file_added = false;
    }
}

impl LibraryBuilder {
    fn add_dir(&mut self, path: impl AsRef<Path>) {
        self.scanner.scan_dir(path, true, &mut self.inner);
    }

    pub fn add_system_fonts(&mut self) -> &mut Self {
        #[cfg(target_os = "macos")]
        {
            self.add_dir("/System/Library/Fonts/");
            self.add_dir("/Library/Fonts/");
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(mut windir) = std::env::var_os("SYSTEMROOT") {
                windir.push("\\Fonts\\");
                self.add_dir(windir);
            } else {
                self.add_dir("C:\\Windows\\Fonts\\");
            }
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            self.add_dir("/usr/share/fonts/");
            self.add_dir("/usr/local/share/fonts/");
        }

        self
    }

    /// Adds user fonts to the library.
    pub fn add_user_fonts(&mut self) -> &mut Self {
        #[cfg(target_os = "macos")]
        {
            if let Some(mut homedir) = std::env::var_os("HOME") {
                homedir.push("/Library/Fonts/");
                self.add_dir(&homedir);
            }
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if let Some(mut homedir) = std::env::var_os("HOME") {
                homedir.push("/.local/share/fonts/");
                self.add_dir(&homedir);
            }
        }
        self
    }

    /// Specifies whether default generic families should be mapped for the
    /// current platform.
    pub fn map_generic_families(&mut self, yes: bool) -> &mut Self {
        self.generics = yes;
        self
    }

    /// Specifies whether default fallbacks should be mapped for the current
    /// platform.
    pub fn map_fallbacks(&mut self, yes: bool) -> &mut Self {
        self.fallbacks = yes;
        self
    }

    /// Builds a library for the current configuration.
    pub fn build(&mut self) -> Library {
        let mut index = StaticIndex::default();
        std::mem::swap(&mut self.inner.index, &mut index);
        for family in index.families.iter_mut() {
            family
                .fonts
                .sort_unstable_by(|a, b| a.weight.cmp(&b.weight));
        }
        if self.generics {
            index.setup_default_generic();
        }
        if self.fallbacks {
            index.setup_default_fallbacks();
        }
        Library::new(index)
    }
}

impl Default for LibraryBuilder {
    fn default() -> Self {
        Self {
            inner: Inner::default(),
            scanner: Scanner::default(),
            fallbacks: true,
            generics: true,
        }
    }
}

impl Library {
    pub fn new(index: StaticIndex) -> Self {
        Self {
            index: Arc::new(index),
        }
    }
    pub fn builder() -> LibraryBuilder {
        LibraryBuilder::default()
    }
}

#[derive(Default)]
pub struct FontInfo {
    pub offset: u32,
    pub index: u32,
    pub name: String,
    pub attrs: Attributes,
    pub stretch: Stretch,
    pub weight: Weight,
    pub style: Style,
    all_names: Vec<String>,
    name_count: usize,
}

impl FontInfo {
    pub fn all_names(&self) -> &[String] {
        &self.all_names[..self.name_count]
    }
}

pub trait ScannerSink {
    fn enter_file(&mut self, path: PathBuf, timestamp: SystemTime, size: u64);
    fn add_font(&mut self, font: &FontInfo);
}

#[derive(Default)]
pub struct Scanner {
    font: FontInfo,
    name: String,
}

impl Scanner {
    pub fn scan_dir(
        &mut self,
        path: impl AsRef<Path>,
        all_names: bool,
        sink: &mut impl ScannerSink,
    ) -> Option<()> {
        self.scan_dir_impl(path, all_names, sink, 0)
    }

    pub fn scan_file(
        &mut self,
        path: impl AsRef<Path>,
        all_names: bool,
        sink: &mut impl ScannerSink,
    ) -> Option<()> {
        let file = fs::File::open(path.as_ref()).ok()?;
        let metadata = file.metadata().ok()?;
        let timestamp = metadata.modified().ok()?;
        let size = metadata.len();
        let data = unsafe { memmap2::Mmap::map(&file).ok()? };
        sink.enter_file(path.as_ref().into(), timestamp, size);
        self.scan_data(&*data, all_names, |f| sink.add_font(f))
    }

    pub fn scan_data(
        &mut self,
        data: &[u8],
        all_names: bool,
        mut f: impl FnMut(&FontInfo),
    ) -> Option<()> {
        self.font.name.clear();
        let font_data = FontDataRef::new(data)?;
        for i in 0..font_data.len() {
            if let Some(font) = font_data.get(i) {
                self.scan_font(font, i as u32, all_names, &mut f);
            }
        }
        Some(())
    }

    fn scan_dir_impl(
        &mut self,
        path: impl AsRef<Path>,
        all_names: bool,
        sink: &mut impl ScannerSink,
        recurse: u32,
    ) -> Option<()> {
        if recurse > 4 {
            return Some(());
        }
        let mut lower_ext = [0u8; 3];
        for entry in fs::read_dir(path).ok()? {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() {
                    let mut is_dfont = false;
                    match path.extension().map(|e| e.to_str()).flatten() {
                        Some("dfont") => is_dfont = true,
                        Some(ext) => {
                            let ext = ext.as_bytes();
                            if ext.len() != 3 {
                                continue;
                            }
                            for i in 0..3 {
                                lower_ext[i] = ext[i].to_ascii_lowercase();
                            }
                        }
                        None => continue,
                    };
                    if !is_dfont {
                        match &lower_ext {
                            b"ttf" | b"otf" | b"ttc" | b"otc" => {}
                            _ => continue,
                        }
                    }
                    if let Ok(file) = fs::File::open(&path) {
                        if let Ok(metadata) = entry.metadata() {
                            if let Ok(timestamp) = metadata.modified() {
                                if let Ok(data) = unsafe { memmap2::Mmap::map(&file) } {
                                    sink.enter_file(path, timestamp, metadata.len());
                                    self.scan_data(&*data, all_names, |f| sink.add_font(f));
                                }
                            }
                        }
                    }
                } else if path.is_dir() {
                    self.scan_dir_impl(&path, all_names, sink, recurse + 1);
                }
            }
        }
        Some(())
    }

    fn scan_font(
        &mut self,
        font: FontRef,
        index: u32,
        all_names: bool,
        f: &mut impl FnMut(&FontInfo),
    ) -> Option<()> {
        self.font.name_count = 0;
        let strings = font.localized_strings();
        let vars = font.variations();
        let var_count = vars.len();
        self.font.name.clear();
        // Use typographic family for variable fonts that tend to encode the
        // full style in the standard family name.
        let mut nid = if var_count != 0 {
            StringId::TypographicFamily
        } else {
            StringId::Family
        };
        if let Some(name) = strings.find_by_id(nid, Some("en")) {
            self.font.name.extend(name.chars());
        } else if let Some(name) = strings.find_by_id(nid, None) {
            self.font.name.extend(name.chars());
        }
        if self.font.name.is_empty() {
            nid = if nid == StringId::Family {
                StringId::TypographicFamily
            } else {
                StringId::Family
            };
            if let Some(name) = strings.find_by_id(nid, Some("en")) {
                self.name.extend(name.chars());
            } else if let Some(name) = strings.find_by_id(nid, None) {
                self.name.extend(name.chars());
            }
        }
        if !self.name.is_empty() && self.name.len() < self.font.name.len() {
            core::mem::swap(&mut self.name, &mut self.font.name);
        }
        if self.font.name.is_empty() {
            if let Some(name) = strings.find_by_id(nid, Some("en")) {
                self.font.name.extend(name.chars());
            } else if let Some(name) = strings.find_by_id(nid, None) {
                self.font.name.extend(name.chars());
            }
        }
        if self.font.name.is_empty() {
            return None;
        }
        self.font.attrs = font.attributes();
        let (stretch, weight, style) = self.font.attrs.parts();
        self.font.stretch = stretch;
        self.font.weight = weight;
        self.font.style = style;
        self.font.index = index;
        self.font.offset = font.offset;
        let mut count = 0;
        if all_names {
            for name in strings
                .clone()
                .filter(|name| name.id() == nid && name.is_unicode())
            {
                if count >= self.font.all_names.len() {
                    self.font.all_names.push(String::default());
                }
                let name_buf = &mut self.font.all_names[count];
                count += 1;
                name_buf.clear();
                for ch in name.chars() {
                    name_buf.extend(ch.to_lowercase());
                }
            }
        }
        f(&self.font);
        Some(())
    }
}

mod test {
    use swash::Attributes;

    use crate::library::{FontContext, Library};

    #[test]
    fn test() {
        let library = Library::builder()
            .add_system_fonts()
            .add_user_fonts()
            .build();

        let mut font_ctx = FontContext::new(library);

        let group_id = font_ctx.register_group("pingfang sc", 0, Attributes::default());
    }
}
