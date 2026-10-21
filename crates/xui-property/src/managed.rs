//! Handwritten output of `#[property(managed)]` on GlobalShortcut.
//! Handler and style props remain ordinary properties, not managed ones.
use crate::{
    abilities::CapScope,
    capability::Capability,
    core::Dirty,
    property::{ManagedProperty, Property, SlotProperty},
    runtime::{CommitCtx, RegistrationId, ShortcutBinding},
};

pub struct ShortcutAbility;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShortcutConfig {
    pub shortcut: Option<ShortcutBinding>,
}
impl Capability for ShortcutAbility { type Config = ShortcutConfig; }

pub struct GlobalShortcut;
impl Property for GlobalShortcut {
    type Scope = CapScope<ShortcutAbility>;
    type Value = Option<ShortcutBinding>;
    fn get(storage: &ShortcutConfig) -> &Self::Value { &storage.shortcut }
    fn replace(storage: &mut ShortcutConfig, value: Self::Value) {
        storage.shortcut = value;
    }
}
impl SlotProperty for GlobalShortcut {
    const HINT: Dirty = Dirty::EVENT;
    fn same(a: &Self::Value, b: &Self::Value) -> bool { a == b }
}

/// Per-mounted-instance resource; no global registry handle in the shareable
/// ShortcutConfig. The registry itself owns the actual registration.
impl ManagedProperty for GlobalShortcut {
    type Resource = Option<RegistrationId>;

    fn on_commit(
        old: Option<&Self::Value>,
        new: Option<&Self::Value>,
        resource: &mut Self::Resource,
        ctx: &mut CommitCtx<'_>,
    ) {
        // Old/new are *node* lifecycles; inner Option indicates whether the
        // node currently declares a shortcut. We never register tentative WIP.
        let wanted = new.and_then(|value| value.as_ref());
        let previously = old.and_then(|value| value.as_ref());
        if wanted == previously && (wanted.is_some() == resource.is_some()) {
            return;
        }
        if let Some(id) = resource.take() {
            ctx.services.shortcuts.unregister(id);
        }
        if let Some(binding) = wanted {
            *resource = Some(ctx.services.shortcuts.register(ctx.node, binding.clone()));
        }
    }
}
