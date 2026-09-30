//! Nodes placed against another node's on-screen bounds.
//!
//! Anchoring is its own subsystem: it knows nothing about overlays. A Portal is
//! today the only thing that anchors, and it registers its visual root here
//! when it commits, but the placement math, the per-frame offsets, and the
//! dependency order between anchors all live here.

use slotmap::SparseSecondaryMap;
use xui_interface::{NodeId, Point};

use crate::anchor::AnchorPlacement;
use crate::ui_runtime::tree::{HostData, HostTree};

/// What an anchored node follows, and where that has put it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Anchor {
    /// The host it is placed against. `None` leaves it where layout put it.
    pub(crate) target: Option<NodeId>,
    pub(crate) placement: AnchorPlacement,
    /// A translation applied at paint and hit-test time; layout never sees
    /// it. The total off the laid-out position, not a per-frame delta.
    pub(crate) offset: Point,
    /// The target's width, when `placement.match_width` asks for it. The one
    /// part of anchoring that reaches layout.
    pub(crate) width: Option<f32>,
}

impl Anchor {
    pub(crate) fn new(target: Option<NodeId>, placement: AnchorPlacement) -> Self {
        Self {
            target,
            placement,
            offset: Point::zero(),
            width: None,
        }
    }
}

/// One anchored node, ready to be placed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AnchorStep {
    pub(crate) node: NodeId,
    pub(crate) target: Option<NodeId>,
    pub(crate) placement: AnchorPlacement,
}

#[derive(Default)]
pub(crate) struct AnchorSystem {
    anchors: SparseSecondaryMap<NodeId, Anchor>,
}

impl AnchorSystem {
    pub(crate) fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    pub(crate) fn get(&self, id: NodeId) -> Option<&Anchor> {
        self.anchors.get(id)
    }

    /// Anchors `id` to `target`, or releases it with `None`. Returns whether
    /// `id` joined or left the set, which takes it out of flow or puts it back.
    ///
    /// Re-anchoring keeps the last offset and width, so a changed placement
    /// moves the node from where it is rather than from its laid-out spot.
    pub(crate) fn set(
        &mut self,
        id: NodeId,
        anchor: Option<(Option<NodeId>, AnchorPlacement)>,
    ) -> bool {
        match anchor {
            Some((target, placement)) => match self.anchors.get_mut(id) {
                Some(existing) => {
                    existing.target = target;
                    existing.placement = placement;
                    false
                }
                None => {
                    self.anchors.insert(id, Anchor::new(target, placement));
                    true
                }
            },
            None => self.anchors.remove(id).is_some(),
        }
    }

    pub(crate) fn remove(&mut self, id: NodeId) {
        self.anchors.remove(id);
    }

    /// How far `id` is translated off its laid-out position.
    ///
    /// Asked for every ancestor of every node placed on screen, so the common
    /// case -- nothing anchored at all -- skips the lookup.
    #[inline]
    pub(crate) fn offset(&self, id: NodeId) -> Point {
        if self.anchors.is_empty() {
            return Point::zero();
        }
        self.anchors
            .get(id)
            .map_or(Point::zero(), |anchor| anchor.offset)
    }

    /// Records a new offset. Returns whether it moved by more than float
    /// noise, i.e. whether the paint transform has to follow.
    pub(crate) fn set_offset(&mut self, id: NodeId, offset: Point) -> bool {
        let anchor = &mut self.anchors[id];
        let current = anchor.offset;
        if (offset.x - current.x).abs() < 0.01 && (offset.y - current.y).abs() < 0.01 {
            return false;
        }
        anchor.offset = offset;
        true
    }

    /// Records the matched width. Returns whether it changed.
    pub(crate) fn set_width(&mut self, id: NodeId, width: Option<f32>) -> bool {
        let anchor = &mut self.anchors[id];
        if anchor.width == width {
            return false;
        }
        anchor.width = width;
        true
    }

    /// Every anchored node, each after every anchored node its position
    /// depends on, so one pass in this order settles every chain.
    ///
    /// A node's on-screen bounds include every anchored ancestor's offset, and
    /// so do its target's. Both must be placed first:
    ///
    /// - the nearest anchored node strictly above it, and
    /// - the nearest anchored node at or above its target.
    ///
    /// Portal roots sit directly under the overlayer, so for them only the
    /// second can exist today; the first keeps the order right for any node.
    pub(crate) fn placement_order(&self, hosts: &HostTree<HostData>) -> Vec<AnchorStep> {
        let mut visited = SparseSecondaryMap::<NodeId, ()>::new();
        let mut order = Vec::with_capacity(self.anchors.len());
        for id in self.anchors.keys() {
            self.visit(id, hosts, &mut visited, &mut order);
        }
        order
    }

    fn visit(
        &self,
        id: NodeId,
        hosts: &HostTree<HostData>,
        visited: &mut SparseSecondaryMap<NodeId, ()>,
        order: &mut Vec<AnchorStep>,
    ) {
        // Marked on entry. A target is never inside what follows it, so
        // dependencies cannot loop; the mark only stops repeat visits.
        if visited.insert(id, ()).is_some() {
            return;
        }
        let Some(anchor) = self.anchors.get(id) else {
            return;
        };
        // A removed target leaves the node where layout put it.
        let target = anchor.target.filter(|target| hosts.contains_key(*target));
        let dependencies = [
            hosts
                .parent(id)
                .and_then(|parent| self.nearest_anchored(hosts, parent)),
            target.and_then(|target| self.nearest_anchored(hosts, target)),
        ];
        for dependency in dependencies.into_iter().flatten() {
            self.visit(dependency, hosts, visited, order);
        }
        order.push(AnchorStep {
            node: id,
            target,
            placement: anchor.placement,
        });
    }

    fn nearest_anchored(&self, hosts: &HostTree<HostData>, mut id: NodeId) -> Option<NodeId> {
        loop {
            if self.anchors.contains_key(id) {
                return Some(id);
            }
            id = hosts.parent(id)?;
        }
    }
}
