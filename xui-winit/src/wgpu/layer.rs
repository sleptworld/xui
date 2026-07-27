use crate::wgpu::{
    LayerSnapshot,
    cache::BackendDirtyRegion,
    snapshot::{LayerItemKind, LayerItemSnapshot},
};
use std::collections::HashMap;
use xui::render::{ContentVersion, PlacementVersion, RenderNodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LayerItemVersion {
    pub content: ContentVersion,
    pub placement: PlacementVersion,
}

pub(super) fn diff_layer(
    previous: &LayerSnapshot,
    next: &LayerSnapshot,
    child_dirty: &HashMap<RenderNodeId, BackendDirtyRegion>,
) -> BackendDirtyRegion {
    if previous.render_bounds != next.render_bounds || previous.effects != next.effects {
        let mut dirty = BackendDirtyRegion::full(previous.render_bounds);
        dirty.add(next.render_bounds);
        return dirty;
    }

    if same_item_sequence(previous, next) {
        diff_stable_topology(previous, next, child_dirty)
    } else {
        diff_changed_topology(previous, next, child_dirty)
    }
}

fn same_item_sequence(previous: &LayerSnapshot, next: &LayerSnapshot) -> bool {
    previous.items.len() == next.items.len()
        && previous
            .items
            .iter()
            .zip(&next.items)
            .all(|(old, new)| old.source == new.source && same_item_kind(&old.kind, &new.kind))
}

fn same_item_kind(a: &LayerItemKind, b: &LayerItemKind) -> bool {
    matches!(
        (a, b),
        (LayerItemKind::Draw, LayerItemKind::Draw)
            | (LayerItemKind::Layer { .. }, LayerItemKind::Layer { .. })
    )
}

#[inline]
fn diff_stable_topology(
    previous: &LayerSnapshot,
    next: &LayerSnapshot,
    child_dirty: &HashMap<RenderNodeId, BackendDirtyRegion>,
) -> BackendDirtyRegion {
    debug_assert_eq!(previous.items.len(), next.items.len());

    diff_ordered_items(&previous.items, &next.items, child_dirty)
}

fn diff_changed_topology(
    previous: &LayerSnapshot,
    next: &LayerSnapshot,
    child_dirty: &HashMap<RenderNodeId, BackendDirtyRegion>,
) -> BackendDirtyRegion {
    let first_changed = previous
        .items
        .iter()
        .zip(&next.items)
        .position(|(old, new)| old.source != new.source || !same_item_kind(&old.kind, &new.kind))
        .unwrap_or(previous.items.len().min(next.items.len()));

    // Backdrop dependencies in the common prefix can only observe damage produced
    // before them. Once topology diverges, conservatively invalidate both suffixes;
    // this also covers every backdrop at or after the first changed item.
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

fn diff_ordered_items(
    previous: &[LayerItemSnapshot],
    next: &[LayerItemSnapshot],
    child_dirty: &HashMap<RenderNodeId, BackendDirtyRegion>,
) -> BackendDirtyRegion {
    debug_assert_eq!(previous.len(), next.len());

    let mut accumulated_dirty = BackendDirtyRegion::default();
    for (item, next_item) in previous.iter().zip(next) {
        debug_assert_eq!(item.source, next_item.source);
        let mut item_dirty = diff_item(item, next_item, child_dirty);

        if let LayerItemKind::Layer {
            backdrop_expansion: Some(expansion),
            ..
        } = next_item.kind
        {
            // A backdrop samples only items painted before it. Map only the
            // accumulated prefix damage into this backdrop's output bounds.
            item_dirty.extend(accumulated_dirty.backdrop_damage(next_item.bounds, expansion));
        }

        accumulated_dirty.extend(item_dirty);
    }
    accumulated_dirty
}

#[inline]
fn diff_item(
    item: &LayerItemSnapshot,
    next_item: &LayerItemSnapshot,
    child_dirty: &HashMap<RenderNodeId, BackendDirtyRegion>,
) -> BackendDirtyRegion {
    let mut dirty = BackendDirtyRegion::default();

    if item.version.placement == next_item.version.placement
        && item.bounds == next_item.bounds
        && item.kind == next_item.kind
    {
        match next_item.kind {
            LayerItemKind::Layer {
                source, transform, ..
            } => {
                if let Some(child) = child_dirty.get(&source) {
                    dirty.add_transformed(child, transform, next_item.bounds);
                } else if item.version.content != next_item.version.content {
                    dirty.add(next_item.bounds);
                }
            }

            LayerItemKind::Draw => {
                if item.version.content != next_item.version.content {
                    dirty.add(next_item.bounds);
                }
            }
        }

        return dirty;
    }

    dirty.add(item.bounds);
    dirty.add(next_item.bounds);

    dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wgpu::snapshot::{LayerItemKind, LayerItemSnapshot};
    use std::sync::Arc;
    use xui::Affine;
    use xui::render::RenderScene;
    use xui_interface::Rect;

    fn test_snapshot(items: Vec<LayerItemSnapshot>) -> LayerSnapshot {
        LayerSnapshot {
            content_version: ContentVersion::default(),
            render_bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            effects: Arc::from([]),
            items,
        }
    }

    fn draw_item(source: RenderNodeId, paint: u64, bounds: Rect) -> LayerItemSnapshot {
        LayerItemSnapshot {
            source,
            version: LayerItemVersion {
                content: ContentVersion {
                    paint,
                    ..ContentVersion::default()
                },
                placement: PlacementVersion::default(),
            },
            bounds,
            kind: LayerItemKind::Draw,
        }
    }

    fn backdrop_item(source: RenderNodeId, bounds: Rect, expansion: f32) -> LayerItemSnapshot {
        LayerItemSnapshot {
            source,
            version: LayerItemVersion {
                content: ContentVersion::default(),
                placement: PlacementVersion::default(),
            },
            bounds,
            kind: LayerItemKind::Layer {
                source,
                transform: Affine::IDENTITY,
                backdrop_expansion: Some(expansion),
            },
        }
    }

    #[test]
    fn dynamic_layer_placement_dirties_old_and_new_tiles_without_content_change() {
        let source = RenderNodeId::default();
        let child_content = ContentVersion {
            paint: 7,
            ..ContentVersion::default()
        };
        let snapshot = |dynamic, bounds| LayerSnapshot {
            content_version: ContentVersion {
                dynamic,
                ..ContentVersion::default()
            },
            render_bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            effects: Arc::from([]),
            items: vec![LayerItemSnapshot {
                source,
                version: LayerItemVersion {
                    content: child_content,
                    placement: PlacementVersion { scene: 3, dynamic },
                },
                bounds,
                kind: LayerItemKind::Layer {
                    source,
                    transform: Affine::IDENTITY,
                    backdrop_expansion: None,
                },
            }],
        };
        let previous = snapshot(1, Rect::new(0.0, 0.0, 10.0, 10.0));
        let next = snapshot(2, Rect::new(20.0, 0.0, 10.0, 10.0));

        let dirty = diff_layer(&previous, &next, &HashMap::new());
        assert_eq!(dirty.tiles(1.0, 10), [(0, 0), (2, 0)].into_iter().collect());
    }

    #[test]
    fn foreground_damage_does_not_propagate_backwards_to_backdrop() {
        let mut scene = RenderScene::new();
        let backdrop = scene.insert_group();
        let foreground = scene.insert_group();
        let backdrop_bounds = Rect::new(15.0, 0.0, 10.0, 10.0);
        let foreground_bounds = Rect::new(5.0, 0.0, 5.0, 10.0);

        let previous = test_snapshot(vec![
            backdrop_item(backdrop, backdrop_bounds, 10.0),
            draw_item(foreground, 1, foreground_bounds),
        ]);
        let next = test_snapshot(vec![
            backdrop_item(backdrop, backdrop_bounds, 10.0),
            draw_item(foreground, 2, foreground_bounds),
        ]);

        let dirty = diff_layer(&previous, &next, &HashMap::new());
        assert_eq!(dirty.tiles(1.0, 10), [(0, 0)].into_iter().collect());
    }

    #[test]
    fn background_damage_propagates_forward_to_affected_backdrop_output() {
        let mut scene = RenderScene::new();
        let background = scene.insert_group();
        let backdrop = scene.insert_group();
        let background_bounds = Rect::new(5.0, 0.0, 5.0, 10.0);
        let backdrop_bounds = Rect::new(15.0, 0.0, 10.0, 10.0);

        let previous = test_snapshot(vec![
            draw_item(background, 1, background_bounds),
            backdrop_item(backdrop, backdrop_bounds, 10.0),
        ]);
        let next = test_snapshot(vec![
            draw_item(background, 2, background_bounds),
            backdrop_item(backdrop, backdrop_bounds, 10.0),
        ]);

        let dirty = diff_layer(&previous, &next, &HashMap::new());
        assert_eq!(dirty.tiles(1.0, 10), [(0, 0), (1, 0)].into_iter().collect());
    }

    #[test]
    fn backdrop_damage_propagates_through_later_backdrops() {
        let mut scene = RenderScene::new();
        let background = scene.insert_group();
        let first_backdrop = scene.insert_group();
        let second_backdrop = scene.insert_group();

        let previous = test_snapshot(vec![
            draw_item(background, 1, Rect::new(5.0, 0.0, 5.0, 10.0)),
            backdrop_item(first_backdrop, Rect::new(15.0, 0.0, 10.0, 10.0), 10.0),
            backdrop_item(second_backdrop, Rect::new(25.0, 0.0, 10.0, 10.0), 10.0),
        ]);
        let next = test_snapshot(vec![
            draw_item(background, 2, Rect::new(5.0, 0.0, 5.0, 10.0)),
            backdrop_item(first_backdrop, Rect::new(15.0, 0.0, 10.0, 10.0), 10.0),
            backdrop_item(second_backdrop, Rect::new(25.0, 0.0, 10.0, 10.0), 10.0),
        ]);

        let dirty = diff_layer(&previous, &next, &HashMap::new());
        assert_eq!(
            dirty.tiles(1.0, 10),
            [(0, 0), (1, 0), (2, 0)].into_iter().collect()
        );
    }
}
