//! Viewport-bounded rendering for fixed-height collections.
//!
//! [`virtual_list`](fn@self::virtual_list) uses [`VirtualItemRenderer`] to build only a
//! visible window plus overscan. [`VirtualRange`] describes a half-open row range. Layout
//! size observations keep the window current after resizes.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`virtual_list`](fn@self::virtual_list) | A fixed-row-height scrolling list that materializes only the visible window and overscan. |

use xui_core::prelude::*;
use std::rc::Rc;

use xui_macros::component;

/// What a row of a [`virtual_list`](fn@virtual_list) shows, given its index:
/// text, or a component (`Children::component::<Row>(RowProps { index })`).
#[derive(Clone)]
pub struct VirtualItemRenderer(Rc<dyn Fn(usize) -> Children>);

impl VirtualItemRenderer {
    pub fn new(render: impl Fn(usize) -> Children + 'static) -> Self {
        Self(Rc::new(render))
    }

    pub fn call(&self, index: usize) -> Children {
        (self.0)(index)
    }
}

/// The same renderer, not an equal one.
impl PartialEq for VirtualItemRenderer {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// The half-open range of rows a `virtual_list` currently materializes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualRange {
    pub start: usize,
    pub end: usize,
}

impl VirtualRange {
    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }
}

/// Rows within `overscan` of the viewport are kept mounted so a scroll does not
/// have to build anything before the new rows can be painted.
fn visible_range(
    scroll_top: f32,
    viewport_height: f32,
    item_height: f32,
    item_count: usize,
    overscan: usize,
) -> VirtualRange {
    if item_count == 0
        || !item_height.is_finite()
        || item_height <= 0.0
        || !viewport_height.is_finite()
        || viewport_height <= 0.0
    {
        return VirtualRange { start: 0, end: 0 };
    }
    let max_scroll =
        (item_count as f64 * f64::from(item_height) - f64::from(viewport_height)).max(0.0);
    let offset = f64::from(scroll_top.max(0.0)).min(max_scroll);
    let first_visible = (offset / f64::from(item_height)).floor() as usize;
    // `+ 1` covers the row straddling the bottom edge, and a second row is
    // needed when the first one is only partly scrolled past the top.
    let span = ((viewport_height / item_height).ceil() as usize).saturating_add(2);
    let start = first_visible.saturating_sub(overscan);
    let end = first_visible
        .saturating_add(span)
        .saturating_add(overscan)
        .min(item_count);
    VirtualRange {
        start: start.min(end),
        end,
    }
}

/// A fixed-row-height scrolling list that materializes only the visible window and
/// overscan.
///
/// `render_item` maps an index near the viewport to the row's content. `viewport_height` seeds layout;
/// subsequent size observations follow resizes without requiring a wheel event. A row
/// mounts when its index enters the window and stays mounted while it remains there.
/// The component owns row position, width, and height; `item_style` customizes
/// decoration. Variable row heights are not supported.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `item_count` | `usize` | **Required** |
/// | `item_height` | `f32` | **Required** |
/// | `viewport_height` | `f32` | **Required** |
/// | `render_item` | `VirtualItemRenderer` | **Required** |
/// | `overscan` | `usize` | `3` |
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `item_style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example_list() {
///     let render_item: VirtualItemRenderer = VirtualItemRenderer::new(|index: usize| index.to_string().into());
///     view! {
///         virtual_list(item_count: 100_000, item_height: 24.0, viewport_height: 240.0, render_item: render_item.clone())
///     }
/// }
/// ```
#[component]
#[defaults(
    overscan = 3,
    style = StyleOf::new(),
    item_style = StyleOf::new(),
)]
pub fn virtual_list(
    #[source] item_count: &usize,
    #[source] item_height: &f32,
    viewport_height: &f32,
    #[source] render_item: &VirtualItemRenderer,
    #[source] overscan: &usize,
    style: &StyleOf<Container>,
    item_style: &StyleOf<Container>,
) {
    let scroll_top: f32 = state!(0.0);
    // The prop seeds layout; size observations also follow resizes without a wheel event.
    let size: Option<Size<f32>> = size!();
    let viewport: f32 = memo!(size.map(|size| size.height).filter(|height| *height > 0.0).unwrap_or(*viewport_height));
    let range: VirtualRange = memo!(visible_range(*scroll_top, *viewport, *item_height, *item_count, *overscan));
    let rows: Vec<usize> = memo!((range.start..range.end).collect());
    let total_height: f32 = memo!(if item_height.is_finite() && *item_height > 0.0 {
        (f64::from(*item_height) * *item_count as f64).min(f64::from(f32::MAX)) as f32
    } else {
        0.0
    });
    let mut viewport_style: StyleOf<Container> = StyleOf::new().with::<container::height>(if viewport_height.is_finite() {
        viewport_height.max(0.0)
    } else {
        0.0
    });
    viewport_style.merge(style);
    viewport_style = viewport_style.with::<container::scroll_direction>(ScrollDirectionStyle::Vertical);
    view! {
        container(
            style: viewport_style.clone(),
            size_observer: size,
            on_scroll: |cx, event, event_cx| {
                // The translator emits a `Scroll` for every node on the wheel's
                // path, and those for the rows beneath bubble through here with
                // no offsets. Only the one describing this container matters.
                if event.scroll_target != event_cx.node_id() {
                    return EventResult::Ignored;
                }
                let Some(offset_after) = event.offset_after else {
                    return EventResult::Ignored;
                };
                let offset = offset_after.y;
                let layout_height = event_cx.node_ref.layout.height();
                let viewport = if layout_height > 0.0 { layout_height } else { viewport.get() };
                // The runtime has already applied the scroll and moved the
                // content, so the offset is only worth writing when different
                // rows are needed.
                let next = visible_range(offset, viewport, item_height.get(), item_count.get(), overscan.get());
                if next != range.get() {
                    scroll_top.set(cx, offset);
                }
                EventResult::Ignored
            },
        ) {
            // The spacer carries the full scrollable height even though almost
            // none of it is occupied, which is what keeps the scrollbar honest.
            container(
                style: StyleOf::new()
                    .with::<container::position>(PositionStyle::Relative)
                    .with::<container::width>(Sizing::Fill)
                    .with::<container::height>(*total_height),
            ) {
                for index in rows.clone() {
                    container(
                        style: item_style
                            .clone()
                            .with::<container::position>(PositionStyle::Absolute)
                            .with::<container::inset>(EdgeInsets::new(0.0, 0.0, *index as f32 * *item_height, 0.0))
                            .with::<container::width>(Sizing::Fill)
                            .with::<container::height>(*item_height),
                    ) {
                        slot(render_item.call(*index))
                    }
                    .key(*index)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_unmeasured_list_materializes_nothing() {
        assert_eq!(
            visible_range(0.0, 200.0, 20.0, 0, 3),
            VirtualRange { start: 0, end: 0 }
        );
        // Before the first layout the viewport is unknown; rendering a guess
        // would only have to be thrown away.
        assert_eq!(
            visible_range(0.0, 0.0, 20.0, 100, 3),
            VirtualRange { start: 0, end: 0 }
        );
        assert_eq!(
            visible_range(0.0, 200.0, 0.0, 100, 3),
            VirtualRange { start: 0, end: 0 }
        );
    }

    #[test]
    fn the_range_covers_the_viewport_plus_overscan() {
        // 200pt of viewport over 20pt rows is 10 rows, plus the two partial
        // edge rows, plus overscan on the trailing side only at the top.
        let range = visible_range(0.0, 200.0, 20.0, 1_000, 3);
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 15);

        // Scrolled to row 50: overscan now applies on both sides.
        let range = visible_range(1_000.0, 200.0, 20.0, 1_000, 3);
        assert_eq!(range.start, 47);
        assert_eq!(range.end, 65);
    }

    #[test]
    fn the_range_stays_inside_the_list_at_both_ends() {
        let range = visible_range(-500.0, 200.0, 20.0, 1_000, 3);
        assert_eq!(range.start, 0);

        // Scrolled past the end: the range clamps instead of running off.
        let range = visible_range(1_000_000.0, 200.0, 20.0, 1_000, 3);
        assert_eq!(range.end, 1_000);
        assert!(range.start <= range.end);
    }

    #[test]
    fn a_row_straddling_the_top_edge_is_still_included() {
        // Half of row 5 is above the fold, so the range must start at 5, not 6.
        let range = visible_range(110.0, 200.0, 20.0, 1_000, 0);
        assert_eq!(range.start, 5);
        assert!(range.end >= 16, "the bottom edge row is missing: {range:?}");
    }

    #[test]
    fn the_rendered_row_count_never_scales_with_the_list_length() {
        let short = visible_range(0.0, 200.0, 20.0, 100, 3).len();
        let long = visible_range(0.0, 200.0, 20.0, 1_000_000, 3).len();
        assert_eq!(short, long);
    }

    #[test]
    fn invalid_dimensions_and_shrunk_sources_have_safe_ranges() {
        assert!(visible_range(0.0, 200.0, f32::NAN, 100, 3).is_empty());
        assert!(visible_range(0.0, f32::INFINITY, 20.0, 100, 3).is_empty());
        let range = visible_range(1_000_000.0, 200.0, 20.0, 20, 0);
        assert_eq!(range, VirtualRange { start: 10, end: 20 });
        assert_eq!(visible_range(0.0, 200.0, 20.0, 20, usize::MAX).end, 20);
    }

    // ---- wheel dispatch ---------------------------------------------------

    use std::time::Instant;
    use xui_core::text::TextHost;
    use xui_cosmic::CosmicEngine;
    use xui_interface::Translation;
    use xui_interface::events::{Modifiers, RawEvent, RawWheel, ScrollDelta};
    fn list_root(app: &mut App) {
        app.mount::<VirtualList>(VirtualListProps {
            item_count: ReadSignal::constant(1_000),
            item_height: ReadSignal::constant(20.0),
            viewport_height: 200.0,
            render_item: ReadSignal::constant(VirtualItemRenderer::new(|index: usize| index.to_string().into())),
            overscan: ReadSignal::constant(3),
            style: StyleOf::new().with::<container::width>(Sizing::Fill).with::<container::height>(200.0),
            item_style: StyleOf::new(),
        });
    }

    /// The translator emits a `Scroll` for every node from the hit row up to
    /// the root, and the ones for non-scrolling nodes carry no offsets. Those
    /// bubble through the list's own handler, which must not mistake them for
    /// a scroll of its container.
    #[test]
    fn a_wheel_over_a_row_scrolls_the_list_without_panicking() {
        let mut app = App::new(list_root);
        app.resize(Size::new(400.0, 200.0));
        let mut text = TextHost::new(CosmicEngine::new(1.0));
        let mut backend = MockRenderBackend::default();
        app.render(&mut backend, &mut text).expect("mock backend");

        let scroller = {
            let rt = app.ui_runtime();
            rt.children(rt.root()).next().expect("the list mounted")
        };
        let hit = app
            .ui_runtime()
            .hit_test(Point::new(10.0, 10.0))
            .expect("the wheel position hits something");
        assert_ne!(
            hit, scroller,
            "the wheel must land on a row, not the container"
        );

        app.dispatch_event(
            RawEvent::Wheel(RawWheel {
                position: Point::new(10.0, 10.0),
                delta: ScrollDelta::Pixels(Translation::new(0.0, -2_000.0)),
                device_id: None,
                pointer_id: None,
                modifiers: Modifiers::default(),
                timestamp: Instant::now(),
                is_inertial: false,
            }),
            &mut text,
        );
        app.render(&mut backend, &mut text).expect("mock backend");

        let offset = app
            .ui_runtime()
            .node(scroller)
            .expect("scroller")
            .scroll_offset
            .y;
        assert!(offset > 0.0, "the list did not scroll");
        let spacer = app
            .ui_runtime()
            .children(scroller)
            .next()
            .expect("the spacer mounted");
        let first_row = app
            .ui_runtime()
            .children(spacer)
            .next()
            .expect("rows mounted");
        let spacer_y = app
            .ui_runtime()
            .node(spacer)
            .expect("spacer")
            .world_origin
            .y;
        let row_y = app
            .ui_runtime()
            .node(first_row)
            .expect("row")
            .world_origin
            .y;
        assert!(
            row_y - spacer_y > 0.0,
            "the window did not follow the scroll to offset {offset}"
        );
    }
}
