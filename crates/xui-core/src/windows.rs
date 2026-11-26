//! The application's windows, each a tree of its own.
//!
//! An [`App`](crate::app::App) has one reactive runtime for all its windows --
//! a signal or a context is the same in every one -- and one [`WindowState`]
//! per window: the host tree with its focus, hover and overlays, the event
//! translator that turns the window's raw input into semantic events, and the
//! size it is laid out at. Each window's node ids are its own, dense from 0,
//! and carry the window's tag, so a `NodeId` still names one node app-wide.
//!
//! The main window is the host's, made at startup. Any other is opened by a
//! `window(..) { .. }` block as it mounts and closed as it unmounts; what the
//! host has to hear of that waits in [`Windows::take_changes`]. See
//! `docs/multi-window.md`.

use std::rc::Rc;

use slotmap::SlotMap;
use xui_interface::{NodeId, WindowId, WindowOptions};

use crate::core::Size;
use crate::event_system::translator::EventTranslator;
use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::tree::{MAX_GENERATION, NodeAlloc, TAGS, tag_of};

/// One window's share of the app.
pub struct WindowState {
    pub(crate) ui: UiRuntime,
    pub(crate) event_translator: EventTranslator,
    /// The logical size the window is laid out at; zero until the host
    /// reports one.
    pub(crate) size: Size<f32>,
    /// What the block that opened the window runs when the user asks to
    /// close it; `None` for the main window, and for a block without one.
    pub(crate) on_close_requested: Option<Rc<dyn Fn()>>,
}

impl WindowState {
    fn new(ui: UiRuntime) -> Self {
        Self {
            ui,
            event_translator: EventTranslator::default(),
            size: Size::<f32>::ZERO,
            on_close_requested: None,
        }
    }
}

/// Which window each node id tag is, and where a window given a tag starts.
struct Tags {
    owners: Vec<Option<WindowId>>,
    /// The generation a window given the tag starts at: above every one the
    /// windows that had it gave out.
    floors: Vec<u32>,
    /// Where the search for a free tag starts, so tags go round before one is
    /// given again.
    next: usize,
}

impl Tags {
    fn new() -> Self {
        Self {
            owners: vec![None; TAGS],
            floors: vec![0; TAGS],
            next: 0,
        }
    }

    /// A tag for `window`, and the ids it makes. One that has not been given
    /// for longest, of those with generations left.
    fn take(&mut self, window: WindowId) -> NodeAlloc {
        let tag = (0..TAGS)
            .map(|offset| (self.next + offset) % TAGS)
            .find(|&tag| self.owners[tag].is_none() && self.floors[tag] <= MAX_GENERATION)
            .expect("no more than 1024 windows open at once, nor tags spent");
        self.owners[tag] = Some(window);
        self.next = (tag + 1) % TAGS;
        NodeAlloc::new(tag as u32, self.floors[tag])
    }

    /// The window with `ids`' tag closed.
    fn give_back(&mut self, ids: &NodeAlloc) {
        let tag = ids.tag() as usize;
        self.owners[tag] = None;
        self.floors[tag] = ids.next_floor();
    }

    fn owner(&self, tag: u32) -> Option<WindowId> {
        self.owners.get(tag as usize).copied().flatten()
    }
}

/// A window opened or closed since the host last heard.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WindowChange {
    Opened(WindowId, WindowOptions),
    Closed(WindowId),
}

pub struct Windows {
    tags: Tags,
    map: SlotMap<WindowId, WindowState>,
    main: WindowId,
    changes: Vec<WindowChange>,
    /// Windows closed whose trees still hold what their removed nodes told
    /// the app, until it has passed that on (see [`Self::take_closed`]).
    closed: Vec<WindowState>,
}

impl Windows {
    /// The main window alone.
    pub(crate) fn new() -> Self {
        let mut tags = Tags::new();
        let mut map = SlotMap::with_key();
        let main =
            map.insert_with_key(|id| WindowState::new(UiRuntime::with_node_ids(tags.take(id))));
        Self {
            tags,
            map,
            main,
            changes: Vec::new(),
            closed: Vec::new(),
        }
    }

    /// The window the host made at startup, which [`App::mount`] mounts into.
    ///
    /// [`App::mount`]: crate::app::App::mount
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

    /// A window beside the main one, with a tree of its own on the shared
    /// node ids. The host hears of it from [`Self::take_changes`].
    pub(crate) fn open(
        &mut self,
        options: WindowOptions,
        on_close_requested: Option<Rc<dyn Fn()>>,
    ) -> WindowId {
        let tags = &mut self.tags;
        let id = self.map.insert_with_key(|id| WindowState {
            on_close_requested,
            ..WindowState::new(UiRuntime::with_node_ids(tags.take(id)))
        });
        self.changes.push(WindowChange::Opened(id, options));
        id
    }

    /// Closes window `id`, which must not be the main one. Its tree is kept
    /// until [`Self::take_closed`], so the nodes removed from it on the way
    /// out are still heard of.
    pub(crate) fn close(&mut self, id: WindowId) {
        assert_ne!(id, self.main, "the main window is the host's to close");
        let state = self.map.remove(id).expect("a window closed twice");
        // No node is made in it from here, so its tag can go to another
        // window, which starts above every generation this one gave out.
        self.tags.give_back(state.ui.node_ids());
        self.closed.push(state);
        self.changes.push(WindowChange::Closed(id));
    }

    /// Windows opened and closed since the last call, in order.
    pub(crate) fn take_changes(&mut self) -> Vec<WindowChange> {
        std::mem::take(&mut self.changes)
    }

    /// The trees of the windows closed since the last call.
    pub(crate) fn take_closed(&mut self) -> Vec<WindowState> {
        std::mem::take(&mut self.closed)
    }

    /// Every open window's tree, the main one included.
    pub(crate) fn uis_mut(&mut self) -> impl Iterator<Item = &mut UiRuntime> {
        self.map.values_mut().map(|state| &mut state.ui)
    }

    #[inline]
    pub(crate) fn get(&self, id: WindowId) -> Option<&WindowState> {
        self.map.get(id)
    }

    /// Window `id`'s tree.
    #[inline]
    pub(crate) fn ui(&self, id: WindowId) -> &UiRuntime {
        &self.map[id].ui
    }

    /// Window `id`'s tree, to change.
    #[inline]
    pub(crate) fn ui_mut(&mut self, id: WindowId) -> &mut UiRuntime {
        &mut self.map[id].ui
    }

    /// The window whose tree `node` is in, if it is still in one.
    pub(crate) fn try_window_of(&self, node: NodeId) -> Option<WindowId> {
        let window = self.tags.owner(tag_of(node))?;
        self.map[window].ui.contains(node).then_some(window)
    }

    /// Every open window, the main one included.
    pub(crate) fn ids(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.map.keys()
    }

    /// Every open window's tree, the main one included.
    pub(crate) fn uis(&self) -> impl Iterator<Item = &UiRuntime> {
        self.map.values().map(|state| &state.ui)
    }

    /// The window whose tree `node` is in. `node` must be live.
    pub(crate) fn window_of(&self, node: NodeId) -> WindowId {
        let window = self
            .tags
            .owner(tag_of(node))
            .expect("a live node is in an open window's tree");
        debug_assert!(self.map[window].ui.contains(node), "a stale node id");
        window
    }
}

impl std::ops::Index<WindowId> for Windows {
    type Output = WindowState;

    /// Open window `id`'s state; `id` must be open.
    #[inline]
    fn index(&self, id: WindowId) -> &WindowState {
        &self.map[id]
    }
}

impl std::ops::IndexMut<WindowId> for Windows {
    #[inline]
    fn index_mut(&mut self, id: WindowId) -> &mut WindowState {
        &mut self.map[id]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_window_numbers_its_nodes_from_zero_under_a_tag_of_its_own() {
        let mut windows = Windows::new();
        let main_root = windows.main().ui.root();
        let id = windows.open(WindowOptions::default(), None);
        let root = windows.ui(id).root();
        let index = |node: NodeId| slotmap::Key::data(&node).as_ffi() as u32;
        // Both roots are the first node of their window...
        assert_eq!(index(root), index(main_root));
        // ...and still two nodes, each found in its own window.
        assert_ne!(root, main_root);
        assert_eq!(windows.window_of(main_root), windows.main_id());
        assert_eq!(windows.window_of(root), id);
    }

    #[test]
    fn opening_and_closing_a_window_is_reported_once_each() {
        let mut windows = Windows::new();
        let id = windows.open(WindowOptions::default().with_title("Second"), None);
        let root = windows.ui(id).root();
        assert_eq!(windows.try_window_of(root), Some(id));
        windows.close(id);
        assert_eq!(
            windows.take_changes(),
            [
                WindowChange::Opened(id, WindowOptions::default().with_title("Second")),
                WindowChange::Closed(id),
            ]
        );
        assert!(windows.take_changes().is_empty());
        assert!(windows.get(id).is_none());
        assert_eq!(windows.try_window_of(root), None, "its window is closed");
        drop(windows.take_closed());
    }

    #[test]
    fn a_stale_id_of_a_closed_window_names_no_node_of_the_next_one() {
        let mut windows = Windows::new();
        let mut stale = Vec::new();
        // Every tag, round and round: windows come and go until the first
        // tag is given again.
        for _ in 0..=TAGS {
            let id = windows.open(WindowOptions::default(), None);
            stale.push(windows.ui(id).root());
            windows.close(id);
        }
        drop(windows.take_closed());
        let id = windows.open(WindowOptions::default(), None);
        let root = windows.ui(id).root();
        assert!(!stale.contains(&root));
        assert!(
            stale
                .iter()
                .all(|&node| windows.try_window_of(node).is_none())
        );
    }
}
