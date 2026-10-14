use std::{collections::HashMap, hash::Hash};

use etagere::{AllocId, AtlasAllocator, size2};
use xui_interface::{
    GlyphId, GlyphImage, GlyphRasterizer, GlyphRequest, RasterOptions, RasterizedGlyphFormat,
    SubpixelOffset,
};

/// Size and growth limits of a [`GlyphAtlas`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasOptions {
    /// Width and height of every page, in texels. A renderer lowers it to
    /// what its device allows.
    pub page_size: u32,
    /// Pages kept per [`AtlasFormat`] between frames. Past this many, glyphs
    /// the frame has not used are evicted to make room, and a page opened
    /// beyond it because one frame needed more is closed again once no frame
    /// uses it.
    pub budget_pages: usize,
    /// Pages per format no frame goes beyond. A glyph that does not fit then
    /// is not placed, and its batch reports it ([`TextBatch::incomplete`]).
    ///
    /// [`TextBatch::incomplete`]: crate::TextBatch::incomplete
    pub max_pages: usize,
    /// Glyphs up to this many physical pixels per em are rendered at
    /// quarter-pixel horizontal offsets; larger ones are snapped to whole
    /// pixels, where an offset is hard to see and its up to four bitmaps a
    /// glyph cost room (CJK text at HiDPI sizes most of all). `0` snaps all.
    pub subpixel_max_ppem: f32,
    /// How glyphs are rendered into the atlas.
    pub raster: RasterOptions,
}

impl Default for AtlasOptions {
    /// Two 2048-texel pages per format, which hold about 7,000 CJK glyphs at
    /// 16px on a 2x display, and room to grow to eight for a frame that needs
    /// it.
    fn default() -> Self {
        Self {
            page_size: 2048,
            budget_pages: 2,
            max_pages: 8,
            subpixel_max_ppem: 24.0,
            raster: RasterOptions::default(),
        }
    }
}

/// Texel layout of an atlas page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtlasFormat {
    /// One coverage byte per texel, for [`RasterizedGlyphFormat::Mask`].
    Alpha8,
    /// Four bytes per texel, RGBA, for colour glyphs and subpixel masks.
    Rgba8,
}

impl AtlasFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Alpha8 => 1,
            Self::Rgba8 => 4,
        }
    }

    fn of(format: RasterizedGlyphFormat) -> Self {
        match format {
            RasterizedGlyphFormat::Mask => Self::Alpha8,
            RasterizedGlyphFormat::SubpixelMask | RasterizedGlyphFormat::Color => Self::Rgba8,
        }
    }
}

/// The space of one texture: which rectangles of it glyphs occupy. The
/// texels themselves are the renderer's.
pub struct AtlasPage {
    id: u64,
    format: AtlasFormat,
    size: u32,
    allocator: AtlasAllocator,
}

impl AtlasPage {
    /// Unique for the life of the atlas: a page index can be reused after its
    /// page is closed, by a page with another id.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn format(&self) -> AtlasFormat {
        self.format
    }

    /// Width and height in texels.
    pub fn size(&self) -> u32 {
        self.size
    }
}

/// A glyph's place in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasGlyph {
    /// Index of its page ([`GlyphAtlas::page`]).
    pub page: usize,
    pub format: RasterizedGlyphFormat,
    /// Texel rectangle of the bitmap in its page.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Offset of the bitmap's left edge from the pen position, in device
    /// pixels.
    pub left: i32,
    /// Distance from the baseline up to the bitmap's top edge, in device
    /// pixels.
    pub top: i32,
}

/// An [`AtlasGlyph`] positioned for drawing: `(x, y)` is the device-pixel
/// top-left corner of its `width × height` quad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedGlyph {
    pub glyph: AtlasGlyph,
    pub x: i32,
    pub y: i32,
}

/// What [`GlyphAtlas::glyph`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphLookup {
    pub placed: PlacedGlyph,
    /// The bitmap of a glyph the atlas has just made room for. Its pixels
    /// belong in `placed.glyph`'s rectangle of its page, and must be written
    /// there before the glyph is drawn; a glyph found in the atlas has `None`.
    pub upload: Option<GlyphImage>,
}

/// A glyph with ink that the atlas has no room for: larger than a page, or
/// the frame's glyphs fill [`AtlasOptions::max_pages`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasFull;

struct Slot {
    /// `None` for a glyph without ink.
    placed: Option<(AtlasGlyph, AllocId)>,
    last_used: u64,
}

/// Where glyphs live in a set of fixed-size texture pages.
///
/// The atlas only allocates: [`etagere`] packs rectangles into each page, and
/// the renderer owning the textures writes a glyph's bitmap into its
/// rectangle once, when [`glyph`](Self::glyph) hands it over. A slot freed by
/// eviction is reused as is, without clearing: glyphs are drawn by loading
/// exactly their own texels, so whatever is left around them is never read.
///
/// Call [`begin_frame`](Self::begin_frame) once per frame before looking up
/// glyphs. Glyphs used in the current frame are never evicted, so every
/// [`AtlasGlyph`] handed out during a frame stays valid until the next
/// `begin_frame`.
///
/// # When pages fill
///
/// Up to [`AtlasOptions::budget_pages`] per format, a full atlas opens a page.
/// Past that, it evicts what the frame has not used, the longest unused
/// first, a quarter of its capacity at a time, so a frame bringing in many
/// new glyphs (scrolling through CJK text) pays for one eviction pass, not one
/// a glyph. When the frame's own glyphs fill the budget, it opens more pages,
/// up to [`AtlasOptions::max_pages`], and closes them in a later
/// `begin_frame` once no frame uses them. A page can be closed and its index
/// reused; [`AtlasPage::id`] tells them apart.
///
/// `F` is the font id type of the collection glyphs are rendered from.
pub struct GlyphAtlas<F> {
    options: AtlasOptions,
    pages: Vec<Option<AtlasPage>>,
    next_page_id: u64,
    slots: HashMap<GlyphRequest<F>, Slot>,
    frame: u64,
    font_epoch: Option<u64>,
}

impl<F: Copy + Eq + Hash> GlyphAtlas<F> {
    pub fn new(options: AtlasOptions) -> Self {
        let budget_pages = options.budget_pages.max(1);
        Self {
            options: AtlasOptions {
                page_size: options.page_size.clamp(16, 16384),
                budget_pages,
                max_pages: options.max_pages.max(budget_pages),
                ..options
            },
            pages: Vec::new(),
            next_page_id: 0,
            slots: HashMap::new(),
            frame: 0,
            font_epoch: None,
        }
    }

    pub fn options(&self) -> AtlasOptions {
        self.options
    }

    /// Starts a frame of lookups. `font_epoch` is the collection's current
    /// `FontCollection::epoch`: when it moved, font ids may refer to other
    /// fonts, so every glyph is forgotten. Pages beyond the budget that the
    /// last frame did not use are closed.
    pub fn begin_frame(&mut self, font_epoch: u64) {
        self.frame += 1;
        if self.font_epoch != Some(font_epoch) {
            self.clear();
            self.font_epoch = Some(font_epoch);
        }
        for format in [AtlasFormat::Alpha8, AtlasFormat::Rgba8] {
            self.reclaim(format);
        }
    }

    /// The page at `index`, if it is open.
    pub fn page(&self, index: usize) -> Option<&AtlasPage> {
        self.pages.get(index)?.as_ref()
    }

    /// The open pages and their indices.
    pub fn pages(&self) -> impl Iterator<Item = (usize, &AtlasPage)> {
        self.pages
            .iter()
            .enumerate()
            .filter_map(|(index, page)| Some((index, page.as_ref()?)))
    }

    /// Number of open pages.
    pub fn page_count(&self) -> usize {
        self.pages.iter().flatten().count()
    }

    /// Number of cached glyphs, inkless ones included.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Forgets every glyph. Pages within the budget stay open for reuse; the
    /// rest are closed at the next `begin_frame`.
    pub fn clear(&mut self) {
        self.slots.clear();
        for page in self.pages.iter_mut().flatten() {
            page.allocator.clear();
        }
    }

    /// Changes how glyphs are rendered from now on. Glyphs rendered the old
    /// way are no longer served and are evicted as space is needed.
    pub fn set_raster_options(&mut self, raster: RasterOptions) {
        self.options.raster = raster;
    }

    /// Finds `glyph` of `font` at `ppem` physical pixels per em, rendering it
    /// and making room for it if the atlas lacks it, and places it at the
    /// pen position `(x, y)`, given in physical pixels with `y` on the
    /// baseline.
    ///
    /// `Ok(None)` for a glyph without ink, which needs drawing no more than
    /// one that is placed; [`AtlasFull`] for one that has ink and no room.
    pub fn glyph<R: GlyphRasterizer<FontId = F> + ?Sized>(
        &mut self,
        rasterizer: &R,
        font: F,
        glyph: GlyphId,
        ppem: f32,
        x: f32,
        y: f32,
    ) -> Result<Option<GlyphLookup>, AtlasFull> {
        let (pen_x, subpixel) = if ppem <= self.options.subpixel_max_ppem {
            SubpixelOffset::split(x)
        } else {
            (x.round() as i32, SubpixelOffset::Zero)
        };
        let key = GlyphRequest::new(font, glyph, ppem, subpixel, self.options.raster);
        let (glyph, upload) = match self.lookup(&key) {
            Some(None) => return Ok(None),
            Some(Some(cached)) => (cached, None),
            None => {
                let Some(image) = rasterizer.rasterize(&key) else {
                    self.slots.insert(
                        key,
                        Slot {
                            placed: None,
                            last_used: self.frame,
                        },
                    );
                    return Ok(None);
                };
                (self.insert(key, &image)?, Some(image))
            }
        };
        Ok(Some(GlyphLookup {
            placed: PlacedGlyph {
                x: pen_x + glyph.left,
                y: y.round() as i32 - glyph.top,
                glyph,
            },
            upload,
        }))
    }

    /// `Some(entry)` when `key` was rendered before, marking it used this
    /// frame; `None` when it still has to be rendered.
    fn lookup(&mut self, key: &GlyphRequest<F>) -> Option<Option<AtlasGlyph>> {
        let slot = self.slots.get_mut(key)?;
        slot.last_used = self.frame;
        Some(slot.placed.map(|(glyph, _)| glyph))
    }

    /// Makes room for a freshly rendered glyph. A glyph that does not fit is
    /// not remembered, so it is tried again once room frees up.
    fn insert(
        &mut self,
        key: GlyphRequest<F>,
        bitmap: &GlyphImage,
    ) -> Result<AtlasGlyph, AtlasFull> {
        let page_size = self.options.page_size;
        if bitmap.width > page_size || bitmap.height > page_size {
            return Err(AtlasFull);
        }
        let format = AtlasFormat::of(bitmap.format);
        let (page, allocation) = self.allocate(format, bitmap.width, bitmap.height)?;
        let origin = allocation.rectangle.min;
        let glyph = AtlasGlyph {
            page,
            format: bitmap.format,
            x: origin.x as u32,
            y: origin.y as u32,
            width: bitmap.width,
            height: bitmap.height,
            left: bitmap.left,
            top: bitmap.top,
        };
        self.slots.insert(
            key,
            Slot {
                placed: Some((glyph, allocation.id)),
                last_used: self.frame,
            },
        );
        Ok(glyph)
    }

    fn allocate(
        &mut self,
        format: AtlasFormat,
        width: u32,
        height: u32,
    ) -> Result<(usize, etagere::Allocation), AtlasFull> {
        if let Some(found) = self.allocate_in_open(format, width, height) {
            return Ok(found);
        }
        let open = self.open_pages(format);
        if open < self.options.budget_pages {
            return self.open_page(format, width, height);
        }
        if self.evict_stale(format, u64::from(width) * u64::from(height))
            && let Some(found) = self.allocate_in_open(format, width, height)
        {
            return Ok(found);
        }
        if open < self.options.max_pages {
            return self.open_page(format, width, height);
        }
        Err(AtlasFull)
    }

    fn allocate_in_open(
        &mut self,
        format: AtlasFormat,
        width: u32,
        height: u32,
    ) -> Option<(usize, etagere::Allocation)> {
        let size = size2(width as i32, height as i32);
        self.pages.iter_mut().enumerate().find_map(|(index, page)| {
            let page = page.as_mut().filter(|page| page.format == format)?;
            Some((index, page.allocator.allocate(size)?))
        })
    }

    fn open_pages(&self, format: AtlasFormat) -> usize {
        self.pages
            .iter()
            .flatten()
            .filter(|page| page.format == format)
            .count()
    }

    fn open_page(
        &mut self,
        format: AtlasFormat,
        width: u32,
        height: u32,
    ) -> Result<(usize, etagere::Allocation), AtlasFull> {
        let size = self.options.page_size;
        let mut page = AtlasPage {
            id: self.next_page_id,
            format,
            size,
            allocator: AtlasAllocator::new(size2(size as i32, size as i32)),
        };
        self.next_page_id += 1;
        let allocation = page
            .allocator
            .allocate(size2(width as i32, height as i32))
            .ok_or(AtlasFull)?;
        let index = match self.pages.iter().position(Option::is_none) {
            Some(index) => {
                self.pages[index] = Some(page);
                index
            }
            None => {
                self.pages.push(Some(page));
                self.pages.len() - 1
            }
        };
        Ok((index, allocation))
    }

    /// Evicts glyphs of `format` the current frame has not used, the longest
    /// unused first, until a quarter of the format's capacity (and at least
    /// `area`) is free again. Whether any were.
    fn evict_stale(&mut self, format: AtlasFormat, area: u64) -> bool {
        let capacity = self.open_pages(format) as u64 * u64::from(self.options.page_size).pow(2);
        let target = area.max(capacity / 4);
        let mut stale: Vec<_> = self
            .slots
            .iter()
            .filter_map(|(key, slot)| {
                let (glyph, _) = slot.placed?;
                (slot.last_used < self.frame && AtlasFormat::of(glyph.format) == format)
                    .then_some((slot.last_used, *key))
            })
            .collect();
        if stale.is_empty() {
            return false;
        }
        stale.sort_unstable_by_key(|(last_used, _)| *last_used);
        let mut freed = 0;
        for (_, key) in stale {
            if freed >= target {
                break;
            }
            let Some(Slot {
                placed: Some((glyph, id)),
                ..
            }) = self.slots.remove(&key)
            else {
                continue;
            };
            if let Some(page) = self.pages[glyph.page].as_mut() {
                page.allocator.deallocate(id);
            }
            freed += u64::from(glyph.width) * u64::from(glyph.height);
        }
        true
    }

    /// Closes pages of `format` beyond the budget that the last frame did
    /// not use, with the glyphs in them, the longest unused first.
    fn reclaim(&mut self, format: AtlasFormat) {
        let mut open = self.open_pages(format);
        if open <= self.options.budget_pages {
            return;
        }
        // The frame each page was last used in; 0 for an empty page.
        let mut last_used: HashMap<usize, u64> = self
            .pages()
            .filter(|(_, page)| page.format == format)
            .map(|(index, _)| (index, 0))
            .collect();
        for slot in self.slots.values() {
            if let Some((glyph, _)) = slot.placed
                && let Some(used) = last_used.get_mut(&glyph.page)
            {
                *used = (*used).max(slot.last_used);
            }
        }
        let mut idle: Vec<_> = last_used
            .into_iter()
            .filter(|(_, used)| *used + 1 < self.frame)
            .collect();
        idle.sort_unstable_by_key(|(_, used)| *used);
        for (index, _) in idle {
            if open <= self.options.budget_pages {
                break;
            }
            self.pages[index] = None;
            self.slots
                .retain(|_, slot| slot.placed.is_none_or(|(glyph, _)| glyph.page != index));
            open -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn key(glyph: u32) -> GlyphRequest<u32> {
        GlyphRequest::new(
            0,
            glyph,
            16.0,
            SubpixelOffset::Zero,
            RasterOptions::default(),
        )
    }

    fn mask(side: u32) -> GlyphImage {
        GlyphImage {
            format: RasterizedGlyphFormat::Mask,
            left: 1,
            top: side as i32,
            width: side,
            height: side,
            pixels: Arc::from(vec![255u8; (side * side) as usize]),
        }
    }

    /// Pages of 32 texels, holding four 16-texel glyphs each.
    fn small_atlas(budget_pages: usize, max_pages: usize) -> GlyphAtlas<u32> {
        let mut atlas = GlyphAtlas::new(AtlasOptions {
            page_size: 32,
            budget_pages,
            max_pages,
            ..AtlasOptions::default()
        });
        atlas.begin_frame(0);
        atlas
    }

    fn overlaps(a: &AtlasGlyph, b: &AtlasGlyph) -> bool {
        a.page == b.page
            && a.x < b.x + b.width
            && b.x < a.x + a.width
            && a.y < b.y + b.height
            && b.y < a.y + a.height
    }

    #[test]
    fn glyphs_get_disjoint_rectangles_and_are_found_again() {
        let mut atlas = small_atlas(1, 1);
        let first = atlas.insert(key(1), &mask(4)).unwrap();
        let second = atlas.insert(key(2), &mask(4)).unwrap();
        assert!(!overlaps(&first, &second));
        assert_eq!(atlas.page_count(), 1);
        assert_eq!(atlas.page(0).unwrap().format(), AtlasFormat::Alpha8);
        assert_eq!(atlas.lookup(&key(1)), Some(Some(first)));
    }

    #[test]
    fn a_full_budget_evicts_what_the_frame_has_not_used() {
        let mut atlas = small_atlas(1, 1);
        for id in 0..4 {
            assert!(atlas.insert(key(id), &mask(16)).is_ok());
        }
        // Same frame, no page to grow into: the fifth glyph has no room.
        assert_eq!(atlas.insert(key(4), &mask(16)), Err(AtlasFull));
        assert!(atlas.lookup(&key(4)).is_none(), "a full atlas is retried");

        atlas.begin_frame(0);
        let kept = atlas.lookup(&key(0)).unwrap().unwrap();
        let fifth = atlas.insert(key(4), &mask(16)).unwrap();
        assert!(
            !overlaps(&kept, &fifth),
            "a glyph used this frame keeps its texels"
        );
        assert_eq!(atlas.lookup(&key(0)), Some(Some(kept)));
        assert_eq!(atlas.page_count(), 1);
    }

    #[test]
    fn eviction_frees_a_quarter_of_the_atlas_at_once() {
        let mut atlas = small_atlas(1, 1);
        // Fill the page with 8-texel glyphs.
        let mut filled = 0;
        while atlas.insert(key(filled), &mask(8)).is_ok() {
            filled += 1;
        }
        let filled = filled as usize;
        atlas.begin_frame(0);
        atlas.insert(key(100), &mask(8)).unwrap();
        // A quarter of 32² is four 8² glyphs: the next three fit without
        // another eviction.
        assert_eq!(atlas.len(), filled - 4 + 1);
        for id in 101..104 {
            atlas.insert(key(id), &mask(8)).unwrap();
        }
        assert_eq!(atlas.len(), filled);
    }

    #[test]
    fn a_frame_needing_more_grows_past_the_budget_and_shrinks_back() {
        let mut atlas = small_atlas(1, 3);
        // Eight 16-texel glyphs in one frame: two pages, one over budget.
        for id in 0..8 {
            atlas.insert(key(id), &mask(16)).unwrap();
        }
        assert_eq!(atlas.page_count(), 2);
        let second = atlas.lookup(&key(7)).unwrap().unwrap().page;
        let second_id = atlas.page(second).unwrap().id();

        // Both pages are used by the next frame, so both stay.
        atlas.begin_frame(0);
        atlas.lookup(&key(0));
        atlas.lookup(&key(7));
        atlas.begin_frame(0);
        assert_eq!(atlas.page_count(), 2);
        // Only the first is used now: the idle one closes, with its glyphs.
        atlas.lookup(&key(0));
        atlas.begin_frame(0);
        assert_eq!(atlas.page_count(), 1);
        assert!(atlas.lookup(&key(7)).is_none());
        assert!(atlas.lookup(&key(0)).is_some());

        // Its index comes back with a page of another id.
        for id in 10..15 {
            atlas.insert(key(id), &mask(16)).unwrap();
        }
        assert_eq!(atlas.page_count(), 2);
        assert_ne!(atlas.page(second).unwrap().id(), second_id);
    }

    #[test]
    fn the_hard_limit_reports_a_full_atlas() {
        let mut atlas = small_atlas(1, 2);
        for id in 0..8 {
            atlas.insert(key(id), &mask(16)).unwrap();
        }
        assert_eq!(atlas.insert(key(8), &mask(16)), Err(AtlasFull));
        assert_eq!(
            atlas.insert(key(9), &mask(40)),
            Err(AtlasFull),
            "larger than a page"
        );
    }

    #[test]
    fn colour_glyphs_get_their_own_rgba_pages() {
        let mut atlas = small_atlas(1, 1);
        let mask_glyph = atlas.insert(key(0), &mask(2)).unwrap();
        let colour = atlas
            .insert(
                key(1),
                &GlyphImage {
                    format: RasterizedGlyphFormat::Color,
                    pixels: Arc::from(vec![255u8; 16]),
                    ..mask(2)
                },
            )
            .unwrap();
        assert_ne!(mask_glyph.page, colour.page);
        assert_eq!(
            atlas.page(colour.page).unwrap().format(),
            AtlasFormat::Rgba8
        );
    }

    #[test]
    fn a_new_font_epoch_forgets_every_glyph() {
        let mut atlas = small_atlas(1, 1);
        atlas.insert(key(1), &mask(2)).unwrap();
        atlas.begin_frame(0);
        assert!(atlas.lookup(&key(1)).is_some());
        atlas.begin_frame(1);
        assert!(atlas.lookup(&key(1)).is_none());
    }

    struct One;

    impl GlyphRasterizer for One {
        type FontId = u32;
        fn rasterize(&self, request: &GlyphRequest<u32>) -> Option<GlyphImage> {
            (request.glyph != 0).then(|| mask(3))
        }
    }

    #[test]
    fn hands_over_the_bitmap_only_when_it_makes_room() {
        let mut atlas = small_atlas(1, 1);
        let first = atlas.glyph(&One, 0, 7, 16.0, 10.0, 20.0).unwrap().unwrap();
        assert_eq!(first.upload, Some(mask(3)));
        assert_eq!(first.placed.x, 10 + 1);
        assert_eq!(first.placed.y, 20 - 3);
        let again = atlas.glyph(&One, 0, 7, 16.0, 30.0, 20.0).unwrap().unwrap();
        assert_eq!(again.upload, None);
        assert_eq!(again.placed.glyph, first.placed.glyph);
        assert_eq!(atlas.glyph(&One, 0, 0, 16.0, 0.0, 0.0), Ok(None), "no ink");
    }

    #[test]
    fn large_glyphs_snap_to_whole_pixels() {
        let mut atlas = small_atlas(1, 1);
        // Quarter-pixel offsets at and below the threshold: two bitmaps.
        let a = atlas.glyph(&One, 0, 7, 24.0, 10.0, 20.0).unwrap().unwrap();
        let b = atlas.glyph(&One, 0, 7, 24.0, 10.5, 20.0).unwrap().unwrap();
        assert_ne!(a.placed.glyph, b.placed.glyph);
        // Above it, one bitmap at the nearest pixel.
        let c = atlas.glyph(&One, 0, 7, 32.0, 10.0, 20.0).unwrap().unwrap();
        let d = atlas.glyph(&One, 0, 7, 32.0, 10.4, 20.0).unwrap().unwrap();
        assert_eq!(c.placed.glyph, d.placed.glyph);
        assert!(d.upload.is_none());
        assert_eq!(d.placed.x, 10 + 1);
    }
}
