//! Runtime event adapter, deliberately distinct from style inheritance/compute.
use std::{fmt, rc::Rc};
use crate::capability::{Capability, FocusCapability, InteractionCapability};
use crate::core::Dirty;
use crate::property::{Property, SlotProperty};
use crate::abilities::CapScope;

#[derive(Clone)]
pub struct ClickHandler(Rc<dyn Fn()>);
impl ClickHandler {
    pub fn new(f: impl Fn() + 'static) -> Self { Self(Rc::new(f)) }
    pub fn call(&self) { (self.0)(); }
}
impl PartialEq for ClickHandler {
    fn eq(&self, rhs: &Self) -> bool { Rc::ptr_eq(&self.0, &rhs.0) }
}
impl fmt::Debug for ClickHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClickHandler(..)")
    }
}

pub struct Interaction;

#[derive(Clone, Debug, PartialEq)]
pub struct InteractionConfig {
    pub enabled: bool,
    pub on_click: Option<ClickHandler>,
}
impl Default for InteractionConfig {
    fn default() -> Self { Self { enabled: true, on_click: None } }
}
impl Capability for Interaction { type Config = InteractionConfig; }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InteractionState { pub hovered: bool, pub pressed: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiEvent { PointerEnter, PointerLeave, PointerDown, PointerUp }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventOutcome { pub handled: bool, pub request_repaint: bool }

impl InteractionCapability for Interaction {
    type LocalState = InteractionState;
    fn on_routed_event(
        config: &Self::Config,
        local: &mut Self::LocalState,
        event: &UiEvent,
    ) -> EventOutcome {
        if !config.enabled {
            local.pressed = false;
            local.hovered = false;
            return EventOutcome::default();
        }
        match event {
            UiEvent::PointerEnter => { local.hovered = true; EventOutcome { request_repaint: true, handled: false } }
            UiEvent::PointerLeave => {
                local.hovered = false;
                local.pressed = false;
                EventOutcome { request_repaint: true, handled: false }
            }
            UiEvent::PointerDown => {
                local.pressed = true;
                EventOutcome { handled: true, request_repaint: true }
            }
            UiEvent::PointerUp => {
                let click = local.pressed && local.hovered;
                local.pressed = false;
                if click {
                    if let Some(handler) = &config.on_click { handler.call(); }
                }
                EventOutcome { handled: click, request_repaint: true }
            }
        }
    }
}

pub struct OnClick;
impl Property for OnClick {
    type Scope = CapScope<Interaction>;
    type Value = Option<ClickHandler>;
    fn get(d: &InteractionConfig) -> &Self::Value { &d.on_click }
    fn replace(d: &mut InteractionConfig, value: Self::Value) { d.on_click = value; }
}
impl SlotProperty for OnClick {
    const HINT: Dirty = Dirty::EVENT;
    fn same(a: &Self::Value, b: &Self::Value) -> bool { a == b }
}

pub struct InteractionEnabled;
impl Property for InteractionEnabled {
    type Scope = CapScope<Interaction>;
    type Value = bool;
    fn get(d: &InteractionConfig) -> &bool { &d.enabled }
    fn replace(d: &mut InteractionConfig, value: bool) { d.enabled = value; }
}
impl SlotProperty for InteractionEnabled {
    const HINT: Dirty = Dirty::EVENT;
    fn same(a: &bool, b: &bool) -> bool { a == b }
}

pub struct Focus;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusConfig { pub enabled: bool, pub tab_index: i16 }
impl Default for FocusConfig {
    fn default() -> Self { Self { enabled: true, tab_index: 0 } }
}
impl Capability for Focus { type Config = FocusConfig; }
impl FocusCapability for Focus {}

pub struct TabIndex;
impl Property for TabIndex {
    type Scope = CapScope<Focus>;
    type Value = i16;
    fn get(c: &FocusConfig) -> &i16 { &c.tab_index }
    fn replace(c: &mut FocusConfig, value: i16) { c.tab_index = value; }
}
impl SlotProperty for TabIndex {
    const HINT: Dirty = Dirty::FOCUS;
    fn same(a: &i16, b: &i16) -> bool { a == b }
}
