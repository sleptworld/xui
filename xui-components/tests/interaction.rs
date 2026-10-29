use std::cell::{Cell, RefCell};
use std::time::Instant;
use xui_components::*;
use xui_core::prelude::*;
use xui_cosmic::CosmicEngine;
use xui_interface::events::*;
use xui_macros::component;

thread_local! {
    static CASE: Cell<usize> = const { Cell::new(0) };
    static VALUES: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    static CONTROLLER: TextController = TextController::with_text("initial");
}
fn record(value: f32) {
    VALUES.with(|v| v.borrow_mut().push(value));
}

#[component]
fn interaction() {
    let case: usize = CASE.get();
    view! {
        column(width: Sizing::Fill, height: Sizing::Fill) {
            match *case {
                0 => checkbox(disabled: true, label: view! { text("Label") }, on_change: |_cx, value| record(if value { 1.0 } else { 0.0 })),
                1 => checkbox(checked: Some(true), on_change: |_cx, value| record(if value { 1.0 } else { 0.0 })),
                2 => radio_group(
                    orientation: SelectionOrientation::Horizontal,
                    items: vec![RadioItem::new("a", "A", "A"), RadioItem::new("b", "B", "B").disabled(true), RadioItem::new("c", "C", "C")],
                    on_change: |_cx, value| record(value as f32),
                ),
                3 => slider(min: 10.0, max: 0.0, step: -2.0, default_value: 4.0, on_change: |_cx, value| record(value)),
                4 => xui_components::input(controller: Some(CONTROLLER.with(Clone::clone)), read_only: true),
                5 => xui_components::input(disabled: true, controller: Some(CONTROLLER.with(Clone::clone))),
                6 => xui_components::input(),
                7 => drop_down(
                    items: vec![DropDownItem::new("a", "A"), DropDownItem::new("b", "B").disabled(true), DropDownItem::new("c", "C")],
                    on_change: |_cx, value| record(value as f32),
                ),
                8 => menu(
                    trigger: view! { text("Open") },
                    items: vec![MenuItem::new("a", "A"), MenuItem::new("b", "B").disabled(true), MenuItem::new("c", "C")],
                    default_open: true,
                    on_select: |_cx, value| record(value as f32),
                ),
                9 => tree_view(
                    nodes: vec![TreeNode::branch("a", "A", vec![TreeNode::leaf("b", "B")])],
                    expanded: Some(Vec::new()),
                    on_expanded_change: |_cx, ids: Vec<String>| record(ids.len() as f32),
                ),
                10 => virtual_list(
                    item_count: 100_000,
                    item_height: 20.0,
                    viewport_height: 100.0,
                    render_item: VirtualItemRenderer::new(|index: usize| index.to_string().into()),
                    style: StyleOf::new().with::<container::height>(Sizing::Fill),
                ),
                _ => container(),
            }
        }
    }
}

fn root(app: &mut App) {
    app.mount::<Interaction>(InteractionProps {});
}

struct Harness {
    runtime: GuiRuntime<MockRenderBackend, CosmicEngine>,
}
impl Harness {
    fn new(case: usize) -> Self {
        CASE.set(case);
        VALUES.with(|v| v.borrow_mut().clear());
        let mut runtime = GuiRuntime::new(
            App::new(root),
            MockRenderBackend::default(),
            CosmicEngine::new(1.0),
        );
        runtime.handle_event(RuntimeEvent::Resize(Size::new(400.0, 200.0)));
        let mut h = Self { runtime };
        h.render();
        h
    }
    fn render(&mut self) {
        self.runtime.frame().unwrap();
    }
    fn event(&mut self, event: RawEvent) {
        self.runtime.handle_event(RuntimeEvent::Input(event));
        self.render();
    }
    fn click(&mut self) {
        let event = RawPointerButton {
            position: Point::new(10.0, 10.0),
            pointer_id: XuiPointerId::new(0),
            device_id: None,
            kind: PointerKind::Mouse,
            button: PointerButton::Primary,
            buttons: PointerButtons::default(),
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
        };
        self.event(RawEvent::PointerDown(event.clone()));
        self.event(RawEvent::PointerUp(event));
    }
    fn key(&mut self, key: NamedKey) {
        self.event(keyboard(Some(key), None));
    }
    fn input_text(&self) -> String {
        let tree = self.runtime.app().ui_runtime();
        let mut stack = vec![tree.root()];
        while let Some(id) = stack.pop() {
            let node = tree.node(id).unwrap();
            if node.node_type == xui_core::widgets::WidgetType::TextInput {
                return node.widget.text().unwrap().as_str().to_owned();
            }
            stack.extend(tree.children(id));
        }
        panic!("input missing")
    }
}
fn keyboard(key: Option<NamedKey>, text: Option<&str>) -> RawEvent {
    RawEvent::Keyboard(RawKeyboard {
        physical_key: PhysicalKey::Unidentified,
        named_key: key,
        state: KeyState::Down,
        text: text.and_then(KeyText::try_new),
        modifiers: Modifiers::default(),
        timestamp: Instant::now(),
        is_repeat: false,
    })
}
fn values() -> Vec<f32> {
    VALUES.with(|v| v.borrow().clone())
}
#[test]
fn disabled_checkbox_ignores_pointer_and_keyboard_activation() {
    let mut h = Harness::new(0);
    h.click();
    h.key(NamedKey::Tab);
    h.key(NamedKey::Space);
    assert!(values().is_empty());
}
#[test]
fn controlled_checkbox_requests_changes_without_overwriting_value() {
    let mut h = Harness::new(1);
    h.click();
    h.click();
    assert_eq!(values(), [0.0, 0.0]);
}
#[test]
fn radio_arrows_skip_disabled_items_and_notify_in_both_directions() {
    let mut h = Harness::new(2);
    h.key(NamedKey::Tab);
    h.key(NamedKey::ArrowRight);
    h.key(NamedKey::ArrowLeft);
    assert_eq!(values(), [2.0, 0.0]);
}
#[test]
fn slider_normalizes_reversed_bounds_and_negative_steps() {
    let mut h = Harness::new(3);
    h.key(NamedKey::Tab);
    h.key(NamedKey::ArrowRight);
    h.key(NamedKey::End);
    assert_eq!(values(), [6.0, 10.0]);
}
#[test]
fn disabled_and_read_only_input_reject_typing_and_deletion() {
    for case in [4, 5] {
        let mut h = Harness::new(case);
        h.click();
        h.event(keyboard(None, Some("x")));
        h.key(NamedKey::Backspace);
        assert_eq!(h.input_text(), "initial");
    }
}
#[test]
fn default_input_controller_survives_parent_rebuilds() {
    let mut h = Harness::new(6);
    h.click();
    h.event(keyboard(None, Some("x")));
    assert_eq!(h.input_text(), "x");
    h.runtime.app_mut().mark_needs_rebuild();
    h.render();
    assert_eq!(h.input_text(), "x");
}
#[test]
fn dropdown_keyboard_navigation_skips_disabled_options() {
    let mut h = Harness::new(7);
    h.key(NamedKey::Tab);
    h.key(NamedKey::ArrowDown);
    assert_eq!(values(), [2.0]);
}

#[test]
fn menu_arrows_move_focus_past_disabled_items_before_activation() {
    let mut h = Harness::new(8);
    h.key(NamedKey::Tab); // trigger
    h.key(NamedKey::Tab); // first menu item
    h.key(NamedKey::ArrowDown);
    h.key(NamedKey::Enter);
    assert_eq!(values(), [2.0]);
}
#[test]
fn controlled_tree_expansion_only_requests_changes() {
    let mut h = Harness::new(9);
    h.click();
    h.click();
    assert_eq!(values(), [1.0, 1.0]);
}
#[test]
fn virtual_list_follows_viewport_resize_without_a_wheel_event() {
    let mut h = Harness::new(10);
    h.render();
    let rows = |h: &Harness| {
        let tree = h.runtime.app().ui_runtime();
        let column = tree.children(tree.root()).next().unwrap();
        let scroller = tree.children(column).next().unwrap();
        let spacer = tree.children(scroller).next().unwrap();
        tree.children(spacer).count()
    };
    let before = rows(&h);
    h.runtime
        .handle_event(RuntimeEvent::Resize(Size::new(400.0, 400.0)));
    h.render();
    h.render();
    let after = rows(&h);
    assert!(
        after > before,
        "resizing did not extend the visible window: {before} -> {after}"
    );
    assert!(after < 30, "resizing mounted too many rows: {after}");
}
