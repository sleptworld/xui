//! Viewport-bounded rendering for long collections, without any UI of its own.
//!
//! A [`Virtualizer`] knows how many items a collection has, how large each one
//! is along the scrolling axis, and where the viewport is. From that it gives
//! the half-open [`VirtualRange`] worth mounting, each item's offset, and the
//! content's total size. It draws nothing: a component lays the items out
//! itself, which is what lets a list, a table body or a grid share it.
//!
//! Items have a [`ItemSize::Fixed`] size, or an [`ItemSize::Estimated`] one
//! that the measured size of each mounted item replaces. Measured sizes are
//! kept in a Fenwick tree, so offsets and the item at an offset cost
//! `O(log n)` however many items there are.
//!
//! A component keeps one in a `state!` and drives it through
//! [`VirtualizerSignal`], whose methods only notify when what is mounted, or
//! where, actually changed:
//!
//! ```ignore
//! #[component]
//! fn messages(#[source] count: &usize) {
//!     let list: Virtualizer = state!(Virtualizer::new(count.get(), ItemSize::Estimated(48.0), 3));
//!     effect!(|cx| list.sync(cx, count.get(), ItemSize::Estimated(48.0), 3));
//!     view! {
//!         container(
//!             style: scroll_y(),
//!             scroll_controller: list.controller(),
//!             on_scroll: |cx, event, event_cx| list.on_scroll(cx, event, event_cx),
//!         ) {
//!             column(style: content(list.total_size(), list.offset_of(list.range().start))) {
//!                 for index in list.range().iter() {
//!                     // each row reports its size: `list.measure(cx, index, size)`
//!                     message_row(list: list, index: *index).key(*index)
//!                 }
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! Measured sizes belong to an index, not to an item: a collection that
//! reorders its items should remeasure, by setting a new [`ItemSize`].

use std::cell::RefCell;
use std::ops::Range;

use xui_interface::Size;
use xui_interface::events::{EventResult, ScrollEvent};

use crate::event_system::EventContext;
use crate::reactive::{Cx, Signal};
use crate::scroll::ScrollController;

/// How large each item is along the scrolling axis, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ItemSize {
    /// Every item is this size; nothing is measured.
    Fixed(f32),
    /// Items are assumed to be this size until they are measured.
    Estimated(f32),
}

impl ItemSize {
    /// The size every item has, if it is fixed.
    pub fn fixed(self) -> Option<f32> {
        match self {
            Self::Fixed(size) => Some(sanitize(size)),
            Self::Estimated(_) => None,
        }
    }

    fn sanitized(self) -> Self {
        match self {
            Self::Fixed(size) => Self::Fixed(sanitize(size)),
            Self::Estimated(size) => Self::Estimated(sanitize(size)),
        }
    }
}

impl From<f32> for ItemSize {
    fn from(size: f32) -> Self {
        Self::Fixed(size)
    }
}

/// A non-finite or negative size counts as zero.
fn sanitize(size: f32) -> f32 {
    if size.is_finite() { size.max(0.0) } else { 0.0 }
}

/// A half-open range of item indices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct VirtualRange {
    pub start: usize,
    pub end: usize,
}

impl VirtualRange {
    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }

    pub fn contains(self, index: usize) -> bool {
        (self.start..self.end).contains(&index)
    }

    pub fn iter(self) -> Range<usize> {
        self.start..self.end
    }
}

impl IntoIterator for VirtualRange {
    type Item = usize;
    type IntoIter = Range<usize>;

    fn into_iter(self) -> Range<usize> {
        self.iter()
    }
}

/// Where [`Virtualizer::scroll_offset_for`] puts an item in the viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollAlign {
    Start,
    Center,
    End,
    /// The least movement that shows the whole item: none if it already is.
    #[default]
    Nearest,
}

/// Per-item sizes as a Fenwick tree over `f64`s, so a million items still
/// sum exactly enough to place the last one.
#[derive(Clone, Debug)]
struct MeasuredSizes {
    estimate: f32,
    /// The size each item counts as: its measurement, or the estimate.
    sizes: Vec<f32>,
    measured: Vec<bool>,
    /// 1-based: `tree[i]` sums the `i & i.wrapping_neg()` sizes ending at `i`.
    tree: Vec<f64>,
}

impl MeasuredSizes {
    fn new(count: usize, estimate: f32) -> Self {
        let mut sizes = Self {
            estimate,
            sizes: Vec::new(),
            measured: Vec::new(),
            tree: Vec::new(),
        };
        sizes.resize(count);
        sizes
    }

    fn len(&self) -> usize {
        self.sizes.len()
    }

    /// Keeps the measurements of the items that remain, and rebuilds the tree
    /// in `O(n)`.
    fn resize(&mut self, count: usize) {
        self.sizes.resize(count, self.estimate);
        self.measured.resize(count, false);
        self.tree.clear();
        self.tree.resize(count + 1, 0.0);
        for i in 1..=count {
            self.tree[i] += f64::from(self.sizes[i - 1]);
            let parent = i + (i & i.wrapping_neg());
            if parent <= count {
                self.tree[parent] += self.tree[i];
            }
        }
    }

    /// Every unmeasured item takes the new estimate.
    fn set_estimate(&mut self, estimate: f32) {
        self.estimate = estimate;
        for (size, measured) in self.sizes.iter_mut().zip(&self.measured) {
            if !measured {
                *size = estimate;
            }
        }
        self.resize(self.len());
    }

    fn size(&self, index: usize) -> f32 {
        self.sizes.get(index).copied().unwrap_or(0.0)
    }

    /// The new size minus the old one.
    fn set(&mut self, index: usize, size: f32) -> f32 {
        let delta = size - self.sizes[index];
        self.sizes[index] = size;
        self.measured[index] = true;
        let mut i = index + 1;
        while i < self.tree.len() {
            self.tree[i] += f64::from(delta);
            i += i & i.wrapping_neg();
        }
        delta
    }

    /// The summed size of the first `count` items.
    fn prefix(&self, count: usize) -> f64 {
        let mut i = count.min(self.len());
        let mut sum = 0.0;
        while i > 0 {
            sum += self.tree[i];
            i -= i & i.wrapping_neg();
        }
        sum
    }

    /// How many leading items end at or before `offset`: the index of the
    /// item `offset` falls in, `len()` past the end.
    fn count_ending_by(&self, offset: f64) -> usize {
        let n = self.len();
        let mut index = 0;
        let mut remaining = offset;
        let mut step = if n == 0 { 0 } else { 1 << n.ilog2() };
        while step > 0 {
            let next = index + step;
            if next <= n && self.tree[next] <= remaining {
                index = next;
                remaining -= self.tree[next];
            }
            step >>= 1;
        }
        index
    }
}

#[derive(Clone, Debug)]
enum Sizes {
    Fixed { count: usize, size: f32 },
    Measured(MeasuredSizes),
}

impl Sizes {
    fn new(count: usize, item_size: ItemSize) -> Self {
        match item_size.sanitized() {
            ItemSize::Fixed(size) => Self::Fixed { count, size },
            ItemSize::Estimated(estimate) => Self::Measured(MeasuredSizes::new(count, estimate)),
        }
    }

    fn count(&self) -> usize {
        match self {
            Self::Fixed { count, .. } => *count,
            Self::Measured(sizes) => sizes.len(),
        }
    }

    fn item_size(&self) -> ItemSize {
        match self {
            Self::Fixed { size, .. } => ItemSize::Fixed(*size),
            Self::Measured(sizes) => ItemSize::Estimated(sizes.estimate),
        }
    }

    fn size(&self, index: usize) -> f32 {
        match self {
            Self::Fixed { count, size } => {
                if index < *count {
                    *size
                } else {
                    0.0
                }
            }
            Self::Measured(sizes) => sizes.size(index),
        }
    }

    fn offset(&self, index: usize) -> f64 {
        match self {
            Self::Fixed { count, size } => index.min(*count) as f64 * f64::from(*size),
            Self::Measured(sizes) => sizes.prefix(index),
        }
    }

    fn total(&self) -> f64 {
        self.offset(self.count())
    }

    /// The item `offset` falls in, clamped into the collection; `None` when
    /// it is empty or has no size at all.
    fn index_at(&self, offset: f64) -> Option<usize> {
        let count = self.count();
        if count == 0 || self.total() <= 0.0 {
            return None;
        }
        let index = match self {
            Self::Fixed { size, .. } => (offset.max(0.0) / f64::from(*size)).floor() as usize,
            Self::Measured(sizes) => sizes.count_ending_by(offset.max(0.0)),
        };
        Some(index.min(count - 1))
    }
}

#[derive(Clone, Debug)]
struct State {
    sizes: Sizes,
    overscan: usize,
    offset: f32,
    viewport: f32,
    range: VirtualRange,
}

impl State {
    /// The visible items plus `overscan` on each side. Empty until the
    /// viewport is known: a guess would only be thrown away.
    fn compute_range(&self) -> VirtualRange {
        let viewport = f64::from(self.viewport);
        if !viewport.is_finite() || viewport <= 0.0 {
            return VirtualRange::default();
        }
        let total = self.sizes.total();
        let offset = f64::from(self.offset).clamp(0.0, (total - viewport).max(0.0));
        let Some(first) = self.sizes.index_at(offset) else {
            return VirtualRange::default();
        };
        let bottom = offset + viewport;
        let mut last = self.sizes.index_at(bottom).unwrap_or(first);
        // An item starting exactly at the bottom edge is not visible.
        if last > first && self.sizes.offset(last) >= bottom {
            last -= 1;
        }
        let count = self.sizes.count();
        VirtualRange {
            start: first.saturating_sub(self.overscan),
            end: last
                .saturating_add(1)
                .saturating_add(self.overscan)
                .min(count),
        }
    }

    /// Whether the range moved.
    fn refresh(&mut self) -> bool {
        let range = self.compute_range();
        let changed = range != self.range;
        self.range = range;
        changed
    }
}

/// The state of one virtualized axis. See the [module docs](self).
///
/// Every method takes `&self`, so it can sit in a `state!` and be updated
/// through [`VirtualizerSignal`] without notifying for changes nothing
/// renders. Each `set_*` method returns whether what is mounted, or where,
/// changed.
pub struct Virtualizer {
    state: RefCell<State>,
    controller: ScrollController,
}

impl Virtualizer {
    pub fn new(count: usize, item_size: ItemSize, overscan: usize) -> Self {
        Self {
            state: RefCell::new(State {
                sizes: Sizes::new(count, item_size),
                overscan,
                offset: 0.0,
                viewport: 0.0,
                range: VirtualRange::default(),
            }),
            controller: ScrollController::new(),
        }
    }

    /// Seeds the viewport size, so the first render already mounts the
    /// items it will show.
    pub fn with_viewport(self, viewport: f32) -> Self {
        self.set_viewport(viewport);
        self
    }

    /// The controller to attach to the scroll container: what
    /// [`Self::scroll_to_index`] and measurement corrections scroll with.
    pub fn controller(&self) -> ScrollController {
        self.controller.clone()
    }

    pub fn count(&self) -> usize {
        self.state.borrow().sizes.count()
    }

    pub fn item_size(&self) -> ItemSize {
        self.state.borrow().sizes.item_size()
    }

    /// Whether items are measured, rather than all of one fixed size.
    pub fn is_measured(&self) -> bool {
        matches!(self.state.borrow().sizes, Sizes::Measured(_))
    }

    pub fn overscan(&self) -> usize {
        self.state.borrow().overscan
    }

    /// The items to mount.
    pub fn range(&self) -> VirtualRange {
        self.state.borrow().range
    }

    pub fn scroll_offset(&self) -> f32 {
        self.state.borrow().offset
    }

    pub fn viewport(&self) -> f32 {
        self.state.borrow().viewport
    }

    /// Where item `index` starts; the total size for `index >= count`.
    pub fn offset_of(&self, index: usize) -> f32 {
        to_f32(self.state.borrow().sizes.offset(index))
    }

    /// The size item `index` counts as: measured, estimated or fixed.
    pub fn size_of(&self, index: usize) -> f32 {
        self.state.borrow().sizes.size(index)
    }

    /// The size of all items together: the scrollable extent.
    pub fn total_size(&self) -> f32 {
        to_f32(self.state.borrow().sizes.total())
    }

    /// The item at `offset`, clamped into the collection.
    pub fn index_at(&self, offset: f32) -> Option<usize> {
        self.state.borrow().sizes.index_at(f64::from(offset))
    }

    /// A new count keeps the measurements of the items that remain.
    pub fn set_count(&self, count: usize) -> bool {
        let mut state = self.state.borrow_mut();
        if state.sizes.count() == count {
            return false;
        }
        match &mut state.sizes {
            Sizes::Fixed { count: current, .. } => *current = count,
            Sizes::Measured(sizes) => sizes.resize(count),
        }
        state.refresh();
        true
    }

    /// A new fixed size, or a switch between fixed and estimated, drops every
    /// measurement; a new estimate keeps them.
    pub fn set_item_size(&self, item_size: ItemSize) -> bool {
        let item_size = item_size.sanitized();
        let mut state = self.state.borrow_mut();
        if state.sizes.item_size() == item_size {
            return false;
        }
        match (&mut state.sizes, item_size) {
            (Sizes::Measured(sizes), ItemSize::Estimated(estimate)) => sizes.set_estimate(estimate),
            (sizes, item_size) => *sizes = Sizes::new(sizes.count(), item_size),
        }
        state.refresh();
        true
    }

    pub fn set_overscan(&self, overscan: usize) -> bool {
        let mut state = self.state.borrow_mut();
        if state.overscan == overscan {
            return false;
        }
        state.overscan = overscan;
        state.refresh()
    }

    pub fn set_viewport(&self, viewport: f32) -> bool {
        let viewport = sanitize(viewport);
        let mut state = self.state.borrow_mut();
        if state.viewport == viewport {
            return false;
        }
        state.viewport = viewport;
        state.refresh()
    }

    /// Records the scroll container's offset. Most offsets move the content
    /// within the mounted items and change nothing.
    pub fn set_scroll_offset(&self, offset: f32) -> bool {
        let mut state = self.state.borrow_mut();
        let offset = if offset.is_finite() { offset.max(0.0) } else { 0.0 };
        state.offset = offset;
        state.refresh()
    }

    /// Records item `index`'s measured size. Ignored for fixed sizes, out of
    /// range indices and changes under half a pixel.
    ///
    /// An item that grows or shrinks above the first visible one would push
    /// the visible ones, so the scroll offset follows by the same amount and
    /// what the user is looking at stays where it is.
    pub fn measure(&self, index: usize, size: f32) -> bool {
        let size = sanitize(size);
        let mut state = self.state.borrow_mut();
        let first_visible = state.sizes.index_at(f64::from(state.offset));
        let offset = state.offset;
        let Sizes::Measured(sizes) = &mut state.sizes else {
            return false;
        };
        if index >= sizes.len() || (sizes.size(index) - size).abs() < 0.5 {
            return false;
        }
        let delta = sizes.set(index, size);
        if offset > 0.0
            && first_visible.is_some_and(|first| index < first)
            && self.controller.scroll_by((0.0, delta))
        {
            state.offset = (offset + delta).max(0.0);
        }
        state.refresh();
        true
    }

    /// The scroll offset that shows item `index` as `align` asks; `None`
    /// for an empty collection.
    pub fn scroll_offset_for(&self, index: usize, align: ScrollAlign) -> Option<f32> {
        let state = self.state.borrow();
        let count = state.sizes.count();
        if count == 0 {
            return None;
        }
        let index = index.min(count - 1);
        let start = state.sizes.offset(index);
        let size = f64::from(state.sizes.size(index));
        let viewport = f64::from(state.viewport);
        let current = f64::from(state.offset);
        let target = match align {
            ScrollAlign::Start => start,
            ScrollAlign::Center => start + size / 2.0 - viewport / 2.0,
            ScrollAlign::End => start + size - viewport,
            ScrollAlign::Nearest => {
                if start < current {
                    start
                } else if start + size > current + viewport {
                    start + size - viewport
                } else {
                    current
                }
            }
        };
        let max = (state.sizes.total() - viewport).max(0.0);
        Some(to_f32(target.clamp(0.0, max)))
    }

    /// Scrolls item `index` into view through [`Self::controller`]. The
    /// container then reports the scroll like any other, which is what moves
    /// the range. `false` when nothing is attached to scroll.
    pub fn scroll_to_index(&self, index: usize, align: ScrollAlign) -> bool {
        self.scroll_offset_for(index, align)
            .is_some_and(|offset| self.controller.scroll_to_y(offset))
    }
}

impl std::fmt::Debug for Virtualizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.borrow();
        f.debug_struct("Virtualizer")
            .field("count", &state.sizes.count())
            .field("item_size", &state.sizes.item_size())
            .field("range", &state.range)
            .field("offset", &state.offset)
            .field("viewport", &state.viewport)
            .finish()
    }
}

fn to_f32(value: f64) -> f32 {
    value.min(f64::from(f32::MAX)) as f32
}

/// Drives a [`Virtualizer`] held in a `state!` from effects and handlers,
/// notifying its readers only when [`Virtualizer`]'s matching method reports a
/// change.
pub trait VirtualizerSignal {
    /// Brings the count, size and overscan up to date with their props.
    fn sync(&self, cx: &mut Cx<'_>, count: usize, item_size: ItemSize, overscan: usize);

    fn set_viewport(&self, cx: &mut Cx<'_>, viewport: f32);

    /// The scroll container's `on_scroll`. Scrolls of other containers
    /// bubbling through it are ignored, and so is the event itself: the
    /// runtime has already moved the content.
    fn on_scroll(
        &self,
        cx: &mut Cx<'_>,
        event: &ScrollEvent,
        event_cx: &EventContext<'_>,
    ) -> EventResult;

    /// An item's observed size: what a measured item's `size!()` reads.
    /// `None`, before layout or after unmounting, keeps the last measurement.
    fn measure(&self, cx: &mut Cx<'_>, index: usize, size: Option<Size<f32>>);
}

impl VirtualizerSignal for Signal<Virtualizer> {
    fn sync(&self, cx: &mut Cx<'_>, count: usize, item_size: ItemSize, overscan: usize) {
        let changed = self.with(|list| {
            // Every setter runs: `|` does not short-circuit.
            list.set_count(count) | list.set_item_size(item_size) | list.set_overscan(overscan)
        });
        notify_if(self, cx, changed);
    }

    fn set_viewport(&self, cx: &mut Cx<'_>, viewport: f32) {
        let changed = self.with(|list| list.set_viewport(viewport));
        notify_if(self, cx, changed);
    }

    fn on_scroll(
        &self,
        cx: &mut Cx<'_>,
        event: &ScrollEvent,
        event_cx: &EventContext<'_>,
    ) -> EventResult {
        if event.scroll_target != event_cx.node_id() {
            return EventResult::Ignored;
        }
        let Some(offset) = event.offset_after else {
            return EventResult::Ignored;
        };
        let height = event_cx.node_ref.layout.height();
        let changed = self.with(|list| {
            let resized = height > 0.0 && list.set_viewport(height);
            list.set_scroll_offset(offset.y) | resized
        });
        notify_if(self, cx, changed);
        EventResult::Ignored
    }

    fn measure(&self, cx: &mut Cx<'_>, index: usize, size: Option<Size<f32>>) {
        let Some(size) = size else { return };
        let changed = self.with(|list| list.measure(index, size.height));
        notify_if(self, cx, changed);
    }
}

fn notify_if(list: &Signal<Virtualizer>, cx: &mut Cx<'_>, changed: bool) {
    if changed {
        list.update(cx, |_| ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(count: usize, size: f32, viewport: f32, overscan: usize) -> Virtualizer {
        Virtualizer::new(count, ItemSize::Fixed(size), overscan).with_viewport(viewport)
    }

    fn range(start: usize, end: usize) -> VirtualRange {
        VirtualRange { start, end }
    }

    #[test]
    fn an_empty_or_unmeasured_collection_mounts_nothing() {
        assert!(fixed(0, 20.0, 200.0, 3).range().is_empty());
        // Before the first layout the viewport is unknown.
        assert!(fixed(100, 20.0, 0.0, 3).range().is_empty());
        assert!(fixed(100, 0.0, 200.0, 3).range().is_empty());
        assert!(fixed(100, f32::NAN, 200.0, 3).range().is_empty());
        assert!(fixed(100, 20.0, f32::INFINITY, 3).range().is_empty());
    }

    #[test]
    fn the_range_covers_the_viewport_plus_overscan() {
        let list = fixed(1_000, 20.0, 200.0, 3);
        // Rows 0..10 fill the viewport; overscan only trails at the top.
        assert_eq!(list.range(), range(0, 13));

        list.set_scroll_offset(1_000.0);
        assert_eq!(list.range(), range(47, 63));
    }

    #[test]
    fn items_straddling_either_edge_are_included() {
        let list = fixed(1_000, 20.0, 200.0, 0);
        list.set_scroll_offset(110.0);
        // Half of row 5 is above the fold, half of row 15 below it.
        assert_eq!(list.range(), range(5, 16));
    }

    #[test]
    fn the_range_stays_inside_the_collection() {
        let list = fixed(1_000, 20.0, 200.0, 3);
        list.set_scroll_offset(-500.0);
        assert_eq!(list.range().start, 0);
        list.set_scroll_offset(1_000_000.0);
        assert_eq!(list.range(), range(987, 1_000));

        // A collection that shrinks under an old offset clamps.
        list.set_count(20);
        assert_eq!(list.range(), range(7, 20));
        assert!(fixed(20, 20.0, 200.0, usize::MAX).range().end == 20);
    }

    #[test]
    fn the_mounted_count_does_not_grow_with_the_collection() {
        let short = fixed(100, 20.0, 200.0, 3).range().len();
        let long = fixed(1_000_000, 20.0, 200.0, 3).range().len();
        assert_eq!(short, long);
    }

    #[test]
    fn setters_report_only_changes_to_what_is_mounted() {
        let list = fixed(1_000, 20.0, 200.0, 3);
        assert!(list.set_scroll_offset(401.0));
        assert!(!list.set_scroll_offset(410.0), "a sub-row scroll changed the range");
        assert!(!list.set_count(1_000));
        assert!(list.set_count(2_000), "the total size changed");
        assert!(!list.set_item_size(ItemSize::Fixed(20.0)));
        assert!(!list.measure(3, 50.0), "fixed items are never measured");
    }

    #[test]
    fn estimated_items_are_placed_by_their_measurements() {
        let list = Virtualizer::new(100, ItemSize::Estimated(10.0), 0).with_viewport(50.0);
        assert_eq!(list.total_size(), 1_000.0);
        assert_eq!(list.range(), range(0, 5));

        assert!(list.measure(0, 30.0));
        assert!(list.measure(1, 30.0));
        assert_eq!(list.offset_of(2), 60.0);
        assert_eq!(list.total_size(), 1_040.0);
        // Two tall rows now fill the viewport.
        assert_eq!(list.range(), range(0, 2));

        assert_eq!(list.index_at(59.0), Some(1));
        assert_eq!(list.index_at(60.0), Some(2));
        assert_eq!(list.index_at(1e9), Some(99));
        assert!(!list.measure(1, 30.2), "sub-pixel changes are noise");
    }

    #[test]
    fn a_new_count_or_estimate_keeps_measurements() {
        let list = Virtualizer::new(10, ItemSize::Estimated(10.0), 0);
        list.measure(2, 40.0);
        list.set_count(20);
        assert_eq!(list.size_of(2), 40.0);
        assert_eq!(list.total_size(), 19.0 * 10.0 + 40.0);
        list.set_item_size(ItemSize::Estimated(20.0));
        assert_eq!(list.size_of(2), 40.0);
        assert_eq!(list.size_of(3), 20.0);
        list.set_count(2);
        assert_eq!(list.total_size(), 40.0);

        // Switching to fixed drops them.
        list.set_item_size(ItemSize::Fixed(5.0));
        assert_eq!(list.total_size(), 10.0);
    }

    #[test]
    fn fenwick_offsets_match_a_running_sum() {
        let count = 1_037;
        let list = Virtualizer::new(count, ItemSize::Estimated(7.0), 0);
        let mut sizes = vec![7.0f32; count];
        for i in (0..count).step_by(13) {
            let size = (i % 50) as f32 + 1.0;
            list.measure(i, size);
            sizes[i] = size;
        }
        let mut sum = 0.0f64;
        for (i, size) in sizes.iter().enumerate() {
            assert_eq!(list.offset_of(i), sum as f32, "offset of {i}");
            if *size > 0.0 {
                assert_eq!(list.index_at(sum as f32), Some(i), "item at {sum}");
            }
            sum += f64::from(*size);
        }
        assert_eq!(list.total_size(), sum as f32);
    }

    #[test]
    fn scroll_offsets_for_each_alignment() {
        let list = fixed(100, 20.0, 100.0, 0);
        list.set_scroll_offset(200.0);
        assert_eq!(list.scroll_offset_for(50, ScrollAlign::Start), Some(1_000.0));
        assert_eq!(list.scroll_offset_for(50, ScrollAlign::End), Some(920.0));
        assert_eq!(list.scroll_offset_for(50, ScrollAlign::Center), Some(960.0));
        // Already visible: no movement. Above: to its start. Below: to its end.
        assert_eq!(list.scroll_offset_for(12, ScrollAlign::Nearest), Some(200.0));
        assert_eq!(list.scroll_offset_for(2, ScrollAlign::Nearest), Some(40.0));
        assert_eq!(list.scroll_offset_for(20, ScrollAlign::Nearest), Some(320.0));
        // Clamped at both ends.
        assert_eq!(list.scroll_offset_for(0, ScrollAlign::End), Some(0.0));
        assert_eq!(list.scroll_offset_for(99, ScrollAlign::Start), Some(1_900.0));
        assert_eq!(fixed(0, 20.0, 100.0, 0).scroll_offset_for(0, ScrollAlign::Start), None);
    }
}
