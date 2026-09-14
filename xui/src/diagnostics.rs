//! Observability for broken internal invariants.
//!
//! The runtime keeps several structures keyed by the same `NodeId`: the host
//! tree, the layout tree, the style system, the render system. A lookup that
//! misses in one of them after hitting in another is not a condition the caller
//! can meaningfully recover from -- it means the frame already went wrong
//! somewhere upstream. Historically those sites bailed out with a bare
//! `else { return }`, which turned a reconciler bug into an unclickable widget
//! or a silently skipped paint with nothing in the logs to work from.
//!
//! [`invariant!`] keeps that recovery -- it hands the `Option` straight back, so
//! control flow is unchanged -- but reports the miss on the way through. A
//! violation stays non-fatal by default because some of these ids legitimately
//! go stale across a teardown, and crashing an application over a cosmetic miss
//! would be worse than the no-op. Enable the `strict-invariants` feature to turn
//! every report into a panic, which is what the test suite runs with.

use std::fmt;

#[cold]
#[inline(never)]
pub(crate) fn report(file: &'static str, line: u32, context: fmt::Arguments<'_>) {
    #[cfg(feature = "strict-invariants")]
    panic!("xui invariant violated at {file}:{line}: {context}");

    #[cfg(not(feature = "strict-invariants"))]
    log::error!("xui invariant violated at {file}:{line}: {context}");
}

/// Passes an `Option` through, reporting a diagnostic when it is `None`.
///
/// Use it only where `None` means the runtime's own state is inconsistent. A
/// `None` that is a normal outcome -- an empty undo stack, a pointer over no
/// widget, the root node's absent parent -- is not an invariant violation and
/// must not be wrapped, or the log fills with noise and stops being read.
macro_rules! invariant {
    ($value:expr, $($arg:tt)+) => {{
        let value = $value;
        if value.is_none() {
            $crate::diagnostics::report(file!(), line!(), format_args!($($arg)+));
        }
        value
    }};
}

pub(crate) use invariant;
