//! Read-only queries: tree shape, geometry, hit testing, focus and pointer state.

use crate::core::Point;
use crate::event_system::EventState;
use crate::event_system::callbacks::{EventHandlers, EventMask};
use crate::focus::FocusManager;
use crate::ui_runtime::{EventPath, NodeView, UiRuntime};
use crate::widgets::{WidgetType, Widgets};
use xui_interface::{Bounds, CursorIcon, Focusability, NodeId};

impl UiRuntime {
    pub fn contains(&self, id: NodeId) -> bool {
        self.hosts.contains_key(id)
    }

    /// A read-only projection of `id`, or `None` once it has been removed.
    ///
    /// A boundary: this is the one query that accepts an id that may be stale.
    pub fn node(&self, id: NodeId) -> Option<NodeView<'_>> {
        self.hosts.contains_key(id).then(|| self.node_view(id))
    }

    /// [`Self::node`] for an id known to be live.
    ///
    /// # Preconditions
    /// - `id` is live.
    pub(crate) fn node_view(&self, id: NodeId) -> NodeView<'_> {
        let (target, effective) = self.style_system.styles(id);
        let mut node = NodeView::new(
            id,
            &self.hosts[id],
            self.layout_tree.node(id),
            target,
            effective,
        );
        // Public/event-facing geometry follows the current visual position.
        // Retained layout origins intentionally exclude dynamic scroll offsets.
        node.world_origin = self.visual_layout(id).min;
        node
    }

    #[inline]
    pub fn children(
        &self,
        id: NodeId,
    ) -> impl DoubleEndedIterator<Item = NodeId> + ExactSizeIterator + '_ {
        self.hosts.children(id)
    }

    #[inline]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.hosts.parent(id)
    }

    #[inline]
    #[cfg(test)]
    pub(crate) fn anchors(&self) -> &super::anchor::AnchorSystem {
        &self.anchor_system
    }

    #[cfg(test)]
    pub(crate) fn compiled_scene(&self) -> Option<&crate::render::CompiledScene> {
        self.render_system.compiler.compiled_scene()
    }

    pub(crate) fn focused_node(&self) -> Option<NodeId> {
        self.interaction_system.focus.focused()
    }

    pub(crate) fn focus_manager(&self) -> &FocusManager {
        &self.interaction_system.focus
    }

    pub(crate) fn resolve_local_shortcut(
        &self,
        event: &xui_interface::RawKeyboard,
    ) -> Option<(NodeId, xui_interface::ShortcutBinding)> {
        let mut current = self
            .focused_node()
            .or_else(|| self.children(self.root).next())
            .or(Some(self.root));
        while let Some(id) = current {
            let interaction = self.interaction_system.get(id);
            if let Some(binding) = interaction
                .into_iter()
                .flat_map(|node| node.properties.shortcuts.iter())
                .rev()
                .find(|binding| binding.shortcut.matches(event))
            {
                return Some((id, *binding));
            }
            current = self.hosts.parent(id);
        }
        None
    }

    pub(crate) fn pointer_capture_node(&self) -> Option<NodeId> {
        self.interaction_system.event_state.pointer_capture()
    }

    pub(crate) fn event_state(&self) -> &EventState {
        &self.interaction_system.event_state
    }

    /// The handlers `id` registered, empty when it registered none.
    #[inline]
    pub(crate) fn handlers(&self, id: NodeId) -> &EventHandlers {
        self.interaction_system.handlers(id)
    }

    /// Whether any live host reads raw device events at all.
    /// The pointer shape the window should be showing.
    ///
    /// Resolved rather than tracked: this is a read of state that already
    /// exists, so it can be pulled once per dispatch instead of invalidated.
    /// That matters because a cursor can change without the pointer moving — a
    /// button becoming disabled under the pointer, or a drag starting and
    /// turning `Grab` into `Grabbing` through `WidgetState::DRAGGING`. Tracking
    /// every such source would be a standing bug farm.
    ///
    /// A captured pointer wins over hit testing, so drag-selecting out of a text
    /// input keeps the I-beam instead of picking up whatever is underneath.
    /// `cursor` is not inherited in the computed style, so this walks up to the
    /// nearest ancestor that specifies one.
    pub(crate) fn resolved_cursor(&self) -> CursorIcon {
        let Some(source) = self.pointer_capture_node().or_else(|| self.hovered_node()) else {
            return CursorIcon::default();
        };

        let mut current = Some(source);
        while let Some(id) = current {
            if let Some(cursor) = self.style_system.effective(id).cursor {
                return cursor;
            }
            current = self.hosts.parent(id);
        }
        CursorIcon::default()
    }

    pub(crate) fn hovered_node(&self) -> Option<NodeId> {
        self.interaction_system.event_state.hovered()
    }

    pub(crate) fn has_raw_event_listeners(&self) -> bool {
        self.raw_event_listeners > 0
    }

    pub(crate) fn node_reads_raw_events(&self, id: NodeId) -> bool {
        self.hosts[id].reads_raw_events
    }

    pub(crate) fn listens_for(&self, id: NodeId, mask: EventMask) -> bool {
        self.interaction_system
            .get(id)
            .is_some_and(|node| node.handlers.listens_for(mask))
    }

    pub(crate) fn has_drag_callbacks(&self, id: NodeId) -> bool {
        self.listens_for(id, EventMask::DRAG)
    }

    pub(crate) fn is_focusable(&self, id: NodeId) -> bool {
        let node = &self.hosts[id];
        let interaction = self.interaction_system.get(id);
        let focus = interaction
            .map(|node| node.properties.focus)
            .unwrap_or_default();
        match focus.focusability {
            Focusability::Focusable => true,
            Focusability::NotFocusable => false,
            Focusability::Auto => {
                focus.tab_index.is_some()
                    || matches!(node.node_type, WidgetType::Button | WidgetType::TextInput)
                    || interaction.is_some_and(|node| node.handlers.listens_for(EventMask::FOCUS))
            }
        }
    }

    pub(crate) fn is_sequentially_focusable(&self, id: NodeId) -> bool {
        self.is_focusable(id)
            && self
                .interaction_system
                .get(id)
                .and_then(|node| node.properties.focus.tab_index)
                .is_none_or(|index| index >= 0)
    }

    pub(crate) fn tab_index(&self, id: NodeId) -> Option<i32> {
        self.interaction_system
            .get(id)
            .and_then(|node| node.properties.focus.tab_index)
    }

    #[cfg(test)]
    pub(crate) fn accessibility(
        &self,
        id: NodeId,
    ) -> Option<&xui_interface::AccessibilityProperties> {
        self.interaction_system
            .get(id)
            .map(|node| &node.properties.accessibility)
    }

    /// `viewport_pos` in `id`'s local coordinates: relative to its border box,
    /// with every ancestor's scroll and anchor applied.
    ///
    /// # Preconditions
    /// - `id` is live.
    pub(crate) fn to_local(&self, id: NodeId, viewport_pos: Point) -> Point {
        let mut local = viewport_pos;
        let mut current = id;
        loop {
            local = local - self.layout_tree.node(current).layout.origin();
            let Some(parent) = self.hosts.parent(current) else {
                return local;
            };
            local =
                local - self.anchor_offset(current) + self.layout_tree.node(parent).scroll_offset;
            current = parent;
        }
    }

    #[inline(always)]
    pub fn hit_test(&self, point: crate::core::Point) -> Option<NodeId> {
        match self.hit_test_from(self.root, point, Point::zero()) {
            HitTestOutcome::Hit(id) => Some(id),
            HitTestOutcome::Miss | HitTestOutcome::Blocked => None,
        }
    }

    /// Returns a node's layout rectangle in window logical coordinates after
    /// applying scroll offsets from its ancestors.
    ///
    /// # Preconditions
    /// - `id` is live.
    pub(crate) fn visual_layout(&self, id: NodeId) -> Bounds {
        // Scrolling moves content back, an anchor moves it forward.
        let mut scroll_offset = Point::zero() - self.anchor_offset(id);
        let mut cursor = self.hosts.parent(id);
        while let Some(parent) = cursor {
            scroll_offset = scroll_offset - self.anchor_offset(parent);
            if self
                .style_system
                .computed(parent)
                .scroll
                .direction
                .is_scrollable()
            {
                scroll_offset = scroll_offset + self.layout_tree.node(parent).scroll_offset;
            }
            cursor = self.hosts.parent(parent);
        }
        self.layout_tree.node(id).visual_bounds(scroll_offset)
    }

    fn hit_test_from(
        &self,
        id: NodeId,
        point: crate::core::Point,
        ancestor_scroll_offset: Point,
    ) -> HitTestOutcome {
        let layout = self.layout_tree.node(id);
        let node_style = self.style_system.effective(id);
        let ancestor_scroll_offset = ancestor_scroll_offset - self.anchor_offset(id);
        let visual_layout = layout.visual_bounds(ancestor_scroll_offset);
        let contains_point = visual_layout.contains(point);
        let clips_children = node_style.paint.clip || node_style.scroll.direction.is_scrollable();

        if clips_children
            && !hit_test_clip_contains(visual_layout, node_style.paint.border_radius, point)
        {
            return HitTestOutcome::Miss;
        }

        let child_scroll_offset = if node_style.scroll.direction.is_scrollable() {
            Point::new(
                ancestor_scroll_offset.x + layout.scroll_offset.x,
                ancestor_scroll_offset.y + layout.scroll_offset.y,
            )
        } else {
            ancestor_scroll_offset
        };

        if self.root_overlayer() == id {
            return self.hit_test_root_overlayer(point, child_scroll_offset);
        }

        for child in self.hosts.children(id).rev() {
            match self.hit_test_from(child, point, child_scroll_offset) {
                HitTestOutcome::Miss => {}
                outcome => return outcome,
            }
        }

        if contains_point {
            HitTestOutcome::Hit(id)
        } else {
            HitTestOutcome::Miss
        }
    }

    fn hit_test_root_overlayer(
        &self,
        point: crate::core::Point,
        ancestor_scroll_offset: Point,
    ) -> HitTestOutcome {
        for child in self.hosts.children(self.root_overlayer).rev() {
            let (hit_test, modal) = self
                .overlay_entry_interaction(child)
                .unwrap_or((true, false));

            if hit_test {
                match self.hit_test_from(child, point, ancestor_scroll_offset) {
                    HitTestOutcome::Miss => {}
                    outcome => return outcome,
                }
            }

            // A modal entry is a stacking barrier even when the point is
            // outside its visual root. This prevents hits from falling through
            // to lower overlays or application content.
            if modal {
                return HitTestOutcome::Blocked;
            }
        }

        // The transparent RootOverlayer surface never intercepts input.
        HitTestOutcome::Miss
    }

    /// The topmost Portal with an `on_dismiss` handler, as its visual root and
    /// handler. A modal Portal above it without one shields it: nothing below
    /// a modal layer can be reached, so nothing below it is dismissed either.
    pub(crate) fn dismissable_overlay(&self) -> Option<(NodeId, crate::widgets::DismissHandler)> {
        let widget = self.hosts[self.root_overlayer].widget.clone();
        widget.with_widgets(|widgets| {
            let Widgets::RootOverlayer(overlayer) = widgets else {
                unreachable!("runtime root overlayer has the wrong widget type")
            };
            // Children are kept in paint order, so the last is on top.
            for root in self.hosts.children(self.root_overlayer).rev() {
                let Some(entry) = overlayer
                    .entry_for_visual_root(root)
                    .and_then(|entry| overlayer.entry(entry))
                else {
                    continue;
                };
                if let Some(handler) = entry.on_dismiss() {
                    return Some((root, handler.clone()));
                }
                if entry.modal() {
                    return None;
                }
            }
            None
        })
    }

    /// Whether a pointer at `point` lands inside the Portal rooted at
    /// `visual_root`, or on a layer painted above it -- a tooltip over a menu
    /// is not outside the menu.
    pub(crate) fn overlay_contains_pointer(&self, visual_root: NodeId, point: Point) -> bool {
        let Some(hit) = self.hit_test(point) else {
            return false;
        };
        let Some(hit_root) = self
            .hosts
            .ancestors(hit)
            .find(|id| self.hosts.parent(*id) == Some(self.root_overlayer))
        else {
            return false;
        };
        let layers: Vec<NodeId> = self.hosts.children(self.root_overlayer).collect();
        let layer = |root| layers.iter().position(|id| *id == root);
        layer(hit_root) >= layer(visual_root)
    }

    fn overlay_entry_interaction(&self, visual_root: NodeId) -> Option<(bool, bool)> {
        let widget = self.hosts[self.root_overlayer].widget.clone();
        widget.with_widgets(|widgets| {
            let Widgets::RootOverlayer(overlayer) = widgets else {
                unreachable!("runtime root overlayer has the wrong widget type")
            };
            let entry = overlayer.entry_for_visual_root(visual_root)?;
            let entry = overlayer.entry(entry)?;
            Some((entry.hit_test(), entry.modal()))
        })
    }

    pub(crate) fn event_path(&self, target: NodeId) -> EventPath {
        let mut path: EventPath = self.hosts.ancestors(target).collect();
        path.reverse();
        path
    }

    /// How far `id` is translated off its laid-out position by an anchor.
    pub(super) fn anchor_offset(&self, id: NodeId) -> Point {
        self.anchor_system.offset(id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HitTestOutcome {
    Miss,
    Hit(NodeId),
    Blocked,
}

fn hit_test_clip_contains(bounds: Bounds, radius: f32, point: Point) -> bool {
    if !bounds.contains(point) {
        return false;
    }

    let radius = radius
        .max(0.0)
        .min(bounds.width().max(0.0) * 0.5)
        .min(bounds.height().max(0.0) * 0.5);
    if radius == 0.0 {
        return true;
    }

    let center_x = point.x.clamp(bounds.min.x + radius, bounds.max.x - radius);
    let center_y = point.y.clamp(bounds.min.y + radius, bounds.max.y - radius);
    let dx = point.x - center_x;
    let dy = point.y - center_y;
    dx * dx + dy * dy <= radius * radius
}
