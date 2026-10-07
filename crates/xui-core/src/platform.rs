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
//! The handle is a cheap clone over shared state, so an event callback can hold
//! one. It is not `Send`: everything here runs on the loop thread, and an answer
//! produced elsewhere reaches it through the shell's event route.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use xui_interface::{
    Appearance, HostCapabilities, MenuBar, NodeId, PlatformCommand, PlatformReply, PlatformRequest,
    PublishedSurface, RequestId, WindowState,
};

type ReplyCallback = Box<dyn FnOnce(PlatformReply)>;

#[derive(Default)]
struct Bridge {
    next_request: u64,
    commands: Vec<PlatformCommand>,
    pending: HashMap<RequestId, ReplyCallback>,
    title: Option<String>,
    menu_bar: Option<std::sync::Arc<MenuBar>>,
    surface: PublishedSurface,
    capabilities: HostCapabilities,
    appearance: Appearance,
    window_state: WindowState,
    /// Answers to requests a node made, waiting for the runtime to deliver them
    /// to that node as events. See [`PlatformHandle::request_for_node`].
    node_replies: Vec<NodeReply>,
}

/// An answer to a request a node made, on its way back to it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NodeReply {
    pub(crate) node: NodeId,
    pub(crate) request: RequestId,
    pub(crate) reply: PlatformReply,
}

/// What the application can ask of the host, and what the host has told it.
#[derive(Clone, Default)]
pub struct PlatformHandle(Rc<RefCell<Bridge>>);

impl PlatformHandle {
    /// Queues a one-shot command for the host.
    ///
    /// Sent in order, after the event or frame that queued it. A host that does
    /// not implement it logs the fact and nothing else happens.
    pub fn send(&self, command: PlatformCommand) {
        self.0.borrow_mut().commands.push(command);
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
        let mut bridge = self.0.borrow_mut();
        let id = RequestId(bridge.next_request);
        bridge.next_request += 1;
        bridge.pending.insert(id, make(id));
        bridge.commands.push(PlatformCommand::Request(id, request));
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
        let bridge = Rc::downgrade(&self.0);
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
        let mut bridge = self.0.borrow_mut();
        let waiting = bridge.pending.remove(&id).is_some();
        if waiting {
            bridge.commands.push(PlatformCommand::Cancel(id));
        }
        waiting
    }

    /// Sets the window title. It stays set; the host applies only changes.
    pub fn set_title(&self, title: impl Into<String>) {
        self.0.borrow_mut().title = Some(title.into());
    }

    /// Replaces the application's menu bar. `None` returns to the host's
    /// default.
    pub fn set_menu_bar(&self, menu_bar: Option<MenuBar>) {
        self.0.borrow_mut().menu_bar = menu_bar.map(std::sync::Arc::new);
    }

    /// Publishes what the host may read without asking the runtime: window
    /// regions, drop targets, the focused text input's document.
    pub fn set_surface(&self, surface: PublishedSurface) {
        self.0.borrow_mut().surface = surface;
    }

    /// What the host can do, as it reported at startup.
    pub fn capabilities(&self) -> HostCapabilities {
        self.0.borrow().capabilities
    }

    /// The system's current appearance.
    pub fn appearance(&self) -> Appearance {
        self.0.borrow().appearance
    }

    /// Whether the window is fullscreen or maximized, including by the user.
    pub fn window_state(&self) -> WindowState {
        self.0.borrow().window_state
    }

    pub(crate) fn take_node_replies(&self) -> Vec<NodeReply> {
        std::mem::take(&mut self.0.borrow_mut().node_replies)
    }

    pub(crate) fn take_commands(&self) -> Vec<PlatformCommand> {
        std::mem::take(&mut self.0.borrow_mut().commands)
    }

    /// Runs the callback waiting on `id`. A reply for a request that was
    /// cancelled, or already answered, finds nothing and is dropped.
    ///
    /// The callback runs with the bridge unborrowed, so it may send commands
    /// or make requests of its own.
    pub(crate) fn deliver_reply(&self, id: RequestId, reply: PlatformReply) -> bool {
        let callback = self.0.borrow_mut().pending.remove(&id);
        match callback {
            Some(callback) => {
                callback(reply);
                true
            }
            None => false,
        }
    }

    pub(crate) fn set_capabilities(&self, capabilities: HostCapabilities) {
        self.0.borrow_mut().capabilities = capabilities;
    }

    pub(crate) fn set_appearance(&self, appearance: Appearance) {
        self.0.borrow_mut().appearance = appearance;
    }

    pub(crate) fn set_window_state(&self, window_state: WindowState) {
        self.0.borrow_mut().window_state = window_state;
    }

    /// The parts of [`xui_interface::PlatformOutput`] that come from here.
    pub(crate) fn output(
        &self,
    ) -> (
        Option<String>,
        Option<std::sync::Arc<MenuBar>>,
        PublishedSurface,
    ) {
        let bridge = self.0.borrow();
        (
            bridge.title.clone(),
            bridge.menu_bar.clone(),
            bridge.surface.clone(),
        )
    }
}

impl std::fmt::Debug for PlatformHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bridge = self.0.borrow();
        f.debug_struct("PlatformHandle")
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

    #[test]
    fn request_queues_a_command_and_runs_its_callback_once() {
        let handle = PlatformHandle::default();
        let seen = Rc::new(Cell::new(0));
        let id = handle.request(read_clipboard(), {
            let seen = Rc::clone(&seen);
            move |_| seen.set(seen.get() + 1)
        });

        assert_eq!(
            handle.take_commands(),
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
        assert_eq!(handle.take_commands(), vec![PlatformCommand::Cancel(id)]);
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
            handle.take_commands(),
            vec![PlatformCommand::OpenUrl("https://example.com".into())]
        );
    }

    #[test]
    fn output_reflects_what_was_set() {
        let handle = PlatformHandle::default();
        assert_eq!(handle.output().0, None);
        handle.set_title("Docs");
        handle.set_menu_bar(Some(MenuBar::default()));
        let (title, menu_bar, _) = handle.output();
        assert_eq!(title.as_deref(), Some("Docs"));
        assert_eq!(menu_bar.as_deref(), Some(&MenuBar::default()));
        handle.set_menu_bar(None);
        assert_eq!(handle.output().1, None);
    }
}
