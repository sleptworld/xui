use std::hash::{Hash, Hasher};

use xui_interface::{
    Bounds, ComputedStyle, EventRef, EventResult, Key, Style, TextContent, TextProps, WidgetType,
    WidgetUpdateFlags,
};

use taffy::{
    Display, Style as TaffyStyle,
    style::{
        GridAutoFlow as TaffyGridAutoFlow, GridTemplateComponent, RepetitionCount,
        TrackSizingFunction,
    },
    style_helpers::{auto, fr, length, max_content, min_content, minmax, repeat},
};

use crate::{
    ElementDesc,
    event_system::{EventContext, interaction::InteractionProperties},
    render::RenderTreeWriter,
    widgets::{EventHandlers, props_hash, widget_element_desc},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridTrackSize {
    Fixed(f32),

    Flexible { min: f32, weight: f32 },

    Bounded { min: f32, max: f32 },

    Auto,
    MinContent,
    MaxContent,
}

impl Hash for GridTrackSize {
    fn hash<H: Hasher>(&self, state: &mut H) {
        fn hash_number<H: Hasher>(value: f32, state: &mut H) {
            // `-0.0 == 0.0`, so both values must also have the same hash.
            if value == 0.0 {
                0_u32.hash(state);
            } else {
                value.to_bits().hash(state);
            }
        }

        std::mem::discriminant(self).hash(state);
        match *self {
            Self::Fixed(px) => hash_number(px, state),
            Self::Flexible { min, weight } => {
                hash_number(min, state);
                hash_number(weight, state);
            }
            Self::Bounded { min, max } => {
                hash_number(min, state);
                hash_number(max, state);
            }
            Self::Auto | Self::MinContent | Self::MaxContent => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GridFlow {
    #[default]
    Row,
    Column,
    RowDense,
    ColumnDense,
}

impl GridTrackSize {
    pub const fn fixed(px: f32) -> Self {
        Self::Fixed(px)
    }

    pub const fn flexible() -> Self {
        Self::Flexible {
            min: 0.0,
            weight: 1.0,
        }
    }

    pub const fn flexible_min(min: f32) -> Self {
        Self::Flexible { min, weight: 1.0 }
    }

    pub const fn flexible_weight(weight: f32) -> Self {
        Self::Flexible { min: 0.0, weight }
    }

    pub const fn bounded(min: f32, max: f32) -> Self {
        Self::Bounded { min, max }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GridTracks {
    Explicit(Vec<GridTrackSize>),
    Repeat { count: usize, track: GridTrackSize },
    Adaptive { min: f32, max: Option<f32> },
}

impl Hash for GridTracks {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Explicit(tracks) => tracks.hash(state),
            Self::Repeat { count, track } => {
                count.hash(state);
                track.hash(state);
            }
            Self::Adaptive { min, max } => {
                GridTrackSize::Fixed(*min).hash(state);
                max.map(GridTrackSize::Fixed).hash(state);
            }
        }
    }
}

impl GridTracks {
    pub fn explicit(tracks: impl IntoIterator<Item = GridTrackSize>) -> Self {
        Self::Explicit(tracks.into_iter().collect())
    }

    pub const fn repeat(count: usize, track: GridTrackSize) -> Self {
        Self::Repeat { count, track }
    }

    pub const fn adaptive(min: f32) -> Self {
        Self::Adaptive { min, max: None }
    }

    pub const fn adaptive_max(min: f32, max: f32) -> Self {
        Self::Adaptive {
            min,
            max: Some(max),
        }
    }
}

pub struct GridWidget {
    pub key: Option<Key>,
    pub style: Style,
    pub event_handlers: EventHandlers,
    pub columns: GridTracks,
    pub rows: Option<GridTracks>,
    pub flow: GridFlow,
    pub interaction: InteractionProperties,
}

impl std::fmt::Debug for GridWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GridWidget")
            .field("key", &self.key)
            .field("columns", &self.columns)
            .field("rows", &self.rows)
            .field("flow", &self.flow)
            .finish()
    }
}

impl GridWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            style: Style::default(),
            columns: GridTracks::Explicit(vec![]),
            rows: None,
            flow: GridFlow::default(),
            event_handlers: EventHandlers::default(),
            interaction: InteractionProperties::default(),
        }
    }

    pub fn columns(mut self, columns: GridTracks) -> Self {
        self.columns = columns;
        self
    }

    pub fn rows(mut self, rows: GridTracks) -> Self {
        self.rows = Some(rows);
        self
    }

    pub fn flow(mut self, flow: GridFlow) -> Self {
        self.flow = flow;
        self
    }

    pub fn columns_count(self, count: usize) -> Self {
        self.columns(GridTracks::repeat(count, GridTrackSize::flexible()))
    }

    pub fn adaptive_columns(self, min: f32) -> Self {
        self.columns(GridTracks::adaptive(min))
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn into_element_desc(self, children: Vec<ElementDesc>) -> ElementDesc {
        widget_element_desc(self, children)
    }

    event_handler_methods!();

    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::Grid
    }

    pub(super) fn get_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    pub(super) fn props_hash(&self) -> u64 {
        props_hash(&(&self.style, &self.columns, &self.rows, self.flow))
    }

    pub(super) fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();

        if self.style != next.style {
            self.style = next.style.clone();
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }

        if self.columns != next.columns {
            self.columns = next.columns.clone();
            flags |= WidgetUpdateFlags::LAYOUT_INPUT;
        }

        if self.rows != next.rows {
            self.rows = next.rows.clone();
            flags |= WidgetUpdateFlags::LAYOUT_INPUT;
        }

        if self.flow != next.flow {
            self.flow = next.flow;
            flags |= WidgetUpdateFlags::LAYOUT_INPUT;
        }

        flags
    }

    pub(super) fn default_style(&self) -> Style {
        Style::new()
    }

    pub(super) fn current_style(&self) -> &Style {
        &self.style
    }

    pub(super) fn render(
        &self,
        _node_id: xui_interface::NodeId,
        _rect: Bounds,
        _style: &ComputedStyle,
        _writer: &mut RenderTreeWriter<'_>,
    ) {
    }

    pub(super) fn handle_event(
        &mut self,
        _event: EventRef<'_>,
        _cx: &mut EventContext<'_>,
    ) -> EventResult {
        EventResult::Ignored
    }

    pub(super) fn text_content(&self) -> Option<TextContent> {
        None
    }

    pub(super) fn text_layout_props(&self, _style: &ComputedStyle) -> Option<TextProps> {
        None
    }
}

impl Default for GridWidget {
    fn default() -> Self {
        Self::new()
    }
}

fn grid_track_to_taffy(track: GridTrackSize) -> TrackSizingFunction {
    match track {
        GridTrackSize::Fixed(px) => length(px),

        GridTrackSize::Flexible { min, weight } => minmax(length(min), fr(weight)),

        GridTrackSize::Bounded { min, max } => minmax(length(min), length(max)),

        GridTrackSize::Auto => auto(),

        GridTrackSize::MinContent => min_content(),

        GridTrackSize::MaxContent => max_content(),
    }
}

fn grid_tracks_to_taffy(tracks: &GridTracks) -> Vec<GridTemplateComponent<String>> {
    match tracks {
        GridTracks::Explicit(tracks) => tracks
            .iter()
            .copied()
            .map(grid_track_to_taffy)
            .map(GridTemplateComponent::Single)
            .collect(),

        GridTracks::Repeat { count, track } => {
            let count = u16::try_from(*count).unwrap_or(u16::MAX);
            vec![repeat(count, vec![grid_track_to_taffy(*track)])]
        }

        GridTracks::Adaptive { min, max: None } => {
            vec![repeat(
                RepetitionCount::AutoFit,
                vec![minmax(length(*min), fr(1.0))],
            )]
        }

        GridTracks::Adaptive {
            min,
            max: Some(max),
        } => {
            vec![repeat(
                RepetitionCount::AutoFit,
                vec![minmax(length(*min), length(*max))],
            )]
        }
    }
}

impl From<GridFlow> for TaffyGridAutoFlow {
    fn from(flow: GridFlow) -> Self {
        match flow {
            GridFlow::Row => Self::Row,
            GridFlow::Column => Self::Column,
            GridFlow::RowDense => Self::RowDense,
            GridFlow::ColumnDense => Self::ColumnDense,
        }
    }
}

pub fn grid_widget_to_taffy(style: &mut TaffyStyle, widget: &GridWidget) {
    style.display = Display::Grid;
    style.grid_template_columns = grid_tracks_to_taffy(&widget.columns);

    style.grid_template_rows = widget
        .rows
        .as_ref()
        .map(grid_tracks_to_taffy)
        .unwrap_or_default();

    style.grid_auto_flow = widget.flow.into();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_every_track_size_variant() {
        let tracks = GridTracks::explicit([
            GridTrackSize::fixed(24.0),
            GridTrackSize::Flexible {
                min: 12.0,
                weight: 2.0,
            },
            GridTrackSize::bounded(30.0, 80.0),
            GridTrackSize::Auto,
            GridTrackSize::MinContent,
            GridTrackSize::MaxContent,
        ]);

        assert_eq!(
            grid_tracks_to_taffy(&tracks),
            vec![
                GridTemplateComponent::Single(length(24.0)),
                GridTemplateComponent::Single(minmax(length(12.0), fr(2.0))),
                GridTemplateComponent::Single(minmax(length(30.0), length(80.0))),
                GridTemplateComponent::Single(auto()),
                GridTemplateComponent::Single(min_content()),
                GridTemplateComponent::Single(max_content()),
            ]
        );
    }

    #[test]
    fn converts_repeat_and_saturates_large_counts() {
        let tracks = GridTracks::repeat(usize::MAX, GridTrackSize::fixed(10.0));
        let converted = grid_tracks_to_taffy(&tracks);

        let GridTemplateComponent::Repeat(repetition) = &converted[0] else {
            panic!("repeat tracks must convert to a Taffy repetition");
        };
        assert_eq!(repetition.count, RepetitionCount::Count(u16::MAX));
        assert_eq!(repetition.tracks, vec![length(10.0)]);
    }

    #[test]
    fn converts_adaptive_tracks_and_all_flow_modes() {
        let cases = [
            (GridFlow::Row, TaffyGridAutoFlow::Row),
            (GridFlow::Column, TaffyGridAutoFlow::Column),
            (GridFlow::RowDense, TaffyGridAutoFlow::RowDense),
            (GridFlow::ColumnDense, TaffyGridAutoFlow::ColumnDense),
        ];

        for (flow, expected_flow) in cases {
            let widget = GridWidget::new()
                .columns(GridTracks::adaptive(20.0))
                .rows(GridTracks::adaptive_max(10.0, 40.0))
                .flow(flow);
            let mut style = TaffyStyle::default();
            grid_widget_to_taffy(&mut style, &widget);

            assert_eq!(style.display, Display::Grid);
            assert_eq!(style.grid_auto_flow, expected_flow);
            assert_eq!(
                style.grid_template_columns,
                vec![repeat(
                    RepetitionCount::AutoFit,
                    vec![minmax(length(20.0), fr(1.0))]
                )]
            );
            assert_eq!(
                style.grid_template_rows,
                vec![repeat(
                    RepetitionCount::AutoFit,
                    vec![minmax(length(10.0), length(40.0))]
                )]
            );
        }
    }

    fn adaptive_card_locations(width: f32) -> Vec<taffy::Point<f32>> {
        let widget = GridWidget::new().adaptive_columns(200.0);
        let mut style = TaffyStyle::default();
        grid_widget_to_taffy(&mut style, &widget);
        style.size.width = taffy::Dimension::percent(1.0);
        style.gap = taffy::Size {
            width: taffy::LengthPercentage::length(12.0),
            height: taffy::LengthPercentage::length(12.0),
        };

        let mut tree = taffy::TaffyTree::<()>::new();
        let children = (0..4)
            .map(|_| {
                tree.new_leaf(TaffyStyle {
                    size: taffy::Size {
                        width: taffy::Dimension::auto(),
                        height: taffy::Dimension::length(100.0),
                    },
                    ..Default::default()
                })
                .unwrap()
            })
            .collect::<Vec<_>>();
        let root = tree.new_with_children(style, &children).unwrap();
        tree.compute_layout(
            root,
            taffy::Size {
                width: taffy::AvailableSpace::Definite(width),
                height: taffy::AvailableSpace::MaxContent,
            },
        )
        .unwrap();

        children
            .into_iter()
            .map(|child| tree.layout(child).unwrap().location)
            .collect()
    }

    #[test]
    fn adaptive_four_card_grid_wraps_from_one_row_to_two() {
        let wide = adaptive_card_locations(860.0);
        assert!(wide.iter().all(|location| location.y == 0.0));
        assert!(wide.windows(2).all(|pair| pair[0].x < pair[1].x));

        let narrow = adaptive_card_locations(430.0);
        assert_eq!(narrow[0].y, narrow[1].y);
        assert_eq!(narrow[2].y, narrow[3].y);
        assert!(narrow[2].y > narrow[0].y);
        assert_eq!(narrow[0].x, narrow[2].x);
        assert_eq!(narrow[1].x, narrow[3].x);
    }

    #[test]
    fn flow_changes_are_layout_changes_and_affect_props_hash() {
        let mut current = GridWidget::new();
        let next = GridWidget::new().flow(GridFlow::ColumnDense);
        let previous_hash = current.props_hash();

        let flags = current.update_from(&next);

        assert!(flags.contains(WidgetUpdateFlags::LAYOUT_INPUT));
        assert_eq!(current.flow, GridFlow::ColumnDense);
        assert_ne!(previous_hash, next.props_hash());
    }
}
