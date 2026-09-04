//! Cross-subsystem orchestration for [`UiRuntime`].

use crate::animation::has_animatable_difference;
use crate::core::{Point, Size};
use crate::event_system::callbacks::{EventHandlers, EventMask};
use crate::event_system::interaction::HostInteraction;
use crate::event_system::{self, EventState, translator::EventTranslator};
use crate::fiber::Key;
use crate::focus::FocusManager;
use crate::layout::{
    ParentLayout, computed_style_for_widget, container_layout, taffy_style_for_widget,
};
use crate::render::{
    ClipShape, FrameProperties, HostRenderBinding, LayerDescriptor, Primitive, RenderScene,
    RenderTreeWriter, SceneError, Shape, ShapePrimitive,
};
use crate::text::{TextHost, TextLayoutSlot};
use crate::ui_runtime::interaction::InteractionSystem;
use crate::ui_runtime::layout::{LayoutTree, MeasuredLeaf, WidgetContext};
use crate::ui_runtime::render::RenderSystem;
use crate::ui_runtime::state::HostWorkFlags;
use crate::ui_runtime::state::UiState;
use crate::ui_runtime::style::StyleSystem;
use crate::ui_runtime::tree::{HostData, HostTree};
use crate::ui_runtime::{NodeView, RenderFrame, RenderFrameError, UiRuntime};
use crate::widgets::{
    OverlayEntryId, OverlayEntryOptions, OverlayModelError, OverlayScopeId, WidgetI, WidgetType,
    Widgets, canvas_text_slot,
};
use std::time::Duration;
use taffy::prelude as tf;
use xui_interface::CursorIcon;
use xui_interface::events::RawEvent;
use xui_interface::{
    AccessibilityProperties, Affine, Bounds, ComputedColorStyle, ComputedScrollStyle,
    ComputedScrollbarStyle, ComputedStyle, EventResult, Focusability, NodeId, NodeLifecycleEvent,
    ScrollbarVisibilityStyle, TextBackend, TextLayoutConstraints, TextLayoutInput, Theme,
    WidgetState, WidgetUpdateFlags,
};

impl UiRuntime {
    pub fn new() -> Self {
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
        layout_tree.create_host(root, root_taffy_style);

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
            canvas_invalidations: crate::widgets::CanvasInvalidator::default(),
            scale_factor: 1.0,
            raw_event_listeners: 0,
            theme,
            update_visits: 0,
            layout_passes: 0,
            repaint_passes: 0,
            style_system,
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
        arena.append_child(root, root_overlayer);
        arena.node_lifecycle_events.clear();

        arena.mark_work(
            root,
            HostWorkFlags::RECALC_STYLE
                | HostWorkFlags::RECALC_LAYOUT
                | HostWorkFlags::REBUILD_PAINT,
        );
        arena
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    /// The runtime-owned visual parent for Portal-mounted overlay entries.
    pub fn root_overlayer(&self) -> NodeId {
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
        self.set_children(self.root_overlayer, order);
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
        self.set_children(self.root_overlayer, order);
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
        self.set_children(self.root_overlayer, order);
        Ok(visual_root)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.hosts.contains_key(id)
    }

    pub fn node(&self, id: NodeId) -> Option<NodeView<'_>> {
        let (target, effective) = self.style_system.styles(id)?;
        let mut node = NodeView::new(
            id,
            self.hosts.get(id)?,
            self.layout_tree.host(id)?,
            target,
            effective,
        );
        // Public/event-facing geometry follows the current visual position.
        // Retained layout origins intentionally exclude dynamic scroll offsets.
        node.world_origin = self.visual_layout(id)?.min;
        Some(node)
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
    pub(crate) fn layout_node_id(&self, id: NodeId) -> tf::NodeId {
        self.layout_tree.node_id(id)
    }

    #[inline]
    pub fn render_scene(&self) -> &RenderScene {
        &self.render_system.scene
    }

    #[inline]
    pub fn compiled_scene(&self) -> Option<&crate::render::CompiledScene> {
        self.render_system.compiler.compiled_scene()
    }

    #[inline]
    pub fn frame_properties(&self) -> &FrameProperties {
        &self.render_system.properties
    }

    #[inline]
    pub fn frame_properties_mut(&mut self) -> &mut FrameProperties {
        &mut self.render_system.properties
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

    #[inline(always)]
    fn sync_render_scene(&mut self) -> Result<(), SceneError> {
        self.sync_render_dirty_subtree(self.root)
    }

    fn sync_render_dirty_subtree(&mut self, id: NodeId) -> Result<(), SceneError> {
        let relevant =
            HostWorkFlags::SYNC_RENDER | HostWorkFlags::SYNC_TREE | HostWorkFlags::REBUILD_PAINT;
        let work = self.hosts[id].work;
        let subtree_work = self.hosts[id].subtree_work;
        if !work.intersects(relevant) && !subtree_work.intersects(relevant) {
            return Ok(());
        }
        if work.intersects(relevant) {
            self.sync_host_render_node(id)?;
        }
        // `sync_host_render_node` only rewrites the render scene, so the host
        // sibling chain stays valid while it is walked in place.
        let mut cursor = self.hosts.link(id).and_then(|node| node.first_child);
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).and_then(|node| node.next_sibling);
            let child_work = self.hosts[child].work | self.hosts[child].subtree_work;
            if !child_work.intersects(relevant) {
                continue;
            }
            self.sync_render_dirty_subtree(child)?;
        }
        Ok(())
    }

    fn sync_host_render_node(&mut self, id: NodeId) -> Result<(), SceneError> {
        let mut binding = self
            .render_system
            .host_binding(id)
            .copied()
            .expect("host render binding missing");
        let host_children: Vec<_> = self.hosts.children(id).collect();

        let (
            local_origin,
            viewport,
            scroll,
            host_children,
            needs_scroll,
            needs_overlay,
            clip_shape,
            layer_descriptor,
        ) = {
            let host = &self.hosts[id];
            let layout = self.layout_tree.host(id).expect("layout node missing");
            let (target, effective) = self.style_system.styles(id).expect("style node missing");
            let node = NodeView::new(id, host, layout, target, effective);
            let viewport = Bounds::from_origin_size(Point::zero(), layout.layout.size());
            let needs_scroll = effective.scroll.is_scrollable();
            let needs_clip = node.effective_style.paint.clip || needs_scroll;
            let clip_shape = needs_clip.then_some({
                if node.effective_style.paint.border_radius > 0.0 {
                    ClipShape::RoundedRect {
                        rect: viewport,
                        radius: node.effective_style.paint.border_radius,
                    }
                } else {
                    ClipShape::Rect(viewport)
                }
            });

            (
                layout.layout.origin(),
                viewport,
                layout.scroll_offset,
                host_children,
                needs_scroll,
                needs_scrollbar_overlay(node),
                clip_shape,
                layer_descriptor_from_style(node.effective_style, viewport),
            )
        };

        // Reconcile the fixed host scaffold.
        self.render_system.scene.update_transform(
            binding.root,
            Affine::translate(local_origin.x, local_origin.y),
        )?;

        self.update_wrappers(id, &mut binding, clip_shape, layer_descriptor)?;

        // The children group is a stable container once it has been created.
        if !host_children.is_empty() && binding.children.is_none() {
            let children = self.render_system.scene.insert_group();
            // `paint` is always at index 0. Insert the child branch at index 1
            // so a scrollbar overlay remains last.
            self.render_system
                .scene
                .insert_child(binding.contents, 1, children)?;
            binding.children = Some(children);
        }

        // Scroll transforms are transient and exist only while scrolling is enabled.
        let removed_scroll_transform = if needs_scroll {
            if binding.scroll_transform.is_none()
                && let Some(children) = binding.children
            {
                self.render_system.scene.detach(children)?;
                let scroll_transform = self.render_system.scene.insert_transform(Affine::IDENTITY);
                self.render_system
                    .scene
                    .set_child(scroll_transform, Some(children))?;
                self.render_system
                    .scene
                    .insert_child(binding.contents, 1, scroll_transform)?;
                binding.scroll_transform = Some(scroll_transform);
            }
            None
        } else if let Some(scroll_transform) = binding.scroll_transform.take() {
            if let Some(children) = binding.children {
                self.render_system.scene.set_child(scroll_transform, None)?;
                self.render_system.scene.detach(scroll_transform)?;
                self.render_system
                    .scene
                    .insert_child(binding.contents, 1, children)?;
            }
            self.render_system
                .properties
                .remove_source(scroll_transform);
            Some(scroll_transform)
        } else {
            None
        };

        // Scrollbar overlays are also transient.
        let removed_overlay = if needs_overlay {
            if binding.overlay.is_none() {
                let overlay = self.render_system.scene.insert_group();
                self.render_system
                    .scene
                    .append_child(binding.contents, overlay)?;
                binding.overlay = Some(overlay);
            }
            None
        } else if let Some(overlay) = binding.overlay.take() {
            self.render_system.scene.detach(overlay)?;
            Some(overlay)
        } else {
            None
        };

        // Remove transient nodes only after the host binding stops referencing them.
        *self
            .render_system
            .host_binding_mut(id)
            .expect("binding disappeared") = binding;

        if let Some(scroll_transform) = removed_scroll_transform {
            self.render_system.scene.remove_subtree(scroll_transform)?;
        }
        if let Some(overlay) = removed_overlay {
            self.render_system.scene.remove_subtree(overlay)?;
        }

        // Scroll offsets remain dynamic frame properties while scrolling is active.
        if let Some(scroll_transform) = binding.scroll_transform {
            self.render_system
                .properties
                .set_transform(scroll_transform, Affine::translate(-scroll.x, -scroll.y));
        }

        // Reconcile host children in declaration/paint order.
        if let Some(children_binding) = binding.children {
            let children_match = self
                .render_system
                .scene
                .children(children_binding)?
                .iter()
                .copied()
                .eq(host_children.iter().map(|host_child| {
                    self.render_system
                        .host_binding(*host_child)
                        .expect("child host render binding missing")
                        .root
                }));

            if !children_match {
                let current = self
                    .render_system
                    .scene
                    .children(children_binding)?
                    .to_vec();

                for child_root in current {
                    self.render_system.scene.detach(child_root)?;
                }

                for host_child in &host_children {
                    let child_root = self
                        .render_system
                        .host_binding(*host_child)
                        .expect("child host render binding missing")
                        .root;

                    self.render_system.scene.detach(child_root)?;
                    self.render_system
                        .scene
                        .append_child(children_binding, child_root)?;
                }
            }
        }

        if let Some(overlay) = binding.overlay {
            let (target, effective) = self.style_system.styles(id).expect("style node missing");
            let node = NodeView::new(
                id,
                &self.hosts[id],
                self.layout_tree.host(id).expect("layout node missing"),
                target,
                effective,
            );
            let mut writer = RenderTreeWriter::new(&mut self.render_system.scene, overlay);
            render_scrollbars_in_rect(node, viewport, &mut writer);
            writer.finish()?;
        }

        self.sync_effective_transform(id);

        Ok(())
    }

    fn update_wrappers(
        &mut self,
        id: NodeId,
        binding: &mut HostRenderBinding,
        clip_shape: Option<ClipShape>,
        layer_descriptor: Option<LayerDescriptor>,
    ) -> Result<(), SceneError> {
        let topology_changed = clip_shape.is_some() != binding.clip.is_some()
            || layer_descriptor.is_some() != binding.layer.is_some();

        let old_clip = binding.clip;
        let old_layer = binding.layer;

        // Reuse existing wrappers and update their descriptors in place.
        let removed_clip = match clip_shape {
            Some(shape) => {
                if let Some(clip) = binding.clip {
                    self.render_system.scene.update_clip(clip, shape)?;
                } else {
                    binding.clip = Some(self.render_system.scene.insert_clip(shape));
                }
                None
            }
            None => binding.clip.take(),
        };

        let removed_layer = match layer_descriptor {
            Some(descriptor) => {
                if let Some(layer) = binding.layer {
                    self.render_system
                        .scene
                        .update_layer_descriptor(layer, descriptor)?;
                } else {
                    binding.layer = Some(self.render_system.scene.insert_layer(descriptor));
                }
                None
            }
            None => binding.layer.take(),
        };

        if !topology_changed {
            return Ok(());
        }

        // Break the old transform -> clip -> layer -> contents chain. The
        // layout root remains permanently attached to the style transform.
        self.render_system
            .scene
            .set_child(binding.transform, None)?;
        if let Some(clip) = old_clip {
            self.render_system.scene.set_child(clip, None)?;
        }
        if let Some(layer) = old_layer {
            self.render_system.scene.set_child(layer, None)?;
        }

        // Rebuild the wrapper chain from inside out.
        let mut child = binding.contents;
        if let Some(layer) = binding.layer {
            self.render_system.scene.set_child(layer, Some(child))?;
            child = layer;
        }
        if let Some(clip) = binding.clip {
            self.render_system.scene.set_child(clip, Some(child))?;
            child = clip;
        }
        self.render_system
            .scene
            .set_child(binding.transform, Some(child))?;

        // Drop binding references before removing obsolete wrapper subtrees.
        let stored = self
            .render_system
            .host_binding_mut(id)
            .expect("binding disappeared before wrapper removal");

        stored.clip = binding.clip;
        stored.layer = binding.layer;

        if let Some(clip) = removed_clip {
            self.render_system.scene.remove_subtree(clip)?;
        }

        if let Some(layer) = removed_layer {
            self.render_system.scene.remove_subtree(layer)?;
        }

        Ok(())
    }

    pub fn focused_node(&self) -> Option<NodeId> {
        self.interaction_system.focus.focused()
    }

    pub fn focus_manager(&self) -> &FocusManager {
        &self.interaction_system.focus
    }
    pub(crate) fn focus_manager_mut(&mut self) -> &mut FocusManager {
        &mut self.interaction_system.focus
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

    pub fn pointer_capture_node(&self) -> Option<NodeId> {
        self.interaction_system.event_state.pointer_capture()
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.style_system
                .set_default_style(ComputedStyle::initial(&self.theme));
            self.mark_work(
                self.root,
                HostWorkFlags::RECALC_STYLE
                    | HostWorkFlags::RECALC_STYLE_SUBTREE
                    | HostWorkFlags::RECALC_LAYOUT
                    | HostWorkFlags::REBUILD_PAINT,
            );
        }
    }

    pub(crate) fn event_state(&self) -> &EventState {
        &self.interaction_system.event_state
    }

    pub(crate) fn event_state_mut(&mut self) -> &mut EventState {
        &mut self.interaction_system.event_state
    }

    /// A read-only node projection paired with that node's own handlers.
    ///
    /// Handlers are shared (`Rc`) rather than uniquely owned boxes, so dispatch
    /// no longer needs `&mut` access to a global store while holding a node
    /// view — the two used to have to be handed out together.
    pub(crate) fn node_and_handlers(&self, id: NodeId) -> Option<(NodeView<'_>, &EventHandlers)> {
        let (target, effective) = self.style_system.styles(id)?;
        let node = NodeView::new(
            id,
            self.hosts.get(id)?,
            self.layout_tree.host(id)?,
            target,
            effective,
        );
        Some((node, &self.interaction_system.get(id)?.handlers))
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
        let Some(source) = self
            .pointer_capture_node()
            .filter(|node| self.contains(*node))
            .or_else(|| self.hovered_node())
        else {
            return CursorIcon::default();
        };

        let mut current = Some(source);
        while let Some(id) = current {
            if let Some((_, effective)) = self.style_system.styles(id)
                && let Some(cursor) = effective.cursor
            {
                return cursor;
            }
            current = self.hosts.parent(id);
        }
        CursorIcon::default()
    }

    pub fn hovered_node(&self) -> Option<NodeId> {
        self.interaction_system.event_state.hovered()
    }

    pub(crate) fn has_raw_event_listeners(&self) -> bool {
        self.raw_event_listeners > 0
    }

    pub(crate) fn node_reads_raw_events(&self, id: NodeId) -> bool {
        self.hosts.get(id).is_some_and(|host| host.reads_raw_events)
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
        let Some(node) = self.hosts.get(id) else {
            return false;
        };
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

    pub(crate) fn accessibility(&self, id: NodeId) -> Option<&AccessibilityProperties> {
        self.interaction_system
            .get(id)
            .map(|node| &node.properties.accessibility)
    }

    pub fn create_node(
        &mut self,
        key: Option<Key>,
        props_hash: u64,
        widget: WidgetI,
        interaction: Option<HostInteraction>,
    ) -> NodeId {
        let id = self
            .hosts
            .insert_with_key(|_| HostData::new(key, props_hash, widget));
        if self.hosts[id].reads_raw_events {
            self.raw_event_listeners += 1;
        }
        self.style_system
            .create(id, self.style_system.default_style().clone(), false);
        self.interaction_system.update(id, interaction);
        self.layout_tree.create_host(id, tf::Style::default());

        self.node_lifecycle_events
            .push(NodeLifecycleEvent::Created(id));
        self.create_host_render_binding(id)
            .expect("failed to create host render binding");
        self.refresh_taffy_context(id);
        let mut work = HostWorkFlags::RECALC_STYLE
            | HostWorkFlags::RECALC_LAYOUT
            | HostWorkFlags::REBUILD_PAINT;
        match self.hosts[id].node_type {
            WidgetType::Canvas => {
                self.canvas_nodes.insert(id, ());
                self.bind_canvas_controller(id);
                work |= HostWorkFlags::SHAPE_CHANGE;
            }
            WidgetType::Text => {
                self.text_nodes.insert(id, ());
            }
            _ => {}
        }
        self.mark_work(id, work);

        id
    }

    pub fn to_local(&self, node_id: NodeId, viewport_pos: Point) -> Option<Point> {
        let node = self.node(node_id)?;

        if let Some(parent_id) = self.hosts.parent(node_id) {
            let parent = self.node(parent_id)?;

            let parent_local = self.to_local(parent_id, viewport_pos)?;
            let parent_content_pos = parent_local + parent.scroll_offset;
            let node_pos = node.layout.origin();
            Some(parent_content_pos - node_pos)
        } else {
            let root_pos = node.layout.origin();
            Some(viewport_pos - root_pos)
        }
    }

    pub fn to_content_local(&self, node_id: NodeId, viewport_pos: Point) -> Option<Point> {
        let node = self.node(node_id)?;

        let local = self.to_local(node_id, viewport_pos)?;

        Some(local + node.scroll_offset)
    }

    pub fn attach(&mut self, parent: NodeId, child: NodeId) {
        self.append_child(parent, child);
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if parent == child || !self.hosts.contains_key(parent) || !self.hosts.contains_key(child) {
            return;
        }
        if child == self.root_overlayer && parent != self.root {
            return;
        }

        let old_parent = self.hosts.parent(child);
        let old_position = self.hosts.position(child).unwrap_or(0);

        if let Some(old_parent) = old_parent {
            self.hosts.detach(child);
            self.sync_taffy_children(old_parent);
            self.mark_work(
                old_parent,
                HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }

        if parent == self.root && child != self.root_overlayer {
            self.hosts.insert_before(parent, child, self.root_overlayer);
        } else {
            self.hosts.append_child(parent, child);
        }
        if old_parent != Some(parent) {
            self.mark_work(
                child,
                HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }
        self.sync_taffy_children(parent);
        let new_position = self.hosts.position(child).unwrap_or(0);
        self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
    }

    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) {
        if child == self.root_overlayer {
            return;
        }
        if child == before {
            return;
        }
        if parent == child || !self.hosts.contains_key(parent) || !self.hosts.contains_key(child) {
            return;
        }
        if !self.hosts.contains_key(before) || self.hosts.parent(before) != Some(parent) {
            self.append_child(parent, child);
            return;
        }

        let old_parent = self.hosts.parent(child);
        let old_position = self.hosts.position(child).unwrap_or(0);

        if let Some(old_parent) = old_parent {
            self.hosts.detach(child);
            self.sync_taffy_children(old_parent);
            self.mark_work(
                old_parent,
                HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }
        self.hosts.insert_before(parent, child, before);
        if old_parent != Some(parent) {
            self.mark_work(
                child,
                HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }
        self.sync_taffy_children(parent);
        let new_position = self.hosts.position(child).unwrap_or(0);
        self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
    }

    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if child == self.root_overlayer {
            return;
        }
        if !self.hosts.contains_key(parent) || !self.hosts.contains_key(child) {
            return;
        }
        if self.hosts.parent(child) != Some(parent) {
            return;
        }

        let old_position = self.hosts.position(child).unwrap_or(0);
        self.hosts.detach(child);
        self.mark_work(
            child,
            HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
        );
        self.sync_taffy_children(parent);
        self.record_node_move(child, Some(parent), None, old_position, 0);
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
    }

    pub fn remove_from_parent(&mut self, child: NodeId) {
        let Some(parent) = self.hosts.parent(child) else {
            return;
        };
        self.remove_child(parent, child);
    }

    pub fn clear_children(&mut self, parent: NodeId) {
        let children: Vec<_> = self.hosts.children(parent).collect();
        for child in children {
            if child != self.root_overlayer {
                self.remove_subtree(child);
            }
        }
        self.sync_taffy_children(parent);
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
    }

    pub fn remove_subtree(&mut self, id: NodeId) {
        if !self.hosts.contains_key(id) || id == self.root || id == self.root_overlayer {
            return;
        }

        let parent = self.hosts.parent(id);
        let removal: Vec<_> = self.hosts.subtree(id).collect();
        for removed in removal.into_iter().rev() {
            self.interaction_system.remove(removed);
            self.style_system.remove(removed);
            self.layout_tree.remove_host(removed);
            if let Some(binding) = self.render_system.unbind_host(removed) {
                self.render_system
                    .properties
                    .remove_source(binding.transform);
                self.render_system
                    .scene
                    .remove_subtree(binding.root)
                    .expect("failed to remove host render subtree");
            }
            self.text_nodes.remove(removed);
            if self.canvas_nodes.remove(removed).is_some() {
                self.unbind_canvas_controller(removed);
            }
            if self
                .hosts
                .get(removed)
                .is_some_and(|host| host.reads_raw_events)
            {
                self.raw_event_listeners -= 1;
            }
            self.hosts.remove(removed);
            self.node_lifecycle_events
                .push(NodeLifecycleEvent::Removed(removed));
        }
        if let Some(parent) = parent {
            self.sync_taffy_children(parent);
            self.mark_work(
                parent,
                HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }
    }

    pub fn drain_node_lifecycle_events(&mut self) -> Vec<NodeLifecycleEvent> {
        std::mem::take(&mut self.node_lifecycle_events)
    }

    pub fn mark_dirty(&mut self, id: NodeId, flags: WidgetUpdateFlags) {
        self.mark_work(id, HostWorkFlags::from_widget_update(flags));
    }

    pub(crate) fn set_widget_state_flag(&mut self, id: NodeId, flag: WidgetState, enabled: bool) {
        let Some(host) = self.hosts.get_mut(id) else {
            return;
        };
        let before = host.state;
        host.state.set(flag, enabled);
        if before != host.state {
            host.state_before_change.get_or_insert(before);
            self.mark_work(id, HostWorkFlags::SYNC_STATE_CHANGE);
        }
    }

    pub(crate) fn set_scroll_offset(&mut self, id: NodeId, offset: Point) -> bool {
        let Some(layout) = self.layout_tree.host_mut(id) else {
            return false;
        };
        if layout.scroll_offset == offset {
            return false;
        }
        layout.scroll_offset = offset;
        self.mark_work(id, HostWorkFlags::SYNC_RENDER);
        true
    }

    fn mark_work(&mut self, id: NodeId, flags: HostWorkFlags) {
        if flags.is_empty() || !self.hosts.contains_key(id) {
            return;
        }

        let newly_added = {
            let node = self.hosts.get_mut(id).expect("checked node existence");
            let newly_added = flags & !node.work;
            node.work |= flags;
            newly_added
        };

        if newly_added.is_empty() {
            return;
        }

        if newly_added.intersects(HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::SYNC_TREE) {
            // Host dirtiness alone is not enough: Taffy keeps intrinsic and
            // final layout entries per node. A resize must invalidate the
            // affected Taffy node too, otherwise a min-content text probe can
            // survive as the child's apparent final layout.
            let taffy_node = self.layout_tree.node_id(id);
            self.layout_tree
                .mark_dirty(taffy_node)
                .expect("failed to invalidate Taffy layout cache");

            self.ui_state.mark_layout_dirty(id);
        }

        if newly_added.intersects(HostWorkFlags::RECALC_STYLE_SUBTREE) {
            self.style_system.mark_subtree_dirty(id);
        }

        if newly_added.intersects(HostWorkFlags::RECALC_STYLE) {
            self.style_system.mark_dirty(id);
        }

        if newly_added.intersects(HostWorkFlags::SYNC_STATE_CHANGE) {
            self.ui_state.mark_state_change_dirty(id);
        }

        let is_canvas = self.hosts[id].node_type == WidgetType::Canvas;
        if newly_added.intersects(HostWorkFlags::SHAPE_CHANGE) {
            if is_canvas {
                // A canvas cannot be compiled before layout: a painter needs the
                // measured size, and its text boxes do not exist until it runs.
                self.ui_state.mark_canvas_dirty(id);
            } else {
                self.ui_state.mark_shape_dirty(id);
            }
        }

        // Painters read the resolved style, so a theme switch has to reach them.
        if is_canvas && newly_added.intersects(HostWorkFlags::RECALC_STYLE) {
            self.ui_state.mark_canvas_dirty(id);
        }

        let mut current = id;
        let mut remaining = newly_added;

        while let Some(parent) = self.hosts.parent(current) {
            let new_for_parent = remaining & !self.hosts[parent].subtree_work;
            self.hosts[parent].subtree_work |= remaining;
            if new_for_parent.is_empty() {
                break;
            }
            remaining = new_for_parent;
            current = parent;
        }
    }

    pub fn clear_work(&mut self, id: NodeId) {
        if let Some(node) = self.hosts.get_mut(id) {
            node.old_props_hash = node.new_props_hash;
            node.work = HostWorkFlags::empty();
            node.subtree_work = HostWorkFlags::empty();
        }
    }

    pub fn mark_subtree_layout_dirty(&mut self, id: NodeId) {
        self.mark_work(
            id,
            HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::REBUILD_PAINT,
        );
        let children: Vec<_> = self.hosts.children(id).collect();
        for child in children {
            self.mark_subtree_layout_dirty(child);
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
    pub fn visual_layout(&self, id: NodeId) -> Option<Bounds> {
        let layout = self.layout_tree.host(id)?;
        let mut scroll_offset = Point::zero();
        let mut cursor = self.hosts.parent(id);
        while let Some(parent) = cursor {
            let ancestor_layout = self.layout_tree.host(parent)?;
            if self
                .style_system
                .computed(parent)?
                .scroll
                .direction
                .is_scrollable()
            {
                scroll_offset = scroll_offset + ancestor_layout.scroll_offset;
            }
            cursor = self.hosts.parent(parent);
        }
        Some(layout.visual_bounds(scroll_offset))
    }

    fn hit_test_from(
        &self,
        id: NodeId,
        point: crate::core::Point,
        ancestor_scroll_offset: Point,
    ) -> HitTestOutcome {
        let Some(layout) = self.layout_tree.host(id) else {
            return HitTestOutcome::Miss;
        };
        let Some(node_style) = self.style_system.effective(id) else {
            return HitTestOutcome::Miss;
        };
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

        if self.hosts[id].node_type == WidgetType::RootOverlayer {
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

    fn overlay_entry_interaction(&self, visual_root: NodeId) -> Option<(bool, bool)> {
        let widget = self.hosts.get(self.root_overlayer)?.widget.clone();
        widget.with_widgets(|widgets| {
            let Widgets::RootOverlayer(overlayer) = widgets else {
                return None;
            };
            let entry = overlayer.entry_for_visual_root(visual_root)?;
            let entry = overlayer.entry(entry)?;
            Some((entry.hit_test(), entry.modal()))
        })
    }

    pub(crate) fn scroll_node_by(&mut self, start: NodeId, delta: Point) -> bool {
        let mut cursor = Some(start);
        while let Some(id) = cursor {
            if self.scroll_single_node_by(id, delta) {
                return true;
            }
            cursor = self.hosts.parent(id);
        }
        false
    }

    fn scroll_single_node_by(&mut self, id: NodeId, delta: Point) -> bool {
        if !self.hosts.contains_key(id) {
            return false;
        }
        let Some(layout) = self.layout_tree.host(id) else {
            return false;
        };

        let direction = self
            .style_system
            .computed(id)
            .expect("style node missing")
            .scroll
            .direction;
        if !direction.is_scrollable() {
            return false;
        }

        let max_x = if direction.allows_horizontal() {
            (layout.content_size.width - layout.layout.width()).max(0.0)
        } else {
            0.0
        };
        let max_y = if direction.allows_vertical() {
            (layout.content_size.height - layout.layout.height()).max(0.0)
        } else {
            0.0
        };
        if max_x <= 0.0 && max_y <= 0.0 {
            return false;
        }

        let scroll_delta = ergonomic_scroll_delta(delta);
        let next = Point::new(
            (layout.scroll_offset.x - scroll_delta.x).clamp(0.0, max_x),
            (layout.scroll_offset.y - scroll_delta.y).clamp(0.0, max_y),
        );
        if next == layout.scroll_offset {
            return false;
        }

        self.layout_tree
            .host_mut(id)
            .expect("checked layout existence")
            .scroll_offset = next;
        self.mark_work(id, HostWorkFlags::SYNC_RENDER);
        true
    }

    pub fn event_path(&self, target: NodeId) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut cursor = Some(target);
        while let Some(id) = cursor {
            path.push(id);
            cursor = self.hosts.parent(id);
        }
        path.reverse();
        path
    }

    #[inline(always)]
    pub fn dispatch_event<T: TextBackend>(
        &mut self,
        text: &TextHost<T>,
        translator: &mut EventTranslator,
        event: RawEvent,
    ) -> EventResult {
        event_system::dispatch_event(self, text, translator, event)
    }

    pub fn tick_style_animations(&mut self, delta: Duration) -> bool {
        if !self.style_system.is_animating() {
            return false;
        }
        let changed = self.style_system.tick(delta, &self.theme);
        let inherited_text_changed = changed
            .iter()
            .any(|(_, diff, _)| diff.intersects(xui_interface::StyleDiffFlags::TEXT));
        for (id, diff, requires_layout) in changed.iter().copied() {
            if diff.intersects(xui_interface::StyleDiffFlags::TRANSFORM) {
                self.sync_effective_transform(id);
            }
            let mut work = HostWorkFlags::from_style_diff(diff);
            // Do not enqueue a target-style subtree recompute every frame.
            // Sampled inherited values are propagated separately below.
            work.remove(HostWorkFlags::RECALC_STYLE_SUBTREE);
            if requires_layout {
                self.sync_effective_taffy_style(id);
                self.refresh_taffy_context(id);
                work |= HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::REBUILD_PAINT;
            } else {
                work.remove(HostWorkFlags::RECALC_LAYOUT);
            }
            self.mark_work(id, work);
        }
        if inherited_text_changed {
            self.sync_sampled_text_inheritance(self.root);
        }
        !changed.is_empty()
    }

    pub fn has_running_style_animations(&self) -> bool {
        self.style_system.is_animating()
    }

    pub fn update_tree<T: TextBackend>(&mut self, size: Size<f32>, measurer: &mut TextHost<T>) {
        self.update_visits = 0;
        self.apply_canvas_invalidations();
        for node_id in self.ui_state.drain_state_change_dirty_list() {
            self.recompute_node_state(node_id);
        }

        for node_id in self.ui_state.drain_shape_dirty_list() {
            self.recompute_node_text_shape(node_id, measurer);
        }

        // Fiber-style bailout: skip the whole branch when neither this node nor
        // any descendant has scheduled work.
        let style_dirty = self.style_system.drain_dirty();
        let mut inherited_text_changed = false;
        for node_id in style_dirty {
            inherited_text_changed |= self.recompute_node_style(node_id);
        }

        let subtree_dirty = self.style_system.drain_subtree_dirty();
        for node_id in subtree_dirty {
            inherited_text_changed |= self.recompute_subtree_styles(node_id);
        }

        // Sampled inheritance only has to be republished when a recomputed
        // style actually moved an inherited text field. Matches the gate
        // `tick_style_animations` already applies to animated samples.
        if inherited_text_changed {
            self.sync_sampled_text_inheritance(self.root);
        }

        // Recompute layout if needed.
        if self.has_layout_dirty() {
            self.compute_layout(size, measurer);
        }

        self.ui_state.layout_dirty_list.clear();
        // After layout, before paint: this is the only point where a canvas has
        // a final size and its text can still be shaped in time to be drawn.
        self.sync_dirty_canvases(measurer);
        self.rebuild_subtree_dirty(self.root);
        self.repaint_dirty_subtree(self.root);
        self.sync_render_scene()
            .expect("host tree must produce a valid render scene");
        self.clear_work_subtree(self.root, HostWorkFlags::all());
    }

    fn recompute_node_state(&mut self, id: NodeId) {
        if !self.hosts.contains_key(id) {
            return;
        }
        self.update_visits += 1;

        let state = self.hosts[id].state;
        let before = self.hosts[id].state_before_change.take().unwrap_or(state);
        let widget = self.hosts[id].widget.clone();

        // Recompute style if the widget's style affects the state change.
        if widget.with_widgets(|w| w.style().affects_state_change(before, state)) {
            self.mark_dirty(id, WidgetUpdateFlags::STYLE_TARGET);
        }
    }

    fn sync_sampled_text_inheritance(&mut self, id: NodeId) {
        let parent_effective = self.style_system.effective(id).cloned();
        let children: Vec<_> = self.hosts.children(id).collect();
        for child in children {
            let state = self.hosts[child].state;
            let patch = self.hosts[child]
                .widget
                .with_widgets(|widget| widget.style().patch_for_state(state));
            let (diff, requires_layout) = self.style_system.sync_inherited_text(
                child,
                parent_effective
                    .as_ref()
                    .expect("parent style missing during inheritance sync"),
                &patch,
            );
            if !diff.is_empty() {
                let mut work = HostWorkFlags::from_style_diff(diff);
                // Descendants are synchronized recursively below, so do not
                // enqueue a second target-style subtree traversal.
                work.remove(HostWorkFlags::RECALC_STYLE_SUBTREE);
                if requires_layout {
                    self.sync_effective_taffy_style(child);
                    self.refresh_taffy_context(child);
                    work |= HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::REBUILD_PAINT;
                }
                self.mark_work(child, work);
            }
            self.sync_sampled_text_inheritance(child);
        }
    }

    fn recompute_subtree_styles(&mut self, id: NodeId) -> bool {
        if !self.hosts.contains_key(id) {
            return false;
        }

        let mut inherited_text_changed = self.recompute_node_style(id);

        let mut cursor = self.hosts.link(id).and_then(|node| node.first_child);
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).and_then(|node| node.next_sibling);
            inherited_text_changed |= self.recompute_subtree_styles(child);
        }
        inherited_text_changed
    }

    /// Returns `true` when the node's effective style moved a text field that
    /// descendants may inherit, which is the only reason to republish sampled
    /// inheritance across the tree.
    fn recompute_node_style(&mut self, id: NodeId) -> bool {
        if !self.hosts.contains_key(id) {
            return false;
        }
        self.update_visits += 1;

        let widget = self.hosts[id].widget.clone();
        let state = self.hosts[id].state;
        let parent = self.hosts.parent(id);
        let parent_style = parent
            .and_then(|p| self.style_system.computed(p))
            .cloned()
            .unwrap_or_else(|| self.style_system.default_style().clone());

        let computed_style = computed_style_for_widget(&widget, &parent_style, &self.theme, state);

        let style_diff = self
            .style_system
            .computed(id)
            .expect("style node missing")
            .diff(&computed_style);
        let mut work_flags = HostWorkFlags::from_style_diff(style_diff);
        let transition = widget.transition();
        if !self.style_system.initialized(id) {
            self.style_system.set_computed(id, computed_style);
            self.style_system.set_initialized(id);
            self.sync_effective_transform(id);
            if self.sync_effective_taffy_style(id) {
                work_flags |= HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::REBUILD_PAINT;
            }
            self.mark_work(id, work_flags);
            self.refresh_taffy_context(id);
            return true;
        }

        let animatable_target_changed = {
            let current_target = self.style_system.computed(id).expect("style node missing");
            has_animatable_difference(current_target, &computed_style)
        };
        let effective_before = self
            .style_system
            .effective(id)
            .expect("style node missing")
            .clone();
        let mut cancelled_transition = false;
        let _started_transition = match (transition, animatable_target_changed) {
            (Some(transition), true) => {
                let from_style = self
                    .style_system
                    .effective(id)
                    .expect("style node missing")
                    .clone();
                self.style_system
                    .start_transition(id, transition, &from_style, &computed_style)
            }
            (Some(_), false) => {
                self.style_system
                    .sync_transition_target(id, &computed_style);
                false
            }
            (None, _) => {
                cancelled_transition = self.style_system.remove_transition(id);
                false
            }
        };

        if style_diff.is_empty() && !cancelled_transition {
            return false;
        }

        self.style_system.set_computed(id, computed_style);
        let effective_diff =
            effective_before.diff(self.style_system.effective(id).expect("style node missing"));
        work_flags |= HostWorkFlags::from_style_diff(effective_diff);

        if effective_diff.intersects(xui_interface::StyleDiffFlags::TRANSFORM) {
            self.sync_effective_transform(id);
        }

        if self.sync_effective_taffy_style(id) {
            work_flags |= HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::REBUILD_PAINT;
        }

        self.refresh_taffy_context(id);

        if work_flags.intersects(HostWorkFlags::RECALC_STYLE_SUBTREE) {
            self.style_system.mark_subtree_dirty(id);
        }

        self.mark_work(id, work_flags & !HostWorkFlags::RECALC_STYLE_SUBTREE);
        effective_diff.intersects(xui_interface::StyleDiffFlags::TEXT)
    }

    fn sync_effective_taffy_style(&mut self, id: NodeId) -> bool {
        if !self.hosts.contains_key(id) {
            return false;
        }
        let parent = self.hosts.parent(id);
        // How the parent arranges its children — not just which way it points.
        // `Sizing::Fill` resolves differently under flex, grid, and block, and
        // reading only `flex_direction` could not tell them apart.
        let parent_layout = parent
            .map(|parent| container_layout(&self.hosts[parent].widget))
            .unwrap_or(ParentLayout::Block);
        let effective = self
            .style_system
            .effective(id)
            .expect("style node missing")
            .clone();
        let widget = self.hosts[id].widget.clone();
        let taffy_style = taffy_style_for_widget(&widget, parent_layout, &effective);
        let taffy_node = self.layout_tree.node_id(id);
        let current = self
            .layout_tree
            .style(taffy_node)
            .expect("layout style missing");
        if *current == taffy_style {
            return false;
        }
        self.layout_tree
            .set_style(taffy_node, taffy_style)
            .expect("failed to update animated layout style");
        true
    }

    pub fn compute_layout_if_needed<T: TextBackend>(
        &mut self,
        size: Size<f32>,
        measurer: &mut TextHost<T>,
    ) {
        if !self.has_layout_dirty() {
            return;
        }
        self.compute_layout(size, measurer);
    }

    #[inline(always)]
    fn has_layout_dirty(&self) -> bool {
        !self.ui_state.layout_dirty_list.is_empty()
    }

    #[inline(always)]
    fn layout_work_flags() -> HostWorkFlags {
        HostWorkFlags::RECALC_LAYOUT
    }

    #[inline(always)]
    fn paint_work_flags() -> HostWorkFlags {
        HostWorkFlags::REBUILD_PAINT
    }

    fn effective_style(&self, id: NodeId) -> Option<&ComputedStyle> {
        self.style_system.effective(id)
    }

    fn sync_effective_transform(&mut self, id: NodeId) -> bool {
        let Some(binding) = self.render_system.host_binding(id).copied() else {
            return false;
        };
        let Some(style) = self.style_system.effective(id).map(|style| style.transform) else {
            return false;
        };
        if style == xui_interface::TransformStyle::IDENTITY {
            return self
                .render_system
                .properties
                .clear_transform(binding.transform);
        }
        let size = self
            .layout_tree
            .host(id)
            .expect("transform host is missing layout")
            .layout
            .size();
        self.render_system
            .properties
            .set_transform(binding.transform, style.to_affine(size))
    }

    pub fn repaint_if_needed(&mut self, id: NodeId) {
        let should_repaint = self
            .hosts
            .get(id)
            .is_some_and(|node| node.work.intersects(HostWorkFlags::REBUILD_PAINT));
        if !should_repaint {
            return;
        }

        self.repaint_passes += 1;
        let layout = self
            .layout_tree
            .host(id)
            .expect("layout node missing")
            .layout;
        let rect = Bounds::from_zero_size(layout.size());
        let style = self
            .effective_style(id)
            .expect("checked node existence before repaint")
            .clone();
        let widget = self.hosts[id].widget.clone();
        let paint = self
            .render_system
            .host_binding(id)
            .expect("host render binding missing")
            .paint;
        let mut writer = RenderTreeWriter::new(&mut self.render_system.scene, paint);
        widget.render(id, rect, &style, &mut writer);
        writer
            .finish()
            .expect("widget emitted an invalid retained render tree");
    }

    pub fn compute_layout<T: TextBackend>(&mut self, size: Size<f32>, measurer: &mut TextHost<T>) {
        self.layout_passes += 1;
        let root_taffy = self.layout_tree.node_id(self.root);
        self.layout_tree
            .compute_layout_with_measure(
                root_taffy,
                tf::Size {
                    width: tf::AvailableSpace::Definite(size.width),
                    height: tf::AvailableSpace::Definite(size.height),
                },
                |known_dimensions, available_space, _node_id, node_context, _style| {
                    measure_layout_context(
                        &self.hosts,
                        &self.style_system,
                        known_dimensions,
                        available_space,
                        node_context,
                        measurer,
                    )
                },
            )
            .expect("failed to compute layout");
        self.sync_layout(self.root, 0.0, 0.0);
        self.activate_final_text_layouts(measurer);
    }

    /// Activates the paint layout only after Taffy has committed every node's
    /// final width. Intrinsic measurement probes are deliberately inactive.
    fn activate_final_text_layouts<T: TextBackend>(&self, measurer: &mut TextHost<T>) {
        let font_context = measurer.backend().epoch();
        let requests: Vec<_> = self
            .text_nodes
            .keys()
            .filter_map(|id| {
                let node = self.hosts.get(id)?;
                let props = node.widget.with_widgets(|widget| {
                    widget.text_layout_props(
                        self.style_system.effective(id).expect("style node missing"),
                    )
                })?;
                let layout = self
                    .layout_tree
                    .host(id)
                    .expect("layout node missing")
                    .layout;
                let input = TextLayoutInput::new(
                    props.text,
                    TextLayoutConstraints::max_width(layout.width().max(0.0)),
                    props.style.into(),
                    props.paragraph,
                    props.text_box,
                    font_context,
                );
                Some((id, input))
            })
            .collect();

        for (id, input) in requests {
            measurer.activate_slot(id, TextLayoutSlot::PRIMARY, input);
        }
    }

    fn sync_layout(&mut self, id: NodeId, offset_x: f32, offset_y: f32) -> HostWorkFlags {
        let taffy_node = self.layout_tree.node_id(id);
        // Pixel-rounded widths can be smaller than the shaped intrinsic width.
        // For CJK text that turns the final character into a second line on
        // alternating resize frames. Preserve Taffy's computed floating-point
        // geometry for text while keeping pixel snapping for other widgets.
        let layout = if self.node_uses_unrounded_layout(id) {
            self.layout_tree.unrounded_layout(taffy_node)
        } else {
            self.layout_tree
                .layout(taffy_node)
                .expect("missing taffy layout result")
        };
        let taffy_content_size =
            Size::<f32>::new(layout.content_size.width, layout.content_size.height);

        let origin = Point::new(layout.location.x, layout.location.y);
        let size = Size::new(layout.size.width, layout.size.height);
        let rect = Bounds::from_origin_size(origin, size);
        let world_origin = Point::new(offset_x + layout.location.x, offset_y + layout.location.y);

        let previous = self.layout_tree.host(id).expect("layout node missing");
        let old_rect = previous.layout;
        let layout_changed = old_rect != rect || previous.world_origin != world_origin;
        let size_changed = old_rect.width() != rect.width() || old_rect.height() != rect.height();
        if size_changed && self.hosts[id].node_type == WidgetType::Canvas {
            self.ui_state.mark_canvas_dirty(id);
        }
        let mut subtree_work = {
            let node = &mut self.hosts[id];
            let should_sync_children = layout_changed
                || node.work.intersects(Self::layout_work_flags())
                || node.subtree_work.intersects(Self::layout_work_flags());

            let layout_node = self.layout_tree.host_mut(id).expect("layout node missing");
            layout_node.previous_layout = layout_node.layout;
            layout_node.layout = rect;
            layout_node.world_origin = world_origin;
            node.work.remove(HostWorkFlags::RECALC_LAYOUT);
            if layout_changed {
                node.work.insert(HostWorkFlags::SYNC_RENDER);
            }
            if size_changed {
                node.work.insert(HostWorkFlags::REBUILD_PAINT);
            }

            if should_sync_children {
                HostWorkFlags::empty()
            } else {
                return node.work | node.subtree_work;
            }
        };

        let mut cursor = self.hosts.link(id).and_then(|node| node.first_child);
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).and_then(|node| node.next_sibling);
            subtree_work |= self.sync_layout(child, world_origin.x, world_origin.y);
        }

        let content_size = self.content_size_from_children(id, taffy_content_size);
        let scroll_dirty = {
            let scroll = self
                .style_system
                .computed(id)
                .expect("style node missing")
                .scroll;
            let direction = scroll.direction;
            let layout = self.layout_tree.host_mut(id).expect("layout node missing");
            let content_size_changed = layout.content_size != content_size;
            let scroll_offset_before_clamp = layout.scroll_offset;
            layout.content_size = content_size;
            clamp_scroll_offset(layout, scroll);
            direction.is_scrollable()
                && (content_size_changed || layout.scroll_offset != scroll_offset_before_clamp)
        };
        if scroll_dirty {
            let node = self.hosts.get_mut(id).expect("node removed during layout");
            node.work.insert(HostWorkFlags::SYNC_RENDER);
        }
        let node = self.hosts.get_mut(id).expect("node removed during layout");
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }

    fn content_size_from_children(&self, id: NodeId, taffy_content_size: Size<f32>) -> Size<f32> {
        let node = self.layout_tree.host(id).expect("layout node missing");
        let mut width = taffy_content_size.width.max(node.layout.width());
        let mut height = taffy_content_size.height.max(node.layout.height());

        for child_id in self.hosts.children(id) {
            let child = self
                .layout_tree
                .host(child_id)
                .expect("layout node missing");
            width = width.max(child.layout.x() + child.layout.width() - node.layout.x());
            height = height.max(child.layout.y() + child.layout.height() - node.layout.y());
        }

        Size::<f32>::new(width, height)
    }

    fn node_uses_unrounded_layout(&self, id: NodeId) -> bool {
        self.hosts
            .get(id)
            .map(|node| {
                matches!(
                    node.widget.node_type(),
                    WidgetType::Text | WidgetType::TextInput
                )
            })
            .unwrap_or(false)
    }

    fn repaint_dirty_subtree(&mut self, id: NodeId) {
        let work = self.hosts[id].work;
        let subtree_work = self.hosts[id].subtree_work;
        if !work.intersects(Self::paint_work_flags())
            && !subtree_work.intersects(Self::paint_work_flags())
        {
            return;
        }

        if work.intersects(Self::paint_work_flags()) {
            self.repaint_if_needed(id);
        }

        let mut cursor = self.hosts.link(id).and_then(|node| node.first_child);
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).and_then(|node| node.next_sibling);
            let child_work = self.hosts[child].work | self.hosts[child].subtree_work;
            if !child_work.intersects(Self::paint_work_flags()) {
                continue;
            }
            self.repaint_dirty_subtree(child);
        }
    }

    fn clear_work_subtree(&mut self, id: NodeId, flags: HostWorkFlags) -> HostWorkFlags {
        if !self.hosts.contains_key(id) {
            return HostWorkFlags::empty();
        }

        let current = self.hosts[id].work | self.hosts[id].subtree_work;
        if !current.intersects(flags) {
            return current;
        }
        let mut subtree_work = HostWorkFlags::empty();
        let mut cursor = self.hosts.link(id).and_then(|node| node.first_child);
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).and_then(|node| node.next_sibling);
            // A child with nothing to clear still contributes its residual
            // work to this node's subtree summary, exactly as the recursive
            // early return used to.
            let child_work = self.hosts[child].work | self.hosts[child].subtree_work;
            if !child_work.intersects(flags) {
                subtree_work |= child_work;
                continue;
            }
            subtree_work |= self.clear_work_subtree(child, flags);
        }

        let node = self.hosts.get_mut(id).expect("checked node existence");
        node.old_props_hash = node.new_props_hash;
        node.work.remove(flags);
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }

    fn rebuild_subtree_dirty(&mut self, id: NodeId) -> HostWorkFlags {
        if !self.hosts.contains_key(id) {
            return HostWorkFlags::empty();
        }

        if self.hosts[id].work.is_empty() && self.hosts[id].subtree_work.is_empty() {
            return HostWorkFlags::empty();
        }
        let mut subtree_work = HostWorkFlags::empty();
        let mut cursor = self.hosts.link(id).and_then(|node| node.first_child);
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).and_then(|node| node.next_sibling);
            if self.hosts[child].work.is_empty() && self.hosts[child].subtree_work.is_empty() {
                continue;
            }
            subtree_work |= self.rebuild_subtree_dirty(child);
        }

        let node = self.hosts.get_mut(id).expect("checked node existence");
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }

    pub fn build_render_frame(&mut self) -> Result<Option<RenderFrame>, RenderFrameError> {
        let dirty_snapshot = self.render_system.scene.dirty_snapshot();
        let properties_snapshot = self.render_system.properties.snapshot();
        let root = self
            .layout_tree
            .host(self.root)
            .expect("root layout node missing")
            .layout;
        let viewport = Bounds::from_zero_size(root.size());
        let viewport_changed = self.render_system.last_viewport != Some(viewport);
        let needs_scene_compile = self.render_system.compiler.compiled_scene().is_none()
            || !dirty_snapshot.nodes.is_empty();
        if !needs_scene_compile && !self.render_system.properties.is_dirty() && !viewport_changed {
            return Ok(None);
        }
        if needs_scene_compile {
            self.render_system
                .compiler
                .compile(&self.render_system.scene, &dirty_snapshot)?;
        }
        let compiled = self
            .render_system
            .compiler
            .compiled_scene()
            .expect("scene compiler is initialized before frame building");
        let built =
            self.render_system
                .builder
                .build(compiled, viewport, &self.render_system.properties)?;
        Ok(Some(RenderFrame {
            built,
            dirty_snapshot,
            properties_snapshot,
            viewport,
        }))
    }

    pub fn finish_render_frame(&mut self, frame: &RenderFrame) {
        self.clear_work_subtree(self.root, HostWorkFlags::REBUILD_PAINT);
        self.render_system.scene.acknowledge(&frame.dirty_snapshot);
        self.render_system
            .properties
            .acknowledge(frame.properties_snapshot);
        self.render_system.last_viewport = Some(frame.viewport);
    }

    pub fn is_dirty(&self) -> bool {
        !self.canvas_invalidations.is_empty()
            || !self.ui_state.canvas_dirty_list.is_empty()
            || self.render_system.scene.is_dirty()
            || self.render_system.properties.is_dirty()
            || self.has_running_style_animations()
            || self.style_system.has_dirty()
            || !self.ui_state.layout_dirty_list.is_empty()
            || !self.hosts[self.root].work.is_empty()
            || !self.hosts[self.root].subtree_work.is_empty()
    }

    pub fn update_widget_node_from_parts(
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
            let node = self.hosts.get_mut(id).expect("reused node missing");

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
        self.mark_dirty(id, flags);
        current_widget
    }

    /// Re-runs a canvas's drawing on the next frame.
    pub fn invalidate_canvas(&mut self, id: NodeId) {
        if self.canvas_nodes.contains_key(id) {
            self.ui_state.mark_canvas_dirty(id);
            self.mark_work(id, HostWorkFlags::REBUILD_PAINT);
        }
    }

    fn sync_taffy_children(&mut self, parent: NodeId) {
        let parent_taffy = self.layout_tree.node_id(parent);
        let taffy_children: Vec<_> = self
            .hosts
            .children(parent)
            .map(|id| self.layout_tree.node_id(id))
            .collect();
        self.layout_tree
            .set_children(parent_taffy, &taffy_children)
            .expect("failed to sync taffy children");
    }

    pub fn set_children(&mut self, parent: NodeId, children: Vec<NodeId>) {
        if !self.hosts.contains_key(parent) {
            return;
        }

        let mut unique = Vec::with_capacity(children.len());
        for child in children {
            if child != parent
                && child != self.root_overlayer
                && self.hosts.contains_key(child)
                && !unique.contains(&child)
            {
                unique.push(child);
            }
        }
        if parent == self.root {
            unique.retain(|child| *child != self.root_overlayer);
            unique.push(self.root_overlayer);
        }
        let old_children: Vec<_> = self.hosts.children(parent).collect();
        if old_children == unique {
            return;
        }

        let old_locations: Vec<_> = unique
            .iter()
            .map(|&child| {
                (
                    child,
                    self.hosts.parent(child),
                    self.hosts.position(child).unwrap_or(0),
                )
            })
            .collect();
        let moved_from: Vec<_> = old_locations
            .iter()
            .filter_map(|(_, old_parent, _)| old_parent.filter(|old| *old != parent))
            .collect();

        self.hosts.set_children(parent, &unique);
        for (old_position, child) in old_children.iter().copied().enumerate() {
            if !unique.contains(&child) {
                self.mark_work(
                    child,
                    HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
                );
                self.record_node_move(child, Some(parent), None, old_position, 0);
            }
        }
        for (new_position, (child, old_parent, old_position)) in
            old_locations.into_iter().enumerate()
        {
            if old_parent != Some(parent) {
                self.mark_work(
                    child,
                    HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
                );
            }
            self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
        }
        for old_parent in moved_from {
            self.sync_taffy_children(old_parent);
            self.mark_work(
                old_parent,
                HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }
        self.sync_taffy_children(parent);
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
    }

    fn record_node_move(
        &mut self,
        id: NodeId,
        old_parent: Option<NodeId>,
        new_parent: Option<NodeId>,
        old_position: usize,
        new_position: usize,
    ) {
        if old_parent == new_parent && old_position == new_position {
            return;
        }
        if old_parent.is_none() && new_parent.is_some() {
            return;
        }
        self.node_lifecycle_events.push(NodeLifecycleEvent::Moved {
            id,
            old_parent,
            new_parent,
            old_position,
            new_position,
        });
    }

    fn refresh_taffy_context(&mut self, id: NodeId) {
        let node = &self.hosts[id];

        let context = match node.node_type {
            WidgetType::Text | WidgetType::TextInput => Some(WidgetContext::Text(id)),
            WidgetType::Image => node.widget.intrinsic_size().map(WidgetContext::Image),
            _ => None,
        };

        let taffy_node = self.layout_tree.node_id(id);
        self.layout_tree
            .set_node_context(taffy_node, context)
            .expect("failed to update taffy context");
    }
    /// Physical pixels per logical pixel, for canvas painters.
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        if self.scale_factor == scale_factor {
            return;
        }
        self.scale_factor = scale_factor;
        // Hairline widths and pixel snapping are both derived from it, so every
        // drawing that used the old factor is now off by a subpixel.
        let canvases: Vec<_> = self.canvas_nodes.keys().collect();
        for id in canvases {
            self.ui_state.mark_canvas_dirty(id);
        }
    }

    pub(crate) fn canvas_invalidator(&self) -> crate::widgets::CanvasInvalidator {
        self.canvas_invalidations.clone()
    }

    /// Turns repaints requested by a controller into host work.
    ///
    /// This is the whole of the direct channel: a `CanvasController` names the
    /// nodes drawing it and the flags they need, and the frame that follows
    /// picks them up without any component ever rebuilding.
    fn apply_canvas_invalidations(&mut self) {
        if self.canvas_invalidations.is_empty() {
            return;
        }
        for (id, flags) in self.canvas_invalidations.drain() {
            if self.hosts.contains_key(id) {
                self.mark_dirty(id, flags);
            }
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
        let Some(node) = self.hosts.get(id) else {
            return;
        };
        node.widget.with_widgets_mut(|widget| {
            if let Widgets::Canvas(canvas) = widget {
                canvas.unbind();
            }
        });
    }

    /// Rebuilds every canvas whose drawing no longer matches its node.
    ///
    /// Runs after layout and before paint, the same window
    /// `activate_final_text_layouts` uses: a painter needs the size Taffy just
    /// committed, and the text boxes it produces have to reach the shaper
    /// before the node is repainted.
    fn sync_dirty_canvases<T: TextBackend>(&mut self, measurer: &mut TextHost<T>) {
        let dirty = self.ui_state.drain_canvas_dirty_list();
        if dirty.is_empty() {
            return;
        }
        let mut compiled = Vec::with_capacity(dirty.len());
        for id in dirty {
            if !self.canvas_nodes.contains_key(id) || compiled.contains(&id) {
                continue;
            }
            compiled.push(id);
            self.compile_canvas(id, measurer);
        }
    }

    fn compile_canvas<T: TextBackend>(&mut self, id: NodeId, measurer: &mut TextHost<T>) {
        let size = self
            .layout_tree
            .host(id)
            .expect("canvas node missing layout")
            .layout
            .size();
        let style = self
            .style_system
            .effective(id)
            .expect("canvas node missing style")
            .clone();
        let theme = self.theme.clone();
        let scale_factor = self.scale_factor;
        let widget = self.hosts[id].widget.clone();

        let text_boxes = widget.with_widgets_mut(|node| match node {
            Widgets::Canvas(canvas) => {
                canvas.compile(size, &style, &theme, scale_factor);
                canvas.text_boxes()
            }
            _ => Vec::new(),
        });

        measurer.retain_direct_slots(
            id,
            text_boxes
                .iter()
                .map(|(text_id, _, _)| canvas_text_slot(*text_id)),
        );
        let font_context = measurer.backend().epoch();
        for (text_id, bounds, props) in text_boxes {
            let input = TextLayoutInput::new(
                props.text,
                TextLayoutConstraints::max_width(bounds.width().max(0.0)),
                props.style.into(),
                props.paragraph,
                props.text_box,
                font_context,
            );
            measurer.get_or_shape_slot(id, canvas_text_slot(text_id), input);
        }

        self.mark_work(id, HostWorkFlags::REBUILD_PAINT);
    }

    fn recompute_node_text_shape<T: TextBackend>(
        &mut self,
        node_id: NodeId,
        measurer: &mut TextHost<T>,
    ) {
        self.update_visits += 1;
        let node = self.hosts.get(node_id).unwrap();
        match node.node_type {
            WidgetType::TextInput => {
                let style = self
                    .style_system
                    .effective(node_id)
                    .expect("style node missing");
                let props = node
                    .widget
                    .with_widgets(|w| w.text_layout_props(style))
                    .expect("C");

                let constraints = TextLayoutConstraints::default();
                let font_context = measurer.backend().epoch();

                let input = TextLayoutInput::new(
                    props.text,
                    constraints,
                    props.style.into(),
                    props.paragraph,
                    props.text_box,
                    font_context,
                );
                measurer.get_or_shape_slot(node_id, TextLayoutSlot::PRIMARY, input);
            }
            _ => {}
        }
    }
}

impl Default for UiRuntime {
    fn default() -> Self {
        Self::new()
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

fn layer_descriptor_from_style(style: &ComputedStyle, bounds: Bounds) -> Option<LayerDescriptor> {
    let descriptor = LayerDescriptor {
        bounds: Some(bounds),
        backdrop_style: style.effect.backdrop.clone(),
        effects: style.effect.effects.clone(),
        ..Default::default()
    };

    descriptor.requires_isolation().then_some(descriptor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::StyleProps;
    use crate::event_system::callbacks::EventProps;
    use crate::event_system::translator::EventTranslator;
    use crate::event_system::{Flow, Handler};
    use crate::focus::FocusHandle;
    use crate::render::RenderNodeKind;
    use crate::text::testing::ZeroTextBackend;
    use crate::widgets::{
        CanvasController, TextWidget, WidgetI, canvas, container, text_input, z_stack,
    };
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::{Duration, Instant};
    use xui_animation::{Easing, Transition};
    use xui_interface::core::Sizing;
    use xui_interface::events::semantic::ClickEvent;
    use xui_interface::events::{
        Modifiers, PointerButton, PointerButtons, PointerKind, RawPointerButton, RawPointerMove,
        XuiPointerId,
    };
    use xui_interface::style::FlexDirectionStyle;
    use xui_interface::{
        Affine, CanvasTextId, Color, ComputedColorStyle, FontDatabase, PathBuilder, PathFill,
        Style, TextProps, VectorSceneBuilder, WidgetState,
    };

    fn create_host(arena: &mut UiRuntime, widget: WidgetI) -> NodeId {
        let parent = arena.root();
        let key = widget.key();
        let props_hash = widget.props_hash();
        let interaction = widget.take_host_interaction();
        let id = arena.create_node(key, props_hash, widget, interaction);
        arena.append_child(parent, id);
        id
    }

    fn canvas_generation(arena: &UiRuntime, id: NodeId) -> u64 {
        arena.hosts[id].widget.with_widgets(|widget| match widget {
            Widgets::Canvas(canvas) => canvas.compiled_generation(),
            _ => panic!("not a canvas node"),
        })
    }

    fn canvas_compiled_size(arena: &UiRuntime, id: NodeId) -> Size<f32> {
        arena.hosts[id].widget.with_widgets(|widget| match widget {
            Widgets::Canvas(canvas) => canvas.compiled_size(),
            _ => panic!("not a canvas node"),
        })
    }

    #[test]
    fn a_controller_edit_repaints_its_canvas_with_no_component_rebuild() {
        let mut arena = UiRuntime::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        let controller = CanvasController::new();
        let id = create_host(
            &mut arena,
            WidgetI::new(
                canvas()
                    .controller(controller.clone())
                    .style(Style::new().width(80.0).height(40.0)),
            ),
        );

        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
        let before = canvas_generation(&arena, id);
        assert!(before > 0, "a mounted canvas is compiled before it paints");
        assert!(arena.canvas_invalidations.is_empty());

        let mut path = PathBuilder::new();
        path.move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(10.0, 10.0));
        let mut scene = VectorSceneBuilder::new();
        scene.fill_path(path.build(), Affine::IDENTITY, PathFill::new(Color::BLACK));
        controller.set_scene(scene.build());

        assert!(
            !arena.canvas_invalidations.is_empty(),
            "the edit must reach the host directly"
        );
        assert!(
            arena.is_dirty(),
            "and it must be enough on its own to schedule a frame"
        );

        let visits_before = arena.update_visits;
        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
        assert!(canvas_generation(&arena, id) > before);
        assert!(arena.canvas_invalidations.is_empty());
        let _ = visits_before;
    }

    #[test]
    fn a_painter_is_re_run_against_the_size_layout_committed() {
        let sizes = Rc::new(RefCell::new(Vec::new()));
        let recorder = sizes.clone();
        let controller = CanvasController::with_painter(move |painter| {
            recorder.borrow_mut().push(painter.size());
            let width = painter.width();
            painter.rect(
                Bounds::from_origin_size((0.0, 0.0), (width, 4.0)),
                Color::BLACK,
            );
        });

        let mut arena = UiRuntime::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        let id = create_host(
            &mut arena,
            WidgetI::new(
                canvas()
                    .controller(controller.clone())
                    .style(Style::new().size(Size::fill())),
            ),
        );

        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
        assert_eq!(canvas_compiled_size(&arena, id), Size::new(400.0, 200.0));
        assert_eq!(controller.size(), Size::new(400.0, 200.0));

        arena.mark_subtree_layout_dirty(arena.root());
        arena.update_tree(Size::new(640.0, 200.0), &mut measurer);
        assert_eq!(canvas_compiled_size(&arena, id), Size::new(640.0, 200.0));
        assert_eq!(
            sizes.borrow().last().copied(),
            Some(Size::new(640.0, 200.0)),
            "the painter has to see the measured size, not the one it was authored for"
        );
    }

    #[test]
    fn a_painter_that_draws_nothing_new_is_not_re_run_when_only_the_viewport_moves() {
        let runs = Rc::new(Cell::new(0));
        let counter = runs.clone();
        let controller = CanvasController::with_painter(move |_| {
            counter.set(counter.get() + 1);
        });

        let mut arena = UiRuntime::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        create_host(
            &mut arena,
            WidgetI::new(
                canvas()
                    .controller(controller)
                    .style(Style::new().width(80.0).height(40.0)),
            ),
        );

        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
        let after_mount = runs.get();

        arena.mark_subtree_layout_dirty(arena.root());
        arena.update_tree(Size::new(640.0, 200.0), &mut measurer);
        assert_eq!(
            runs.get(),
            after_mount,
            "a fixed-size canvas keeps its drawing when the window resizes around it"
        );
    }

    #[test]
    fn text_a_painter_produces_is_shaped_before_the_canvas_paints() {
        let controller = CanvasController::with_painter(|painter| {
            let width = painter.width();
            painter.text(
                Bounds::from_origin_size((0.0, 0.0), (width, 20.0)),
                TextProps::new("measured after layout"),
            );
        });

        let mut arena = UiRuntime::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        let id = create_host(
            &mut arena,
            WidgetI::new(
                canvas()
                    .controller(controller)
                    .style(Style::new().size(Size::fill())),
            ),
        );

        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
        assert!(
            measurer
                .active_slot(id, canvas_text_slot(CanvasTextId::new(1)))
                .is_some(),
            "a painter's text box has to reach the shaper in the same frame it is drawn"
        );
    }

    #[test]
    fn removing_a_canvas_stops_its_controller_from_marking_the_dead_node() {
        let mut arena = UiRuntime::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        let controller = CanvasController::new();
        let id = create_host(
            &mut arena,
            WidgetI::new(
                canvas()
                    .controller(controller.clone())
                    .style(Style::new().width(80.0).height(40.0)),
            ),
        );
        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

        arena.remove_subtree(id);
        controller.invalidate();
        assert!(
            arena.canvas_invalidations.is_empty(),
            "an unmounted canvas must not keep a queue entry alive"
        );
    }

    #[test]
    fn runtime_keeps_one_root_overlayer_last_without_blocking_content_hits() {
        let mut arena = UiRuntime::new();
        let content = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().size(Size::fill()))),
        );
        let overlayer = arena.root_overlayer();
        let mut measurer = TextHost::new(ZeroTextBackend);

        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

        assert_eq!(
            arena
                .children(arena.root())
                .filter(|id| arena.node(*id).unwrap().node_type == WidgetType::RootOverlayer)
                .count(),
            1
        );
        assert_eq!(arena.children(arena.root()).last(), Some(overlayer));
        assert_eq!(
            arena.node(overlayer).unwrap().layout.size(),
            Size::new(400.0, 200.0)
        );
        assert_eq!(arena.hit_test(Point::new(20.0, 20.0)), Some(content));

        arena.remove_subtree(overlayer);
        arena.clear_children(arena.root());
        assert!(arena.contains(overlayer));
        assert_eq!(
            arena.children(arena.root()).collect::<Vec<_>>(),
            vec![overlayer]
        );
    }

    #[test]
    fn hit_test_tracks_scrolled_content_without_moving_the_scroll_viewport() {
        let mut arena = UiRuntime::new();
        let scroll = create_host(
            &mut arena,
            WidgetI::new(
                container().style(Style::new().width(100.0).height(100.0).scroll_vertical()),
            ),
        );
        let content = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(20.0).height(200.0))),
        );
        let target = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .absolute()
                        .inset(xui_interface::EdgeInsets::new(0.0, 0.0, 80.0, 0.0))
                        .width(20.0)
                        .height(20.0),
                ),
            ),
        );
        arena.append_child(scroll, content);
        arena.append_child(content, target);

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
        assert!(arena.set_scroll_offset(scroll, Point::new(0.0, 60.0)));

        assert_eq!(arena.visual_layout(target).unwrap().y(), 20.0);
        assert_eq!(arena.node(target).unwrap().world_origin.y, 20.0);
        assert_eq!(arena.hit_test(Point::new(10.0, 25.0)), Some(target));
        assert_ne!(arena.hit_test(Point::new(10.0, 85.0)), Some(target));
        assert_eq!(arena.hit_test(Point::new(50.0, 90.0)), Some(scroll));
    }

    #[test]
    fn hit_test_accumulates_nested_scroll_offsets() {
        let mut arena = UiRuntime::new();
        let outer = create_host(
            &mut arena,
            WidgetI::new(
                container().style(Style::new().width(120.0).height(100.0).scroll_vertical()),
            ),
        );
        let inner = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .absolute()
                        .inset(xui_interface::EdgeInsets::new(0.0, 0.0, 40.0, 0.0))
                        .width(100.0)
                        .height(80.0)
                        .scroll_vertical(),
                ),
            ),
        );
        let target = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .absolute()
                        .inset(xui_interface::EdgeInsets::new(0.0, 0.0, 50.0, 0.0))
                        .width(20.0)
                        .height(20.0),
                ),
            ),
        );
        arena.append_child(outer, inner);
        arena.append_child(inner, target);

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
        assert!(arena.set_scroll_offset(outer, Point::new(0.0, 20.0)));
        assert!(arena.set_scroll_offset(inner, Point::new(0.0, 30.0)));

        assert_eq!(arena.visual_layout(target).unwrap().y(), 40.0);
        assert_eq!(arena.hit_test(Point::new(10.0, 45.0)), Some(target));
        assert_ne!(arena.hit_test(Point::new(10.0, 95.0)), Some(target));
    }

    #[test]
    fn hit_test_allows_unclipped_overflow_and_respects_rounded_clips() {
        let mut arena = UiRuntime::new();
        let overflow_parent = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(40.0).height(40.0))),
        );
        let overflow_child = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .absolute()
                        .inset(xui_interface::EdgeInsets::new(50.0, 0.0, 0.0, 0.0))
                        .width(20.0)
                        .height(20.0),
                ),
            ),
        );
        arena.append_child(overflow_parent, overflow_child);
        let corner_child = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(40.0).height(40.0))),
        );
        arena.append_child(overflow_parent, corner_child);

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        assert_eq!(arena.hit_test(Point::new(55.0, 10.0)), Some(overflow_child));

        update_host(
            &mut arena,
            overflow_parent,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(40.0)
                        .height(40.0)
                        .clip(true)
                        .border_radius(20.0),
                ),
            ),
        );
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        assert_eq!(arena.hit_test(Point::new(1.0, 1.0)), Some(arena.root()));
        assert_eq!(arena.hit_test(Point::new(20.0, 20.0)), Some(corner_child));
        assert_ne!(arena.hit_test(Point::new(55.0, 10.0)), Some(overflow_child));
    }

    #[test]
    fn root_overlayer_honors_hit_test_and_modal_stacking() {
        let mut arena = UiRuntime::new();
        let content = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().size(Size::fill()))),
        );
        let pass_through = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .absolute()
                        .inset(xui_interface::EdgeInsets::zero())
                        .width(40.0)
                        .height(40.0),
                ),
            ),
        );
        let pass_through_entry = arena
            .mount_overlay_entry(
                pass_through,
                None,
                OverlayEntryOptions {
                    hit_test: false,
                    ..Default::default()
                },
            )
            .unwrap();

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        assert_eq!(arena.hit_test(Point::new(10.0, 10.0)), Some(content));

        arena
            .update_overlay_entry(pass_through_entry, None, OverlayEntryOptions::default())
            .unwrap();
        assert_eq!(arena.hit_test(Point::new(10.0, 10.0)), Some(pass_through));

        arena
            .update_overlay_entry(
                pass_through_entry,
                None,
                OverlayEntryOptions {
                    modal: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(arena.hit_test(Point::new(80.0, 80.0)), None);
    }

    #[test]
    fn hit_test_uses_reverse_paint_order_for_overlapping_layers() {
        let mut arena = UiRuntime::new();
        let stack = create_host(
            &mut arena,
            WidgetI::new(z_stack().style(Style::new().width(50.0).height(50.0))),
        );
        let back = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(50.0).height(50.0))),
        );
        let front = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(50.0).height(50.0))),
        );
        arena.append_child(stack, back);
        arena.append_child(stack, front);

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        assert_eq!(arena.hit_test(Point::new(10.0, 10.0)), Some(front));
    }

    fn update_host(arena: &mut UiRuntime, id: NodeId, widget: WidgetI) {
        let key = widget.key();
        let props_hash = widget.props_hash();
        let interaction = widget.take_host_interaction();
        arena.update_widget_node_from_parts(id, key, props_hash, widget, interaction);
    }

    // ---------------------------------------------------------------------
    // Event system: handler sharing, Flow, and the widget stage
    // ---------------------------------------------------------------------

    fn pointer_at(position: Point) -> RawPointerButton {
        RawPointerButton {
            position,
            pointer_id: XuiPointerId::new(0),
            device_id: None,
            kind: PointerKind::Mouse,
            button: PointerButton::Primary,
            buttons: PointerButtons::default(),
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
        }
    }

    /// Clicks the centre of `node` by driving the real raw-event pipeline, so
    /// the semantic events under test are the ones the translator actually
    /// produces.
    fn click(arena: &mut UiRuntime, measurer: &TextHost<ZeroTextBackend>, node: NodeId) {
        let bounds = arena.node(node).unwrap().layout;
        let point = Point::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        );
        let mut translator =
            EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
        arena.dispatch_event(measurer, &mut translator, pointer_move(point));
        arena.dispatch_event(
            measurer,
            &mut translator,
            RawEvent::PointerDown(pointer_at(point)),
        );
        arena.dispatch_event(
            measurer,
            &mut translator,
            RawEvent::PointerUp(pointer_at(point)),
        );
    }

    /// One handler, two widgets. Impossible with the previous
    /// `Box<dyn FnMut>`, which could be neither cloned nor shared, and the
    /// reason a component could not accept an event handler as a prop.
    #[test]
    fn one_handler_can_be_attached_to_two_widgets() {
        let calls = Rc::new(Cell::new(0u32));
        let handler = {
            let calls = Rc::clone(&calls);
            Handler::<ClickEvent>::new(move |_, _| {
                calls.set(calls.get() + 1);
            })
        };

        let mut arena = UiRuntime::new();
        let left = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(40.0, 40.0)))
                    .on_click(handler.clone().into_fn()),
            ),
        );
        let right = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(40.0, 40.0)))
                    .on_click(handler.clone().into_fn()),
            ),
        );

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

        click(&mut arena, &measurer, left);
        click(&mut arena, &measurer, right);

        assert_eq!(calls.get(), 2, "the shared handler did not run on both");
        assert!(handler.ptr_eq(&handler.clone()), "cloning changed identity");
    }

    /// Hovering a leaf marks every ancestor on the way to it, each exactly once.
    ///
    /// The translator diffs the old and new root-to-target paths, so the whole
    /// entered segment is reported — not just the leaf. This is what makes
    /// `style!(.. if hovered ..)` work on a container whose child is under the
    /// pointer.
    #[test]
    fn entering_a_leaf_reports_every_ancestor_that_gained_hover_once_each() {
        let counts: Rc<RefCell<Vec<(&'static str, bool)>>> = Rc::new(RefCell::new(Vec::new()));

        let mut arena = UiRuntime::new();

        let outer_log = Rc::clone(&counts);
        let outer = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(100.0, 100.0)))
                    .on_hovered(move |event, _| {
                        outer_log.borrow_mut().push(("outer", event.hovered));
                    }),
            ),
        );

        let mut nest = |arena: &mut UiRuntime, parent: NodeId, name: &'static str, size: f32| {
            let log = Rc::clone(&counts);
            let widget = WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(size, size)))
                    .on_hovered(move |event, _| {
                        log.borrow_mut().push((name, event.hovered));
                    }),
            );
            let key = widget.key();
            let props_hash = widget.props_hash();
            let interaction = widget.take_host_interaction();
            let id = arena.create_node(key, props_hash, widget, interaction);
            arena.append_child(parent, id);
            id
        };

        let middle = nest(&mut arena, outer, "middle", 80.0);
        let leaf = nest(&mut arena, middle, "leaf", 60.0);

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

        let bounds = arena.node(leaf).unwrap().layout;
        let inside = Point::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        );
        let mut translator =
            EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
        arena.dispatch_event(&measurer, &mut translator, pointer_move(inside));

        let seen = counts.borrow().clone();
        for name in ["outer", "middle", "leaf"] {
            let entered = seen
                .iter()
                .filter(|(node, hovered)| *node == name && *hovered)
                .count();
            assert_eq!(
                entered, 1,
                "`{name}` should have been reported hovered exactly once, got {entered} in {seen:?}"
            );
        }
        assert!(
            !seen.iter().any(|(_, hovered)| !*hovered),
            "nothing left the hover path yet, but a leave was reported: {seen:?}"
        );
    }

    /// `Hovered` is `Direct`, so an ancestor is reached because it is genuinely
    /// on the hover path — never a second time by a child's event bubbling up.
    /// Making it bubble would double-report every ancestor.
    #[test]
    fn an_ancestor_is_not_reported_twice_when_a_child_is_hovered() {
        let outer_events = Rc::new(Cell::new(0u32));
        let mut arena = UiRuntime::new();

        let counter = Rc::clone(&outer_events);
        let outer = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(100.0, 100.0)))
                    .on_hovered(move |_, _| counter.set(counter.get() + 1)),
            ),
        );
        let child = {
            let widget = WidgetI::new(container().style(Style::new().size(Size::fix(60.0, 60.0))));
            let key = widget.key();
            let props_hash = widget.props_hash();
            let interaction = widget.take_host_interaction();
            let id = arena.create_node(key, props_hash, widget, interaction);
            arena.append_child(outer, id);
            id
        };

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

        let bounds = arena.node(child).unwrap().layout;
        let inside = Point::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        );
        let mut translator =
            EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
        arena.dispatch_event(&measurer, &mut translator, pointer_move(inside));

        assert_eq!(
            outer_events.get(),
            1,
            "the ancestor saw its own event plus a bubbled copy of the child's"
        );
    }

    /// `cursor` lives in `ComputedStyle` but must never reach `StyleDiffFlags`.
    ///
    /// It has no scene output, so treating it like other style properties would
    /// make moving the pointer across a button dirty layout, paint, or text.
    /// This is the guard against someone completing the `diff` match later.
    #[test]
    fn changing_the_cursor_dirties_nothing() {
        let theme = Theme::default();
        let base = ComputedStyle::initial(&theme);
        let mut with_cursor = base.clone();
        with_cursor.cursor = Some(CursorIcon::Pointer);

        assert_ne!(
            base.cursor, with_cursor.cursor,
            "the fixture is not testing anything"
        );
        assert!(
            base.diff(&with_cursor).is_empty(),
            "a cursor change produced invalidation flags"
        );
    }

    /// Not inherited in the computed style: resolution walks up instead, so a
    /// child that specifies nothing shows its ancestor's cursor.
    #[test]
    fn an_unspecified_cursor_resolves_from_the_nearest_ancestor() {
        let mut arena = UiRuntime::new();
        let outer = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(100.0, 100.0)))
                    .cursor(CursorIcon::Pointer),
            ),
        );
        let inner = {
            let widget = WidgetI::new(container().style(Style::new().size(Size::fix(50.0, 50.0))));
            let key = widget.key();
            let props_hash = widget.props_hash();
            let interaction = widget.take_host_interaction();
            let id = arena.create_node(key, props_hash, widget, interaction);
            arena.append_child(outer, id);
            id
        };

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

        assert_eq!(
            arena.style_system.styles(inner).unwrap().1.cursor,
            None,
            "the child should not have inherited a resolved cursor"
        );

        let bounds = arena.node(inner).unwrap().layout;
        let inside = Point::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        );
        let mut translator =
            EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
        arena.dispatch_event(&measurer, &mut translator, pointer_move(inside));

        assert_eq!(arena.hovered_node(), Some(inner));
        assert_eq!(arena.resolved_cursor(), CursorIcon::Pointer);
    }

    /// A captured pointer keeps its own cursor even once it has left the
    /// capturing node — the same rule that keeps events aimed there.
    #[test]
    fn a_captured_pointer_keeps_its_own_cursor() {
        let mut arena = UiRuntime::new();
        let grabber = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(50.0, 50.0)))
                    .cursor(CursorIcon::Grabbing)
                    .on_press_start(|_, cx| {
                        cx.capture_pointer();
                    }),
            ),
        );
        let elsewhere = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(50.0, 50.0)))
                    .cursor(CursorIcon::Text),
            ),
        );

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

        let centre = |arena: &UiRuntime, node: NodeId| {
            let bounds = arena.node(node).unwrap().layout;
            Point::new(
                (bounds.min.x + bounds.max.x) * 0.5,
                (bounds.min.y + bounds.max.y) * 0.5,
            )
        };
        let on_grabber = centre(&arena, grabber);
        let on_elsewhere = centre(&arena, elsewhere);

        let mut translator =
            EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
        arena.dispatch_event(&measurer, &mut translator, pointer_move(on_grabber));
        assert_eq!(arena.resolved_cursor(), CursorIcon::Grabbing);

        arena.dispatch_event(
            &measurer,
            &mut translator,
            RawEvent::PointerDown(pointer_at(on_grabber)),
        );
        arena.dispatch_event(&measurer, &mut translator, pointer_move(on_elsewhere));

        assert_eq!(
            arena.resolved_cursor(),
            CursorIcon::Grabbing,
            "the cursor followed hit testing instead of the capture"
        );
    }

    /// A widget's own default reaches the platform without the application
    /// asking, and stays overridable.
    #[test]
    fn a_widget_default_cursor_applies_and_can_be_overridden() {
        let mut arena = UiRuntime::new();
        let input = create_host(
            &mut arena,
            WidgetI::new(text_input().style(Style::new().size(Size::fix(80.0, 30.0)))),
        );
        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

        let bounds = arena.node(input).unwrap().layout;
        let inside = Point::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        );
        let mut translator =
            EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
        arena.dispatch_event(&measurer, &mut translator, pointer_move(inside));
        assert_eq!(arena.resolved_cursor(), CursorIcon::Text);

        let mut overridden = UiRuntime::new();
        let node = create_host(
            &mut overridden,
            WidgetI::new(
                text_input().style(
                    Style::new()
                        .size(Size::fix(80.0, 30.0))
                        .cursor(CursorIcon::NotAllowed),
                ),
            ),
        );
        overridden.update_tree(Size::new(200.0, 200.0), &mut measurer);
        let bounds = overridden.node(node).unwrap().layout;
        let inside = Point::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        );
        let mut translator =
            EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
        overridden.dispatch_event(&measurer, &mut translator, pointer_move(inside));
        assert_eq!(overridden.resolved_cursor(), CursorIcon::NotAllowed);
    }

    fn child_of(arena: &mut UiRuntime, parent: NodeId, widget: WidgetI) -> NodeId {
        let key = widget.key();
        let props_hash = widget.props_hash();
        let interaction = widget.take_host_interaction();
        let id = arena.create_node(key, props_hash, widget, interaction);
        arena.append_child(parent, id);
        id
    }

    /// A pane that fills the leftover height and scrolls, above a pane sized by
    /// its content.
    ///
    /// The canonical app shell, and the layout most likely to break: it depends
    /// on `Sizing::Fill` becoming `flex_grow` on the main axis, on the sibling
    /// staying content-sized, and on the scrolling pane being allowed to be
    /// shorter than its content. The last one is why `min_size` is pinned to
    /// zero rather than taffy's `auto` — see `taffy_style_for_widget`.
    fn build_two_pane_shell(scrollable: bool) -> (UiRuntime, NodeId, NodeId, NodeId) {
        let mut arena = UiRuntime::new();
        let vcol = || container().flex_direction(FlexDirectionStyle::Column);

        let outer = create_host(
            &mut arena,
            WidgetI::new(vcol().style(Style::new().width(Sizing::Fill).height(Sizing::Fill))),
        );

        let mut top = Style::new().width(Sizing::Fill).height(Sizing::Fill);
        if scrollable {
            top = top.scroll_vertical();
        }
        let filling = child_of(&mut arena, outer, WidgetI::new(vcol().style(top)));
        // 500 of content in a pane that can only be 240 tall.
        for _ in 0..5 {
            child_of(
                &mut arena,
                filling,
                WidgetI::new(container().style(Style::new().width(Sizing::Fill).height(100.0))),
            );
        }

        let hugging = child_of(
            &mut arena,
            outer,
            WidgetI::new(vcol().style(Style::new().width(Sizing::Fill))),
        );
        child_of(
            &mut arena,
            hugging,
            WidgetI::new(container().style(Style::new().width(Sizing::Fill).height(60.0))),
        );

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(400.0, 300.0), &mut measurer);
        (arena, outer, filling, hugging)
    }

    #[test]
    fn a_filling_scroll_pane_shares_the_viewport_with_a_content_sized_sibling() {
        let (arena, outer, filling, hugging) = build_two_pane_shell(true);
        let height = |id: NodeId| arena.node(id).unwrap().layout.height();

        assert_eq!(
            height(outer),
            300.0,
            "the shell must not exceed the viewport"
        );
        assert_eq!(
            height(hugging),
            60.0,
            "the bottom pane is sized by its content"
        );
        assert_eq!(
            height(filling),
            240.0,
            "the top pane takes exactly what is left"
        );
        assert_eq!(
            arena.node(filling).unwrap().content_size.height,
            500.0,
            "the pane must still know how tall its content is, or it cannot scroll"
        );
    }

    /// Without a scroller, the filling pane is still capped at the space it was
    /// given; its content overflows rather than pushing the shell past the
    /// viewport.
    ///
    /// This is what `min_size: ZERO` buys. With taffy's default of `auto`, the
    /// automatic minimum size of a flex item is its content, so this same tree
    /// lays out 560 tall inside a 300 tall window.
    #[test]
    fn a_filling_pane_is_capped_even_when_its_content_does_not_fit() {
        let (arena, outer, filling, hugging) = build_two_pane_shell(false);
        let height = |id: NodeId| arena.node(id).unwrap().layout.height();

        assert_eq!(height(outer), 300.0);
        assert_eq!(height(filling), 240.0);
        assert_eq!(height(hugging), 60.0);
    }

    /// The counter that lets raw dispatch skip its ancestor walk. If it ever
    /// drifts, raw events stop reaching the widgets that need them — silently —
    /// so it is checked against node creation and removal directly.
    #[test]
    fn the_raw_listener_count_tracks_node_lifetime() {
        let mut arena = UiRuntime::new();
        assert!(
            !arena.has_raw_event_listeners(),
            "an empty tree has nothing reading raw events"
        );

        let plain = create_host(&mut arena, WidgetI::new(container()));
        assert!(!arena.has_raw_event_listeners());
        assert!(!arena.node_reads_raw_events(plain));

        let input = create_host(&mut arena, WidgetI::new(text_input()));
        assert!(arena.has_raw_event_listeners());
        assert!(arena.node_reads_raw_events(input));

        arena.remove_subtree(plain);
        assert!(
            arena.has_raw_event_listeners(),
            "removing an unrelated node dropped the count"
        );

        arena.remove_subtree(input);
        assert!(!arena.has_raw_event_listeners());
    }

    /// Capture steers *both* dispatch layers.
    ///
    /// The runtime and the translator used to keep separate pointer-capture
    /// state, and only the runtime's was ever written to — so raw events
    /// followed the capture while semantic events silently hit-tested. They now
    /// resolve through the same one.
    #[test]
    fn a_captured_pointer_keeps_aiming_at_the_capturing_node() {
        let hovered_sibling = Rc::new(Cell::new(false));
        let flag = Rc::clone(&hovered_sibling);

        let mut arena = UiRuntime::new();
        let grabber = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(50.0, 50.0)))
                    .on_press_start(|_, cx| {
                        cx.capture_pointer();
                    }),
            ),
        );
        let sibling = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(50.0, 50.0)))
                    .on_hovered(move |event, _| {
                        if event.hovered {
                            flag.set(true);
                        }
                    }),
            ),
        );

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

        let centre = |arena: &UiRuntime, node: NodeId| {
            let bounds = arena.node(node).unwrap().layout;
            Point::new(
                (bounds.min.x + bounds.max.x) * 0.5,
                (bounds.min.y + bounds.max.y) * 0.5,
            )
        };
        let on_grabber = centre(&arena, grabber);
        let on_sibling = centre(&arena, sibling);
        assert_ne!(on_grabber, on_sibling, "the two nodes must not overlap");

        let mut translator =
            EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
        arena.dispatch_event(&measurer, &mut translator, pointer_move(on_grabber));
        arena.dispatch_event(
            &measurer,
            &mut translator,
            RawEvent::PointerDown(pointer_at(on_grabber)),
        );
        assert_eq!(
            arena.pointer_capture_node(),
            Some(grabber),
            "the press handler did not take the capture"
        );

        arena.dispatch_event(&measurer, &mut translator, pointer_move(on_sibling));

        assert!(
            !hovered_sibling.get(),
            "the pointer was captured, but the semantic layer still hit-tested \
             its way onto the sibling"
        );
    }

    /// A handler that returns nothing at all is the common case; it used to have
    /// to spell out `EventResult::Ignored`.
    #[test]
    fn a_handler_may_return_unit_flow_or_the_old_event_result() {
        let mut arena = UiRuntime::new();
        let node = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(40.0, 40.0)))
                    .on_click(|_, _| {})
                    .on_press_start(|_, _| Flow::empty())
                    .on_hovered(|_, _| EventResult::Ignored),
            ),
        );
        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
        click(&mut arena, &measurer, node);
        assert!(arena.contains(node));
    }

    /// `STOP_PROPAGATION` keeps an ancestor from seeing the event; on its own it
    /// says nothing about the widget's own behaviour.
    #[test]
    fn stopping_propagation_hides_the_event_from_ancestors() {
        let seen_by_parent = Rc::new(Cell::new(false));
        let mut arena = UiRuntime::new();

        let parent_flag = Rc::clone(&seen_by_parent);
        let parent = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(80.0, 80.0)))
                    .on_click(move |_, _| parent_flag.set(true)),
            ),
        );
        let child = {
            let widget = WidgetI::new(
                container()
                    .style(Style::new().size(Size::fix(40.0, 40.0)))
                    .on_click(|_, _| Flow::STOP_PROPAGATION),
            );
            let key = widget.key();
            let props_hash = widget.props_hash();
            let interaction = widget.take_host_interaction();
            let id = arena.create_node(key, props_hash, widget, interaction);
            arena.append_child(parent, id);
            id
        };

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
        click(&mut arena, &measurer, child);

        assert!(
            !seen_by_parent.get(),
            "the click bubbled past a handler that stopped it"
        );
    }

    fn pointer_move(position: Point) -> RawEvent {
        RawEvent::PointerMove(RawPointerMove {
            position,
            pointer_id: XuiPointerId::new(0),
            device_id: None,
            kind: PointerKind::Mouse,
            button: None,
            buttons: PointerButtons::default(),
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
        })
    }

    #[test]
    fn host_metadata_binds_focus_handle_and_preserves_accessibility() {
        let mut arena = UiRuntime::new();
        let handle = FocusHandle::new();
        let widget = WidgetI::new(
            container()
                .focusable(true)
                .tab_index(-1)
                .focus_handle(handle.clone())
                .accessibility_role(xui_interface::AccessibilityRole::Tab)
                .accessibility_id("settings-tab")
                .accessibility_label("Settings")
                .accessibility_selected(true)
                .accessibility_controls("settings-panel"),
        );
        let key = widget.key();
        let props_hash = widget.props_hash();
        let handlers = widget.take_host_interaction();
        let node = arena.create_node(key, props_hash, widget, handlers);
        arena.append_child(arena.root(), node);

        assert_eq!(handle.node_id(), Some(node));
        assert!(arena.is_focusable(node));
        assert!(!arena.is_sequentially_focusable(node));
        assert_eq!(
            arena.accessibility(node).unwrap().role,
            Some(xui_interface::AccessibilityRole::Tab)
        );
        assert_eq!(
            arena.accessibility(node).unwrap().controls.as_deref(),
            Some("settings-panel")
        );

        arena.remove_subtree(node);
        assert!(!handle.is_bound());
    }

    #[test]
    fn subtree_removal_clears_all_subsystem_caches() {
        let mut arena = UiRuntime::new();
        let node = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .focusable(true)
                    .on_click(|_, _| EventResult::Ignored),
            ),
        );

        assert!(arena.hosts.contains_key(node));
        assert!(arena.style_system.contains(node));
        assert!(arena.layout_tree.contains_host(node));
        assert!(arena.interaction_system.get(node).is_some());
        assert!(arena.render_system.host_binding(node).is_some());

        arena.remove_subtree(node);

        assert!(!arena.hosts.contains_key(node));
        assert!(!arena.style_system.contains(node));
        assert!(!arena.layout_tree.contains_host(node));
        assert!(arena.interaction_system.get(node).is_none());
        assert!(arena.render_system.host_binding(node).is_none());
    }

    #[test]
    fn interaction_cache_remains_sparse_for_default_widgets() {
        let mut arena = UiRuntime::new();
        let node = create_host(&mut arena, WidgetI::new(container()));

        assert!(arena.interaction_system.get(node).is_none());
    }

    #[test]
    fn focus_handle_can_request_focus_for_another_node() {
        let mut arena = UiRuntime::new();
        let target_handle = FocusHandle::new();

        let source_widget = WidgetI::new(container().focusable(true));
        let source_hash = source_widget.props_hash();
        let source_handlers = source_widget.take_host_interaction();
        let source = arena.create_node(None, source_hash, source_widget, source_handlers);
        arena.append_child(arena.root(), source);

        let target_widget = WidgetI::new(
            container()
                .focusable(true)
                .focus_handle(target_handle.clone()),
        );
        let target_hash = target_widget.props_hash();
        let target_handlers = target_widget.take_host_interaction();
        let target = arena.create_node(None, target_hash, target_widget, target_handlers);
        arena.append_child(arena.root(), target);

        let mut flags = WidgetUpdateFlags::empty();
        let mut requests = xui_interface::EventRequests::default();
        let mut cx = crate::event_system::EventContext::new(
            arena.node(source).unwrap(),
            None,
            xui_interface::EventPhase::Target,
            &mut flags,
            &mut requests,
        );
        assert!(target_handle.request_focus(&mut cx));
        assert_eq!(
            requests.iter().collect::<Vec<_>>(),
            vec![xui_interface::EventRequest::Focus(target)]
        );
    }

    #[test]
    fn host_metadata_and_focus_handle_update_with_reused_node() {
        let mut arena = UiRuntime::new();
        let old_handle = FocusHandle::new();
        let initial = WidgetI::new(
            container()
                .tab_index(0)
                .focus_handle(old_handle.clone())
                .accessibility_role(xui_interface::AccessibilityRole::Tab)
                .accessibility_selected(false),
        );
        let initial_hash = initial.props_hash();
        let initial_handlers = initial.take_host_interaction();
        let node = arena.create_node(None, initial_hash, initial, initial_handlers);
        arena.append_child(arena.root(), node);

        let new_handle = FocusHandle::new();
        let updated = WidgetI::new(
            container()
                .tab_index(-1)
                .focus_handle(new_handle.clone())
                .accessibility_role(xui_interface::AccessibilityRole::Tab)
                .accessibility_selected(true),
        );
        let updated_hash = updated.props_hash();
        let updated_handlers = updated.take_host_interaction();
        arena.update_widget_node_from_parts(node, None, updated_hash, updated, updated_handlers);

        assert!(!old_handle.is_bound());
        assert_eq!(new_handle.node_id(), Some(node));
        assert_eq!(arena.tab_index(node), Some(-1));
        assert_eq!(arena.accessibility(node).unwrap().selected, Some(true));
    }

    #[test]
    fn final_text_width_is_activated_after_layout_measurement() {
        let mut arena = UiRuntime::new();
        let node = create_host(
            &mut arena,
            WidgetI::new(TextWidget::new("飞行监测").style(Style::new().width(120.0))),
        );
        let mut measurer = TextHost::new(ZeroTextBackend);

        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

        let active = measurer
            .active_slot(node, TextLayoutSlot::PRIMARY)
            .expect("final layout must activate regular text");
        let host_node = arena.node(node).unwrap();
        assert_eq!(host_node.layout.width(), 120.0);
        let props = host_node
            .widget
            .with_widgets(|widget| widget.text_layout_props(&host_node.effective_style))
            .unwrap();
        let final_input = TextLayoutInput::new(
            props.text,
            TextLayoutConstraints::max_width(host_node.layout.width()),
            props.style.into(),
            props.paragraph,
            props.text_box,
            measurer.backend().epoch(),
        );

        let expected = measurer.activate_slot(node, TextLayoutSlot::PRIMARY, final_input);
        assert_eq!(active, expected);

        let host_node = arena.node(node).unwrap();
        let props = host_node
            .widget
            .with_widgets(|widget| widget.text_layout_props(&host_node.effective_style))
            .unwrap();
        let font_context = measurer.backend().epoch();
        measurer.measure_slot(
            node,
            TextLayoutSlot::PRIMARY,
            TextLayoutInput::new(
                props.text,
                TextLayoutConstraints::MIN_SIZE,
                props.style.into(),
                props.paragraph,
                props.text_box,
                font_context,
            ),
        );
        assert_eq!(
            measurer.active_slot(node, TextLayoutSlot::PRIMARY),
            Some(active)
        );
    }

    #[test]
    fn text_nodes_preserve_fractional_taffy_geometry() {
        let mut arena = UiRuntime::new();
        let node = create_host(
            &mut arena,
            WidgetI::new(TextWidget::new("中文").style(Style::new().width(45.5).height(20.0))),
        );
        let mut measurer = TextHost::new(ZeroTextBackend);

        arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

        let taffy_node = arena.layout_node_id(node);
        let rounded_width = arena.layout_tree.layout(taffy_node).unwrap().size.width;
        let unrounded_width = arena.layout_tree.unrounded_layout(taffy_node).size.width;
        assert_eq!(rounded_width, 46.0);
        assert_eq!(unrounded_width, 45.5);
        assert_eq!(arena.node(node).unwrap().layout.width(), unrounded_width);
    }

    #[test]
    fn resizing_invalidates_intrinsic_text_layout_caches() {
        fn create_child(arena: &mut UiRuntime, parent: NodeId, widget: WidgetI) -> NodeId {
            let child = create_host(arena, widget);
            arena.append_child(parent, child);
            child
        }

        let mut arena = UiRuntime::new();
        let outer = create_host(
            &mut arena,
            WidgetI::new(
                container()
                    .flex_direction(xui_interface::FlexDirectionStyle::Row)
                    .style(
                        Style::new()
                            .size(Size::fill())
                            .padding(xui_interface::EdgeInsets::all(16.0))
                            .gap(16.0),
                    ),
            ),
        );
        create_child(
            &mut arena,
            outer,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(xui_interface::Sizing::percent(0.4))
                        .height(xui_interface::Sizing::fill()),
                ),
            ),
        );
        let analytics = create_child(
            &mut arena,
            outer,
            WidgetI::new(
                container()
                    .flex_direction(xui_interface::FlexDirectionStyle::Column)
                    .style(
                        Style::new()
                            .size(Size::fill())
                            .padding(xui_interface::EdgeInsets::all(16.0))
                            .gap(12.0),
                    ),
            ),
        );
        let tabs = create_child(
            &mut arena,
            analytics,
            WidgetI::new(
                container()
                    .flex_direction(xui_interface::FlexDirectionStyle::Row)
                    .style(
                        Style::new()
                            .gap(3.0)
                            .padding(xui_interface::EdgeInsets::all(4.0))
                            .border_width(1.0),
                    ),
            ),
        );
        let tab = create_child(
            &mut arena,
            tabs,
            WidgetI::new(
                container().style(
                    Style::new()
                        .padding(xui_interface::EdgeInsets::symmetric(16.0, 6.0))
                        .font_size(12.0)
                        .border_width(1.0),
                ),
            ),
        );
        let label = create_child(&mut arena, tab, WidgetI::new(TextWidget::new("飞行监测")));
        create_child(
            &mut arena,
            analytics,
            WidgetI::new(container().style(Style::new().size(Size::fill()))),
        );

        let mut measurer = TextHost::new(xui_cosmic::CosmicEngine::new(1.0));
        arena.update_tree(Size::new(1600.0, 900.0), &mut measurer);
        let expected_width = arena.node(label).unwrap().layout.width();
        assert!(expected_width > 12.0);

        for width in [900.0, 2000.0] {
            arena.mark_subtree_layout_dirty(arena.root());
            arena.update_tree(Size::new(width, 900.0), &mut measurer);

            let final_width = arena.node(label).unwrap().layout.width();
            let taffy_node = arena.layout_node_id(label);
            let unrounded_width = arena.layout_tree.unrounded_layout(taffy_node).size.width;
            assert!(
                (final_width - expected_width).abs() < 0.01,
                "text width changed from {expected_width} to {final_width} after resizing to {width}"
            );
            assert_eq!(
                final_width, unrounded_width,
                "text layout must preserve its fractional intrinsic width"
            );
            let active = measurer
                .active_slot(label, TextLayoutSlot::PRIMARY)
                .and_then(|handle| measurer.layout(handle))
                .expect("final text layout must be active");
            assert!((active.size().width - final_width).abs() < 0.01);
            assert_eq!(
                active.lines.len(),
                1,
                "intrinsically-sized CJK text wrapped after resizing to {width}: rounded={:?}, unrounded={:?}, lines={:?}",
                arena.layout_tree.layout(taffy_node).unwrap(),
                arena.layout_tree.unrounded_layout(taffy_node),
                active.lines,
            );
        }
    }

    #[test]
    fn local_paint_style_update_skips_unrelated_branch_and_layout() {
        let mut arena = UiRuntime::new();
        let left = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(100.0).height(100.0))),
        );
        let right = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(100.0).height(100.0))),
        );
        let left_leaf = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(20.0).height(20.0))),
        );
        let right_leaf = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(20.0).height(20.0))),
        );
        arena.append_child(left, left_leaf);
        arena.append_child(right, right_leaf);

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
        let layout_passes = arena.layout_passes;
        let repaint_passes = arena.repaint_passes;

        update_host(
            &mut arena,
            left_leaf,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(20.0)
                        .height(20.0)
                        .background(Color::BLACK),
                ),
            ),
        );
        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

        assert_eq!(arena.update_visits, 1);
        assert_eq!(arena.layout_passes, layout_passes);
        assert_eq!(arena.repaint_passes - repaint_passes, 1);
        assert_eq!(arena.node(right_leaf).unwrap().layout.width(), 20.0);
    }

    #[test]
    fn state_style_changes_use_transition_owned_by_style() {
        let mut arena = UiRuntime::new();
        let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
        let style = Style::new()
            .width(20.0)
            .height(20.0)
            .background(Color::BLACK)
            .when(WidgetState::HOVERED, |patch| patch.background(Color::WHITE))
            .transition(transition);
        let node = create_host(&mut arena, WidgetI::new(container().style(style)));
        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        arena.set_widget_state_flag(node, WidgetState::HOVERED, true);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        assert!(arena.has_running_style_animations());

        arena.tick_style_animations(Duration::from_millis(50));
        let effective = arena.style_system.effective(node).unwrap();
        let ComputedColorStyle::Solid(background) = effective.paint.background else {
            panic!("expected solid background")
        };
        assert!((background.r - 0.5).abs() < 0.0001);
        let layout_passes = arena.layout_passes;
        let repaint_passes = arena.repaint_passes;

        let style_without_transition = Style::new()
            .width(20.0)
            .height(20.0)
            .background(Color::BLACK)
            .when(WidgetState::HOVERED, |patch| patch.background(Color::WHITE));
        update_host(
            &mut arena,
            node,
            WidgetI::new(container().style(style_without_transition)),
        );
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        assert!(!arena.has_running_style_animations());
        let effective = arena.style_system.effective(node).unwrap();
        assert_eq!(
            effective.paint.background,
            ComputedColorStyle::Solid(Color::WHITE)
        );
        assert_eq!(arena.layout_passes, layout_passes);
        assert_eq!(arena.repaint_passes, repaint_passes + 1);
    }

    #[test]
    fn inherited_text_color_follows_parent_transition_sample_without_layout() {
        let mut arena = UiRuntime::new();
        let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
        let tab = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .color(Color::BLACK)
                        .when(WidgetState::HOVERED, |patch| patch.color(Color::WHITE))
                        .transition(transition),
                ),
            ),
        );
        let label = create_host(&mut arena, WidgetI::new(TextWidget::new("New member")));
        let explicit_label = create_host(
            &mut arena,
            WidgetI::new(TextWidget::new("Pinned").style(Style::new().color(Color::BLUE_500))),
        );
        arena.append_child(tab, label);
        arena.append_child(tab, explicit_label);

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
        arena.set_widget_state_flag(tab, WidgetState::HOVERED, true);
        arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
        let layout_passes = arena.layout_passes;

        assert!(arena.tick_style_animations(Duration::from_millis(50)));
        arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

        let parent_color = arena.style_system.effective(tab).unwrap().text.color;
        let label_color = arena.style_system.effective(label).unwrap().text.color;
        assert!((parent_color.r - 0.5).abs() < 0.0001);
        assert_eq!(label_color, parent_color);
        assert_eq!(
            arena
                .style_system
                .effective(explicit_label)
                .unwrap()
                .text
                .color,
            Color::BLUE_500
        );
        assert_eq!(arena.layout_passes, layout_passes);
    }

    #[test]
    fn hovered_cjk_tab_keeps_one_line_and_finishes_paint_transition() {
        let mut arena = UiRuntime::new();
        let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
        let tab = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .padding(xui_interface::EdgeInsets::symmetric(16.0, 6.0))
                        .color(Color::BLACK)
                        .font_family("PingFang SC")
                        .font_size(12.0)
                        .border_width(1.0)
                        .when(WidgetState::HOVERED, |patch| {
                            patch.background(Color::BLACK).color(Color::WHITE)
                        })
                        .transition(transition),
                ),
            ),
        );
        let label = create_host(&mut arena, WidgetI::new(TextWidget::new("飞行监测")));
        arena.append_child(tab, label);

        let size = Size::new(400.0, 200.0);
        let mut measurer = TextHost::new(xui_cosmic::CosmicEngine::new(1.0));
        arena.update_tree(size, &mut measurer);
        let layout_passes = arena.layout_passes;
        let tab_bounds = arena.node(tab).unwrap().layout;
        let pointer = Point::new(
            (tab_bounds.min.x + tab_bounds.max.x) * 0.5,
            (tab_bounds.min.y + tab_bounds.max.y) * 0.5,
        );
        let mut translator =
            EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());

        arena.dispatch_event(&measurer, &mut translator, pointer_move(pointer));
        arena.update_tree(size, &mut measurer);
        assert!(
            arena
                .node(tab)
                .unwrap()
                .state
                .contains(WidgetState::HOVERED)
        );
        assert!(arena.has_running_style_animations());
        assert_eq!(arena.layout_passes, layout_passes);

        for frame in 0..20 {
            // Repeated cursor notifications at a stationary position must not
            // toggle the ancestor hover state or restart its transition.
            arena.dispatch_event(&measurer, &mut translator, pointer_move(pointer));
            assert!(
                arena.ui_state.layout_dirty_list.is_empty(),
                "pointer dispatch dirtied layout on frame {frame}"
            );
            arena.tick_style_animations(Duration::from_millis(8));
            assert!(
                arena.ui_state.layout_dirty_list.is_empty(),
                "animation tick dirtied layout on frame {frame}"
            );
            arena.update_tree(size, &mut measurer);

            assert!(
                arena
                    .node(tab)
                    .unwrap()
                    .state
                    .contains(WidgetState::HOVERED)
            );
            let active = measurer
                .active_slot(label, TextLayoutSlot::PRIMARY)
                .and_then(|handle| measurer.layout(handle))
                .expect("hovered tab label must retain an active layout");
            assert_eq!(active.lines.len(), 1, "hover animation wrapped CJK label");
            assert_eq!(
                arena.layout_passes, layout_passes,
                "paint-only hover transition triggered layout on frame {frame}"
            );
        }

        assert!(!arena.has_running_style_animations());
    }

    #[test]
    fn layout_transition_updates_effective_taffy_style_each_frame() {
        let mut arena = UiRuntime::new();
        let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
        let node = create_host(
            &mut arena,
            WidgetI::new(
                container().style(Style::new().width(20.0).height(20.0).transition(transition)),
            ),
        );
        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

        update_host(
            &mut arena,
            node,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(100.0)
                        .height(20.0)
                        .transition(transition),
                ),
            ),
        );
        arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
        let layout_passes = arena.layout_passes;

        assert!(arena.tick_style_animations(Duration::from_millis(50)));
        arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

        let effective = arena.style_system.effective(node).unwrap();
        let xui_interface::Sizing::Fix(width) = effective.layout.width else {
            panic!("expected fixed animated width")
        };
        assert!((width.into_inner() - 60.0).abs() < 0.0001);
        assert!((arena.node(node).unwrap().layout.width() - 60.0).abs() < 0.0001);
        assert_eq!(arena.layout_passes, layout_passes + 1);
    }

    #[test]
    fn paint_only_transition_does_not_run_layout() {
        let mut arena = UiRuntime::new();
        let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
        let node = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(20.0)
                        .height(20.0)
                        .background(Color::BLACK)
                        .transition(transition),
                ),
            ),
        );
        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        update_host(
            &mut arena,
            node,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(20.0)
                        .height(20.0)
                        .background(Color::WHITE)
                        .transition(transition),
                ),
            ),
        );
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        let layout_passes = arena.layout_passes;

        assert!(arena.tick_style_animations(Duration::from_millis(50)));
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        assert_eq!(arena.layout_passes, layout_passes);
    }

    #[test]
    fn state_transform_transition_updates_only_frame_properties() {
        let mut arena = UiRuntime::new();
        let transition = Transition::new(Duration::from_millis(100)).ease(Easing::QuadIn);
        let style = Style::new()
            .width(20.0)
            .height(20.0)
            .when(WidgetState::PRESSED, |patch| patch.translate_y(4.0))
            .transition(transition);
        let node = create_host(&mut arena, WidgetI::new(container().style(style)));
        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        arena.set_widget_state_flag(node, WidgetState::PRESSED, true);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        let layout_passes = arena.layout_passes;
        let repaint_passes = arena.repaint_passes;
        let transform_node = arena.render_system.host_binding(node).unwrap().transform;
        assert!(
            arena
                .render_system
                .properties
                .transform(transform_node)
                .is_none()
        );

        assert!(arena.tick_style_animations(Duration::from_millis(50)));
        let effective = arena.style_system.effective(node).unwrap();
        assert!((effective.transform.translate.y - 1.0).abs() < 0.0001);
        assert_eq!(
            arena
                .render_system
                .properties
                .transform(transform_node)
                .unwrap()
                .value,
            Affine::translate(0.0, 1.0)
        );

        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        assert_eq!(arena.layout_passes, layout_passes);
        assert_eq!(arena.repaint_passes, repaint_passes);
    }

    #[test]
    fn direct_transform_update_skips_layout_and_repaint() {
        let mut arena = UiRuntime::new();
        let node = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(20.0).height(20.0))),
        );
        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        let layout_passes = arena.layout_passes;
        let repaint_passes = arena.repaint_passes;

        update_host(
            &mut arena,
            node,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(20.0)
                        .height(20.0)
                        .translate(Point::new(3.0, 5.0)),
                ),
            ),
        );
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        let transform_node = arena.render_system.host_binding(node).unwrap().transform;
        assert_eq!(
            arena
                .render_system
                .properties
                .transform(transform_node)
                .unwrap()
                .value,
            Affine::translate(3.0, 5.0)
        );
        assert_eq!(arena.layout_passes, layout_passes);
        assert_eq!(arena.repaint_passes, repaint_passes);
    }
}

fn measure_layout_context<T: TextBackend>(
    ui_tree: &HostTree<HostData>,
    styles: &StyleSystem,
    known_dimensions: tf::Size<Option<f32>>,
    available_space: tf::Size<tf::AvailableSpace>,
    node_context: Option<&mut WidgetContext>,
    measurer: &mut TextHost<T>,
) -> MeasuredLeaf {
    let known_size = if let tf::Size {
        width: Some(width),
        height: Some(height),
    } = known_dimensions
    {
        Some(tf::Size { width, height })
    } else {
        None
    };

    match node_context {
        Some(WidgetContext::Text(node_id)) => {
            let node = ui_tree.get(*node_id).expect("node not found");
            let effective = styles.effective(*node_id).expect("style node missing");
            if let Some(props) = node.widget.with_widgets(|w| w.text_layout_props(effective)) {
                let constraints = if node.node_type == WidgetType::TextInput {
                    TextLayoutConstraints::UNBOUNDED
                } else {
                    match known_dimensions.width {
                        Some(width) => TextLayoutConstraints::max_width(width),
                        None => match available_space.width {
                            tf::AvailableSpace::MaxContent => TextLayoutConstraints::UNBOUNDED,
                            tf::AvailableSpace::MinContent => TextLayoutConstraints::MIN_SIZE,
                            tf::AvailableSpace::Definite(width) => {
                                TextLayoutConstraints::max_width(width)
                            }
                        },
                    }
                };

                let font_context = measurer.backend().epoch();
                let input = TextLayoutInput::new(
                    props.text,
                    constraints,
                    props.style.into(),
                    props.paragraph,
                    props.text_box,
                    font_context,
                );
                let metrics =
                    measurer.measure_slot_metrics(*node_id, TextLayoutSlot::PRIMARY, input);
                return MeasuredLeaf {
                    size: tf::Size {
                        width: known_dimensions.width.unwrap_or(metrics.size.width),
                        height: known_dimensions.height.unwrap_or(metrics.size.height),
                    },
                    // Paragraph baselines are content-box local. Taffy expects
                    // the baseline from the leaf's border-box top edge.
                    first_baseline: metrics
                        .first_baseline
                        .map(|baseline| effective.layout.padding.top + baseline),
                };
            } else {
                return MeasuredLeaf::from_size(tf::Size {
                    width: known_dimensions.width.unwrap_or(0.0),
                    height: known_dimensions.height.unwrap_or(0.0),
                });
            }
        }
        Some(WidgetContext::Image(size)) => MeasuredLeaf::from_size(tf::Size {
            width: known_dimensions.width.unwrap_or(size.width),
            height: known_dimensions.height.unwrap_or(size.height),
        }),

        _ => {
            if let Some(size) = known_size {
                return MeasuredLeaf::from_size(size);
            }
            MeasuredLeaf::from_size(tf::Size {
                width: 0.0,
                height: 0.0,
            })
        }
    }
}

fn ergonomic_scroll_delta(delta: Point) -> Point {
    let magnitude = (delta.x * delta.x + delta.y * delta.y).sqrt();
    let factor = scroll_acceleration_factor(magnitude);
    Point::new(delta.x * factor, delta.y * factor)
}

fn scroll_acceleration_factor(magnitude: f32) -> f32 {
    const MIN_ACCELERATION_MAGNITUDE: f32 = 80.0;
    const FULL_ACCELERATION_MAGNITUDE: f32 = 160.0;
    const MAX_ACCELERATION: f32 = 2.75;

    if magnitude <= MIN_ACCELERATION_MAGNITUDE {
        return 1.0;
    }

    let progress =
        ((magnitude - MIN_ACCELERATION_MAGNITUDE) / FULL_ACCELERATION_MAGNITUDE).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - progress).powi(2);
    1.0 + eased * (MAX_ACCELERATION - 1.0)
}

fn clamp_scroll_offset(
    layout: &mut crate::ui_runtime::layout::LayoutNode,
    scroll: ComputedScrollStyle,
) {
    let direction = scroll.direction;
    let max_x = if direction.allows_horizontal() {
        (layout.content_size.width - layout.layout.width()).max(0.0)
    } else {
        0.0
    };
    let max_y = if direction.allows_vertical() {
        (layout.content_size.height - layout.layout.height()).max(0.0)
    } else {
        0.0
    };
    layout.scroll_offset.x = layout.scroll_offset.x.clamp(0.0, max_x);
    layout.scroll_offset.y = layout.scroll_offset.y.clamp(0.0, max_y);
}

fn needs_scrollbar_overlay(node: NodeView<'_>) -> bool {
    let direction = node.effective_style.scroll.direction;
    let scrollbar = node.effective_style.scroll.scrollbar;
    if scrollbar.visibility == ScrollbarVisibilityStyle::Hidden
        || scrollbar.width <= 0.0
        || !scrollbar.thumb_color.is_visible()
    {
        return false;
    }

    let max_x = (node.content_size.width - node.layout.width()).max(0.0);
    let max_y = (node.content_size.height - node.layout.height()).max(0.0);
    (direction.allows_vertical() && should_paint_scrollbar(scrollbar, max_y))
        || (direction.allows_horizontal() && should_paint_scrollbar(scrollbar, max_x))
}

fn render_scrollbars_in_rect(node: NodeView<'_>, rect: Bounds, writer: &mut RenderTreeWriter<'_>) {
    let direction = node.effective_style.scroll.direction;
    let scrollbar = node.effective_style.scroll.scrollbar;
    if scrollbar.visibility == ScrollbarVisibilityStyle::Hidden || scrollbar.width <= 0.0 {
        return;
    }

    let max_x = (node.content_size.width - node.layout.width()).max(0.0);
    let max_y = (node.content_size.height - node.layout.height()).max(0.0);

    if direction.allows_vertical()
        && should_paint_scrollbar(scrollbar, max_y)
        && scrollbar.thumb_color.is_visible()
    {
        let track = vertical_scrollbar_track(rect, scrollbar.width);
        render_scrollbar_part(track, scrollbar.track_color, scrollbar.radius, writer);

        if max_y > 0.0 {
            let ratio = (node.layout.height() / node.content_size.height).clamp(0.0, 1.0);
            let thumb_height = (track.height() * ratio)
                .max(scrollbar.width * 2.0)
                .min(track.height());
            let travel = (track.height() - thumb_height).max(0.0);
            let top = track.y() + travel * (node.scroll_offset.y / max_y);
            render_scrollbar_part(
                Bounds::from_origin_size((track.x(), top), (track.width(), thumb_height)),
                scrollbar.thumb_color,
                scrollbar.radius,
                writer,
            );
        }
    }

    if direction.allows_horizontal()
        && should_paint_scrollbar(scrollbar, max_x)
        && scrollbar.thumb_color.is_visible()
    {
        let track = horizontal_scrollbar_track(rect, scrollbar.width);
        render_scrollbar_part(track, scrollbar.track_color, scrollbar.radius, writer);

        if max_x > 0.0 {
            let ratio = (node.layout.width() / node.content_size.width).clamp(0.0, 1.0);
            let thumb_width = (track.width() * ratio)
                .max(scrollbar.width * 2.0)
                .min(track.width());
            let travel = (track.width() - thumb_width).max(0.0);
            let left = track.x() + travel * (node.scroll_offset.x / max_x);
            render_scrollbar_part(
                Bounds::from_origin_size((left, track.y()), (thumb_width, track.height())),
                scrollbar.thumb_color,
                scrollbar.radius,
                writer,
            );
        }
    }
}

fn should_paint_scrollbar(scrollbar: ComputedScrollbarStyle, max_offset: f32) -> bool {
    match scrollbar.visibility {
        ScrollbarVisibilityStyle::Auto => max_offset > 0.0,
        ScrollbarVisibilityStyle::Always => true,
        ScrollbarVisibilityStyle::Hidden => false,
    }
}

fn vertical_scrollbar_track(rect: Bounds, width: f32) -> Bounds {
    Bounds::from_origin_size(
        Point::new(rect.min.x + (rect.width() - width).max(0.0), rect.y()),
        (width.min(rect.width()), rect.height()),
    )
}

fn horizontal_scrollbar_track(rect: Bounds, width: f32) -> Bounds {
    Bounds::from_origin_size(
        (rect.x(), rect.y() + (rect.height() - width).max(0.0)),
        (rect.width(), width.min(rect.height())),
    )
}

fn render_scrollbar_part(
    rect: Bounds,
    color: ComputedColorStyle,
    radius: f32,
    writer: &mut RenderTreeWriter<'_>,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 || !color.is_visible() {
        return;
    }

    let shape = if radius > 0.0 {
        Shape::RoundedRect(radius)
    } else {
        Shape::Rect
    };
    writer
        .primitive(Primitive::Shape(ShapePrimitive {
            bounds: rect,
            shape,
            fill: Some(color),
            stroke: None,
            shadow: None,
        }))
        .expect("scrollbar render tree must remain valid");
}
