//! A text and code editor for xui, built on `TextSurface`.
//!
//! xui lays out, scrolls and paints a `TextSurface`; this crate is what the
//! surface shows and how it is edited (`docs/text-surface-api.md`). It comes
//! in two layers:
//!
//! - [`Buffer`], shared through a [`BufferHandle`]: the [`Document`] (a rope,
//!   one surface block per line, with ids that survive edits), undo history,
//!   the [`Highlighter`], and a log of every edit. Any number of views show
//!   one buffer.
//! - [`EditorView`], one per surface: its [`Selections`] (any number of
//!   carets and ranges), [`Keymap`], [`EditorStyle`], IME composition and
//!   pointer state. It turns input into edits of the buffer.
//!
//! Every edit, from a view's input or from application code, is a
//! [`Transaction`]: applied at once, logged with the blocks it changed, and
//! recorded as one undo step (typing and deleting group across steps). Views
//! read the log to know what to lay out again and where their carets went.
//!
//! ```ignore
//! let buffer = BufferHandle::new("fn main() {}\n");
//! let left = EditorHandle::new(&buffer);
//! let right = EditorHandle::new(&buffer); // the same text, its own carets
//! view! { editor_view(&left).with::<text_surface::size>(Size::fix(640.0, 480.0)) }
//! // From application code: one undo step, seen by both views.
//! buffer.transact(|tx| { tx.insert(Pos::new(0, 0), "// generated\n"); });
//! ```

mod buffer;
mod document;
mod highlight;
mod history;
mod keymap;
mod selection;
mod view;

use std::cell::RefCell;
use std::rc::Rc;

use xui_core::prelude::{Draft, SurfaceWrap, TextSurface, TextSurfaceHandle, text_surface};

pub use buffer::{Buffer, BufferHandle, Edit, Origin, Revision, Transaction, ViewId};
pub use document::{Document, Pos};
pub use highlight::{Highlighter, KeywordHighlighter};
pub use history::EditKind;
pub use keymap::{ChordKey, EditorCommand, KeyChord, Keymap, Motion};
pub use selection::{Selection, Selections};
pub use view::{EditorHost, EditorStyle, EditorView};

/// An [`EditorView`] shared between the application and the surface showing
/// it.
///
/// Change the view through [`update`](Self::update), which tells its surface;
/// change the text through the [`buffer`](Self::buffer), which tells every
/// view. Neither may run while the surface is laying out or handling an event
/// of its own, which is to say not from inside the view's `on_change`.
#[derive(Clone)]
pub struct EditorHandle {
    view: Rc<RefCell<EditorView>>,
    surface: TextSurfaceHandle,
}

impl EditorHandle {
    /// A new view of `buffer`, with a caret at its start.
    pub fn new(buffer: &BufferHandle) -> Self {
        Self::from_view(EditorView::new(buffer))
    }

    /// A view of a buffer of its own, holding `text`.
    pub fn with_text(text: &str) -> Self {
        Self::new(&BufferHandle::new(text))
    }

    /// Shows `view`, which may already be set up.
    pub fn from_view(view: EditorView) -> Self {
        let buffer = view.buffer().clone();
        let id = view.id();
        let view = Rc::new(RefCell::new(view));
        let surface = TextSurfaceHandle::new(view.clone());
        buffer.update(|buffer| buffer.set_view_surface(id, surface.downgrade()));
        Self { view, surface }
    }

    /// The handle a `TextSurface` takes as its `model`.
    pub fn surface(&self) -> TextSurfaceHandle {
        self.surface.clone()
    }

    pub fn buffer(&self) -> BufferHandle {
        self.view.borrow().buffer().clone()
    }

    pub fn read<R>(&self, read: impl FnOnce(&EditorView) -> R) -> R {
        read(&self.view.borrow())
    }

    /// Changes the view, then has its surface sync.
    pub fn update<R>(&self, update: impl FnOnce(&mut EditorView) -> R) -> R {
        let result = update(&mut self.view.borrow_mut());
        self.surface.invalidate();
        result
    }

    pub fn text(&self) -> String {
        self.read(EditorView::text)
    }
}

impl PartialEq for EditorHandle {
    fn eq(&self, other: &Self) -> bool {
        self.surface.same_handle(&other.surface)
    }
}

impl std::fmt::Debug for EditorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.view.try_borrow() {
            Ok(view) => f.debug_tuple("EditorHandle").field(&*view).finish(),
            Err(_) => f.write_str("EditorHandle(<borrowed>)"),
        }
    }
}

/// A `TextSurface` showing `editor`, wrapping as its style says.
pub fn editor_view(editor: &EditorHandle) -> Draft<TextSurface> {
    let wrap = if editor.read(|view| view.style().wrap) {
        SurfaceWrap::Width
    } else {
        SurfaceWrap::None
    };
    text_surface()
        .with::<text_surface::model>(editor.surface())
        .with::<text_surface::wrap>(wrap)
}
