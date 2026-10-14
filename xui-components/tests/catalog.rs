//! Exercise every exported component against the retained runtime, including
//! live overlays, both themes, narrow layout, and extreme configuration values.
use std::cell::Cell;
use xui_components::*;
use xui_core::prelude::*;
use xui_core::text::TextHost;
use xui_cosmic::CosmicEngine;
use xui_macros::view;

thread_local! { static CASE: Cell<usize> = const { Cell::new(0) }; }
const COMPONENTS: &[&str] = &[
    "button",
    "input",
    "drop_down",
    "tabs",
    "image",
    "virtual_list",
    "skeleton",
    "spinner",
    "callout",
    "checkbox",
    "switch",
    "radio_group",
    "slider",
    "progress",
    "separator",
    "badge",
    "avatar",
    "card",
    "form_field",
    "dialog",
    "popover",
    "tooltip",
    "drawer",
    "menu",
    "context_menu",
    "toast",
    "collapsible",
    "accordion",
    "breadcrumb",
    "pagination",
    "data_table",
    "data_grid",
    "tree_view",
    "calendar",
    "date_picker",
    "command_palette",
];

fn root(cx: &mut HookContext<'_>) -> ElementDesc {
    let content = view! { text("Content") };
    let trigger = view! { text("Open") };
    let render_item = cx.use_callback((), |index: usize| view! { text(index.to_string()) });
    match CASE.get() {
        0 => view! { xui_components::button(text: "Save", loading: true) },
        1 => view! { xui_components::input(invalid: true, required: true) },
        2 => {
            view! { drop_down(items: vec![DropDownItem::new("a", "Alpha"), DropDownItem::new("b", "Beta").disabled(true)], default_open: true) }
        }
        3 => {
            view! { tabs(items: vec![TabItem::new("a", "Alpha", content)], activation: SelectionActivationMode::Manual, orientation: SelectionOrientation::Vertical) }
        }
        4 => {
            view! { xui_components::image::image(src: "asset://missing.png", alt: Some("Missing".to_owned()), width: 80.0, height: 60.0) }
        }
        5 => {
            view! { virtual_list(item_count: 100_000usize, item_height: 20.0, viewport_height: 100.0, render_item: render_item) }
        }
        6 => view! { skeleton() },
        7 => view! { spinner(paused: true) },
        8 => view! { callout(content: content, title: Some("Note".to_owned())) },
        9 => view! { checkbox(label: Some(content), indeterminate: true) },
        10 => view! { switch(label: Some(content), checked: Some(true)) },
        11 => view! { radio_group(items: vec![RadioItem::new("a", "Alpha", content)]) },
        12 => view! { slider(min: 10.0, max: -10.0, step: -2.0) },
        13 => view! { progress(value: f32::NAN, max: f32::INFINITY) },
        14 => view! { separator() },
        15 => view! { badge(text: "New") },
        16 => view! { avatar(accessibility_label: "Profile", fallback: "AB") },
        17 => view! { card(content: content) },
        18 => {
            view! { form_field(label: "Email", control: content, required: true, status: FieldStatus::Error, message: Some("Required".to_owned())) }
        }
        19 => {
            view! { dialog(content: content, open: Some(true), title: Some("Dialog".to_owned())) }
        }
        20 => view! { popover(trigger: trigger, content: content, default_open: true) },
        21 => view! { tooltip(trigger: trigger, text: "Tip") },
        22 => view! { drawer(content: content, open: Some(true)) },
        23 => {
            view! { menu(trigger: trigger, items: vec![MenuItem::new("a", "Action")], default_open: true) }
        }
        24 => {
            view! { context_menu(trigger: trigger, items: vec![MenuItem::new("a", "Action")], default_open: true) }
        }
        25 => view! { toast(message: "Saved", open: true) },
        26 => view! { collapsible(trigger: trigger, content: content, default_open: true) },
        27 => {
            view! { accordion(items: vec![AccordionItem::new("a", trigger, content)], default_expanded: Some(0)) }
        }
        28 => {
            view! { breadcrumb(items: vec![BreadcrumbItem::new("Home"), BreadcrumbItem::new("Current")]) }
        }
        29 => view! { pagination(page_count: 100usize, page: 99usize, sibling_count: usize::MAX) },
        30 => view! { data_table(rows: vec![TableRow::new("a", vec![content])]) },
        31 => {
            view! { data_grid(columns: vec![DataGridColumn::new("name", "Name")], rows: vec![DataGridRow::new("a", vec!["Alpha".to_owned()])]) }
        }
        32 => view! { tree_view(nodes: vec![TreeNode::leaf("a", "Alpha")]) },
        33 => view! { calendar(month: CalendarMonth { year: 2026, month: 255 }) },
        34 => {
            view! { date_picker(trigger: trigger, month: CalendarMonth::new(2026, 10), default_open: true) }
        }
        35 => {
            view! { command_palette(items: vec![CommandItem::new("open", "Open File")], query: "FILE", open: true) }
        }
        _ => unreachable!(),
    }
}

#[test]
fn every_component_renders_in_both_themes_and_narrow_layout() {
    for (case, name) in COMPONENTS.iter().enumerate() {
        CASE.set(case);
        for theme in [Theme::light(), Theme::dark()] {
            let mut app = App::new(root);
            app.set_theme(theme);
            app.resize(Size::new(280.0, 400.0));
            let mut text = TextHost::new(CosmicEngine::new(1.0));
            let mut backend = MockRenderBackend::default();
            for _ in 0..3 {
                app.render(&mut backend, &mut text)
                    .unwrap_or_else(|error| panic!("{name}: {error:?}"));
            }
            let tree = app.ui_runtime();
            let mut pending = vec![tree.root()];
            while let Some(id) = pending.pop() {
                let node = tree.node(id).unwrap();
                assert!(
                    node.layout.width().is_finite() && node.layout.height().is_finite(),
                    "{name} has invalid geometry"
                );
                pending.extend(tree.children(id));
            }
        }
    }
}
