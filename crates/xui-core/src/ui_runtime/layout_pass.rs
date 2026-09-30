//! The layout pass: Taffy, geometry write-back, and anchored placement.

use crate::core::{Point, Size};
use crate::text::{TextHost, TextLayoutSlot};
use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::layout::{MeasuredLeaf, WidgetContext};
use crate::ui_runtime::scroll::clamp_scroll_offset;
use crate::ui_runtime::state::HostWorkFlags;
use crate::ui_runtime::style::StyleSystem;
use crate::ui_runtime::tree::{HostData, HostTree};
use crate::widgets::WidgetType;
use taffy::prelude as tf;
use xui_interface::{Bounds, NodeId, TextBackend, TextLayoutConstraints, TextLayoutInput};

impl UiRuntime {
    #[inline(always)]
    pub(super) fn has_layout_dirty(&self) -> bool {
        !self.ui_state.layout_dirty_list.is_empty()
    }

    pub(super) fn compute_layout<T: TextBackend>(
        &mut self,
        size: Size<f32>,
        measurer: &mut TextHost<T>,
    ) {
        self.run_taffy(size, measurer);
        self.sync_layout(self.root, 0.0, 0.0);
        self.resolve_anchored_widths(size, measurer);
        self.activate_final_text_layouts(measurer);
    }

    fn run_taffy<T: TextBackend>(&mut self, size: Size<f32>, measurer: &mut TextHost<T>) {
        self.stats.layout_passes += 1;
        self.layout_tree.compute_layout_with_measure(
            self.root,
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
        );
    }

    /// Gives each `match_width` node its target's width. The one part of
    /// anchoring that is layout: a changed width lays out again, in dependency
    /// order, so a node matching something inside another anchored node sees
    /// that node at its final width.
    fn resolve_anchored_widths<T: TextBackend>(
        &mut self,
        viewport: Size<f32>,
        measurer: &mut TextHost<T>,
    ) {
        if self.anchor_system.is_empty() {
            return;
        }
        for step in self.anchor_system.placement_order(&self.hosts) {
            let width = step
                .target
                .filter(|_| step.placement.match_width)
                .map(|target| self.visual_layout(target).width());
            if !self.anchor_system.set_width(step.node, width) {
                continue;
            }
            if self.sync_effective_taffy_style(step.node) {
                self.mark_work(step.node, HostWorkFlags::RECALC_LAYOUT);
                self.run_taffy(viewport, measurer);
                self.sync_layout(self.root, 0.0, 0.0);
            }
        }
    }

    /// Translates every anchored node next to its target's on-screen bounds,
    /// the way a Flutter follower layer tracks its target.
    ///
    /// The offset never enters layout: it is a paint transform, and a shift
    /// that `visual_layout`, hit testing and `to_local` add on top of the laid
    /// out position. So it can run every frame, after a scroll as well as a
    /// layout pass, for the price of a walk up the tree per node. Nodes go in
    /// dependency order, so a single walk settles every chain.
    pub(super) fn place_anchored_nodes(&mut self, viewport: Size<f32>) {
        if self.anchor_system.is_empty() {
            return;
        }
        for step in self.anchor_system.placement_order(&self.hosts) {
            // Without a target the node stays where layout put it.
            let offset = match step.target {
                Some(target) => {
                    let anchor = self.visual_layout(target);
                    let own = self.visual_layout(step.node);
                    let origin = crate::anchor::place(anchor, own.size(), viewport, step.placement);
                    // `own` already includes the current offset, so this is
                    // `origin` minus the laid-out position: the full offset.
                    self.anchor_system.offset(step.node) + (origin - own.min)
                }
                None => Point::zero(),
            };
            if self.anchor_system.set_offset(step.node, offset) {
                self.mark_work(step.node, HostWorkFlags::SYNC_TRANSFORM);
            }
        }
    }

    /// Activates the paint layout only after Taffy has committed every node's
    /// final width. Intrinsic measurement probes are deliberately inactive.
    fn activate_final_text_layouts<T: TextBackend>(&self, measurer: &mut TextHost<T>) {
        let font_context = measurer.backend().epoch();
        let requests: Vec<_> = self
            .text_nodes
            .keys()
            .filter_map(|id| {
                let node = &self.hosts[id];
                let effective = self.style_system.effective(id);
                let props = node
                    .widget
                    .with_widgets(|widget| widget.text_layout_props(effective))?;
                let layout = self.layout_tree.node(id).layout;
                // Taffy measured the paragraph against the content box. Wrapping
                // the painted variant at the border-box width instead gave a
                // padded paragraph fewer, longer lines than its box was sized for.
                let padding = effective.layout.padding;
                let content_width = layout.width() - padding.left() - padding.right();
                // A text input scrolls horizontally instead of wrapping, so it
                // paints the same unbounded variant it was measured with.
                let constraints = if node.node_type == WidgetType::TextInput {
                    TextLayoutConstraints::UNBOUNDED
                } else {
                    TextLayoutConstraints::max_width(content_width.max(0.0))
                };
                let input = TextLayoutInput::new(
                    props.text,
                    constraints,
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
        // Pixel-rounded widths can be smaller than the shaped intrinsic width.
        // For CJK text that turns the final character into a second line on
        // alternating resize frames. Preserve Taffy's computed floating-point
        // geometry for text while keeping pixel snapping for other widgets.
        let layout = if self.node_uses_unrounded_layout(id) {
            self.layout_tree.unrounded_layout(id)
        } else {
            self.layout_tree.layout(id)
        };
        let taffy_content_size =
            Size::<f32>::new(layout.content_size.width, layout.content_size.height);

        let origin = Point::new(layout.location.x, layout.location.y);
        let size = Size::new(layout.size.width, layout.size.height);
        let rect = Bounds::from_origin_size(origin, size);
        let world_origin = Point::new(offset_x + origin.x, offset_y + origin.y);

        let previous = self.layout_tree.node(id);
        let old_rect = previous.layout;
        let layout_changed = old_rect != rect || previous.world_origin != world_origin;
        let size_changed = old_rect.width() != rect.width() || old_rect.height() != rect.height();
        if size_changed && self.hosts[id].node_type == WidgetType::Canvas {
            self.ui_state.mark_canvas_dirty(id);
        }
        let mut subtree_work = {
            let node = &mut self.hosts[id];
            let should_sync_children = layout_changed
                || node.work.intersects(HostWorkFlags::RECALC_LAYOUT)
                || node.subtree_work.intersects(HostWorkFlags::RECALC_LAYOUT);

            let layout_node = self.layout_tree.node_mut(id);
            layout_node.previous_layout = layout_node.layout;
            layout_node.layout = rect;
            layout_node.world_origin = world_origin;
            node.work.remove(HostWorkFlags::RECALC_LAYOUT);
            if layout_changed {
                node.work.insert(HostWorkFlags::SYNC_RENDER);
            }
            if size_changed {
                // A transform origin is a fraction of the size.
                node.work
                    .insert(HostWorkFlags::REBUILD_PAINT | HostWorkFlags::SYNC_TRANSFORM);
            }

            if should_sync_children {
                HostWorkFlags::empty()
            } else {
                return node.work | node.subtree_work;
            }
        };

        let mut cursor = self.hosts.link(id).first_child;
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).next_sibling;
            subtree_work |= self.sync_layout(child, world_origin.x, world_origin.y);
        }

        let content_size = self.content_size_from_children(id, taffy_content_size);
        let (scroll_dirty, clamped) = {
            let scroll = self.style_system.computed(id).scroll;
            let direction = scroll.direction;
            let layout = self.layout_tree.node_mut(id);
            let content_size_changed = layout.content_size != content_size;
            let scroll_offset_before_clamp = layout.scroll_offset;
            layout.content_size = content_size;
            clamp_scroll_offset(layout, scroll);
            let clamped = (direction.is_scrollable()
                && layout.scroll_offset != scroll_offset_before_clamp)
                .then_some((scroll_offset_before_clamp, layout.scroll_offset));
            (
                direction.is_scrollable() && (content_size_changed || clamped.is_some()),
                clamped,
            )
        };
        if scroll_dirty {
            let node = &mut self.hosts[id];
            node.work.insert(HostWorkFlags::SYNC_RENDER);
        }
        if let Some((before, after)) = clamped {
            self.interaction_system
                .record_clamped_scroll(id, before, after);
        }
        let node = &mut self.hosts[id];
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }

    fn content_size_from_children(&self, id: NodeId, taffy_content_size: Size<f32>) -> Size<f32> {
        let node = self.layout_tree.node(id);
        let mut width = taffy_content_size.width.max(node.layout.width());
        let mut height = taffy_content_size.height.max(node.layout.height());

        for child_id in self.hosts.children(id) {
            let child = self.layout_tree.node(child_id);
            width = width.max(child.layout.x() + child.layout.width() - node.layout.x());
            height = height.max(child.layout.y() + child.layout.height() - node.layout.y());
        }

        Size::<f32>::new(width, height)
    }

    fn node_uses_unrounded_layout(&self, id: NodeId) -> bool {
        matches!(
            self.hosts[id].node_type,
            WidgetType::Text | WidgetType::TextInput
        )
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
            let node = &ui_tree[*node_id];
            let effective = styles.effective(*node_id);
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
