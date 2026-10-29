//! Undo and redo steps, with runs of typing grouped into one.
//!
//! History belongs to the buffer, so every view undoes the same steps. Each
//! step remembers which view made it and that view's selections around it,
//! so undoing in that view puts its carets back where they were; any other
//! view just follows the edits.

use std::time::{Duration, Instant};

use crate::buffer::{Edit, ViewId};
use crate::selection::Selections;

/// Edits of one kind closer together than this undo as one.
const GROUP_INTERVAL: Duration = Duration::from_millis(1000);
const DEFAULT_LIMIT: usize = 1000;

/// What an edit was, for grouping: a run of typing undoes in one step, and so
/// does a run of deleting, but never one of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    /// Never grouped: a paste, a cut, a newline, an indent, an edit from code.
    Other,
}

/// One undo step: edits in the order they were applied.
#[derive(Debug, Clone)]
pub(crate) struct Step {
    pub edits: Vec<Edit>,
    pub view: Option<ViewId>,
    pub before: Option<Selections>,
    pub after: Option<Selections>,
    pub kind: EditKind,
    pub at: Instant,
}

#[derive(Debug, Clone)]
pub(crate) struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    limit: usize,
    /// Set to stop the next step from joining the last.
    sealed: bool,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: DEFAULT_LIMIT,
            sealed: false,
        }
    }
}

impl History {
    pub fn record(&mut self, step: Step) {
        if step.edits.is_empty() {
            return;
        }
        self.redo.clear();
        if !self.sealed
            && step.kind != EditKind::Other
            && let Some(last) = self.undo.last_mut()
            && last.kind == step.kind
            && last.view == step.view
            && step.at.saturating_duration_since(last.at) <= GROUP_INTERVAL
            && last.after == step.before
        {
            last.edits.extend(step.edits);
            last.after = step.after;
            last.at = step.at;
            return;
        }
        self.sealed = false;
        self.undo.push(step);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Makes the next step start a step of its own.
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.sealed = false;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn pop_undo(&mut self) -> Option<Step> {
        self.sealed = true;
        self.undo.pop()
    }

    pub fn pop_redo(&mut self) -> Option<Step> {
        self.sealed = true;
        self.redo.pop()
    }

    /// Puts an undone step where redo finds it.
    pub fn push_redo(&mut self, step: Step) {
        self.redo.push(step);
    }

    /// Puts a redone step back on the undo stack, keeping the rest of redo.
    pub fn push_undone(&mut self, step: Step) {
        self.undo.push(step);
    }
}
