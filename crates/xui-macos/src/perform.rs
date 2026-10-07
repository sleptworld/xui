//! Carrying out the commands and requests the application leaves for the host.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardTypeString, NSWindow, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSArray, NSSize, NSString, NSURL};
use xui_interface::{
    ClipboardContent, ClipboardTarget, PlatformCommand, PlatformError, PlatformReply,
    PlatformRequest, WindowCommand,
};
use xui_shell::Performed;

use crate::window::MacWindow;

/// `NSNormalWindowLevel` and `NSFloatingWindowLevel`.
const NORMAL_WINDOW_LEVEL: isize = 0;
const FLOATING_WINDOW_LEVEL: isize = 3;

pub(crate) fn perform(
    mtm: MainThreadMarker,
    window: &MacWindow,
    command: PlatformCommand,
) -> Performed {
    match command {
        PlatformCommand::WriteClipboard { target, content } => write_clipboard(target, content),
        PlatformCommand::OpenUrl(url) => open_url(&url),
        PlatformCommand::RevealPath(path) => reveal_path(&path),
        PlatformCommand::Window(command) => window_command(mtm, window, command),
        PlatformCommand::Request(_, PlatformRequest::ReadClipboard(target)) => {
            read_clipboard(target)
        }
        // Dialogs are not implemented yet; the shell turns this into an
        // `Unsupported` reply for the caller.
        PlatformCommand::Request(..) => Performed::Unsupported,
        // Nothing this host does outlives the call that started it, so there is
        // never anything to dismiss.
        PlatformCommand::Cancel(_) => Performed::Done,
        _ => Performed::Unsupported,
    }
}

fn write_clipboard(target: ClipboardTarget, content: ClipboardContent) -> Performed {
    if target != ClipboardTarget::General {
        // macOS has no primary selection.
        return Performed::Unsupported;
    }
    let ClipboardContent::Text(text) = content else {
        return Performed::Unsupported;
    };
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: `NSPasteboardTypeString` is a constant AppKit defines.
    let written = pasteboard.setString_forType(&NSString::from_str(&text), unsafe {
        NSPasteboardTypeString
    });
    if written {
        Performed::Done
    } else {
        Performed::Failed(PlatformError::new("the pasteboard refused the text"))
    }
}

fn read_clipboard(target: ClipboardTarget) -> Performed {
    if target != ClipboardTarget::General {
        return Performed::Unsupported;
    }
    let text = NSPasteboard::generalPasteboard()
        // SAFETY: `NSPasteboardTypeString` is a constant AppKit defines.
        .stringForType(unsafe { NSPasteboardTypeString })
        .map(|text| ClipboardContent::Text(text.to_string()));
    Performed::Reply(PlatformReply::Clipboard(text))
}

fn open_url(url: &str) -> Performed {
    let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
        return Performed::Failed(PlatformError::new(format!("{url:?} is not a URL")));
    };
    if NSWorkspace::sharedWorkspace().openURL(&url) {
        Performed::Done
    } else {
        Performed::Failed(PlatformError::new("no application could open the URL"))
    }
}

fn reveal_path(path: &std::path::Path) -> Performed {
    let Some(path) = path.to_str() else {
        return Performed::Failed(PlatformError::new("the path is not valid UTF-8"));
    };
    let url: Retained<NSURL> = NSURL::fileURLWithPath(&NSString::from_str(path));
    NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_slice(&[&*url]));
    Performed::Done
}

fn window_command(mtm: MainThreadMarker, host: &MacWindow, command: WindowCommand) -> Performed {
    let window: &NSWindow = host.ns_window(mtm);
    match command {
        WindowCommand::SetFullscreen(fullscreen) => {
            let is_fullscreen = window.styleMask().contains(NSWindowStyleMask::FullScreen);
            if is_fullscreen != fullscreen {
                window.toggleFullScreen(None);
            }
            Performed::Done
        }
        WindowCommand::SetMaximized(maximized) => {
            if window.isZoomed() != maximized {
                window.zoom(None);
            }
            Performed::Done
        }
        WindowCommand::Minimize => {
            window.miniaturize(None);
            Performed::Done
        }
        WindowCommand::SetInnerSize(size) => {
            window.setContentSize(NSSize::new(f64::from(size.width), f64::from(size.height)));
            Performed::Done
        }
        WindowCommand::SetSizeLimits { min, max } => {
            let unbounded = NSSize::new(f64::from(f32::MAX), f64::from(f32::MAX));
            let size = |size: xui_interface::Size<f32>| {
                NSSize::new(f64::from(size.width), f64::from(size.height))
            };
            window.setContentMinSize(min.map_or(NSSize::new(0.0, 0.0), size));
            window.setContentMaxSize(max.map_or(unbounded, size));
            Performed::Done
        }
        WindowCommand::SetAlwaysOnTop(on_top) => {
            window.setLevel(if on_top {
                FLOATING_WINDOW_LEVEL
            } else {
                NORMAL_WINDOW_LEVEL
            });
            Performed::Done
        }
        WindowCommand::BeginMove => {
            if host.begin_window_drag(mtm) {
                Performed::Done
            } else {
                Performed::Failed(PlatformError::new(
                    "no pointer button is down to start a move from",
                ))
            }
        }
        // AppKit has no way to start an edge resize from code.
        _ => Performed::Unsupported,
    }
}
