//! Nodes for the node-abstraction tests. `container` and `text` come from
//! `#[widget]`; the component `counter` is written by hand, since
//! `#[component]` does not produce a `NodeDef` yet.

#![allow(dead_code, non_camel_case_types, unused_imports)]

use xui_core::node::{ComponentKind, Draft, NodeDef, OwnProps};
use xui_core::property::{Accepts, FieldProperty, Property};
use xui_interface::WidgetUpdateFlags;
use xui_macros::widget;

// ---------------------------------------------------------------------------
// Host widgets
// ---------------------------------------------------------------------------

/// A flex container.
#[widget(
    style(box_size, padding, background, flex_layout),
    behavior(focus, accessibility)
)]
pub struct Container {
    /// Wraps overflowing children onto new lines.
    #[default]
    #[dirty(LAYOUT_INPUT)]
    pub flex_wrap: bool,
}

pub fn container() -> Draft<Container> {
    Draft::new(ContainerProps::builder().build())
}

/// A run of text.
#[widget(style(typography))]
pub struct Text {
    /// Required: a text node has no default content.
    #[dirty(TEXT_SHAPE | LAYOUT_INPUT)]
    pub content: String,
}

pub fn text(content: &str) -> Draft<Text> {
    Draft::new(TextProps::builder().content(content).build())
}

// ---------------------------------------------------------------------------
// counter: a component. Same NodeDef, no capabilities, Property only.
// ---------------------------------------------------------------------------

pub mod counter {
    use super::*;

    pub struct Counter;

    #[derive(Debug, Clone, PartialEq, Default)]
    pub struct Props {
        pub count: u32,
    }

    impl NodeDef for Counter {
        type Kind = ComponentKind;
        type Props = Props;
        type Style = ();
        type Behavior = ();
    }

    /// No `HostProperty`: a changed component prop means "render again",
    /// not a set of host dirty flags.
    pub enum count {}

    impl Property for count {
        type Scope = OwnProps<Counter>;
        type Value = u32;
        fn set(props: &mut Props, value: u32) {
            props.count = value;
        }
    }

    impl FieldProperty for count {
        fn get(props: &Props) -> &u32 {
            &props.count
        }
        fn same(a: &u32, b: &u32) -> bool {
            a == b
        }
    }

    impl Accepts<u32> for count {
        fn accept(value: u32) -> u32 {
            value
        }
    }
}

pub fn counter() -> Draft<counter::Counter> {
    Draft::new(counter::Props::default())
}

/// Selects every capability, so that two capabilities exporting the same
/// property name would make that name ambiguous here.
#[widget(
    style(
        box_size,
        margin,
        padding,
        flex_layout,
        position,
        display,
        typography,
        background,
        corner_radius,
        border,
        box_shadow,
        clip,
        visibility,
        transform,
        effects,
        scroll,
        cursor
    ),
    behavior(
        focus,
        accessibility,
        shortcuts,
        motion,
        focus_binding,
        scroll_binding,
        size_observation,
        events
    )
)]
pub struct Everything {}

pub fn everything() -> Draft<Everything> {
    Draft::new(EverythingProps::builder().build())
}
