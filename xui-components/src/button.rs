use std::time::Duration;
use xui::prelude::*;

#[derive(Debug, Hash)]
pub struct ButtonProp {
    pub padding: EdgeInsets,
    pub background: ColorStyle,
    pub border_radius: u8,
    pub text_color: ColorValue,
}

impl Default for ButtonProp {
    fn default() -> Self {
        Self {
            text_color: ColorValue::Color(Color::BLACK),
            background: Color::hex("#f5f5f5").into(),
            border_radius: 4,
            padding: EdgeInsets::symmetric(12.0, 4.0),
        }
    }
}

#[component]
#[defaults(
    ps = ButtonProp::default()
)]
pub fn button(text: &String, ps: &ButtonProp) {
    xui! {
        <container
            background={ps.background}
            border_radius = {ps.border_radius as f32}
            padding = {ps.padding}
            font_color = {px.text_color}
            font_weight= {FontWeight::Bold}
            style={Style::new().when(WidgetState::HOVERED, |s| s.background(Color::hex("#f5f5f5").alpha(0.8)))}
            transition={Transition::new(Duration::from_millis(100))}
            on_click={|_, _| {
                println!("HEllo");
                EventResult::Ignored
            }}

            on_hover_enter={|_,_| {
                println!("ABABA");
                EventResult::Ignored
            }}
        >
            <text>{text}</text>
        </container>
    }
}
