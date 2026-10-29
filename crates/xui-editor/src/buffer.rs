//! The shared part of an editor: the text, its history and styling, and the
//! log of edits every view of it catches up from.
//!
//! A [`Buffer`] can be shown by any number of views at once (a split
//! editor, a preview, a minimap), each with its own selections. Every edit,
//! whoever makes it, goes through a [`Transaction`]: it is applied to the
//! document, appended to the buffer's log with the blocks it changed, and,
//! unless it is a remote edit, recorded as one undo step. Views read the log
//! from where they last were: the blocks they have to lay out again, and how
//! to move their carets.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

use rustc_hash::FxHashMap;
use xui_core::prelude::{
    BlockChange, BlockId, TextContent, TextRun, TextRuns, WeakTextSurfaceHandle,
};

use crate::document::{Document, Pos, end_of, normalize_breaks, ordered};
use crate::highlight::Highlighter;
use crate::history::{EditKind, History, Step};
use crate::selection::Selections;

/// How far a buffer's log has got: one more for every logged change.
pub type Revision = u64;

/// One view of a buffer, unique within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewId(u64);

/// Who made an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// A view, from its input.
    View(ViewId),
    /// Application code. Recorded in history like a view's edit.
    External,
    /// Another replica of the document (a collaborator, a language server's
    /// workspace edit). Not undone locally. History is kept as positions
    /// into the text, which a remote edit invalidates, so it clears local
    /// history until history learns to rebase.
    Remote,
}

impl Origin {
    pub fn view(self) -> Option<ViewId> {
        match self {
            Self::View(view) => Some(view),
            _ => None,
        }
    }
}

/// One replacement as it was applied: at `start`, `removed` became `inserted`.
/// Positions are those of the document at that moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub start: Pos,
    pub removed: String,
    pub inserted: String,
}

impl Edit {
    /// Where the removed text ended.
    pub fn old_end(&self) -> Pos {
        end_of(self.start, &self.removed)
    }

    /// Where the inserted text ends.
    pub fn new_end(&self) -> Pos {
        end_of(self.start, &self.inserted)
    }

    /// Where `pos` is once this edit is applied. A position at the start
    /// stays before what was inserted; one inside what was removed goes to
    /// the start.
    pub fn map(&self, pos: Pos) -> Pos {
        if pos <= self.start {
            return pos;
        }
        let old_end = self.old_end();
        if pos < old_end {
            return self.start;
        }
        shift(pos, old_end, self.new_end())
    }

    /// The edit that undoes this one.
    pub fn inverse(&self) -> Self {
        Self {
            start: self.start,
            removed: self.inserted.clone(),
            inserted: self.removed.clone(),
        }
    }
}

/// Where `pos`, at or after `old_end`, is once `..old_end` became `..new_end`.
pub(crate) fn shift(pos: Pos, old_end: Pos, new_end: Pos) -> Pos {
    if pos < old_end {
        return pos;
    }
    if pos.line == old_end.line {
        Pos::new(new_end.line, new_end.byte + (pos.byte - old_end.byte))
    } else {
        // A later line: only its number moves, by the lines the edit added
        // or took away.
        Pos::new(pos.line - old_end.line + new_end.line, pos.byte)
    }
}

/// One entry of the log.
#[derive(Debug, Clone)]
pub(crate) struct LogEntry {
    pub revision: Revision,
    pub change: LogChange,
}

#[derive(Debug, Clone)]
pub(crate) enum LogChange {
    Edit {
        edit: Edit,
        blocks: Vec<BlockChange>,
    },
    /// The whole text was replaced.
    Reset,
    /// Only the styling: these blocks are drawn again.
    Restyle { blocks: Vec<BlockChange> },
}

/// What the buffer knows of one of its views.
#[derive(Debug)]
struct ViewEntry {
    /// The oldest revision the view still has to read.
    seen: Revision,
    /// Its surface, told to sync when anything changes.
    surface: Option<WeakTextSurfaceHandle>,
}

/// Text, history and styling shared by every view of them.
pub struct Buffer {
    doc: Document,
    history: History,
    log: VecDeque<LogEntry>,
    revision: Revision,
    highlighter: Option<Box<dyn Highlighter>>,
    /// Each line's text and runs as last built, until it changes.
    styled: RefCell<FxHashMap<BlockId, (TextContent, TextRuns<TextRun>)>>,
    views: FxHashMap<ViewId, ViewEntry>,
    next_view: u64,
}

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buffer")
            .field("lines", &self.doc.line_count())
            .field("revision", &self.revision)
            .field("views", &self.views.len())
            .field("log", &self.log.len())
            .finish_non_exhaustive()
    }
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        Self {
            doc: Document::new(text),
            history: History::default(),
            log: VecDeque::new(),
            revision: 0,
            highlighter: None,
            styled: RefCell::new(FxHashMap::default()),
            views: FxHashMap::default(),
            next_view: 0,
        }
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn text(&self) -> String {
        self.doc.text()
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Applies the edits `edit` makes as one step, from `origin`, and tells
    /// every view. Edits inside apply at once, so each one sees the text as
    /// the ones before it left it.
    pub fn transact<R>(
        &mut self,
        origin: Origin,
        edit: impl FnOnce(&mut Transaction<'_>) -> R,
    ) -> R {
        let mut transaction = Transaction {
            buffer: self,
            origin,
            kind: EditKind::Other,
            record: origin != Origin::Remote,
            edits: Vec::new(),
            selections: None,
        };
        let result = edit(&mut transaction);
        let Transaction {
            origin,
            kind,
            record,
            edits,
            selections,
            ..
        } = transaction;
        if edits.is_empty() {
            return result;
        }
        if origin == Origin::Remote {
            self.history.clear();
        } else if record {
            let (before, after) = selections.unzip();
            self.history.record(Step {
                edits,
                view: origin.view(),
                before,
                after,
                kind,
                at: Instant::now(),
            });
        }
        self.notify();
        result
    }

    /// Replaces the whole text, for every view, and forgets history.
    pub fn set_text(&mut self, text: &str) {
        self.doc = Document::new(text);
        self.history.clear();
        self.styled.get_mut().clear();
        self.push_log(LogChange::Reset);
        self.notify();
    }

    pub fn set_highlighter(&mut self, highlighter: Option<Box<dyn Highlighter>>) {
        self.highlighter = highlighter;
        self.restyle_all();
    }

    /// Lines whose styling changed outside the buffer's knowledge, as a
    /// highlighter with its own parse reports: they are drawn again.
    pub fn restyle_lines(&mut self, lines: impl IntoIterator<Item = usize>) {
        let mut blocks = Vec::new();
        for line in lines {
            if line < self.doc.line_count() {
                let id = self.doc.id(line);
                self.styled.get_mut().remove(&id);
                blocks.push(BlockChange::Text { id });
            }
        }
        if !blocks.is_empty() {
            self.push_log(LogChange::Restyle { blocks });
            self.notify();
        }
    }

    fn restyle_all(&mut self) {
        self.styled.get_mut().clear();
        let blocks = (0..self.doc.line_count())
            .map(|line| BlockChange::Paint {
                id: self.doc.id(line),
            })
            .collect();
        self.push_log(LogChange::Restyle { blocks });
        self.notify();
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Makes the next edit start an undo step of its own.
    pub fn seal_history(&mut self) {
        self.history.seal();
    }

    /// Reverts the last step. Returns the view that made it and
    /// its selections from before, when it was a view.
    pub fn undo(&mut self) -> Option<(Option<ViewId>, Option<Selections>)> {
        let step = self.history.pop_undo()?;
        for edit in step.edits.iter().rev() {
            let inverse = edit.inverse();
            self.apply(inverse.start, inverse.old_end(), &inverse.inserted);
        }
        let result = (step.view, step.before.clone());
        self.history.push_redo(step);
        self.notify();
        Some(result)
    }

    /// Applies the last undone step again. Returns the view that made it and
    /// its selections from after, when it was a view.
    pub fn redo(&mut self) -> Option<(Option<ViewId>, Option<Selections>)> {
        let step = self.history.pop_redo()?;
        for edit in &step.edits {
            self.apply(edit.start, edit.old_end(), &edit.inserted);
        }
        let result = (step.view, step.after.clone());
        self.history.push_undone(step);
        self.notify();
        Some(result)
    }

    /// Applies one replacement and logs it.
    fn apply(&mut self, start: Pos, end: Pos, text: &str) -> Edit {
        let (start, end) = ordered(self.doc.clamp(start), self.doc.clamp(end));
        let inserted = normalize_breaks(text).into_owned();
        let removed = self.doc.slice(start, end);
        let mut blocks = Vec::new();
        self.doc.replace(start, end, &inserted, &mut blocks);
        let styled = self.styled.get_mut();
        for block in &blocks {
            if let BlockChange::Text { id } | BlockChange::Removed { id } = block {
                styled.remove(id);
            }
        }
        let edit = Edit {
            start,
            removed,
            inserted,
        };
        self.push_log(LogChange::Edit {
            edit: edit.clone(),
            blocks,
        });
        edit
    }

    fn push_log(&mut self, change: LogChange) {
        self.revision += 1;
        self.log.push_back(LogEntry {
            revision: self.revision,
            change,
        });
        self.trim_log();
    }

    /// The log after `revision`, or `None` when it has been dropped: a view
    /// that far behind reads everything again.
    pub(crate) fn log_since(&self, revision: Revision) -> Option<impl Iterator<Item = &LogEntry>> {
        let oldest = self
            .log
            .front()
            .map_or(self.revision + 1, |entry| entry.revision);
        if revision + 1 < oldest {
            return None;
        }
        let skip = (revision + 1 - oldest) as usize;
        Some(self.log.iter().skip(skip))
    }

    /// Each line's text and styling, built once until it changes.
    pub(crate) fn styled_line(&self, id: BlockId) -> Option<(TextContent, TextRuns<TextRun>)> {
        if let Some(styled) = self.styled.borrow().get(&id) {
            return Some(styled.clone());
        }
        let line = self.doc.line_of(id)?;
        let text = self.doc.line(line);
        let runs = self
            .highlighter
            .as_ref()
            .map_or_else(TextRuns::default, |highlighter| {
                TextRuns::new(highlighter.highlight(&text))
            });
        let styled = (TextContent::from(text), runs);
        self.styled.borrow_mut().insert(id, styled.clone());
        Some(styled)
    }

    pub(crate) fn register_view(&mut self) -> ViewId {
        let id = ViewId(self.next_view);
        self.next_view += 1;
        self.views.insert(
            id,
            ViewEntry {
                seen: self.revision,
                surface: None,
            },
        );
        id
    }

    pub(crate) fn set_view_surface(&mut self, view: ViewId, surface: WeakTextSurfaceHandle) {
        if let Some(entry) = self.views.get_mut(&view) {
            entry.surface = Some(surface);
        }
    }

    pub(crate) fn unregister_view(&mut self, view: ViewId) {
        self.views.remove(&view);
        self.trim_log();
    }

    /// Records that `view` has read the log up to `revision`.
    pub(crate) fn mark_seen(&mut self, view: ViewId, revision: Revision) {
        if let Some(entry) = self.views.get_mut(&view) {
            entry.seen = revision;
        }
        self.trim_log();
    }

    /// Drops the entries every view has read.
    fn trim_log(&mut self) {
        let floor = self
            .views
            .values()
            .map(|entry| entry.seen)
            .min()
            .unwrap_or(self.revision);
        while self
            .log
            .front()
            .is_some_and(|entry| entry.revision <= floor)
        {
            self.log.pop_front();
        }
    }

    /// Has every view's surface sync; forgets views whose surface is gone.
    fn notify(&mut self) {
        self.views.retain(|_, entry| match &entry.surface {
            None => true,
            Some(weak) => match weak.upgrade() {
                Some(surface) => {
                    surface.invalidate();
                    true
                }
                None => false,
            },
        });
    }
}

/// The edits of one step, applied as they are made. See [`Buffer::transact`].
pub struct Transaction<'a> {
    buffer: &'a mut Buffer,
    origin: Origin,
    kind: EditKind,
    record: bool,
    edits: Vec<Edit>,
    selections: Option<(Selections, Selections)>,
}

impl Transaction<'_> {
    /// The text as the edits so far left it.
    pub fn document(&self) -> &Document {
        &self.buffer.doc
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// Replaces `start..end` (in either order) with `text`. Returns where the
    /// inserted text ends.
    pub fn replace(&mut self, start: Pos, end: Pos, text: &str) -> Pos {
        let edit = self.buffer.apply(start, end, text);
        let end = edit.new_end();
        self.edits.push(edit);
        end
    }

    pub fn insert(&mut self, at: Pos, text: &str) -> Pos {
        self.replace(at, at, text)
    }

    pub fn delete(&mut self, start: Pos, end: Pos) {
        self.replace(start, end, "");
    }

    /// How the step groups with the ones before it in undo.
    pub fn set_kind(&mut self, kind: EditKind) {
        self.kind = kind;
    }

    /// The edits made so far, in order.
    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    /// Keeps the step out of history: text that is there only for now, as an
    /// IME composition is, and is taken out again before anything is undone.
    pub(crate) fn untracked(&mut self) {
        self.record = false;
    }

    /// The selections of the view making the step, before and after it, for
    /// undo and redo to put back.
    pub(crate) fn set_selections(&mut self, before: Selections, after: Selections) {
        self.selections = Some((before, after));
    }
}

/// A [`Buffer`] shared between views and the application.
///
/// Edits made through it come from [`Origin::External`] (or
/// [`transact_remote`](Self::transact_remote)) and reach every view. Do not
/// call it while a view of the buffer is handling an event or syncing, which
/// is to say not from inside a view's `on_change`.
#[derive(Clone)]
pub struct BufferHandle {
    inner: Rc<RefCell<Buffer>>,
}

impl BufferHandle {
    pub fn new(text: &str) -> Self {
        Self::from_buffer(Buffer::new(text))
    }

    pub fn from_buffer(buffer: Buffer) -> Self {
        Self {
            inner: Rc::new(RefCell::new(buffer)),
        }
    }

    pub fn read<R>(&self, read: impl FnOnce(&Buffer) -> R) -> R {
        read(&self.inner.borrow())
    }

    pub fn update<R>(&self, update: impl FnOnce(&mut Buffer) -> R) -> R {
        update(&mut self.inner.borrow_mut())
    }

    /// One undo step of edits from application code.
    pub fn transact<R>(&self, edit: impl FnOnce(&mut Transaction<'_>) -> R) -> R {
        self.update(|buffer| buffer.transact(Origin::External, edit))
    }

    /// Edits from another replica; see [`Origin::Remote`].
    pub fn transact_remote<R>(&self, edit: impl FnOnce(&mut Transaction<'_>) -> R) -> R {
        self.update(|buffer| buffer.transact(Origin::Remote, edit))
    }

    pub fn text(&self) -> String {
        self.read(Buffer::text)
    }

    pub fn set_text(&self, text: &str) {
        self.update(|buffer| buffer.set_text(text));
    }

    pub fn set_highlighter(&self, highlighter: Option<Box<dyn Highlighter>>) {
        self.update(|buffer| buffer.set_highlighter(highlighter));
    }

    pub fn same_buffer(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    pub(crate) fn borrow(&self) -> std::cell::Ref<'_, Buffer> {
        self.inner.borrow()
    }

    pub(crate) fn borrow_mut(&self) -> std::cell::RefMut<'_, Buffer> {
        self.inner.borrow_mut()
    }

    pub(crate) fn try_borrow_mut(&self) -> Option<std::cell::RefMut<'_, Buffer>> {
        self.inner.try_borrow_mut().ok()
    }
}

impl PartialEq for BufferHandle {
    fn eq(&self, other: &Self) -> bool {
        self.same_buffer(other)
    }
}

impl std::fmt::Debug for BufferHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner.try_borrow() {
            Ok(buffer) => f.debug_tuple("BufferHandle").field(&*buffer).finish(),
            Err(_) => f.write_str("BufferHandle(<borrowed>)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_maps_positions_around_it() {
        let edit = Edit {
            start: Pos::new(0, 2),
            removed: "cd\nef".into(),
            inserted: "X".into(),
        };
        assert_eq!(edit.old_end(), Pos::new(1, 2));
        assert_eq!(edit.map(Pos::new(0, 1)), Pos::new(0, 1));
        assert_eq!(edit.map(Pos::new(0, 2)), Pos::new(0, 2));
        assert_eq!(edit.map(Pos::new(1, 0)), Pos::new(0, 2));
        assert_eq!(edit.map(Pos::new(1, 4)), Pos::new(0, 5));
        assert_eq!(edit.map(Pos::new(3, 1)), Pos::new(2, 1));
    }

    #[test]
    fn the_log_keeps_what_some_view_has_not_read() {
        let mut buffer = Buffer::new("a");
        let slow = buffer.register_view();
        let fast = buffer.register_view();
        for _ in 0..3 {
            buffer.transact(Origin::External, |tx| {
                tx.insert(Pos::new(0, 0), "x");
            });
        }
        buffer.mark_seen(fast, buffer.revision());
        assert_eq!(buffer.log_since(0).map(Iterator::count), Some(3));
        buffer.mark_seen(slow, 2);
        assert_eq!(buffer.log_since(2).map(Iterator::count), Some(1));
        assert!(
            buffer.log_since(0).is_none(),
            "dropped once both had read it"
        );
    }

    #[test]
    fn a_remote_edit_is_not_undone_and_clears_history() {
        let mut buffer = Buffer::new("");
        buffer.transact(Origin::External, |tx| {
            tx.insert(Pos::new(0, 0), "local");
        });
        assert!(buffer.can_undo());
        buffer.transact(Origin::Remote, |tx| {
            tx.insert(Pos::new(0, 0), "remote ");
        });
        assert!(!buffer.can_undo());
        assert_eq!(buffer.text(), "remote local");
    }
}
