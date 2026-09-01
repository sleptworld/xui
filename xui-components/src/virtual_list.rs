use xui::prelude::*;

/// Builds the row at a given index. Only indices near the viewport are ever
/// asked for, so the cost of a list stops depending on how long it is.
pub type VirtualItemRenderer = Callback<usize, ElementDesc>;

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
    if item_count == 0 || item_height <= 0.0 || viewport_height <= 0.0 {
        return VirtualRange { start: 0, end: 0 };
    }
    let first_visible = (scroll_top.max(0.0) / item_height).floor() as usize;
    // `+ 1` covers the row straddling the bottom edge, and a second row is
    // needed when the first one is only partly scrolled past the top.
    let span = (viewport_height / item_height).ceil() as usize + 2;
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

/// A scrolling list that only builds the rows near the viewport.
///
/// The runtime already translates a scroll container's content on its own, so
/// scrolling itself never re-renders this component: the rows sit at absolute
/// offsets inside a spacer as tall as the whole list, and a re-render happens
/// only when the visible range actually changes.
///
/// `viewport_height` seeds the first frame, after which the list uses the
/// height it measures from its own layout, so a stale hint corrects itself
/// rather than leaving gaps.
#[component]
#[defaults(
    overscan = 3,
    style = Style::new(),
    item_style = Style::new(),
)]
pub fn virtual_list(
    item_count: &usize,
    item_height: &f32,
    viewport_height: &f32,
    render_item: &VirtualItemRenderer,
    overscan: &usize,
    style: &Style,
    item_style: &Style,
) {
    let scroll_top = cx.use_state(|| 0.0f32);
    // Zero until the first layout is observed; the prop is the fallback.
    let measured_height = cx.use_state(|| 0.0f32);

    let item_count = *item_count;
    let item_height = *item_height;
    let overscan = *overscan;
    let height_hint = *viewport_height;
    let measured = *measured_height.get();
    let viewport = if measured > 0.0 {
        measured
    } else {
        height_hint
    };

    let range = visible_range(
        *scroll_top.get(),
        viewport,
        item_height,
        item_count,
        overscan,
    );
    let total_height = item_height * item_count as f32;

    let mut rows = Vec::with_capacity(range.len());
    for index in range.start..range.end {
        let row = render_item.call(index);
        rows.push(
            ContainerWidget::new()
                // Keyed by index so reconciliation reuses a row's hosts when
                // the range slides instead of tearing them down.
                .key(format!("xui-virtual-row-{index}"))
                .style(
                    item_style
                        .clone()
                        .absolute()
                        .inset(EdgeInsets::new(0.0, 0.0, index as f32 * item_height, 0.0))
                        .width(Sizing::Fill)
                        .height(item_height),
                )
                .into_element_desc(vec![row]),
        );
    }

    // The spacer carries the full scrollable height even though almost none of
    // it is occupied, which is what keeps the scrollbar honest.
    let content = ContainerWidget::new()
        .key("xui-virtual-content")
        .style(
            Style::new()
                .relative()
                .width(Sizing::Fill)
                .height(total_height),
        )
        .into_element_desc(rows);

    ContainerWidget::new()
        .style(style.clone().scroll_vertical())
        .on_scroll(move |event, event_cx| {
            let offset = event.offset_after.unwrap().y;
            let layout_height = event_cx.node_ref.layout.height();
            let viewport = if layout_height > 0.0 {
                layout_height
            } else {
                viewport
            };

            // The runtime has already applied the scroll and moved the content,
            // so a re-render is only warranted when different rows are needed.
            // The measured height is folded into that same decision rather than
            // stored eagerly: learning it is pointless if the window is
            // unchanged, and storing it would cost a render of its own.
            let next = visible_range(offset, viewport, item_height, item_count, overscan);
            if next != range {
                scroll_top.set(offset);
                if layout_height > 0.0 && (layout_height - measured).abs() > 0.5 {
                    measured_height.set(layout_height);
                }
            }
            EventResult::Ignored
        })
        .into_element_desc(vec![content])
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
}
