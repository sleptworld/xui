//! Viewport-bounded rendering for long collections.
//!
//! [`virtual_list`](fn@self::virtual_list) builds only the rows near the viewport,
//! through a [`VirtualItemRenderer`]. Rows are all one [`ItemSize::Fixed`] size, or
//! start from an [`ItemSize::Estimated`] one and are measured once laid out. The
//! window logic is `xui_core::virtualize`'s [`Virtualizer`], so a component with
//! its own layout can virtualize the same way, measuring its rows with
//! [`virtual_item`](fn@self::virtual_item).
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`virtual_list`](fn@self::virtual_list) | A scrolling list that materializes only the visible window and overscan. |
//! | [`virtual_item`](fn@self::virtual_item) | A row that reports its laid-out size to a [`Virtualizer`]. |

use std::rc::Rc;
use xui_core::prelude::*;

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

/// A row of a virtualized collection that reports its laid-out size to `list`,
/// whose [`Virtualizer::measure`] then places the rows after it. The parent
/// keys it by `index` and lays it out; it adds no layout of its own beyond
/// `style`.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `list` | `Virtualizer` (`#[state]`) | **Required** |
/// | `index` | `usize` | **Required** |
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `children` | `Children` | none |
#[component]
#[defaults(style = StyleOf::new())]
pub fn virtual_item(
    #[state] list: &Virtualizer,
    index: &usize,
    style: &StyleOf<Container>,
    children: &Children,
) {
    let size: Option<Size<f32>> = size!();
    effect!(|cx| untrack!(list).measure(cx, *index, size.get()));
    view! {
        container(style: style.clone(), size_observer: size) {
            slot(children)
        }
    }
}

/// A scrolling list that materializes only the visible window and overscan.
///
/// `render_item` maps an index near the viewport to the row's content. With
/// [`ItemSize::Fixed`] every row is that tall; with [`ItemSize::Estimated`] a row
/// is as tall as its content, and rows not yet laid out count as the estimate
/// toward the scrollbar. `viewport_height` seeds the first window; later size
/// observations follow resizes without requiring a wheel event. A row mounts
/// when its index enters the window and stays mounted while it remains there.
/// The component owns row position and width (and height, when fixed);
/// `item_style` customizes decoration.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `item_count` | `usize` | **Required** |
/// | `item_size` | `ItemSize` | **Required** |
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
///         virtual_list(item_count: 100_000, item_size: ItemSize::Fixed(24.0), viewport_height: 240.0, render_item: render_item.clone())
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
    #[source] item_size: &ItemSize,
    viewport_height: &f32,
    #[source] render_item: &VirtualItemRenderer,
    #[source] overscan: &usize,
    style: &StyleOf<Container>,
    item_style: &StyleOf<Container>,
) {
    let list: Virtualizer = state!(
        Virtualizer::new(item_count.get(), item_size.get(), overscan.get())
            .with_viewport(*viewport_height)
    );
    effect!(|cx| untrack!(list).sync(cx, item_count.get(), item_size.get(), overscan.get()));
    // The prop seeds the window; size observations also follow resizes
    // without a wheel event.
    let size: Option<Size<f32>> = size!();
    effect!(|cx| {
        if let Some(size) = size.get().filter(|size| size.height > 0.0) {
            untrack!(list).set_viewport(cx, size.height);
        }
    });
    let mut viewport_style: StyleOf<Container> =
        StyleOf::new().with::<container::height>(if viewport_height.is_finite() {
            viewport_height.max(0.0)
        } else {
            0.0
        });
    viewport_style.merge(style);
    viewport_style =
        viewport_style.with::<container::scroll_direction>(ScrollDirectionStyle::Vertical);
    let row_style: StyleOf<Container> = memo!({
        let mut row_style = item_style.clone();
        row_style = row_style.with::<container::width>(Sizing::Fill);
        if let Some(height) = item_size.fixed() {
            row_style = row_style.with::<container::height>(height);
        }
        row_style
    });
    view! {
        container(
            style: viewport_style.clone(),
            size_observer: size,
            scroll_controller: list.controller(),
            on_scroll: |cx, event, event_cx| list.on_scroll(cx, event, event_cx),
        ) {
            // The content carries the full scrollable height even though
            // almost none of it is occupied, which keeps the scrollbar honest.
            // Rows stack in normal flow below a leading inset, so a measured
            // row pushes the ones after it within the same layout.
            container(
                style: StyleOf::new()
                    .with::<container::width>(Sizing::Fill)
                    .with::<container::height>(list.total_size())
                    .with::<container::padding>(EdgeInsets::new(0.0, 0.0, list.offset_of(list.range().start), 0.0)),
            ) {
                for index in list.range().iter() {
                    virtual_item(list: list, index: *index, style: row_style.clone()) {
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

    // ---- wheel dispatch ---------------------------------------------------

    use std::time::Instant;
    use xui_core::text::TextHost;
    use xui_cosmic::CosmicEngine;
    use xui_interface::Translation;
    use xui_interface::events::{Modifiers, RawEvent, RawWheel, ScrollDelta};
    fn list_root(app: &mut App) {
        app.mount::<VirtualList>(VirtualListProps {
            item_count: ReadSignal::constant(1_000),
            item_size: ReadSignal::constant(ItemSize::Fixed(20.0)),
            viewport_height: 200.0,
            render_item: ReadSignal::constant(VirtualItemRenderer::new(|index: usize| {
                index.to_string().into()
            })),
            overscan: ReadSignal::constant(3),
            style: StyleOf::new()
                .with::<container::width>(Sizing::Fill)
                .with::<container::height>(200.0),
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
        app.resize(app.main_window(), Size::new(400.0, 200.0));
        let mut text = TextHost::new(CosmicEngine::new(1.0));
        let mut backend = MockRenderBackend::default();
        app.render(app.main_window(), &mut backend, &mut text)
            .expect("mock backend");

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
            app.main_window(),
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
        app.render(app.main_window(), &mut backend, &mut text)
            .expect("mock backend");

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
