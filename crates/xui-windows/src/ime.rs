//! Text input through IMM32.
//!
//! The runtime wants preedit and commit as events, and gives back a cursor
//! area for the candidate window. IMM32 reports both through
//! `WM_IME_COMPOSITION`, whose default handling also draws a composition
//! window of its own -- so every message handled here is answered without
//! calling on to `DefWindowProc`.

use std::time::Instant;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::Ime::{
    CANDIDATEFORM, CFS_EXCLUDE, GCS_COMPSTR, GCS_CURSORPOS, GCS_RESULTSTR, HIMC, IACE_DEFAULT,
    IME_COMPOSITION_STRING, ImmAssociateContextEx, ImmGetCompositionStringW, ImmGetContext,
    ImmReleaseContext, ImmSetCandidateWindow,
};
use xui_interface::events::{RawIme, TextPayload};
use xui_interface::{TextOffset, TextRange};

use crate::host::send_current;
use crate::window::{STATE, scale_of};

/// Turns IME on or off for the window, which is how a control that takes text
/// differs from one that does not.
pub(crate) fn set_allowed(hwnd: HWND, allowed: bool) {
    let changed = STATE.with(|state| state.ime_allowed.replace(allowed) != allowed);
    if !changed {
        return;
    }
    // `IACE_DEFAULT` restores the default context; a null context with no flag
    // detaches it, which is how IMM32 spells "this window takes no IME input".
    // SAFETY: a live window handle.
    unsafe {
        let _ = ImmAssociateContextEx(
            hwnd,
            HIMC::default(),
            if allowed { IACE_DEFAULT } else { 0 },
        );
    }
    let timestamp = Instant::now();
    if allowed {
        send_current(ShellIme::Enabled.into_event(timestamp));
    } else {
        STATE.with(|state| state.composing.set(false));
        send_current(ShellIme::Disabled.into_event(timestamp));
    }
}

/// Moves the candidate window to the runtime's cursor area.
pub(crate) fn move_candidate_window(hwnd: HWND) {
    let area = STATE.with(|state| state.ime_area.get());
    let scale = scale_of(hwnd);
    let to_px = |value: f32| (f64::from(value) * scale).round() as i32;
    let form = CANDIDATEFORM {
        dwIndex: 0,
        // Exclude the caret's own rectangle, so the candidate list is placed
        // clear of the text being composed rather than on top of it.
        dwStyle: CFS_EXCLUDE,
        ptCurrentPos: POINT {
            x: to_px(area.x),
            y: to_px(area.y + area.height),
        },
        rcArea: RECT {
            left: to_px(area.x),
            top: to_px(area.y),
            right: to_px(area.x + area.width),
            bottom: to_px(area.y + area.height),
        },
    };
    with_context(hwnd, |context| {
        // SAFETY: `form` outlives the call, and the context belongs to `hwnd`.
        unsafe {
            let _ = ImmSetCandidateWindow(context, &form);
        }
    });
}

/// Handles `WM_IME_COMPOSITION`: a preedit update, a commit, or both.
pub(crate) fn composition(hwnd: HWND, flags: u32) {
    let timestamp = Instant::now();
    let result = with_context(hwnd, |context| {
        let committed = if flags & GCS_RESULTSTR.0 != 0 {
            string(context, GCS_RESULTSTR)
        } else {
            None
        };
        let preedit = if flags & GCS_COMPSTR.0 != 0 {
            let text = string(context, GCS_COMPSTR).unwrap_or_default();
            // The cursor is an offset in UTF-16 units into the preedit.
            // SAFETY: a null buffer asks for the value rather than a string.
            let units = unsafe { ImmGetCompositionStringW(context, GCS_CURSORPOS, None, 0) };
            Some((text, units.max(0) as usize))
        } else {
            None
        };
        (committed, preedit)
    });
    let Some((committed, preedit)) = result else {
        return;
    };

    if let Some(text) = committed {
        STATE.with(|state| state.composing.set(false));
        // Clearing first, so the runtime's preedit is gone before the text it
        // was standing in for arrives.
        send_current(ShellEventExt::preedit("", None, timestamp));
        send_current(ShellEventExt::commit(&text, timestamp));
    }
    if let Some((text, cursor_units)) = preedit {
        STATE.with(|state| state.composing.set(!text.is_empty()));
        let cursor = utf16_to_byte(&text, cursor_units);
        send_current(ShellEventExt::preedit(
            &text,
            Some((cursor, cursor)),
            timestamp,
        ));
    }
}

/// Handles `WM_IME_ENDCOMPOSITION`: whatever was being composed is gone.
pub(crate) fn end_composition() {
    if !STATE.with(|state| state.composing.replace(false)) {
        return;
    }
    send_current(ShellEventExt::preedit("", None, Instant::now()));
}

pub(crate) fn is_composing() -> bool {
    STATE.with(|state| state.composing.get())
}

fn with_context<R>(hwnd: HWND, f: impl FnOnce(HIMC) -> R) -> Option<R> {
    // SAFETY: a live window handle. A window with no input context returns a
    // null one, which must not be used and must not be released.
    let context = unsafe { ImmGetContext(hwnd) };
    if context.0.is_null() {
        return None;
    }
    let result = f(context);
    // SAFETY: the context came from `ImmGetContext` on this window.
    unsafe {
        let _ = ImmReleaseContext(hwnd, context);
    }
    Some(result)
}

/// Reads one of the composition strings, which IMM32 reports in bytes of
/// UTF-16 rather than in characters.
fn string(context: HIMC, kind: IME_COMPOSITION_STRING) -> Option<String> {
    // SAFETY: a null buffer asks for the size the string would need.
    let bytes = unsafe { ImmGetCompositionStringW(context, kind, None, 0) };
    if bytes <= 0 {
        return None;
    }
    let mut buffer = vec![0u16; bytes as usize / 2];
    // SAFETY: the buffer is exactly the size just reported.
    let written = unsafe {
        ImmGetCompositionStringW(
            context,
            kind,
            Some(buffer.as_mut_ptr().cast()),
            bytes as u32,
        )
    };
    if written <= 0 {
        return None;
    }
    buffer.truncate(written as usize / 2);
    Some(String::from_utf16_lossy(&buffer))
}

/// The byte offset `units` UTF-16 code units into `text`, clamped to its end.
fn utf16_to_byte(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (byte, ch) in text.char_indices() {
        if seen >= units {
            return byte;
        }
        seen += ch.len_utf16();
    }
    text.len()
}

/// The two IME events that carry no text, spelled out so the call sites above
/// read as what they mean.
enum ShellIme {
    Enabled,
    Disabled,
}

impl ShellIme {
    fn into_event(self, timestamp: Instant) -> xui_shell::ShellEvent {
        xui_shell::ShellEvent::Ime(match self {
            Self::Enabled => RawIme::Enabled { timestamp },
            Self::Disabled => RawIme::Disabled { timestamp },
        })
    }
}

struct ShellEventExt;

impl ShellEventExt {
    fn preedit(
        text: &str,
        cursor: Option<(usize, usize)>,
        timestamp: Instant,
    ) -> xui_shell::ShellEvent {
        xui_shell::ShellEvent::Ime(RawIme::Preedit {
            text: TextPayload::new(text),
            cursor: cursor.map(|(start, end)| {
                TextRange::new(TextOffset::byte_offset(start), TextOffset::byte_offset(end))
            }),
            timestamp,
        })
    }

    fn commit(text: &str, timestamp: Instant) -> xui_shell::ShellEvent {
        xui_shell::ShellEvent::Ime(RawIme::Commit {
            text: TextPayload::new(text),
            timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::utf16_to_byte;

    #[test]
    fn utf16_offsets_become_byte_offsets() {
        assert_eq!(utf16_to_byte("abc", 2), 2);
        assert_eq!(utf16_to_byte("中文", 1), 3);
        assert_eq!(utf16_to_byte("ab", 9), 2);
    }
}
