use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use xui_core::app::AppRenderError;
use xui_core::render::RenderBackend;
use xui_core::text::TextHost;
use xui_interface::TextBackend;

use crate::published::PublishedHandle;
use crate::shell::{Shell, ShellControl, ShellEvent, ShellOutcome};
use crate::window::PlatformWindow;

struct Queued {
    sequence: u64,
    at: Instant,
    event: ShellEvent,
}

/// A [`Shell`] that can be handed events at any time, including from inside a
/// call it is itself making.
///
/// Native callbacks do not wait their turn. Showing a window reports focus;
/// turning input-method support off can end a composition; a modal dialog runs
/// a nested event loop. Each of those can deliver an event while the shell is
/// already handling one, and the shell is exclusively borrowed while it does.
/// Every host used to carry its own queue to absorb that. This is that queue,
/// written once.
///
/// [`Self::submit`] takes `&self` and never fails: an event that arrives while
/// the shell is busy is queued and handled by the call that is already running.
pub struct ShellHandle<W, B, T>
where
    W: PlatformWindow,
    B: RenderBackend<TextHost<T>>,
    T: TextBackend,
{
    shell: RefCell<Shell<W, B, T>>,
    window: Arc<W>,
    published: PublishedHandle,
    queue: RefCell<VecDeque<Queued>>,
    next_sequence: Cell<u64>,
    exited: Cell<bool>,
}

impl<W, B, T> ShellHandle<W, B, T>
where
    W: PlatformWindow,
    B: RenderBackend<TextHost<T>>,
    T: TextBackend,
{
    pub fn new(shell: Shell<W, B, T>) -> Self {
        Self {
            window: Arc::clone(shell.window()),
            published: shell.published(),
            shell: RefCell::new(shell),
            queue: RefCell::new(VecDeque::new()),
            next_sequence: Cell::new(0),
            exited: Cell::new(false),
        }
    }

    pub fn window(&self) -> &Arc<W> {
        &self.window
    }

    /// What the runtime has published about the window's surface. Readable from
    /// any native callback, including one that arrives inside [`Self::submit`].
    pub fn published(&self) -> PublishedHandle {
        self.published.clone()
    }

    /// Whether a step ended the run. Once it has, nothing further is handled.
    pub fn has_exited(&self) -> bool {
        self.exited.get()
    }

    /// Handles `event`, which happened at `at`.
    ///
    /// Returns what handling it meant when it ran during this call. An event
    /// that had to wait -- because it arrived inside another -- returns
    /// [`ShellOutcome::CONTINUE`], and the call it waited on handles it.
    pub fn submit(&self, at: Instant, event: ShellEvent) -> ShellOutcome {
        if self.exited.get() {
            return Self::exited_outcome();
        }

        let sequence = self.next_sequence.get();
        self.next_sequence.set(sequence + 1);
        self.enqueue(Queued {
            sequence,
            at,
            event,
        });

        // Already inside the shell: whoever holds the borrow drains the queue
        // before it lets go.
        let Ok(mut shell) = self.shell.try_borrow_mut() else {
            return ShellOutcome::CONTINUE;
        };

        let mut mine = None;
        loop {
            // The borrow ends before the event runs, so an event that queues
            // another while it is handled does not find the queue held.
            let next = self.queue.borrow_mut().pop_front();
            let Some(next) = next else {
                break;
            };
            let outcome = shell.dispatch_event(next.at, next.event);
            if outcome.control == ShellControl::Exit {
                self.exited.set(true);
                self.queue.borrow_mut().clear();
            }
            if next.sequence == sequence {
                mine = Some(outcome);
            }
        }
        mine.unwrap_or(ShellOutcome::CONTINUE)
    }

    /// Call when the event loop has drained its queue and is about to block.
    pub fn about_to_wait(&self) -> ShellControl {
        if self.exited.get() {
            return ShellControl::Exit;
        }
        let Ok(mut shell) = self.shell.try_borrow_mut() else {
            return ShellControl::Continue;
        };
        let control = shell.about_to_wait();
        if control == ShellControl::Exit {
            self.exited.set(true);
        }
        control
    }

    /// The error that made a frame fail, if one did.
    pub fn take_render_error(&self) -> Option<AppRenderError<B::Error>> {
        self.shell.try_borrow_mut().ok()?.take_render_error()
    }

    /// Runs `f` on the shell, for what the handle does not wrap. `None` if the
    /// shell is busy.
    pub fn with_shell<R>(&self, f: impl FnOnce(&mut Shell<W, B, T>) -> R) -> Option<R> {
        let mut shell = self.shell.try_borrow_mut().ok()?;
        Some(f(&mut shell))
    }

    fn exited_outcome() -> ShellOutcome {
        ShellOutcome {
            control: ShellControl::Exit,
            consumed: false,
        }
    }

    /// Queues `event`, folding it into the one before it when the two are
    /// successive samples of the same thing and only the latest matters.
    ///
    /// This only ever acts on a backlog: an idle handle runs each event the
    /// moment it arrives. A backlog forms when a nested loop or a long frame
    /// holds the shell, and what piled up behind it is a stale pointer position
    /// or window size for every one but the last.
    fn enqueue(&self, queued: Queued) {
        let mut queue = self.queue.borrow_mut();
        if let Some(back) = queue.back_mut() {
            let same_kind = matches!(
                (&back.event, &queued.event),
                (
                    ShellEvent::PointerMoved { .. },
                    ShellEvent::PointerMoved { .. }
                ) | (ShellEvent::Resized(_), ShellEvent::Resized(_))
            );
            if same_kind {
                // The earlier slot keeps its sequence number: if its caller is
                // the one that ends up draining, it is the one waiting for an
                // outcome.
                back.event = queued.event;
                back.at = queued.at;
                return;
            }
        }
        queue.push_back(queued);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
    };
    use xui_core::prelude::*;
    use xui_core::render::MockRenderBackend;
    use xui_f::FBackend;
    use xui_macros::component;
    use xui_interface::WindowRegion;

    use super::*;
    use crate::{PhysicalSize, ShellOptions, SurfaceTarget};

    type Hook = Box<dyn Fn() + Send>;

    /// A window whose cursor changes call back into whatever the test wants,
    /// standing in for a native callback that fires inside the shell.
    struct ReentrantWindow {
        on_set_cursor: Mutex<Option<Hook>>,
        size: Mutex<PhysicalSize>,
    }

    impl HasWindowHandle for ReentrantWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Err(HandleError::NotSupported)
        }
    }

    impl HasDisplayHandle for ReentrantWindow {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Err(HandleError::NotSupported)
        }
    }

    impl SurfaceTarget for ReentrantWindow {
        fn surface_size(&self) -> PhysicalSize {
            *self.size.lock().unwrap()
        }

        fn scale_factor(&self) -> f64 {
            1.0
        }
    }

    impl PlatformWindow for ReentrantWindow {
        fn request_redraw(&self) {}
        fn set_visible(&self, _visible: bool) {}
        fn focus(&self) {}
        fn is_minimized(&self) -> Option<bool> {
            Some(false)
        }
        fn set_cursor(&self, _cursor: CursorIcon) {
            if let Some(hook) = self.on_set_cursor.lock().unwrap().as_ref() {
                hook();
            }
        }
        fn set_ime_allowed(&self, _allowed: bool) {}
        fn set_ime_cursor_area(&self, _area: Rect) {}
    }

    type TestHandle = ShellHandle<ReentrantWindow, MockRenderBackend, FBackend>;

    thread_local! {
        static HANDLE: RefCell<Option<Rc<TestHandle>>> = const { RefCell::new(None) };
        static MOVES: RefCell<Vec<(f32, f32)>> = const { RefCell::new(Vec::new()) };
    }

    #[component]
    fn recording() {
        view! {
            container(
                width: 200.0,
                height: 200.0,
                cursor: CursorIcon::Pointer,
                on_pointer_move: |_cx, event| {
                    MOVES.with(|moves| {
                        moves
                            .borrow_mut()
                            .push((event.pointer.coords.window.x, event.pointer.coords.window.y));
                    });
                },
            )
        }
    }

    fn recording_root(app: &mut App) {
        app.mount::<Recording>(RecordingProps {});
    }

    fn start() -> (Rc<TestHandle>, Arc<ReentrantWindow>) {
        MOVES.with(|moves| moves.borrow_mut().clear());
        let window = Arc::new(ReentrantWindow {
            on_set_cursor: Mutex::new(None),
            size: Mutex::new(PhysicalSize::new(400, 400)),
        });
        let (shell, _) = Shell::new(
            window.clone(),
            App::new(recording_root),
            FBackend::new(),
            MockRenderBackend::default(),
            ShellOptions::default(),
        );
        let handle = Rc::new(ShellHandle::new(shell));
        HANDLE.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&handle)));
        (handle, window)
    }

    fn moved(x: f32, y: f32) -> ShellEvent {
        ShellEvent::PointerMoved {
            position: Point::new(x, y),
            device_id: None,
        }
    }

    fn moves() -> Vec<(f32, f32)> {
        MOVES.with(|moves| moves.borrow().clone())
    }

    #[test]
    fn an_idle_handle_runs_each_event_at_once_and_reports_its_outcome() {
        let (handle, _window) = start();
        let outcome = handle.submit(Instant::now(), moved(10.0, 10.0));
        assert_eq!(outcome.control, ShellControl::Continue);
        assert_eq!(moves(), vec![(10.0, 10.0)]);

        let outcome = handle.submit(Instant::now(), ShellEvent::CloseRequested);
        assert_eq!(outcome.control, ShellControl::Exit);
        assert!(handle.has_exited());
    }

    #[test]
    fn an_event_that_arrives_inside_the_shell_is_queued_and_run_before_submit_returns() {
        let (handle, window) = start();
        // Moving onto the container changes the cursor, which calls back into
        // the handle from inside the shell, the way a native callback does.
        let fired = Arc::new(Mutex::new(0));
        {
            let fired = Arc::clone(&fired);
            *window.on_set_cursor.lock().unwrap() = Some(Box::new(move || {
                let mut fired = fired.lock().unwrap();
                *fired += 1;
                if *fired == 1 {
                    HANDLE.with(|slot| {
                        let handle = slot.borrow().clone().unwrap();
                        let outcome = handle.submit(Instant::now(), moved(50.0, 60.0));
                        // It could not run yet: the shell is busy with the move
                        // that caused this.
                        assert_eq!(outcome, ShellOutcome::CONTINUE);
                    });
                }
            }));
        }

        let _ = handle.submit(Instant::now(), moved(10.0, 10.0));

        assert_eq!(*fired.lock().unwrap(), 1);
        assert_eq!(
            moves(),
            vec![(10.0, 10.0), (50.0, 60.0)],
            "the nested event ran after the one it arrived inside, in order"
        );
    }

    #[test]
    fn a_backlog_of_pointer_moves_keeps_only_the_latest() {
        let (handle, window) = start();
        *window.on_set_cursor.lock().unwrap() = Some(Box::new(|| {
            HANDLE.with(|slot| {
                let handle = slot.borrow().clone().unwrap();
                for step in 1..=4 {
                    let _ = handle.submit(Instant::now(), moved(20.0 + step as f32, 20.0));
                }
            });
        }));

        let _ = handle.submit(Instant::now(), moved(10.0, 10.0));

        assert_eq!(
            moves(),
            vec![(10.0, 10.0), (24.0, 20.0)],
            "four queued moves folded into the last"
        );
    }

    #[test]
    fn nothing_runs_after_the_run_has_ended() {
        let (handle, _window) = start();
        let _ = handle.submit(Instant::now(), ShellEvent::CloseRequested);
        let outcome = handle.submit(Instant::now(), moved(10.0, 10.0));
        assert_eq!(outcome.control, ShellControl::Exit);
        assert!(moves().is_empty());
        assert_eq!(handle.about_to_wait(), ShellControl::Exit);
    }

    #[test]
    fn the_published_handle_is_readable_while_the_shell_is_busy() {
        let (handle, window) = start();
        let seen = Arc::new(Mutex::new(None));
        {
            let seen = Arc::clone(&seen);
            *window.on_set_cursor.lock().unwrap() = Some(Box::new(move || {
                HANDLE.with(|slot| {
                    let handle = slot.borrow().clone().unwrap();
                    // The shell is borrowed right now; the published state is not.
                    *seen.lock().unwrap() =
                        Some(handle.published().region_at(Point::new(1.0, 1.0)));
                });
            }));
        }
        let _ = handle.submit(Instant::now(), moved(10.0, 10.0));
        assert_eq!(*seen.lock().unwrap(), Some(WindowRegion::Content));
    }
}
