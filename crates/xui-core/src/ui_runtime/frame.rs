//! The per-frame driver `App` calls, and what it asks before scheduling a frame.

use crate::core::Size;
use crate::text::TextHost;
use crate::ui_runtime::state::HostWorkFlags;
use crate::ui_runtime::{RenderFrame, RenderFrameError, UiRuntime};
use std::time::Duration;
use xui_interface::{Bounds, NodeId, TextBackend};

impl UiRuntime {
    /// # Preconditions
    /// - `id` is live.
    pub(crate) fn mark_subtree_layout_dirty(&mut self, id: NodeId) {
        let subtree: Vec<_> = self.hosts.subtree(id).collect();
        for node in subtree {
            self.mark_work(node, HostWorkFlags::RELAYOUT);
        }
    }

    /// Publishes the time of the frame about to run. See
    /// [`crate::app::App::begin_frame`].
    pub(crate) fn begin_frame(&mut self, frame: crate::clock::FrameTime) {
        self.frame_time = frame;
    }

    /// The time of the frame in progress.
    pub(crate) fn frame_time(&self) -> crate::clock::FrameTime {
        self.frame_time
    }

    pub(crate) fn tick_style_animations(&mut self, delta: Duration) -> bool {
        if !self.style_system.is_animating() {
            return false;
        }
        self.stats.inheritance_visits = 0;
        let changed = self.style_system.tick(delta, &self.theme);
        for (id, diff, requires_layout) in changed.iter().copied() {
            if diff.intersects(xui_interface::StyleDiffFlags::INHERITED) {
                self.ui_state.mark_inheritance_sync(id, true);
            }
            // Only the sample moved, never a target, so descendants follow
            // through the sampled inheritance sync below rather than a
            // target-style subtree recompute.
            let mut work = HostWorkFlags::from_style_diff(diff);
            if requires_layout {
                self.sync_effective_taffy_style(id);
                self.refresh_taffy_context(id);
                work |= HostWorkFlags::RELAYOUT;
            } else {
                work.remove(HostWorkFlags::RECALC_LAYOUT);
            }
            self.mark_work(id, work);
        }
        self.sync_pending_inheritance();
        !changed.is_empty()
    }

    pub(crate) fn has_running_style_animations(&self) -> bool {
        self.style_system.is_animating()
    }

    pub(crate) fn update_tree<T: TextBackend>(
        &mut self,
        size: Size<f32>,
        measurer: &mut TextHost<T>,
    ) {
        self.stats.update_visits = 0;
        self.stats.inheritance_visits = 0;
        self.apply_canvas_invalidations();
        // Every queue below may name nodes removed after they were queued;
        // `live` drops those, and nothing past it checks again.
        let state_dirty = self.ui_state.drain_state_change_dirty_list();
        for node_id in self.live(state_dirty) {
            self.recompute_node_state(node_id);
        }

        let shape_dirty = self.ui_state.drain_shape_dirty_list();
        for node_id in self.live(shape_dirty) {
            self.recompute_node_text_shape(node_id, measurer);
        }

        let style_dirty = self.style_system.drain_dirty();
        for node_id in self.live(style_dirty) {
            self.recompute_node_style(node_id);
        }

        let subtree_dirty = self.style_system.drain_subtree_dirty();
        for node_id in self.live(subtree_dirty) {
            self.recompute_subtree_styles(node_id);
        }

        // Only after every target is settled: a sample is resolved against its
        // parent's effective style, which a later recompute could still move.
        self.sync_pending_inheritance();

        // Recompute layout if needed.
        if self.has_layout_dirty() {
            self.compute_layout(size, measurer);
        }

        self.ui_state.layout_dirty_list.clear();
        // Every frame, not only after layout: a scroll moves an anchor without
        // laying anything out.
        self.place_anchored_nodes(size);
        // After layout, before paint: this is the only point where a canvas has
        // a final size and its text can still be shaped in time to be drawn.
        self.sync_dirty_canvases(measurer);
        self.rebuild_subtree_dirty(self.root);
        self.repaint_dirty_subtree(self.root);
        self.sync_render_scene()
            .expect("host tree must produce a valid render scene");
        self.clear_work_subtree(self.root, HostWorkFlags::all());
        // Layout can move an offset without anything scrolling -- content
        // shrinking under a scrolled container clamps it -- so controllers are
        // refreshed from the settled tree rather than only on scroll.
        self.publish_all_scroll_metrics();
    }

    /// `ids` without those removed since they were queued. The one place a
    /// queued id is checked.
    fn live(&self, mut ids: Vec<NodeId>) -> Vec<NodeId> {
        ids.retain(|id| self.hosts.contains_key(*id));
        ids
    }

    pub(crate) fn build_render_frame(&mut self) -> Result<Option<RenderFrame>, RenderFrameError> {
        let dirty_snapshot = self.render_system.scene.dirty_snapshot();
        let properties_snapshot = self.render_system.properties.snapshot();
        let root = self.layout_tree.node(self.root).layout;
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

    pub(crate) fn finish_render_frame(&mut self, frame: &RenderFrame) {
        self.clear_work_subtree(self.root, HostWorkFlags::REBUILD_PAINT);
        self.render_system.scene.acknowledge(&frame.dirty_snapshot);
        self.render_system
            .properties
            .acknowledge(frame.properties_snapshot);
        self.render_system.last_viewport = Some(frame.viewport);
    }

    pub(crate) fn is_dirty(&self) -> bool {
        !self.canvas_invalidations.is_empty()
            || self.has_pending_scroll_requests()
            || !self.ui_state.canvas_dirty_list.is_empty()
            || self.render_system.scene.is_dirty()
            || self.render_system.properties.is_dirty()
            || self.is_animating()
            || self.style_system.has_dirty()
            || !self.ui_state.layout_dirty_list.is_empty()
            || !self.hosts[self.root].work.is_empty()
            || !self.hosts[self.root].subtree_work.is_empty()
    }

    /// Whether any canvas asked, as it drew, to be drawn again.
    ///
    /// A set maintained as canvases compile, not a walk of the widget tree:
    /// `is_dirty` is consulted after every batch of events, and it should stay
    /// a handful of flag checks rather than borrowing every canvas widget.
    pub(crate) fn has_animating_canvases(&self) -> bool {
        !self.canvases_wanting_repaint.is_empty()
    }

    /// Whether anything wants the next frame for its own sake.
    ///
    /// Reported by `is_dirty`, and the reason an animation sustains itself:
    /// after a frame renders and drains the dirty list, this is what tells the
    /// runner to ask for the next one.
    ///
    /// The single place visibility is applied. An animation that nobody can see
    /// must not keep the loop running, and it is the *asking* that stops, not
    /// the animation -- every request is still recorded, so coming back on
    /// screen resumes exactly what was going on. Pairing that with a clock that
    /// does not advance while off screen (`FrameClock::hold`) is what makes the
    /// pause continuous rather than a skip.
    pub(crate) fn is_animating(&self) -> bool {
        self.window_visible
            && (self.has_running_style_animations()
                || self.has_animating_canvases()
                || self.tickers.is_running())
    }

    /// The handle `use_ticker` installs into. Cloned once, at startup, into the
    /// component runtime.
    pub(crate) fn tickers(&self) -> crate::ticker::TickerRegistry {
        self.tickers.clone()
    }

    /// Runs every live ticker once, for this frame.
    ///
    /// Beside [`Self::tick_animating_canvases`] and under the same rule: no
    /// frame is produced for a window nobody can see, and a frame that is
    /// produced anyway -- something invalidated the window while it was
    /// hidden -- must not advance anything through it.
    pub(crate) fn tick_tickers(&mut self, frame: crate::clock::FrameTime) {
        if !self.window_visible {
            return;
        }
        self.tickers.tick(frame);
    }

    /// Marks every canvas that asked for another frame dirty, once per frame.
    ///
    /// The sibling of `tick_style_animations`: `has_animating_canvases` gets a
    /// frame scheduled, and this is what makes the painter actually re-run in
    /// it. A painter that stops asking is dropped from the set as it compiles,
    /// so the loop ends on its own.
    pub(crate) fn tick_animating_canvases(&mut self) {
        if !self.window_visible {
            return;
        }
        let animating: Vec<_> = self.canvases_wanting_repaint.keys().collect();
        for id in animating {
            self.invalidate_canvas(id);
        }
    }

    /// Re-runs a canvas's drawing on the next frame.
    ///
    /// # Preconditions
    /// - `id` is a live canvas.
    fn invalidate_canvas(&mut self, id: NodeId) {
        self.ui_state.mark_canvas_dirty(id);
        self.mark_work(id, HostWorkFlags::REBUILD_PAINT);
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
            // Queued by a controller, which can outlive the node it names.
            if self.hosts.contains_key(id) {
                self.request_update(id, flags);
            }
        }
    }
}
