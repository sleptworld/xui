use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{ClassType, msg_send, sel};
use objc2_app_kit::NSCursor;
use xui_interface::CursorIcon;

/// The AppKit cursor for `icon`; `None` means hide the cursor.
// `resizeLeftRightCursor`/`resizeUpDownCursor` are superseded by the
// frame-resize cursors, which `class_cursor` picks up when the system is new
// enough (macOS 15).
#[allow(deprecated)]
pub(crate) fn ns_cursor(icon: CursorIcon) -> Option<Retained<NSCursor>> {
    Some(match icon {
        CursorIcon::Default => NSCursor::arrowCursor(),
        CursorIcon::Pointer => NSCursor::pointingHandCursor(),
        CursorIcon::Text => NSCursor::IBeamCursor(),
        CursorIcon::Crosshair => NSCursor::crosshairCursor(),
        CursorIcon::Grab => NSCursor::openHandCursor(),
        CursorIcon::Grabbing => NSCursor::closedHandCursor(),
        CursorIcon::NotAllowed => NSCursor::operationNotAllowedCursor(),
        CursorIcon::Move => {
            class_cursor(sel!(_moveCursor)).unwrap_or_else(NSCursor::openHandCursor)
        }
        CursorIcon::Wait | CursorIcon::Progress => {
            class_cursor(sel!(busyButClickableCursor)).unwrap_or_else(NSCursor::arrowCursor)
        }
        CursorIcon::Help => class_cursor(sel!(_helpCursor)).unwrap_or_else(NSCursor::arrowCursor),
        CursorIcon::ColumnResize => {
            class_cursor(sel!(columnResizeCursor)).unwrap_or_else(NSCursor::resizeLeftRightCursor)
        }
        CursorIcon::RowResize => {
            class_cursor(sel!(rowResizeCursor)).unwrap_or_else(NSCursor::resizeUpDownCursor)
        }
        CursorIcon::EastWestResize => NSCursor::resizeLeftRightCursor(),
        CursorIcon::NorthSouthResize => NSCursor::resizeUpDownCursor(),
        CursorIcon::None => return None,
    })
}

/// An `NSCursor` class method by selector, if this system has it.
///
/// Two kinds of selector go through here. `columnResizeCursor` and
/// `rowResizeCursor` are public but need macOS 15, so they are asked for
/// rather than linked against. `_moveCursor`, `_helpCursor` and
/// `busyButClickableCursor` are private: AppKit publishes no cursor for move,
/// help, wait or progress, and these are the ones every toolkit uses for them.
/// Each is guarded by `respondsToSelector:`, so a macOS that drops one falls
/// back to a public cursor instead of crashing -- but an App Store submission
/// may want to avoid the private three.
fn class_cursor(selector: Sel) -> Option<Retained<NSCursor>> {
    let class = NSCursor::class();
    // Class methods live on the metaclass.
    if !class.metaclass().responds_to(selector) {
        return None;
    }
    // SAFETY: every selector passed here is a zero-argument class method
    // returning an `NSCursor`, which is what `performSelector:` forwards to.
    let cursor: Option<Retained<AnyObject>> =
        unsafe { msg_send![class, performSelector: selector] };
    cursor.and_then(|cursor| cursor.downcast::<NSCursor>().ok())
}
