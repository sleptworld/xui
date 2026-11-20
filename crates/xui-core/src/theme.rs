//! Which theme is in use: part of the app's environment, like its scale
//! factor, rather than reactive state.
//!
//! The [`App`](crate::app::App) provides a [`ThemeHandle`] as a context.
//! Setting a mode or themes through it records a request; the app takes it
//! before the next style pass, selects the theme from the mode, the themes and
//! the system color scheme, and when that is another theme restyles the tree
//! and redraws every painter canvas. Components do not rerun for it: tokens
//! resolve when styles compute, and painters read
//! [`CanvasPainter::theme`](crate::widgets::CanvasPainter::theme) when they
//! draw.
//!
//! ```ignore
//! let theme: ThemeHandle = context!();
//! // in a handler
//! theme.read().set_mode(ThemeMode::Dark);
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use xui_interface::{ColorScheme, Theme, ThemeMode, Themes};

/// The application's themes and which of them is in use. Clones share one
/// state; reads are not tracked.
#[derive(Clone, Debug)]
pub struct ThemeHandle(Rc<RefCell<State>>);

#[derive(Debug)]
struct State {
    mode: ThemeMode,
    themes: Themes,
    /// The theme selected at the last apply.
    current: Rc<Theme>,
    /// A mode or themes were set since the last apply.
    requested: bool,
}

impl ThemeHandle {
    pub(crate) fn new(system: ColorScheme) -> Self {
        let mode = ThemeMode::default();
        let themes = Themes::default();
        let current = themes.select(mode, system).clone();
        Self(Rc::new(RefCell::new(State {
            mode,
            themes,
            current,
            requested: false,
        })))
    }

    pub fn mode(&self) -> ThemeMode {
        self.0.borrow().mode
    }

    pub fn themes(&self) -> Themes {
        self.0.borrow().themes.clone()
    }

    /// The theme in use.
    pub fn current(&self) -> Rc<Theme> {
        self.0.borrow().current.clone()
    }

    /// Selects which theme is in use from the next frame;
    /// [`ThemeMode::System`] follows the system's color scheme.
    pub fn set_mode(&self, mode: ThemeMode) {
        let mut state = self.0.borrow_mut();
        state.mode = mode;
        state.requested = true;
    }

    /// Replaces the light and dark themes from the next frame.
    pub fn set_themes(&self, themes: Themes) {
        let mut state = self.0.borrow_mut();
        state.themes = themes;
        state.requested = true;
    }

    /// Whether a request waits for [`Self::apply`].
    pub(crate) fn is_requested(&self) -> bool {
        self.0.borrow().requested
    }

    /// Takes the request and selects again while the system is in `system`;
    /// the newly selected theme, if it is another one.
    pub(crate) fn apply(&self, system: ColorScheme) -> Option<Rc<Theme>> {
        let mut state = self.0.borrow_mut();
        state.requested = false;
        let selected = state.themes.select(state.mode, system).clone();
        if Rc::ptr_eq(&selected, &state.current) {
            return None;
        }
        state.current = selected.clone();
        Some(selected)
    }
}

/// The app provides one handle for its whole life, so as a context it never
/// changes.
impl PartialEq for ThemeHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

#[cfg(test)]
mod tests {
    use xui_interface::events::RawEvent;
    use xui_interface::tokens::color;
    use xui_interface::{Appearance, ColorScheme, ComputedColorStyle, Theme, ThemeMode};
    use xui_macros::component;

    use super::ThemeHandle;
    use crate::app::App;
    use crate::core::{Point, Size};
    use crate::reactive::InstanceId;
    use crate::render::MockRenderBackend;
    use crate::runtime::{GuiRuntime, RuntimeEvent};
    use crate::text::testing::ZeroTextBackend;
    use crate::ui_runtime::tests::{pointer_at, pointer_move};
    use crate::widgets::container::container;

    type Gui = GuiRuntime<MockRenderBackend, ZeroTextBackend>;

    #[component]
    fn dark_button() {
        let theme: ThemeHandle = context!();
        view! {
            container(
                width: 20.0,
                height: 20.0,
                background: color::BACKGROUND,
                on_click: |_| theme.read().set_mode(ThemeMode::Dark),
            )
        }
    }

    fn runtime() -> (Gui, InstanceId) {
        let mut app = App::new(|_| {});
        let id = app.mount::<DarkButton>(DarkButtonProps {});
        let mut gui = GuiRuntime::new(app, MockRenderBackend::default(), ZeroTextBackend);
        gui.handle_event(RuntimeEvent::Resize(Size::new(100.0, 100.0)));
        gui.frame().unwrap();
        (gui, id)
    }

    /// What the button's background token resolved to.
    fn background(gui: &Gui, id: InstanceId) -> ComputedColorStyle {
        let app = gui.app();
        let host = app.reactive().hosts(id)[0];
        app.ui_runtime()
            .node(host)
            .unwrap()
            .effective_style
            .paint
            .background
    }

    fn click(gui: &mut Gui) {
        let point = Point::new(10.0, 10.0);
        gui.handle_event(RuntimeEvent::Input(pointer_move(point)));
        gui.handle_event(RuntimeEvent::Input(RawEvent::PointerDown(pointer_at(
            point,
        ))));
        gui.handle_event(RuntimeEvent::Input(RawEvent::PointerUp(pointer_at(point))));
    }

    #[test]
    fn a_handler_switches_the_theme_from_the_next_frame() {
        let (mut gui, id) = runtime();
        assert_eq!(background(&gui, id), color::BACKGROUND.fallback.into());

        click(&mut gui);
        assert!(gui.app().is_dirty(), "the request alone asks for a frame");
        assert_eq!(gui.app().theme().scheme, ColorScheme::Light);

        gui.frame().unwrap();
        assert_eq!(gui.app().theme().scheme, ColorScheme::Dark);
        assert_eq!(
            background(&gui, id),
            Theme::dark().get(color::BACKGROUND).into()
        );
    }

    #[test]
    fn the_system_mode_follows_the_reported_appearance() {
        let (mut gui, _) = runtime();
        let dark = Appearance {
            color_scheme: ColorScheme::Dark,
            ..Appearance::default()
        };
        gui.handle_event(RuntimeEvent::Appearance(dark));
        assert_eq!(gui.app().theme().scheme, ColorScheme::Dark);

        gui.app_mut().set_theme_mode(ThemeMode::Light);
        assert_eq!(
            gui.app().theme().scheme,
            ColorScheme::Light,
            "an explicit mode ignores the system"
        );
    }
}
