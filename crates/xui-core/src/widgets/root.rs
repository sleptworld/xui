//! The node every tree hangs from.

use xui_interface::Color;
use xui_interface::core::Sizing;
use xui_macros::widget;

use crate::node::{Draft, WidgetT};

/// The root: it fills the window, white under everything else, and lays its
/// children out in a column.
#[widget(style(box_size, background))]
#[props(derive(Debug, Clone, PartialEq))]
pub struct Root {}

impl WidgetT for Root {
    type State = ();

    fn create_state(_: &RootProps) {}
}

pub(crate) fn root() -> Draft<Root> {
    let full = Sizing::Percent(1.0.try_into().expect("1.0 is a number"));
    Draft::new(RootProps::builder().build())
        .with::<root::width>(full)
        .with::<root::height>(full)
        .with::<root::background>(Color::WHITE)
}
