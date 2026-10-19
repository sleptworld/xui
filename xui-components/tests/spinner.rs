//! The spinner turns on its own, frame by frame, and stops asking for frames
//! when nobody can see it.

use std::f32::consts::FRAC_PI_2;
use std::time::{Duration, Instant};

use xui_components::Spinner;
use xui_core::prelude::*;
use xui_core::widgets::WidgetType;
use xui_cosmic::CosmicEngine;
use xui_macros::xui;

fn root(_cx: &mut HookContext<'_>) -> ElementDesc {
    xui! { <Spinner size={24.0} /> }
}

fn runtime() -> GuiRuntime<MockRenderBackend, CosmicEngine> {
    let mut runtime = GuiRuntime::new(
        App::new(root),
        MockRenderBackend::default(),
        CosmicEngine::new(1.0),
    );
    runtime.handle_event(RuntimeEvent::Resize(Size::new(200.0, 200.0)));
    runtime
}

/// The icon's rotation as the frame shows it.
fn rotation(runtime: &GuiRuntime<MockRenderBackend, CosmicEngine>) -> f32 {
    let tree = runtime.app().ui_runtime();
    let mut pending = vec![tree.root()];
    while let Some(id) = pending.pop() {
        let node = tree.node(id).expect("a child of a live node is live");
        if node.node_type == WidgetType::Icon {
            return node.effective_style.transform.rotate;
        }
        pending.extend(tree.children(id));
    }
    panic!("the spinner has no icon");
}

#[test]
fn the_spinner_turns_every_frame_while_it_can_be_seen() {
    let mut runtime = runtime();
    let t0 = Instant::now();
    runtime.frame_at(t0).unwrap();
    assert_eq!(rotation(&runtime), 0.0);
    assert!(
        runtime.app().is_dirty(),
        "a spinner asks for the next frame"
    );

    // 200ms in 50ms frames: a quarter of an 800ms turn.
    for step in 1..=4 {
        let report = runtime
            .frame_at(t0 + Duration::from_millis(50 * step))
            .unwrap();
        assert!(report.rendered);
    }
    assert!(
        (rotation(&runtime) - FRAC_PI_2).abs() < 1e-3,
        "{}",
        rotation(&runtime)
    );

    runtime.set_window_visible(false);
    assert!(
        !runtime.app().is_dirty(),
        "nothing to show, nothing to ask for"
    );
}
