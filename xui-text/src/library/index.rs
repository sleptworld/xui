use super::fallback::Fallbacks;
use crate::library::utils::{AtomicCounter, LowercaseString};
use crate::library::{ShareFontDataWeak, SharedFontData};
use crate::typ::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::SystemTime;
use std::{path::Path, sync::Arc};
use swash::text::{Cjk, Script};
use swash::{Attributes, CacheKey, Stretch, Style, Weight};

pub(crate) static FONT_FAMILY_KEYS: AtomicCounter = AtomicCounter::new();
pub type RequestedAttributes = Attributes;

#[derive(Default)]
pub struct BaseIndex {
    pub family_map: HashMap<SmallString, FamilyId>,
    pub fonts: Vec<FontData>,
    pub sources: Vec<SourceData>,
}

pub struct StaticIndex {
    pub base: BaseIndex,
    pub families: Vec<FamilyData>,
    pub script_map: HashMap<Script, Fallbacks>,
    pub cjk: [Fallbacks; 5],
    pub generic: [Option<FamilyId>; 13],
}

impl Default for StaticIndex {
    fn default() -> Self {
        let fallbacks = Fallbacks::new();
        Self {
            base: BaseIndex::default(),
            families: Vec::new(),
            script_map: Default::default(),
            cjk: [fallbacks; 5],
            generic: [None; 13],
        }
    }
}

impl StaticIndex {
    pub fn setup_default_fallbacks(&mut self) {
        use Cjk::*;
        use Script::*;

        #[cfg(target_os = "windows")]
        {
            self.cjk[Simplified as usize] =
                self.find_fallbacks(&["microsoft yahei", "simsun", "simsun-extb"]);

            self.cjk[Japanese as usize] = self.find_fallbacks(&[
                "meiryo",
                "yu gothic",
                "microsoft yahei",
                "simsun",
                "simsun-extb",
            ]);
            self.cjk[Korean as usize] = self.find_fallbacks(&[
                "malgun gothic",
                "gulim",
                "microsoft yahei",
                "simsun",
                "simsun-extb",
            ]);
            self.map_script(Latin, &["times new roman"]);
            self.map_script(Arabic, &["tahoma", "segoe ui"]);
            self.map_script(Armenian, &["segoe ui", "sylfaen"]);
            self.map_script(Bengali, &["nirmala ui", "vrinda"]);
            self.map_script(Brahmi, &["segoe ui historic"]);
            self.map_script(Braille, &["segoe ui symbol"]);
            self.map_script(Buginese, &["leelawadee ui"]);
            self.map_script(CanadianAboriginal, &["gadugi", "euphemia"]);
            self.map_script(Carian, &["segoe ui historic"]);
            self.map_script(Devanagari, &["nirmala ui", "mangal"]);
            self.map_script(Hebrew, &["david", "segoe ui", "calibri"]);
            self.map_script(Hangul, &["malgun gothic", "gulim"]);
            self.map_script(Myanmar, &["myanmar text"]);
            self.map_script(Malayalam, &["nirmala ui", "kartika"]);
            self.map_script(Han, &["microsoft yahei", "simsun", "simsun-extb"]);
            self.map_script(
                Hiragana,
                &["meiryo", "yu gothic", "ms pgothic", "microsoft yahei"],
            );
            self.map_script(
                Katakana,
                &["meiryo", "yu gothic", "ms pgothic", "microsoft yahei"],
            );
            self.map_script(Kharoshthi, &["segoe ui historic"]);
            self.map_script(
                Khmer,
                &[
                    "leelawadee ui",
                    "khmer ui",
                    "khmer os",
                    "moolboran",
                    "daunpenh",
                ],
            );
            self.map_script(
                Lao,
                &[
                    "leelawadee ui",
                    "lao ui",
                    "dokchampa",
                    "saysettha ot",
                    "phetsarath ot",
                    "code2000",
                ],
            );
            self.map_script(Lisu, &["segoe ui"]);
            self.map_script(
                Syriac,
                &["estrangelo edessa", "estrangelo nisibin", "code2000"],
            );
            self.map_script(Thai, &["tahoma", "leelawadee ui", "leelawadee"]);
            self.map_script(
                Tibetan,
                &["microsoft himalaya", "jomolhari", "tibetan machine uni"],
            );
            self.map_script(Vai, &["ebrima"]);
            self.map_script(Yi, &["microsoft yi baiti", "nuosu sil", "code2000"]);
        }

        #[cfg(target_os = "macos")]
        {
            // Simplified Chinese
            self.cjk[Simplified as usize] = self.find_fallbacks(&["pingfang sc"]);
            // Traditional Chinese
            self.cjk[Traditional as usize] = self.find_fallbacks(&["pingfang tc"]);
            self.cjk[Cjk::None as usize] = self.cjk[Traditional as usize];
            // Japanese
            self.cjk[Japanese as usize] = self.find_fallbacks(&["hiragino kaku gothic pron w3"]);
            // Korean
            self.cjk[Korean as usize] = self.find_fallbacks(&["apple sd gothic neo"]);
            self.map_script(Latin, &["times", "times new roman"]);
            self.map_script(Arabic, &["geeza pro"]);
            self.map_script(
                Devanagari,
                &[
                    "itf devanagari",
                    "kohinoor devanagari",
                    "devanagari sangam mn",
                    "devanagari mt",
                ],
            );
            self.map_script(Bengali, &[]);
            self.map_script(Myanmar, &["noto sans myanmar", "myanmar mn"]);
            self.map_script(Malayalam, &["malayalam mn"]);
            self.map_script(Hebrew, &["lucida grande", "arial hebrew"]);
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            self.map_script(
                Latin,
                &[
                    "liberation sans",
                    "dejavu sans",
                    "ubuntu",
                    "source sans pro",
                ],
            );
            self.map_script(Arabic, &["noto sans arabic"]);
            self.map_script(Hebrew, &["noto sans hebrew", "noto serif hebrew"]);
            self.map_script(Bengali, &["noto sans bengali", "noto serif bengali"]);
            self.map_script(
                Devanagari,
                &["noto sans devanagari", "noto serif devanagari"],
            );
            self.map_script(Malayalam, &["noto sans malayalam", "noto serif malayalam"]);
            self.map_script(Myanmar, &["noto sans myanmar", "noto serif myanmar"]);
        }
    }

    pub fn setup_default_generic(&mut self) {
        use GenericFamily::*;
        #[cfg(target_os = "windows")]
        {
            self.generic[SansSerif as usize] = self.find_family(&["arial"]);
            self.generic[Serif as usize] = self.find_family(&["times new roman"]);
            self.generic[Monospace as usize] = self.find_family(&["courier new"]);
            self.generic[Fantasy as usize] = self.find_family(&["impact"]);
            self.generic[Cursive as usize] = self.find_family(&["comic sans ms"]);
            self.generic[SystemUI as usize] = self.find_family(&["segoe ui"]);
            self.generic[Emoji as usize] = self.find_family(&["segoe ui emoji"]);
        }

        #[cfg(target_os = "macos")]
        {
            self.generic[SansSerif as usize] = self.find_family(&["helvetica"]);
            self.generic[Serif as usize] = self.find_family(&["times"]);
            self.generic[Monospace as usize] = self.find_family(&["courier"]);
            self.generic[Fantasy as usize] = self.find_family(&["papyrus"]);
            self.generic[Cursive as usize] = self.find_family(&["apple chancery"]);
            self.generic[SystemUI as usize] = self.find_family(&["system font", "helvetica"]);
            self.generic[Emoji as usize] = self.find_family(&["apple color emoji"]);
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            self.generic[SansSerif as usize] =
                self.find_family(&["liberation sans", "dejavu sans"]);
            self.generic[Serif as usize] = self.find_family(&[
                "liberation serif",
                "dejavu serif",
                "noto serif",
                "times new roman",
            ]);
            self.generic[Monospace as usize] = self.find_family(&["dejavu sans mono"]);
            self.generic[Fantasy as usize] =
                self.find_family(&["liberation serif", "dejavu serif"]);
            self.generic[Cursive as usize] =
                self.find_family(&["liberation serif", "dejavu serif"]);
            self.generic[SystemUI as usize] = self.find_family(&["liberation sans", "dejavu sans"]);
            self.generic[Emoji as usize] = self.find_family(&["noto color emoji", "emoji one"]);
        }
    }

    pub fn emoji_family(&self) -> Option<FamilyId> {
        self.generic[GenericFamily::Emoji as usize]
    }

    pub fn fallbacks(&self, script: Script, cjk: Cjk) -> &[FamilyId] {
        self.script_map.get(&script).map(|f| f.get()).unwrap_or(&[])
    }

    fn map_script(&mut self, script: Script, families: &[&str]) {
        let fallbacks = self.find_fallbacks(families);
        if fallbacks.len() != 0 {
            self.script_map.insert(script, fallbacks);
        }
    }

    fn find_family(&self, families: &[&str]) -> Option<FamilyId> {
        for family in families {
            if let Some(id) = self.base.family_map.get(*family) {
                return Some(*id);
            }
        }
        None
    }

    fn find_fallbacks(&self, families: &[&str]) -> Fallbacks {
        let mut fallbacks = Fallbacks::new();
        for family in families {
            if let Some(id) = self.base.family_map.get(*family) {
                if !fallbacks.push(*id) {
                    break;
                }
            }
        }
        fallbacks
    }
}

impl StaticIndex {
    /// Returns a font entry that matches the specified family and
    /// attributes.
    pub fn query<'a>(
        &'a self,
        family: impl Into<FamilyKey<'a>>,
        attributes: impl Into<Attributes>,
    ) -> Option<FontEntry<'a>> {
        let family = self.family_by_key(family)?;
        let attrs = attributes.into();
        let font_id = family.data.query(attrs)?;
        let data = self.base.fonts.get(font_id.to_usize())?;
        Some(FontEntry {
            index: &self.base,
            family: family.data,
            data,
        })
    }

    /// Returns a font family entry for the specified family key.
    pub fn family_by_key<'a>(&'a self, key: impl Into<FamilyKey<'a>>) -> Option<FamilyEntry<'a>> {
        match key.into() {
            FamilyKey::Id(id) => self.family_by_id(id),
            FamilyKey::Name(name) => self.family_by_name(name),
            FamilyKey::Generic(generic) => {
                self.family_by_id(self.generic.get(generic as usize).copied()??)
            }
        }
    }

    /// Returns a font family entry for the specified name.
    pub fn family_by_name<'a>(&'a self, name: &str) -> Option<FamilyEntry<'a>> {
        let mut s = LowercaseString::new();
        let lowercase_name = s.get(name)?;
        let id = *self.base.family_map.get(lowercase_name)?;
        self.family_by_id(id)
    }

    /// Returns a font family entry for the specified identifier.
    pub fn family_by_id<'a>(&'a self, id: FamilyId) -> Option<FamilyEntry<'a>> {
        let data = self.families.get(id.to_usize())?;
        Some(FamilyEntry {
            index: &self.base,
            data,
        })
    }

    /// Returns a font entry for the specified identifier.
    pub fn font_by_id<'a>(&'a self, id: FontId) -> Option<FontEntry<'a>> {
        let data = self.base.fonts.get(id.to_usize())?;
        let family = self.families.get(data.family.to_usize())?;
        Some(FontEntry {
            index: &self.base,
            family,
            data,
        })
    }
}

#[derive(Default)]
pub struct DynamicIndex {
    pub base: BaseIndex,
    pub families: Vec<Arc<FamilyData>>,
}

#[derive(Copy, Clone)]
pub struct FontData {
    pub id: FontId,
    pub family: FamilyId,
    pub source: SourceId,
    pub index: u32,
    pub offset: u32,
    pub attributes: Attributes,
    pub key: CacheKey,
}

pub struct SourceData {
    pub id: SourceId,
    pub kind: SourceKind,
}

impl SourceData {
    pub fn get(&self) -> Option<SharedFontData> {
        match &self.kind {
            SourceKind::File(f) => f.get(),
            SourceKind::Memory(s) => Some(s.clone()),
        }
    }
}

pub enum SourceKind {
    Memory(SharedFontData),
    File(FileData),
}

pub enum FileDataStatus {
    Error,
    Empty,
    Present(ShareFontDataWeak),
}

pub struct FileData {
    pub path: PathBuf,
    pub timestamp: SystemTime,
    pub mmap: bool,
    pub status: RwLock<FileDataStatus>,
}

impl FileData {
    pub fn get(&self) -> Option<SharedFontData> {
        {
            let status = self.status.read().unwrap();
            match *status {
                FileDataStatus::Error => return None,
                FileDataStatus::Present(ref data) => {
                    if let Some(data) = data.upgrade() {
                        return Some(data);
                    }
                }
                FileDataStatus::Empty => {}
            }
        }

        let loaded = SharedFontData::from_file(&self.path, self.mmap);
        let mut status = self.status.write().unwrap();
        // If we raced with another writer, the data may have already been
        // loaded, so check again.
        match *status {
            FileDataStatus::Error => return None,
            FileDataStatus::Present(ref data) => {
                if let Some(data) = data.upgrade() {
                    return Some(data);
                }
            }
            _ => {}
        }
        if let Ok(data) = loaded {
            *status = FileDataStatus::Present(data.downgrade());
            Some(data)
        } else {
            *status = FileDataStatus::Error;
            None
        }
    }
}

#[derive(Clone)]
pub struct FamilyData {
    pub id: FamilyId,
    pub name: SmallString,
    pub fonts: Vec<FamilyFontData>,
    pub has_stretch: bool,
}

#[derive(Debug, Clone)]
pub struct FamilyList {
    names: SmallString,
    key: u64,
}

impl FamilyList {
    /// Creates a new font descriptor from a CSS style list of family names.
    pub fn new(names: &str) -> Self {
        Self {
            names: SmallString::from(names).unwrap(),
            key: FONT_FAMILY_KEYS.next(),
        }
    }

    /// Returns the family names.
    pub fn names(&self) -> &str {
        self.names.as_str()
    }

    /// Returns an iterator over the font families represented
    /// by the names.
    pub fn families<'a>(&'a self) -> impl Iterator<Item = FamilyKey<'a>> + Clone + 'a {
        parse_families(self.names())
    }

    pub(crate) fn key(&self) -> u64 {
        self.key
    }
}

pub fn parse_families<'a>(families: &'a str) -> impl Iterator<Item = FamilyKey<'a>> + Clone {
    FamilyParser {
        source: families.as_bytes(),
        cur: 0,
        len: families.len(),
    }
}

#[derive(Clone)]
struct FamilyParser<'a> {
    source: &'a [u8],
    cur: usize,
    len: usize,
}

impl<'a> Iterator for FamilyParser<'a> {
    type Item = FamilyKey<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut quote = None;
        let mut cur = self.cur;
        while cur < self.len && {
            let ch = self.source[cur];
            ch.is_ascii_whitespace() || ch == b','
        } {
            cur += 1;
        }
        self.cur = cur;
        if cur >= self.len {
            return None;
        }
        let first = self.source[cur];
        let mut start = cur;
        match first {
            b'"' | b'\'' => {
                quote = Some(first);
                cur += 1;
                start += 1;
            }
            _ => {}
        }
        if let Some(quote) = quote {
            while cur < self.len {
                if self.source[cur] == quote {
                    self.cur = cur + 1;
                    return Some(FamilyKey::Name(
                        core::str::from_utf8(self.source.get(start..cur)?)
                            .ok()?
                            .trim(),
                    ));
                }
                cur += 1;
            }
            self.cur = cur;
            return Some(FamilyKey::Name(
                core::str::from_utf8(self.source.get(start..cur)?)
                    .ok()?
                    .trim(),
            ));
        }
        let mut end = start;
        while cur < self.len {
            if self.source[cur] == b',' {
                cur += 1;
                break;
            }
            cur += 1;
            end += 1;
        }
        self.cur = cur;
        let name = core::str::from_utf8(self.source.get(start..end)?)
            .ok()?
            .trim();
        Some(match GenericFamily::parse(name) {
            Some(family) => FamilyKey::Generic(family),
            _ => FamilyKey::Name(name),
        })
    }
}

impl FamilyData {
    pub fn contains(&self, stretch: Stretch, weight: Weight, style: Style) -> bool {
        for font in &self.fonts {
            if font.stretch == stretch && font.weight == weight && font.style == style {
                return true;
            }
        }
        false
    }

    /// Returns the font that most closely matches the specified attributes.
    pub fn query(&self, attributes: Attributes) -> Option<FontId> {
        let style = attributes.style();
        let weight = attributes.weight();
        let stretch = attributes.stretch();
        let mut min_stretch_dist = i32::MAX;
        let mut matching_stretch = Stretch::NORMAL;
        if self.has_stretch {
            if stretch <= Stretch::NORMAL {
                for font in &self.fonts {
                    let val = font.stretch;
                    let font_stretch = if val > Stretch::NORMAL {
                        val.raw() as i32 - Stretch::NORMAL.raw() as i32
                            + Stretch::ULTRA_EXPANDED.raw() as i32
                    } else {
                        val.raw() as i32
                    };
                    let offset = (font_stretch - stretch.raw() as i32).abs();
                    if offset < min_stretch_dist {
                        min_stretch_dist = offset;
                        matching_stretch = val;
                    }
                }
            } else {
                for font in &self.fonts {
                    let val = font.stretch;
                    let font_stretch = if val < Stretch::NORMAL {
                        val.raw() as i32 - Stretch::NORMAL.raw() as i32
                            + Stretch::ULTRA_EXPANDED.raw() as i32
                    } else {
                        val.raw() as i32
                    };
                    let offset = (font_stretch - stretch.raw() as i32).abs();
                    if offset < min_stretch_dist {
                        min_stretch_dist = offset;
                        matching_stretch = val;
                    }
                }
            }
        }
        let mut matching_style;
        match style {
            Style::Normal => {
                matching_style = Style::Italic;
                for font in self.fonts.iter().filter(|f| f.stretch == matching_stretch) {
                    let val = font.style;
                    match val {
                        Style::Normal => {
                            matching_style = style;
                            break;
                        }
                        Style::Oblique(_) => {
                            matching_style = val;
                        }
                        _ => {}
                    }
                }
            }
            Style::Oblique(_) => {
                matching_style = Style::Normal;
                for font in self.fonts.iter().filter(|f| f.stretch == matching_stretch) {
                    let val = font.style;
                    match val {
                        Style::Oblique(_) => {
                            matching_style = style;
                            break;
                        }
                        Style::Italic => {
                            matching_style = val;
                        }
                        _ => {}
                    }
                }
            }
            Style::Italic => {
                matching_style = Style::Normal;
                for font in self.fonts.iter().filter(|f| f.stretch == matching_stretch) {
                    let val = font.style;
                    match val {
                        Style::Italic => {
                            matching_style = style;
                            break;
                        }
                        Style::Oblique(_) => {
                            matching_style = val;
                        }
                        _ => {}
                    }
                }
            }
        }
        // If the desired weight is inclusively between 400 and 500
        if weight >= Weight(400) && weight <= Weight(500) {
            // weights greater than or equal to the target weight are checked
            // in ascending order until 500 is hit and checked
            for font in self.fonts.iter().filter(|f| {
                f.stretch == matching_stretch
                    && f.style == matching_style
                    && f.weight >= weight
                    && f.weight <= Weight(500)
            }) {
                return Some(font.id);
            }
            // followed by weights less than the target weight in descending
            // order
            for font in self.fonts.iter().rev().filter(|f| {
                f.stretch == matching_stretch && f.style == matching_style && f.weight < weight
            }) {
                return Some(font.id);
            }
            // followed by weights greater than 500, until a match is found
            return self
                .fonts
                .iter()
                .filter(|f| {
                    f.stretch == matching_stretch
                        && f.style == matching_style
                        && f.weight > Weight(500)
                })
                .map(|f| f.id)
                .next();
        // If the desired weight is less than 400
        } else if weight < Weight(400) {
            // weights less than or equal to the desired weight are checked in
            // descending order
            for font in self.fonts.iter().rev().filter(|f| {
                f.stretch == matching_stretch && f.style == matching_style && f.weight <= weight
            }) {
                return Some(font.id);
            }
            // followed by weights above the desired weight in ascending order
            // until a match is found
            return self
                .fonts
                .iter()
                .filter(|f| {
                    f.stretch == matching_stretch && f.style == matching_style && f.weight > weight
                })
                .map(|f| f.id)
                .next();
        // If the desired weight is greater than 500
        } else {
            // weights greater than or equal to the desired weight are checked
            // in ascending order
            for font in self.fonts.iter().filter(|f| {
                f.stretch == matching_stretch && f.style == matching_style && f.weight >= weight
            }) {
                return Some(font.id);
            }
            // followed by weights below the desired weight in descending order
            // until a match is found
            return self
                .fonts
                .iter()
                .rev()
                .filter(|f| {
                    f.stretch == matching_stretch && f.style == matching_style && f.weight < weight
                })
                .map(|f| f.id)
                .next();
        }
    }
}

#[derive(Clone)]
pub struct FamilyFontData {
    pub id: FontId,
    pub stretch: Stretch,
    pub weight: Weight,
    pub style: Style,
}

/// Font entry in a library.
#[derive(Copy, Clone)]
pub struct FontEntry<'a> {
    index: &'a BaseIndex,
    family: &'a FamilyData,
    data: &'a FontData,
}

impl<'a> FontEntry<'a> {
    /// Returns the font identifier.
    pub fn id(&self) -> FontId {
        self.data.id
    }

    /// Returns the font source.
    pub fn source(&self) -> SourceEntry<'a> {
        SourceEntry {
            index: self.index,
            data: &self.index.sources[self.data.source.to_usize()],
        }
    }

    /// Returns the index of the font in the source.
    pub fn index(&self) -> u32 {
        self.data.index
    }

    /// Returns the offset to the font table directory in the source.
    pub fn offset(&self) -> u32 {
        self.data.offset
    }

    /// Returns the family entry.
    pub fn family(&self) -> FamilyEntry<'a> {
        FamilyEntry {
            index: self.index,
            data: self.family,
        }
    }

    /// Returns the family name.
    pub fn family_name(&self) -> &str {
        self.family.name.as_str()
    }

    /// Returns the font attributes.
    pub fn attributes(&self) -> Attributes {
        self.data.attributes
    }

    pub(crate) fn cache_key(&self) -> CacheKey {
        self.data.key
    }

    pub(crate) fn selector(
        &self,
        attrs: RequestedAttributes,
    ) -> (FontId, Attributes, RequestedAttributes) {
        (self.data.id, self.data.attributes, attrs)
    }
}

// Font family entry in a library.
#[derive(Copy, Clone)]
pub struct FamilyEntry<'a> {
    index: &'a BaseIndex,
    data: &'a FamilyData,
}

impl<'a> FamilyEntry<'a> {
    /// Returns the family identifier.
    pub fn id(&self) -> FamilyId {
        self.data.id
    }

    /// Returns the name of the family.
    pub fn name(&self) -> &str {
        self.data.name.as_str()
    }

    /// Returns an iterator over the fonts in the family.
    pub fn fonts(&'a self) -> impl Iterator<Item = FontEntry<'a>> + 'a {
        self.data.fonts.iter().filter_map(move |f| {
            let data = self.index.fonts.get(f.id.to_usize())?;
            Some(FontEntry {
                index: self.index,
                family: self.data,
                data,
            })
        })
    }
}

/// Source entry in a library.
#[derive(Copy, Clone)]
pub struct SourceEntry<'a> {
    index: &'a BaseIndex,
    data: &'a SourceData,
}

impl<'a> SourceEntry<'a> {
    /// Returns the source identifier.
    pub fn id(&self) -> SourceId {
        self.data.id
    }

    /// Returns the path of the source, if it is represented by a file.
    pub fn path(&self) -> Option<&Path> {
        None
    }
}
