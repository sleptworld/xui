use std::time::Duration;
use xui_core::prelude::*;
use xui_macros::{component, style};

/// A transition is the node's `motion`, not part of a style: a style can be
/// shared by nodes that move differently, and a state rule cannot change how
/// the node moves.
#[component]
fn transitioned() {
    let transition: Transition = Transition::new(Duration::from_millis(120)).ease(Easing::CubicOut);
    xui! {
        <column>
            <container
                style={style!(container { background: Color::BLACK, border_radius: 4.0 })}
                transition={*transition}
            />
            <container background={Color::WHITE} transition={*transition} />
            <text transition={*transition}>{"transitioned text"}</text>
            <container
                translate_y={2.0}
                scale={0.98}
                rotate={0.05}
                transform_origin={Point::new(0.5, 0.5)}
                transition={*transition}
            />
        </column>
    }
}

#[test]
fn a_transition_is_an_attribute_beside_the_style() {
    let app = App::new(|app| {
        app.mount::<Transitioned>(TransitionedProps {});
    });
    let root = app.ui_runtime().root();
    let column = app.ui_runtime().children(root).next().unwrap();
    assert_eq!(app.ui_runtime().children(column).count(), 4);
}
