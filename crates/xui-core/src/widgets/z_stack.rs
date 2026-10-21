//! Children stacked on one another in a single cell.

use xui_interface::Alignment;
use xui_macros::widget;

use crate::node::{Draft, WidgetT};

/// A SwiftUI-style overlay container. Children share one grid cell and are
/// painted in declaration order, later children on top.
#[widget(
    style(
        box_size, margin, padding, flex_layout, position, display, typography, background,
        corner_radius, border, box_shadow, clip, visibility, transform, effects, scroll, cursor
    ),
    behavior(
        focus, accessibility, shortcuts, motion, focus_binding, scroll_binding, size_observation,
        events
    )
)]
#[props(derive(Debug, Clone, PartialEq))]
pub struct ZStack {
    /// Where each child sits in the shared cell.
    #[default(Alignment::CENTER)]
    #[dirty(LAYOUT_INPUT)]
    pub alignment: Alignment,
}

impl WidgetT for ZStack {
    type State = ();

    fn create_state(_: &ZStackProps) {}
}

pub fn z_stack() -> Draft<ZStack> {
    Draft::new(ZStackProps::builder().build())
}
