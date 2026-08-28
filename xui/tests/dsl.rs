//! Behavioural tests for the element DSL.
//!
//! These replace the old token-string assertions (`expanded.contains("xui ::
//! canvas (...)")`), which broke on formatting changes and could not tell
//! whether an attribute actually reached the widget. Compile-failure behaviour
//! is covered separately by `tests/ui`.

use xui::prelude::*;
use xui::widgets::WidgetType;
use xui::{component, xui};

fn host(element: &ElementDesc) -> &WidgetDesc {
    match element {
        ElementDesc::Host(desc) => desc,
        other => panic!("expected a host element, got {other:?}"),
    }
}

#[test]
fn host_tags_resolve_to_host_widgets_not_components() {
    let controller = CanvasController::new();
    for (element, expected) in [
        (xui! { <container /> }, WidgetType::Container),
        (xui! { <grid /> }, WidgetType::Grid),
        (xui! { <z_stack /> }, WidgetType::ZStack),
        (xui! { <text /> }, WidgetType::Text),
        (xui! { <icon /> }, WidgetType::Icon),
        (
            xui! { <canvas controller={controller.clone()} /> },
            WidgetType::Canvas,
        ),
    ] {
        assert_eq!(host(&element).widget.node_type(), expected);
    }
}

#[test]
fn style_attributes_reach_the_widget() {
    let plain = xui! { <container /> };
    let styled = xui! { <container padding={EdgeInsets::all(8.0)} /> };

    assert_ne!(
        host(&plain).widget.props_hash(),
        host(&styled).widget.props_hash(),
        "a style attribute did not change the widget"
    );
}

/// Before the rewrite this attribute had no entry in the macro's table and was
/// silently discarded; now it is an ordinary inherent method.
#[test]
fn interaction_attributes_are_not_silently_dropped() {
    let plain = xui! { <container /> };
    let labelled = xui! { <container accessibility_role={AccessibilityRole::Tab} /> };

    assert_ne!(
        host(&plain).widget.props_hash(),
        host(&labelled).widget.props_hash(),
    );
}

#[test]
fn a_widget_specific_attribute_wins_over_the_style_property_of_the_same_name() {
    // `IconWidget::color` tints the icon; `StyleProps::color` sets text colour.
    // Inherent methods take precedence, so the icon keeps its own meaning.
    let plain = xui! { <icon /> };
    let tinted = xui! { <icon color={Color::BLUE_500} /> };

    assert_ne!(
        host(&plain).widget.props_hash(),
        host(&tinted).widget.props_hash(),
    );
}

#[test]
fn a_lone_braced_child_means_whatever_the_widget_says_it_means() {
    // `text` reads it as content...
    let label = xui! { <text>{"hello"}</text> };
    let content = host(&label).widget.text().expect("text content was set");
    assert_eq!(content.as_str(), "hello");
    assert!(host(&label).children.is_empty());

    // ...while a container reads it as children.
    let rows: Vec<ElementDesc> = vec![xui! { <container /> }, xui! { <container /> }];
    let list = xui! { <container>{rows}</container> };
    assert_eq!(host(&list).children.len(), 2);
}

#[test]
fn element_children_and_spread_children_combine() {
    let extra: Vec<ElementDesc> = vec![xui! { <container /> }, xui! { <container /> }];
    let tree = xui! {
        <container>
            <text>{"first"}</text>
            {extra}
            <text>{"last"}</text>
        </container>
    };

    assert_eq!(host(&tree).children.len(), 4);
}

#[test]
fn keys_are_carried_by_hosts_and_components() {
    let keyed_host = xui! { <container key="row" /> };
    assert_eq!(host(&keyed_host).widget.key(), Some("row".into()));

    let keyed_component = xui! { <greeting name={String::from("x")} key="hi" /> };
    match keyed_component {
        ElementDesc::Component(desc) => assert_eq!(desc.key, Some("hi".into())),
        other => panic!("expected a component element, got {other:?}"),
    }
}

#[component]
fn greeting(name: &String) {
    xui! { <text>{name.clone()}</text> }
}

#[component]
#[defaults(children = Vec::new())]
fn shell(children: &Vec<ElementDesc>) {
    xui! { <container>{children.to_vec()}</container> }
}

#[test]
fn component_tags_build_component_elements() {
    let element = xui! { <greeting name={String::from("world")} /> };
    assert!(matches!(element, ElementDesc::Component(_)));
}

#[test]
fn a_component_that_declares_children_accepts_a_body() {
    let element = xui! {
        <shell>
            <container />
            <container />
        </shell>
    };
    assert!(matches!(element, ElementDesc::Component(_)));
}

#[test]
fn a_path_can_be_used_as_a_tag() {
    let element = xui! { <xui::widgets::container /> };
    assert_eq!(host(&element).widget.node_type(), WidgetType::Container);
}

// ---------------------------------------------------------------------------
// style!
// ---------------------------------------------------------------------------

/// `style!` passes property names straight through to `StylePatch`, and lowers
/// `if <state>` conditions to static `WidgetStateMatcher` rules at compile time.
/// It has no call sites in the workspace, so without this it has no coverage.
#[test]
fn style_macro_lowers_state_conditions_to_rules() {
    let style = xui::style!(
        background: Color::BLACK,
        color: if hovered { Color::BLACK } else { Color::WHITE },
    );

    assert!(
        style.state_deps().contains(WidgetState::HOVERED),
        "the hovered branch did not become a state dependency"
    );
    assert_ne!(
        style.patch_for_state(WidgetState::empty()),
        style.patch_for_state(WidgetState::HOVERED),
        "hovering did not select a different patch"
    );
}

#[test]
fn style_macro_without_conditions_has_no_state_dependencies() {
    let style = xui::style!(background: Color::BLACK, font_size: 12.0);
    assert!(style.state_deps().is_empty());
}

// ---------------------------------------------------------------------------
// row / column
// ---------------------------------------------------------------------------

/// `row` and `column` are container presets rather than components, so they
/// take the whole style vocabulary *and* event handlers in one tag. As
/// components they could do neither without redeclaring every property as a
/// prop, and event handlers were not expressible as props at all.
#[test]
fn a_row_takes_style_and_event_attributes_together() {
    let element = xui! {
        <row
            gap={12.0}
            padding={EdgeInsets::all(8.0)}
            background={Color::BLACK}
            on_click={|_: &ClickEvent, _: &mut xui::event_system::EventContext<'_>| EventResult::Consumed}
        >
            <text>{"left"}</text>
            <text>{"right"}</text>
        </row>
    };

    let desc = host(&element);
    assert_eq!(desc.widget.node_type(), WidgetType::Container);
    assert_eq!(desc.children.len(), 2);
}

#[test]
fn row_and_column_are_hosts_with_opposite_directions() {
    let row = xui! { <row /> };
    let column = xui! { <column /> };

    assert_eq!(host(&row).widget.node_type(), WidgetType::Container);
    assert_eq!(host(&column).widget.node_type(), WidgetType::Container);
    assert_ne!(
        host(&row).widget.props_hash(),
        host(&column).widget.props_hash(),
        "row and column produced identical containers"
    );
}
