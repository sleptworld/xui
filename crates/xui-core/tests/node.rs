//! The node abstraction, exercised through hand-written expansions (see
//! `node_fixture/mod.rs`) of what `#[widget]` / `#[component]` will generate.

#[path = "node_fixture/mod.rs"]
mod common;

use common::{container, counter, everything, text};
use xui_core::node::{Node, NodeStorage};
use xui_core::property::Slot;
use xui_interface::core::{Color, EdgeInsets, Sizing};
use xui_interface::style::{ComputedStyle, Theme, WidgetStateMatcher};
use xui_interface::styles::units::{ColorToken, LengthValue, StyleValue};
use xui_interface::{WidgetState, WidgetUpdateFlags};

#[test]
fn one_setter_reaches_props_style_and_behavior() {
    let mut node = container()
        .with::<container::width>(120.0)
        .with::<container::padding>(EdgeInsets::all(8.0))
        .with::<container::flex_wrap>(true);
    node.set::<container::tab_index>(3);
    node.set::<container::accessibility_label>("Toolbar");
    node.set::<container::gap>(LengthValue::Px(4.0));

    let storage = &node.storage;
    assert!(storage.props.flex_wrap);
    assert_eq!(
        storage.style.box_size.width,
        StyleValue::Value(Sizing::from(120.0))
    );
    assert_eq!(
        storage.style.padding.padding,
        StyleValue::Value(EdgeInsets::all(8.0))
    );
    assert_eq!(
        storage.style.flex_layout.gap,
        StyleValue::Value(LengthValue::Px(4.0))
    );
    assert_eq!(storage.behavior.focus.tab_index, Some(3));
    assert_eq!(
        storage.behavior.accessibility.label.as_deref(),
        Some("Toolbar")
    );
}

#[test]
fn slots_report_host_dirt_and_component_change() {
    let mut node = container();
    assert_eq!(
        Slot::<container::width>::new().update(&mut node, 10.0),
        WidgetUpdateFlags::STYLE_TARGET
    );
    assert_eq!(
        Slot::<container::flex_wrap>::new().update(&mut node, true),
        WidgetUpdateFlags::LAYOUT_INPUT
    );
    assert_eq!(
        Slot::<container::flex_wrap>::new().update(&mut node, true),
        WidgetUpdateFlags::empty()
    );

    let mut label = text("a");
    assert_eq!(
        Slot::<text::content>::new().update(&mut label, "b"),
        WidgetUpdateFlags::TEXT_SHAPE | WidgetUpdateFlags::LAYOUT_INPUT
    );

    // A component prop has no host dirt: `apply` only says it changed.
    let mut component = counter();
    assert!(Slot::<counter::count>::new().apply(&mut component, 2));
    assert!(!Slot::<counter::count>::new().apply(&mut component, 2));
    assert_eq!(component.storage.props.count, 2);
}

/// Stands in for the mounted instance the runtime will keep: a different
/// holder of the same storage gets the same setter for free.
struct Instance<N: xui_core::node::NodeDef> {
    storage: NodeStorage<N>,
    mounted: bool,
}

impl<N: xui_core::node::NodeDef> Node for Instance<N> {
    type Def = N;
    fn storage(&self) -> &NodeStorage<N> {
        &self.storage
    }
    fn storage_mut(&mut self) -> &mut NodeStorage<N> {
        &mut self.storage
    }
}

#[test]
fn prototype_and_instance_share_the_setter() {
    let draft = container().with::<container::width>(Sizing::Fill);
    let mut instance = Instance {
        storage: draft.storage,
        mounted: true,
    };
    assert!(instance.mounted);

    instance.set::<container::width>(40.0);
    instance.set::<container::flex_wrap>(true);
    assert_eq!(
        Slot::<container::width>::new().update(&mut instance, 40.0),
        WidgetUpdateFlags::empty()
    );
    assert_eq!(
        instance.storage.style.box_size.width,
        StyleValue::Value(Sizing::from(40.0))
    );
    assert!(instance.storage.props.flex_wrap);
}

#[test]
fn state_rules_layer_over_the_base_style() {
    let theme = Theme::default();
    let parent = ComputedStyle::initial(&theme);
    let node = container()
        .with::<container::background>(Color::WHITE)
        .with::<container::width>(100.0)
        .when(WidgetStateMatcher::all(WidgetState::HOVERED), |layer| {
            layer.set::<container::background>(ColorToken::Accent);
        })
        .when(WidgetStateMatcher::all(WidgetState::PRESSED), |layer| {
            layer.set::<container::width>(90.0);
        });

    let idle = node.compute_style(WidgetState::empty(), &parent, &theme);
    let hovered = node.compute_style(WidgetState::HOVERED, &parent, &theme);
    let both = node.compute_style(WidgetState::HOVERED | WidgetState::PRESSED, &parent, &theme);

    assert_eq!(idle.paint.background, Color::WHITE.into());
    assert_eq!(
        hovered.paint.background,
        theme.color(ColorToken::Accent).into()
    );
    assert_eq!(hovered.layout.width, Sizing::from(100.0));
    assert_eq!(both.layout.width, Sizing::from(90.0));
    assert_eq!(
        both.paint.background,
        theme.color(ColorToken::Accent).into()
    );
}

#[test]
fn unselected_style_passes_through() {
    let theme = Theme::default();
    let mut parent = ComputedStyle::initial(&theme);
    parent.text.font_size = 30.0;
    parent.text.color = Color::BLACK;

    // A container has no Typography: its children see the parent's text style.
    let box_style = container().with::<container::width>(10.0).compute_style(
        WidgetState::empty(),
        &parent,
        &theme,
    );
    assert_eq!(box_style.text, parent.text);

    // A text node has no BoxSize: its size stays initial whatever it declares.
    let label = text("hi")
        .with::<text::font_size>(LengthValue::Px(12.0))
        .compute_style(WidgetState::empty(), &box_style, &theme);
    assert_eq!(label.text.font_size, 12.0);
    assert_eq!(label.text.color, Color::BLACK);
    assert_eq!(
        label.layout.width,
        ComputedStyle::initial(&theme).layout.width
    );
}

#[test]
fn every_capability_composes_on_one_node() {
    use xui_interface::styles::units::{
        AlignStyle, BackdropStyle, ColorStyle, DisplayStyle, Effect, PositionStyle,
        ScrollDirectionStyle, ShadowStyle, StrokeStyle, VisibilityStyle,
    };
    use xui_interface::{
        AccessibilityProperties, CursorIcon, FontWeight, ShortcutBinding, Transition,
    };

    let node = common::everything()
        .with::<everything::width>(1.0)
        .with::<everything::margin>(EdgeInsets::all(1.0))
        .with::<everything::padding>(EdgeInsets::all(1.0))
        .with::<everything::align>(AlignStyle::Center)
        .with::<everything::position>(PositionStyle::Absolute)
        .with::<everything::display>(DisplayStyle::None)
        .with::<everything::font_weight>(FontWeight::Bold)
        .with::<everything::background>(ColorStyle::from(Color::WHITE))
        .with::<everything::border_radius>(4.0)
        .with::<everything::stroke>(StrokeStyle::new(Color::BLACK, 1.0))
        .with::<everything::shadow>(ShadowStyle::new())
        .with::<everything::clip>(true)
        .with::<everything::visibility>(VisibilityStyle::Hidden)
        .with::<everything::rotate>(0.5)
        .with::<everything::backdrop>(BackdropStyle::new())
        .with::<everything::effects>(Vec::<Effect>::new())
        .with::<everything::scroll_direction>(ScrollDirectionStyle::Vertical)
        .with::<everything::cursor>(CursorIcon::Pointer)
        .with::<everything::focusable>(true)
        .with::<everything::accessibility>(AccessibilityProperties::default())
        .with::<everything::accessibility_label>("all")
        .with::<everything::shortcuts>(Vec::<ShortcutBinding>::new())
        .with::<everything::transition>(Transition::default())
        .with::<everything::focus_handle>(xui_core::focus::FocusHandle::new())
        .with::<everything::scroll_controller>(xui_core::scroll::ScrollController::new())
        .with::<everything::size_observer>(None)
        .with_on::<everything::on_click, _>(|event, _| {
            let _ = event.click_count;
        });

    let storage = &node.storage;
    assert_eq!(storage.style.clip.clip, StyleValue::Value(true));
    assert_eq!(storage.behavior.accessibility.label.as_deref(), Some("all"));
    assert_eq!(
        storage.behavior.motion.transition,
        Some(Transition::default())
    );
}
