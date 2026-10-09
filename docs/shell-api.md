# xui-shell interface revision

Status: **phases 0 and 1 are implemented, and the text input uses them.** Phase 0
is the vocabulary and the runtime and shell plumbing; phase 1 puts the macOS host
on it; the text input now copies, cuts and pastes through it. `xui-winit` and
`xui-windows` do not use it yet; see [Rollout](#rollout).

## Why

`xui-shell` used to carry three things between a host and the runtime:

- **events in**: `ShellEvent` (resize, pointer, wheel, key, IME, focus, redraw, wake);
- **state out**: `PlatformOutput` (cursor, IME session), diffed by `Shell` and
  applied through `PlatformWindow::set_cursor` / `set_ime_*`;
- **window creation**: `WindowOptions`.

Everything else a native app needs did not fit those channels, so it was absent
on every host: clipboard, window title after creation, fullscreen, dialogs,
menus, drag and drop, appearance, pointer-leave, context menu, touch and
gestures. `RawEvent::ContextMenu` and `PointerCancel` were defined but no host
could send them.

## Four channels, not one

| Channel | Direction | Shape | Examples |
|---|---|---|---|
| Events | host → runtime | `ShellEvent` | pointer left, touch, gesture, menu command, appearance changed |
| State | runtime → host | `PlatformOutput`, diffed **and readable at any time** | cursor, IME, title, menu bar, window regions, drop targets, text document |
| Commands | runtime → host | `PlatformCommand`, drained once | write clipboard, fullscreen, open URL, begin window move |
| Requests | runtime → host → runtime | `PlatformCommand::Request` + `PlatformReply` | read clipboard, file dialog, message box |

State and commands are different things. A title is something the runtime
*wants to be true*, so it is diffed and applied idempotently. "Write this to the
clipboard" is an action; replaying it every frame would be wrong.

Requests are **async-shaped but sync-capable**. Wayland and X11 clipboards are
asynchronous; AppKit and Win32 are not. The host answers inline when it can
(`Performed::Reply`) and later through `ShellEvent::Reply` when it cannot
(`Performed::Pending`). One shape, no per-platform special cases in the runtime.

## Where things live

`xui-shell` depends on `xui-core`, so anything the runtime produces or consumes
sits below it:

| Crate | What |
|---|---|
| `xui-interface::platform` | `PlatformCommand`, `WindowCommand`, `PlatformRequest`, `PlatformReply`, `PlatformError`, `RequestId`, `HostCapabilities`, `Appearance`, `WindowState`, `WindowGeometry`, `MenuBar` and friends, `commands::*`, `PublishedSurface`, `DragEvent`, `DropEffect`. `PlatformOutput` gains `title`, `menu_bar`, `surface` |
| `xui-interface::events` | `RawEvent::PointerLeave`, `RawEvent::Gesture`, `GestureKind` |
| `xui-core::platform` | `PlatformHandle`, the application's end of the contract; `App::platform()` |
| `xui-core::runtime` | new `RuntimeEvent` variants; `GuiRuntime::take_platform_commands()` |
| `xui-shell` | the new `ShellEvent` variants, `ShellOutcome`, `Performed`, `FrameInfo`, `PublishedHandle`, `ShellHandle`, and the defaulted `PlatformWindow` methods |

## The host contract

Every addition to `PlatformWindow` has a default, so `xui-macos`, `xui-windows`,
`xui-winit` and any test window keep compiling unchanged and opt in one
capability at a time.

```rust
fn capabilities(&self) -> HostCapabilities { HostCapabilities::NONE }
fn appearance(&self)   -> Appearance       { Appearance::default() }
fn set_title(&self, title: &str) {}
fn set_menu_bar(&self, menu_bar: Option<&MenuBar>) {}   // None: the host's default
fn geometry(&self) -> Option<WindowGeometry> { None }
fn perform(&self, command: PlatformCommand) -> Performed { Performed::Unsupported }
fn request_redraw_at(&self, at: Instant) {}             // with HostCapabilities::TIMED_REDRAW
```

`perform` is the single entry point for commands and requests. Adding a command
later changes an enum, not this trait, so a host that does not know the new
variant answers `Unsupported` instead of failing to build.

`Performed::Unsupported` is never silent for requests: `Shell` turns it into
`PlatformReply::Unsupported` and delivers it, so code waiting on a file dialog
cannot hang on a host that has none. A request a host completes with `Done`
instead of a reply is a host bug; it is logged and the caller gets `Failed`.

A host that shows something outliving the call (a sheet, a portal dialog) must
answer `Pending` rather than block in `perform`: blocking runs the platform's
nested event loop, which delivers events while the shell is busy.

`HostCapabilities` is reported once, before the first frame, and readable from
`PlatformHandle::capabilities()`, so components can hide or degrade UI the host
cannot back.

### Existing machinery that is reused

- **Menus and standard actions use `CommandId`.** A `MenuItem` names a
  `CommandId` and shows a `Shortcut`; choosing it raises `ShellEvent::Command`,
  which the runtime dispatches the way a shortcut's command is. The system's own
  standard actions (AppKit's `copy:`, `paste:`, …) map to the constants in
  `xui_interface::commands`. There is no `EditCommand` or `MenuItemId` type.
  `CommandEvent::shortcut` is now `Option<Shortcut>` (`None` when the platform
  raised it) and `EventSource` gained `Platform`.
- **Touch uses `PointerKind::Touch`.** A finger is a pointer of its own: id 0 is
  the mouse, finger *n* is pointer *n + 1*, so a widget written for the pointer
  works under a finger. A landing is a move followed by a press.
- **Context menu and pointer cancel** use the existing `RawContextMenu` and
  `RawPointerCancel`. The gap was the host-side route, not the vocabulary.

## Event routing

| `ShellEvent` | Becomes |
|---|---|
| `PointerLeft` | `RawEvent::PointerLeave`; clears hover (ignored for a pointer that still holds a press or drag, which keeps its capture) |
| `PointerCancelled` | `RawEvent::PointerCancel`; also clears the shell's button state |
| `ContextMenuRequested { position, trigger }` | `RawEvent::ContextMenu` |
| `Touch { id, phase, position, .. }` | `RawPointerMove` / `Down` / `Up` / `Cancel` with `PointerKind::Touch` |
| `Gesture { kind, phase, position }` | `RawEvent::Gesture` |
| `Command(CommandId)` | `RuntimeEvent::Command`, dispatched to the focused node (or the command root) and bubbling |
| `Reply(RequestId, PlatformReply)` | `RuntimeEvent::Reply`, resolves the pending request |
| `AppearanceChanged(Appearance)` | `RuntimeEvent::Appearance` |
| `WindowStateChanged(WindowState)` | `RuntimeEvent::WindowState` |
| `Drag(DragEvent)` | `RuntimeEvent::Drag` |
| `LiveResize(bool)` | shell state; see below |
| `FrameTick(FrameInfo)` | a frame at the host's target time |

Drag and drop is delivered as events, but the *answer* the OS wants during
`draggingUpdated:` / `IDropTarget::DragOver` comes from published drop targets
(`PublishedHandle::drop_effect_at`), not from a call into the runtime.

`ShellEvent` is now `#[non_exhaustive]`: a `match` outside `xui-shell` needs a
wildcard arm. Only `Shell` matches it today.

## The channel contract

The capabilities above are only half of it. The event channel itself was
under-specified, in six ways already visible in the code:

| # | Problem | Evidence (before) |
|---|---|---|
| 1 | Events were fire-and-forget; the host never learned whether the runtime consumed one | `xui-macos` `handle_key_down` guesses "a Command key is the runtime's" with a hardcoded `!meta`; native menu key equivalents and the `DefWindowProc` fallback both need the real answer |
| 2 | A host needing an answer *during* a native callback had no channel, and calling into `Shell` is unsafe because it is exclusively borrowed while handling | `selectedRange` can only return "not found"; `WM_NCHITTEST`, `draggingUpdated:` and menu validation have the same shape |
| 3 | Platform-independent policy was duplicated per host | the re-entrancy `VecDeque` + `try_borrow_mut` is identical in `xui-macos/src/host.rs` and `xui-windows/src/host.rs`; "redraw synchronously after a resize" is hand-written in both and missing from `xui-winit` |
| 4 | The display's frame timing was discarded | `GuiRuntime::frame_at(now)` exists *for* caller-supplied instants, but `RedrawRequested` carries nothing and the shell called `frame()` |
| 5 | Events were stamped when handled, not when they happened | an event that waited in the re-entrancy queue got the wrong time, skewing double-click spacing and scroll velocity |
| 6 | Requests had no lifecycle | no cancel, no failure reason |

### 1. Events return a disposition

```rust
Shell::dispatch_event(&mut self, at: Instant, event: ShellEvent) -> ShellOutcome
struct ShellOutcome { control: ShellControl, consumed: bool }
```

`consumed` comes from the runtime's `EventResult`s. A host uses it to decide
whether the native layer still gets the event: `performKeyEquivalent:` returns
it, `DefWindowProc` runs only when it is false, an unconsumed wheel goes to an
enclosing view. `handle_event(event) -> ShellControl` stays as a wrapper that
stamps `Instant::now()`, so no host had to change.

### 2. State is pushed *and* pullable

`PlatformOutput` carries state out, diffed. It is also readable by the host at
any moment through `Shell::published() -> PublishedHandle`, updated after every
runtime step, and now carries three published pieces in `PublishedSurface`:

- **window regions** (`Content`, `Caption`, `Resize(edge)`) — answers
  `WM_NCHITTEST` natively, giving Windows snap layouts and double-click-to-zoom
  and macOS a title-bar drag region, which a `BeginMove` command cannot;
- **drop targets** (rect, accepted kinds, effect) — answers `draggingUpdated:` /
  `DragOver` on the spot;
- **text document** (text, selection, marked range, UTF-16 offsets) — what the
  system's text services query.

A native callback can arrive while `Shell` is mid-event, so it has to be
answerable from published state rather than by re-entering the runtime.
`PublishedSurface::region_at` and `drop_effect_at` are pure functions.

`BeginMove` / `BeginResize` stay as commands for the rarer "start a move from
code" case.

### 3. Policy moves into the shell

- **`ShellHandle`** wraps a `Shell` and takes events through `submit(&self, at,
  event)`. Called from inside a `PlatformWindow` method the shell is itself
  running, it queues the event and returns; the call already running drains it.
  After `CloseRequested` nothing further runs. A backlog of `PointerMoved` or
  `Resized` folds into the latest, since only that one is current; an idle
  handle runs everything immediately, so this only acts when something held the
  shell. Hosts delete their own queues.
- **`LiveResize(bool)`**: while true, a `Resized` is painted on the spot rather
  than on the next requested frame. The macOS `redraw_now` and the Windows
  `WM_SIZE` redraw become one rule, and `xui-winit` gets it too.
  `SurfaceTarget::in_live_resize()` stays, because the Metal presenter needs it
  at present time.

### 4. Frame info rides the redraw

`FrameTick(FrameInfo { target, interval })`: the shell passes `target` to
`frame_at`, so animation is paced to when the frame will be *shown*, not when it
was built. On macOS the display link supplies `targetTimestamp`. `RedrawRequested`
remains for hosts without one.

### 4a. Deadlines get a timed frame, not the display link

Something that changes rarely -- a caret blinking twice a second -- must not keep
the loop at the display's rate. A node asks with
`EventContext::request_wake_at(Instant)` (one per node, replacing the last);
`GuiRuntime::next_wake()` is the earliest of them (`None` off screen); and when a
settle finds nothing dirty, `Shell` hands that instant to
`PlatformWindow::request_redraw_at`, once per distinct deadline. The frame at or
after it delivers `SemanticEvent::Wake` to the node before anything is stepped,
so the change is painted in that frame. The deadline is on the input clock, not
the frame clock, which stands still across exactly the idle gap a wake ends.

`xui-macos` schedules a main-queue block for the instant; `xui-winit` waits with
`ControlFlow::WaitUntil`. Both report `HostCapabilities::TIMED_REDRAW`. A host
without it gets no timed frames, and UI that would look broken arriving late
checks the capability first: the text input then keeps a steady caret.

### 5. Timestamps are the host's

`dispatch_event` and `submit` take `at: Instant`: the native timestamp
(`NSEvent.timestamp`, `GetMessageTime`), or `Instant::now()` for a host with none.

### 6. Requests have a lifecycle

`PlatformHandle::cancel(id)` drops the callback and tells the host
(`PlatformCommand::Cancel`); a late reply is dropped. `PlatformReply::Failed` and
`Performed::Failed` carry a `PlatformError`, and the shell logs every failure,
including for commands with no reply. **Replies are always delivered through the
runtime's event route, never from inside `perform`**, even when the host answers
inline: the runtime is mid-handover of its command list at that point and must
not be re-entered. A reply handler may make another request; the shell serves up
to eight rounds per settle and then leaves the rest for the next event, so a loop
that never settles cannot spin.

## The application's side

`App::platform()` returns a `PlatformHandle`: a cheap clone over shared state,
`!Send`, safe to capture in an event callback.

```rust
let platform = app.platform().clone();
platform.set_title("Report");                       // state, diffed
platform.set_menu_bar(Some(menu_bar));
platform.send(PlatformCommand::OpenUrl(url));       // one-shot
let id = platform.request(                          // answers later
    PlatformRequest::ReadClipboard(ClipboardTarget::General),
    |reply| { /* PlatformReply::Clipboard(..) | Unsupported | Failed(..) */ },
);
platform.cancel(id);
platform.capabilities(); platform.appearance(); platform.window_state();
```

The callback of a request runs at most once, from the event route, never inside
`request`. There is no hook for it yet; components reach the handle by capturing
a clone.

## Widgets and the host

A widget handles events with an `EventContext`, which now reaches the host:

```rust
cx.send_platform(PlatformCommand::WriteClipboard { .. });   // one-shot
let id = cx.request_platform(PlatformRequest::ReadClipboard(ClipboardTarget::General));
```

Both are `None`/`false` on a context with no platform handle, as one built by hand
in a test has. The dispatcher always sets it, from the `PlatformHandle` the
application shares with the runtime (`UiRuntime::platform`).

A request made this way is **addressed to the node**. `PlatformHandle::request`
takes a callback, which is `'static` and cannot reach a widget; a widget needs the
answer with its own context, so that it can edit itself and mark itself for
layout. `request_platform` files the request under the node, and when the host's
answer comes back the runtime delivers it to that node alone as
`SemanticEvent::PlatformReply` (`on_platform_reply`), with the id the request
returned. It does not propagate: an ancestor handling it too would act twice. An
answer for a node that is gone by then is dropped, and so is one for an id nothing
registered.

### The text input

`TextInputWidget` handles the six standard edit commands, from either route:

- **From the keyboard.** Its keymap (`Cmd` on macOS, `Ctrl` elsewhere, with
  `Shift` for redo) was already resolving them; copy, cut and paste had been
  stubs that consumed the key and did nothing.
- **As commands.** `SemanticEvent::Command` for `commands::COPY`, `CUT`, `PASTE`,
  `SELECT_ALL`, `UNDO`, `REDO`, which is what a menu item or the system's own Copy
  raises. Both routes run the same code.

Behaviours worth knowing:

- Copy of an empty selection does nothing and leaves the clipboard alone; cut
  copies, then deletes. Selections are by character, so multibyte text is safe.
- Paste asks the host and waits for the reply as an event. A newer paste
  supersedes an older one still waiting. Over a selection it replaces it, and the
  caret ends after the pasted text. A reply of nothing, `Unsupported` or a failure
  pastes nothing.
- It is a single-line field, so a paste has its line breaks at either end dropped
  (a line copied whole carries its trailing break) and each run inside, and each
  tab, turned into one space.
- Both commands return `Consumed` when handled. That is what lets a host skip its
  own handling of the key: on macOS `performKeyEquivalent:` offers the key to the
  runtime first, so the text input answers Cmd+C itself and the Edit menu's Copy is
  only the fallback when nothing is focused that wants it.
- A host with no clipboard needs nothing special: the paste request is answered
  `Unsupported` and ignored.
- The caret blinks every 500ms while the field is focused and left alone, and is
  held shown for a full period after anything the field acts on (typing, moving
  the caret, focus). Each half-period is a wake (see 4a), so an idle focused field
  costs two frames a second. Without `TIMED_REDRAW` the caret does not blink.

`TextSelection` is now exported (`xui_core::widgets`, and the prelude).
`TextController::set_selection` took one and nothing outside `xui-core` could name
the type, so an application could read a selection but not set one.

Not covered: a password mode (there is none, so copy is allowed everywhere),
multi-line input, drag and drop of text, and rich text on the clipboard.

## What phase 0 did not do

Deliberately, so the first step stays additive and testable without a native
window:

- **No host implements any of it.** Every default is "unsupported".
- **Few widgets use it.** Only the text input does, for the edit commands (see
  above). Nothing declares a window region or a drop target yet (a style flag is
  the obvious spelling for a caption region), and `RuntimeEvent::Drag` and
  `RawEvent::Gesture` are accepted and go nowhere.
- **`Appearance`, capabilities and window state are stored, not reactive.** A
  component reading them is not re-rendered when they change, and `Appearance`
  does not yet feed the theme tokens.
- **`WindowOptions` is unchanged.** Max size, position, always-on-top,
  fullscreen and icon wait for hosts that honour them; an option that silently
  does nothing is worse than none.
- **`ShellEvent` is not grouped into families.** Doing it breaks every host's
  event construction; it belongs in the first host-touching phase.
- **Touch carries no pressure,** and the old `RedrawRequested` / `Resized`
  variants are untouched.
- **No component-facing hook** for the platform handle.

## Rollout

0. **Types and plumbing, no host changes.** *Done.*
1. **macOS: the daily-use gaps.** *Done* — see below.
2. **macOS: appearance, dialogs, drag and drop, native context menu.**
3. Touch, gestures, notifications.
4. Windows, then `xui-winit` forwarding the winit events it currently drops
   (`CursorLeft`, `Touch`, `DroppedFile`, `ThemeChanged`).

### What phase 1 changed in `xui-macos`

- **On `ShellHandle`.** The host's own re-entrancy queue and `redraw_now` are
  gone. A small queue remains only for events that arrive before the shell
  exists, because starting the shell shows the window and AppKit reports that
  synchronously.
- **Native timestamps.** Pointer, wheel and key events carry `NSEvent.timestamp`,
  converted to an `Instant` against `NSProcessInfo.systemUptime`.
- **`FrameTick`.** The display link reports `targetTimestamp` and the refresh
  interval. On an idle machine the spacing of frames the application sees is
  16.67 ms with a standard deviation of about 0.001 ms, against about 0.026 ms
  when the shell used `Instant::now()`: real, but 25 microseconds, so the benefit
  is expected under load, which was not measured.
- **Resizes paint inside `windowDidResize:`.** Every resize is wrapped in
  `LiveResize(true)` / `(false)` unless AppKit says it is already a live drag,
  and `MacWindow::in_live_resize()` is true for the length of the callback, so
  the Metal presenter's synchronized present also covers a zoom or a fullscreen
  transition, not only a drag. A resize the *runtime* asks for through
  `SetInnerSize` happens inside the shell, so its callback is queued and handled
  afterwards, outside the flag; that is correct, since it is not arriving from
  the window system.
- **Menus.** A host-owned application menu (About, Hide, Quit) is always first.
  An application's `MenuBar` follows it, or, if it supplied none, an Edit and a
  Window menu. Standard items (`commands::COPY` …, `MenuRole`) have no target, so
  AppKit's responder chain answers them: `XuiView` implements `copy:`, `cut:`,
  `paste:`, `selectAll:`, `undo:`, `redo:` by raising the command. Other items
  target a `MenuTarget` that raises the command by tag. `enabled` and `checked`
  are honoured, so auto-enabling is off.
- **Key equivalents go to the runtime first.** `performKeyEquivalent:` offers a
  Command or Control key to the runtime before the menu bar sees it, and uses
  `ShellOutcome::consumed` as its answer, so a shortcut the application has bound
  wins over a menu item showing the same keys, and the menu is the fallback. A
  key offered and refused is remembered so `keyDown:` does not deliver it again.
  Not offered: anything while an input method is composing, and a Control-only
  chord while input is allowed, because input methods use those themselves.
- **Pointer and context menu.** An `NSTrackingArea` gives `PointerLeft`
  (ignored while a button is down). A right click raises the button event and
  then `ContextMenuRequested`; a control-click is a right click, press and
  release. Buttons are read from the event type, not `buttonNumber`.
- **Services.** `perform` implements clipboard write and read (general
  pasteboard; the primary selection answers `Unsupported`), `OpenUrl`,
  `RevealPath`, and the window commands `SetFullscreen`, `SetMaximized`,
  `Minimize`, `SetInnerSize`, `SetSizeLimits`, `SetAlwaysOnTop`, `BeginMove`.
  `WindowStateChanged` is sent when the user fullscreens or zooms the window.
- **Title and caption.** `set_title`. A press on a published `Caption` region
  moves the window (a double-click zooms it) and never reaches the runtime.

Verified by `cargo run -p xui-macos --example smoke`, which drives a real window
with synthesized `NSEvent`s and asserts 27 things (the first is that the window
is visible: with the screen locked or asleep AppKit runs no display link and
sends no key events, so it stops there rather than report nine failures that
are not the host's), and by 19 unit tests of the
parts that do not need AppKit (menu mapping, key equivalents, time conversion).

### Not done in phase 1

- **`ShellEvent` is still flat.** Grouping it would change how `xui-winit` and
  `xui-windows` build events, and `xui-windows` cannot be compiled on macOS, so
  it waits for a phase that touches those hosts.
- **`WindowOptions` is unchanged**, and `PlatformWindow::geometry` and
  `appearance` are unimplemented: nothing yet restores a window's frame or
  follows the system's dark mode.
- **`BeginResize` and `ShowContextMenu` are `Unsupported`.** AppKit has no
  programmatic edge resize; a native context menu is phase 2.
- **Fullscreen, `OpenUrl`, `RevealPath` and `BeginMove` are implemented but not
  exercised by the smoke example**, because they are visible, launch other
  applications, or run a modal drag loop.
- **A caption region is honoured but nothing declares one.**
- **`Cmd+Q` and friends now reach the runtime as keys before the menu takes
  them.** Harmless, but a change.

## Out of scope: accessibility

Reserved but not specified. The per-widget `AccessibilityRole` /
`AccessibilityProperties` exist, but there is **no `AccessibilityTree` type**
anywhere in the workspace, so `xui-winit/src/accessibility.rs` (which imports
one, and is not declared as a module) could not compile if it were wired in. The
tree has to be designed in the runtime first. The shell side will be one state
channel (`PlatformOutput.accessibility`, `Arc`-wrapped and generation-compared),
one event in (`AccessibilityAction`), and an "assistive technology is attached"
signal so the runtime builds the tree only when someone is listening.

## Decisions taken, and what is still open

Taken, following the recommendations in the proposal:

1. **Standard actions go through `CommandId`.** On macOS with a menu bar, Cmd+C
   is consumed by the menu and arrives as `Command(COPY)`; on Windows and Linux
   it arrives as a key and the runtime's shortcut bindings produce the same
   command.
2. **A paste costs an extra event turn,** because a clipboard read is a request.
3. **Dialogs are non-blocking** (`Performed::Pending`), not `runModal`.
4. **`menu_bar: None` means the host's default**; an app replaces it wholesale.
5. **The re-entrant queue is a wrapper (`ShellHandle`),** not a change to
   `Shell`, which stays `&mut self`. Hosts migrate when they are next touched.

Still open:

- **Per-menu extension.** Is "replace the whole bar" enough, or must an app be
  able to add to the host's default menus?
- **Caption regions in the widget layer.** How an application declares one.
- **Multi-window.** Kept out: one `Shell` per window, one window per run. When it
  arrives, `perform` needs to know which window; adding that is mechanical.
