//! `#[widget]` checks its own arguments and attributes.

use xui_macros::widget;

#[widget(styles(box_size))]
pub struct Misspelled {}

#[widget(style(box_size), style(padding))]
pub struct Twice {}

#[widget(style(box_size, box_size))]
pub struct SameCapability {}

#[widget]
pub struct DirtyState {
    #[state]
    #[dirty(PAINT_OUTPUT)]
    shown: f32,
}

fn main() {}
