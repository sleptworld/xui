use xui::prelude::*;
use xui::text::TextHost;
use xui_components::*;
use xui_cosmic::CosmicEngine;

fn text(value: impl Into<String>) -> ElementDesc {
    TextWidget::new(value.into()).into_element_desc()
}

fn catalog_root(cx: &mut HookContext<'_>) -> ElementDesc {
    let controller = cx.use_memo(TextController::new).get().clone();
    let rows = vec![DataGridRow::new("one", ["Ada", "Engineer"])];
    let columns = vec![
        DataGridColumn::new("name", "Name").sortable(true),
        DataGridColumn::new("role", "Role"),
    ];
    let accordion_items = vec![AccordionItem::new(
        "section",
        text("Section"),
        text("Section content"),
    )];
    let radio_items = vec![RadioItem::new("one", "One", text("One"))];
    let menu_items = vec![MenuItem::new("open", "Open")];
    let commands = vec![CommandItem::new("save", "Save")];
    let tree = vec![TreeNode::branch(
        "root",
        "Root",
        vec![TreeNode::leaf("leaf", "Leaf")],
    )];

    let controls = vec![
        xui! { <checkbox label={Some(text("Checkbox"))} /> },
        xui! { <switch label={Some(text("Switch"))} /> },
        xui! { <radio_group items={radio_items} /> },
        xui! { <slider accessibility_label={Some("Volume".to_owned())} /> },
        xui! { <progress value={42.0} accessibility_label={Some("Progress".to_owned())} /> },
        xui! { <text_area controller={controller} accessibility_label={Some("Notes".to_owned())} /> },
        xui! { <accordion items={accordion_items} default_expanded={Some(0)} /> },
        xui! { <calendar month={CalendarMonth::new(2026, 9)} /> },
        xui! { <data_grid columns={columns} rows={rows} /> },
        xui! { <tree_view nodes={tree} /> },
        xui! { <context_menu trigger={text("Context target")} items={menu_items.clone()} /> },
        xui! { <menu trigger={text("Menu target")} items={menu_items} /> },
        xui! { <command_palette items={commands} open={true} /> },
        xui! { <form_field label={"Email".to_owned()} control={text("Control")} message={Some("Required".to_owned())} status={FieldStatus::Error} /> },
        xui! { <skeleton /> },
        xui! { <spinner /> },
        xui! { <callout content={text("Take care")} tone={CalloutTone::Warning} /> },
    ];
    ContainerWidget::new()
        .style(
            Style::new()
                .width(Sizing::Fill)
                .height(1800.0)
                .gap(12.0)
                .scroll_vertical(),
        )
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(controls)
}

#[test]
fn production_catalog_mounts_renders_and_exports_semantics() {
    let mut app = App::new(catalog_root);
    app.resize(Size::new(900.0, 700.0));
    let mut backend = MockRenderBackend::default();
    let mut text_host = TextHost::new(CosmicEngine::new(1.0));
    app.render(&mut backend, &mut text_host)
        .expect("the complete component catalog should render");

    let roles = app
        .ui_runtime()
        .accessibility_tree()
        .nodes
        .into_iter()
        .filter_map(|node| node.properties.role)
        .collect::<Vec<_>>();
    for expected in [
        AccessibilityRole::Checkbox,
        AccessibilityRole::Switch,
        AccessibilityRole::RadioGroup,
        AccessibilityRole::Slider,
        AccessibilityRole::ProgressIndicator,
        AccessibilityRole::TextField,
        AccessibilityRole::Table,
        AccessibilityRole::ColumnHeader,
        AccessibilityRole::Tree,
        AccessibilityRole::Dialog,
    ] {
        assert!(
            roles.contains(&expected),
            "missing accessibility role {expected:?}"
        );
    }
}
