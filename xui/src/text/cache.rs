use std::{
    cell::Cell,
    collections::{HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    mem::{size_of, size_of_val},
    ops::Range,
    sync::Arc,
};

use quick_cache::{DefaultHashBuilder, Lifecycle, Weighter, unsync::Cache as QuickCache};
use rustc_hash::FxHashMap;
use slotmap::{SlotMap, new_key_type};
use smallvec::SmallVec;
use xui_interface::{
    Affinity, NodeId, NodeLifecycleEvent, ParagraphLayout, Point, Rect, Shaper, Size, TextBackend,
    TextLayoutConstraints, TextLayoutInput, TextLayoutKey, TextOffset, TextOffsetUnit,
    TextPosition, TextRange,
};

use super::{TextLayoutMetrics, TextLayoutQuery};

new_key_type! {
    /// Stable identity of one logical text unit.
    pub struct TextUnitId;

    /// Stable handle of one shaped layout result.
    pub struct TextLayoutHandle;

    /// Owner-local document identity.
    pub struct TextDocumentId;

    /// Document-local paragraph identity.
    pub struct ParagraphId;
}

const DEFAULT_ESTIMATED_LAYOUTS: usize = 128;
const DEFAULT_MAX_LAYOUT_BYTES: u64 = 32 * 1024 * 1024;
const DEFAULT_MAX_VARIANTS_PER_UNIT: usize = 4;

/// A direct, owner-local text slot. Documents use `ParagraphId` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextLayoutSlot(u32);

impl TextLayoutSlot {
    pub const PRIMARY: Self = Self(0);

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// The reverse location of a logical text unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextUnitLocation {
    Direct {
        owner: NodeId,
        slot: TextLayoutSlot,
    },
    Paragraph {
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    },
}

/// Metadata kept for a virtual-document paragraph independently of shaped layouts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParagraphInfo {
    pub id: ParagraphId,
    pub unit: TextUnitId,
    pub source_range: TextRange,
    pub text_revision: u64,
    pub style_revision: u64,
    pub estimated_height: f32,
    pub measured_height: Option<f32>,
}

impl ParagraphInfo {
    pub fn height(self) -> f32 {
        self.measured_height.unwrap_or(self.estimated_height)
    }
}

/// A text backend plus logical text ownership and globally bounded layout results.
///
/// Each [`TextUnitId`] owns one persistent shaper state. Width-specific layout
/// variants share that state but remain independently cached for rendering.
pub struct TextHost<B: TextBackend> {
    backend: B,
    units: SlotMap<TextUnitId, TextUnitCache<B::State>>,
    entries: SlotMap<TextLayoutHandle, LayoutEntry<B>>,
    residency: GlobalLayoutCache,
    owners: FxHashMap<NodeId, OwnerCache>,
    max_variants_per_unit: usize,
    access_epoch: Cell<u64>,
}

impl<B: TextBackend> TextHost<B> {
    pub fn new(backend: B) -> Self {
        Self::with_limits(backend, DEFAULT_ESTIMATED_LAYOUTS, DEFAULT_MAX_LAYOUT_BYTES)
    }

    pub fn with_limits(backend: B, estimated_layouts: usize, max_layout_bytes: u64) -> Self {
        Self::with_variant_limit(
            backend,
            estimated_layouts,
            max_layout_bytes,
            DEFAULT_MAX_VARIANTS_PER_UNIT,
        )
    }

    pub fn with_variant_limit(
        backend: B,
        estimated_layouts: usize,
        max_layout_bytes: u64,
        max_variants_per_unit: usize,
    ) -> Self {
        Self {
            backend,
            units: SlotMap::with_key(),
            entries: SlotMap::with_key(),
            residency: QuickCache::with(
                estimated_layouts.max(1),
                max_layout_bytes.max(1),
                LayoutWeighter,
                DefaultHashBuilder::default(),
                LayoutLifecycle,
            ),
            owners: FxHashMap::default(),
            max_variants_per_unit: max_variants_per_unit.max(1),
            access_epoch: Cell::new(0),
        }
    }

    pub fn max_variants_per_unit(&self) -> usize {
        self.max_variants_per_unit
    }

    pub fn set_max_variants_per_unit(&mut self, max_variants_per_unit: usize) {
        self.max_variants_per_unit = max_variants_per_unit.max(1);
        let units: SmallVec<[TextUnitId; 16]> = self.units.keys().collect();
        for unit in units {
            self.enforce_unit_variant_limit(unit);
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Returns or creates the stable unit behind a normal owner slot.
    pub fn ensure_direct_unit(&mut self, owner: NodeId, slot: TextLayoutSlot) -> TextUnitId {
        if let Some(unit) = self.direct_unit(owner, slot) {
            return unit;
        }

        let state = self.backend.create_state();
        let unit = self.units.insert(TextUnitCache {
            owner,
            location: TextUnitLocation::Direct { owner, slot },
            state,
            active: None,
            resident_layouts: SmallVec::new(),
        });
        self.owners
            .entry(owner)
            .or_default()
            .direct_slots
            .insert(slot, unit);
        unit
    }

    pub fn direct_unit(&self, owner: NodeId, slot: TextLayoutSlot) -> Option<TextUnitId> {
        self.owners.get(&owner)?.direct_slots.get(&slot).copied()
    }

    pub fn unit_location(&self, unit: TextUnitId) -> Option<TextUnitLocation> {
        Some(self.units.get(unit)?.location)
    }

    pub fn find_unit(&self, unit: TextUnitId, key: &TextLayoutKey) -> Option<TextLayoutHandle> {
        self.units.get(unit)?;
        let cache_key = LayoutCacheKey { unit, layout: *key };
        let handle = self.residency.get(&cache_key)?.handle;
        let entry = self.entries.get(handle)?;
        if entry.cache_key != cache_key {
            return None;
        }
        entry.last_access.set(self.next_access_epoch());
        Some(handle)
    }

    pub fn active_unit(&self, unit: TextUnitId) -> Option<TextLayoutHandle> {
        let (key, handle) = self.units.get(unit)?.active?;
        let cache_key = LayoutCacheKey { unit, layout: key };
        if self.residency.get(&cache_key)?.handle != handle {
            return None;
        }
        let entry = self.entries.get(handle)?;
        entry.last_access.set(self.next_access_epoch());
        Some(handle)
    }

    pub fn active_slot(&self, owner: NodeId, slot: TextLayoutSlot) -> Option<TextLayoutHandle> {
        self.active_unit(self.direct_unit(owner, slot)?)
    }

    /// Measures an owner-local slot without changing the layout used for paint.
    ///
    /// Layout engines may query the same text under several intrinsic and
    /// definite constraints. Those probes must not decide which variant is
    /// active for rendering.
    pub fn measure_slot(
        &mut self,
        owner: NodeId,
        slot: TextLayoutSlot,
        input: TextLayoutInput,
    ) -> Size<f32> {
        self.measure_slot_metrics(owner, slot, input).size
    }

    pub(crate) fn measure_slot_metrics(
        &mut self,
        owner: NodeId,
        slot: TextLayoutSlot,
        input: TextLayoutInput,
    ) -> TextLayoutMetrics {
        let unit = self.ensure_direct_unit(owner, slot);
        let key = layout_key_for_input(&input);
        self.measure_unit_metrics(unit, key, input)
            .expect("a newly ensured text unit must exist")
    }

    /// Measures a logical unit without changing its active layout.
    ///
    /// Document/rich-text callers provide the key because their style spans and
    /// source revisions can live outside `TextLayoutInput`.
    pub fn measure_unit(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        input: TextLayoutInput,
    ) -> Option<Size<f32>> {
        self.measure_unit_metrics(unit, key, input)
            .map(|metrics| metrics.size)
    }

    fn measure_unit_metrics(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        input: TextLayoutInput,
    ) -> Option<TextLayoutMetrics> {
        self.units.get(unit)?;
        if let Some(handle) = self.find_unit(unit, &key) {
            return self
                .layout(handle)
                .map(|layout| text_layout_metrics(&layout));
        }

        let layout = {
            let state = &mut self.units.get_mut(unit)?.state;
            Arc::new(self.backend.layout_paragraph(state, input.clone()))
        };
        let metrics = text_layout_metrics(&layout);
        let memory_cost = Self::estimated_memory_cost(&layout, &self.units.get(unit)?.state);
        if self
            .insert_entry(unit, key, layout, Some(input), memory_cost, false)
            .is_some()
        {
            self.enforce_unit_variant_limit(unit);
        }
        Some(metrics)
    }

    /// Returns a cached layout for an owner-local slot, or shapes and activates it.
    pub fn activate_slot(
        &mut self,
        owner: NodeId,
        slot: TextLayoutSlot,
        input: TextLayoutInput,
    ) -> TextLayoutHandle {
        let unit = self.ensure_direct_unit(owner, slot);
        let key = layout_key_for_input(&input);
        self.activate_unit(unit, key, input)
            .expect("a newly ensured text unit must exist")
    }

    /// Returns a cached layout for a logical unit, or shapes and activates it.
    pub fn activate_unit(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        input: TextLayoutInput,
    ) -> Option<TextLayoutHandle> {
        self.units.get(unit)?;
        if let Some(handle) = self.find_unit(unit, &key) {
            self.set_active_unit(unit, key, handle);
            return Some(handle);
        }

        let layout = {
            let state = &mut self.units.get_mut(unit)?.state;
            Arc::new(self.backend.layout_paragraph(state, input.clone()))
        };
        let memory_cost = Self::estimated_memory_cost(&layout, &self.units.get(unit)?.state);
        let handle = self.insert_entry(unit, key, layout, Some(input), memory_cost, true)?;
        self.replace_active_unit(unit, key, handle, true);
        self.enforce_unit_variant_limit(unit);
        Some(handle)
    }

    /// Backwards-compatible convenience for callers that intentionally shape
    /// and activate in one operation.
    pub fn get_or_shape_slot(
        &mut self,
        owner: NodeId,
        slot: TextLayoutSlot,
        input: TextLayoutInput,
    ) -> TextLayoutHandle {
        self.activate_slot(owner, slot, input)
    }

    /// Backwards-compatible convenience for callers that intentionally shape
    /// and activate in one operation.
    pub fn get_or_shape_unit(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        input: TextLayoutInput,
    ) -> Option<TextLayoutHandle> {
        self.activate_unit(unit, key, input)
    }

    pub fn layout(
        &self,
        handle: TextLayoutHandle,
    ) -> Option<Arc<ParagraphLayout<<B as Shaper>::FontId, <B as Shaper>::GlyphKey>>> {
        let entry = self.resident_entry(handle)?;
        Some(entry.layout.clone())
    }

    pub fn query(&self, handle: TextLayoutHandle) -> Option<&dyn TextLayoutQuery> {
        self.resident_entry(handle)
            .map(|entry| entry as &dyn TextLayoutQuery)
    }

    pub fn state(&self, handle: TextLayoutHandle) -> Option<&B::State> {
        let unit = self.resident_entry(handle)?.cache_key.unit;
        Some(&self.units.get(unit)?.state)
    }

    pub fn state_mut(&mut self, handle: TextLayoutHandle) -> Option<&mut B::State> {
        let cache_key = self.entries.get(handle)?.cache_key;
        self.residency.get(&cache_key)?;
        let last_access = self.next_access_epoch();
        let entry = self.entries.get_mut(handle)?;
        entry.last_access.set(last_access);
        Some(&mut self.units.get_mut(cache_key.unit)?.state)
    }

    pub fn insert_unit_variant(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        layout: Arc<ParagraphLayout<<B as Shaper>::FontId, <B as Shaper>::GlyphKey>>,
        state: B::State,
    ) -> Option<TextLayoutHandle> {
        let memory_cost = Self::estimated_memory_cost(&layout, &state);
        self.insert_unit_variant_with_cost(unit, key, layout, state, memory_cost)
    }

    pub fn insert_unit_variant_with_cost(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        layout: Arc<ParagraphLayout<<B as Shaper>::FontId, <B as Shaper>::GlyphKey>>,
        state: B::State,
        memory_cost: u64,
    ) -> Option<TextLayoutHandle> {
        if let Some(handle) = self.find_unit(unit, &key) {
            return Some(handle);
        }
        self.units.get_mut(unit)?.state = state;
        let handle = self.insert_entry(unit, key, layout, None, memory_cost, false)?;
        self.enforce_unit_variant_limit(unit);
        self.entries.contains_key(handle).then_some(handle)
    }

    pub fn insert_unit_active(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        layout: Arc<ParagraphLayout<<B as Shaper>::FontId, <B as Shaper>::GlyphKey>>,
        state: B::State,
    ) -> TextLayoutHandle {
        let memory_cost = Self::estimated_memory_cost(&layout, &state);
        self.insert_unit_active_with_cost(unit, key, layout, state, memory_cost)
    }

    pub fn insert_unit_active_with_cost(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        layout: Arc<ParagraphLayout<<B as Shaper>::FontId, <B as Shaper>::GlyphKey>>,
        state: B::State,
        memory_cost: u64,
    ) -> TextLayoutHandle {
        if let Some(handle) = self.find_unit(unit, &key)
            && self.set_active_unit(unit, key, handle)
        {
            return handle;
        }

        self.units
            .get_mut(unit)
            .expect("active text layout unit must exist")
            .state = state;
        let handle = self
            .insert_entry(unit, key, layout, None, memory_cost, true)
            .expect("a pinned text layout must be admitted to the cache");
        self.replace_active_unit(unit, key, handle, true);
        self.enforce_unit_variant_limit(unit);
        handle
    }

    pub fn set_active_unit(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        handle: TextLayoutHandle,
    ) -> bool {
        if self.find_unit(unit, &key) != Some(handle) || !self.pin(handle) {
            return false;
        }
        self.replace_active_unit(unit, key, handle, true);
        self.enforce_unit_variant_limit(unit);
        true
    }

    pub fn clear_active_unit(&mut self, unit: TextUnitId) -> Option<TextLayoutHandle> {
        let (_, handle) = self.units.get_mut(unit)?.active.take()?;
        self.unpin(handle);
        Some(handle)
    }

    pub fn pin(&mut self, handle: TextLayoutHandle) -> bool {
        let Some(cache_key) = self.entries.get(handle).map(|entry| entry.cache_key) else {
            return false;
        };
        let Some(mut residency) = self.residency.get_mut(&cache_key) else {
            return false;
        };
        if residency.handle != handle {
            return false;
        }
        residency.pin_count = residency
            .pin_count
            .checked_add(1)
            .expect("text layout pin count overflowed");
        true
    }

    pub fn unpin(&mut self, handle: TextLayoutHandle) -> bool {
        let Some(cache_key) = self.entries.get(handle).map(|entry| entry.cache_key) else {
            return false;
        };
        {
            let Some(mut residency) = self.residency.get_mut(&cache_key) else {
                return false;
            };
            if residency.handle != handle || residency.pin_count == 0 {
                return false;
            }
            residency.pin_count -= 1;
        }
        self.enforce_unit_variant_limit(cache_key.unit);
        true
    }

    pub fn remove(&mut self, handle: TextLayoutHandle) -> bool {
        let Some(cache_key) = self.entries.get(handle).map(|entry| entry.cache_key) else {
            return false;
        };
        self.residency.remove(&cache_key);
        self.remove_entry_and_indexes(handle)
    }

    /// Drops all shaped variants while preserving the logical unit identity.
    pub fn invalidate_unit(&mut self, unit: TextUnitId) -> usize {
        let Some(handles) = self
            .units
            .get(unit)
            .map(|cache| cache.resident_layouts.clone())
        else {
            return 0;
        };
        let mut removed = 0;
        for handle in handles {
            if self.remove(handle) {
                removed += 1;
            }
        }
        removed
    }

    pub fn invalidate_slot(&mut self, owner: NodeId, slot: TextLayoutSlot) -> usize {
        self.direct_unit(owner, slot)
            .map(|unit| self.invalidate_unit(unit))
            .unwrap_or(0)
    }

    /// Drops every shaped paragraph in a document but keeps its paragraph ids,
    /// ordering, revisions, and height estimates intact.
    pub fn invalidate_document(&mut self, owner: NodeId, document: TextDocumentId) -> usize {
        let Some(units) = self
            .owners
            .get(&owner)
            .and_then(|cache| cache.documents.get(document))
            .map(|document| {
                document
                    .paragraphs
                    .values()
                    .map(|paragraph| paragraph.unit)
                    .collect::<Vec<_>>()
            })
        else {
            return 0;
        };
        units
            .into_iter()
            .map(|unit| self.invalidate_unit(unit))
            .sum()
    }

    pub fn contains(&self, handle: TextLayoutHandle) -> bool {
        self.resident_entry(handle).is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.owners.clear();
        self.units.clear();
        self.residency.clear();
        self.entries.clear();
        self.access_epoch.set(0);
    }

    pub fn remove_slot(&mut self, owner: NodeId, slot: TextLayoutSlot) -> usize {
        let Some(unit) = self
            .owners
            .get_mut(&owner)
            .and_then(|cache| cache.direct_slots.remove(&slot))
        else {
            return 0;
        };
        let removed = self.remove_unit(unit);
        self.remove_owner_if_empty(owner);
        removed
    }

    /// Removes owner-local direct slots that are not present in `retained`.
    ///
    /// This is used by retained multi-text owners such as Canvas to reconcile
    /// stable text identities after their command list changes.
    pub fn retain_direct_slots(
        &mut self,
        owner: NodeId,
        retained: impl IntoIterator<Item = TextLayoutSlot>,
    ) -> usize {
        let retained: HashSet<_> = retained.into_iter().collect();
        let stale: Vec<_> = self
            .owners
            .get(&owner)
            .map(|cache| {
                cache
                    .direct_slots
                    .keys()
                    .copied()
                    .filter(|slot| !retained.contains(slot))
                    .collect()
            })
            .unwrap_or_default();
        stale
            .into_iter()
            .map(|slot| self.remove_slot(owner, slot))
            .sum()
    }

    pub fn remove_owner(&mut self, owner: NodeId) -> usize {
        self.owners.remove(&owner);
        let units: Vec<_> = self
            .units
            .iter()
            .filter_map(|(unit, cache)| (cache.owner == owner).then_some(unit))
            .collect();
        units.into_iter().map(|unit| self.remove_unit(unit)).sum()
    }

    pub fn create_document(&mut self, owner: NodeId) -> TextDocumentId {
        self.owners
            .entry(owner)
            .or_default()
            .documents
            .insert(DocumentCache::default())
    }

    pub fn remove_document(&mut self, owner: NodeId, document: TextDocumentId) -> usize {
        let Some(document_cache) = self
            .owners
            .get_mut(&owner)
            .and_then(|cache| cache.documents.remove(document))
        else {
            return 0;
        };
        let units: Vec<_> = document_cache
            .paragraphs
            .values()
            .map(|paragraph| paragraph.unit)
            .collect();
        let removed = units.into_iter().map(|unit| self.remove_unit(unit)).sum();
        self.remove_owner_if_empty(owner);
        removed
    }

    pub fn insert_paragraph(
        &mut self,
        owner: NodeId,
        document: TextDocumentId,
        index: usize,
        source_range: TextRange,
        text_revision: u64,
        style_revision: u64,
        estimated_height: f32,
    ) -> Option<ParagraphInfo> {
        self.owners.get(&owner)?.documents.get(document)?;

        let state = self.backend.create_state();
        let unit = self.units.insert(TextUnitCache {
            owner,
            // Replaced immediately after the paragraph id is allocated.
            location: TextUnitLocation::Direct {
                owner,
                slot: TextLayoutSlot::PRIMARY,
            },
            state,
            active: None,
            resident_layouts: SmallVec::new(),
        });
        let document_cache = self.owners.get_mut(&owner)?.documents.get_mut(document)?;
        let paragraph = document_cache.paragraphs.insert(ParagraphRecord {
            source_range,
            unit,
            text_revision,
            style_revision,
            estimated_height: sanitize_height(estimated_height),
            measured_height: None,
        });
        let index = index.min(document_cache.order.len());
        document_cache.order.insert(index, paragraph);
        document_cache.rebuild_height_index();

        self.units.get_mut(unit)?.location = TextUnitLocation::Paragraph {
            owner,
            document,
            paragraph,
        };
        self.paragraph(owner, document, paragraph)
    }

    pub fn remove_paragraph(
        &mut self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    ) -> usize {
        let Some(unit) = self
            .owners
            .get_mut(&owner)
            .and_then(|cache| cache.documents.get_mut(document))
            .and_then(|document| {
                let record = document.paragraphs.remove(paragraph)?;
                document.order.retain(|candidate| *candidate != paragraph);
                document.rebuild_height_index();
                Some(record.unit)
            })
        else {
            return 0;
        };
        self.remove_unit(unit)
    }

    pub fn paragraph_unit(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    ) -> Option<TextUnitId> {
        Some(
            self.owners
                .get(&owner)?
                .documents
                .get(document)?
                .paragraphs
                .get(paragraph)?
                .unit,
        )
    }

    pub fn paragraph(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    ) -> Option<ParagraphInfo> {
        let document = self.owners.get(&owner)?.documents.get(document)?;
        let record = document.paragraphs.get(paragraph)?;
        Some(record.info(paragraph))
    }

    pub fn paragraph_at(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        index: usize,
    ) -> Option<ParagraphInfo> {
        let document = self.owners.get(&owner)?.documents.get(document)?;
        let paragraph = *document.order.get(index)?;
        Some(document.paragraphs.get(paragraph)?.info(paragraph))
    }

    pub fn document_len(&self, owner: NodeId, document: TextDocumentId) -> Option<usize> {
        Some(
            self.owners
                .get(&owner)?
                .documents
                .get(document)?
                .order
                .len(),
        )
    }

    pub fn document_height(&self, owner: NodeId, document: TextDocumentId) -> Option<f32> {
        let document = self.owners.get(&owner)?.documents.get(document)?;
        Some(document.height_index.prefix_sum(document.order.len()))
    }

    pub fn paragraph_top(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    ) -> Option<f32> {
        let document = self.owners.get(&owner)?.documents.get(document)?;
        let index = document
            .order
            .iter()
            .position(|candidate| *candidate == paragraph)?;
        Some(document.height_index.prefix_sum(index))
    }

    pub fn paragraph_at_y(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        y: f32,
    ) -> Option<ParagraphInfo> {
        let document = self.owners.get(&owner)?.documents.get(document)?;
        if document.order.is_empty() || y < 0.0 {
            return None;
        }
        let total = document.height_index.prefix_sum(document.order.len());
        if y >= total {
            return None;
        }

        let mut low = 0;
        let mut high = document.order.len();
        while low < high {
            let mid = low + (high - low) / 2;
            if document.height_index.prefix_sum(mid + 1) <= y {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        let paragraph = *document.order.get(low)?;
        Some(document.paragraphs.get(paragraph)?.info(paragraph))
    }

    pub fn update_paragraph_height(
        &mut self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
        measured_height: Option<f32>,
    ) -> bool {
        let Some(document) = self
            .owners
            .get_mut(&owner)
            .and_then(|cache| cache.documents.get_mut(document))
        else {
            return false;
        };
        let Some(index) = document
            .order
            .iter()
            .position(|candidate| *candidate == paragraph)
        else {
            return false;
        };
        let Some(record) = document.paragraphs.get_mut(paragraph) else {
            return false;
        };
        let old_height = record.height();
        record.measured_height = measured_height.map(sanitize_height);
        let delta = record.height() - old_height;
        if delta != 0.0 {
            document.height_index.add(index, delta);
        }
        true
    }

    pub fn set_visible_paragraphs(
        &mut self,
        owner: NodeId,
        document: TextDocumentId,
        range: Range<usize>,
    ) -> bool {
        let Some(document) = self
            .owners
            .get_mut(&owner)
            .and_then(|cache| cache.documents.get_mut(document))
        else {
            return false;
        };
        let start = range.start.min(document.order.len());
        let end = range.end.clamp(start, document.order.len());
        document.visible_range = start..end;
        true
    }

    pub fn visible_paragraphs(
        &self,
        owner: NodeId,
        document: TextDocumentId,
    ) -> Option<Range<usize>> {
        Some(
            self.owners
                .get(&owner)?
                .documents
                .get(document)?
                .visible_range
                .clone(),
        )
    }

    pub fn stats(&self) -> TextCacheStats {
        let documents = self
            .owners
            .values()
            .map(|owner| owner.documents.len())
            .sum();
        let paragraphs = self
            .owners
            .values()
            .flat_map(|owner| owner.documents.values())
            .map(|document| document.paragraphs.len())
            .sum();
        TextCacheStats {
            owners: self.owners.len(),
            documents,
            paragraphs,
            units: self.units.len(),
            layouts: self.entries.len(),
            resident_layouts: self.residency.len(),
            resident_bytes: self.residency.weight(),
            capacity_bytes: self.residency.capacity(),
        }
    }

    pub fn estimated_memory_cost<F, K>(layout: &ParagraphLayout<F, K>, state: &B::State) -> u64 {
        let bytes = size_of_val(layout)
            .saturating_add(
                layout
                    .lines
                    .capacity()
                    .saturating_mul(size_of::<xui_interface::LineLayout>()),
            )
            .saturating_add(
                layout
                    .runs
                    .capacity()
                    .saturating_mul(size_of::<xui_interface::GlyphRun<F>>()),
            )
            .saturating_add(layout.glyphs.capacity().saturating_mul(size_of::<
                xui_interface::GlyphInstance<<B as Shaper>::GlyphKey>,
            >()))
            .saturating_add(
                layout
                    .clusters
                    .capacity()
                    .saturating_mul(size_of::<xui_interface::TextCluster>()),
            )
            .saturating_add(size_of_val(state));
        u64::try_from(bytes).unwrap_or(u64::MAX).max(1)
    }

    pub(crate) fn handle_node_lifecycle(&mut self, event: &NodeLifecycleEvent) {
        if let NodeLifecycleEvent::Removed(owner) = event {
            self.remove_owner(*owner);
        }
        self.backend.handle_node_lifecycle(event);
    }

    fn insert_entry(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        layout: Arc<ParagraphLayout<<B as Shaper>::FontId, <B as Shaper>::GlyphKey>>,
        input: Option<TextLayoutInput>,
        memory_cost: u64,
        pinned: bool,
    ) -> Option<TextLayoutHandle> {
        self.units.get(unit)?;
        let cache_key = LayoutCacheKey { unit, layout: key };
        let last_access = self.next_access_epoch();
        let handle = self.entries.insert(LayoutEntry {
            cache_key,
            layout,
            input,
            last_access: Cell::new(last_access),
        });
        self.units.get_mut(unit)?.resident_layouts.push(handle);

        let mut evicted = Vec::new();
        self.residency.insert_with_lifecycle(
            cache_key,
            LayoutResidency {
                handle,
                memory_cost: memory_cost.max(1),
                pin_count: u32::from(pinned),
            },
            &mut evicted,
        );
        self.reclaim_evicted(evicted);
        self.entries.contains_key(handle).then_some(handle)
    }

    fn replace_active_unit(
        &mut self,
        unit: TextUnitId,
        key: TextLayoutKey,
        handle: TextLayoutHandle,
        handle_is_pinned: bool,
    ) {
        if !handle_is_pinned {
            assert!(self.pin(handle), "active text layout must be resident");
        }

        let previous = {
            let unit_cache = self
                .units
                .get_mut(unit)
                .expect("active text layout unit must exist");
            if unit_cache.active == Some((key, handle)) {
                if handle_is_pinned {
                    self.unpin(handle);
                }
                return;
            }
            unit_cache.active.replace((key, handle))
        };
        if let Some((_, previous_handle)) = previous {
            self.unpin(previous_handle);
        }
    }

    fn resident_entry(&self, handle: TextLayoutHandle) -> Option<&LayoutEntry<B>> {
        let entry = self.entries.get(handle)?;
        let residency = self.residency.get(&entry.cache_key)?;
        if residency.handle != handle {
            return None;
        }
        entry.last_access.set(self.next_access_epoch());
        Some(entry)
    }

    fn reclaim_evicted(&mut self, evicted: Vec<TextLayoutHandle>) {
        for handle in evicted {
            self.remove_entry_and_indexes(handle);
        }
    }

    fn remove_entry_and_indexes(&mut self, handle: TextLayoutHandle) -> bool {
        let Some(entry) = self.entries.remove(handle) else {
            return false;
        };
        if let Some(unit) = self.units.get_mut(entry.cache_key.unit) {
            unit.resident_layouts
                .retain(|candidate| *candidate != handle);
            if unit.active.is_some_and(|(_, active)| active == handle) {
                unit.active = None;
            }
        }
        true
    }

    fn remove_unit(&mut self, unit: TextUnitId) -> usize {
        let Some(unit_cache) = self.units.remove(unit) else {
            return 0;
        };
        let mut removed = 0;
        for handle in unit_cache.resident_layouts {
            if let Some(entry) = self.entries.remove(handle) {
                self.residency.remove(&entry.cache_key);
                removed += 1;
            }
        }
        removed
    }

    fn remove_owner_if_empty(&mut self, owner: NodeId) {
        let empty = self.owners.get(&owner).is_some_and(OwnerCache::is_empty);
        if empty {
            self.owners.remove(&owner);
        }
    }

    fn enforce_unit_variant_limit(&mut self, unit: TextUnitId) {
        loop {
            let Some(unit_cache) = self.units.get(unit) else {
                return;
            };
            if unit_cache.resident_layouts.len() <= self.max_variants_per_unit {
                return;
            }

            let active = unit_cache.active.map(|(_, handle)| handle);
            let candidate = unit_cache
                .resident_layouts
                .iter()
                .copied()
                .filter(|handle| Some(*handle) != active && !self.handle_is_pinned(*handle))
                .min_by_key(|handle| {
                    self.entries
                        .get(*handle)
                        .map(|entry| entry.last_access.get())
                        .unwrap_or(0)
                });
            let Some(candidate) = candidate else {
                // The limit is intentionally soft while every excess variant is pinned.
                return;
            };
            self.remove(candidate);
        }
    }

    fn handle_is_pinned(&self, handle: TextLayoutHandle) -> bool {
        let Some(entry) = self.entries.get(handle) else {
            return false;
        };
        self.residency
            .peek(&entry.cache_key)
            .is_some_and(|residency| residency.handle == handle && residency.is_pinned())
    }

    fn next_access_epoch(&self) -> u64 {
        let next = self.access_epoch.get().wrapping_add(1);
        self.access_epoch.set(next);
        next
    }
}

struct TextUnitCache<S> {
    owner: NodeId,
    location: TextUnitLocation,
    state: S,
    active: Option<(TextLayoutKey, TextLayoutHandle)>,
    resident_layouts: SmallVec<[TextLayoutHandle; DEFAULT_MAX_VARIANTS_PER_UNIT]>,
}

#[derive(Default)]
struct OwnerCache {
    direct_slots: FxHashMap<TextLayoutSlot, TextUnitId>,
    documents: SlotMap<TextDocumentId, DocumentCache>,
}

impl OwnerCache {
    fn is_empty(&self) -> bool {
        self.direct_slots.is_empty() && self.documents.is_empty()
    }
}

#[derive(Default)]
struct DocumentCache {
    paragraphs: SlotMap<ParagraphId, ParagraphRecord>,
    order: Vec<ParagraphId>,
    height_index: HeightIndex,
    visible_range: Range<usize>,
}

impl DocumentCache {
    fn rebuild_height_index(&mut self) {
        self.height_index =
            HeightIndex::from_values(self.order.iter().filter_map(|paragraph| {
                self.paragraphs.get(*paragraph).map(ParagraphRecord::height)
            }));
        let start = self.visible_range.start.min(self.order.len());
        let end = self.visible_range.end.clamp(start, self.order.len());
        self.visible_range = start..end;
    }
}

/// A compact Fenwick tree used only by virtual-document paragraph metadata.
///
/// Indices accepted by the public methods are zero based; prefix sums use an
/// exclusive end, which keeps document calculations free of `index - 1`.
#[derive(Default)]
struct HeightIndex {
    inner: Vec<f32>,
}

impl HeightIndex {
    fn from_values(values: impl IntoIterator<Item = f32>) -> Self {
        let values: Vec<_> = values.into_iter().collect();
        let mut index = Self {
            inner: vec![0.0; values.len() + 1],
        };
        for (position, value) in values.into_iter().enumerate() {
            index.add(position, value);
        }
        index
    }

    fn add(&mut self, index: usize, delta: f32) {
        let mut cursor = index + 1;
        while cursor < self.inner.len() {
            self.inner[cursor] += delta;
            cursor += cursor & cursor.wrapping_neg();
        }
    }

    fn prefix_sum(&self, end: usize) -> f32 {
        let mut cursor = end.min(self.inner.len().saturating_sub(1));
        let mut sum = 0.0;
        while cursor > 0 {
            sum += self.inner[cursor];
            cursor -= cursor & cursor.wrapping_neg();
        }
        sum
    }
}

struct ParagraphRecord {
    source_range: TextRange,
    unit: TextUnitId,
    text_revision: u64,
    style_revision: u64,
    estimated_height: f32,
    measured_height: Option<f32>,
}

impl ParagraphRecord {
    fn height(&self) -> f32 {
        self.measured_height.unwrap_or(self.estimated_height)
    }

    fn info(&self, id: ParagraphId) -> ParagraphInfo {
        ParagraphInfo {
            id,
            unit: self.unit,
            source_range: self.source_range,
            text_revision: self.text_revision,
            style_revision: self.style_revision,
            estimated_height: self.estimated_height,
            measured_height: self.measured_height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LayoutCacheKey {
    unit: TextUnitId,
    layout: TextLayoutKey,
}

#[derive(Debug, Clone, Copy)]
struct LayoutResidency {
    handle: TextLayoutHandle,
    memory_cost: u64,
    pin_count: u32,
}

impl LayoutResidency {
    fn is_pinned(self) -> bool {
        self.pin_count > 0
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct LayoutWeighter;

impl Weighter<LayoutCacheKey, LayoutResidency> for LayoutWeighter {
    fn weight(&self, _key: &LayoutCacheKey, value: &LayoutResidency) -> u64 {
        value.memory_cost
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct LayoutLifecycle;

impl Lifecycle<LayoutCacheKey, LayoutResidency> for LayoutLifecycle {
    type RequestState = Vec<TextLayoutHandle>;

    fn is_pinned(&self, _key: &LayoutCacheKey, value: &LayoutResidency) -> bool {
        value.is_pinned()
    }

    fn on_evict(
        &self,
        evicted: &mut Self::RequestState,
        _key: LayoutCacheKey,
        value: LayoutResidency,
    ) {
        evicted.push(value.handle);
    }
}

type GlobalLayoutCache = QuickCache<
    LayoutCacheKey,
    LayoutResidency,
    LayoutWeighter,
    DefaultHashBuilder,
    LayoutLifecycle,
>;

fn text_layout_metrics<F, K>(layout: &ParagraphLayout<F, K>) -> TextLayoutMetrics {
    TextLayoutMetrics {
        size: layout.size(),
        first_baseline: layout.lines.first().map(|line| line.baseline),
        line_count: layout.lines.len(),
    }
}

struct LayoutEntry<B: TextBackend> {
    cache_key: LayoutCacheKey,
    layout: Arc<ParagraphLayout<<B as Shaper>::FontId, <B as Shaper>::GlyphKey>>,
    input: Option<TextLayoutInput>,
    last_access: Cell<u64>,
}

impl<B: TextBackend> TextLayoutQuery for LayoutEntry<B> {
    fn size(&self) -> Size<f32> {
        self.layout.size()
    }

    fn hit_test_point(&self, point: Point) -> Option<TextPosition> {
        self.layout.hit_test_point(point)
    }

    fn caret_rect(&self, char_index: usize) -> Option<Rect> {
        let input = self.input.as_ref()?;
        let text = input.text.as_str();
        let offset = char_to_layout_offset(text, char_index, layout_offset_unit(&self.layout));
        self.layout.caret_rect(TextPosition {
            offset,
            affinity: Affinity::After,
        })
    }

    fn selection_rects(&self, range: TextRange) -> Vec<Rect> {
        let Some(input) = self.input.as_ref() else {
            return Vec::new();
        };
        self.layout.selection_rects(normalize_range_for_layout(
            input.text.as_str(),
            &self.layout,
            range,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextCacheStats {
    pub owners: usize,
    pub documents: usize,
    pub paragraphs: usize,
    pub units: usize,
    pub layouts: usize,
    pub resident_layouts: usize,
    pub resident_bytes: u64,
    pub capacity_bytes: u64,
}

fn sanitize_height(height: f32) -> f32 {
    if height.is_finite() {
        height.max(0.0)
    } else {
        0.0
    }
}

fn hash_value(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn layout_key_for_input(input: &TextLayoutInput) -> TextLayoutKey {
    let max_width_bits = match input.constraints {
        TextLayoutConstraints::Definate(width) => width.to_bits(),
        TextLayoutConstraints::Unbound => f32::INFINITY.to_bits(),
        TextLayoutConstraints::MinSize => 0,
    };
    TextLayoutKey {
        text_revision: hash_value(&input.text),
        style_revision: shape_style_hash(&input.default_style),
        layout_style_hash: hash_value(&(&input.paragraph_style, &input.text_box_style)),
        max_width_bits,
        max_height_bits: f32::INFINITY.to_bits(),
        scale_factor_bits: 1.0_f32.to_bits(),
        font_context_revision: input.font_context_revision,
    }
}

fn shape_style_hash(style: &xui_interface::ComputedTextStyle) -> u64 {
    let mut hasher = DefaultHasher::new();
    style.font_family.hash(&mut hasher);
    style.font_size.to_bits().hash(&mut hasher);
    style.font_weight.hash(&mut hasher);
    style.font_style.hash(&mut hasher);
    style.line_height.hash(&mut hasher);
    style.letter_spacing.to_bits().hash(&mut hasher);
    hasher.finish()
}

fn layout_offset_unit<K, F>(layout: &ParagraphLayout<F, K>) -> TextOffsetUnit {
    layout
        .clusters
        .first()
        .map(|cluster| cluster.text_range.start.unit)
        .or_else(|| layout.lines.first().map(|line| line.text_range.start.unit))
        .unwrap_or(TextOffsetUnit::Char)
}

fn normalize_range_for_layout<F, K>(
    text: &str,
    layout: &ParagraphLayout<F, K>,
    range: TextRange,
) -> TextRange {
    let unit = layout_offset_unit(layout);
    TextRange::new(
        char_to_layout_offset(text, text_offset_to_char(text, range.start), unit),
        char_to_layout_offset(text, text_offset_to_char(text, range.end), unit),
    )
}

fn char_to_layout_offset(text: &str, char_index: usize, unit: TextOffsetUnit) -> TextOffset {
    let char_index = char_index.min(text.chars().count());
    match unit {
        TextOffsetUnit::Char => TextOffset::char_offset(char_index),
        TextOffsetUnit::Utf8Byte => TextOffset::byte_offset(char_to_byte_offset(text, char_index)),
        TextOffsetUnit::Utf16CodeUnit => {
            TextOffset::utf16_offset(char_to_utf16_offset(text, char_index))
        }
    }
}

fn text_offset_to_char(text: &str, offset: TextOffset) -> usize {
    match offset.unit {
        TextOffsetUnit::Char => offset.raw.min(text.chars().count()),
        TextOffsetUnit::Utf8Byte => byte_to_char_index(text, offset.raw),
        TextOffsetUnit::Utf16CodeUnit => utf16_to_char_index(text, offset.raw),
    }
}

fn char_to_byte_offset(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .map(|(byte, _)| byte)
        .nth(char_index)
        .unwrap_or(text.len())
}

fn char_to_utf16_offset(text: &str, char_index: usize) -> usize {
    text.chars().take(char_index).map(char::len_utf16).sum()
}

fn byte_to_char_index(text: &str, byte_offset: usize) -> usize {
    let byte_offset = byte_offset.min(text.len());
    text.char_indices()
        .take_while(|(byte, _)| *byte < byte_offset)
        .count()
}

fn utf16_to_char_index(text: &str, utf16_offset: usize) -> usize {
    let mut units = 0;
    for (char_index, ch) in text.chars().enumerate() {
        if units >= utf16_offset {
            return char_index;
        }
        units += ch.len_utf16();
    }
    text.chars().count()
}
