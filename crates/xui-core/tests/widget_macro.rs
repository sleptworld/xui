//! `#[widget]` turns one struct into a widget's empty marker type, props, state
//! and typed props builder; `WidgetT` is implemented by hand.

use xui_core::node::WidgetT;
use xui_interface::WidgetUpdateFlags;
use xui_macros::widget;

const DEFAULT_MAX: f32 = 10.0;

/// A slider whose state follows its value.
#[widget]
#[defaults(max = DEFAULT_MAX)]
#[props(derive(Clone, Debug, PartialEq))]
#[state(derive(Debug, PartialEq))]
pub struct Slider {
    /// The current value.
    pub value: f32,
    #[default]
    pub min: f32,
    pub max: f32,
    #[default("slider".into())]
    pub label: String,

    #[state]
    dragging: bool,
    #[state]
    shown: f32,
}

impl WidgetT for Slider {
    type State = SliderState;

    fn create_state(props: &SliderProps) -> SliderState {
        SliderState {
            dragging: false,
            shown: props.value.clamp(props.min, props.max),
        }
    }

    fn update(old: &SliderProps, new: &SliderProps, state: &mut SliderState) -> WidgetUpdateFlags {
        if old.value != new.value {
            state.shown = new.value.clamp(new.min, new.max);
        }
        WidgetUpdateFlags::empty()
    }
}

#[widget]
struct Spacer {}

#[widget]
struct Counter {
    start: u32,
    #[state]
    count: u32,
}

#[test]
fn the_marker_is_an_empty_type() {
    assert_eq!(std::mem::size_of::<Slider>(), 0);
    let _: Spacer = Spacer;
}

#[test]
fn the_builder_fills_in_defaults() {
    let props = SliderProps::builder().value(3.0).build();
    assert_eq!(
        props,
        SliderProps {
            value: 3.0,
            min: 0.0,
            max: DEFAULT_MAX,
            label: "slider".to_string(),
        }
    );
}

#[test]
fn setters_take_anything_into_the_prop_type_in_any_order() {
    let props = SliderProps::builder()
        .label("volume")
        .max(20.0)
        .value(30.0)
        .min(1.0)
        .build();
    assert_eq!(props.label, "volume");
    assert_eq!((props.min, props.max, props.value), (1.0, 20.0, 30.0));
}

#[test]
fn the_generated_types_plug_into_a_handwritten_widget_impl() {
    let props = SliderProps::builder().value(30.0).build();
    let state = Slider::create_state(&props);
    assert_eq!(
        state,
        SliderState {
            dragging: false,
            shown: DEFAULT_MAX,
        }
    );
}

#[test]
fn update_sees_the_generated_state() {
    let old = SliderProps::builder().value(1.0).build();
    let mut state = Slider::create_state(&old);
    state.dragging = true;
    let new = SliderProps {
        value: 4.0,
        ..old.clone()
    };
    Slider::update(&old, &new, &mut state);
    assert_eq!(state.shown, 4.0);
    assert!(state.dragging, "update leaves the rest of the state alone");
}

#[test]
fn primitive_setters_infer_integer_literals() {
    let props = CounterProps::builder().start(4).build();
    let state = CounterState {
        count: props.start * 2,
    };
    assert_eq!(state.count, 8);
}

#[test]
fn a_widget_without_fields_has_empty_props_and_no_state_struct() {
    let _: SpacerProps = SpacerProps::builder().build();
    assert_eq!(std::mem::size_of::<SpacerProps>(), 0);
}
