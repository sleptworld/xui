//! The window procedure: `WM_*` in, [`ShellEvent`] out.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, HTCLIENT, MINMAXINFO, MSG, PM_NOREMOVE, PeekMessageW, PostQuitMessage,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_DPICHANGED,
    WM_GETMINMAXINFO, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SETCURSOR, WM_SETFOCUS, WM_SIZE, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN,
    WM_XBUTTONUP,
};
use xui_interface::events::{KeyState, KeyText};
use xui_interface::{PointerButton, ScrollDelta, Translation};
use xui_shell::{KeyInput, ShellEvent};

use crate::host::{WM_XUI_WAKE, send_current};
use crate::window::{STATE, client_size, mouse_position, scale_of, screen_to_client, wheel_delta};
use crate::{cursor, ime, keycode};

/// `WM_SIZE`'s `wparam` when the window was minimized.
const SIZE_MINIMIZED: usize = 1;

/// SAFETY: registered as a window class's procedure; Windows calls it with the
/// arguments its signature declares, on the thread that owns the window.
pub(crate) unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLOSE => {
            // The shell decides. When it exits, the loop ends and the window is
            // destroyed with it.
            send_current(ShellEvent::CloseRequested);
            LRESULT(0)
        }
        WM_DESTROY => {
            // Something outside this crate destroyed the window; without a
            // quit the loop would spin on a window that no longer exists.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_PAINT => {
            // Validated rather than painted through GDI: the render backend
            // owns these pixels, and leaving the region invalid would have
            // Windows ask for it again immediately, forever.
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            STATE.with(|state| state.redraw_pending.set(false));
            send_current(ShellEvent::RedrawRequested);
            LRESULT(0)
        }
        WM_SIZE => {
            let size = client_size(hwnd);
            send_current(ShellEvent::Resized(size));
            if wparam.0 != SIZE_MINIMIZED {
                // Drawn inside the message rather than on the next turn of the
                // loop: a resize runs a modal loop of its own, and a frame that
                // arrives after it shows the window stretched in the meantime.
                STATE.with(|state| state.redraw_pending.set(false));
                send_current(ShellEvent::RedrawRequested);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            // Windows suggests where the window should go to keep its apparent
            // size across the change; not taking it leaves the window the wrong
            // size on the new monitor.
            let suggested = lparam.0 as *const RECT;
            if !suggested.is_null() {
                let rect = unsafe { *suggested };
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            send_current(ShellEvent::ScaleFactorChanged(
                f64::from(dpi) / crate::window::BASE_DPI,
            ));
            send_current(ShellEvent::Resized(client_size(hwnd)));
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let min = STATE.with(|state| state.min_size.get());
            if let Some((width, height)) = min {
                let info = lparam.0 as *mut MINMAXINFO;
                if !info.is_null() {
                    let scale = scale_of(hwnd);
                    unsafe {
                        (*info).ptMinTrackSize.x = (width * scale).round() as i32;
                        (*info).ptMinTrackSize.y = (height * scale).round() as i32;
                    }
                }
            }
            LRESULT(0)
        }
        WM_SETFOCUS => {
            send_current(ShellEvent::Focused(true));
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            send_current(ShellEvent::Focused(false));
            LRESULT(0)
        }
        WM_SETCURSOR => {
            // Only the client area is ours; the frame's own cursors -- the
            // resize arrows on the border -- are Windows' business.
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                cursor::set_current();
                return LRESULT(1);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_MOUSEMOVE => {
            pointer_moved(hwnd, lparam);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => button(hwnd, lparam, PointerButton::Primary, true),
        WM_LBUTTONUP => button(hwnd, lparam, PointerButton::Primary, false),
        WM_RBUTTONDOWN => button(hwnd, lparam, PointerButton::Secondary, true),
        WM_RBUTTONUP => button(hwnd, lparam, PointerButton::Secondary, false),
        WM_MBUTTONDOWN => button(hwnd, lparam, PointerButton::Auxiliary, true),
        WM_MBUTTONUP => button(hwnd, lparam, PointerButton::Auxiliary, false),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let pressed = msg == WM_XBUTTONDOWN;
            let which = ((wparam.0 >> 16) & 0xFFFF) as u16;
            let which = if which == 1 {
                PointerButton::Back
            } else {
                PointerButton::Forward
            };
            button(hwnd, lparam, which, pressed);
            // The X buttons are the one pair whose handler must report back.
            LRESULT(1)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let scale = scale_of(hwnd);
            let position = screen_to_client(hwnd, lparam, scale);
            track_position(position);
            let notches = wheel_delta(wparam);
            let delta = if msg == WM_MOUSEWHEEL {
                Translation::new(0.0, notches)
            } else {
                // Horizontal wheels report the direction the content should
                // move, which is the opposite sign of the vertical one.
                Translation::new(-notches, 0.0)
            };
            send_current(ShellEvent::Wheel {
                delta: ScrollDelta::Lines(delta),
                device_id: None,
                // Win32 wheel messages carry no phase; a precision touchpad's
                // fling arrives as ordinary wheel messages.
                is_inertial: false,
            });
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            key(hwnd, wparam, lparam, KeyState::Down);
            // System keys still reach `DefWindowProc`, which is what keeps
            // Alt+F4 and the window menu working.
            if msg == WM_SYSKEYDOWN {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }
            LRESULT(0)
        }
        WM_KEYUP | WM_SYSKEYUP => {
            key(hwnd, wparam, lparam, KeyState::Up);
            if msg == WM_SYSKEYUP {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }
            LRESULT(0)
        }
        WM_CHAR | WM_SYSCHAR => {
            // The character was already attached to the key event that
            // produced it; see `key`.
            LRESULT(0)
        }
        WM_IME_STARTCOMPOSITION => {
            ime::move_candidate_window(hwnd);
            // Answered here so IMM32 does not also draw a composition window of
            // its own over the one the runtime is drawing.
            LRESULT(0)
        }
        WM_IME_COMPOSITION => {
            ime::composition(hwnd, lparam.0 as u32);
            LRESULT(0)
        }
        WM_IME_ENDCOMPOSITION => {
            ime::end_composition();
            LRESULT(0)
        }
        WM_XUI_WAKE => {
            send_current(ShellEvent::Wake);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The shell reports buttons and wheels at the last moved-to position, and a
/// message is not always preceded by a move to where it happened.
fn track_position(position: xui_interface::Point) {
    let changed = STATE.with(|state| state.last_position.replace(Some(position))) != Some(position);
    if changed {
        send_current(ShellEvent::PointerMoved {
            position,
            device_id: None,
        });
    }
}

fn pointer_moved(hwnd: HWND, lparam: LPARAM) {
    sync_modifiers();
    let position = mouse_position(lparam, scale_of(hwnd));
    STATE.with(|state| state.last_position.set(Some(position)));
    send_current(ShellEvent::PointerMoved {
        position,
        device_id: None,
    });
}

fn button(hwnd: HWND, lparam: LPARAM, button: PointerButton, pressed: bool) -> LRESULT {
    sync_modifiers();
    track_position(mouse_position(lparam, scale_of(hwnd)));

    // Captured while anything is held, so a drag that leaves the window keeps
    // reporting; released when the last button comes up, so the window stops
    // swallowing the rest of the desktop's input.
    let bit = 1u32 << button_index(button);
    let held = STATE.with(|state| {
        let held = if pressed {
            state.buttons_down.get() | bit
        } else {
            state.buttons_down.get() & !bit
        };
        state.buttons_down.set(held);
        held
    });
    // SAFETY: a live window handle, on its own thread.
    unsafe {
        if pressed {
            SetCapture(hwnd);
        } else if held == 0 {
            let _ = ReleaseCapture();
        }
    }

    send_current(ShellEvent::PointerButton {
        button,
        pressed,
        device_id: None,
    });
    LRESULT(0)
}

fn button_index(button: PointerButton) -> u32 {
    match button {
        PointerButton::Primary => 0,
        PointerButton::Secondary => 1,
        PointerButton::Auxiliary => 2,
        PointerButton::Back => 3,
        PointerButton::Forward => 4,
        PointerButton::Other(other) => 5 + u32::from(other % 8),
    }
}

fn key(hwnd: HWND, wparam: WPARAM, lparam: LPARAM, state: KeyState) {
    sync_modifiers();
    // A key the input method is using belongs to the composition, not to the
    // runtime -- the same rule the macOS host applies.
    if ime::is_composing() {
        return;
    }
    let vk = wparam.0 as u16;
    let scancode = ((lparam.0 >> 16) & 0xFF) as u16;
    let extended = (lparam.0 >> 24) & 1 != 0;
    let is_repeat = state == KeyState::Down && (lparam.0 >> 30) & 1 != 0;
    let text = if state == KeyState::Down {
        pending_char(hwnd)
    } else {
        None
    };

    send_current(ShellEvent::Keyboard(KeyInput {
        physical_key: keycode::physical_key(scancode, extended),
        named_key: keycode::named_key(vk),
        state,
        text: text.as_deref().and_then(KeyText::try_new),
        is_repeat,
    }));
}

/// The character this key will produce, if any.
///
/// `TranslateMessage` posts `WM_CHAR` before the key message is dispatched, so
/// by the time this runs the character is already in the queue and can be read
/// without removing it. That keeps the layout's own translation -- dead keys,
/// AltGr, everything -- instead of reimplementing it over `ToUnicode`.
fn pending_char(hwnd: HWND) -> Option<String> {
    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid out-parameter; `PM_NOREMOVE` leaves the queue
    // as it found it.
    let found = unsafe { PeekMessageW(&mut msg, Some(hwnd), WM_CHAR, WM_CHAR, PM_NOREMOVE) };
    if !found.as_bool() {
        return None;
    }
    let unit = msg.wParam.0 as u16;
    // A lone surrogate is half of a character; those arrive from input methods,
    // which deliver their text through the IME path instead.
    let ch = char::from_u32(u32::from(unit))?;
    (!ch.is_control()).then(|| ch.to_string())
}

fn sync_modifiers() {
    let modifiers = keycode::modifiers();
    let changed = STATE.with(|state| state.modifiers.replace(modifiers)) != modifiers;
    if changed {
        send_current(ShellEvent::ModifiersChanged(modifiers));
    }
}
