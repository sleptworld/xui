use std::hash::Hash;

use moka::sync::Cache;
use xui_interface::GlyphImage;

/// Default byte budget for whole-glyph bitmaps handed out through
/// `GlyphRasterizer::rasterize`.
pub(crate) const DEFAULT_BUDGET_BYTES: u64 = 16 * 1024 * 1024;

/// What an entry costs besides its pixels, so caching many inkless glyphs
/// (spaces at every size) is bounded too.
const ENTRY_OVERHEAD: u32 = 64;

/// Rasterized glyphs bounded by their pixel bytes. Glyphs without ink are
/// cached as `None` so they are not re-rendered.
///
/// Moka evicts by TinyLFU, so a glyph drawn every frame survives a burst of
/// one-off glyphs (a scrolled-past paragraph in another script) that a plain
/// LRU would let flush it. Eviction runs in moka's maintenance passes, so the
/// budget can be exceeded briefly between them.
pub(crate) struct BitmapCache<K> {
    entries: Cache<K, Option<GlyphImage>>,
}

impl<K: Hash + Eq + Send + Sync + 'static> BitmapCache<K> {
    pub fn new(budget_bytes: u64) -> Self {
        Self {
            entries: Cache::builder()
                .max_capacity(budget_bytes)
                .weigher(|_, glyph: &Option<GlyphImage>| {
                    let pixels = glyph.as_ref().map_or(0, |glyph| glyph.pixels.len());
                    ENTRY_OVERHEAD.saturating_add(u32::try_from(pixels).unwrap_or(u32::MAX))
                })
                .build(),
        }
    }

    pub fn get(&self, key: &K) -> Option<Option<GlyphImage>> {
        self.entries.get(key)
    }

    pub fn insert(&self, key: K, glyph: Option<GlyphImage>) {
        self.entries.insert(key, glyph);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use xui_interface::RasterizedGlyphFormat;

    use super::*;

    fn glyph(bytes: usize) -> GlyphImage {
        GlyphImage {
            format: RasterizedGlyphFormat::Mask,
            width: 1,
            height: 1,
            left: 0,
            top: 0,
            pixels: Arc::from(vec![0u8; bytes]),
        }
    }

    #[test]
    fn stays_within_the_byte_budget() {
        let budget = 8 * (ENTRY_OVERHEAD as u64 + 100);
        let cache = BitmapCache::new(budget);
        for key in 0u32..64 {
            cache.insert(key, Some(glyph(100)));
        }
        cache.entries.run_pending_tasks();
        assert!(cache.entries.weighted_size() <= budget);
        assert!(cache.entries.entry_count() <= 8);
    }

    #[test]
    fn remembers_glyphs_without_ink() {
        let cache = BitmapCache::new(1024);
        cache.insert(0u32, None);
        assert!(matches!(cache.get(&0), Some(None)));
    }
}
