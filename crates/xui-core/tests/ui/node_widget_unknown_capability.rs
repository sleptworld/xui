//! A capability that does not exist is reported at its name.

use xui_macros::widget;

#[widget(style(box_size, no_such_capability))]
pub struct Unknown {}

fn main() {}
