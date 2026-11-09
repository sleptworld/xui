//! Drop targets for drags from outside the window: which node a drag at a
//! point is offered to, and what the runtime publishes so the host can answer
//! the platform while a drag is moving.
//!
//! Both read the `droppable` column, which holds only the nodes that declared
//! something, so an application with no drop target pays for neither.

use std::sync::Arc;

use crate::caps::Registered;
use crate::caps::behavior::Droppable;
use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::paint_pass::is_drawn;
use xui_interface::{
    Bounds, DragPayload, DropAccepts, DropEffect, DropTarget, NodeId, Point, Rect,
};

/// A drag from outside the window that is over it.
#[derive(Debug, Clone)]
pub(crate) struct ExternalDrag {
    pub payload: Arc<DragPayload>,
    /// Where it was last, in window coordinates.
    pub position: Point,
    /// The target it is over, and the effect that target declared.
    pub target: Option<(NodeId, DropEffect)>,
}

impl UiRuntime {
    /// The drop target a drag carrying `kinds` at `point` is offered to, with
    /// its effect.
    ///
    /// The innermost node under `point` that declared `drop_accepts`. If that
    /// node does not take what the drag carries, there is no target: a drop
    /// zone inside another refuses for both, which is also what
    /// [`xui_interface::PublishedSurface::drop_effect_at`] answers.
    pub(crate) fn drop_target_at(
        &self,
        point: Point,
        kinds: DropAccepts,
    ) -> Option<(NodeId, DropEffect)> {
        let hit = self.hit_test(point)?;
        let (node, accepts, effect) = self
            .hosts
            .ancestors(hit)
            .find_map(|id| self.drop_declaration(id).map(|(a, e)| (id, a, e)))?;
        (accepts.admits(kinds) && effect != DropEffect::None).then_some((node, effect))
    }

    /// What `id` declared as a drop target, if it is one.
    fn drop_declaration(&self, id: NodeId) -> Option<(DropAccepts, DropEffect)> {
        let row = Droppable::get(&self.columns, id)?;
        (!row.drop_accepts.is_empty()).then(|| {
            (
                row.drop_accepts,
                row.drop_effect.unwrap_or(DropEffect::Copy),
            )
        })
    }

    /// Every drop target the pointer could reach, in paint order, each clipped
    /// to what its ancestors show: what the host answers the platform from.
    ///
    /// A target under a modal layer, inside an overlay that takes no input, or
    /// in a subtree that is not drawn is left out, as hit testing leaves it out.
    /// One covered by a sibling that is no target is not: the host would offer
    /// a drop there that the runtime, hitting the sibling, then has nowhere to
    /// deliver. Rare enough to accept for not hit testing every point.
    pub(crate) fn drop_targets(&self) -> Vec<DropTarget> {
        let mut targets: Vec<(Vec<usize>, DropTarget)> = Droppable::iter(&self.columns)
            .filter_map(|(id, _)| {
                let (accepts, effect) = self.drop_declaration(id)?;
                let (order, rect) = self.reachable_rect(id)?;
                Some((
                    order,
                    DropTarget {
                        rect,
                        accepts,
                        effect,
                    },
                ))
            })
            .collect();
        // Document order is paint order: a later sibling, and a descendant,
        // draw over what comes before.
        targets.sort_by(|(a, _), (b, _)| a.cmp(b));
        targets.into_iter().map(|(_, target)| target).collect()
    }

    /// `id`'s position in document order (its child index at each level, from
    /// the root) and the part of it the pointer can reach, or `None` if it
    /// cannot be reached at all.
    fn reachable_rect(&self, id: NodeId) -> Option<(Vec<usize>, Rect)> {
        let path: Vec<NodeId> = self.event_path(id).into_iter().collect();
        if !path.contains(&self.root_overlayer) && self.modal_layer_open() {
            return None;
        }
        let mut order = Vec::with_capacity(path.len());
        let mut visible = self.visual_layout(id);
        for (depth, &node) in path.iter().enumerate() {
            let style = self.style_system.effective(node);
            if !is_drawn(style) {
                return None;
            }
            if let Some(&child) = path.get(depth + 1) {
                order.push(self.hosts.children(node).position(|c| c == child)?);
                if node == self.root_overlayer && !self.overlay_layer_reachable(child) {
                    return None;
                }
                if style.paint.clip || style.scroll.direction.is_scrollable() {
                    visible = intersect(visible, self.visual_layout(node))?;
                }
            }
        }
        let rect = Rect::new(
            visible.min.x,
            visible.min.y,
            visible.width(),
            visible.height(),
        );
        Some((order, rect))
    }

    /// Whether the overlay layer rooted at `layer` takes input: it hit tests,
    /// and no modal layer sits above it.
    fn overlay_layer_reachable(&self, layer: NodeId) -> bool {
        let mut above = false;
        for child in self.hosts.children(self.root_overlayer) {
            if above {
                let (_, modal) = self
                    .overlay_entry_interaction(child)
                    .unwrap_or((true, false));
                if modal {
                    return false;
                }
            } else if child == layer {
                above = true;
                let (hit_test, _) = self
                    .overlay_entry_interaction(child)
                    .unwrap_or((true, false));
                if !hit_test {
                    return false;
                }
            }
        }
        true
    }

    /// Whether a modal layer is open, which shuts off everything outside the
    /// overlays.
    fn modal_layer_open(&self) -> bool {
        self.hosts.children(self.root_overlayer).any(|child| {
            self.overlay_entry_interaction(child)
                .is_some_and(|(_, modal)| modal)
        })
    }

    pub(crate) fn external_drag_mut(&mut self) -> &mut Option<ExternalDrag> {
        &mut self.interaction_system.external_drag
    }
}

fn intersect(a: Bounds, b: Bounds) -> Option<Bounds> {
    let min = Point::new(a.min.x.max(b.min.x), a.min.y.max(b.min.y));
    let max = Point::new(a.max.x.min(b.max.x), a.max.y.min(b.max.y));
    (min.x < max.x && min.y < max.y).then_some(Bounds::new(min, max))
}
