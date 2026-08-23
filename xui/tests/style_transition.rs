use std::time::Duration;
use xui::prelude::*;

#[test]
fn style_owned_transition_compiles_for_builder_and_xui_sugar() {
    let transition = Transition::new(Duration::from_millis(120)).ease(Easing::CubicOut);
    let style = style! {
        background: Color::BLACK,
        border_radius: 4.0,
    }
    .transition(transition);

    let _: ElementDesc = xui! { <container style={style} /> };
    let _: ElementDesc = xui! {
        <container
            background={Color::WHITE}
            transition={transition}
        />
    };
    let _: ElementDesc = xui! {
        <text style={Style::new().transition(transition)}>
            {"style-owned transition"}
        </text>
    };
}
