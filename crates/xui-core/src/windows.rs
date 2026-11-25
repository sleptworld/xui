//! The application's windows, each a tree of its own.
//!
//! An [`App`](crate::app::App) has one reactive runtime for all its windows --
//! a signal or a context is the same in every one -- and one [`WindowState`]
//! per window: the host tree with its focus, hover and overlays, the event
//! translator that turns the window's raw input into semantic events, and the
//! size it is laid out at. Node ids come from one allocator shared by every
//! window's tree, so a `NodeId` names one node app-wide.
//!
//! There is only the main window for now: the host makes it at startup, and
//! nothing yet opens another. See `docs/multi-window.md`.

use slotmap::SlotMap;
use xui_interface::WindowId;

use crate::core::Size;
use crate::event_system::translator::EventTranslator;
use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::tree::NodeIds;

/// One window's share of the app.
pub(crate) struct WindowState {
    pub(crate) ui: UiRuntime,
    pub(crate) event_translator: EventTranslator,
    /// The logical size the window is laid out at; zero until the host
    /// reports one.
    pub(crate) size: Size<f32>,
}

impl WindowState {
    fn new(ui: UiRuntime) -> Self {
        Self {
            ui,
            event_translator: EventTranslator::default(),
            size: Size::<f32>::ZERO,
        }
    }
}

pub(crate) struct Windows {
    /// Read once a window other than the main one opens.
    #[cfg_attr(not(test), allow(dead_code))]
    ids: NodeIds,
    map: SlotMap<WindowId, WindowState>,
    main: WindowId,
}

impl Windows {
    /// The main window alone.
    pub(crate) fn new() -> Self {
        let ids = NodeIds::default();
        let mut map = SlotMap::with_key();
        let main =
            map.insert_with_key(|id| WindowState::new(UiRuntime::with_node_ids(ids.clone(), id)));
        Self { ids, map, main }
    }

    /// The window the host made at startup, which [`App::mount`] mounts into.
    ///
    /// [`App::mount`]: crate::app::App::mount
    #[cfg(test)]
    pub(crate) fn main_id(&self) -> WindowId {
        self.main
    }

    #[inline]
    pub(crate) fn main(&self) -> &WindowState {
        &self.map[self.main]
    }

    #[inline]
    pub(crate) fn main_mut(&mut self) -> &mut WindowState {
        &mut self.map[self.main]
    }

    /// The allocator every window's tree takes its node ids from.
    #[cfg(test)]
    pub(crate) fn node_ids(&self) -> &NodeIds {
        &self.ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_main_window_takes_its_ids_from_the_shared_allocator() {
        let windows = Windows::new();
        let root = windows.main().ui.root();
        assert_eq!(windows.node_ids().window_of(root), Some(windows.main_id()));
    }
}
