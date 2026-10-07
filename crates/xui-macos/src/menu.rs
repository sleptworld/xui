//! The menu bar.
//!
//! The runtime describes a [`MenuBar`] as data; this turns it into `NSMenu`s.
//! Every menu item is one of two things:
//!
//! - **a standard action**, for a [`commands`] id or a [`MenuRole`]. The item
//!   has no target, so AppKit walks the responder chain to find who handles
//!   `copy:`, `performMiniaturize:` and the rest. [`XuiView`](crate::view) answers
//!   the edit ones by raising the matching command; the window and the
//!   application answer the others themselves.
//! - **a custom command**: the item targets a [`MenuTarget`], which raises the
//!   command its tag names.
//!
//! Either way the runtime ends up with a command event, the same as a shortcut's.
//!
//! The application menu -- About, Hide, Quit -- is the host's, and always first:
//! a Mac app without one has no way to quit. What follows it is the
//! application's menus, or, when it supplied none, an Edit and a Window menu, so
//! Cmd+C and Cmd+M work out of the box.

use std::cell::RefCell;
use std::ffi::CString;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSControlStateValueOn, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::{NSObject, NSObjectProtocol, NSProcessInfo, NSString};
use xui_interface::events::{NamedKey, PhysicalKey};
use xui_interface::{
    CommandId, Menu, MenuBar, MenuEntry, MenuItem, MenuRole, Shortcut, ShortcutKey,
    ShortcutModifiers, commands,
};
use xui_shell::ShellEvent;

use crate::host;

/// What choosing a menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// A selector sent down the responder chain.
    Selector(&'static str),
    /// A command the runtime handles.
    Command(CommandId),
}

pub(crate) fn action_for(item: &MenuItem) -> Action {
    let role_selector = item.role.and_then(|role| match role {
        MenuRole::About => Some("orderFrontStandardAboutPanel:"),
        MenuRole::Quit => Some("terminate:"),
        MenuRole::Hide => Some("hide:"),
        MenuRole::HideOthers => Some("hideOtherApplications:"),
        MenuRole::Minimize => Some("performMiniaturize:"),
        MenuRole::Zoom => Some("performZoom:"),
        MenuRole::Fullscreen => Some("toggleFullScreen:"),
        // A role this host does not know is an ordinary command.
        _ => None,
    });
    if let Some(selector) = role_selector {
        return Action::Selector(selector);
    }
    match item.command {
        command if command == commands::UNDO => Action::Selector("undo:"),
        command if command == commands::REDO => Action::Selector("redo:"),
        command if command == commands::CUT => Action::Selector("cut:"),
        command if command == commands::COPY => Action::Selector("copy:"),
        command if command == commands::PASTE => Action::Selector("paste:"),
        command if command == commands::SELECT_ALL => Action::Selector("selectAll:"),
        command => Action::Command(command),
    }
}

/// How AppKit spells a shortcut on a menu item: the key as a string, and the
/// modifiers it needs.
///
/// `None` for a key a menu cannot show, such as a modifier on its own.
pub(crate) fn key_equivalent(shortcut: &Shortcut) -> Option<(String, NSEventModifierFlags)> {
    let key = match shortcut.key {
        ShortcutKey::Physical(key) => physical_key_equivalent(key)?,
        ShortcutKey::Named(key) => named_key_equivalent(key)?,
    };

    let resolved = shortcut.resolved_modifiers();
    let mut flags = NSEventModifierFlags::empty();
    if resolved.shift {
        flags |= NSEventModifierFlags::Shift;
    }
    if resolved.ctrl {
        flags |= NSEventModifierFlags::Control;
    }
    if resolved.alt {
        flags |= NSEventModifierFlags::Option;
    }
    if resolved.meta {
        flags |= NSEventModifierFlags::Command;
    }
    Some((key, flags))
}

fn physical_key_equivalent(key: PhysicalKey) -> Option<String> {
    use PhysicalKey::*;
    let character = match key {
        KeyA => 'a',
        KeyB => 'b',
        KeyC => 'c',
        KeyD => 'd',
        KeyE => 'e',
        KeyF => 'f',
        KeyG => 'g',
        KeyH => 'h',
        KeyI => 'i',
        KeyJ => 'j',
        KeyK => 'k',
        KeyL => 'l',
        KeyM => 'm',
        KeyN => 'n',
        KeyO => 'o',
        KeyP => 'p',
        KeyQ => 'q',
        KeyR => 'r',
        KeyS => 's',
        KeyT => 't',
        KeyU => 'u',
        KeyV => 'v',
        KeyW => 'w',
        KeyX => 'x',
        KeyY => 'y',
        KeyZ => 'z',
        Digit0 => '0',
        Digit1 => '1',
        Digit2 => '2',
        Digit3 => '3',
        Digit4 => '4',
        Digit5 => '5',
        Digit6 => '6',
        Digit7 => '7',
        Digit8 => '8',
        Digit9 => '9',
        Comma => ',',
        Period => '.',
        Slash => '/',
        Semicolon => ';',
        Quote => '\'',
        Minus => '-',
        Equal => '=',
        BracketLeft => '[',
        BracketRight => ']',
        Backslash => '\\',
        Backquote => '`',
        Space => ' ',
        Enter => '\r',
        Tab => '\t',
        Backspace => '\u{8}',
        _ => return None,
    };
    Some(character.to_string())
}

fn named_key_equivalent(key: NamedKey) -> Option<String> {
    // AppKit spells function keys as code points in a private-use block.
    let character = match key {
        NamedKey::Enter => '\r',
        NamedKey::Tab => '\t',
        NamedKey::Escape => '\u{1b}',
        NamedKey::Backspace => '\u{8}',
        NamedKey::Space => ' ',
        NamedKey::Delete => '\u{F728}',
        NamedKey::ArrowUp => '\u{F700}',
        NamedKey::ArrowDown => '\u{F701}',
        NamedKey::ArrowLeft => '\u{F702}',
        NamedKey::ArrowRight => '\u{F703}',
        NamedKey::Home => '\u{F729}',
        NamedKey::End => '\u{F72B}',
        NamedKey::PageUp => '\u{F72C}',
        NamedKey::PageDown => '\u{F72D}',
        NamedKey::F(number @ 1..=35) => char::from_u32(0xF703 + u32::from(number))?,
        _ => return None,
    };
    Some(character.to_string())
}

fn command_shortcut(key: PhysicalKey, shift: bool) -> Shortcut {
    Shortcut::physical(key).modifiers(ShortcutModifiers {
        shift,
        primary: true,
        ..ShortcutModifiers::default()
    })
}

fn item(label: &str, command: CommandId, shortcut: Option<Shortcut>) -> MenuEntry {
    let mut item = MenuItem::new(label, command);
    item.shortcut = shortcut;
    MenuEntry::Item(item)
}

fn role_item(label: &str, role: MenuRole, shortcut: Option<Shortcut>) -> MenuEntry {
    let mut item = MenuItem::new(label, CommandId("xui.role"));
    item.role = Some(role);
    item.shortcut = shortcut;
    MenuEntry::Item(item)
}

/// The menus an application gets when it supplies none, after the application
/// menu.
pub(crate) fn default_menus() -> Vec<Menu> {
    vec![
        Menu {
            label: "Edit".into(),
            entries: vec![
                item(
                    "Undo",
                    commands::UNDO,
                    Some(command_shortcut(PhysicalKey::KeyZ, false)),
                ),
                item(
                    "Redo",
                    commands::REDO,
                    Some(command_shortcut(PhysicalKey::KeyZ, true)),
                ),
                MenuEntry::Separator,
                item(
                    "Cut",
                    commands::CUT,
                    Some(command_shortcut(PhysicalKey::KeyX, false)),
                ),
                item(
                    "Copy",
                    commands::COPY,
                    Some(command_shortcut(PhysicalKey::KeyC, false)),
                ),
                item(
                    "Paste",
                    commands::PASTE,
                    Some(command_shortcut(PhysicalKey::KeyV, false)),
                ),
                item(
                    "Select All",
                    commands::SELECT_ALL,
                    Some(command_shortcut(PhysicalKey::KeyA, false)),
                ),
            ],
        },
        Menu {
            label: "Window".into(),
            entries: vec![
                role_item(
                    "Minimize",
                    MenuRole::Minimize,
                    Some(command_shortcut(PhysicalKey::KeyM, false)),
                ),
                role_item("Zoom", MenuRole::Zoom, None),
                MenuEntry::Separator,
                role_item(
                    "Enter Full Screen",
                    MenuRole::Fullscreen,
                    Some(
                        Shortcut::physical(PhysicalKey::KeyF).modifiers(ShortcutModifiers {
                            control: true,
                            primary: true,
                            ..ShortcutModifiers::default()
                        }),
                    ),
                ),
            ],
        },
    ]
}

/// The application menu: what every Mac app has first, named after the app.
pub(crate) fn application_menu(name: &str) -> Menu {
    Menu {
        label: name.to_owned(),
        entries: vec![
            role_item(&format!("About {name}"), MenuRole::About, None),
            MenuEntry::Separator,
            role_item(
                &format!("Hide {name}"),
                MenuRole::Hide,
                Some(command_shortcut(PhysicalKey::KeyH, false)),
            ),
            role_item(
                "Hide Others",
                MenuRole::HideOthers,
                Some(
                    Shortcut::physical(PhysicalKey::KeyH).modifiers(ShortcutModifiers {
                        alt: true,
                        primary: true,
                        ..ShortcutModifiers::default()
                    }),
                ),
            ),
            MenuEntry::Separator,
            role_item(
                &format!("Quit {name}"),
                MenuRole::Quit,
                Some(command_shortcut(PhysicalKey::KeyQ, false)),
            ),
        ],
    }
}

/// What an application's menu bar becomes: the host's application menu, then
/// the application's own menus, or the defaults if it has none.
pub(crate) fn complete_menu_bar(name: &str, app: Option<&MenuBar>) -> Vec<Menu> {
    let mut menus = vec![application_menu(name)];
    match app {
        Some(bar) => menus.extend(bar.0.iter().cloned()),
        None => menus.extend(default_menus()),
    }
    menus
}

#[derive(Default)]
pub(crate) struct MenuTargetIvars {
    /// Indexed by the tag of the item that was chosen.
    commands: RefCell<Vec<CommandId>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `MenuTarget` does
    // not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "XuiMenuTarget"]
    #[ivars = MenuTargetIvars]
    pub(crate) struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(xuiMenuItem:))]
        fn menu_item_chosen(&self, sender: &NSMenuItem) {
            let command = usize::try_from(sender.tag())
                .ok()
                .and_then(|index| self.ivars().commands.borrow().get(index).copied());
            if let Some(command) = command {
                host::send_current(ShellEvent::Command(command));
            }
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuTargetIvars::default());
        unsafe { msg_send![super(this), init] }
    }

    fn register(&self, command: CommandId) -> isize {
        let mut commands = self.ivars().commands.borrow_mut();
        commands.push(command);
        (commands.len() - 1) as isize
    }
}

/// A menu bar, and the target its custom items point at. AppKit does not retain
/// an item's target, so whoever holds the bar holds this too.
pub(crate) struct BuiltMenuBar {
    pub(crate) bar: Retained<NSMenu>,
    pub(crate) _target: Retained<MenuTarget>,
}

pub(crate) fn build_menu_bar(mtm: MainThreadMarker, menus: &[Menu]) -> BuiltMenuBar {
    let target = MenuTarget::new(mtm);
    let bar = NSMenu::new(mtm);
    // The runtime decides what is enabled; AppKit's own validation would
    // disable an item whose target does not answer the action.
    bar.setAutoenablesItems(false);
    for menu in menus {
        let top = NSMenuItem::new(mtm);
        top.setSubmenu(Some(&build_menu(mtm, &target, menu)));
        bar.addItem(&top);
    }
    BuiltMenuBar {
        bar,
        _target: target,
    }
}

fn build_menu(mtm: MainThreadMarker, target: &MenuTarget, menu: &Menu) -> Retained<NSMenu> {
    let built = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(&menu.label));
    built.setAutoenablesItems(false);
    for entry in &menu.entries {
        match entry {
            MenuEntry::Separator => built.addItem(&NSMenuItem::separatorItem(mtm)),
            MenuEntry::Submenu(submenu) => {
                let item = NSMenuItem::new(mtm);
                item.setTitle(&NSString::from_str(&submenu.label));
                item.setSubmenu(Some(&build_menu(mtm, target, submenu)));
                built.addItem(&item);
            }
            MenuEntry::Item(entry) => built.addItem(&build_item(mtm, target, entry)),
        }
    }
    built
}

fn build_item(
    mtm: MainThreadMarker,
    target: &MenuTarget,
    entry: &MenuItem,
) -> Retained<NSMenuItem> {
    let (key, flags) = entry
        .shortcut
        .as_ref()
        .and_then(key_equivalent)
        .unwrap_or_else(|| (String::new(), NSEventModifierFlags::empty()));

    let action = action_for(entry);
    let selector = match action {
        Action::Selector(name) => {
            let name = CString::new(name).expect("selector names contain no NUL");
            Sel::register(&name)
        }
        Action::Command(_) => sel!(xuiMenuItem:),
    };

    // SAFETY: the selector is either a standard AppKit action or `xuiMenuItem:`,
    // which `MenuTarget` implements, and the key equivalent is a plain string.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(&entry.label),
            Some(selector),
            &NSString::from_str(&key),
        )
    };
    item.setKeyEquivalentModifierMask(flags);
    item.setEnabled(entry.enabled);
    if entry.checked == Some(true) {
        item.setState(NSControlStateValueOn);
    }
    if let Action::Command(command) = action {
        item.setTag(target.register(command));
        let target: &AnyObject = target;
        // SAFETY: `target` outlives the item: `BuiltMenuBar` holds both.
        unsafe { item.setTarget(Some(target)) };
    }
    item
}

/// What the application is called in its menu: the process name, which for a
/// bundled app is the bundle's executable.
pub(crate) fn application_name() -> String {
    NSProcessInfo::processInfo().processName().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_for(command: CommandId) -> MenuItem {
        MenuItem::new("x", command)
    }

    #[test]
    fn standard_commands_become_responder_chain_selectors() {
        for (command, selector) in [
            (commands::UNDO, "undo:"),
            (commands::REDO, "redo:"),
            (commands::CUT, "cut:"),
            (commands::COPY, "copy:"),
            (commands::PASTE, "paste:"),
            (commands::SELECT_ALL, "selectAll:"),
        ] {
            assert_eq!(
                action_for(&item_for(command)),
                Action::Selector(selector),
                "{command:?}"
            );
        }
    }

    #[test]
    fn a_role_wins_over_the_command_and_an_unknown_command_is_custom() {
        let mut quit = item_for(CommandId("app.quit"));
        quit.role = Some(MenuRole::Quit);
        assert_eq!(action_for(&quit), Action::Selector("terminate:"));

        assert_eq!(
            action_for(&item_for(CommandId("app.save"))),
            Action::Command(CommandId("app.save"))
        );
    }

    #[test]
    fn a_primary_shortcut_is_command_on_a_mac() {
        let (key, flags) = key_equivalent(&command_shortcut(PhysicalKey::KeyC, false)).unwrap();
        assert_eq!(key, "c");
        assert_eq!(flags, NSEventModifierFlags::Command);
    }

    #[test]
    fn modifiers_combine() {
        let shortcut = Shortcut::physical(PhysicalKey::KeyZ).modifiers(ShortcutModifiers {
            shift: true,
            alt: true,
            primary: true,
            ..ShortcutModifiers::default()
        });
        let (key, flags) = key_equivalent(&shortcut).unwrap();
        assert_eq!(key, "z");
        assert_eq!(
            flags,
            NSEventModifierFlags::Shift
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Command
        );
    }

    #[test]
    fn named_keys_use_appkits_function_key_code_points() {
        let (up, _) = key_equivalent(&Shortcut::named(NamedKey::ArrowUp)).unwrap();
        assert_eq!(up, "\u{F700}");
        let (f1, _) = key_equivalent(&Shortcut::named(NamedKey::F(1))).unwrap();
        assert_eq!(f1, "\u{F704}");
        let (f12, _) = key_equivalent(&Shortcut::named(NamedKey::F(12))).unwrap();
        assert_eq!(f12, "\u{F70F}");
        let (enter, _) = key_equivalent(&Shortcut::named(NamedKey::Enter)).unwrap();
        assert_eq!(enter, "\r");
    }

    #[test]
    fn a_key_a_menu_cannot_show_has_no_key_equivalent() {
        assert!(key_equivalent(&Shortcut::named(NamedKey::Shift)).is_none());
        assert!(key_equivalent(&Shortcut::physical(PhysicalKey::CapsLock)).is_none());
    }

    #[test]
    fn the_default_edit_menu_covers_every_standard_edit_command() {
        let menus = default_menus();
        let edit = menus.iter().find(|menu| menu.label == "Edit").unwrap();
        let commands_in_edit: Vec<_> = edit
            .entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some(item.command),
                _ => None,
            })
            .collect();
        for command in [
            commands::UNDO,
            commands::REDO,
            commands::CUT,
            commands::COPY,
            commands::PASTE,
            commands::SELECT_ALL,
        ] {
            assert!(commands_in_edit.contains(&command), "{command:?}");
        }
    }

    #[test]
    fn every_default_item_has_a_native_action_and_most_have_a_key() {
        for menu in complete_menu_bar("App", None) {
            for entry in &menu.entries {
                if let MenuEntry::Item(item) = entry {
                    assert!(
                        matches!(action_for(item), Action::Selector(_)),
                        "{} needs no target",
                        item.label
                    );
                    if let Some(shortcut) = &item.shortcut {
                        assert!(key_equivalent(shortcut).is_some(), "{}", item.label);
                    }
                }
            }
        }
    }

    #[test]
    fn the_application_menu_is_always_first_and_the_apps_menus_follow() {
        let app = MenuBar(vec![Menu {
            label: "File".into(),
            entries: Vec::new(),
        }]);
        let labels: Vec<_> = complete_menu_bar("Demo", Some(&app))
            .into_iter()
            .map(|menu| menu.label)
            .collect();
        assert_eq!(labels, ["Demo", "File"]);

        let labels: Vec<_> = complete_menu_bar("Demo", None)
            .into_iter()
            .map(|menu| menu.label)
            .collect();
        assert_eq!(labels, ["Demo", "Edit", "Window"]);
    }
}
