use std::collections::HashMap;

use xui::{
    Affine, Point,
    render::{BuiltFrame, BuiltItem, ContentVersion, PlacementVersion, RenderNodeId},
};
use xui_interface::{Bounds, Rect};
use xui_render_graph::{ProgramFingerprint, SampleExpansion};

#[derive(Debug, Clone, Default)]
pub(crate) struct DamageRegion {
    rects: Vec<Bounds>,
}

impl DamageRegion {
    pub(crate) fn full(rect: Bounds) -> Self {
        let mut region = Self::default();
        region.add(rect);
        region
    }

    pub(crate) fn add(&mut self, rect: Bounds) {
        if rect.width() > 0.0 && rect.height() > 0.0 {
            self.rects.push(rect);
        }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.rects.extend(other.rects);
    }

    pub(crate) fn rects(&self) -> &[Bounds] {
        &self.rects
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub(crate) fn bounds(&self) -> Option<Bounds> {
        self.rects.iter().copied().reduce(Bounds::union)
    }

    fn backdrop_damage(&self, output_bounds: Bounds, expansion: SampleExpansion) -> Self {
        let mut damage = Self::default();
        for rect in &self.rects {
            let expanded = Bounds::new(
                Point::new(rect.x() - expansion.left, rect.y() - expansion.top),
                Point::new(rect.max.x + expansion.right, rect.max.y + expansion.bottom),
            );
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
        let Some(parent_to_child) = inverse_affine(child_to_parent) else {
            return Self::default();
        };
        let mut result = Self::default();
        for rect in &self.rects {
            let expanded = expansion.apply_to_bounds(*rect);
            let Some(parent_visible) = expanded & parent_clip  else {
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
    render_bounds: Bounds,
    items: Vec<ItemSnapshot>,
}

#[derive(Clone, Default)]
pub(crate) struct DamageTracker {
    snapshots: HashMap<RenderNodeId, LayerSnapshot>,
    last_damage: HashMap<RenderNodeId, DamageRegion>,
}

impl DamageTracker {
    pub(crate) fn clear(&mut self) {
        self.snapshots.clear();
        self.last_damage.clear();
    }

    pub(crate) fn update(&mut self, frame: &BuiltFrame) -> DamageRegion {
        let mut next_snapshots = HashMap::new();
        let mut dirty_by_layer = HashMap::new();
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
        self.snapshots = next_snapshots;
        self.last_damage = dirty_by_layer;
        self.last_damage
            .get(&frame.layers[frame.root_layer.0].source)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn layer(&self, source: RenderNodeId) -> DamageRegion {
        self.last_damage.get(&source).cloned().unwrap_or_default()
    }

    pub(crate) fn dirty_region_count(&self) -> usize {
        self.last_damage
            .values()
            .map(|region| region.rects.len())
            .sum()
    }
}

fn layer_snapshot(frame: &BuiltFrame, layer: &xui::render::BuiltLayer) -> LayerSnapshot {
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
        render_bounds: layer.render_bounds,
        items,
    }
}

fn diff_layer(
    previous: &LayerSnapshot,
    next: &LayerSnapshot,
    child_dirty: &HashMap<RenderNodeId, DamageRegion>,
) -> DamageRegion {
    if previous.render_bounds != next.render_bounds {
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
    child_dirty: &HashMap<RenderNodeId, DamageRegion>,
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
    child_dirty: &HashMap<RenderNodeId, DamageRegion>,
) -> DamageRegion {
    let mut dirty = DamageRegion::default();
    if old.version.placement == new.version.placement
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

fn layer_effect_expansions(frame: &BuiltFrame) -> HashMap<RenderNodeId, SampleExpansion> {
    let mut expansions = HashMap::<RenderNodeId, SampleExpansion>::new();
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
    dirty_by_layer: &mut HashMap<RenderNodeId, DamageRegion>,
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

fn intersect_rect(a: Rect, b: Rect) -> Option<Rect> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

fn inverse_affine(value: Affine) -> Option<Affine> {
    let determinant = value.xx * value.yy - value.xy * value.yx;
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    let inverse = determinant.recip();
    let xx = value.yy * inverse;
    let yx = -value.yx * inverse;
    let xy = -value.xy * inverse;
    let yy = value.xx * inverse;
    Some(Affine::new(
        xx,
        yx,
        xy,
        yy,
        -(xx * value.dx + xy * value.dy),
        -(yx * value.dx + yy * value.dy),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
