use crate::animation::AnimableStyle;
use crate::core::{Point, Rect, Size};
use crate::event_system::callbacks::{CallbackHandleSet, CallbackStore, EventHandlers};
use crate::event_system::{self, EventState, translator::EventTranslator};
use crate::fiber::Key;
use crate::focus::{FocusHandle, FocusManager};
use crate::layout::{computed_style_for_widget, taffy_style_for_widget};
use crate::render::{
    BuiltFrame, ClipShape, DirtySnapshot, FrameBuildError, FrameBuilder, FrameProperties,
    FramePropertiesSnapshot, HostRenderBinding, LayerDescriptor, Primitive, RenderNodeId,
    RenderScene, RenderTreeWriter, SceneCompileError, SceneCompiler, SceneError, Shape,
    ShapePrimitive,
};
use crate::text::{TextHost, TextLayoutSlot};
use crate::widgets::{WidgetI, WidgetType, Widgets, canvas_text_slot};
use slotmap::SlotMap;
use std::collections::HashMap;
use std::time::Duration;
use taffy::prelude as tf;
use xui_animation::{Animatable, Timeline, Transition};
use xui_interface::events::RawEvent;
use xui_interface::{
    AccessibilityProperties, Affine, ComputedColorStyle, ComputedScrollStyle,
    ComputedScrollbarStyle, ComputedStyle, EventResult, FocusProperties, Focusability, NodeId,
    NodeLifecycleEvent, ScrollbarVisibilityStyle, StyleDiffFlags, TextBackend,
    TextLayoutConstraints, TextLayoutInput, Theme, VectorCommand, WidgetState, WidgetUpdateFlags,
};

pub enum WidgetContext {
    Text(NodeId),
    Image(Size<f32>),
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct HostWorkFlags: u8 {
        const RECALC_STYLE = 1 << 0;
        const RECALC_STYLE_SUBTREE = 1 << 1;
        const RECALC_LAYOUT = 1 << 2;
        const REBUILD_PAINT = 1 << 3;
        const SHAPE_CHANGE = 1 << 4;
        const SYNC_TREE = 1 << 5;
        const SYNC_STATE_CHANGE = 1 << 6;
        const SYNC_RENDER = 1 << 7;
    }
}

impl HostWorkFlags {
    fn from_widget_update(flags: WidgetUpdateFlags) -> Self {
        let mut work = Self::empty();
        if flags.intersects(WidgetUpdateFlags::STYLE_TARGET) {
            work |= Self::RECALC_STYLE | Self::RECALC_LAYOUT;
        }
        if flags.intersects(WidgetUpdateFlags::LAYOUT_INPUT) {
            work |= Self::RECALC_LAYOUT;
        }
        if flags.intersects(WidgetUpdateFlags::PAINT_OUTPUT) {
            work |= Self::REBUILD_PAINT;
        }
        if flags.intersects(WidgetUpdateFlags::TREE) {
            work |=
                Self::SYNC_TREE | Self::RECALC_STYLE | Self::RECALC_LAYOUT | Self::REBUILD_PAINT;
        }

        if flags.intersects(WidgetUpdateFlags::TEXT_SHAPE) {
            work |= Self::SHAPE_CHANGE;
        }

        if flags.intersects(WidgetUpdateFlags::STATE_CHANGE) {
            work |= Self::SYNC_STATE_CHANGE;
        }
        work
    }

    fn from_style_diff(flags: StyleDiffFlags) -> Self {
        let mut work = Self::empty();
        if flags.intersects(StyleDiffFlags::TEXT) {
            work |= Self::REBUILD_PAINT | Self::RECALC_STYLE_SUBTREE;
        }
        if flags.intersects(StyleDiffFlags::LAYOUT) {
            work |= Self::RECALC_LAYOUT | Self::REBUILD_PAINT;
        }
        if flags.intersects(StyleDiffFlags::PAINT) {
            work |= Self::REBUILD_PAINT;
        }
        if flags.intersects(StyleDiffFlags::SCROLL) {
            work |= Self::RECALC_LAYOUT | Self::REBUILD_PAINT;
        }
        if flags.intersects(StyleDiffFlags::EFFECT) {
            work |= Self::SYNC_RENDER;
        }
        work
    }
}

struct ActiveStyleTransition {
    timeline: Timeline,
    from: AnimableStyle,
    to: AnimableStyle,
}

impl ActiveStyleTransition {
    fn new(
        transition: Transition,
        from_style: &ComputedStyle,
        target_style: &ComputedStyle,
    ) -> Option<Self> {
        let (from, to) = AnimableStyle::diff(from_style, target_style);
        if !to.has_properties() {
            return None;
        }

        Some(Self {
            timeline: Timeline::new(transition),
            from,
            to,
        })
    }

    fn tick(&mut self, delta: Duration, node: &mut Node, theme: &Theme) -> bool {
        let progress = self.timeline.tick(delta);
        if progress.completed {
            node.effective_style = node.target_style.clone();
            return true;
        }

        node.effective_style = node.target_style.clone();
        let interpolated = AnimableStyle::interpolate(&self.from, &self.to, progress.eased);
        interpolated.apply_to_computed(&mut node.effective_style, theme);
        false
    }
}

pub struct Node {
    pub id: NodeId,
    pub node_type: WidgetType,
    pub key: Option<Key>,
    // Link
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    // Layout
    pub taffy_node: tf::NodeId,
    pub position: usize,
    // Local
    pub layout: Rect,
    pub previous_layout: Rect,

    // World Bound
    pub world_origin: Point,

    pub content_size: Size<f32>,
    pub scroll_offset: Point,
    work: HostWorkFlags,
    subtree_work: HostWorkFlags,
    pub old_props_hash: u64,
    pub new_props_hash: u64,
    // Style
    pub target_style: ComputedStyle,
    pub effective_style: ComputedStyle,
    style_initialized: bool,
    // State
    pub state: WidgetState,
    pub(crate) state_before_change: Option<WidgetState>,
    // Rendering is retained by UiArena::render_scene.
    pub widget: WidgetI,
    pub event_callbacks: CallbackHandleSet,
    pub shortcut_bindings: Vec<xui_interface::ShortcutBinding>,
    pub focus: FocusProperties,
    pub accessibility: AccessibilityProperties,
    pub(crate) focus_handle: Option<FocusHandle>,
}

impl Node {
    fn new(
        id: NodeId,
        key: Option<Key>,
        position: usize,
        props_hash: u64,
        target_style: ComputedStyle,
        widget: WidgetI,
        event_callbacks: CallbackHandleSet,
        shortcut_bindings: Vec<xui_interface::ShortcutBinding>,
        focus: FocusProperties,
        accessibility: AccessibilityProperties,
        focus_handle: Option<FocusHandle>,
        taffy_node: tf::NodeId,
    ) -> Self {
        let node_type = widget.node_type();

        Self {
            id,
            parent: None,
            children: Vec::new(),
            taffy_node,
            node_type,
            key,
            position,
            layout: Rect::ZERO,
            previous_layout: Rect::ZERO,
            content_size: Size::<f32>::ZERO,
            scroll_offset: Point::new(0.0, 0.0),
            work: HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::REBUILD_PAINT,
            subtree_work: HostWorkFlags::empty(),
            old_props_hash: 0,
            new_props_hash: props_hash,
            effective_style: target_style.clone(),
            target_style,
            style_initialized: false,
            // state
            state: WidgetState::default(),
            state_before_change: None,
            world_origin: Point::zero(),
            widget,
            event_callbacks,
            shortcut_bindings,
            focus,
            accessibility,
            focus_handle,
        }
    }

    pub fn is_focusable(&self) -> bool {
        match self.focus.focusability {
            Focusability::Focusable => true,
            Focusability::NotFocusable => false,
            Focusability::Auto => {
                self.focus.tab_index.is_some()
                    || matches!(self.node_type, WidgetType::Button | WidgetType::TextInput)
                    || self.event_callbacks.has_focus_callbacks()
            }
        }
    }

    pub fn is_sequentially_focusable(&self) -> bool {
        self.is_focusable() && self.focus.tab_index.is_none_or(|index| index >= 0)
    }

    #[inline(always)]
    fn scroll_style(&self) -> &ComputedScrollStyle {
        &self.target_style.scroll
    }

    #[inline(always)]
    fn visual_bounds(&self) -> Rect {
        Rect::new(
            self.world_origin.x - self.scroll_offset.x,
            self.world_origin.y - self.scroll_offset.y,
            self.layout.width,
            self.layout.height,
        )
    }
}

#[derive(Default)]
struct UiState {
    animation_driver: AnimationDriver,
    layout_dirty_list: Vec<NodeId>,
    shape_dirty_list: Vec<NodeId>,
    style_subtree_dirty_list: Vec<NodeId>,
    style_dirty_list: Vec<NodeId>,
    state_change_dirty_list: Vec<NodeId>,
}

impl UiState {
    #[inline]
    fn mark_layout_dirty(&mut self, id: NodeId) {
        self.layout_dirty_list.push(id);
    }

    #[inline]
    fn start_style_transition(
        &mut self,
        id: NodeId,
        transition: Transition,
        from_style: &ComputedStyle,
        target_style: &ComputedStyle,
    ) -> bool {
        self.animation_driver
            .start_node(id, transition, from_style, target_style)
    }

    #[inline]
    fn mark_style_subtree_dirty(&mut self, id: NodeId) {
        self.style_subtree_dirty_list.push(id);
    }

    #[inline]
    fn mark_state_change_dirty(&mut self, id: NodeId) {
        self.state_change_dirty_list.push(id);
    }

    #[inline]
    fn mark_style_dirty(&mut self, id: NodeId) {
        self.style_dirty_list.push(id);
    }

    #[inline]
    fn mark_shape_dirty(&mut self, id: NodeId) {
        self.shape_dirty_list.push(id);
    }

    fn drain_subtree_dirty_list(&mut self) -> Vec<NodeId> {
        let mut list = std::mem::take(&mut self.style_subtree_dirty_list);
        list.sort();
        list.dedup();
        list
    }

    fn drain_style_dirty_list(&mut self) -> Vec<NodeId> {
        let mut list = std::mem::take(&mut self.style_dirty_list);
        list.sort();
        list.dedup();
        list
    }

    fn drain_state_change_dirty_list(&mut self) -> Vec<NodeId> {
        let mut list = std::mem::take(&mut self.state_change_dirty_list);
        list.sort();
        list.dedup();
        list
    }

    fn drain_shape_dirty_list(&mut self) -> Vec<NodeId> {
        let mut list = std::mem::take(&mut self.shape_dirty_list);
        list.sort();
        list.dedup();
        list
    }
}

#[derive(Default)]
struct AnimationDriver {
    nodes: HashMap<NodeId, ActiveStyleTransition>,
}

impl AnimationDriver {
    fn start_node(
        &mut self,
        id: NodeId,
        transition: Transition,
        from_style: &ComputedStyle,
        target_style: &ComputedStyle,
    ) -> bool {
        let Some(animation) = ActiveStyleTransition::new(transition, from_style, target_style)
        else {
            self.nodes.remove(&id);
            return false;
        };

        self.nodes.insert(id, animation);
        true
    }

    fn remove_node(&mut self, id: NodeId) {
        self.nodes.remove(&id);
    }

    fn is_running(&self) -> bool {
        !self.nodes.is_empty()
    }

    fn take_nodes(&mut self) -> HashMap<NodeId, ActiveStyleTransition> {
        std::mem::take(&mut self.nodes)
    }

    fn set_nodes(&mut self, nodes: HashMap<NodeId, ActiveStyleTransition>) {
        self.nodes = nodes;
    }
}

pub struct UiArena {
    pub(crate) nodes: SlotMap<NodeId, Node>,
    taffy: tf::TaffyTree<WidgetContext>,
    root: NodeId,
    node_lifecycle_events: Vec<NodeLifecycleEvent>,
    pub event_state: EventState,
    focus_manager: FocusManager,
    pub(crate) event_callbacks: CallbackStore,
    theme: Theme,
    pub update_visits: usize,
    pub layout_passes: usize,
    pub repaint_passes: usize,
    default_style: ComputedStyle,
    ui_state: UiState,
    render_scene: RenderScene,
    scene_compiler: SceneCompiler,
    frame_builder: FrameBuilder,
    frame_properties: FrameProperties,
    last_presented_viewport: Option<Rect>,
}

pub struct RenderFrame {
    pub built: BuiltFrame,
    pub dirty_snapshot: DirtySnapshot,
    pub properties_snapshot: FramePropertiesSnapshot,
    pub viewport: Rect,
}

#[derive(Debug)]
pub enum RenderFrameError {
    Compile(SceneCompileError),
    Build(FrameBuildError),
}

impl std::fmt::Display for RenderFrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compile(error) => error.fmt(formatter),
            Self::Build(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RenderFrameError {}

impl From<SceneCompileError> for RenderFrameError {
    fn from(value: SceneCompileError) -> Self {
        Self::Compile(value)
    }
}

impl From<FrameBuildError> for RenderFrameError {
    fn from(value: FrameBuildError) -> Self {
        Self::Build(value)
    }
}

impl UiArena {
    pub fn new() -> Self {
        let mut taffy = tf::TaffyTree::new();
        let theme = Theme::default();
        let root_widget = crate::widgets::root_widget();
        let root_parent_style = ComputedStyle::initial(&theme);
        let root_computed_style = computed_style_for_widget(
            &root_widget,
            &root_parent_style,
            &theme,
            WidgetState::empty(),
        );
        let root_taffy_style = taffy_style_for_widget(
            &root_widget,
            &root_parent_style,
            &root_computed_style,
            false,
        );
        let taffy_root = taffy
            .new_leaf(root_taffy_style)
            .expect("failed to create taffy root");
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert_with_key(|id| {
            Node::new(
                id,
                None,
                0,
                0,
                root_computed_style,
                root_widget,
                CallbackHandleSet::default(),
                Vec::new(),
                FocusProperties::default(),
                AccessibilityProperties::default(),
                None,
                taffy_root,
            )
        });
        nodes[root].style_initialized = true;
        let default_style = ComputedStyle::initial(&theme);
        let mut arena = Self {
            nodes,
            taffy,
            root,
            node_lifecycle_events: Vec::new(),
            event_state: EventState::default(),
            focus_manager: FocusManager::default(),
            event_callbacks: CallbackStore::default(),
            theme,
            update_visits: 0,
            layout_passes: 0,
            repaint_passes: 0,
            ui_state: UiState::default(),
            render_scene: RenderScene::new(),
            scene_compiler: SceneCompiler::new(),
            frame_builder: FrameBuilder::new(),
            frame_properties: FrameProperties::default(),
            last_presented_viewport: None,
            default_style,
        };
        arena
            .create_host_render_binding(root)
            .expect("failed to create root render binding");
        arena
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id].children
    }

    pub fn render_scene(&self) -> &RenderScene {
        &self.render_scene
    }

    pub fn compiled_scene(&self) -> Option<&crate::render::CompiledScene> {
        self.scene_compiler.compiled_scene()
    }

    pub fn frame_properties(&self) -> &FrameProperties {
        &self.frame_properties
    }

    pub fn frame_properties_mut(&mut self) -> &mut FrameProperties {
        &mut self.frame_properties
    }

    fn create_host_render_binding(&mut self, host: NodeId) -> Result<(), SceneError> {
        let root = self.render_scene.insert_transform(Affine::IDENTITY);
        let contents = self.render_scene.insert_group();
        let paint = self.render_scene.insert_group();
        self.render_scene.append_child(contents, paint)?;

        self.render_scene.set_child(root, Some(contents))?;
        self.render_scene.bind_host(
            host,
            HostRenderBinding::scaffold(root, contents, paint, None, None, None),
        )?;
        if host == self.root {
            self.render_scene
                .append_child(self.render_scene.root(), root)?;
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
        let work = self.nodes[id].work;
        let subtree_work = self.nodes[id].subtree_work;
        if !work.intersects(relevant) && !subtree_work.intersects(relevant) {
            return Ok(());
        }
        if work.intersects(relevant) {
            self.sync_host_render_node(id)?;
        }
        let children = self.nodes[id].children.clone();
        for child in children {
            self.sync_render_dirty_subtree(child)?;
        }
        Ok(())
    }

    fn sync_host_render_node(&mut self, id: NodeId) -> Result<(), SceneError> {
        let mut binding = self
            .render_scene
            .host_binding(id)
            .copied()
            .expect("host render binding missing");

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
            let node = &self.nodes[id];
            let viewport = Rect::new(0.0, 0.0, node.layout.width, node.layout.height);
            let needs_scroll = node.scroll_style().is_scrollable();
            let needs_clip = node.target_style.paint.clip || needs_scroll;
            let clip_shape = needs_clip.then(|| {
                if node.target_style.paint.border_radius > 0.0 {
                    ClipShape::RoundedRect {
                        rect: viewport,
                        radius: node.target_style.paint.border_radius,
                    }
                } else {
                    ClipShape::Rect(viewport)
                }
            });

            (
                node.layout.origin(),
                viewport,
                node.scroll_offset,
                node.children.clone(),
                needs_scroll,
                needs_scrollbar_overlay(node),
                clip_shape,
                layer_descriptor_from_style(&node.target_style, viewport),
            )
        };

        // Reconcile the fixed host scaffold.
        self.render_scene.update_transform(
            binding.transform,
            Affine::translate(local_origin.x, local_origin.y),
        )?;

        self.update_wrappers(id, &mut binding, clip_shape, layer_descriptor)?;

        // The children group is a stable container once it has been created.
        if !host_children.is_empty() && binding.children.is_none() {
            let children = self.render_scene.insert_group();
            // `paint` is always at index 0. Insert the child branch at index 1
            // so a scrollbar overlay remains last.
            self.render_scene
                .insert_child(binding.contents, 1, children)?;
            binding.children = Some(children);
        }

        // Scroll transforms are transient and exist only while scrolling is enabled.
        let removed_scroll_transform = if needs_scroll {
            if binding.scroll_transform.is_none() {
                if let Some(children) = binding.children {
                    self.render_scene.detach(children)?;
                    let scroll_transform = self.render_scene.insert_transform(Affine::IDENTITY);
                    self.render_scene
                        .set_child(scroll_transform, Some(children))?;
                    self.render_scene
                        .insert_child(binding.contents, 1, scroll_transform)?;
                    binding.scroll_transform = Some(scroll_transform);
                }
            }
            None
        } else if let Some(scroll_transform) = binding.scroll_transform.take() {
            if let Some(children) = binding.children {
                self.render_scene.set_child(scroll_transform, None)?;
                self.render_scene.detach(scroll_transform)?;
                self.render_scene
                    .insert_child(binding.contents, 1, children)?;
            }
            self.frame_properties.remove_source(scroll_transform);
            Some(scroll_transform)
        } else {
            None
        };

        // Scrollbar overlays are also transient.
        let removed_overlay = if needs_overlay {
            if binding.overlay.is_none() {
                let overlay = self.render_scene.insert_group();
                self.render_scene.append_child(binding.contents, overlay)?;
                binding.overlay = Some(overlay);
            }
            None
        } else if let Some(overlay) = binding.overlay.take() {
            self.render_scene.detach(overlay)?;
            Some(overlay)
        } else {
            None
        };

        // Remove transient nodes only after the host binding stops referencing them.
        *self
            .render_scene
            .host_binding_mut(id)
            .expect("binding disappeared") = binding;

        if let Some(scroll_transform) = removed_scroll_transform {
            self.render_scene.remove_subtree(scroll_transform)?;
        }
        if let Some(overlay) = removed_overlay {
            self.render_scene.remove_subtree(overlay)?;
        }

        // Scroll offsets remain dynamic frame properties while scrolling is active.
        if let Some(scroll_transform) = binding.scroll_transform {
            self.frame_properties
                .set_transform(scroll_transform, Affine::translate(-scroll.x, -scroll.y));
        }

        // Reconcile host children in declaration/paint order.
        if let Some(children_binding) = binding.children {
            let children_match = self
                .render_scene
                .children(children_binding)?
                .iter()
                .copied()
                .eq(host_children.iter().map(|host_child| {
                    self.render_scene
                        .host_binding(*host_child)
                        .expect("child host render binding missing")
                        .root
                }));

            if !children_match {
                let current = self.render_scene.children(children_binding)?.to_vec();

                for child_root in current {
                    self.render_scene.detach(child_root)?;
                }

                for host_child in &host_children {
                    let child_root = self
                        .render_scene
                        .host_binding(*host_child)
                        .expect("child host render binding missing")
                        .root;

                    self.render_scene.detach(child_root)?;
                    self.render_scene
                        .append_child(children_binding, child_root)?;
                }
            }
        }

        if let Some(overlay) = binding.overlay {
            let node = &self.nodes[id];
            let mut writer = RenderTreeWriter::new(&mut self.render_scene, overlay);
            render_scrollbars_in_rect(node, viewport, &mut writer);
            writer.finish()?;
        }

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
                    self.render_scene.update_clip(clip, shape)?;
                } else {
                    binding.clip = Some(self.render_scene.insert_clip(shape));
                }
                None
            }
            None => binding.clip.take(),
        };

        let removed_layer = match layer_descriptor {
            Some(descriptor) => {
                if let Some(layer) = binding.layer {
                    self.render_scene
                        .update_layer_descriptor(layer, descriptor)?;
                } else {
                    binding.layer = Some(self.render_scene.insert_layer(descriptor));
                }
                None
            }
            None => binding.layer.take(),
        };

        if !topology_changed {
            return Ok(());
        }

        // Break the old root -> clip -> layer -> contents chain.
        self.render_scene.set_child(binding.root, None)?;
        if let Some(clip) = old_clip {
            self.render_scene.set_child(clip, None)?;
        }
        if let Some(layer) = old_layer {
            self.render_scene.set_child(layer, None)?;
        }

        // Rebuild the wrapper chain from inside out.
        let mut child = binding.contents;
        if let Some(layer) = binding.layer {
            self.render_scene.set_child(layer, Some(child))?;
            child = layer;
        }
        if let Some(clip) = binding.clip {
            self.render_scene.set_child(clip, Some(child))?;
            child = clip;
        }
        self.render_scene.set_child(binding.root, Some(child))?;

        // Drop binding references before removing obsolete wrapper subtrees.
        let stored = self
            .render_scene
            .host_binding_mut(id)
            .expect("binding disappeared before wrapper removal");

        stored.clip = binding.clip;
        stored.layer = binding.layer;

        if let Some(clip) = removed_clip {
            self.render_scene.remove_subtree(clip)?;
        }

        if let Some(layer) = removed_layer {
            self.render_scene.remove_subtree(layer)?;
        }

        Ok(())
    }

    pub fn focused_node(&self) -> Option<NodeId> {
        self.focus_manager.focused()
    }

    pub fn focus_manager(&self) -> &FocusManager {
        &self.focus_manager
    }
    pub(crate) fn focus_manager_mut(&mut self) -> &mut FocusManager {
        &mut self.focus_manager
    }

    pub(crate) fn resolve_local_shortcut(
        &self,
        event: &xui_interface::RawKeyboard,
    ) -> Option<(NodeId, xui_interface::ShortcutBinding)> {
        let mut current = self
            .focused_node()
            .or_else(|| self.children(self.root).first().copied())
            .or(Some(self.root));
        while let Some(id) = current {
            let node = self.nodes.get(id)?;
            if let Some(binding) = node
                .shortcut_bindings
                .iter()
                .rev()
                .find(|binding| binding.shortcut.matches(event))
            {
                return Some((id, *binding));
            }
            current = node.parent;
        }
        None
    }

    pub fn hovered_node(&self) -> Option<NodeId> {
        self.event_state.hovered()
    }

    pub fn pointer_capture_node(&self) -> Option<NodeId> {
        self.event_state.pointer_capture()
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
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
        &self.event_state
    }

    pub(crate) fn event_state_mut(&mut self) -> &mut EventState {
        &mut self.event_state
    }

    pub(crate) fn event_callbacks(&mut self) -> &mut CallbackStore {
        &mut self.event_callbacks
    }

    #[cfg(test)]
    pub(crate) fn set_event_handlers(&mut self, id: NodeId, mut event_handlers: EventHandlers) {
        let Some(current) = self.nodes.get(id).map(|node| node.event_callbacks) else {
            return;
        };
        let shortcut_bindings = std::mem::take(&mut event_handlers.shortcuts);
        let focus = event_handlers.focus;
        let focus_handle = event_handlers.focus_handle.take();
        let accessibility = std::mem::take(&mut event_handlers.accessibility);
        let event_callbacks = self.event_callbacks.update_set(current, event_handlers);
        if let Some(node) = self.nodes.get_mut(id) {
            if let Some(handle) = node.focus_handle.take() {
                handle.unbind(id);
            }
            node.event_callbacks = event_callbacks;
            node.shortcut_bindings = shortcut_bindings;
            node.focus = focus;
            node.accessibility = accessibility;
            node.focus_handle = focus_handle;
            if let Some(handle) = node.focus_handle.as_ref() {
                handle.bind(id);
            }
        }
    }

    pub fn create_node(
        &mut self,
        key: Option<Key>,
        props_hash: u64,
        widget: WidgetI,
        mut event_handlers: EventHandlers,
    ) -> NodeId {
        let taffy_node = self
            .taffy
            .new_leaf(tf::Style::default())
            .expect("failed to create taffy node");
        let shortcut_bindings = std::mem::take(&mut event_handlers.shortcuts);
        let focus = event_handlers.focus;
        let focus_handle = event_handlers.focus_handle.take();
        let accessibility = std::mem::take(&mut event_handlers.accessibility);
        let event_callbacks = self
            .event_callbacks
            .update_set(CallbackHandleSet::default(), event_handlers);

        let id = self.nodes.insert_with_key(|id| {
            Node::new(
                id,
                key,
                0,
                props_hash,
                self.default_style.clone(),
                widget,
                event_callbacks,
                shortcut_bindings,
                focus,
                accessibility,
                focus_handle,
                taffy_node,
            )
        });
        if let Some(handle) = self.nodes[id].focus_handle.as_ref() {
            handle.bind(id);
        }
        self.node_lifecycle_events
            .push(NodeLifecycleEvent::Created(id));
        self.create_host_render_binding(id)
            .expect("failed to create host render binding");
        self.refresh_taffy_context(id);
        let mut work = HostWorkFlags::RECALC_STYLE
            | HostWorkFlags::RECALC_LAYOUT
            | HostWorkFlags::REBUILD_PAINT;
        if self.nodes[id].node_type == WidgetType::Canvas {
            work |= HostWorkFlags::SHAPE_CHANGE;
        }
        self.mark_work(id, work);

        id
    }

    pub fn to_local(&self, node_id: NodeId, viewport_pos: Point) -> Option<Point> {
        let node = self.node(node_id)?;

        if let Some(parent_id) = node.parent {
            let parent = self.node(parent_id)?;

            let parent_local = self.to_local(parent_id, viewport_pos)?;
            let parent_content_pos = parent_local + parent.scroll_offset;

            let node_pos = Point {
                x: node.layout.x,
                y: node.layout.y,
            };

            Some(parent_content_pos - node_pos)
        } else {
            let root_pos = Point {
                x: node.layout.x,
                y: node.layout.y,
            };

            Some(viewport_pos - root_pos)
        }
    }

    pub fn to_content_local(&self, node_id: NodeId, viewport_pos: Point) -> Option<Point> {
        let node = self.node(node_id)?;

        let local = self.to_local(node_id, viewport_pos)?;

        Some(local + node.scroll_offset)
    }

    pub fn attach(&mut self, parent: NodeId, child: NodeId) {
        let old_parent = self.nodes[child].parent;
        let old_position = self.nodes[child].position;
        if let Some(old_parent) = old_parent.filter(|old_parent| *old_parent != parent) {
            self.nodes[old_parent]
                .children
                .retain(|candidate| *candidate != child);
            self.sync_taffy_children(old_parent);
            self.reindex_children(old_parent);
            self.mark_work(
                old_parent,
                HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }
        let parent_taffy = self.nodes[parent].taffy_node;
        let child_taffy = self.nodes[child].taffy_node;
        self.nodes[child].parent = Some(parent);
        self.mark_work(
            child,
            HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
        );
        if !self.nodes[parent].children.contains(&child) {
            self.nodes[parent].children.push(child);
        }
        let taffy_children: Vec<_> = self.nodes[parent]
            .children
            .iter()
            .map(|id| self.nodes[*id].taffy_node)
            .collect();
        self.taffy
            .set_children(parent_taffy, &taffy_children)
            .expect("failed to attach taffy child");
        self.reindex_children(parent);
        let new_position = self.nodes[child].position;
        self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
        let _ = child_taffy;
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if parent == child || !self.nodes.contains_key(parent) || !self.nodes.contains_key(child) {
            return;
        }

        let old_parent = self.nodes[child].parent;
        let old_position = self.nodes[child].position;

        if let Some(old_parent) = old_parent {
            self.detach_child_from_current_parent(child, old_parent);
        }

        if !self.nodes[parent].children.contains(&child) {
            self.nodes[parent].children.push(child);
        }
        self.nodes[child].parent = Some(parent);
        if old_parent != Some(parent) {
            self.mark_work(
                child,
                HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }
        self.sync_taffy_children(parent);
        self.reindex_children(parent);

        let new_position = self.nodes[child].position;
        self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
    }

    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) {
        if child == before {
            return;
        }
        if parent == child || !self.nodes.contains_key(parent) || !self.nodes.contains_key(child) {
            return;
        }
        if !self.nodes.contains_key(before)
            || self.nodes.get(before).and_then(|node| node.parent) != Some(parent)
        {
            self.append_child(parent, child);
            return;
        }

        let old_parent = self.nodes[child].parent;
        let old_position = self.nodes[child].position;

        if let Some(old_parent) = old_parent {
            self.detach_child_from_current_parent(child, old_parent);
        }

        let Some(index) = self.nodes[parent]
            .children
            .iter()
            .position(|candidate| *candidate == before)
        else {
            panic!("ERROR");
        };

        self.nodes[parent].children.insert(index, child);
        self.nodes[child].parent = Some(parent);
        if old_parent != Some(parent) {
            self.mark_work(
                child,
                HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }
        self.sync_taffy_children(parent);
        self.reindex_children(parent);

        let new_position = self.nodes[child].position;
        self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
    }

    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if !self.nodes.contains_key(parent) || !self.nodes.contains_key(child) {
            return;
        }
        if self.nodes[child].parent != Some(parent) {
            return;
        }

        let old_position = self.nodes[child].position;
        self.nodes[parent]
            .children
            .retain(|candidate| *candidate != child);
        self.nodes[child].parent = None;
        self.mark_work(
            child,
            HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
        );
        self.nodes[child].position = 0;
        self.sync_taffy_children(parent);
        self.reindex_children(parent);
        self.record_node_move(child, Some(parent), None, old_position, 0);
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
    }

    pub fn remove_from_parent(&mut self, child: NodeId) {
        let Some(parent) = self.nodes.get(child).and_then(|node| node.parent) else {
            return;
        };
        self.remove_child(parent, child);
    }

    pub fn clear_children(&mut self, parent: NodeId) {
        let children = self.nodes[parent].children.clone();
        for child in children {
            self.remove_subtree(child);
        }
        self.nodes[parent].children.clear();
        let parent_taffy = self.nodes[parent].taffy_node;
        self.taffy
            .set_children(parent_taffy, &[])
            .expect("failed to clear taffy children");
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
    }

    pub fn remove_subtree(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) || id == self.root {
            return;
        }

        let children = self.nodes[id].children.clone();
        for child in children {
            self.remove_subtree(child);
        }

        if let Some(parent) = self.nodes[id].parent {
            self.nodes[parent].children.retain(|child| *child != id);
            let parent_taffy = self.nodes[parent].taffy_node;
            let taffy_children: Vec<_> = self.nodes[parent]
                .children
                .iter()
                .map(|child| self.nodes[*child].taffy_node)
                .collect();
            let _ = self.taffy.set_children(parent_taffy, &taffy_children);
            self.reindex_children(parent);
            self.mark_work(
                parent,
                HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }

        self.event_state.clear_node(id);
        self.focus_manager.clear_node(id);
        if let Some(handle) = self.nodes[id].focus_handle.as_ref() {
            handle.unbind(id);
        }
        self.event_callbacks
            .clear_set(self.nodes[id].event_callbacks);
        self.ui_state.animation_driver.remove_node(id);

        let _ = self.taffy.remove(self.nodes[id].taffy_node);
        if let Some(binding) = self.render_scene.host_binding(id).cloned() {
            self.render_scene
                .remove_subtree(binding.root)
                .expect("failed to remove host render subtree");
        }
        self.nodes.remove(id);
        self.node_lifecycle_events
            .push(NodeLifecycleEvent::Removed(id));
    }

    pub fn drain_node_lifecycle_events(&mut self) -> Vec<NodeLifecycleEvent> {
        std::mem::take(&mut self.node_lifecycle_events)
    }

    pub fn mark_dirty(&mut self, id: NodeId, flags: WidgetUpdateFlags) {
        self.mark_work(id, HostWorkFlags::from_widget_update(flags));
    }

    fn mark_work(&mut self, id: NodeId, flags: HostWorkFlags) {
        if flags.is_empty() || !self.nodes.contains_key(id) {
            return;
        }
        {
            let node = self.nodes.get_mut(id).expect("checked node existence");
            node.work |= flags;
        }

        if flags.intersects(HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::SYNC_TREE) {
            // Host dirtiness alone is not enough: Taffy keeps intrinsic and
            // final layout entries per node. A resize must invalidate the
            // affected Taffy node too, otherwise a min-content text probe can
            // survive as the child's apparent final layout.
            let taffy_node = self.nodes[id].taffy_node;
            // self.taffy
            //     .mark_dirty(taffy_node)
            //     .expect("failed to invalidate Taffy layout cache");
            self.ui_state.mark_layout_dirty(id);
        }

        if flags.intersects(HostWorkFlags::RECALC_STYLE_SUBTREE) {
            self.ui_state.mark_style_subtree_dirty(id);
        }

        if flags.intersects(HostWorkFlags::RECALC_STYLE) {
            self.ui_state.mark_style_dirty(id);
        }

        if flags.intersects(HostWorkFlags::SYNC_STATE_CHANGE) {
            self.ui_state.mark_state_change_dirty(id);
        }

        if flags.intersects(HostWorkFlags::SHAPE_CHANGE) {
            self.ui_state.mark_shape_dirty(id);
        }

        let mut current = id;
        while let Some(parent) = self.nodes[current].parent {
            self.nodes[parent].subtree_work |= flags;
            current = parent;
        }
    }

    pub fn clear_work(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get_mut(id) {
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
        let children = self.nodes[id].children.clone();
        for child in children {
            self.mark_subtree_layout_dirty(child);
        }
    }

    #[inline(always)]
    pub fn hit_test(&self, point: crate::core::Point) -> Option<NodeId> {
        self.hit_test_from(self.root, point, Point::zero())
    }

    /// Returns a node's layout rectangle in window logical coordinates after
    /// applying scroll offsets from its ancestors.
    pub fn visual_layout(&self, id: NodeId) -> Option<Rect> {
        let node = self.nodes.get(id)?;
        let mut rect = node.layout;
        let mut cursor = node.parent;
        while let Some(parent) = cursor {
            let ancestor = self.nodes.get(parent)?;
            if ancestor.scroll_style().direction.is_scrollable() {
                rect.x -= ancestor.scroll_offset.x;
                rect.y -= ancestor.scroll_offset.y;
            }
            cursor = ancestor.parent;
        }
        Some(rect)
    }

    fn hit_test_from(
        &self,
        id: NodeId,
        point: crate::core::Point,
        scroll_offset: Point,
    ) -> Option<NodeId> {
        let node = self.nodes.get(id)?;
        let visual_layout = node.visual_bounds();
        if !visual_layout.contains(point) {
            return None;
        }
        let node_style = &node.target_style;

        let child_scroll_offset = if node_style.scroll.direction.is_scrollable() {
            Point::new(
                scroll_offset.x + node.scroll_offset.x,
                scroll_offset.y + node.scroll_offset.y,
            )
        } else {
            scroll_offset
        };

        for child in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_from(*child, point, child_scroll_offset) {
                return Some(hit);
            }
        }

        Some(id)
    }

    pub(crate) fn scroll_node_by(&mut self, start: NodeId, delta: Point) -> bool {
        let mut cursor = Some(start);
        while let Some(id) = cursor {
            if self.scroll_single_node_by(id, delta) {
                return true;
            }
            cursor = self.nodes.get(id).and_then(|node| node.parent);
        }
        false
    }

    fn scroll_single_node_by(&mut self, id: NodeId, delta: Point) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };

        let direction = node.scroll_style().direction;
        if !direction.is_scrollable() {
            return false;
        }

        let max_x = if direction.allows_horizontal() {
            (node.content_size.width - node.layout.width).max(0.0)
        } else {
            0.0
        };
        let max_y = if direction.allows_vertical() {
            (node.content_size.height - node.layout.height).max(0.0)
        } else {
            0.0
        };
        if max_x <= 0.0 && max_y <= 0.0 {
            return false;
        }

        let scroll_delta = ergonomic_scroll_delta(delta);
        let next = Point::new(
            (node.scroll_offset.x - scroll_delta.x).clamp(0.0, max_x),
            (node.scroll_offset.y - scroll_delta.y).clamp(0.0, max_y),
        );
        if next == node.scroll_offset {
            return false;
        }

        let node = self.nodes.get_mut(id).expect("checked node existence");
        node.scroll_offset = next;
        self.mark_work(id, HostWorkFlags::SYNC_RENDER);
        true
    }

    pub fn event_path(&self, target: NodeId) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut cursor = Some(target);
        while let Some(id) = cursor {
            path.push(id);
            cursor = self.nodes[id].parent;
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
        if !self.ui_state.animation_driver.is_running() {
            return false;
        }

        let mut changed = false;
        let mut remaining = HashMap::new();
        let active = self.ui_state.animation_driver.take_nodes();

        for (id, mut animation) in active {
            if !self.nodes.contains_key(id) {
                continue;
            }

            let completed = {
                let node = self.nodes.get_mut(id).expect("checked node existence");
                animation.tick(delta, node, &self.theme)
            };

            self.mark_work(id, HostWorkFlags::REBUILD_PAINT);
            changed = true;
            if !completed {
                remaining.insert(id, animation);
            }
        }

        self.ui_state.animation_driver.set_nodes(remaining);
        changed
    }

    pub fn has_running_style_animations(&self) -> bool {
        self.ui_state.animation_driver.is_running()
    }

    pub fn update_tree<T: TextBackend>(&mut self, size: Size<f32>, measurer: &mut TextHost<T>) {
        for node_id in self.ui_state.drain_state_change_dirty_list() {
            self.recompute_node_state(node_id);
        }

        for node_id in self.ui_state.drain_shape_dirty_list() {
            self.recompute_node_text_shape(node_id, measurer);
        }

        // Fiber-style bailout: skip the whole branch when neither this node nor
        // any descendant has scheduled work.
        for node_id in self.ui_state.drain_style_dirty_list() {
            self.recompute_node_style(node_id);
        }

        for node_id in self.ui_state.drain_subtree_dirty_list() {
            self.recompute_subtree_styles(node_id);
        }

        // Recompute layout if needed.
        if self.has_layout_dirty() {
            self.compute_layout(size, measurer);
        }

        self.ui_state.layout_dirty_list.clear();
        self.rebuild_subtree_dirty(self.root);
        self.repaint_dirty_subtree(self.root);
        self.sync_render_scene()
            .expect("host tree must produce a valid render scene");
        self.clear_work_subtree(self.root, HostWorkFlags::all());
    }

    fn recompute_node_state(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }

        let state = self.nodes[id].state;
        let before = self.nodes[id].state_before_change.take().unwrap_or(state);
        let widget = self.nodes[id].widget.clone();

        // Recompute style if the widget's style affects the state change.
        if widget.with_widgets(|w| w.style().affects_state_change(before, state)) {
            self.mark_dirty(id, WidgetUpdateFlags::STYLE_TARGET);
        }
    }

    fn recompute_subtree_styles(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }

        self.recompute_node_style(id);

        let children = self.nodes[id].children.clone();
        for child in children {
            self.recompute_subtree_styles(child);
        }
    }

    fn recompute_node_style(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }

        let widget = self.nodes[id].widget.clone();
        let state = self.nodes[id].state;
        let parent_is_z_stack = self.nodes[id].parent.is_some_and(|parent| {
            self.nodes[parent]
                .widget
                .with_widgets(|widget| matches!(widget, crate::widgets::Widgets::ZStack(_)))
        });
        let parent_style = self.nodes[id]
            .parent
            .and_then(|p| self.node(p))
            .map(|p| &p.target_style)
            .unwrap_or(&self.default_style);

        let computed_style = computed_style_for_widget(&widget, parent_style, &self.theme, state);
        let taffy_style =
            taffy_style_for_widget(&widget, parent_style, &computed_style, parent_is_z_stack);

        let node = self.node(id).unwrap();
        let current_taffy_style = self
            .taffy
            .style(node.taffy_node)
            .expect("Missing taffy node");

        let style_diff = node.target_style.diff(&computed_style);
        let mut work_flags = HostWorkFlags::from_style_diff(style_diff);
        if *current_taffy_style != taffy_style {
            self.taffy
                .set_style(node.taffy_node, taffy_style)
                .expect("failed to update taffy style");
            work_flags |= HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::REBUILD_PAINT;
        }

        let transition = widget.transition();
        if !self.nodes[id].style_initialized {
            let node_mut = self.node_mut(id).unwrap();
            node_mut.target_style = computed_style;
            node_mut.effective_style = node_mut.target_style.clone();
            node_mut.style_initialized = true;
            node_mut.work |= work_flags;
            self.refresh_taffy_context(id);
            return;
        }

        if work_flags.is_empty() {
            return;
        }

        let from_style = self.nodes[id].effective_style.clone();
        let started_transition = transition
            .map(|transition| {
                self.ui_state
                    .start_style_transition(id, transition, &from_style, &computed_style)
            })
            .unwrap_or_else(|| {
                self.ui_state.animation_driver.remove_node(id);
                false
            });
        self.nodes[id].target_style = computed_style;

        if started_transition {
            work_flags |= HostWorkFlags::REBUILD_PAINT;
        } else {
            let effective_style = self.nodes[id].target_style.clone();
            self.nodes[id].effective_style = effective_style;
        }

        self.refresh_taffy_context(id);

        if work_flags.intersects(HostWorkFlags::RECALC_STYLE_SUBTREE) {
            self.ui_state.mark_style_subtree_dirty(id);
        }

        let node_mut = self.node_mut(id).unwrap();
        node_mut.work |= work_flags;
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
        self.nodes
            .values()
            .any(|node| node.work.intersects(Self::layout_work_flags()))
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
        self.nodes.get(id).map(|node| &node.effective_style)
    }

    pub fn repaint_if_needed(&mut self, id: NodeId) {
        let should_repaint = self
            .nodes
            .get(id)
            .is_some_and(|node| node.work.intersects(HostWorkFlags::REBUILD_PAINT));
        if !should_repaint {
            return;
        }

        self.repaint_passes += 1;
        let layout = self.nodes[id].layout;
        let rect = Rect::new(0.0, 0.0, layout.width, layout.height);
        let style = self
            .effective_style(id)
            .expect("checked node existence before repaint")
            .clone();
        let widget = self.nodes[id].widget.clone();
        let paint = self
            .render_scene
            .host_binding(id)
            .expect("host render binding missing")
            .paint;
        let mut writer = RenderTreeWriter::new(&mut self.render_scene, paint);
        widget.render(id, rect, &style, &mut writer);
        writer
            .finish()
            .expect("widget emitted an invalid retained render tree");
    }

    pub fn compute_layout<T: TextBackend>(&mut self, size: Size<f32>, measurer: &mut TextHost<T>) {
        self.layout_passes += 1;
        let root_taffy = self.nodes[self.root].taffy_node;
        self.taffy
            .compute_layout_with_measure(
                root_taffy,
                tf::Size {
                    width: tf::AvailableSpace::Definite(size.width),
                    height: tf::AvailableSpace::Definite(size.height),
                },
                |known_dimensions, available_space, _node_id, node_context, _style| {
                    measure_layout_context(
                        &self.nodes,
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
            .nodes
            .iter()
            .filter_map(|(id, node)| {
                if node.node_type != WidgetType::Text {
                    return None;
                }
                let props = node
                    .widget
                    .with_widgets(|widget| widget.text_layout_props(&node.effective_style))?;
                let input = TextLayoutInput::new(
                    props.text,
                    TextLayoutConstraints::max_width(node.layout.width.max(0.0)),
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
        let taffy_node = self.nodes[id].taffy_node;
        // Pixel-rounded widths can be smaller than the shaped intrinsic width.
        // For CJK text that turns the final character into a second line on
        // alternating resize frames. Preserve Taffy's computed floating-point
        // geometry for text while keeping pixel snapping for other widgets.
        let layout = if self.node_uses_unrounded_layout(id) {
            self.taffy.unrounded_layout(taffy_node)
        } else {
            self.taffy
                .layout(taffy_node)
                .expect("missing taffy layout result")
        };
        let taffy_content_size =
            Size::<f32>::new(layout.content_size.width, layout.content_size.height);

        let rect = Rect::new(
            layout.location.x,
            layout.location.y,
            layout.size.width,
            layout.size.height,
        );

        let world_origin = Point::new(offset_x + layout.location.x, offset_y + layout.location.y);

        let old_rect = self.nodes[id].layout;
        let layout_changed = old_rect != rect || self.nodes[id].world_origin != world_origin;
        let size_changed = old_rect.width != rect.width || old_rect.height != rect.height;
        let (children, mut subtree_work) = {
            let node = &mut self.nodes[id];
            let should_sync_children = layout_changed
                || node.work.intersects(Self::layout_work_flags())
                || node.subtree_work.intersects(Self::layout_work_flags());

            node.previous_layout = node.layout;
            node.layout = rect;
            node.world_origin = world_origin;
            node.work.remove(HostWorkFlags::RECALC_LAYOUT);
            if layout_changed {
                node.work.insert(HostWorkFlags::SYNC_RENDER);
            }
            if size_changed {
                node.work.insert(HostWorkFlags::REBUILD_PAINT);
            }

            if should_sync_children {
                (node.children.clone(), HostWorkFlags::empty())
            } else {
                return node.work | node.subtree_work;
            }
        };

        for child in children {
            subtree_work |= self.sync_layout(child, world_origin.x, world_origin.y);
        }

        let content_size = self.content_size_from_children(id, taffy_content_size);
        let scroll_dirty = {
            let node = self.nodes.get_mut(id).expect("node removed during layout");
            let content_size_changed = node.content_size != content_size;
            let scroll_offset_before_clamp = node.scroll_offset;
            node.content_size = content_size;
            clamp_scroll_offset(node);
            node.target_style.scroll.direction.is_scrollable()
                && (content_size_changed || node.scroll_offset != scroll_offset_before_clamp)
        };
        if scroll_dirty {
            let node = self.nodes.get_mut(id).expect("node removed during layout");
            node.work.insert(HostWorkFlags::SYNC_RENDER);
        }
        let node = self.nodes.get_mut(id).expect("node removed during layout");
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }

    fn content_size_from_children(&self, id: NodeId, taffy_content_size: Size<f32>) -> Size<f32> {
        let node = &self.nodes[id];
        let mut width = taffy_content_size.width.max(node.layout.width);
        let mut height = taffy_content_size.height.max(node.layout.height);

        for child in &node.children {
            let child = &self.nodes[*child];
            width = width.max(child.layout.x + child.layout.width - node.layout.x);
            height = height.max(child.layout.y + child.layout.height - node.layout.y);
        }

        Size::<f32>::new(width, height)
    }

    fn node_uses_unrounded_layout(&self, id: NodeId) -> bool {
        self.nodes
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
        let work = self.nodes[id].work;
        let subtree_work = self.nodes[id].subtree_work;
        if !work.intersects(Self::paint_work_flags())
            && !subtree_work.intersects(Self::paint_work_flags())
        {
            return;
        }

        if work.intersects(Self::paint_work_flags()) {
            self.repaint_if_needed(id);
        }

        let children = self.nodes[id].children.clone();
        for child in children {
            self.repaint_dirty_subtree(child);
        }
    }

    fn clear_work_subtree(&mut self, id: NodeId, flags: HostWorkFlags) -> HostWorkFlags {
        if !self.nodes.contains_key(id) {
            return HostWorkFlags::empty();
        }

        let children = self.nodes[id].children.clone();
        let mut subtree_work = HostWorkFlags::empty();
        for child in children {
            subtree_work |= self.clear_work_subtree(child, flags);
        }

        let node = self.nodes.get_mut(id).expect("checked node existence");
        node.old_props_hash = node.new_props_hash;
        node.work.remove(flags);
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }

    fn rebuild_subtree_dirty(&mut self, id: NodeId) -> HostWorkFlags {
        if !self.nodes.contains_key(id) {
            return HostWorkFlags::empty();
        }

        let children = self.nodes[id].children.clone();
        let mut subtree_work = HostWorkFlags::empty();
        for child in children {
            subtree_work |= self.rebuild_subtree_dirty(child);
        }

        let node = self.nodes.get_mut(id).expect("checked node existence");
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }

    pub fn build_render_frame(&mut self) -> Result<Option<RenderFrame>, RenderFrameError> {
        let dirty_snapshot = self.render_scene.dirty_snapshot();
        let properties_snapshot = self.frame_properties.snapshot();
        let root = self.nodes[self.root].layout;
        let viewport = Rect::new(0.0, 0.0, root.width, root.height);
        let viewport_changed = self.last_presented_viewport != Some(viewport);
        let needs_scene_compile =
            self.scene_compiler.compiled_scene().is_none() || !dirty_snapshot.nodes.is_empty();
        if !needs_scene_compile && !self.frame_properties.is_dirty() && !viewport_changed {
            return Ok(None);
        }
        if needs_scene_compile {
            self.scene_compiler
                .compile(&self.render_scene, &dirty_snapshot)?;
        }
        let compiled = self
            .scene_compiler
            .compiled_scene()
            .expect("scene compiler is initialized before frame building");
        let built = self
            .frame_builder
            .build(compiled, viewport, &self.frame_properties)?;
        Ok(Some(RenderFrame {
            built,
            dirty_snapshot,
            properties_snapshot,
            viewport,
        }))
    }

    pub fn finish_render_frame(&mut self, frame: &RenderFrame) {
        self.clear_work_subtree(self.root, HostWorkFlags::REBUILD_PAINT);
        self.render_scene.acknowledge(&frame.dirty_snapshot);
        self.frame_properties.acknowledge(frame.properties_snapshot);
        self.last_presented_viewport = Some(frame.viewport);
    }

    pub fn is_dirty(&self) -> bool {
        self.render_scene.is_dirty()
            || self.frame_properties.is_dirty()
            || self.has_running_style_animations()
            || !self.ui_state.style_dirty_list.is_empty()
            || !self.ui_state.style_subtree_dirty_list.is_empty()
            || !self.ui_state.layout_dirty_list.is_empty()
            || self
                .nodes
                .values()
                .any(|node| !node.work.is_empty() || !node.subtree_work.is_empty())
    }

    pub fn update_widget_node_from_parts(
        &mut self,
        id: NodeId,
        key: Option<Key>,
        props_hash: u64,
        widget: WidgetI,
        mut event_handlers: EventHandlers,
    ) -> WidgetI {
        let mut flags = WidgetUpdateFlags::empty();
        let current_widget;
        let shortcut_bindings = std::mem::take(&mut event_handlers.shortcuts);
        let focus = event_handlers.focus;
        let focus_handle = event_handlers.focus_handle.take();
        let accessibility = std::mem::take(&mut event_handlers.accessibility);
        let event_callbacks = {
            let current = self
                .nodes
                .get(id)
                .expect("reused node missing")
                .event_callbacks;
            self.event_callbacks.update_set(current, event_handlers)
        };

        let old_focus_handle;
        {
            let node = self.nodes.get_mut(id).expect("reused node missing");

            node.key = key;
            node.new_props_hash = props_hash;

            let widget_flags = node.widget.update_from(&widget);

            flags |= widget_flags;
            node.event_callbacks = event_callbacks;
            node.shortcut_bindings = shortcut_bindings;
            node.focus = focus;
            node.accessibility = accessibility;
            old_focus_handle = std::mem::replace(&mut node.focus_handle, focus_handle);
            current_widget = node.widget.clone();
        }

        if let Some(handle) = old_focus_handle {
            handle.unbind(id);
        }
        if let Some(handle) = self.nodes[id].focus_handle.as_ref() {
            handle.bind(id);
        }
        if self.focus_manager.focused() == Some(id) && !self.nodes[id].is_focusable() {
            self.focus_manager
                .request_focus(None, xui_interface::FocusReason::Disabled);
        }

        self.refresh_taffy_context(id);
        self.mark_dirty(id, flags);
        current_widget
    }

    fn sync_taffy_children(&mut self, parent: NodeId) {
        let parent_taffy = self.nodes[parent].taffy_node;
        let taffy_children: Vec<_> = self.nodes[parent]
            .children
            .iter()
            .map(|id| self.nodes[*id].taffy_node)
            .collect();
        self.taffy
            .set_children(parent_taffy, &taffy_children)
            .expect("failed to sync taffy children");
    }

    fn detach_child_from_current_parent(&mut self, child: NodeId, old_parent: NodeId) {
        self.nodes[old_parent]
            .children
            .retain(|candidate| *candidate != child);
        self.nodes[child].parent = None;
        self.nodes[child].position = 0;
        self.sync_taffy_children(old_parent);
        self.reindex_children(old_parent);
        self.mark_work(
            old_parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
    }

    pub fn set_children(&mut self, parent: NodeId, children: Vec<NodeId>) {
        if !self.nodes.contains_key(parent) {
            return;
        }

        let old_children = self.nodes[parent].children.clone();
        let tree_changed = old_children != children;

        for (old_position, child) in old_children.iter().copied().enumerate() {
            if !children.contains(&child)
                && self.nodes.contains_key(child)
                && self.nodes[child].parent == Some(parent)
            {
                self.nodes[child].parent = None;
                self.mark_work(
                    child,
                    HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
                );
                self.nodes[child].position = 0;
                self.record_node_move(child, Some(parent), None, old_position, 0);
            }
        }

        self.nodes[parent].children = children;
        let new_children = self.nodes[parent].children.clone();
        for (new_position, child) in new_children.iter().copied().enumerate() {
            if self.nodes.contains_key(child) {
                let old_parent = self.nodes[child].parent;
                let old_position = self.nodes[child].position;
                if let Some(old_parent) = old_parent.filter(|old_parent| *old_parent != parent) {
                    self.nodes[old_parent]
                        .children
                        .retain(|candidate| *candidate != child);
                    self.sync_taffy_children(old_parent);
                    self.reindex_children(old_parent);
                    self.mark_work(
                        old_parent,
                        HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
                    );
                }
                self.nodes[child].parent = Some(parent);
                if old_parent != Some(parent) {
                    self.mark_work(
                        child,
                        HostWorkFlags::RECALC_STYLE_SUBTREE | HostWorkFlags::RECALC_LAYOUT,
                    );
                }
                self.nodes[child].position = new_position;
                self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
            }
        }
        self.reindex_children(parent);
        self.sync_taffy_children(parent);

        if tree_changed {
            self.mark_work(
                parent,
                HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }
    }

    fn reindex_children(&mut self, parent: NodeId) {
        let children = self.nodes[parent].children.clone();
        for (position, child) in children.into_iter().enumerate() {
            self.nodes[child].position = position;
        }
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
        let node = &self.nodes[id];

        let context = match node.node_type {
            WidgetType::Text | WidgetType::TextInput => Some(WidgetContext::Text(id)),
            WidgetType::Image => node.widget.intrinsic_size().map(WidgetContext::Image),
            _ => None,
        };

        self.taffy
            .set_node_context(node.taffy_node, context)
            .expect("failed to update taffy context");
    }
    fn recompute_node_text_shape<T: TextBackend>(
        &mut self,
        node_id: NodeId,
        measurer: &mut TextHost<T>,
    ) {
        let node = self.nodes.get_mut(node_id).unwrap();
        match node.node_type {
            WidgetType::TextInput => {
                let style = &node.effective_style;
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
                measurer.get_or_shape_slot(node.id, TextLayoutSlot::PRIMARY, input);
            }
            WidgetType::Canvas => {
                let commands = node.widget.with_widgets(|widget| match widget {
                    Widgets::Canvas(canvas) => canvas.controller.scene(),
                    _ => unreachable!("Canvas node must hold a CanvasWidget"),
                });
                let requests: Vec<_> = commands
                    .commands()
                    .iter()
                    .filter_map(|command| match command {
                        VectorCommand::TextBox { id, bounds, props } => {
                            Some((canvas_text_slot(*id), *bounds, props.clone()))
                        }
                        _ => None,
                    })
                    .collect();
                measurer.retain_direct_slots(node.id, requests.iter().map(|(slot, _, _)| *slot));
                let font_context = measurer.backend().epoch();
                for (slot, bounds, props) in requests {
                    let input = TextLayoutInput::new(
                        props.text,
                        TextLayoutConstraints::max_width(bounds.width.max(0.0)),
                        props.style.into(),
                        props.paragraph,
                        props.text_box,
                        font_context,
                    );
                    measurer.get_or_shape_slot(node.id, slot, input);
                }
            }
            _ => {
                return;
            }
        }
    }
}

impl Default for UiArena {
    fn default() -> Self {
        Self::new()
    }
}

fn layer_descriptor_from_style(style: &ComputedStyle, bounds: Rect) -> Option<LayerDescriptor> {
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
    use crate::event_system::callbacks::EventHandlers;
    use crate::event_system::translator::EventTranslator;
    use crate::render::RenderNodeKind;
    use crate::text::testing::ZeroTextBackend;
    use crate::widgets::{CanvasController, WidgetI, canvas, container, text, text_input, z_stack};
    use std::time::{Duration, Instant};
    use xui_animation::{Easing, Transition};
    use xui_interface::events::{
        Modifiers, PointerButtons, PointerKind, RawPointerMove, XuiPointerId,
    };
    use xui_interface::{
        Affine, CanvasTextId, Color, ComputedColorStyle, FontDatabase, PathBuilder, PathFill,
        Style, TextProps, VectorSceneBuilder, WidgetState,
    };

    fn create_host(arena: &mut UiArena, widget: WidgetI) -> NodeId {
        let parent = arena.root();
        let key = widget.key();
        let props_hash = widget.props_hash();
        let id = arena.create_node(key, props_hash, widget, EventHandlers::default());
        arena.append_child(parent, id);
        id
    }

    fn update_host(arena: &mut UiArena, id: NodeId, widget: WidgetI) {
        let key = widget.key();
        let props_hash = widget.props_hash();
        arena.update_widget_node_from_parts(id, key, props_hash, widget, EventHandlers::default());
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
        let mut arena = UiArena::new();
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
        let handlers = widget.take_event_handlers();
        let node = arena.create_node(key, props_hash, widget, handlers);
        arena.append_child(arena.root(), node);

        assert_eq!(handle.node_id(), Some(node));
        assert!(arena.node(node).unwrap().is_focusable());
        assert!(!arena.node(node).unwrap().is_sequentially_focusable());
        assert_eq!(
            arena.node(node).unwrap().accessibility.role,
            Some(xui_interface::AccessibilityRole::Tab)
        );
        assert_eq!(
            arena.node(node).unwrap().accessibility.controls.as_deref(),
            Some("settings-panel")
        );

        arena.remove_subtree(node);
        assert!(!handle.is_bound());
    }

    #[test]
    fn focus_handle_can_request_focus_for_another_node() {
        let mut arena = UiArena::new();
        let target_handle = FocusHandle::new();

        let source_widget = WidgetI::new(container().focusable(true));
        let source_hash = source_widget.props_hash();
        let source_handlers = source_widget.take_event_handlers();
        let source = arena.create_node(None, source_hash, source_widget, source_handlers);
        arena.append_child(arena.root(), source);

        let target_widget = WidgetI::new(
            container()
                .focusable(true)
                .focus_handle(target_handle.clone()),
        );
        let target_hash = target_widget.props_hash();
        let target_handlers = target_widget.take_event_handlers();
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
        let mut arena = UiArena::new();
        let old_handle = FocusHandle::new();
        let initial = WidgetI::new(
            container()
                .tab_index(0)
                .focus_handle(old_handle.clone())
                .accessibility_role(xui_interface::AccessibilityRole::Tab)
                .accessibility_selected(false),
        );
        let initial_hash = initial.props_hash();
        let initial_handlers = initial.take_event_handlers();
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
        let updated_handlers = updated.take_event_handlers();
        arena.update_widget_node_from_parts(node, None, updated_hash, updated, updated_handlers);

        assert!(!old_handle.is_bound());
        assert_eq!(new_handle.node_id(), Some(node));
        assert_eq!(arena.node(node).unwrap().focus.tab_index, Some(-1));
        assert_eq!(arena.node(node).unwrap().accessibility.selected, Some(true));
    }

    #[test]
    fn final_text_width_is_activated_after_layout_measurement() {
        let mut arena = UiArena::new();
        let node = create_host(
            &mut arena,
            WidgetI::new(text("飞行监测").style(Style::new().width(120.0))),
        );
        let mut measurer = TextHost::new(ZeroTextBackend);

        arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

        let active = measurer
            .active_slot(node, TextLayoutSlot::PRIMARY)
            .expect("final layout must activate regular text");
        let host_node = arena.node(node).unwrap();
        assert_eq!(host_node.layout.width, 120.0);
        let props = host_node
            .widget
            .with_widgets(|widget| widget.text_layout_props(&host_node.effective_style))
            .unwrap();
        let final_input = TextLayoutInput::new(
            props.text,
            TextLayoutConstraints::max_width(host_node.layout.width),
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
        let mut arena = UiArena::new();
        let node = create_host(
            &mut arena,
            WidgetI::new(text("中文").style(Style::new().width(45.5).height(20.0))),
        );
        let mut measurer = TextHost::new(ZeroTextBackend);

        arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

        let taffy_node = arena.nodes[node].taffy_node;
        let rounded_width = arena.taffy.layout(taffy_node).unwrap().size.width;
        let unrounded_width = arena.taffy.unrounded_layout(taffy_node).size.width;
        assert_eq!(rounded_width, 46.0);
        assert_eq!(unrounded_width, 45.5);
        assert_eq!(arena.node(node).unwrap().layout.width, unrounded_width);
    }

    #[test]
    fn resizing_invalidates_intrinsic_text_layout_caches() {
        fn create_child(arena: &mut UiArena, parent: NodeId, widget: WidgetI) -> NodeId {
            let child = create_host(arena, widget);
            arena.append_child(parent, child);
            child
        }

        let mut arena = UiArena::new();
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
        let label = create_child(&mut arena, tab, WidgetI::new(text("飞行监测")));
        create_child(
            &mut arena,
            analytics,
            WidgetI::new(container().style(Style::new().size(Size::fill()))),
        );

        let mut measurer = TextHost::new(crate::Engine::new());
        arena.update_tree(Size::new(1600.0, 900.0), &mut measurer);
        let expected_width = arena.node(label).unwrap().layout.width;
        assert!(expected_width > 12.0);

        for width in [900.0, 2000.0] {
            arena.mark_subtree_layout_dirty(arena.root());
            arena.update_tree(Size::new(width, 900.0), &mut measurer);

            let final_width = arena.node(label).unwrap().layout.width;
            let unrounded_width = arena
                .taffy
                .unrounded_layout(arena.nodes[label].taffy_node)
                .size
                .width;
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
                arena.taffy.layout(arena.nodes[label].taffy_node).unwrap(),
                arena.taffy.unrounded_layout(arena.nodes[label].taffy_node),
                active.lines,
            );
        }
    }
}

fn measure_layout_context<T: TextBackend>(
    ui_tree: &SlotMap<NodeId, Node>,
    known_dimensions: tf::Size<Option<f32>>,
    available_space: tf::Size<tf::AvailableSpace>,
    node_context: Option<&mut WidgetContext>,
    measurer: &mut TextHost<T>,
) -> tf::Size<f32> {
    let known_size = if let tf::Size {
        width: Some(width),
        height: Some(height),
    } = known_dimensions
    {
        Some(tf::Size { width, height })
    } else {
        None
    };

    let measured = match node_context {
        Some(WidgetContext::Text(node_id)) => {
            let node = ui_tree.get(*node_id).expect("node not found");
            if let Some(props) = node
                .widget
                .with_widgets(|w| w.text_layout_props(&node.effective_style))
            {
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
                let size = measurer.measure_slot(*node_id, TextLayoutSlot::PRIMARY, input);
                return tf::Size {
                    width: known_dimensions.width.unwrap_or(size.width),
                    height: known_dimensions.height.unwrap_or(size.height),
                };
            } else {
                return tf::Size {
                    width: known_dimensions.width.unwrap_or(0.0),
                    height: known_dimensions.height.unwrap_or(0.0),
                };
            }
        }
        Some(WidgetContext::Image(size)) => {
            return tf::Size {
                width: known_dimensions.width.unwrap_or(size.width),
                height: known_dimensions.height.unwrap_or(size.height),
            };
        }

        _ => {
            if let Some(size) = known_size {
                return size;
            }
            Size::<f32>::ZERO
        }
    };

    tf::Size {
        width: measured.width,
        height: measured.height,
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

fn clamp_scroll_offset(node: &mut Node) {
    let direction = node.target_style.scroll.direction;
    let max_x = if direction.allows_horizontal() {
        (node.content_size.width - node.layout.width).max(0.0)
    } else {
        0.0
    };
    let max_y = if direction.allows_vertical() {
        (node.content_size.height - node.layout.height).max(0.0)
    } else {
        0.0
    };
    node.scroll_offset.x = node.scroll_offset.x.clamp(0.0, max_x);
    node.scroll_offset.y = node.scroll_offset.y.clamp(0.0, max_y);
}

fn needs_scrollbar_overlay(node: &Node) -> bool {
    let direction = node.target_style.scroll.direction;
    let scrollbar = node.target_style.scroll.scrollbar;
    if scrollbar.visibility == ScrollbarVisibilityStyle::Hidden
        || scrollbar.width <= 0.0
        || !scrollbar.thumb_color.is_visible()
    {
        return false;
    }

    let max_x = (node.content_size.width - node.layout.width).max(0.0);
    let max_y = (node.content_size.height - node.layout.height).max(0.0);
    (direction.allows_vertical() && should_paint_scrollbar(scrollbar, max_y))
        || (direction.allows_horizontal() && should_paint_scrollbar(scrollbar, max_x))
}

fn render_scrollbars_in_rect(node: &Node, rect: Rect, writer: &mut RenderTreeWriter<'_>) {
    let direction = node.target_style.scroll.direction;
    let scrollbar = node.target_style.scroll.scrollbar;
    if scrollbar.visibility == ScrollbarVisibilityStyle::Hidden || scrollbar.width <= 0.0 {
        return;
    }

    let max_x = (node.content_size.width - node.layout.width).max(0.0);
    let max_y = (node.content_size.height - node.layout.height).max(0.0);

    if direction.allows_vertical()
        && should_paint_scrollbar(scrollbar, max_y)
        && scrollbar.thumb_color.is_visible()
    {
        let track = vertical_scrollbar_track(rect, scrollbar.width);
        render_scrollbar_part(track, scrollbar.track_color, scrollbar.radius, writer);

        if max_y > 0.0 {
            let ratio = (node.layout.height / node.content_size.height).clamp(0.0, 1.0);
            let thumb_height = (track.height * ratio)
                .max(scrollbar.width * 2.0)
                .min(track.height);
            let travel = (track.height - thumb_height).max(0.0);
            let top = track.y + travel * (node.scroll_offset.y / max_y);
            render_scrollbar_part(
                Rect::new(track.x, top, track.width, thumb_height),
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
            let ratio = (node.layout.width / node.content_size.width).clamp(0.0, 1.0);
            let thumb_width = (track.width * ratio)
                .max(scrollbar.width * 2.0)
                .min(track.width);
            let travel = (track.width - thumb_width).max(0.0);
            let left = track.x + travel * (node.scroll_offset.x / max_x);
            render_scrollbar_part(
                Rect::new(left, track.y, thumb_width, track.height),
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

fn vertical_scrollbar_track(rect: Rect, width: f32) -> Rect {
    Rect::new(
        rect.x + (rect.width - width).max(0.0),
        rect.y,
        width.min(rect.width),
        rect.height,
    )
}

fn horizontal_scrollbar_track(rect: Rect, width: f32) -> Rect {
    Rect::new(
        rect.x,
        rect.y + (rect.height - width).max(0.0),
        rect.width,
        width.min(rect.height),
    )
}

fn render_scrollbar_part(
    rect: Rect,
    color: ComputedColorStyle,
    radius: f32,
    writer: &mut RenderTreeWriter<'_>,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || !color.is_visible() {
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
