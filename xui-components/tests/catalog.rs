//! Exercise every exported component against the retained runtime, including
//! live overlays, both themes, narrow layout, and extreme configuration values.
use std::cell::Cell;
use xui_components::*;
use xui_core::prelude::*;
use xui_core::text::TextHost;
use xui_cosmic::CosmicEngine;
use xui_components::image::{ImageSize, ImageSrc};
use xui_macros::component;

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

#[component]
fn catalog() {
    let case: usize = CASE.get();
    let render_item: VirtualItemRenderer = VirtualItemRenderer::new(|index: usize| index.to_string().into());
    view! {
        column {
            match *case {
                0 => xui_components::button(text: "Save".to_string(), loading: true),
                1 => xui_components::input(invalid: true, required: true),
                2 => drop_down(
                    items: vec![DropDownItem::new("a", "Alpha"), DropDownItem::new("b", "Beta").disabled(true)],
                    default_open: true,
                ),
                3 => tabs(
                    items: vec![TabItem::new("a", "Alpha", "Content")],
                    activation: SelectionActivationMode::Manual,
                    orientation: SelectionOrientation::Vertical,
                ),
                4 => xui_components::image::image(
                    src: ImageSrc::from("asset://missing.png"),
                    alt: Some("Missing".to_owned()),
                    width: ImageSize::from(80.0),
                    height: ImageSize::from(60.0),
                ),
                5 => virtual_list(item_count: 100_000, item_height: 20.0, viewport_height: 100.0, render_item: render_item.clone()),
                6 => skeleton(),
                7 => spinner(paused: true),
                8 => callout(title: Some("Note".to_owned())) { text("Content") },
                9 => checkbox(label: view! { text("Content") }, indeterminate: true),
                10 => switch(label: view! { text("Content") }, checked: Some(true)),
                11 => radio_group(items: vec![RadioItem::new("a", "Alpha", "Content")]),
                12 => slider(min: 10.0, max: -10.0, step: -2.0),
                13 => progress(value: f32::NAN, max: f32::INFINITY),
                14 => separator(),
                15 => badge(text: "New".to_string()),
                16 => avatar(accessibility_label: "Profile".to_string(), fallback: "AB".to_string()),
                17 => card { text("Content") },
                18 => form_field(
                    label: "Email".to_string(),
                    required: true,
                    status: FieldStatus::Error,
                    message: Some("Required".to_owned()),
                ) {
                    text("Content")
                },
                19 => dialog(open: Some(true), title: Some("Dialog".to_owned())) { text("Content") },
                20 => popover(trigger: view! { text("Open") }, default_open: true) { text("Content") },
                21 => tooltip(trigger: view! { text("Open") }, text: "Tip".to_string()),
                22 => drawer(open: Some(true)) { text("Content") },
                23 => menu(trigger: view! { text("Open") }, items: vec![MenuItem::new("a", "Action")], default_open: true),
                24 => context_menu(trigger: view! { text("Open") }, items: vec![MenuItem::new("a", "Action")], default_open: true),
                25 => toast(message: "Saved".to_string(), open: true),
                26 => collapsible(trigger: view! { text("Open") }, default_open: true) { text("Content") },
                27 => accordion(items: vec![AccordionItem::new("a", "Open", "Content")], default_expanded: Some(0)),
                28 => breadcrumb(items: vec![BreadcrumbItem::new("Home"), BreadcrumbItem::new("Current")]),
                29 => pagination(page_count: 100, page: 99, sibling_count: usize::MAX),
                30 => data_table(rows: vec![TableRow::new("a", ["Content"])]),
                31 => data_grid(
                    columns: vec![DataGridColumn::new("name", "Name")],
                    rows: vec![DataGridRow::new("a", ["Alpha"])],
                ),
                32 => tree_view(nodes: vec![TreeNode::leaf("a", "Alpha")]),
                33 => calendar(month: CalendarMonth { year: 2026, month: 255 }),
                34 => date_picker(trigger: view! { text("Open") }, month: CalendarMonth::new(2026, 10), default_open: true),
                35 => command_palette(items: vec![CommandItem::new("open", "Open File")], query: "FILE".to_string(), open: true),
                _ => container(),
            }
        }
    }
}

fn root(app: &mut App) {
    app.mount::<Catalog>(CatalogProps {});
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
