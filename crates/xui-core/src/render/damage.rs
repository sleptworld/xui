//! What of a frame changed since the last one, as regions of the root surface.
//!
//! Backend-independent: it reads only a [`BuiltFrame`], so every renderer can
//! repaint just the damaged part of its persistent frame instead of all of it.

use rustc_hash::FxHashMap;
use xui_interface::Bounds;
use xui_render_graph::{ProgramFingerprint, SampleExpansion};

use super::{BuiltFrame, BuiltItem, ContentVersion, PlacementVersion, RenderNodeId};
use crate::{Affine, Point};

/// Upper bound on the rects a simplified region keeps. Skia builds one region
/// band per distinct y-span, and the software presenter blits per rect, so a
/// handful of slightly-too-large rects beats a hundred exact ones.
const MAX_RECTS: usize = 32;
/// How much dead area a merge may introduce, relative to the two inputs.
const MERGE_SLACK: f32 = 0.25;
/// Above this many rects the pairwise passes cost more than they save, and
/// their cost grows faster than the input: the whole job goes to `band_merge`,
/// which is O(n log n) regardless.
const PAIRWISE_LIMIT: usize = 64;

fn contains(outer: Bounds, inner: Bounds) -> bool {
    outer.min.x <= inner.min.x
        && outer.min.y <= inner.min.y
        && outer.max.x >= inner.max.x
        && outer.max.y >= inner.max.y
}

#[derive(Debug, Clone, Default)]
pub struct DamageRegion {
    rects: Vec<Bounds>,
}

impl DamageRegion {
    pub fn full(rect: Bounds) -> Self {
        let mut region = Self::default();
        region.add(rect);
        region
    }

    pub fn add(&mut self, rect: Bounds) {
        if rect.width() > 0.0 && rect.height() > 0.0 {
            self.rects.push(rect);
        }
    }

    pub fn extend(&mut self, other: Self) {
        self.rects.extend(other.rects);
    }

    pub fn rects(&self) -> &[Bounds] {
        &self.rects
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn bounds(&self) -> Option<Bounds> {
        self.rects.iter().copied().reduce(Bounds::union)
    }

    /// True when any damaged rect touches `bounds`. Used to cull scene items
    /// that cannot contribute to this repaint.
    pub fn intersects(&self, bounds: Bounds) -> bool {
        self.rects.iter().any(|rect| rect.intersects(bounds))
    }

    /// Merges overlapping and near-overlapping rects in place.
    ///
    /// Damage is accumulated by appending, so a frame that touches one subtree
    /// through several paths (composite prefixes, backdrop expansion, child
    /// propagation) ends up with many rects covering nearly the same pixels.
    /// Every one of them costs a `Region` band, a clip band, and — on the
    /// software presenter — its own `read_pixels` plus per-pixel conversion.
    ///
    /// Every operation here only ever *grows* the covered area, so a simplified
    /// region is always safe to repaint: it can waste work, never lose it.
    pub fn simplify(&mut self) {
        if self.rects.len() < 2 {
            return;
        }
        if self.rects.len() > PAIRWISE_LIMIT {
            self.band_merge();
            return;
        }
        // Two passes: a merge can open up a further merge with a rect already
        // visited, and a fixpoint loop is not worth the extra scans.
        for _ in 0..2 {
            let before = self.rects.len();
            self.merge_pass();
            if self.rects.len() == before {
                break;
            }
        }
        // Bounded by `PAIRWISE_LIMIT`, so this stays a fixed small cost.
        while self.rects.len() > MAX_RECTS {
            if !self.merge_closest_pair() {
                break;
            }
        }
    }

    /// Collapses a large region into at most `MAX_RECTS` horizontal bands.
    ///
    /// Sorting by position first keeps each band spatially tight, which matters
    /// because the union of a band is what gets repainted. Quality is lower
    /// than the pairwise merge, but the cost is O(n log n) instead of growing
    /// with the square of a count this code does not control.
    fn band_merge(&mut self) {
        self.rects.sort_unstable_by(|a, b| {
            a.min
                .y
                .total_cmp(&b.min.y)
                .then_with(|| a.min.x.total_cmp(&b.min.x))
        });
        let group = self.rects.len().div_ceil(MAX_RECTS);
        self.rects = self
            .rects
            .chunks(group)
            .filter_map(|chunk| chunk.iter().copied().reduce(Bounds::union))
            .collect();
    }

    fn merge_pass(&mut self) {
        let mut merged: Vec<Bounds> = Vec::with_capacity(self.rects.len());
        'next: for rect in std::mem::take(&mut self.rects) {
            for existing in &mut merged {
                if contains(*existing, rect) {
                    continue 'next;
                }
                if contains(rect, *existing) {
                    *existing = rect;
                    continue 'next;
                }
                if existing.intersects(rect) {
                    let union = existing.union(rect);

                    if union.area() <= (existing.area() + rect.area()) * (1.0 + MERGE_SLACK) {
                        *existing = union;
                        continue 'next;
                    }
                }
            }
            merged.push(rect);
        }
        self.rects = merged;
    }

    /// Unions the pair whose union wastes the fewest pixels. Returns false when
    /// there is nothing left to merge.
    fn merge_closest_pair(&mut self) -> bool {
        let mut best: Option<(usize, usize, f32)> = None;
        for (i, a) in self.rects.iter().enumerate() {
            for (j, b) in self.rects.iter().enumerate().skip(i + 1) {
                let waste = a.union(*b).area() - a.area() - b.area();
                if best.is_none_or(|(_, _, current)| waste < current) {
                    best = Some((i, j, waste));
                }
            }
        }
        let Some((i, j, _)) = best else {
            return false;
        };
        let merged = self.rects[i].union(self.rects[j]);
        self.rects.swap_remove(j);
        self.rects[i] = merged;
        true
    }

    fn backdrop_damage(&self, output_bounds: Bounds, expansion: SampleExpansion) -> Self {
        let mut damage = Self::default();
        for rect in &self.rects {
            let expanded = expansion.apply_to_bounds(*rect);
            if let Some(affected) = expanded & output_bounds {
                damage.add(affected);
            }
        }
        damage
    }

    fn expand_sample(&mut self, expansion: SampleExpansion) {
        for rect in &mut self.rects {
            *rect = Bounds::new(
                Point::new(rect.min.x - expansion.left, rect.min.y - expansion.top),
                Point::new(rect.max.x + expansion.right, rect.max.y + expansion.bottom),
            );
        }
    }

    fn add_transformed(&mut self, other: &Self, transform: Affine, clip: Bounds) {
        for rect in &other.rects {
            if let Some(clipped) = transform.transform_bounds(*rect) & clip {
                self.add(clipped);
            }
        }
    }

    fn through_prefix_placement(
        &self,
        child_to_parent: Affine,
        parent_clip: Bounds,
        expansion: SampleExpansion,
        child_clip: Bounds,
    ) -> Self {
        let Some(parent_to_child) = child_to_parent.invert() else {
            return Self::default();
        };
        let mut result = Self::default();
        for rect in &self.rects {
            let expanded = expansion.apply_to_bounds(*rect);
            let Some(parent_visible) = expanded & parent_clip else {
                continue;
            };
            if let Some(child) = parent_to_child.transform_bounds(parent_visible) & child_clip {
                result.add(child);
            }
        }
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ItemVersion {
    content: ContentVersion,
    placement: PlacementVersion,
    /// A child layer's own effects; zero for a draw.
    effect: u64,
}

#[derive(Debug, Clone, PartialEq)]
enum ItemKind {
    Draw,
    Layer {
        source: RenderNodeId,
        transform: Affine,
        program: ProgramFingerprint,
        backdrop_expansion: SampleExpansion,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct ItemSnapshot {
    source: RenderNodeId,
    version: ItemVersion,
    bounds: Bounds,
    kind: ItemKind,
}

#[derive(Debug, Clone)]
struct LayerSnapshot {
    /// The layer's own damage is about what is drawn in it, so it follows the
    /// content bounds; the render bounds also move with its effects.
    content_bounds: Bounds,
    render_bounds: Bounds,
    items: Vec<ItemSnapshot>,
}

#[derive(Clone, Default)]
pub struct DamageTracker {
    snapshots: FxHashMap<RenderNodeId, LayerSnapshot>,
    last_damage: FxHashMap<RenderNodeId, DamageRegion>,
}

impl DamageTracker {
    pub fn clear(&mut self) {
        self.snapshots.clear();
        self.last_damage.clear();
    }

    pub fn update(&mut self, frame: &BuiltFrame) -> DamageRegion {
        let mut next_snapshots = FxHashMap::default();
        let mut dirty_by_layer = FxHashMap::default();
        let effect_expansions = layer_effect_expansions(frame);

        for layer in frame.layers.iter().rev() {
            let next = layer_snapshot(frame, layer);
            let mut dirty = match self.snapshots.get(&layer.source) {
                Some(previous) => diff_layer(previous, &next, &dirty_by_layer),
                None => DamageRegion::full(layer.render_bounds),
            };
            dirty.expand_sample(
                effect_expansions
                    .get(&layer.source)
                    .copied()
                    .unwrap_or(SampleExpansion::ZERO),
            );
            next_snapshots.insert(layer.source, next);
            dirty_by_layer.insert(layer.source, dirty);
        }

        propagate_composite_prefix_damage(frame, &mut dirty_by_layer);
        for region in dirty_by_layer.values_mut() {
            region.simplify();
        }
        self.snapshots = next_snapshots;
        self.last_damage = dirty_by_layer;
        self.last_damage
            .get(&frame.layers[frame.root_layer.0].source)
            .cloned()
            .unwrap_or_default()
    }

    pub fn layer(&self, source: RenderNodeId) -> DamageRegion {
        self.last_damage.get(&source).cloned().unwrap_or_default()
    }

    pub fn dirty_region_count(&self) -> usize {
        self.last_damage
            .values()
            .map(|region| region.rects.len())
            .sum()
    }
}

fn layer_snapshot(frame: &BuiltFrame, layer: &super::BuiltLayer) -> LayerSnapshot {
    let items = layer
        .items
        .iter()
        .map(|item| match item {
            BuiltItem::Draw(draw) => {
                let common = draw.common();
                ItemSnapshot {
                    source: common.source,
                    version: ItemVersion {
                        content: common.content_version,
                        placement: PlacementVersion::default(),
                        effect: 0,
                    },
                    bounds: common.world_bounds,
                    kind: ItemKind::Draw,
                }
            }
            BuiltItem::Layer(instance_id) => {
                let instance = frame
                    .layer_instance(*instance_id)
                    .expect("built layer instance");
                ItemSnapshot {
                    source: instance.source,
                    version: ItemVersion {
                        content: frame.layers[instance.layer.0].content_version,
                        placement: instance.placement_version,
                        effect: frame.layers[instance.layer.0].effect_version,
                    },
                    bounds: instance.world_bounds,
                    kind: ItemKind::Layer {
                        source: frame.layers[instance.layer.0].source,
                        transform: instance.composite.transform,
                        program: instance.render_program.program().fingerprint(),
                        backdrop_expansion: instance
                            .render_program
                            .program()
                            .backdrop_input_expansion(),
                    },
                }
            }
        })
        .collect();
    LayerSnapshot {
        content_bounds: layer.content_bounds,
        render_bounds: layer.render_bounds,
        items,
    }
}

fn diff_layer(
    previous: &LayerSnapshot,
    next: &LayerSnapshot,
    child_dirty: &FxHashMap<RenderNodeId, DamageRegion>,
) -> DamageRegion {
    if previous.content_bounds != next.content_bounds {
        let mut dirty = DamageRegion::full(previous.render_bounds);
        dirty.add(next.render_bounds);
        return dirty;
    }
    let stable = previous.items.len() == next.items.len()
        && previous
            .items
            .iter()
            .zip(&next.items)
            .all(|(old, new)| old.source == new.source && same_item_kind(&old.kind, &new.kind));
    if stable {
        return diff_ordered_items(&previous.items, &next.items, child_dirty);
    }

    let first_changed = previous
        .items
        .iter()
        .zip(&next.items)
        .position(|(old, new)| old.source != new.source || !same_item_kind(&old.kind, &new.kind))
        .unwrap_or(previous.items.len().min(next.items.len()));
    let mut dirty = diff_ordered_items(
        &previous.items[..first_changed],
        &next.items[..first_changed],
        child_dirty,
    );
    for item in previous.items.iter().skip(first_changed) {
        dirty.add(item.bounds);
    }
    for item in next.items.iter().skip(first_changed) {
        dirty.add(item.bounds);
    }
    dirty
}

fn same_item_kind(a: &ItemKind, b: &ItemKind) -> bool {
    matches!(
        (a, b),
        (ItemKind::Draw, ItemKind::Draw) | (ItemKind::Layer { .. }, ItemKind::Layer { .. })
    )
}

fn diff_ordered_items(
    previous: &[ItemSnapshot],
    next: &[ItemSnapshot],
    child_dirty: &FxHashMap<RenderNodeId, DamageRegion>,
) -> DamageRegion {
    let mut accumulated = DamageRegion::default();
    for (old, new) in previous.iter().zip(next) {
        let mut item_dirty = diff_item(old, new, child_dirty);
        if let ItemKind::Layer {
            backdrop_expansion, ..
        } = new.kind
        {
            item_dirty.extend(accumulated.backdrop_damage(new.bounds, backdrop_expansion));
        }
        accumulated.extend(item_dirty);
    }
    accumulated
}

fn diff_item(
    old: &ItemSnapshot,
    new: &ItemSnapshot,
    child_dirty: &FxHashMap<RenderNodeId, DamageRegion>,
) -> DamageRegion {
    let mut dirty = DamageRegion::default();
    // A child layer's effects changing changes what it shows everywhere, and
    // not what is drawn in it, which is all its own damage says.
    if old.version.placement == new.version.placement
        && old.version.effect == new.version.effect
        && old.bounds == new.bounds
        && old.kind == new.kind
    {
        match &new.kind {
            ItemKind::Layer {
                source, transform, ..
            } => {
                if let Some(child) = child_dirty.get(source) {
                    dirty.add_transformed(child, *transform, new.bounds);
                } else if old.version.content != new.version.content {
                    dirty.add(new.bounds);
                }
            }
            ItemKind::Draw if old.version.content != new.version.content => dirty.add(new.bounds),
            ItemKind::Draw => {}
        }
        return dirty;
    }
    dirty.add(old.bounds);
    dirty.add(new.bounds);
    dirty
}

fn layer_effect_expansions(frame: &BuiltFrame) -> FxHashMap<RenderNodeId, SampleExpansion> {
    let mut expansions = FxHashMap::<RenderNodeId, SampleExpansion>::default();
    for parent in &frame.layers {
        for item in &parent.items {
            let BuiltItem::Layer(instance_id) = item else {
                continue;
            };
            let instance = frame
                .layer_instance(*instance_id)
                .expect("built layer instance");
            let child = &frame.layers[instance.layer.0];
            let expansion = instance.render_program.program().layer_visual_expansion();
            expansions
                .entry(child.source)
                .and_modify(|current| {
                    current.left = current.left.max(expansion.left);
                    current.top = current.top.max(expansion.top);
                    current.right = current.right.max(expansion.right);
                    current.bottom = current.bottom.max(expansion.bottom);
                })
                .or_insert(expansion);
        }
    }
    expansions
}

fn propagate_composite_prefix_damage(
    frame: &BuiltFrame,
    dirty_by_layer: &mut FxHashMap<RenderNodeId, DamageRegion>,
) {
    let mut additions = Vec::new();
    for parent in &frame.layers {
        for item in &parent.items {
            let BuiltItem::Layer(instance_id) = item else {
                continue;
            };
            let instance = frame
                .layer_instance(*instance_id)
                .expect("built layer instance");
            let Some(mut prefix) = instance.destination_prefix else {
                continue;
            };
            let mut chain = Vec::new();
            while let Some(node) = frame.composite_prefix(prefix).copied() {
                chain.push(node);
                let Some(parent_prefix) = node.parent else {
                    break;
                };
                prefix = parent_prefix;
            }
            chain.reverse();
            let expansion = instance.render_program.program().backdrop_input_expansion();
            for ancestor_index in 0..chain.len().saturating_sub(1) {
                let ancestor = frame.layers[chain[ancestor_index].local.layer.0].source;
                let Some(mut dirty) = dirty_by_layer.get(&ancestor).cloned() else {
                    continue;
                };
                for node in &chain[ancestor_index + 1..] {
                    let Some(placement_id) = node.placement else {
                        dirty = DamageRegion::default();
                        break;
                    };
                    let placement = frame
                        .layer_instance(placement_id)
                        .expect("prefix placement exists");
                    dirty = dirty.through_prefix_placement(
                        placement.composite.transform,
                        placement.world_bounds,
                        placement
                            .render_program
                            .program()
                            .backdrop_input_expansion(),
                        frame.layers[node.local.layer.0].render_bounds,
                    );
                }
                let affected = dirty.backdrop_damage(instance.world_bounds, expansion);
                if !affected.is_empty() {
                    additions.push((parent.source, affected));
                }
            }
        }
    }
    for (source, damage) in additions {
        dirty_by_layer.entry(source).or_default().extend(damage);
    }
    for parent in frame.layers.iter().rev() {
        let mut propagated = DamageRegion::default();
        for item in &parent.items {
            let BuiltItem::Layer(instance_id) = item else {
                continue;
            };
            let instance = frame
                .layer_instance(*instance_id)
                .expect("built layer instance");
            let child = &frame.layers[instance.layer.0];
            if let Some(dirty) = dirty_by_layer.get(&child.source) {
                propagated.add_transformed(
                    dirty,
                    instance.composite.transform,
                    instance.world_bounds,
                );
            }
        }
        dirty_by_layer
            .entry(parent.source)
            .or_default()
            .extend(propagated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod effects {
        use std::sync::Arc;

        use super::super::DamageTracker;
        use crate::render::{
            BackdropIsolation, BuiltFrame, FrameBuilder, FrameProperties, LayerDescriptor,
            Primitive, RenderNodeId, RenderScene, SceneCompiler, Shape, ShapePrimitive,
        };
        use xui_interface::{Bounds, Color, ComputedColorStyle, ComputedEffect, FilterQuality};

        struct Session {
            source: RenderScene,
            compiler: SceneCompiler,
            tracker: DamageTracker,
        }

        impl Session {
            fn frame(&mut self) -> BuiltFrame {
                let snapshot = self.source.dirty_snapshot();
                let scene = self.compiler.compile(&self.source, &snapshot).unwrap();
                let frame = FrameBuilder::new()
                    .build(
                        scene,
                        Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0)),
                        &FrameProperties::default(),
                    )
                    .unwrap();
                self.source.acknowledge(&snapshot);
                self.tracker.update(&frame);
                frame
            }
        }

        fn blurred(sigma: f32) -> LayerDescriptor {
            LayerDescriptor {
                effects: Arc::from([ComputedEffect::Blur {
                    sigma_x: sigma,
                    sigma_y: sigma,
                    quality: FilterQuality::Medium,
                }]),
                ..LayerDescriptor::default()
            }
        }

        /// A blurred layer holding a square at 30..50.
        fn session() -> (Session, RenderNodeId) {
            let mut source = RenderScene::new();
            let layer = source.insert_layer(blurred(2.0));
            let square = source.insert_primitive(Primitive::Shape(ShapePrimitive {
                bounds: Bounds::from_origin_size((30.0, 30.0), (20.0, 20.0)),
                shape: Shape::Rect,
                fill: Some(ComputedColorStyle::Solid(Color::BLACK)),
                stroke: None,
                shadow: None,
            }));
            source.append_child(source.root(), layer).unwrap();
            source.set_child(layer, Some(square)).unwrap();
            let mut session = Session {
                source,
                compiler: SceneCompiler::new(),
                tracker: DamageTracker::default(),
            };
            session.frame();
            (session, layer)
        }

        fn layer_of(frame: &BuiltFrame, source: RenderNodeId) -> &crate::render::BuiltLayer {
            frame
                .layers
                .iter()
                .find(|layer| layer.source == source)
                .unwrap()
        }

        fn root_damage(session: &Session, frame: &BuiltFrame) -> Option<Bounds> {
            session
                .tracker
                .layer(frame.layers[frame.root_layer.0].source)
                .bounds()
        }

        fn contains(outer: Bounds, inner: Bounds) -> bool {
            outer.min.x <= inner.min.x
                && outer.min.y <= inner.min.y
                && outer.max.x >= inner.max.x
                && outer.max.y >= inner.max.y
        }

        /// What is drawn in the layer is as it was; what the parent shows of it
        /// is not.
        #[test]
        fn new_effects_damage_a_layer_in_its_parent_and_not_inside_it() {
            let (mut session, layer) = session();
            let before = session.frame();
            let before_layer = layer_of(&before, layer).clone();
            let before_root = before.layers[before.root_layer.0].content_version;

            session
                .source
                .update_layer_descriptor(layer, blurred(6.0))
                .unwrap();
            let after = session.frame();
            let after_layer = layer_of(&after, layer);

            assert_eq!(after_layer.content_version, before_layer.content_version);
            assert_ne!(after_layer.effect_version, before_layer.effect_version);
            assert_ne!(
                after.layers[after.root_layer.0].content_version,
                before_root
            );
            assert!(
                session.tracker.layer(layer).is_empty(),
                "{:?}",
                session.tracker.layer(layer)
            );
            // The parent repaints the layer where it reaches now: further than
            // before, by the larger blur.
            let reach = (after_layer.render_bounds
                & Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0)))
            .unwrap();
            assert!(reach.width() > before_layer.render_bounds.width());
            let damage = root_damage(&session, &after).expect("the parent is damaged");
            assert!(
                contains(damage, reach),
                "{damage:?} does not cover {reach:?}"
            );
        }

        /// Isolation is not in the layer's program, so nothing else says the
        /// layer changed: its effect version does.
        #[test]
        fn an_effect_outside_the_program_still_repaints_the_layer() {
            let (mut session, layer) = session();
            let before = session.frame();
            let reach = layer_of(&before, layer).render_bounds;
            session
                .source
                .update_layer_descriptor(
                    layer,
                    LayerDescriptor {
                        backdrop_isolation: BackdropIsolation::Isolate,
                        ..blurred(2.0)
                    },
                )
                .unwrap();
            let after = session.frame();
            let damage = root_damage(&session, &after).expect("the parent is damaged");
            assert!(
                contains(damage, reach),
                "{damage:?} does not cover {reach:?}"
            );
        }
    }

    #[test]
    fn transformed_damage_is_clipped() {
        let mut output = DamageRegion::default();
        output.add_transformed(
            &DamageRegion::full(Bounds::from_origin_size((0.0, 0.0), (5.0, 5.0))),
            Affine::translate(8.0, 0.0),
            Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
        );
        assert_eq!(
            output.bounds(),
            Some(Bounds::from_origin_size((8.0, 0.0), (2.0, 5.0)))
        );
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds {
        Bounds::from_origin_size((x, y), (w, h))
    }

    /// Every merge is a union and every drop is of a rect already contained,
    /// so each original rect must still sit entirely inside *some* result rect.
    fn covers(region: &DamageRegion, probe: Bounds) -> bool {
        region.rects.iter().any(|rect| contains(*rect, probe))
    }

    #[test]
    fn simplify_folds_a_contained_rect_away() {
        let mut region = DamageRegion::default();
        region.add(rect(0.0, 0.0, 100.0, 100.0));
        region.add(rect(10.0, 10.0, 5.0, 5.0));
        region.simplify();
        assert_eq!(region.rects, vec![rect(0.0, 0.0, 100.0, 100.0)]);
    }

    #[test]
    fn simplify_merges_overlapping_rects_it_barely_grows() {
        let mut region = DamageRegion::default();
        region.add(rect(0.0, 0.0, 100.0, 10.0));
        region.add(rect(90.0, 0.0, 100.0, 10.0));
        region.simplify();
        assert_eq!(region.rects, vec![rect(0.0, 0.0, 190.0, 10.0)]);
    }

    #[test]
    fn simplify_keeps_rects_whose_union_would_waste_space() {
        let mut region = DamageRegion::default();
        region.add(rect(0.0, 0.0, 10.0, 10.0));
        region.add(rect(500.0, 500.0, 10.0, 10.0));
        region.simplify();
        assert_eq!(region.rects.len(), 2);
    }

    /// Simplification may only ever grow the covered area. Losing a pixel here
    /// means a stale pixel left on screen.
    ///
    /// Swept across the pairwise limit so both the pairwise and the banding
    /// path are covered.
    #[test]
    fn simplify_never_drops_coverage() {
        for count in [2, 17, PAIRWISE_LIMIT, PAIRWISE_LIMIT + 1, 200] {
            let mut region = DamageRegion::default();
            let probes: Vec<Bounds> = (0..count)
                .map(|i| {
                    let i = i as f32;
                    rect((i * 7.0) % 400.0, (i * 13.0) % 300.0, 12.0, 9.0)
                })
                .collect();
            for probe in &probes {
                region.add(*probe);
            }
            region.simplify();
            assert!(region.rects.len() <= MAX_RECTS, "count {count}");
            for probe in &probes {
                assert!(
                    covers(&region, *probe),
                    "simplify dropped coverage of {probe:?} at count {count}"
                );
            }
        }
    }

    /// A region far past the pairwise limit still has to come out bounded, and
    /// without paying a cost that grows with the square of the input.
    #[test]
    fn simplify_bounds_a_huge_region_without_losing_coverage() {
        let mut region = DamageRegion::default();
        let probes: Vec<Bounds> = (0..20_000)
            .map(|i| {
                let i = i as f32;
                rect((i * 3.0) % 1900.0, (i * 11.0) % 1000.0, 4.0, 4.0)
            })
            .collect();
        for probe in &probes {
            region.add(*probe);
        }
        let started = std::time::Instant::now();
        region.simplify();
        let elapsed = started.elapsed();
        assert!(region.rects.len() <= MAX_RECTS);
        for probe in &probes {
            assert!(covers(&region, *probe), "band merge dropped {probe:?}");
        }
        // Generous next to O(n log n) on 20k rects, tight next to the O(n^3)
        // the pairwise path would have cost here.
        assert!(
            elapsed < std::time::Duration::from_millis(100),
            "simplify took {elapsed:?} on 20k rects"
        );
    }

    #[test]
    fn backdrop_expands_only_intersecting_damage() {
        let damage = DamageRegion::full(Bounds::from_origin_size((0.0, 0.0), (2.0, 2.0)));
        let affected = damage.backdrop_damage(
            Bounds::from_origin_size((3.0, 0.0), (4.0, 4.0)),
            SampleExpansion {
                left: 2.0,
                top: 0.0,
                right: 2.0,
                bottom: 0.0,
            },
        );
        assert_eq!(
            affected.bounds(),
            Some(Bounds::from_origin_size((3.0, 0.0), (1.0, 2.0)))
        );
    }
}
