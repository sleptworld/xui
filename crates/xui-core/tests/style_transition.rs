use std::time::Duration;
use xui_core::prelude::*;
use xui_macros::{style, xui};

/// A transition is the node's `motion`, not part of a style: a style can be
/// shared by nodes that move differently, and a state rule cannot change how
/// the node moves.
#[test]
fn a_transition_is_an_attribute_beside_the_style() {
    let transition = Transition::new(Duration::from_millis(120)).ease(Easing::CubicOut);
    let style = style!(container {
        background: Color::BLACK,
        border_radius: 4.0,
    });

    let _: ElementDesc = xui! { <container style={style} transition={transition} /> };
    let _: ElementDesc = xui! {
        <container
            background={Color::WHITE}
            transition={transition}
        />
    };
    let _: ElementDesc = xui! {
        <text transition={transition}>
            {"transitioned text"}
        </text>
    };
    let _: ElementDesc = xui! {
        <container
            translate_y={2.0}
            scale={0.98}
            rotate={0.05}
            transform_origin={Point::new(0.5, 0.5)}
            transition={transition}
        />
    };
}
