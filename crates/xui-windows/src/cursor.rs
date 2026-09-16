use windows::Win32::UI::WindowsAndMessaging::{
    HCURSOR, IDC_APPSTARTING, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_HELP, IDC_IBEAM, IDC_NO,
    IDC_SIZEALL, IDC_SIZENS, IDC_SIZEWE, IDC_WAIT, LoadCursorW, SetCursor, ShowCursor,
};
use xui_interface::CursorIcon;

use crate::window::STATE;

/// Remembers what the runtime last asked for, so `WM_SETCURSOR` can restore it.
///
/// Windows resets the cursor to the window class's on every move over the
/// client area; a class cursor of null plus this is what keeps ours in place.
pub(crate) fn apply(icon: CursorIcon) {
    STATE.with(|state| match handle_for(icon) {
        None => {
            if !state.cursor_hidden.replace(true) {
                // SAFETY: no preconditions. The count is balanced by `unhide`.
                unsafe { ShowCursor(false) };
            }
        }
        Some(cursor) => {
            if state.cursor_hidden.replace(false) {
                // SAFETY: no preconditions.
                unsafe { ShowCursor(true) };
            }
            state.cursor.set(cursor.0 as isize);
            // SAFETY: a cursor loaded from the system's shared set, which is
            // valid for the life of the process.
            unsafe { SetCursor(Some(cursor)) };
        }
    });
}

/// Answers `WM_SETCURSOR` for the client area.
pub(crate) fn set_current() {
    STATE.with(|state| {
        if state.cursor_hidden.get() {
            // SAFETY: no preconditions; a null cursor hides it.
            unsafe { SetCursor(None) };
            return;
        }
        let cursor = state.cursor.get();
        let cursor = if cursor == 0 {
            load(IDC_ARROW)
        } else {
            Some(HCURSOR(cursor as *mut _))
        };
        // SAFETY: as above.
        unsafe { SetCursor(cursor) };
    });
}

/// The Win32 cursor for `icon`; `None` means hide the cursor.
///
/// Win32 has no column/row-resize cursors distinct from the plain
/// left-right/up-down ones, so those pairs share a shape.
fn handle_for(icon: CursorIcon) -> Option<HCURSOR> {
    let name = match icon {
        CursorIcon::Default => IDC_ARROW,
        CursorIcon::Pointer => IDC_HAND,
        CursorIcon::Text => IDC_IBEAM,
        CursorIcon::Crosshair => IDC_CROSS,
        // No open/closed hand in the system set; the four-way move cursor is
        // what Windows applications use while dragging.
        CursorIcon::Move | CursorIcon::Grab | CursorIcon::Grabbing => IDC_SIZEALL,
        CursorIcon::NotAllowed => IDC_NO,
        CursorIcon::Wait => IDC_WAIT,
        CursorIcon::Progress => IDC_APPSTARTING,
        CursorIcon::Help => IDC_HELP,
        CursorIcon::ColumnResize | CursorIcon::EastWestResize => IDC_SIZEWE,
        CursorIcon::RowResize | CursorIcon::NorthSouthResize => IDC_SIZENS,
        CursorIcon::None => return None,
    };
    load(name)
}

fn load(name: windows::core::PCWSTR) -> Option<HCURSOR> {
    // SAFETY: a null instance loads from the system's shared cursors, which
    // need no unloading.
    unsafe { LoadCursorW(None, name).ok() }
}
