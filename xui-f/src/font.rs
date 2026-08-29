use std::{collections::HashMap, sync::Arc};

use fontique::{
    Attributes, Blob, CharmapIndex, Collection, CollectionOptions, FallbackKey,
    FontStyle as FqFontStyle, FontWeight as FqFontWeight, FontWidth as FqFontWidth, GenericFamily,
    QueryFamily, QueryFont, QueryStatus, Script as FqScript, SourceCache, SourceCacheOptions,
};
use harfrust::{FontRef, ShaperData};
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size as SkrifaSize},
    string::StringId,
};
use unicode_script::Script;
use xui_interface::{FontDataRef, FontFamily, FontQuery, FontStretch, FontStyle, FontWeight};

use crate::FFontId;

pub(crate) struct FaceRecord {
    pub blob: Blob<u8>,
    pub index: u32,
    pub family: String,
    pub postscript_name: String,
    pub weight: FontWeight,
    pub style: FontStyle,
    pub stretch: FontStretch,
    pub metrics: FaceMetrics,
    pub custom: bool,
    charmap_index: CharmapIndex,
    shaper_data: Option<ShaperData>,
}

/// Vertical face metrics normalized to the em square.
///
/// A line box has to contain `ascent + descent` or the renderer clips the
/// glyphs; CJK faces routinely need more than one em (PingFang SC asks for
/// 1.06em above and 0.34em below the baseline), which is why these are tracked
/// per face instead of being approximated from the font size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FaceMetrics {
    /// Distance from the baseline to the top of the line, in ems.
    pub ascent: f32,
    /// Distance from the baseline to the bottom of the line, in ems (positive).
    pub descent: f32,
    /// Recommended extra leading between lines, in ems.
    pub line_gap: f32,
}

impl FaceMetrics {
    /// Used when no face is available, e.g. an empty paragraph shaped before
    /// any font resolves. Close to the common Latin UI face.
    pub const FALLBACK: Self = Self {
        ascent: 0.8,
        descent: 0.2,
        line_gap: 0.0,
    };

    /// The face's natural line height: `normal` line spacing in CSS terms.
    pub fn line_height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }

    pub fn max(self, other: Self) -> Self {
        Self {
            ascent: self.ascent.max(other.ascent),
            descent: self.descent.max(other.descent),
            line_gap: self.line_gap.max(other.line_gap),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct QueryKey {
    families: Vec<FontFamily>,
    weight: FontWeight,
    style: FontStyle,
    stretch: FontStretch,
}

impl From<&FontQuery> for QueryKey {
    fn from(query: &FontQuery) -> Self {
        Self {
            families: query.families.clone(),
            weight: query.weight,
            style: query.style,
            stretch: query.stretch,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum CandidateScope {
    Primary,
    Fallback([u8; 4]),
    LastResort,
}

pub(crate) struct FontStore {
    collection: Collection,
    system_fonts: bool,
    source_cache: SourceCache,
    faces: Vec<FaceRecord>,
    face_ids: HashMap<(u64, u32), FFontId>,
    candidates: HashMap<(QueryKey, CandidateScope), Vec<FFontId>>,
    epoch: u64,
}

impl FontStore {
    pub fn empty() -> Self {
        Self::with_system_fonts(false)
    }

    pub fn with_system_fonts(system_fonts: bool) -> Self {
        Self {
            collection: Collection::new(CollectionOptions {
                shared: false,
                system_fonts,
            }),
            system_fonts,
            // The store is owned by a single backend and never cloned across
            // threads, so the shared (mutex guarded) variant is pure overhead.
            source_cache: SourceCache::new(SourceCacheOptions { shared: false }),
            faces: Vec::new(),
            face_ids: HashMap::new(),
            candidates: HashMap::new(),
            epoch: 0,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    pub fn face(&self, id: FFontId) -> Option<&FaceRecord> {
        self.faces.get(id.0 as usize)
    }

    /// Makes the platform font sources available. Rebuilding the whole store
    /// would drop application fonts registered through `load_font_bytes` and
    /// pay for a second full system scan, so this only extends the collection.
    pub fn load_system_fonts(&mut self) {
        if self.system_fonts {
            return;
        }
        self.collection.load_system_fonts();
        self.system_fonts = true;
        self.candidates.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// Resolve the normal system UI face so common UI text has no first-use
    /// lookup cost. Other styles and fallback scripts stay lazy.
    pub fn warm_system_ui(&mut self) {
        let _ = self.query(&FontQuery {
            families: vec![FontFamily::System],
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
            stretch: FontStretch::Normal,
        });
    }

    /// Metrics of the face a paragraph would use before any text is shaped.
    /// Lines that contain no runs (empty paragraphs) and the minimum height of
    /// every other line are derived from this, so a block of text keeps a
    /// stable line rhythm regardless of which fallbacks its content pulls in.
    pub fn strut_metrics(&mut self, query: &FontQuery) -> FaceMetrics {
        self.candidates_for(query)
            .first()
            .and_then(|id| self.face(*id))
            .map_or(FaceMetrics::FALLBACK, |face| face.metrics)
    }

    pub fn load_font_bytes(&mut self, bytes: Arc<[u8]>) -> FFontId {
        let blob = Blob::new(Arc::new(bytes));
        let registered = self.collection.register_fonts(blob, None);
        let mut first = None;
        for (family_id, fonts) in registered {
            let family = self
                .collection
                .family_name(family_id)
                .unwrap_or_default()
                .to_owned();
            for font in fonts {
                let Some(blob) = font.load(Some(&mut self.source_cache)) else {
                    continue;
                };
                let candidate = MaterializedFont {
                    blob,
                    index: font.index(),
                    family: family.clone(),
                    charmap_index: font.charmap_index(),
                    weight: xui_weight(font.weight()),
                    style: xui_style(font.style()),
                    stretch: xui_stretch(font.width()),
                    custom: true,
                };
                if let Some(id) = self.register_materialized(candidate) {
                    first.get_or_insert(id);
                }
            }
        }
        if first.is_some() {
            self.candidates.clear();
            self.epoch = self.epoch.wrapping_add(1);
        }
        first.unwrap_or(FFontId::INVALID)
    }

    pub fn query(&mut self, query: &FontQuery) -> Option<FFontId> {
        self.candidates_for(query).into_iter().next()
    }

    pub fn candidates_for(&mut self, query: &FontQuery) -> Vec<FFontId> {
        self.candidates_for_scope(query, CandidateScope::Primary)
    }

    pub fn font_for_grapheme(
        &mut self,
        query: &FontQuery,
        primary_candidates: &[FFontId],
        grapheme: &str,
        script: Script,
    ) -> Option<(FFontId, bool)> {
        let primary = primary_candidates.first().copied();
        if let Some(id) = self.first_supporting(primary_candidates, grapheme) {
            return Some((id, Some(id) != primary));
        }

        let tag = script.as_iso15924_tag().to_be_bytes();
        let fallback = self.candidates_for_scope(query, CandidateScope::Fallback(tag));
        if let Some(id) = self.first_supporting(&fallback, grapheme) {
            return Some((id, Some(id) != primary));
        }

        // Common/Inherited text such as emoji and mathematical symbols may not
        // have a script fallback. Ask Fontique's platform generic mappings only
        // after the normal script fallback misses.
        let last_resort = self.candidates_for_scope(query, CandidateScope::LastResort);
        if let Some(id) = self.first_supporting(&last_resort, grapheme) {
            return Some((id, Some(id) != primary));
        }

        primary_candidates
            .iter()
            .chain(fallback.iter())
            .chain(last_resort.iter())
            .copied()
            .find(|id| self.shape_resources(*id).is_some())
            .map(|id| (id, Some(id) != primary))
    }

    fn first_supporting(&self, candidates: &[FFontId], grapheme: &str) -> Option<FFontId> {
        candidates
            .iter()
            .copied()
            .find(|id| self.supports_grapheme(*id, grapheme))
    }

    fn supports_grapheme(&self, id: FFontId, grapheme: &str) -> bool {
        let Some(face) = self.face(id) else {
            return false;
        };
        let Some(charmap) = face.charmap_index.charmap(face.blob.as_ref()) else {
            return false;
        };
        grapheme
            .chars()
            .all(|character| !requires_glyph(character) || charmap.map(character).is_some())
    }

    fn candidates_for_scope(&mut self, query: &FontQuery, scope: CandidateScope) -> Vec<FFontId> {
        const MAX_CANDIDATE_QUERIES: usize = 256;
        let cache_key = (QueryKey::from(query), scope);
        if let Some(cached) = self.candidates.get(&cache_key) {
            return cached.clone();
        }

        let attributes = attributes(query);
        let mut selected = Vec::new();
        {
            let mut font_query = self.collection.query(&mut self.source_cache);
            match scope {
                CandidateScope::Primary => {
                    font_query.set_families(query_families(&query.families));
                }
                CandidateScope::Fallback(tag) => {
                    font_query.set_families(query_families(&query.families));
                    font_query.set_fallbacks(FallbackKey::new(FqScript::from_bytes(tag), None));
                }
                CandidateScope::LastResort => {
                    font_query.set_families([
                        QueryFamily::Generic(GenericFamily::Emoji),
                        QueryFamily::Generic(GenericFamily::Math),
                        QueryFamily::Generic(GenericFamily::SansSerif),
                    ]);
                }
            }
            font_query.set_attributes(attributes);
            font_query.matches_with(|font| {
                selected.push(font.clone());
                QueryStatus::Continue
            });
        }

        let mut ids = Vec::with_capacity(selected.len());
        for selected in selected {
            if let Some(id) = self.register_query_font(selected)
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
        // Fontique bounds the expensive source blobs through SourceCache. This
        // small front cache avoids repeating callbacks for hot style/script
        // combinations while preventing arbitrary family names from growing
        // the application cache forever.
        if self.candidates.len() >= MAX_CANDIDATE_QUERIES {
            self.candidates.clear();
        }
        self.candidates.insert(cache_key, ids.clone());
        ids
    }

    fn register_query_font(&mut self, selected: QueryFont) -> Option<FFontId> {
        let family_info = self.collection.family(selected.family.0)?;
        let font_info = family_info.fonts().get(selected.family.1)?;
        self.register_materialized(MaterializedFont {
            blob: selected.blob,
            index: selected.index,
            family: family_info.name().to_owned(),
            charmap_index: selected.charmap_index,
            weight: xui_weight(font_info.weight()),
            style: xui_style(font_info.style()),
            stretch: xui_stretch(font_info.width()),
            custom: false,
        })
    }

    fn register_materialized(&mut self, font: MaterializedFont) -> Option<FFontId> {
        let key = (font.blob.id(), font.index);
        if let Some(id) = self.face_ids.get(&key) {
            return Some(*id);
        }
        let raw = skrifa::FontRef::from_index(font.blob.as_ref(), font.index).ok()?;
        let metrics = raw.metrics(SkrifaSize::unscaled(), LocationRef::default());
        let units_per_em = metrics.units_per_em.max(1) as f32;
        let postscript_name = raw
            .localized_strings(StringId::POSTSCRIPT_NAME)
            .english_or_first()
            .map(|name| name.to_string())
            .unwrap_or_default();
        let id = FFontId(self.faces.len().try_into().ok()?);
        self.faces.push(FaceRecord {
            blob: font.blob,
            index: font.index,
            family: font.family,
            postscript_name,
            weight: font.weight,
            style: font.style,
            stretch: font.stretch,
            metrics: FaceMetrics {
                ascent: (metrics.ascent / units_per_em).clamp(0.0, 2.0),
                // Skrifa reports the descender as a negative offset from the
                // baseline; line boxes need it as a downward distance.
                descent: (-metrics.descent / units_per_em).clamp(0.0, 2.0),
                line_gap: (metrics.leading / units_per_em).clamp(0.0, 2.0),
            },
            custom: font.custom,
            charmap_index: font.charmap_index,
            shaper_data: None,
        });
        self.face_ids.insert(key, id);
        Some(id)
    }

    pub fn shape_resources(&mut self, id: FFontId) -> Option<(&[u8], u32, &ShaperData)> {
        let face = self.faces.get_mut(id.0 as usize)?;
        if face.shaper_data.is_none() {
            let font = FontRef::from_index(face.blob.as_ref(), face.index).ok()?;
            face.shaper_data = Some(ShaperData::new(&font));
        }
        Some((face.blob.as_ref(), face.index, face.shaper_data.as_ref()?))
    }

    pub fn prune_sources(&mut self, max_age: u64) {
        self.source_cache.prune(max_age, true);
    }

    pub fn font_data(&self, id: FFontId) -> Option<FontDataRef<'_>> {
        let face = self.face(id)?;
        if face.custom {
            Some(FontDataRef::Bytes {
                bytes: face.blob.as_ref(),
                index: face.index,
            })
        } else {
            Some(FontDataRef::SystemMemory {
                bytes: face.blob.as_ref(),
                index: face.index,
                family: &face.family,
                postscript_name: &face.postscript_name,
                weight: face.weight,
                style: face.style,
                stretch: face.stretch,
            })
        }
    }
}

struct MaterializedFont {
    blob: Blob<u8>,
    index: u32,
    family: String,
    charmap_index: CharmapIndex,
    weight: FontWeight,
    style: FontStyle,
    stretch: FontStretch,
    custom: bool,
}

fn query_families(families: &[FontFamily]) -> Vec<QueryFamily<'_>> {
    let mut output = Vec::new();
    for family in families {
        match family {
            FontFamily::System => {
                output.push(QueryFamily::Generic(GenericFamily::SystemUi));
                output.push(QueryFamily::Generic(GenericFamily::SansSerif));
            }
            FontFamily::Named(name) => output.push(QueryFamily::Named(name)),
            FontFamily::Stack(names) => {
                output.extend(names.iter().map(|name| QueryFamily::Named(name)))
            }
        }
    }
    if output.is_empty() {
        output.push(QueryFamily::Generic(GenericFamily::SystemUi));
        output.push(QueryFamily::Generic(GenericFamily::SansSerif));
    }
    output
}

fn attributes(query: &FontQuery) -> Attributes {
    Attributes::new(
        fontique_width(query.stretch),
        fontique_style(query.style),
        FqFontWeight::new(weight_number(query.weight) as f32),
    )
}

fn requires_glyph(character: char) -> bool {
    !character.is_whitespace()
        && !character.is_control()
        && character != '\u{200d}'
        && !(0xfe00..=0xfe0f).contains(&(character as u32))
        && !(0xe0100..=0xe01ef).contains(&(character as u32))
}

fn weight_number(weight: FontWeight) -> u16 {
    match weight {
        FontWeight::Thin => 100,
        FontWeight::ExtraLight => 200,
        FontWeight::Light => 300,
        FontWeight::Normal => 400,
        FontWeight::Medium => 500,
        FontWeight::SemiBold => 600,
        FontWeight::Bold => 700,
        FontWeight::ExtraBold => 800,
        FontWeight::Black => 900,
        FontWeight::Number(value) => value.clamp(1, 1000),
    }
}

fn fontique_width(stretch: FontStretch) -> FqFontWidth {
    match stretch {
        FontStretch::UltraCondensed => FqFontWidth::ULTRA_CONDENSED,
        FontStretch::ExtraCondensed => FqFontWidth::EXTRA_CONDENSED,
        FontStretch::Condensed => FqFontWidth::CONDENSED,
        FontStretch::SemiCondensed => FqFontWidth::SEMI_CONDENSED,
        FontStretch::Normal => FqFontWidth::NORMAL,
        FontStretch::SemiExpanded => FqFontWidth::SEMI_EXPANDED,
        FontStretch::Expanded => FqFontWidth::EXPANDED,
        FontStretch::ExtraExpanded => FqFontWidth::EXTRA_EXPANDED,
        FontStretch::UltraExpanded => FqFontWidth::ULTRA_EXPANDED,
    }
}

fn fontique_style(style: FontStyle) -> FqFontStyle {
    match style {
        FontStyle::Normal => FqFontStyle::Normal,
        FontStyle::Italic => FqFontStyle::Italic,
        FontStyle::Oblique => FqFontStyle::Oblique(None),
    }
}

fn xui_weight(weight: FqFontWeight) -> FontWeight {
    match weight.value().round().clamp(1.0, 1000.0) as u16 {
        100 => FontWeight::Thin,
        200 => FontWeight::ExtraLight,
        300 => FontWeight::Light,
        400 => FontWeight::Normal,
        500 => FontWeight::Medium,
        600 => FontWeight::SemiBold,
        700 => FontWeight::Bold,
        800 => FontWeight::ExtraBold,
        900 => FontWeight::Black,
        value => FontWeight::Number(value),
    }
}

fn xui_style(style: FqFontStyle) -> FontStyle {
    match style {
        FqFontStyle::Normal => FontStyle::Normal,
        FqFontStyle::Italic => FontStyle::Italic,
        FqFontStyle::Oblique(_) => FontStyle::Oblique,
    }
}

fn xui_stretch(width: FqFontWidth) -> FontStretch {
    let choices = [
        (FqFontWidth::ULTRA_CONDENSED, FontStretch::UltraCondensed),
        (FqFontWidth::EXTRA_CONDENSED, FontStretch::ExtraCondensed),
        (FqFontWidth::CONDENSED, FontStretch::Condensed),
        (FqFontWidth::SEMI_CONDENSED, FontStretch::SemiCondensed),
        (FqFontWidth::NORMAL, FontStretch::Normal),
        (FqFontWidth::SEMI_EXPANDED, FontStretch::SemiExpanded),
        (FqFontWidth::EXPANDED, FontStretch::Expanded),
        (FqFontWidth::EXTRA_EXPANDED, FontStretch::ExtraExpanded),
        (FqFontWidth::ULTRA_EXPANDED, FontStretch::UltraExpanded),
    ];
    choices
        .into_iter()
        .min_by(|(left, _), (right, _)| {
            (width.ratio() - left.ratio())
                .abs()
                .total_cmp(&(width.ratio() - right.ratio()).abs())
        })
        .map(|(_, value)| value)
        .unwrap_or(FontStretch::Normal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn grapheme_iteration_does_not_split_emoji_sequences() {
        let graphemes: Vec<_> = "A👩‍💻B".graphemes(true).collect();
        assert_eq!(graphemes, ["A", "👩‍💻", "B"]);
    }

    #[test]
    fn invalid_data_does_not_advance_epoch() {
        let mut store = FontStore::empty();
        let epoch = store.epoch();
        assert_eq!(
            store.load_font_bytes(Arc::from(&b"not a font"[..])),
            FFontId::INVALID
        );
        assert_eq!(store.epoch(), epoch);
    }
}
