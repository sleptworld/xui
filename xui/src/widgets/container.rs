use xui_interface::{Color, NodeId, PaintCommand, Widget};

#[derive(Debug)]
pub struct Container {
    fill_color: Option<Color>,
    stroke_color: Option<Color>,
    stroke_width: f32,
    border_radius: f32,
    zorder: i32,
    shadow: Option<[f32; 4]>,
    children: Vec<NodeId>,
}
