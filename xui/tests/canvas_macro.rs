use xui::prelude::*;
use xui::widgets::WidgetType;

#[test]
fn xui_macro_expands_canvas_as_a_host_widget() {
    let controller = CanvasController::new();
    let element = xui! {
        <canvas controller={controller.clone()} width={320.0} height={180.0} />
    };

    assert_eq!(element.node_type(), Some(WidgetType::Canvas));
}
