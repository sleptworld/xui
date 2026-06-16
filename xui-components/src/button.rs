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
            text_color: ColorValue::Color(Color::WHITE),
            background: Color::BLACK.into(),
            border_radius: 4,
            padding: EdgeInsets::symmetric(12.0, 4.0),
        }
    }
}

#[component]
#[defaults(
    ps = ButtonProp::default()
)]
pub fn pbutton(text: &String, ps: &ButtonProp) {
    xui! {
        <container
            background={ps.background}
            border_radius = {ps.border_radius as f32}
            padding = {ps.padding}
            color = {ps.text_color}
            on_click={|_, _| {
                println!("HEllo");
                EventResult::Ignored
            }}

            on_hover_enter={|_,_| {
                println!("ABABA");
                EventResult::Ignored
            }}
            // animation= {(
            //     EventTrigger::OnHover,
            //     Style::new()
            //         .background(Color::BLACK),
            //     AnimationTransition::new(Duration::from_millis(100)),
            // )}
        >
            <text>{text}</text>
        </container>
    }
}
