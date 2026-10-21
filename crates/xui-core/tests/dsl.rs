//! Behavioural tests for the element DSL.
//!
//! These replace the old token-string assertions (`expanded.contains("xui ::
//! canvas (...)")`), which broke on formatting changes and could not tell
//! whether an attribute actually reached the widget. Compile-failure behaviour
//! is covered separately by `tests/ui`.

use xui_core::prelude::*;

use xui_core::widgets::WidgetType;
use xui_macros::{component, style, xui};

/// The host element `element` is, a template expanded into the plain tree it
/// stands for (and leaked: these are tests).
fn host(element: &ElementDesc) -> &WidgetDesc {
    match element {
        ElementDesc::Host(desc) => desc,
        ElementDesc::Template(template) => host(Box::leak(Box::new(template.clone().expand()))),
        other => panic!("expected a host element, got {other:?}"),
    }
}

fn draft<N: xui_core::node::NodeDef>(element: &ElementDesc) -> &Draft<N> {
    host(element).draft::<N>().expect("a draft of another widget")
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
        (xui! { <row /> }, WidgetType::Container),
        (xui! { <column /> }, WidgetType::Container),
        (xui! { <center /> }, WidgetType::Container),
        (xui! { <image /> }, WidgetType::Image),
        (xui! { <text_input /> }, WidgetType::TextInput),
        (
            xui! { <canvas controller={controller.clone()} /> },
            WidgetType::Canvas,
        ),
    ] {
        assert_eq!(host(&element).node_type(), expected);
    }
}

#[test]
fn style_attributes_reach_the_widget() {
    let plain = xui! { <container /> };
    let styled = xui! { <container padding={EdgeInsets::all(8.0)} /> };

    assert_eq!(
        draft::<Container>(&plain).storage.style.padding.padding,
        StyleValue::Unset
    );
    assert_eq!(
        draft::<Container>(&styled).storage.style.padding.padding,
        StyleValue::Value(EdgeInsets::all(8.0)),
        "a style attribute did not reach the widget"
    );
}

#[test]
fn visibility_and_display_are_style_attributes() {
    let hidden = xui! { <container visibility={VisibilityStyle::Hidden} /> };
    let undisplayed = xui! { <container display={DisplayStyle::None} /> };

    assert_eq!(
        draft::<Container>(&hidden).storage.style.visibility.visibility,
        StyleValue::Value(VisibilityStyle::Hidden)
    );
    assert_eq!(
        draft::<Container>(&undisplayed).storage.style.display.display,
        StyleValue::Value(DisplayStyle::None)
    );
}

#[test]
fn interaction_attributes_are_not_silently_dropped() {
    let labelled = xui! { <container accessibility_role={AccessibilityRole::Tab} /> };

    assert_eq!(
        draft::<Container>(&labelled).storage.behavior.accessibility.role,
        Some(AccessibilityRole::Tab)
    );
}

#[test]
fn a_widget_specific_attribute_wins_over_the_capability_property_of_the_same_name() {
    // An icon's own `color` tints it; it is not the typography colour, which
    // an icon does not select.
    let tinted = xui! { <icon color={Color::BLUE_500} /> };

    assert_eq!(draft::<Icon>(&tinted).storage.props.color, Some(Color::BLUE_500));
}

#[test]
fn a_lone_braced_child_means_whatever_the_widget_says_it_means() {
    // `text` reads it as content...
    let label = xui! { <text>{"hello"}</text> };
    assert_eq!(draft::<Text>(&label).storage.props.content.as_str(), "hello");
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
    assert_eq!(host(&keyed_host).key(), Some(&"row".into()));

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
    let element = xui! { <xui_core::prelude::container /> };
    assert_eq!(host(&element).node_type(), WidgetType::Container);
}

// ---------------------------------------------------------------------------
// style!
// ---------------------------------------------------------------------------

/// `style!` looks property names up in the node's table, as `xui!` does, and
/// lowers `if <state>` conditions to static `WidgetStateMatcher` rules at
/// compile time.
#[test]
fn style_macro_lowers_state_conditions_to_rules() {
    let style = style!(container {
        background: Color::BLACK,
        color: if hovered { Color::BLACK } else { Color::WHITE },
    });

    assert_eq!(
        style.base.0.background.background,
        StyleValue::Value(Color::BLACK.into())
    );
    assert_eq!(style.rules.len(), 2, "one rule per branch");
    assert!(
        style
            .rules
            .iter()
            .any(|rule| rule.matcher.matches(WidgetState::HOVERED)
                && !rule.matcher.matches(WidgetState::empty())),
        "the hovered branch did not become a hovered rule"
    );
}

#[test]
fn style_macro_without_conditions_has_no_rules() {
    let style = style!(container { background: Color::BLACK, font_size: 12.0 });
    assert!(style.rules.is_empty());
}

// ---------------------------------------------------------------------------
// row / column
// ---------------------------------------------------------------------------

/// `row` and `column` are container presets rather than components, so they
/// take the whole style vocabulary *and* event handlers in one tag.
#[test]
fn a_row_takes_style_and_event_attributes_together() {
    let element = xui! {
        <row
            gap={12.0}
            padding={EdgeInsets::all(8.0)}
            background={Color::BLACK}
            on_click={|_, _| EventResult::Consumed}
        >
            <text>{"left"}</text>
            <text>{"right"}</text>
        </row>
    };

    let desc = host(&element);
    assert_eq!(desc.node_type(), WidgetType::Container);
    assert_eq!(desc.children.len(), 2);
    assert!(
        draft::<Container>(&element)
            .storage
            .behavior
            .events
            .listens_for(EventMask::Click)
    );
}

#[test]
fn row_and_column_are_hosts_with_opposite_directions() {
    let row = xui! { <row /> };
    let column = xui! { <column /> };

    assert_eq!(
        draft::<Container>(&row).storage.props.flex_direction,
        Some(FlexDirectionStyle::Row)
    );
    assert_eq!(
        draft::<Container>(&column).storage.props.flex_direction,
        Some(FlexDirectionStyle::Column)
    );
}

// ---------------------------------------------------------------------------
// style={..} alongside individual attributes
// ---------------------------------------------------------------------------

/// State-conditioned values come from `style!`, which means a tag mixes
/// `style={..}` with ordinary attributes. That only works if `style` merges,
/// whatever order the attributes are written in.
#[test]
fn a_style_attribute_merges_with_the_attributes_around_it() {
    let element = xui! {
        <container
            padding={EdgeInsets::all(12.0)}
            style={style!(container {
                border_color: if hovered { Color::WHITE } else { Color::BLACK },
            })}
            border_width={1.0}
        />
    };
    let draft = draft::<Container>(&element);

    assert_eq!(
        draft.storage.style.padding.padding,
        StyleValue::Value(EdgeInsets::all(12.0)),
        "an attribute written before `style` was discarded"
    );
    assert_eq!(
        draft.storage.style.border.border_width,
        StyleValue::Value(1.0.into())
    );
    // And the state rules that `style!` produced survive the merge.
    assert_eq!(draft.storage.rules.len(), 2);
}
