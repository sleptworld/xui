use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use fontique::{
    Attributes as FqAttributes, Blob, Collection, CollectionOptions, FallbackKey,
    FontStyle as FqFontStyle, FontWeight as FqFontWeight, FontWidth as FqFontWidth,
    GenericFamily as FqGenericFamily, Language as FqLanguage, QueryFamily, QueryFont, QueryStatus,
    Script as FqScript, SourceCache, SourceCacheOptions, Synthesis as FqSynthesis,
};
use swash::{
    Attributes, CacheKey, FontDataRef, FontRef, Setting, Stretch, Style, Synthesis,
    text::{
        Language, Script,
        cluster::{CharCluster, Status},
    },
};

/// Identifier for a cached font group.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FontGroupId(pub u64);

#[derive(Clone)]
pub struct Font {
    data: Blob<u8>,
    offset: u32,
    key: CacheKey,
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
            data: self.data.as_ref(),
            offset: self.offset,
            key: self.key,
        }
    }
}

impl PartialEq for Font {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.offset == other.offset
    }
}

impl Eq for Font {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyList {
    names: Arc<str>,
    key: u64,
}

impl FamilyList {
    pub fn new(names: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        names.hash(&mut hasher);
        Self {
            names: Arc::from(names),
            key: hasher.finish(),
        }
    }

    pub fn names(&self) -> &str {
        &self.names
    }

    pub fn families(&self) -> impl Iterator<Item = &str> {
        self.names.split(',').map(normalize_family_name)
    }

    pub fn key(&self) -> u64 {
        self.key
    }

    fn query_families(&self) -> Vec<QueryFamily<'_>> {
        let mut families = Vec::new();
        for name in self.families() {
            if name.is_empty() {
                continue;
            }
            if let Some(generic) = FqGenericFamily::parse(name) {
                families.push(QueryFamily::Generic(generic));
            } else {
                families.push(QueryFamily::Named(name));
            }
        }
        if families.is_empty() {
            families.push(QueryFamily::Generic(FqGenericFamily::SystemUi));
            families.push(QueryFamily::Generic(FqGenericFamily::SansSerif));
        }
        families
    }
}

pub struct FontContext {
    collection: Collection,
    source_cache: SourceCache,
    groups: HashMap<FontGroupId, FontGroup>,
    group_keys: HashMap<(u64, Attributes), FontGroupId>,
    fonts: HashMap<FontCacheKey, CachedFont>,
    selected_group: Option<FontGroupId>,
    fallback_key: Option<FallbackKey>,
    next_group_id: u64,
}

impl Default for FontContext {
    fn default() -> Self {
        Self::new()
    }
}

impl FontContext {
    pub fn new() -> Self {
        Self {
            collection: Collection::new(CollectionOptions::default()),
            source_cache: SourceCache::new(SourceCacheOptions { shared: true }),
            groups: HashMap::default(),
            group_keys: HashMap::default(),
            fonts: HashMap::default(),
            selected_group: None,
            fallback_key: None,
            next_group_id: 0,
        }
    }

    pub fn register_group(&mut self, families: &str, key: u64, attrs: Attributes) -> FontGroupId {
        let group_key = (key, attrs);
        if let Some(id) = self.group_keys.get(&group_key) {
            return *id;
        }

        let id = FontGroupId(self.next_group_id);
        self.next_group_id = self.next_group_id.saturating_add(1);
        self.groups.insert(
            id,
            FontGroup {
                families: FamilyList::new(families),
                attrs,
            },
        );
        self.group_keys.insert(group_key, id);
        id
    }

    pub fn select_group(&mut self, descriptor: FontGroupId) {
        self.selected_group = Some(descriptor);
    }

    pub fn reset_group_state(&mut self) {
        self.selected_group = None;
        self.fallback_key = None;
    }

    pub fn select_fallbacks(&mut self, script: Script, language: Option<&Language>) {
        let script = fontique_script(script);
        let language = language.and_then(fontique_language);
        self.fallback_key = Some(FallbackKey::new(script, language.as_ref()));
    }

    pub fn map_cluster(
        &mut self,
        cluster: &mut CharCluster,
        synthesis: &mut Synthesis,
    ) -> Option<Font> {
        let group_id = self.selected_group?;
        let group = self.groups.get(&group_id)?.clone();
        let candidates = self.query_candidates(&group);
        let mut best = None;

        for (index, candidate) in candidates.into_iter().enumerate() {
            let font = match self.cached_font(candidate.font) {
                Some(font) => font,
                None => continue,
            };
            let charmap = font.charmap();
            let status = cluster.map(|ch| charmap.map(ch));
            if status != Status::Discard || index == 0 {
                *synthesis = fontique_synthesis(candidate.synthesis);
                if status == Status::Complete {
                    return Some(font);
                }
                best = Some(font);
            }
        }

        best
    }

    fn query_candidates(&mut self, group: &FontGroup) -> Vec<CandidateFont> {
        let mut query = self.collection.query(&mut self.source_cache);
        query.set_families(group.families.query_families());
        query.set_attributes(fontique_attributes(group.attrs));
        if let Some(fallback_key) = self.fallback_key {
            query.set_fallbacks(fallback_key);
        }

        let mut candidates = Vec::new();
        query.matches_with(|font| {
            candidates.push(CandidateFont::new(font));
            QueryStatus::Continue
        });
        candidates
    }

    fn cached_font(&mut self, query_font: QueryFont) -> Option<Font> {
        let cache_key = FontCacheKey {
            family: query_font.family.0,
            index: query_font.family.1,
        };
        if let Some(font) = self.fonts.get(&cache_key) {
            return Some(font.font.clone());
        }

        let blob = query_font.blob.clone();
        let (offset, swash_key) = {
            let font_ref = FontDataRef::new(blob.as_ref())?.get(query_font.index as usize)?;
            (font_ref.offset, font_ref.key)
        };
        let font = Font {
            data: blob,
            offset,
            key: swash_key,
        };
        self.fonts
            .insert(cache_key, CachedFont { font: font.clone() });
        Some(font)
    }
}

#[derive(Clone)]
struct FontGroup {
    families: FamilyList,
    attrs: Attributes,
}

struct CandidateFont {
    font: QueryFont,
    synthesis: FqSynthesis,
}

impl CandidateFont {
    fn new(font: &QueryFont) -> Self {
        Self {
            font: font.clone(),
            synthesis: font.synthesis,
        }
    }
}

struct CachedFont {
    font: Font,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FontCacheKey {
    family: fontique::FamilyId,
    index: usize,
}

fn normalize_family_name(name: &str) -> &str {
    name.trim().trim_matches('"').trim_matches('\'').trim()
}

fn fontique_attributes(attrs: Attributes) -> FqAttributes {
    let (stretch, weight, style) = attrs.parts();
    FqAttributes::new(
        fontique_width(stretch),
        fontique_style(style),
        FqFontWeight::new(weight.0 as f32),
    )
}

fn fontique_width(stretch: Stretch) -> FqFontWidth {
    FqFontWidth::from_percentage(stretch.to_percentage())
}

fn fontique_style(style: Style) -> FqFontStyle {
    match style {
        Style::Normal => FqFontStyle::Normal,
        Style::Italic => FqFontStyle::Italic,
        Style::Oblique(angle) => FqFontStyle::Oblique(Some(angle.to_degrees())),
    }
}

fn fontique_script(script: Script) -> FqScript {
    let tag = script.to_opentype().to_be_bytes();
    let Ok(tag) = core::str::from_utf8(&tag) else {
        return FqScript::COMMON;
    };
    FqScript::parse(tag).unwrap_or(FqScript::COMMON)
}

fn fontique_language(language: &Language) -> Option<FqLanguage> {
    let mut tag = String::from(language.language());
    if let Some(script) = language.script() {
        tag.push('-');
        tag.push_str(script);
    }
    if let Some(region) = language.region() {
        tag.push('-');
        tag.push_str(region);
    }
    FqLanguage::parse(&tag).ok()
}

fn fontique_synthesis(synthesis: FqSynthesis) -> Synthesis {
    Synthesis::new(
        synthesis
            .variation_settings()
            .iter()
            .map(|(tag, value)| Setting {
                tag: u32::from_be_bytes(tag.to_be_bytes()),
                value: *value,
            }),
        synthesis.embolden(),
        synthesis.skew().unwrap_or(0.0),
    )
}

#[cfg(test)]
mod tests {
    use swash::{Stretch, Style, Weight};

    use super::*;

    #[test]
    fn family_list_maps_css_generics() {
        let families = FamilyList::new("system-ui, sans-serif");
        let query_families = families.query_families();
        assert!(matches!(
            query_families[0],
            QueryFamily::Generic(FqGenericFamily::SystemUi)
        ));
        assert!(matches!(
            query_families[1],
            QueryFamily::Generic(FqGenericFamily::SansSerif)
        ));
    }

    #[test]
    fn attributes_convert_weight_style_and_width() {
        let attrs = Attributes::new(Stretch::CONDENSED, Weight::BOLD, Style::Italic);
        let converted = fontique_attributes(attrs);
        assert_eq!(converted.weight, FqFontWeight::BOLD);
        assert_eq!(converted.style, FqFontStyle::Italic);
        assert_eq!(converted.width, FqFontWidth::CONDENSED);
    }
}
