//! Scroll containers: offsets, controller requests, and scrollbar geometry.

use crate::core::{Point, Size};
use crate::scroll::{
    AppliedScroll, ScrollMetrics, ScrollRequestQueue, ScrollbarAxis, ScrollbarHit, ScrollbarPart,
};
use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::state::HostWorkFlags;
use xui_interface::{
    Bounds, ComputedScrollStyle, ComputedScrollbarStyle, NodeId, ScrollbarVisibilityStyle,
};

impl UiRuntime {
    /// # Preconditions
    /// - `id` is live.
    pub(crate) fn set_scroll_offset(&mut self, id: NodeId, offset: Point) -> bool {
        let layout = self.layout_tree.node_mut(id);
        if layout.scroll_offset == offset {
            return false;
        }
        layout.scroll_offset = offset;
        self.mark_work(id, HostWorkFlags::SYNC_RENDER);
        self.publish_scroll_metrics(id);
        true
    }

    /// The scroll state of `id`, or `None` when it is not a scroll container.
    ///
    /// # Preconditions
    /// - `id` is live.
    pub(crate) fn scroll_metrics(&self, id: NodeId) -> Option<ScrollMetrics> {
        let direction = self.style_system.computed(id).scroll.direction;
        if !direction.is_scrollable() {
            return None;
        }
        let layout = self.layout_tree.node(id);
        let viewport_size = layout.layout.size();
        let max_offset = Point::new(
            if direction.allows_horizontal() {
                (layout.content_size.width - viewport_size.width).max(0.0)
            } else {
                0.0
            },
            if direction.allows_vertical() {
                (layout.content_size.height - viewport_size.height).max(0.0)
            } else {
                0.0
            },
        );
        Some(ScrollMetrics {
            offset: layout.scroll_offset,
            content_size: layout.content_size,
            viewport_size,
            max_offset,
        })
    }

    /// Moves `id` to `target`, clamped into its scrollable range.
    ///
    /// Returns what moved, or `None` when `id` does not scroll or is already
    /// there. The caller owns reporting it: this does not dispatch an event.
    pub(crate) fn scroll_node_to(&mut self, id: NodeId, target: Point) -> Option<AppliedScroll> {
        let metrics = self.scroll_metrics(id)?;
        let after = metrics.clamp(target);
        if after == metrics.offset {
            return None;
        }
        self.set_scroll_offset(id, after);
        Some(AppliedScroll {
            node: id,
            before: metrics.offset,
            after,
        })
    }

    fn publish_scroll_metrics(&self, id: NodeId) {
        if let Some(controller) = self.interaction_system.scroll_controller(id) {
            // A controller on a widget that does not scroll reads as empty
            // rather than keeping whatever it last saw.
            controller.publish(id, self.scroll_metrics(id).unwrap_or_default());
        }
    }

    pub(super) fn publish_all_scroll_metrics(&self) {
        for id in self.interaction_system.scroll_controller_nodes() {
            self.publish_scroll_metrics(id);
        }
    }

    pub(crate) fn scroll_request_queue(&self) -> ScrollRequestQueue {
        self.interaction_system.scroll_requests.clone()
    }

    /// Offsets layout clamped since the last call, for nodes still mounted.
    pub(crate) fn take_clamped_scrolls(&mut self) -> Vec<AppliedScroll> {
        let mut clamped = self.interaction_system.take_clamped_scrolls();
        clamped.retain(|scroll| self.hosts.contains_key(scroll.node));
        clamped
    }

    pub(crate) fn has_pending_scroll_requests(&self) -> bool {
        !self.interaction_system.scroll_requests.is_empty()
    }

    /// Applies every queued `ScrollController` request, in order.
    ///
    /// Resolved against the current layout, so the caller brings layout up to
    /// date first. Returns the scrolls that moved something; dispatching their
    /// events is the caller's job, since this layer has no translator.
    pub(crate) fn apply_scroll_requests(&mut self) -> Vec<AppliedScroll> {
        let mut applied = Vec::new();
        for (id, request) in self.interaction_system.scroll_requests.drain() {
            // Queued by a controller, which can outlive its node.
            if !self.hosts.contains_key(id) {
                continue;
            }
            let Some(metrics) = self.scroll_metrics(id) else {
                continue;
            };
            if let Some(scroll) = self.scroll_node_to(id, request.target(&metrics)) {
                applied.push(scroll);
            }
        }
        applied
    }

    /// The scrollbar `id` paints along `axis`, in window coordinates.
    pub(crate) fn scrollbar_part(&self, id: NodeId, axis: ScrollbarAxis) -> Option<ScrollbarPart> {
        self.scrollbar_parts(id)?
            .into_iter()
            .flatten()
            .find(|part| part.axis == axis)
    }

    fn scrollbar_parts(&self, id: NodeId) -> Option<[Option<ScrollbarPart>; 2]> {
        let scroll = self.style_system.effective(id).scroll;
        if !scroll.direction.is_scrollable() {
            return None;
        }
        let layout = self.layout_tree.node(id);
        let rect = self.visual_layout(id);
        Some(scrollbar_parts(
            rect,
            scroll,
            layout.content_size,
            layout.scroll_offset,
        ))
    }

    /// The scrollbar under `point`, if any.
    ///
    /// Only containers on the hit path are candidates, which is what keeps a
    /// bar that is clipped away or covered by an overlay from being grabbed.
    /// Outermost wins: a container paints its bars above all of its content,
    /// nested scroll containers included.
    pub(crate) fn scrollbar_hit(&self, point: Point) -> Option<ScrollbarHit> {
        let target = self.hit_test(point)?;
        self.event_path(target).into_iter().find_map(|id| {
            self.scrollbar_parts(id)?
                .into_iter()
                .flatten()
                .filter(|part| part.max_offset > 0.0)
                .find(|part| part.hit_track().contains(point))
                .map(|part| ScrollbarHit {
                    node: id,
                    axis: part.axis,
                    on_thumb: part.hit_thumb().is_some_and(|thumb| thumb.contains(point)),
                })
        })
    }
}

/// The scrollbars a container with viewport `rect` paints, vertical first.
///
/// The single source of scrollbar geometry: painting and hit testing both call
/// it, with `rect` in local and window space respectively.
pub(super) fn scrollbar_parts(
    rect: Bounds,
    scroll: ComputedScrollStyle,
    content_size: Size<f32>,
    scroll_offset: Point,
) -> [Option<ScrollbarPart>; 2] {
    let direction = scroll.direction;
    let scrollbar = scroll.scrollbar;
    if scrollbar.visibility == ScrollbarVisibilityStyle::Hidden
        || scrollbar.width <= 0.0
        || !scrollbar.thumb_color.is_visible()
    {
        return [None, None];
    }

    let max_x = (content_size.width - rect.width()).max(0.0);
    let max_y = (content_size.height - rect.height()).max(0.0);

    let vertical =
        (direction.allows_vertical() && should_paint_scrollbar(scrollbar, max_y)).then(|| {
            let track = vertical_scrollbar_track(rect, scrollbar.width);
            let thumb = (max_y > 0.0).then(|| {
                let ratio = (rect.height() / content_size.height).clamp(0.0, 1.0);
                let thumb_height = (track.height() * ratio)
                    .max(scrollbar.width * 2.0)
                    .min(track.height());
                let travel = (track.height() - thumb_height).max(0.0);
                let top = track.y() + travel * (scroll_offset.y / max_y);
                Bounds::from_origin_size((track.x(), top), (track.width(), thumb_height))
            });
            ScrollbarPart {
                axis: ScrollbarAxis::Vertical,
                track,
                thumb,
                max_offset: max_y,
            }
        });

    let horizontal = (direction.allows_horizontal() && should_paint_scrollbar(scrollbar, max_x))
        .then(|| {
            let track = horizontal_scrollbar_track(rect, scrollbar.width);
            let thumb = (max_x > 0.0).then(|| {
                let ratio = (rect.width() / content_size.width).clamp(0.0, 1.0);
                let thumb_width = (track.width() * ratio)
                    .max(scrollbar.width * 2.0)
                    .min(track.width());
                let travel = (track.width() - thumb_width).max(0.0);
                let left = track.x() + travel * (scroll_offset.x / max_x);
                Bounds::from_origin_size((left, track.y()), (thumb_width, track.height()))
            });
            ScrollbarPart {
                axis: ScrollbarAxis::Horizontal,
                track,
                thumb,
                max_offset: max_x,
            }
        });

    [vertical, horizontal]
}

pub(super) fn clamp_scroll_offset(
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

pub(super) fn should_paint_scrollbar(scrollbar: ComputedScrollbarStyle, max_offset: f32) -> bool {
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
