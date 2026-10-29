use xui_core::prelude::*;
use xui_core::widgets::WidgetType;
use xui_macros::component;

#[component]
fn drawing(controller: &CanvasController) {
    xui! { <canvas controller={controller.clone()} width={320.0} height={180.0} /> }
}

#[test]
fn a_component_mounts_canvas_as_a_host_widget() {
    let mut app = App::new(|app| {
        app.mount::<Drawing>(DrawingProps { controller: CanvasController::new() });
    });
    let root = app.ui_runtime().root();
    let host = app.ui_runtime().children(root).next().unwrap();
    assert_eq!(app.ui_runtime().node(host).unwrap().node_type, WidgetType::Canvas);
    let _ = &mut app;
}
