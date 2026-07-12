use std::{
    cell::RefCell,
    fmt,
    hash::{Hash, Hasher},
    ops::RangeBounds,
    rc::Rc,
    sync::Arc,
};

use xui_interface::TextPayload;

use super::value::*;

#[derive(Clone)]
pub struct TextController {
    inner: Rc<RefCell<TextControllerState>>,
}

#[derive(Debug, Clone)]
struct TextControllerState {
    value: TextEditingValue,
    history: TextHistory,
}

#[derive(Debug, Clone, Default)]
struct TextHistory {
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    redo: TextChangeSet,
    undo: TextChangeSet,
    before_selection: TextSelection,
    after_selection: TextSelection,
    before_composing: Option<TextRange>,
    after_composing: Option<TextRange>,
}

impl TextController {
    pub fn new() -> Self {
        Self::from_value(TextEditingValue::default())
    }

    pub fn with_text(text: impl AsRef<str>) -> Self {
        Self::from_value(TextEditingValue::with_text(text))
    }

    pub fn from_value(value: TextEditingValue) -> Self {
        Self {
            inner: Rc::new(RefCell::new(TextControllerState {
                value,
                history: TextHistory::default(),
            })),
        }
    }

    pub fn value(&self) -> TextEditingValue {
        self.inner.borrow().value.clone()
    }

    pub fn text(&self) -> String {
        self.inner.borrow().value.text()
    }

    pub fn len_chars(&self) -> usize {
        self.inner.borrow().value.len_chars()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.borrow().value.is_empty()
    }

    pub fn selection(&self) -> TextSelection {
        self.inner.borrow().value.selection
    }

    pub fn composing(&self) -> Option<TextRange> {
        self.inner.borrow().value.composing
    }

    pub fn revision(&self) -> u64 {
        self.inner.borrow().value.revision()
    }

    pub fn same_handle(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn can_undo(&self) -> bool {
        !self.inner.borrow().history.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.inner.borrow().history.redo_stack.is_empty()
    }

    pub fn clear_history(&self) {
        self.inner.borrow_mut().history = TextHistory::default();
    }

    pub fn set_value(&self, mut value: TextEditingValue) {
        let mut inner = self.inner.borrow_mut();
        value.revision = inner.value.revision().wrapping_add(1);
        inner.value = value;
        inner.history = TextHistory::default();
    }

    pub fn set_text(&self, text: impl AsRef<str>) {
        let mut value = TextEditingValue::with_text(text);
        let mut inner = self.inner.borrow_mut();
        value.revision = inner.value.revision().wrapping_add(1);
        inner.value = value;
        inner.history = TextHistory::default();
    }

    pub fn set_selection(&self, selection: TextSelection) -> Result<(), TextEditError> {
        let mut inner = self.inner.borrow_mut();
        selection.validate(inner.value.len_chars())?;

        if inner.value.selection != selection {
            inner.value.selection = selection;
            inner.value.revision = inner.value.revision().wrapping_add(1);
        }

        Ok(())
    }

    pub fn set_cursor(&self, offset: usize) -> Result<(), TextEditError> {
        self.set_selection(TextSelection::collapsed(offset))
    }

    pub fn set_composing(&self, composing: Option<TextRange>) -> Result<(), TextEditError> {
        let mut inner = self.inner.borrow_mut();
        if let Some(range) = composing {
            range.validate(inner.value.len_chars()).map_err(|_| {
                TextEditError::InvalidComposing {
                    range,
                    len: inner.value.len_chars(),
                }
            })?;
        }

        if inner.value.composing != composing {
            inner.value.composing = composing;
            inner.value.revision = inner.value.revision().wrapping_add(1);
        }

        Ok(())
    }

    pub fn apply_change_set(
        &self,
        change_set: TextChangeSet,
    ) -> Result<AppliedChangeSet, TextEditError> {
        self.apply_change_set_with_state(change_set, None, None)
    }

    pub fn apply_change_set_with_state(
        &self,
        change_set: TextChangeSet,
        selection_after: Option<TextSelection>,
        composing_after: Option<Option<TextRange>>,
    ) -> Result<AppliedChangeSet, TextEditError> {
        self.apply_change_set_inner(change_set, selection_after, composing_after, true)
    }

    pub fn insert_text(
        &self,
        text: impl Into<Arc<str>>,
    ) -> Result<AppliedChangeSet, TextEditError> {
        let text = text.into();
        let value = self.value();
        let selection = value.selection;
        let range = selection.range();

        if range.is_empty() {
            let end = range.start + text.chars().count();
            self.apply_change_set_with_state(
                TextChangeSet::insert(value.len_chars(), range.start, text),
                Some(TextSelection::collapsed(end)),
                Some(None),
            )
        } else {
            self.replace(range, text)
        }
    }

    pub fn delete<R>(&self, range: R) -> Result<AppliedChangeSet, TextEditError>
    where
        R: RangeBounds<usize>,
    {
        let len = self.len_chars();
        let range = TextRange::from_bounds(range, len)?;
        self.apply_change_set_with_state(
            TextChangeSet::delete(len, range),
            Some(TextSelection::collapsed(range.start)),
            Some(None),
        )
    }

    pub fn replace(
        &self,
        range: TextRange,
        text: impl Into<Arc<str>>,
    ) -> Result<AppliedChangeSet, TextEditError> {
        let text = text.into();
        let len = self.len_chars();
        range.validate(len)?;
        let end = range.start + text.chars().count();
        self.apply_change_set_with_state(
            TextChangeSet::replace(len, range, text),
            Some(TextSelection::collapsed(end)),
            Some(None),
        )
    }

    pub fn backspace(&self) -> Result<Option<AppliedChangeSet>, TextEditError> {
        let value = self.value();
        let selection = value.selection;
        let range = selection.range();

        if !range.is_empty() {
            return self.delete(range.as_range()).map(Some);
        }

        if range.start == 0 {
            return Ok(None);
        }

        self.delete(range.start - 1..range.start).map(Some)
    }

    pub fn clear(&self) -> Result<AppliedChangeSet, TextEditError> {
        let len = self.len_chars();
        self.delete(0..len)
    }

    pub fn undo(&self) -> Result<bool, TextEditError> {
        let mut inner = self.inner.borrow_mut();
        let Some(entry) = inner.history.undo_stack.pop() else {
            return Ok(false);
        };

        inner.value.apply_change_set(
            entry.undo.clone(),
            Some(entry.before_selection),
            Some(entry.before_composing),
        )?;
        inner.history.redo_stack.push(entry);
        Ok(true)
    }

    pub fn redo(&self) -> Result<bool, TextEditError> {
        let mut inner = self.inner.borrow_mut();
        let Some(entry) = inner.history.redo_stack.pop() else {
            return Ok(false);
        };

        inner.value.apply_change_set(
            entry.redo.clone(),
            Some(entry.after_selection),
            Some(entry.after_composing),
        )?;
        inner.history.undo_stack.push(entry);
        Ok(true)
    }

    fn replace_with_state(
        &self,
        range: TextRange,
        text: impl Into<Arc<str>>,
        selection_after: Option<TextSelection>,
        composing_after: Option<Option<TextRange>>,
        record_history: bool,
    ) -> Result<AppliedChangeSet, TextEditError> {
        let text = text.into();
        let len = self.len_chars();
        range.validate(len)?;
        self.apply_change_set_inner(
            TextChangeSet::replace(len, range, text),
            selection_after,
            composing_after,
            record_history,
        )
    }

    fn apply_change_set_inner(
        &self,
        change_set: TextChangeSet,
        selection_after: Option<TextSelection>,
        composing_after: Option<Option<TextRange>>,
        record_history: bool,
    ) -> Result<AppliedChangeSet, TextEditError> {
        let mut inner = self.inner.borrow_mut();
        let redo = change_set.clone();
        let applied = inner
            .value
            .apply_change_set(change_set, selection_after, composing_after)?;

        if record_history && !redo.is_empty() {
            inner.history.undo_stack.push(HistoryEntry {
                redo,
                undo: applied.inverse.clone(),
                before_selection: applied.old_selection,
                after_selection: applied.new_selection,
                before_composing: applied.old_composing,
                after_composing: applied.new_composing,
            });
            inner.history.redo_stack.clear();
        }

        Ok(applied)
    }
}

impl Hash for TextController {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.inner).hash(state);
    }
}

impl Default for TextController {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TextController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner.borrow();
        f.debug_struct("TextController")
            .field("value", &inner.value)
            .field("can_undo", &!inner.history.undo_stack.is_empty())
            .field("can_redo", &!inner.history.redo_stack.is_empty())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct ImeSession {
    active: bool,
    anchor: usize,
    original_range: TextRange,
    original_text: Arc<str>,
    preedit_range: Option<TextRange>,
    preedit_cursor: Option<TextSelection>,
}

impl Default for ImeSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ImeSession {
    pub fn new() -> Self {
        Self {
            active: false,
            anchor: 0,
            original_range: TextRange::collapsed(0),
            original_text: Arc::from(""),
            preedit_range: None,
            preedit_cursor: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn anchor(&self) -> Option<usize> {
        self.active.then_some(self.anchor)
    }

    pub fn preedit_range(&self) -> Option<TextRange> {
        self.preedit_range
    }

    pub fn preedit_cursor(&self) -> Option<TextSelection> {
        self.preedit_cursor
    }

    pub fn begin(&mut self, controller: &TextController) -> Result<(), TextEditError> {
        let value = controller.value();
        let original_range = value.selection.range();
        original_range.validate(value.len_chars())?;
        let original_text: Arc<str> = value
            .slice(original_range.as_range())
            .unwrap()
            .to_string()
            .into();

        self.active = true;
        self.anchor = original_range.start;
        self.original_range = original_range;
        self.original_text = original_text;
        self.preedit_range = None;
        self.preedit_cursor = None;
        controller.set_composing(None)?;
        Ok(())
    }

    pub fn preedit(
        &mut self,
        controller: &TextController,
        text: &TextPayload,
        cursor_byte_range: Option<xui_interface::TextRange>,
    ) -> Result<Option<AppliedChangeSet>, TextEditError> {
        self.ensure_active(controller)?;

        if text.is_empty() {
            let applied = self.restore_original(controller)?;
            self.preedit_range = None;
            self.preedit_cursor = None;
            controller.set_composing(None)?;
            return Ok(applied);
        }

        let text = text.as_str();

        let target_range = self.preedit_range.unwrap_or(self.original_range);
        let inserted_len = text.chars().count();
        let new_range = TextRange::new(target_range.start, target_range.start + inserted_len);

        let cursor = cursor_byte_range
            .map(|range| {
                preedit_cursor_to_selection(
                    &text,
                    (range.start.raw, range.end.raw),
                    new_range.start,
                )
            })
            .unwrap_or_else(|| TextSelection::collapsed(new_range.end));

        let applied = controller.replace_with_state(
            target_range,
            text,
            Some(cursor),
            Some(Some(new_range)),
            false,
        )?;

        self.preedit_range = Some(new_range);
        self.preedit_cursor = cursor_byte_range.map(|_| cursor);

        Ok(Some(applied))
    }

    pub fn commit(
        &mut self,
        controller: &TextController,
        text: &TextPayload,
    ) -> Result<AppliedChangeSet, TextEditError> {
        self.ensure_active(controller)?;

        if self.preedit_range.is_some() {
            self.restore_original(controller)?;
        }

        let text = text.as_str();

        let selection_after =
            TextSelection::collapsed(self.original_range.start + text.chars().count());
        let applied = controller.replace_with_state(
            self.original_range,
            text,
            Some(selection_after),
            Some(None),
            true,
        )?;

        self.reset();
        Ok(applied)
    }

    pub fn cancel(
        &mut self,
        controller: &TextController,
    ) -> Result<Option<AppliedChangeSet>, TextEditError> {
        if !self.active {
            return Ok(None);
        }

        let applied = self.restore_original(controller)?;
        controller.set_composing(None)?;
        self.reset();
        Ok(applied)
    }

    pub fn end(
        &mut self,
        controller: &TextController,
    ) -> Result<Option<AppliedChangeSet>, TextEditError> {
        self.cancel(controller)
    }

    fn ensure_active(&mut self, controller: &TextController) -> Result<(), TextEditError> {
        if self.active {
            Ok(())
        } else {
            self.begin(controller)
        }
    }

    fn restore_original(
        &mut self,
        controller: &TextController,
    ) -> Result<Option<AppliedChangeSet>, TextEditError> {
        let Some(preedit_range) = self.preedit_range else {
            return Ok(None);
        };

        let restored_len = self.original_text.chars().count();
        let restored_range = TextRange::new(self.anchor, self.anchor + restored_len);
        let selection = TextSelection::new(restored_range.start, restored_range.end);
        let applied = controller.replace_with_state(
            preedit_range,
            self.original_text.clone(),
            Some(selection),
            Some(None),
            false,
        )?;

        self.original_range = restored_range;
        self.preedit_range = None;
        self.preedit_cursor = None;
        Ok(Some(applied))
    }

    fn reset(&mut self) {
        self.active = false;
        self.anchor = 0;
        self.original_range = TextRange::collapsed(0);
        self.original_text = Arc::from("");
        self.preedit_range = None;
        self.preedit_cursor = None;
    }
}

fn preedit_cursor_to_selection(
    text: &str,
    cursor_byte_range: (usize, usize),
    absolute_start: usize,
) -> TextSelection {
    TextSelection::new(
        absolute_start + byte_to_char_index(text, cursor_byte_range.0),
        absolute_start + byte_to_char_index(text, cursor_byte_range.1),
    )
}

fn byte_to_char_index(text: &str, byte_offset: usize) -> usize {
    let mut offset = byte_offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    text[..offset].chars().count()
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn controller_handles_are_shared() {
//         let controller = TextController::with_text("a好");
//         let other = controller.clone();

//         other.insert_text("🙂").unwrap();

//         assert!(controller.same_handle(&other));
//         assert_eq!(controller.text(), "a好🙂");
//         assert_eq!(controller.selection(), TextSelection::collapsed(3));
//     }

//     #[test]
//     fn replaces_selection_and_backspaces_unicode() {
//         let controller = TextController::with_text("a好🙂");
//         controller.set_selection(TextSelection::new(1, 3)).unwrap();

//         controller.insert_text("界").unwrap();
//         assert_eq!(controller.text(), "a界");
//         assert_eq!(controller.selection(), TextSelection::collapsed(2));

//         assert!(controller.backspace().unwrap().is_some());
//         assert_eq!(controller.text(), "a");
//         assert_eq!(controller.selection(), TextSelection::collapsed(1));
//     }

//     #[test]
//     fn undo_redo_tracks_formal_edits() {
//         let controller = TextController::with_text("ab");
//         controller.insert_text("c").unwrap();
//         controller.backspace().unwrap();

//         assert_eq!(controller.text(), "ab");
//         assert!(controller.can_undo());

//         controller.undo().unwrap();
//         assert_eq!(controller.text(), "abc");

//         controller.undo().unwrap();
//         assert_eq!(controller.text(), "ab");

//         controller.redo().unwrap();
//         assert_eq!(controller.text(), "abc");

//         controller.insert_text("!").unwrap();
//         assert!(!controller.can_redo());
//     }

//     #[test]
//     fn set_text_resets_history() {
//         let controller = TextController::with_text("a");
//         controller.insert_text("b").unwrap();
//         assert!(controller.can_undo());

//         controller.set_text("reset");

//         assert_eq!(controller.text(), "reset");
//         assert!(!controller.can_undo());
//         assert!(!controller.can_redo());
//     }

//     #[test]
//     fn ime_preedit_is_temporary_and_commit_is_undoable() {
//         let controller = TextController::with_text("ab");
//         controller.set_cursor(1).unwrap();
//         let mut ime = ImeSession::new();

//         ime.begin(&controller).unwrap();
//         ime.preedit(&controller, "ni", Some((2, 2))).unwrap();

//         assert_eq!(controller.text(), "anib");
//         assert_eq!(controller.composing(), Some(TextRange::new(1, 3)));
//         assert!(!controller.can_undo());

//         ime.commit(&controller, "你").unwrap();

//         assert_eq!(controller.text(), "a你b");
//         assert_eq!(controller.composing(), None);
//         assert!(controller.can_undo());

//         controller.undo().unwrap();
//         assert_eq!(controller.text(), "ab");
//     }

//     #[test]
//     fn ime_cancel_restores_original_selection_text() {
//         let controller = TextController::with_text("hello");
//         controller.set_selection(TextSelection::new(1, 4)).unwrap();
//         let mut ime = ImeSession::new();

//         ime.begin(&controller).unwrap();
//         ime.preedit(&controller, "X", Some((1, 1))).unwrap();
//         assert_eq!(controller.text(), "hXo");

//         ime.cancel(&controller).unwrap();
//         assert_eq!(controller.text(), "hello");
//         assert_eq!(controller.selection(), TextSelection::new(1, 4));
//         assert!(!controller.can_undo());
//     }

//     #[test]
//     fn ime_preedit_cursor_range_uses_byte_offsets() {
//         let controller = TextController::with_text("");
//         let mut ime = ImeSession::new();

//         ime.begin(&controller).unwrap();
//         ime.preedit(&controller, "你a", Some((3, 4))).unwrap();

//         assert_eq!(ime.preedit_cursor(), Some(TextSelection::new(1, 2)));
//         assert_eq!(controller.selection(), TextSelection::new(1, 2));
//     }
// }
