//! Handwritten output of caps!(...). Only selected capabilities occupy fields.
//! STYLE patches deliberately omit Interaction and Focus.
use crate::abilities::{
    BoxLayout, BoxLayoutComputed, BoxLayoutConfig, Decoration, DecorationComputed,
    DecorationConfig, Typography, TypographyComputed, TypographyConfig,
};
use crate::capability::{
    CapSet, Capability, Has, HasRuntime, Resolved, StateRule, StyleCapability, StyleOverlay,
    StyleSet,
};
use crate::core::{Dirty, ParentStyle, Theme, WidgetState};
use crate::interaction::{Focus, FocusConfig, Interaction, InteractionConfig, InteractionState};
use crate::managed::{GlobalShortcut, ShortcutAbility, ShortcutConfig};
use crate::runtime::RegistrationId;

// Also handwritten: generated runtime state access, never present on
// a Widget whose capability list omits Interaction.
macro_rules! impl_has_runtime_interaction {
    ($state:ty) => {
        impl HasRuntime<Interaction> for $state {
            fn get_runtime(&self) -> &InteractionState {
                &self.interaction
            }
            fn get_runtime_mut(&mut self) -> &mut InteractionState {
                &mut self.interaction
            }
        }
    };
}

#[derive(Debug, Default)]
pub struct TextRuntimeState {
    pub interaction: InteractionState,
}
impl_has_runtime_interaction!(TextRuntimeState);
#[derive(Debug, Default)]
pub struct ButtonRuntimeState {
    pub interaction: InteractionState,
}
impl_has_runtime_interaction!(ButtonRuntimeState);
#[derive(Debug, Default)]
pub struct ActionRuntimeState {
    pub interaction: InteractionState,
}
impl_has_runtime_interaction!(ActionRuntimeState);

#[derive(Debug, Default)]
pub struct ButtonManagedState {
    pub shortcut: Option<RegistrationId>,
}

// Handwritten equivalent of generated field access for chosen abilities.
macro_rules! impl_has {
    ($config:ty, $cap:ty, $field:ident) => {
        impl Has<$cap> for $config {
            fn get(&self) -> &<$cap as Capability>::Config {
                &self.$field
            }
            fn get_mut(&mut self) -> &mut <$cap as Capability>::Config {
                &mut self.$field
            }
        }
    };
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextConfig {
    pub layout: BoxLayoutConfig,
    pub typography: TypographyConfig,
    pub decoration: DecorationConfig,
    pub interaction: InteractionConfig,
}
impl_has!(TextConfig, BoxLayout, layout);
impl_has!(TextConfig, Typography, typography);
impl_has!(TextConfig, Decoration, decoration);
impl_has!(TextConfig, Interaction, interaction);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextStylePatch {
    pub layout: BoxLayoutConfig,
    pub typography: TypographyConfig,
    pub decoration: DecorationConfig,
}

impl StyleOverlay for TextStylePatch {
    fn overlay(&mut self, from: &Self) {
        self.layout.overlay(&from.layout);
        self.typography.overlay(&from.typography);
        self.decoration.overlay(&from.decoration);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextComputedStyle {
    pub layout: BoxLayoutComputed,
    pub typography: TypographyComputed,
    pub decoration: DecorationComputed,
}

pub struct TextCaps;
impl CapSet for TextCaps {
    type Config = TextConfig;
    type RuntimeState = TextRuntimeState;
    type ManagedState = ();
}

impl StyleSet for TextCaps {
    type Patch = TextStylePatch;
    type Computed = TextComputedStyle;
    fn resolve_style(
        config: &Self::Config,
        rules: &[StateRule<Self::Patch>],
        state: WidgetState,
        parent: &ParentStyle,
        theme: &Theme,
    ) -> Resolved<Self::Computed> {
        // Only style fields are cloned for state matching, never handlers.
        let mut effective = TextStylePatch {
            layout: config.layout.clone(),
            typography: config.typography.clone(),
            decoration: config.decoration.clone(),
        };
        for rule in rules {
            if rule.matcher.matches(state) {
                effective.overlay(&rule.patch);
            }
        }
        let layout = BoxLayout::resolve(&effective.layout, parent, theme);
        let typography = Typography::resolve(&effective.typography, parent, theme);
        let decoration = Decoration::resolve(&effective.decoration, parent, theme);
        let mut child = parent.for_child();
        BoxLayout::propagate(&layout, &mut child);
        Typography::propagate(&typography, &mut child);
        Decoration::propagate(&decoration, &mut child);
        Resolved {
            computed: TextComputedStyle {
                layout,
                typography,
                decoration,
            },
            for_children: child,
        }
    }
    fn diff(a: &Self::Computed, b: &Self::Computed) -> Dirty {
        let mut dirty = Dirty::NONE;
        if a.layout != b.layout {
            dirty |= Dirty::LAYOUT;
        }
        if a.typography != b.typography {
            dirty |= Dirty::TEXT | Dirty::PAINT;
        }
        if a.decoration != b.decoration {
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageConfig {
    pub layout: BoxLayoutConfig,
    pub decoration: DecorationConfig,
}
impl_has!(ImageConfig, BoxLayout, layout);
impl_has!(ImageConfig, Decoration, decoration);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageStylePatch {
    pub layout: BoxLayoutConfig,
    pub decoration: DecorationConfig,
}
impl StyleOverlay for ImageStylePatch {
    fn overlay(&mut self, from: &Self) {
        self.layout.overlay(&from.layout);
        self.decoration.overlay(&from.decoration);
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ImageComputedStyle {
    pub layout: BoxLayoutComputed,
    pub decoration: DecorationComputed,
}
pub struct ImageCaps;
impl CapSet for ImageCaps {
    type Config = ImageConfig;
    type RuntimeState = ();
    type ManagedState = ();
}
impl StyleSet for ImageCaps {
    type Patch = ImageStylePatch;
    type Computed = ImageComputedStyle;
    fn resolve_style(
        config: &Self::Config,
        rules: &[StateRule<Self::Patch>],
        state: WidgetState,
        parent: &ParentStyle,
        theme: &Theme,
    ) -> Resolved<Self::Computed> {
        let mut effective = ImageStylePatch {
            layout: config.layout.clone(),
            decoration: config.decoration.clone(),
        };
        for rule in rules {
            if rule.matcher.matches(state) {
                effective.overlay(&rule.patch);
            }
        }
        let layout = BoxLayout::resolve(&effective.layout, parent, theme);
        let decoration = Decoration::resolve(&effective.decoration, parent, theme);
        let mut child = parent.for_child();
        BoxLayout::propagate(&layout, &mut child);
        Decoration::propagate(&decoration, &mut child);
        // No typography: inherited text values are forwarded unmodified.
        Resolved {
            computed: ImageComputedStyle { layout, decoration },
            for_children: child,
        }
    }
    fn diff(a: &Self::Computed, b: &Self::Computed) -> Dirty {
        let mut dirty = Dirty::NONE;
        if a.layout != b.layout {
            dirty |= Dirty::LAYOUT;
        }
        if a.decoration != b.decoration {
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ButtonConfig {
    pub layout: BoxLayoutConfig,
    pub decoration: DecorationConfig,
    pub interaction: InteractionConfig,
    pub focus: FocusConfig,
    pub shortcut: ShortcutConfig,
}
impl_has!(ButtonConfig, BoxLayout, layout);
impl_has!(ButtonConfig, Decoration, decoration);
impl_has!(ButtonConfig, Interaction, interaction);
impl_has!(ButtonConfig, Focus, focus);
impl_has!(ButtonConfig, ShortcutAbility, shortcut);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ButtonStylePatch {
    pub layout: BoxLayoutConfig,
    pub decoration: DecorationConfig,
}
impl StyleOverlay for ButtonStylePatch {
    fn overlay(&mut self, from: &Self) {
        self.layout.overlay(&from.layout);
        self.decoration.overlay(&from.decoration);
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonComputedStyle {
    pub layout: BoxLayoutComputed,
    pub decoration: DecorationComputed,
}
pub struct ButtonCaps;
impl CapSet for ButtonCaps {
    type Config = ButtonConfig;
    type RuntimeState = ButtonRuntimeState;
    type ManagedState = ButtonManagedState;

    // Generated hook list: one call for each selected ManagedProperty.
    // It does not depend on runtime trait detection or specialization.
    fn commit_managed(
        old: Option<&ButtonConfig>,
        new: Option<&ButtonConfig>,
        state: &mut ButtonManagedState,
        ctx: &mut crate::runtime::CommitCtx<'_>,
    ) {
        crate::property::commit_managed::<GlobalShortcut>(
            old.map(|c| &c.shortcut),
            new.map(|c| &c.shortcut),
            &mut state.shortcut,
            ctx,
        );
    }
}
impl StyleSet for ButtonCaps {
    type Patch = ButtonStylePatch;
    type Computed = ButtonComputedStyle;
    fn resolve_style(
        config: &Self::Config,
        rules: &[StateRule<Self::Patch>],
        state: WidgetState,
        parent: &ParentStyle,
        theme: &Theme,
    ) -> Resolved<Self::Computed> {
        let mut effective = ButtonStylePatch {
            layout: config.layout.clone(),
            decoration: config.decoration.clone(),
        };
        for rule in rules {
            if rule.matcher.matches(state) {
                effective.overlay(&rule.patch);
            }
        }
        let layout = BoxLayout::resolve(&effective.layout, parent, theme);
        let decoration = Decoration::resolve(&effective.decoration, parent, theme);
        let mut child = parent.for_child();
        BoxLayout::propagate(&layout, &mut child);
        Decoration::propagate(&decoration, &mut child);
        Resolved {
            computed: ButtonComputedStyle { layout, decoration },
            for_children: child,
        }
    }
    fn diff(a: &Self::Computed, b: &Self::Computed) -> Dirty {
        let mut dirty = Dirty::NONE;
        if a.layout != b.layout {
            dirty |= Dirty::LAYOUT;
        }
        if a.decoration != b.decoration {
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

pub struct EmptyCaps;
impl CapSet for EmptyCaps {
    type Config = ();
    type RuntimeState = ();
    type ManagedState = ();
}
impl StyleSet for EmptyCaps {
    type Patch = ();
    type Computed = ();
    fn resolve_style(
        _: &(),
        _: &[StateRule<()>],
        _: WidgetState,
        parent: &ParentStyle,
        _: &Theme,
    ) -> Resolved<()> {
        Resolved {
            computed: (),
            for_children: parent.for_child(),
        }
    }
    fn diff(_: &(), _: &()) -> Dirty {
        Dirty::NONE
    }
}

/// An interaction-only logical node: zero style storage and zero style output.
/// Demonstrates that not every Capability needs any style-related trait.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionConfig {
    pub interaction: InteractionConfig,
}
impl_has!(ActionConfig, Interaction, interaction);

pub struct ActionCaps;
impl CapSet for ActionCaps {
    type Config = ActionConfig;
    type RuntimeState = ActionRuntimeState;
    type ManagedState = ();
}
impl StyleSet for ActionCaps {
    type Patch = ();
    type Computed = ();
    fn resolve_style(
        _: &Self::Config,
        _: &[StateRule<()>],
        _: WidgetState,
        parent: &ParentStyle,
        _: &Theme,
    ) -> Resolved<()> {
        Resolved {
            computed: (),
            for_children: parent.for_child(),
        }
    }
    fn diff(_: &(), _: &()) -> Dirty {
        Dirty::NONE
    }
}
