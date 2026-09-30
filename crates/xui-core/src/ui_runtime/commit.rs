//! Fiber commit: the one door through which the host tree changes shape.

use crate::anchor::AnchorPlacement;
use crate::event_system::interaction::HostInteraction;
use crate::fiber::Key;
use crate::layout::{ParentLayout, computed_style_for_widget, taffy_style_for_widget};
use crate::render::{HostRenderBinding, SceneError};
use crate::ui_runtime::interaction::InteractionSystem;
use crate::ui_runtime::layout::{LayoutTree, WidgetContext};
use crate::ui_runtime::render::RenderSystem;
use crate::ui_runtime::state::{HostWorkFlags, UiState};
use crate::ui_runtime::style::StyleSystem;
use crate::ui_runtime::tree::{HostData, HostTree};
use crate::ui_runtime::{FrameStats, UiRuntime};
use crate::widgets::{
    OverlayEntryId, OverlayEntryOptions, OverlayModelError, OverlayScopeId, WidgetI, WidgetType,
    Widgets,
};
use taffy::prelude as tf;
use xui_interface::{
    Affine, ComputedStyle, NodeId, NodeLifecycleEvent, Theme, WidgetState, WidgetUpdateFlags,
};

impl UiRuntime {
    pub(crate) fn new() -> Self {
        let mut layout_tree = LayoutTree::new();

        // Default Theme
        let theme = Theme::default();
        let root_widget = crate::widgets::root_widget();
        let root_parent_style = ComputedStyle::initial(&theme);
        let root_computed_style = computed_style_for_widget(
            &root_widget,
            &root_parent_style,
            &theme,
            WidgetState::empty(),
        );
        // The root has no parent; block flow is the neutral choice and the
        // root sizes itself to the viewport regardless.
        let root_taffy_style =
            taffy_style_for_widget(&root_widget, ParentLayout::Block, &root_computed_style);
        // Initialize Host Tree
        let mut hosts = HostTree::new();
        let root = hosts.insert_with_key(|_| HostData::new(None, 0, root_widget));
        // Layout Binding
        layout_tree.create(root, root_taffy_style, None);

        // Default Root Style
        let default_style = ComputedStyle::initial(&theme);
        let mut style_system = StyleSystem::new(default_style);
        style_system.create(root, root_computed_style, true);

        // Create Self
        let mut arena = Self {
            hosts,
            layout_tree,
            root,
            root_overlayer: root,
            node_lifecycle_events: Vec::new(),
            interaction_system: InteractionSystem::new(),
            text_nodes: slotmap::SparseSecondaryMap::new(),
            canvas_nodes: slotmap::SparseSecondaryMap::new(),
            canvases_wanting_repaint: slotmap::SparseSecondaryMap::new(),
            window_visible: true,
            frame_time: crate::clock::FrameTime::ZERO,
            canvas_invalidations: crate::widgets::CanvasInvalidator::default(),
            tickers: crate::ticker::TickerRegistry::default(),
            scale_factor: 1.0,
            gpu_context: None,
            raw_event_listeners: 0,
            theme,
            stats: FrameStats::default(),
            style_system,
            anchor_system: Default::default(),
            ui_state: UiState::default(),
            render_system: RenderSystem::new(),
        };

        arena
            .create_host_render_binding(root)
            .expect("failed to create root render binding");

        // The runtime owns exactly one overlayer. Application hosts are always
        // inserted before it so this remains the final paint branch.
        let root_overlayer_widget = WidgetI::new(crate::widgets::root_overlayer_widget());
        let key = root_overlayer_widget.key();
        let props_hash = root_overlayer_widget.props_hash();
        let root_overlayer = arena.create_node(key, props_hash, root_overlayer_widget, None);
        arena.root_overlayer = root_overlayer;
        arena.attach(root, root_overlayer, None);
        arena.node_lifecycle_events.clear();

        arena.mark_work(root, HostWorkFlags::MOUNT);
        arena
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    /// The runtime-owned visual parent for Portal-mounted overlay entries.
    pub(crate) fn root_overlayer(&self) -> NodeId {
        self.root_overlayer
    }

    pub(crate) fn mount_overlay_entry(
        &mut self,
        visual_root: NodeId,
        scope: Option<OverlayScopeId>,
        options: OverlayEntryOptions,
    ) -> Result<OverlayEntryId, OverlayModelError> {
        let widget = self.hosts[self.root_overlayer].widget.clone();
        let (entry, order) = widget.with_widgets_mut(|widgets| {
            let Widgets::RootOverlayer(overlayer) = widgets else {
                unreachable!("runtime root overlayer has the wrong widget type")
            };
            let scope = scope.unwrap_or_else(|| overlayer.root_scope());
            let entry = overlayer.insert_entry(scope, visual_root, options)?;
            Ok((entry, overlayer.visual_roots_in_paint_order()))
        })?;
        self.sync_overlayer_children(order);
        Ok(entry)
    }

    pub(crate) fn update_overlay_entry(
        &mut self,
        entry: OverlayEntryId,
        scope: Option<OverlayScopeId>,
        options: OverlayEntryOptions,
    ) -> Result<(), OverlayModelError> {
        let widget = self.hosts[self.root_overlayer].widget.clone();
        let order = widget.with_widgets_mut(|widgets| {
            let Widgets::RootOverlayer(overlayer) = widgets else {
                unreachable!("runtime root overlayer has the wrong widget type")
            };
            let next_scope = scope.unwrap_or_else(|| overlayer.root_scope());
            overlayer.move_entry(entry, next_scope)?;
            overlayer.update_entry_options(entry, options)?;
            Ok(overlayer.visual_roots_in_paint_order())
        })?;
        self.sync_overlayer_children(order);
        Ok(())
    }

    pub(crate) fn unmount_overlay_entry(
        &mut self,
        entry: OverlayEntryId,
    ) -> Result<NodeId, OverlayModelError> {
        let widget = self.hosts[self.root_overlayer].widget.clone();
        let (visual_root, order) = widget.with_widgets_mut(|widgets| {
            let Widgets::RootOverlayer(overlayer) = widgets else {
                unreachable!("runtime root overlayer has the wrong widget type")
            };
            let visual_root = overlayer.remove_entry(entry)?;
            Ok((visual_root, overlayer.visual_roots_in_paint_order()))
        })?;
        self.sync_overlayer_children(order);
        Ok(visual_root)
    }

    /// Records the handler that closes an entry; see [`DismissReason`].
    ///
    /// [`DismissReason`]: crate::widgets::DismissReason
    pub(crate) fn set_overlay_entry_dismiss(
        &mut self,
        entry: OverlayEntryId,
        on_dismiss: Option<crate::widgets::DismissHandler>,
    ) -> Result<(), OverlayModelError> {
        let widget = self.hosts[self.root_overlayer].widget.clone();
        widget.with_widgets_mut(|widgets| {
            let Widgets::RootOverlayer(overlayer) = widgets else {
                unreachable!("runtime root overlayer has the wrong widget type")
            };
            overlayer.set_entry_dismiss(entry, on_dismiss)
        })
    }

    /// Places `node` against `target` every frame, or releases it with `None`.
    /// A `None` target keeps the node anchored -- out of flow -- but where
    /// layout put it.
    pub(crate) fn set_anchor(
        &mut self,
        node: NodeId,
        anchor: Option<(Option<NodeId>, AnchorPlacement)>,
    ) {
        // A boundary: a Portal releasing a visual root it already swapped out
        // may name a node that is gone, and removal dropped its anchor.
        if !self.hosts.contains_key(node) || !self.anchor_system.set(node, anchor) {
            return;
        }
        // An anchored node is always out of flow, and draws at its offset;
        // joining or leaving changes both. Before its first style pass there
        // is no Taffy style yet, and that pass applies it.
        if !self.style_system.initialized(node) {
            return;
        }
        let mut work = HostWorkFlags::SYNC_TRANSFORM;
        if self.sync_effective_taffy_style(node) {
            work |= HostWorkFlags::RECALC_LAYOUT;
        }
        self.mark_work(node, work);
    }

    fn create_host_render_binding(&mut self, host: NodeId) -> Result<(), SceneError> {
        let root = self.render_system.scene.insert_transform(Affine::IDENTITY);
        let transform = self.render_system.scene.insert_transform(Affine::IDENTITY);
        let contents = self.render_system.scene.insert_group();
        let paint = self.render_system.scene.insert_group();
        self.render_system.scene.append_child(contents, paint)?;

        self.render_system
            .scene
            .set_child(transform, Some(contents))?;
        self.render_system.scene.set_child(root, Some(transform))?;
        self.render_system.bind_host(
            host,
            HostRenderBinding::scaffold(root, transform, contents, paint, None, None, None),
        )?;

        if host == self.root {
            self.render_system
                .scene
                .append_child(self.render_system.scene.root(), root)?;
        }
        Ok(())
    }

    pub(crate) fn create_node(
        &mut self,
        key: Option<Key>,
        props_hash: u64,
        widget: WidgetI,
        interaction: Option<HostInteraction>,
    ) -> NodeId {
        let id = self
            .hosts
            .insert_with_key(|_| HostData::new(key, props_hash, widget));
        let host = &self.hosts[id];
        if host.reads_raw_events {
            self.raw_event_listeners += 1;
        }
        let node_type = host.node_type;
        let context = taffy_context(id, host);
        self.style_system
            .create(id, self.style_system.default_style().clone(), false);
        self.interaction_system.update(id, interaction);
        self.layout_tree.create(id, tf::Style::default(), context);
        self.create_host_render_binding(id)
            .expect("failed to create host render binding");
        self.node_lifecycle_events
            .push(NodeLifecycleEvent::Created(id));

        let mut work = HostWorkFlags::MOUNT;
        match node_type {
            WidgetType::Canvas => {
                self.canvas_nodes.insert(id, ());
                self.bind_canvas_controller(id);
                work |= HostWorkFlags::SHAPE_CHANGE;
            }
            WidgetType::Text | WidgetType::TextInput => {
                self.text_nodes.insert(id, ());
            }
            _ => {}
        }
        self.mark_work(id, work);
        id
    }

    /// Places `child` under `parent`, before `before` or last, moving it from
    /// wherever it was. The one way the fiber commit attaches or moves a host.
    ///
    /// A `before` that is not a child of `parent` places `child` last: the
    /// reconciler's next-sibling search can land on a host a Portal moved
    /// under the overlayer.
    ///
    /// # Preconditions
    /// - `parent`, `child` and `before` are live, and `child` is neither the
    ///   root, the root overlayer, nor `before`.
    pub(crate) fn place(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
        debug_assert!(
            child != self.root && child != self.root_overlayer && Some(child) != before,
            "place: cannot place {child:?} there"
        );
        let before = before
            .filter(|before| self.hosts.parent(*before) == Some(parent))
            // Application content always paints below the overlayer.
            .or((parent == self.root).then_some(self.root_overlayer));
        self.attach(parent, child, before);
    }

    fn attach(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
        let old_parent = self.hosts.parent(child);
        if old_parent == Some(parent) && self.hosts.link(child).next_sibling == before {
            return;
        }
        if let Some(old_parent) = old_parent {
            self.hosts.detach(child);
            self.layout_tree.detach(child);
            self.mark_work(old_parent, HostWorkFlags::CHILDREN_CHANGED);
        }
        self.hosts.insert(parent, child, before);
        self.layout_tree.insert(parent, child, before);
        if old_parent != Some(parent) {
            self.mark_work(child, HostWorkFlags::REPARENTED);
        }
        self.mark_work(parent, HostWorkFlags::CHILDREN_CHANGED);
    }

    /// Removes `id` and everything under it from every subsystem.
    ///
    /// # Preconditions
    /// - `id` is live, and is neither the root nor the root overlayer.
    pub(crate) fn remove_subtree(&mut self, id: NodeId) {
        debug_assert!(
            id != self.root && id != self.root_overlayer,
            "remove_subtree: {id:?} is runtime-owned"
        );
        if let Some(parent) = self.hosts.parent(id) {
            self.hosts.detach(id);
            self.layout_tree.detach(id);
            self.mark_work(parent, HostWorkFlags::CHILDREN_CHANGED);
        }
        let removal: Vec<_> = self.hosts.subtree(id).collect();
        // Children first: a host is removed only once it has none.
        for removed in removal.into_iter().rev() {
            self.drop_node(removed);
        }
    }

    /// Everything `create_node` set up for `id`, torn down again.
    fn drop_node(&mut self, id: NodeId) {
        if self.hosts[id].reads_raw_events {
            self.raw_event_listeners -= 1;
        }
        if self.canvas_nodes.remove(id).is_some() {
            self.unbind_canvas_controller(id);
        }
        self.canvases_wanting_repaint.remove(id);
        self.text_nodes.remove(id);
        self.interaction_system.remove(id);
        self.style_system.remove(id);
        self.anchor_system.remove(id);
        // Its parent is gone too, or it was detached above.
        self.layout_tree.forget(id);
        if let Some(binding) = self.render_system.unbind_host(id) {
            self.render_system
                .properties
                .remove_source(binding.transform);
            self.render_system
                .scene
                .remove_subtree(binding.root)
                .expect("failed to remove host render subtree");
        }
        self.hosts.remove(id);
        self.node_lifecycle_events
            .push(NodeLifecycleEvent::Removed(id));
    }

    pub(crate) fn drain_node_lifecycle_events(&mut self) -> Vec<NodeLifecycleEvent> {
        std::mem::take(&mut self.node_lifecycle_events)
    }

    pub(crate) fn update_node(
        &mut self,
        id: NodeId,
        key: Option<Key>,
        props_hash: u64,
        widget: WidgetI,
        interaction: Option<HostInteraction>,
    ) -> WidgetI {
        let mut flags = WidgetUpdateFlags::empty();
        let current_widget;
        {
            let node = &mut self.hosts[id];

            node.key = key;
            node.new_props_hash = props_hash;

            let widget_flags = node.widget.update_from(&widget);

            flags |= widget_flags;
            current_widget = node.widget.clone();
        }
        self.interaction_system.update(id, interaction);
        if self.focused_node() == Some(id) && !self.is_focusable(id) {
            self.interaction_system
                .focus
                .request_focus(None, xui_interface::FocusReason::Disabled);
        }

        self.refresh_taffy_context(id);
        self.request_update(id, flags);
        current_widget
    }

    /// Makes the overlayer's children exactly `order`, the overlay model's
    /// paint order. Entries leaving are detached, not removed: the Portal that
    /// owned one removes its host itself, or mounts it again.
    ///
    /// A boundary: the overlay model is updated in a different commit step
    /// from host deletion, so mid-commit it can still list a visual root
    /// that is already gone. Those are skipped.
    ///
    /// # Preconditions
    /// - The ids in `order` are distinct and not runtime-owned.
    fn sync_overlayer_children(&mut self, mut order: Vec<NodeId>) {
        order.retain(|id| self.hosts.contains_key(*id));
        let overlayer = self.root_overlayer;
        if self.hosts.children(overlayer).eq(order.iter().copied()) {
            return;
        }
        let leaving: Vec<_> = self
            .hosts
            .children(overlayer)
            .filter(|child| !order.contains(child))
            .collect();
        for child in leaving {
            self.hosts.detach(child);
            self.layout_tree.detach(child);
            self.mark_work(child, HostWorkFlags::REPARENTED);
            self.mark_work(overlayer, HostWorkFlags::CHILDREN_CHANGED);
        }
        // Appending each in turn leaves them in `order`; one already last
        // stays put.
        for child in order {
            self.attach(overlayer, child, None);
        }
    }

    fn bind_canvas_controller(&mut self, id: NodeId) {
        let invalidator = self.canvas_invalidator();
        self.hosts[id].widget.with_widgets_mut(|widget| {
            if let Widgets::Canvas(canvas) = widget {
                canvas.bind(id, invalidator);
            }
        });
    }

    fn unbind_canvas_controller(&mut self, id: NodeId) {
        self.hosts[id].widget.with_widgets_mut(|widget| {
            if let Widgets::Canvas(canvas) = widget {
                canvas.unbind();
            }
        });
    }
}

impl Default for UiRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// What Taffy's measure callback needs to size `id` as a leaf.
pub(super) fn taffy_context(id: NodeId, host: &HostData) -> Option<WidgetContext> {
    match host.node_type {
        WidgetType::Text | WidgetType::TextInput => Some(WidgetContext::Text(id)),
        WidgetType::Image => host.widget.intrinsic_size().map(WidgetContext::Image),
        _ => None,
    }
}
