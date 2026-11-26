//! The runtime's side of the host contract: where application code leaves
//! things for the host to do, and where the host's answers arrive.
//!
//! Three kinds of traffic pass through a [`PlatformHandle`]:
//!
//! - **state the runtime wants kept true** -- the window title, the menu bar,
//!   what a native callback may need to know about the surface -- which the
//!   shell reads into [`xui_interface::PlatformOutput`] and diffs;
//! - **commands**, one-shot actions the shell drains and hands to the host;
//! - **requests**, commands that answer. A request carries a callback, run when
//!   the reply arrives, so the code that asked for a clipboard read or a file
//!   dialog does not have to stay around polling for it.
//!
//! A handle is one window's: the state it sets is that window's, and the
//! commands it sends are marked as that window's, while the queue, the
//! requests and what the host said about itself are the app's, shared by
//! every window's handle. Each window's tree holds its own, so what a widget
//! sends goes out as its window's.
//!
//! The handle is a cheap clone over shared state, so an event callback can hold
//! one. It is not `Send`: everything here runs on the loop thread, and an answer
//! produced elsewhere reaches it through the shell's event route.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use xui_interface::{
    Appearance, DropTarget, HostCapabilities, MenuBar, NodeId, PlatformCommand, PlatformReply,
    PlatformRequest, PublishedSurface, RequestId, WindowId, WindowMenu, WindowState,
};

type ReplyCallback = Box<dyn FnOnce(PlatformReply)>;

/// What every window's handle shares: the queue, the requests waiting on an
/// answer, and what the host said about itself.
#[derive(Default)]
struct Bridge {
    next_request: u64,
    /// Each command with the window whose handle sent it.
    commands: Vec<(WindowId, PlatformCommand)>,
    pending: HashMap<RequestId, ReplyCallback>,
    /// The application's menu bar: the default, and what a platform shows
    /// with no window at all.
    menu_bar: Option<std::sync::Arc<MenuBar>>,
    capabilities: HostCapabilities,
    appearance: Appearance,
    windows: HashMap<WindowId, WindowBridge>,
    /// Answers to requests a node made, waiting for the runtime to deliver them
    /// to that node as events. See [`PlatformHandle::request_for_node`].
    node_replies: Vec<NodeReply>,
}

/// One window's state: what the runtime wants kept true of it, and what the
/// host reported of it.
#[derive(Default)]
struct WindowBridge {
    title: Option<String>,
    menu: WindowMenu,
    surface: PublishedSurface,
    window_state: WindowState,
}

impl Bridge {
    fn window(&mut self, window: WindowId) -> &mut WindowBridge {
        self.windows.entry(window).or_default()
    }
}

/// An answer to a request a node made, on its way back to it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NodeReply {
    pub(crate) node: NodeId,
    pub(crate) request: RequestId,
    pub(crate) reply: PlatformReply,
}

/// What the application can ask of the host, and what the host has told it,
/// as one of its windows.
///
/// Every window's handle is over the same state: a request made through one
/// is answered like one made through any other, and the capabilities and the
/// appearance are the same for all. What is a window's -- its title, menu,
/// surface and state -- is this handle's window's, and every command sent
/// through it is marked as that window's, so a window command goes to it.
/// [`crate::app::App::platform`] is the main window's.
#[derive(Clone, Default)]
pub struct PlatformHandle {
    bridge: Rc<RefCell<Bridge>>,
    window: WindowId,
}

impl PlatformHandle {
    /// A handle for `window`, over state of its own.
    pub(crate) fn new(window: WindowId) -> Self {
        Self {
            bridge: Rc::default(),
            window,
        }
    }

    /// A handle for `window`, over the same state as this one.
    pub(crate) fn for_window(&self, window: WindowId) -> Self {
        Self {
            bridge: Rc::clone(&self.bridge),
            window,
        }
    }

    /// The window this handle is.
    pub fn window(&self) -> WindowId {
        self.window
    }

    /// Queues a one-shot command for the host, as this handle's window's.
    ///
    /// Sent in order, after the event or frame that queued it. A host that does
    /// not implement it logs the fact and nothing else happens.
    pub fn send(&self, command: PlatformCommand) {
        self.bridge
            .borrow_mut()
            .commands
            .push((self.window, command));
    }

    /// Asks the host something, and runs `on_reply` with the answer.
    ///
    /// The callback always runs, at most once, unless the request is cancelled:
    /// a host that cannot do what was asked answers
    /// [`PlatformReply::Unsupported`] rather than staying silent. It runs from
    /// the event route, never from inside this call.
    pub fn request(
        &self,
        request: PlatformRequest,
        on_reply: impl FnOnce(PlatformReply) + 'static,
    ) -> RequestId {
        self.register(request, |_| Box::new(on_reply))
    }

    /// Allocates an id, files the callback `make` builds for it, and queues the
    /// request. The callback is built from the id because one of the two callers
    /// needs to know the id it was registered under.
    fn register(
        &self,
        request: PlatformRequest,
        make: impl FnOnce(RequestId) -> ReplyCallback,
    ) -> RequestId {
        let mut bridge = self.bridge.borrow_mut();
        let id = RequestId(bridge.next_request);
        bridge.next_request += 1;
        bridge.pending.insert(id, make(id));
        bridge
            .commands
            .push((self.window, PlatformCommand::Request(id, request)));
        id
    }

    /// Asks the host something on behalf of `node`, whose answer comes back to
    /// that node as a [`xui_interface::SemanticEvent::PlatformReply`] rather than
    /// to a callback.
    ///
    /// That is how a widget makes a request: a callback has to be `'static` and
    /// cannot reach the widget, but an event arrives with the widget's own
    /// context, so it can edit itself and mark itself for layout. A reply for a
    /// node that is gone by then is dropped.
    pub fn request_for_node(&self, node: NodeId, request: PlatformRequest) -> RequestId {
        // Weak, so a request nobody ever answers does not keep the bridge alive
        // through the callback stored in it.
        let bridge = Rc::downgrade(&self.bridge);
        self.register(request, move |id| {
            Box::new(move |reply| {
                if let Some(bridge) = bridge.upgrade() {
                    bridge.borrow_mut().node_replies.push(NodeReply {
                        node,
                        request: id,
                        reply,
                    });
                }
            })
        })
    }

    /// Withdraws a request whose answer is no longer wanted, so its callback
    /// never runs and the host may dismiss whatever it was showing. Returns
    /// whether the request was still waiting.
    pub fn cancel(&self, id: RequestId) -> bool {
        let mut bridge = self.bridge.borrow_mut();
        let waiting = bridge.pending.remove(&id).is_some();
        if waiting {
            bridge
                .commands
                .push((self.window, PlatformCommand::Cancel(id)));
        }
        waiting
    }

    /// Sets this handle's window's title. It stays set; the host applies only
    /// changes.
    pub fn set_title(&self, title: impl Into<String>) {
        self.bridge.borrow_mut().window(self.window).title = Some(title.into());
    }

    /// Replaces the application's menu bar: the default, which a window
    /// [`WindowMenu::Inherit`]s, and what shows with no window at all. `None`
    /// returns to the host's default.
    pub fn set_menu_bar(&self, menu_bar: Option<MenuBar>) {
        self.bridge.borrow_mut().menu_bar = menu_bar.map(std::sync::Arc::new);
    }

    /// Sets the menu that applies while this handle's window is active. See
    /// [`WindowMenu`] for what each platform makes of it.
    pub fn set_menu(&self, menu: WindowMenu) {
        self.bridge.borrow_mut().window(self.window).menu = menu;
    }

    /// Publishes what the host may read of this handle's window without
    /// asking the runtime: window regions, drop targets, the focused text
    /// input's document.
    ///
    /// Drop targets set here last until the next frame, which replaces them
    /// with those the nodes declare.
    pub fn set_surface(&self, surface: PublishedSurface) {
        self.bridge.borrow_mut().window(self.window).surface = surface;
    }

    /// Replaces this handle's window's published drop targets, which the
    /// runtime gathers from the nodes that declare `drop_accepts`, after each
    /// layout.
    pub(crate) fn set_drop_targets(&self, targets: Vec<DropTarget>) {
        let mut bridge = self.bridge.borrow_mut();
        let surface = &mut bridge.window(self.window).surface;
        if *surface.drop_targets != *targets {
            surface.drop_targets = targets.into();
        }
    }

    /// What the host can do, as it reported at startup.
    pub fn capabilities(&self) -> HostCapabilities {
        self.bridge.borrow().capabilities
    }

    /// The system's current appearance.
    pub fn appearance(&self) -> Appearance {
        self.bridge.borrow().appearance
    }

    /// Whether this handle's window is fullscreen or maximized, including by
    /// the user.
    pub fn window_state(&self) -> WindowState {
        self.bridge
            .borrow()
            .windows
            .get(&self.window)
            .map_or_else(WindowState::default, |window| window.window_state)
    }

    pub(crate) fn take_node_replies(&self) -> Vec<NodeReply> {
        std::mem::take(&mut self.bridge.borrow_mut().node_replies)
    }

    /// Every window's commands, in the order they were sent, each with the
    /// window that sent it.
    pub(crate) fn take_commands(&self) -> Vec<(WindowId, PlatformCommand)> {
        std::mem::take(&mut self.bridge.borrow_mut().commands)
    }

    /// Runs the callback waiting on `id`. A reply for a request that was
    /// cancelled, or already answered, finds nothing and is dropped.
    ///
    /// The callback runs with the bridge unborrowed, so it may send commands
    /// or make requests of its own.
    pub(crate) fn deliver_reply(&self, id: RequestId, reply: PlatformReply) -> bool {
        let callback = self.bridge.borrow_mut().pending.remove(&id);
        match callback {
            Some(callback) => {
                callback(reply);
                true
            }
            None => false,
        }
    }

    pub(crate) fn set_capabilities(&self, capabilities: HostCapabilities) {
        self.bridge.borrow_mut().capabilities = capabilities;
    }

    pub(crate) fn set_appearance(&self, appearance: Appearance) {
        self.bridge.borrow_mut().appearance = appearance;
    }

    /// Records what the host reported of this handle's window's state.
    pub(crate) fn set_window_state(&self, window_state: WindowState) {
        self.bridge.borrow_mut().window(self.window).window_state = window_state;
    }

    /// Forgets window `window`, which closed.
    pub(crate) fn forget_window(&self, window: WindowId) {
        self.bridge.borrow_mut().windows.remove(&window);
    }

    /// The parts of this handle's window's [`xui_interface::PlatformOutput`]
    /// that come from here.
    pub(crate) fn output(&self) -> WindowOutput {
        let bridge = self.bridge.borrow();
        let window = bridge.windows.get(&self.window);
        WindowOutput {
            title: window.and_then(|window| window.title.clone()),
            menu_bar: bridge.menu_bar.clone(),
            menu: window.map(|window| window.menu.clone()).unwrap_or_default(),
            surface: window
                .map(|window| window.surface.clone())
                .unwrap_or_default(),
        }
    }
}

/// What [`PlatformHandle::output`] gives a window's `PlatformOutput`.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct WindowOutput {
    pub(crate) title: Option<String>,
    /// The application's, the same for every window.
    pub(crate) menu_bar: Option<std::sync::Arc<MenuBar>>,
    pub(crate) menu: WindowMenu,
    pub(crate) surface: PublishedSurface,
}

/// Whether `command` acts on the window it is sent for -- moves it, resizes
/// it, shows a menu at a point in it -- rather than on the app.
pub(crate) fn targets_window(command: &PlatformCommand) -> bool {
    matches!(
        command,
        PlatformCommand::Window(_) | PlatformCommand::ShowContextMenu { .. }
    )
}

impl std::fmt::Debug for PlatformHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bridge = self.bridge.borrow();
        f.debug_struct("PlatformHandle")
            .field("window", &self.window)
            .field("queued_commands", &bridge.commands.len())
            .field("pending_requests", &bridge.pending.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use xui_interface::{ClipboardContent, ClipboardTarget};

    use super::*;

    fn read_clipboard() -> PlatformRequest {
        PlatformRequest::ReadClipboard(ClipboardTarget::General)
    }

    /// The commands queued, without the window each came from.
    fn commands(handle: &PlatformHandle) -> Vec<PlatformCommand> {
        handle
            .take_commands()
            .into_iter()
            .map(|(_, command)| command)
            .collect()
    }

    #[test]
    fn request_queues_a_command_and_runs_its_callback_once() {
        let handle = PlatformHandle::default();
        let seen = Rc::new(Cell::new(0));
        let id = handle.request(read_clipboard(), {
            let seen = Rc::clone(&seen);
            move |_| seen.set(seen.get() + 1)
        });

        assert_eq!(
            commands(&handle),
            vec![PlatformCommand::Request(id, read_clipboard())]
        );
        assert_eq!(seen.get(), 0, "the callback does not run at request time");

        let reply = PlatformReply::Clipboard(Some(ClipboardContent::Text("hi".into())));
        assert!(handle.deliver_reply(id, reply.clone()));
        assert!(
            !handle.deliver_reply(id, reply),
            "a second reply is dropped"
        );
        assert_eq!(seen.get(), 1);
    }

    #[test]
    fn request_ids_are_distinct() {
        let handle = PlatformHandle::default();
        let first = handle.request(read_clipboard(), |_| {});
        let second = handle.request(read_clipboard(), |_| {});
        assert_ne!(first, second);
    }

    #[test]
    fn cancelling_drops_the_callback_and_tells_the_host() {
        let handle = PlatformHandle::default();
        let ran = Rc::new(Cell::new(false));
        let id = handle.request(read_clipboard(), {
            let ran = Rc::clone(&ran);
            move |_| ran.set(true)
        });
        let _ = handle.take_commands();

        assert!(handle.cancel(id));
        assert_eq!(commands(&handle), vec![PlatformCommand::Cancel(id)]);
        assert!(!handle.deliver_reply(id, PlatformReply::Unsupported));
        assert!(!ran.get());
        assert!(!handle.cancel(id), "cancelling twice does nothing");
        assert!(handle.take_commands().is_empty());
    }

    #[test]
    fn a_callback_may_use_the_handle() {
        let handle = PlatformHandle::default();
        let id = handle.request(read_clipboard(), {
            let handle = handle.clone();
            move |_| handle.send(PlatformCommand::OpenUrl("https://example.com".into()))
        });
        let _ = handle.take_commands();

        assert!(handle.deliver_reply(id, PlatformReply::Unsupported));
        assert_eq!(
            commands(&handle),
            vec![PlatformCommand::OpenUrl("https://example.com".into())]
        );
    }

    #[test]
    fn output_reflects_what_was_set() {
        let handle = PlatformHandle::default();
        assert_eq!(handle.output().title, None);
        handle.set_title("Docs");
        handle.set_menu_bar(Some(MenuBar::default()));
        let output = handle.output();
        assert_eq!(output.title.as_deref(), Some("Docs"));
        assert_eq!(output.menu_bar.as_deref(), Some(&MenuBar::default()));
        handle.set_menu_bar(None);
        assert_eq!(handle.output().menu_bar, None);
    }

    fn two_windows() -> (PlatformHandle, PlatformHandle) {
        let mut ids = slotmap::SlotMap::<WindowId, ()>::with_key();
        let main = PlatformHandle::new(ids.insert(()));
        let second = main.for_window(ids.insert(()));
        (main, second)
    }

    #[test]
    fn what_is_a_windows_is_kept_apart_and_what_is_the_apps_is_shared() {
        let (main, second) = two_windows();
        main.set_title("Main");
        second.set_title("Settings");
        second.set_menu(WindowMenu::Hidden);
        second.set_window_state(WindowState {
            fullscreen: true,
            maximized: false,
        });
        main.set_menu_bar(Some(MenuBar::default()));
        main.set_capabilities(HostCapabilities::MULTI_WINDOW);

        assert_eq!(main.output().title.as_deref(), Some("Main"));
        assert_eq!(second.output().title.as_deref(), Some("Settings"));
        assert_eq!(main.output().menu, WindowMenu::Inherit);
        assert_eq!(second.output().menu, WindowMenu::Hidden);
        assert!(!main.window_state().fullscreen);
        assert!(second.window_state().fullscreen);
        // The app's menu bar and what the host said of itself, through either.
        assert_eq!(
            second.output().menu_bar.as_deref(),
            Some(&MenuBar::default())
        );
        assert_eq!(second.capabilities(), HostCapabilities::MULTI_WINDOW);

        main.forget_window(second.window());
        assert_eq!(second.output().title, None);
        assert_eq!(main.output().title.as_deref(), Some("Main"));
    }

    #[test]
    fn commands_carry_the_window_that_sent_them_in_one_queue() {
        let (main, second) = two_windows();
        let seen = Rc::new(Cell::new(false));
        main.send(PlatformCommand::OpenUrl("https://example.com".into()));
        let id = second.request(read_clipboard(), {
            let seen = Rc::clone(&seen);
            move |_| seen.set(true)
        });
        assert_eq!(
            main.take_commands(),
            vec![
                (
                    main.window(),
                    PlatformCommand::OpenUrl("https://example.com".into())
                ),
                (
                    second.window(),
                    PlatformCommand::Request(id, read_clipboard())
                ),
            ]
        );
        // A request is the app's: answered wherever the reply comes in.
        assert!(main.deliver_reply(id, PlatformReply::Unsupported));
        assert!(seen.get());
    }
}
