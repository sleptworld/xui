//! Behavioural tests for the element DSL.
//!
//! These replace the old token-string assertions (`expanded.contains("xui ::
//! canvas (...)")`), which broke on formatting changes and could not tell
//! whether an attribute actually reached the widget. Compile-failure behaviour
//! is covered separately by `tests/ui`.

use xui_core::prelude::*;

use xui_core::widgets::{container, WidgetType};
use xui_macros::{component, xui};

/// The element as the builder chain would have described it, however the
/// macro chose to build it (plainly, hoisted, or templated).
fn host(element: &ElementDesc) -> WidgetDesc {
    element
        .to_host()
        .unwrap_or_else(|| panic!("expected a host element, got {element:?}"))
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

#[test]
fn visibility_and_display_are_style_attributes() {
    let plain = xui! { <container /> };
    let hidden = xui! { <container visibility={VisibilityStyle::Hidden} /> };
    let undisplayed = xui! { <container display={DisplayStyle::None} /> };

    let hashes = [&plain, &hidden, &undisplayed].map(|element| host(element).widget.props_hash());
    assert_ne!(hashes[0], hashes[1]);
    assert_ne!(hashes[0], hashes[2]);
    assert_ne!(hashes[1], hashes[2]);
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

    let keyed_component = xui! { <Greeting name={String::from("x")} key="hi" /> };
    match keyed_component.resolved() {
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
    let element = xui! { <Greeting name={String::from("world")} /> };
    assert!(matches!(element.resolved(), ElementDesc::Component(_)));
}

#[test]
fn a_component_that_declares_children_accepts_a_body() {
    let element = xui! {
        <Shell>
            <container />
            <container />
        </Shell>
    };
    assert!(matches!(element.resolved(), ElementDesc::Component(_)));
}

#[test]
fn a_path_can_be_used_as_a_tag() {
    let element = xui! { <xui_core::widgets::container /> };
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
    let style = xui_macros::style!(
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
    let style = xui_macros::style!(background: Color::BLACK, font_size: 12.0);
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
            on_click={|_: &ClickEvent, _: &mut xui_core::event_system::EventContext<'_>| EventResult::Consumed}
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

// ---------------------------------------------------------------------------
// style={..} alongside individual attributes
// ---------------------------------------------------------------------------

/// State-conditioned values come from `style!`, which means a tag mixes
/// `style={..}` with ordinary attributes. That only works if `style` merges:
/// while it assigned, everything written before it was silently discarded, and
/// whether an attribute survived depended on where in the tag it appeared.
#[test]
fn a_style_attribute_merges_with_the_attributes_around_it() {
    let before_and_after = container()
        .padding(EdgeInsets::all(12.0))
        .style(xui_macros::style!(border_color: if hovered { Color::WHITE } else { Color::BLACK }))
        .border_width(1.0);

    let patch = before_and_after.style.patch_for_state(WidgetState::empty());
    assert_eq!(
        patch.layout.padding,
        StyleValue::Value(EdgeInsets::all(12.0)),
        "an attribute written before `style` was discarded"
    );
    assert_eq!(patch.paint.border_width, StyleValue::Value(1.0.into()));
    assert!(matches!(patch.paint.border_color, StyleValue::Value(_)));
}

/// And the state rules that `style!` produced survive the merge — otherwise the
/// hover branch would be silently flattened to its else arm.
#[test]
fn merging_a_style_keeps_its_state_rules() {
    let widget = container()
        .padding(EdgeInsets::all(12.0))
        .style(xui_macros::style!(border_color: if hovered { Color::WHITE } else { Color::BLACK }));

    assert!(
        widget.style.state_deps().contains(WidgetState::HOVERED),
        "the hovered rule did not survive being merged"
    );
    assert_ne!(
        widget
            .style
            .patch_for_state(WidgetState::empty())
            .paint
            .border_color,
        widget
            .style
            .patch_for_state(WidgetState::HOVERED)
            .paint
            .border_color,
    );
}

fn static_card() -> ElementDesc {
    xui! {
        <column gap={8.0}>
            <text>{"title"}</text>
            <container key="body" />
        </column>
    }
}

/// Literals and constants all the way down: built once per call site.
#[test]
fn a_static_element_is_hoisted() {
    let (ElementDesc::Hoisted(first), ElementDesc::Hoisted(second)) =
        (static_card(), static_card())
    else {
        panic!("a static element is hoisted");
    };
    assert!(std::rc::Rc::ptr_eq(&first, &second));
    assert_eq!(host(&ElementDesc::Hoisted(first)).children.len(), 2);
}

/// One local anywhere in the subtree keeps the element from being hoisted;
/// its static children are still hoisted on their own.
#[test]
fn a_dynamic_value_keeps_an_element_out_of_hoisting() {
    let gap = 8.0;
    let element = xui! {
        <column gap={gap}>
            <text>{"title"}</text>
        </column>
    };
    assert!(matches!(element, ElementDesc::Templated(_)));
    assert!(matches!(host(&element).children[0], ElementDesc::Hoisted(_)));

    let call = xui! { <container padding={EdgeInsets::all(4.0)} /> };
    assert!(
        !matches!(call, ElementDesc::Hoisted(_)),
        "a call is never static, however pure it looks"
    );
}

// ---------------------------------------------------------------------------
// Templated hosts (docs/template-slots.md §12)
// ---------------------------------------------------------------------------

const SQUARE: Size<Sizing> = Size {
    width: Sizing::Fill,
    height: Sizing::Hug,
};

/// A templated element must describe exactly what its builder chain does.
fn assert_same_as_builder(templated: ElementDesc, builder: ElementDesc) {
    assert!(
        matches!(templated, ElementDesc::Templated(_)),
        "expected a templated element, got {templated:?}"
    );
    let (templated, builder) = (host(&templated), host(&builder));
    assert_eq!(templated.widget.props_hash(), builder.widget.props_hash());
    assert_eq!(templated.widget.key(), builder.widget.key());
    assert_eq!(templated.children.len(), builder.children.len());
}

#[test]
fn a_templated_host_describes_what_its_builder_chain_does() {
    let color = Color::BLACK;
    let width = 120.0;
    let label = String::from("hello");

    // Static attributes before the changing one go into the base.
    assert_same_as_builder(
        xui! { <row gap={8.0} background={color} /> },
        xui_core::widgets::row()
            .gap(8.0)
            .background(color)
            .into_element_desc(Vec::new()),
    );
    // A static attribute after a changing one that writes the same field
    // stays after it.
    assert_same_as_builder(
        xui! { <container width={width} size={SQUARE} /> },
        container().width(width).size(SQUARE).into_element_desc(Vec::new()),
    );
    // And one before it stays before it.
    assert_same_as_builder(
        xui! { <container size={SQUARE} width={width} /> },
        container().size(SQUARE).width(width).into_element_desc(Vec::new()),
    );
    // Content, a style chain, and a computed key.
    assert_same_as_builder(
        xui_macros::view! { text(&label).color(color).font_size(14.0).key(label.as_str()) },
        xui_core::widgets::text()
            .text(&label)
            .color(color)
            .font_size(14.0)
            .key(label.as_str())
            .into_element_desc(),
    );
    // Static interaction alongside a handler.
    assert_same_as_builder(
        xui! { <container focusable={true} on_click={move |_, _| EventResult::Consumed} /> },
        container()
            .focusable(true)
            .on_click(move |_: &ClickEvent, _| EventResult::Consumed)
            .into_element_desc(Vec::new()),
    );
}

#[test]
fn a_changing_attribute_outside_the_vocabulary_builds_the_element_as_before() {
    let wrap = true;
    assert!(matches!(
        xui! { <row flex_wrap={wrap} /> },
        ElementDesc::Host(_)
    ));
    let style = Style::new().gap(4.0);
    assert!(matches!(
        xui! { <row style={style} /> },
        ElementDesc::Host(_)
    ));
}
