//! Programmatic access to a scroll container's position.
//!
//! A [`ScrollController`] is attached to a scrollable widget with
//! `widget.scroll_controller(controller.clone())`. The runtime binds it to that
//! widget's host node, keeps [`ScrollController::metrics`] current, and applies
//! the requests it queues on the next frame -- after layout, so a request made
//! right after adding content clamps against the content that was added.
//!
//! An applied request is reported like any other scroll: the container
//! receives a `Scroll` event whose source is `ScrollSource::Programmatic`. That
//! is what lets a component that only listens to `on_scroll`, such as a
//! virtual list, follow a jump it did not cause.

use std::cell::RefCell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use xui_interface::core::Bounds;
use xui_interface::{NodeId, Point, Size};

/// The scroll state of a container, as of the last layout or scroll.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollMetrics {
    /// The current offset of the content, in logical pixels from its origin.
    pub offset: Point,
    /// The size of the content, never smaller than the viewport.
    pub content_size: Size<f32>,
    /// The size of the visible area.
    pub viewport_size: Size<f32>,
    /// The largest offset on each axis. Zero on an axis the container does not
    /// scroll along, even when its content overflows there.
    pub max_offset: Point,
}

impl ScrollMetrics {
    pub fn can_scroll_x(&self) -> bool {
        self.max_offset.x > 0.0
    }

    pub fn can_scroll_y(&self) -> bool {
        self.max_offset.y > 0.0
    }

    /// Clamps a requested offset into the scrollable range.
    ///
    /// A NaN component keeps the current offset on that axis, so a bad
    /// computation in application code cannot poison the layout tree.
    pub fn clamp(&self, offset: Point) -> Point {
        let axis = |requested: f32, current: f32, max: f32| {
            if requested.is_nan() {
                current
            } else {
                requested.clamp(0.0, max)
            }
        };
        Point::new(
            axis(offset.x, self.offset.x, self.max_offset.x),
            axis(offset.y, self.offset.y, self.max_offset.y),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ScrollRequest {
    /// Absolute offsets; `None` keeps the axis where it is.
    To { x: Option<f32>, y: Option<f32> },
    /// A change in offset: positive moves toward the end of the content.
    By(Point),
}

impl ScrollRequest {
    pub(crate) fn target(self, metrics: &ScrollMetrics) -> Point {
        match self {
            Self::To { x, y } => {
                Point::new(x.unwrap_or(metrics.offset.x), y.unwrap_or(metrics.offset.y))
            }
            Self::By(delta) => Point::new(metrics.offset.x + delta.x, metrics.offset.y + delta.y),
        }
    }
}

/// The queue a [`ScrollController`] drops requests into.
///
/// Shaped like the canvas invalidator: a request carries the node it targets,
/// the runtime reports pending requests through `is_dirty`, and pushing one
/// wakes the loop so a request made from a timer or a task is not stranded
/// until the next input event.
#[derive(Clone, Default)]
pub(crate) struct ScrollRequestQueue {
    pending: Rc<RefCell<Vec<(NodeId, ScrollRequest)>>>,
    wake: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

impl ScrollRequestQueue {
    pub(crate) fn set_wake(&self, wake: Rc<dyn Fn()>) {
        *self.wake.borrow_mut() = Some(wake);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.borrow().is_empty()
    }

    pub(crate) fn drain(&self) -> Vec<(NodeId, ScrollRequest)> {
        std::mem::take(&mut *self.pending.borrow_mut())
    }

    fn push(&self, node: NodeId, request: ScrollRequest) {
        self.pending.borrow_mut().push((node, request));
        let wake = self.wake.borrow().clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

#[derive(Default)]
struct ScrollControllerState {
    node: Option<NodeId>,
    queue: Option<ScrollRequestQueue>,
    metrics: ScrollMetrics,
}

/// A handle for reading and driving a scroll container's position.
///
/// Create it once -- typically in `use_memo` -- and attach it with
/// `widget.scroll_controller(controller.clone())`. Requests made while the
/// widget is not mounted are dropped and report `false`, the same contract as
/// [`crate::focus::FocusHandle::request_focus`].
///
/// Requests queue and apply in order on the next frame, so
/// `scroll_to_end()` followed by `scroll_by((0.0, -20.0))` lands 20px above
/// the end. [`Self::metrics`] reflects the last applied state, not pending
/// requests.
#[derive(Clone, Default)]
pub struct ScrollController {
    inner: Rc<RefCell<ScrollControllerState>>,
}

impl ScrollController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn node_id(&self) -> Option<NodeId> {
        self.inner.borrow().node
    }

    pub fn is_bound(&self) -> bool {
        self.node_id().is_some()
    }

    pub fn metrics(&self) -> ScrollMetrics {
        self.inner.borrow().metrics
    }

    pub fn offset(&self) -> Point {
        self.metrics().offset
    }

    /// Scrolls so the content's `offset` sits at the viewport's origin.
    pub fn scroll_to(&self, offset: impl Into<Point>) -> bool {
        let offset = offset.into();
        self.request(ScrollRequest::To {
            x: Some(offset.x),
            y: Some(offset.y),
        })
    }

    pub fn scroll_to_x(&self, x: f32) -> bool {
        self.request(ScrollRequest::To {
            x: Some(x),
            y: None,
        })
    }

    pub fn scroll_to_y(&self, y: f32) -> bool {
        self.request(ScrollRequest::To {
            x: None,
            y: Some(y),
        })
    }

    /// Moves the offset by `delta`; positive values move toward the end.
    pub fn scroll_by(&self, delta: impl Into<Point>) -> bool {
        self.request(ScrollRequest::By(delta.into()))
    }

    pub fn scroll_to_start(&self) -> bool {
        self.scroll_to((0.0, 0.0))
    }

    /// Scrolls to the end of the content on every axis the container scrolls
    /// along, resolved against the layout at the time the request applies.
    pub fn scroll_to_end(&self) -> bool {
        self.scroll_to((f32::INFINITY, f32::INFINITY))
    }

    fn request(&self, request: ScrollRequest) -> bool {
        // Release the borrow before pushing: the wake callback is foreign code.
        let target = {
            let state = self.inner.borrow();
            state.node.zip(state.queue.clone())
        };
        let Some((node, queue)) = target else {
            return false;
        };
        queue.push(node, request);
        true
    }

    pub(crate) fn bind(&self, node: NodeId, queue: ScrollRequestQueue) {
        let mut state = self.inner.borrow_mut();
        state.node = Some(node);
        state.queue = Some(queue);
    }

    pub(crate) fn unbind(&self, node: NodeId) {
        let mut state = self.inner.borrow_mut();
        if state.node == Some(node) {
            state.node = None;
            state.queue = None;
        }
    }

    pub(crate) fn publish(&self, node: NodeId, metrics: ScrollMetrics) {
        let mut state = self.inner.borrow_mut();
        if state.node == Some(node) {
            state.metrics = metrics;
        }
    }
}

/// How far one arrow-key press scrolls, in logical pixels.
pub(crate) const SCROLL_LINE_STEP: f32 = 40.0;

/// How far one page -- a track press, Page Up/Down, Space -- scrolls, given the
/// viewport's length along that axis. Short of a full viewport so the reader
/// keeps a line of context across the jump.
pub(crate) fn scroll_page_step(viewport_length: f32) -> f32 {
    viewport_length * 0.875
}

/// A scroll the runtime applied, reported back so its event can be dispatched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AppliedScroll {
    pub node: NodeId,
    pub before: Point,
    pub after: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScrollbarAxis {
    Vertical,
    Horizontal,
}

/// Thin scrollbars are hard to hit, so the interactive area of a track is at
/// least this thick, growing inward from the container's edge.
pub(crate) const SCROLLBAR_MIN_HIT_THICKNESS: f32 = 12.0;

/// One painted scrollbar, in whatever space its container's rect was given in.
///
/// Rendering and hit testing both derive from this, so what can be grabbed is
/// always exactly what was drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScrollbarPart {
    pub axis: ScrollbarAxis,
    pub track: Bounds,
    /// `None` when there is nothing to scroll: an `Always` bar with no overflow.
    pub thumb: Option<Bounds>,
    pub max_offset: f32,
}

impl ScrollbarPart {
    /// The coordinate of `point` along this bar's axis.
    pub(crate) fn along(&self, point: Point) -> f32 {
        match self.axis {
            ScrollbarAxis::Vertical => point.y,
            ScrollbarAxis::Horizontal => point.x,
        }
    }

    /// `base` with its coordinate along this bar's axis replaced by `value`.
    pub(crate) fn with_along(&self, base: Point, value: f32) -> Point {
        match self.axis {
            ScrollbarAxis::Vertical => Point::new(base.x, value),
            ScrollbarAxis::Horizontal => Point::new(value, base.y),
        }
    }

    fn length(&self, bounds: Bounds) -> f32 {
        match self.axis {
            ScrollbarAxis::Vertical => bounds.height(),
            ScrollbarAxis::Horizontal => bounds.width(),
        }
    }

    /// How far the thumb can move, in the same units as the track.
    pub(crate) fn travel(&self) -> f32 {
        self.thumb.map_or(0.0, |thumb| {
            (self.length(self.track) - self.length(thumb)).max(0.0)
        })
    }

    pub(crate) fn hit_track(&self) -> Bounds {
        self.widen(self.track)
    }

    pub(crate) fn hit_thumb(&self) -> Option<Bounds> {
        self.thumb.map(|thumb| self.widen(thumb))
    }

    fn widen(&self, bounds: Bounds) -> Bounds {
        match self.axis {
            ScrollbarAxis::Vertical => {
                let thickness = bounds.width().max(SCROLLBAR_MIN_HIT_THICKNESS);
                Bounds::from_origin_size(
                    (bounds.max.x - thickness, bounds.min.y),
                    (thickness, bounds.height()),
                )
            }
            ScrollbarAxis::Horizontal => {
                let thickness = bounds.height().max(SCROLLBAR_MIN_HIT_THICKNESS);
                Bounds::from_origin_size(
                    (bounds.min.x, bounds.max.y - thickness),
                    (bounds.width(), thickness),
                )
            }
        }
    }
}

/// A point that landed on a scrollbar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScrollbarHit {
    pub node: NodeId,
    pub axis: ScrollbarAxis,
    pub on_thumb: bool,
}

impl fmt::Debug for ScrollController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.borrow();
        f.debug_struct("ScrollController")
            .field("node", &state.node)
            .field("metrics", &state.metrics)
            .finish()
    }
}

impl PartialEq for ScrollController {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for ScrollController {}

impl Hash for ScrollController {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.inner).hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::KeyData;

    fn node(value: u64) -> NodeId {
        NodeId::from(KeyData::from_ffi(value))
    }

    fn metrics() -> ScrollMetrics {
        ScrollMetrics {
            offset: Point::new(0.0, 40.0),
            content_size: Size::new(100.0, 300.0),
            viewport_size: Size::new(100.0, 100.0),
            max_offset: Point::new(0.0, 200.0),
        }
    }

    #[test]
    fn an_unbound_controller_drops_requests() {
        let controller = ScrollController::new();
        assert!(!controller.scroll_to_end());
    }

    #[test]
    fn requests_target_the_bound_node_in_order() {
        let controller = ScrollController::new();
        let queue = ScrollRequestQueue::default();
        controller.bind(node(1), queue.clone());

        assert!(controller.scroll_to_end());
        assert!(controller.scroll_by((0.0, -20.0)));

        let drained = queue.drain();
        assert_eq!(drained.len(), 2);
        assert!(drained.iter().all(|(target, _)| *target == node(1)));
        assert!(queue.is_empty());
    }

    #[test]
    fn only_the_current_node_unbinds_or_publishes() {
        let controller = ScrollController::new();
        let queue = ScrollRequestQueue::default();
        controller.bind(node(1), queue.clone());
        controller.bind(node(2), queue);

        controller.publish(node(1), metrics());
        assert_eq!(controller.metrics(), ScrollMetrics::default());
        controller.unbind(node(1));
        assert_eq!(controller.node_id(), Some(node(2)));

        controller.publish(node(2), metrics());
        assert_eq!(controller.offset(), Point::new(0.0, 40.0));
        controller.unbind(node(2));
        assert!(!controller.is_bound());
    }

    #[test]
    fn targets_resolve_against_metrics_and_clamp() {
        let metrics = metrics();
        let end = ScrollRequest::To {
            x: Some(f32::INFINITY),
            y: Some(f32::INFINITY),
        };
        assert_eq!(metrics.clamp(end.target(&metrics)), Point::new(0.0, 200.0));

        let back = ScrollRequest::By(Point::new(0.0, -100.0));
        assert_eq!(metrics.clamp(back.target(&metrics)), Point::new(0.0, 0.0));

        let keep_x = ScrollRequest::To {
            x: None,
            y: Some(f32::NAN),
        };
        assert_eq!(metrics.clamp(keep_x.target(&metrics)), metrics.offset);
    }
}
