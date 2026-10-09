//! Isolated layers' content, kept between frames.
//!
//! A layer whose scene node asks to be cached (`CachePolicy::Auto` or
//! `Always`, as the Skia backend reads it) keeps the texture its content was
//! composed into: the whole of its content bounds, in its own pixels. The next
//! frame redraws only what the damage tracker says changed inside the layer,
//! into that texture, and runs the layer's plan from it. Nothing else of the
//! layer is redrawn; scrolling a cached page is a composite.
//!
//! A layer with effects keeps their result too: the layer branch of its plan,
//! run on its own over the whole of the content bounds grown by the effects.
//! It is recomputed where the content changed (the damage tracker grows a
//! layer's damage by its effects' reach), or whole when the effects did, and
//! the layer is composited from it by its program with that branch cut out.
//!
//! What the texture holds is right as long as every change to the layer has
//! been drawn into it. A frame that changes a cached layer without drawing it
//! (it is off screen, or outside the region being repainted) drops the entry,
//! since the next frame's damage only says what changed since this one.

use rustc_hash::{FxHashMap, FxHashSet};
use xui_core::render::{BuiltFrame, CachePolicy, LayerCacheId, RenderNodeId, damage::DamageTracker};
use xui_render_graph::{PixelRect, ProgramFingerprint};

use crate::filter::{FilterRenderer, Slot};

/// One cached layer's content.
pub(crate) struct Entry {
    pub source: RenderNodeId,
    /// What the texture covers, in the layer's physical pixels.
    pub rect: PixelRect,
    pub scale_bits: u32,
    pub policy: CachePolicy,
    pub slot: Slot,
    /// The layer branch's result, for a layer with effects.
    pub filtered: Option<Filtered>,
    last_used: u64,
}

/// A cached layer's effects, applied.
pub(crate) struct Filtered {
    pub slot: Slot,
    /// What the texture covers, in the layer's physical pixels: the content
    /// bounds grown by the effects.
    pub rect: PixelRect,
    /// The program whose layer branch made it.
    pub program: ProgramFingerprint,
}

/// A cached layer's use of its filtered result, decided by the encoder.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FilterUse {
    pub rect: PixelRect,
    pub program: ProgramFingerprint,
    pub update: CacheUpdate,
}

/// What a cached layer needs this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheUpdate {
    /// Nothing cached that fits: the content is drawn whole and kept.
    Fresh,
    /// Cached, with damage: the content is drawn there and copied in.
    Patch,
    /// Cached and unchanged: nothing is drawn.
    Keep,
}

/// A layer job's use of the cache, decided by the encoder.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CacheUse {
    pub id: LayerCacheId,
    pub source: RenderNodeId,
    pub policy: CachePolicy,
    pub rect: PixelRect,
    pub scale_bits: u32,
    pub update: CacheUpdate,
    /// For a layer with effects whose result is kept.
    pub filter: Option<FilterUse>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub hits: u64,
    pub misses: u64,
    pub patches: u64,
    pub filtered_hits: u64,
    pub filtered_misses: u64,
    pub filtered_patches: u64,
}

#[derive(Default)]
pub(crate) struct LayerCache {
    entries: FxHashMap<LayerCacheId, Entry>,
    /// Entries a job used this frame.
    used: FxHashSet<LayerCacheId>,
    frame: u64,
    counts: Counts,
}

impl LayerCache {
    pub(crate) fn get(&self, id: LayerCacheId) -> Option<&Entry> {
        self.entries.get(&id)
    }

    pub(crate) fn begin_frame(&mut self) {
        self.frame += 1;
        self.used.clear();
    }

    /// The texture a job reads its content from: `content` (just composed,
    /// for a fresh or patched use) goes into the cache. A patch is copied into
    /// the entry and released.
    pub(crate) fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        filters: &mut FilterRenderer,
        use_: CacheUse,
        content: Option<(Slot, (i32, i32))>,
    ) -> Slot {
        self.used.insert(use_.id);
        match (use_.update, content, self.entries.get_mut(&use_.id)) {
            (CacheUpdate::Keep, _, Some(entry)) => {
                self.counts.hits += 1;
                entry.last_used = self.frame;
                entry.slot
            }
            (CacheUpdate::Patch, Some((patch, origin)), Some(entry)) => {
                self.counts.hits += 1;
                self.counts.patches += 1;
                entry.last_used = self.frame;
                let at = (
                    (origin.0 - entry.rect.x) as u32,
                    (origin.1 - entry.rect.y) as u32,
                );
                filters.copy_into(device, queue, patch, entry.slot, at);
                filters.release(patch);
                entry.slot
            }
            (_, Some((slot, _)), _) => {
                self.counts.misses += 1;
                filters.pin(slot);
                let entry = Entry {
                    source: use_.source,
                    rect: use_.rect,
                    scale_bits: use_.scale_bits,
                    policy: use_.policy,
                    slot,
                    filtered: None,
                    last_used: self.frame,
                };
                if let Some(old) = self.entries.insert(use_.id, entry) {
                    release(filters, old);
                }
                slot
            }
            (_, None, _) => unreachable!("the encoder draws content for every use it cannot keep"),
        }
    }

    /// The texture a job's composite reads: the filtered result, after
    /// [`Self::apply`] has brought the content up to date. `filtered` (just
    /// computed, for a fresh or patched use) goes into the cache.
    pub(crate) fn apply_filtered(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        filters: &mut FilterRenderer,
        id: LayerCacheId,
        use_: FilterUse,
        filtered: Option<(Slot, (i32, i32))>,
    ) -> Slot {
        let entry = self.entries.get_mut(&id).expect("the content is applied first");
        match (use_.update, filtered, &entry.filtered) {
            (CacheUpdate::Keep, _, Some(kept)) => {
                self.counts.filtered_hits += 1;
                kept.slot
            }
            (CacheUpdate::Patch, Some((patch, origin)), Some(kept)) => {
                self.counts.filtered_hits += 1;
                self.counts.filtered_patches += 1;
                let at = ((origin.0 - kept.rect.x) as u32, (origin.1 - kept.rect.y) as u32);
                filters.copy_into(device, queue, patch, kept.slot, at);
                filters.release(patch);
                kept.slot
            }
            (_, Some((slot, _)), _) => {
                self.counts.filtered_misses += 1;
                filters.pin(slot);
                let new = Filtered {
                    slot,
                    rect: use_.rect,
                    program: use_.program,
                };
                if let Some(old) = entry.filtered.replace(new) {
                    filters.unpin(old.slot);
                }
                slot
            }
            (_, None, _) => unreachable!("the encoder filters for every use it cannot keep"),
        }
    }

    /// Drops what can no longer be trusted or is no longer wanted: entries for
    /// layers gone from the frame, entries that changed this frame without
    /// being drawn, and the least recently used `Auto` ones past `auto_budget`.
    pub(crate) fn finish_frame(
        &mut self,
        frame: &BuiltFrame,
        damage: &DamageTracker,
        filters: &mut FilterRenderer,
        auto_budget: u64,
    ) {
        let live: FxHashSet<_> = frame.live_layer_caches.iter().copied().collect();
        let used = &self.used;
        let stale: Vec<_> = self
            .entries
            .iter()
            .filter(|(id, entry)| {
                !live.contains(id) || (!used.contains(id) && !damage.layer(entry.source).is_empty())
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.evict(id, filters);
        }
        loop {
            let auto = || self.entries.iter().filter(|(_, entry)| entry.policy == CachePolicy::Auto);
            let bytes: u64 = auto().map(|(_, entry)| entry_bytes(filters, entry)).sum();
            if bytes <= auto_budget {
                break;
            }
            let Some(oldest) = auto().min_by_key(|(_, entry)| entry.last_used).map(|(id, _)| *id) else {
                break;
            };
            self.evict(oldest, filters);
        }
    }

    pub(crate) fn clear(&mut self, filters: &mut FilterRenderer) {
        for (_, entry) in self.entries.drain() {
            release(filters, entry);
        }
    }

    fn evict(&mut self, id: LayerCacheId, filters: &mut FilterRenderer) {
        if let Some(entry) = self.entries.remove(&id) {
            release(filters, entry);
        }
    }

    /// Entries, and the bytes of their textures, filtered results included.
    pub(crate) fn size(&self, filters: &FilterRenderer) -> (usize, u64) {
        (
            self.entries.len(),
            self.entries.values().map(|entry| entry_bytes(filters, entry)).sum(),
        )
    }

    pub(crate) fn counts(&self) -> Counts {
        self.counts
    }
}

/// Gives an entry's textures back to the pool.
fn release(filters: &mut FilterRenderer, entry: Entry) {
    filters.unpin(entry.slot);
    if let Some(filtered) = entry.filtered {
        filters.unpin(filtered.slot);
    }
}

fn entry_bytes(filters: &FilterRenderer, entry: &Entry) -> u64 {
    filters.bytes(entry.slot) + entry.filtered.as_ref().map_or(0, |f| filters.bytes(f.slot))
}
