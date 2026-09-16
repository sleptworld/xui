use std::{
    ops::{Bound, Range, RangeBounds},
    sync::Arc,
};

use ropey::{Rope, RopeSlice};
use smallvec::{SmallVec, smallvec};

#[derive(Debug, Clone)]
pub struct TextEditingValue {
    pub text: Rope,
    pub selection: TextSelection,
    pub composing: Option<TextRange>,
    pub(crate) revision: u64,
}

impl Default for TextEditingValue {
    fn default() -> Self {
        Self {
            text: Rope::new(),
            selection: TextSelection::collapsed(0),
            composing: None,
            revision: 0,
        }
    }
}

impl TextEditingValue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_text(text: impl AsRef<str>) -> Self {
        let text = text.as_ref();
        Self {
            text: Rope::from_str(text),
            selection: TextSelection::collapsed(text.chars().count()),
            composing: None,
            revision: 0,
        }
    }

    pub fn text(&self) -> String {
        self.text.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.text.len_chars()
    }

    pub fn is_empty(&self) -> bool {
        self.text.len_chars() == 0
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn slice(&self, range: impl RangeBounds<usize>) -> Result<RopeSlice<'_>, SliceError> {
        let range = TextRange::from_bounds(range, self.len_chars()).map_err(SliceError::from)?;
        Ok(self.text.slice(range.as_range()))
    }

    pub fn apply_change_set(
        &mut self,
        change_set: TextChangeSet,
        selection_after: Option<TextSelection>,
        composing_after: Option<Option<TextRange>>,
    ) -> Result<AppliedChangeSet, TextEditError> {
        let old_len = self.len_chars();
        change_set.validate(old_len)?;

        let old_selection = self.selection;
        let old_composing = self.composing;
        let mut inverse_changes = SmallVec::with_capacity(change_set.changes.len());
        let mut segments = SmallVec::with_capacity(change_set.changes.len());
        let mut delta: isize = 0;

        for change in &change_set.changes {
            if change.is_noop() {
                continue;
            }

            let old_text: Arc<str> = self.text.slice(change.range.as_range()).to_string().into();
            let inserted_len = change.inserted_len_chars();
            let new_start = change.range.start.saturating_add_signed(delta);
            let new_end = new_start + inserted_len;

            inverse_changes.push(TextChange {
                range: TextRange::new(new_start, new_end),
                insert: Some(old_text),
            });

            segments.push(ChangeSegment {
                old_range: change.range,
                new_range: TextRange::new(new_start, new_end),
            });

            delta += inserted_len as isize - change.range.len() as isize;
        }

        let new_len = old_len.saturating_add_signed(delta);
        let desc = ChangeDesc {
            base_len: old_len,
            new_len,
            segments,
        };

        let final_selection = selection_after.unwrap_or_else(|| desc.map_selection(old_selection));
        final_selection.validate(new_len)?;

        let final_composing = composing_after.unwrap_or_else(|| desc.map_composing(old_composing));
        if let Some(range) = final_composing {
            range
                .validate(new_len)
                .map_err(|_| TextEditError::InvalidComposing {
                    range,
                    len: new_len,
                })?;
        }

        for change in change_set.changes.iter().rev() {
            if change.is_noop() {
                continue;
            }

            self.text.remove(change.range.as_range());
            if let Some(ref insert) = change.insert
                && !insert.is_empty()
            {
                self.text.insert(change.range.start, insert);
            }
        }

        self.selection = final_selection;
        self.composing = final_composing;

        if !change_set.is_empty()
            || old_selection != self.selection
            || old_composing != self.composing
        {
            self.revision = self.revision.wrapping_add(1);
        }

        let inverse = TextChangeSet {
            base_len: new_len,
            changes: inverse_changes,
        };

        Ok(AppliedChangeSet {
            inverse,
            desc,
            old_selection,
            new_selection: self.selection,
            old_composing,
            new_composing: self.composing,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceError {
    InvalidRange,
    OutOfBounds { range: TextRange, len: usize },
}

impl From<TextEditError> for SliceError {
    fn from(error: TextEditError) -> Self {
        match error {
            TextEditError::InvalidRange { .. } => Self::InvalidRange,
            TextEditError::OutOfBounds { range, len } => Self::OutOfBounds { range, len },
            _ => Self::InvalidRange,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppliedChangeSet {
    pub inverse: TextChangeSet,
    pub desc: ChangeDesc,
    pub old_selection: TextSelection,
    pub new_selection: TextSelection,
    pub old_composing: Option<TextRange>,
    pub new_composing: Option<TextRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

impl TextRange {
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end);
        Self { start, end }
    }

    pub fn collapsed(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    pub fn from_bounds(range: impl RangeBounds<usize>, len: usize) -> Result<Self, TextEditError> {
        let start = match range.start_bound() {
            Bound::Included(start) => *start,
            Bound::Excluded(start) => start.saturating_add(1),
            Bound::Unbounded => 0,
        };

        let end = match range.end_bound() {
            Bound::Included(end) => end.saturating_add(1),
            Bound::Excluded(end) => *end,
            Bound::Unbounded => len,
        };

        let range = Self { start, end };
        range.validate(len)?;
        Ok(range)
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn contains(&self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    pub fn as_range(&self) -> Range<usize> {
        self.start..self.end
    }

    pub fn validate(&self, len: usize) -> Result<(), TextEditError> {
        if self.start > self.end {
            return Err(TextEditError::InvalidRange { range: *self });
        }

        if self.end > len {
            return Err(TextEditError::OutOfBounds { range: *self, len });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSelection {
    pub base: usize,
    pub extent: usize,
}

impl TextSelection {
    pub fn new(base: usize, extent: usize) -> Self {
        Self { base, extent }
    }

    pub fn collapsed(offset: usize) -> Self {
        Self {
            base: offset,
            extent: offset,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.base == self.extent
    }

    pub fn range(&self) -> TextRange {
        TextRange {
            start: self.base.min(self.extent),
            end: self.base.max(self.extent),
        }
    }

    pub fn start(&self) -> usize {
        self.base.min(self.extent)
    }

    pub fn end(&self) -> usize {
        self.base.max(self.extent)
    }

    pub fn validate(&self, len: usize) -> Result<(), TextEditError> {
        if self.base > len || self.extent > len {
            return Err(TextEditError::InvalidSelection {
                selection: *self,
                len,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChange {
    pub range: TextRange,
    pub insert: Option<Arc<str>>,
}

impl TextChange {
    pub fn insert(offset: usize, text: impl Into<Arc<str>>) -> Self {
        Self {
            range: TextRange::collapsed(offset),
            insert: Some(text.into()),
        }
    }

    pub fn delete(range: TextRange) -> Self {
        Self {
            range,
            insert: None,
        }
    }

    pub fn replace(range: TextRange, text: impl Into<Arc<str>>) -> Self {
        Self {
            range,
            insert: Some(text.into()),
        }
    }

    pub fn inserted_len_chars(&self) -> usize {
        self.insert
            .as_ref()
            .map(|text| text.chars().count())
            .unwrap_or_default()
    }

    pub fn old_len_chars(&self) -> usize {
        self.range.len()
    }

    pub fn is_noop(&self) -> bool {
        self.range.is_empty() && self.insert.as_ref().is_none_or(|text| text.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChangeSet {
    pub base_len: usize,
    pub changes: SmallVec<[TextChange; 20]>,
}

impl TextChangeSet {
    pub fn new(base_len: usize, changes: SmallVec<[TextChange; 20]>) -> Self {
        Self { base_len, changes }
    }

    pub fn insert(base_len: usize, offset: usize, text: impl Into<Arc<str>>) -> Self {
        Self {
            base_len,
            changes: smallvec![TextChange::insert(offset, text)],
        }
    }

    pub fn delete(base_len: usize, range: TextRange) -> Self {
        Self {
            base_len,
            changes: smallvec![TextChange::delete(range)],
        }
    }

    pub fn replace(base_len: usize, range: TextRange, text: impl Into<Arc<str>>) -> Self {
        Self {
            base_len,
            changes: smallvec![TextChange::replace(range, text)],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.changes.iter().all(TextChange::is_noop)
    }

    pub fn validate(&self, actual_len: usize) -> Result<(), TextEditError> {
        if self.base_len != actual_len {
            return Err(TextEditError::BaseLenMismatch {
                expected: self.base_len,
                actual: actual_len,
            });
        }

        let mut last_end = 0;

        for change in &self.changes {
            change.range.validate(actual_len)?;

            if change.range.start < last_end {
                return Err(TextEditError::OverlappingChanges);
            }

            last_end = change.range.end;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEditError {
    BaseLenMismatch {
        expected: usize,
        actual: usize,
    },
    InvalidRange {
        range: TextRange,
    },
    OutOfBounds {
        range: TextRange,
        len: usize,
    },
    InvalidSelection {
        selection: TextSelection,
        len: usize,
    },
    InvalidComposing {
        range: TextRange,
        len: usize,
    },
    OverlappingChanges,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeDesc {
    pub base_len: usize,
    pub new_len: usize,
    pub segments: SmallVec<[ChangeSegment; 20]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeSegment {
    pub old_range: TextRange,
    pub new_range: TextRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affinity {
    Before,
    After,
}

impl ChangeDesc {
    pub fn map_offset(&self, offset: usize, affinity: Affinity) -> usize {
        let mut diff: isize = 0;

        for segment in &self.segments {
            let old_start = segment.old_range.start;
            let old_end = segment.old_range.end;
            let old_len = segment.old_range.len();
            let new_len = segment.new_range.len();

            if offset < old_start {
                break;
            }

            if offset > old_end {
                diff += new_len as isize - old_len as isize;
                continue;
            }

            return match affinity {
                Affinity::Before => segment.new_range.start,
                Affinity::After => segment.new_range.end,
            };
        }

        offset.saturating_add_signed(diff)
    }

    pub fn map_range(&self, range: TextRange) -> TextRange {
        TextRange {
            start: self.map_offset(range.start, Affinity::Before),
            end: self.map_offset(range.end, Affinity::After),
        }
    }

    pub fn map_selection(&self, selection: TextSelection) -> TextSelection {
        if selection.is_collapsed() {
            TextSelection::collapsed(self.map_offset(selection.extent, Affinity::After))
        } else {
            TextSelection {
                base: self.map_offset(selection.base, Affinity::Before),
                extent: self.map_offset(selection.extent, Affinity::After),
            }
        }
    }

    pub fn map_composing(&self, composing: Option<TextRange>) -> Option<TextRange> {
        composing.map(|range| self.map_range(range))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_unicode_changes_by_char_index() {
        let mut value = TextEditingValue::with_text("a好c");
        let applied = value
            .apply_change_set(
                TextChangeSet::replace(value.len_chars(), TextRange::new(1, 2), "🙂界"),
                None,
                None,
            )
            .unwrap();

        assert_eq!(value.text(), "a🙂界c");
        assert_eq!(value.selection, TextSelection::collapsed(4));
        assert_eq!(applied.inverse.base_len, 4);

        value
            .apply_change_set(applied.inverse, Some(applied.old_selection), Some(None))
            .unwrap();
        assert_eq!(value.text(), "a好c");
    }

    #[test]
    fn rejects_invalid_change_sets() {
        let mut value = TextEditingValue::with_text("abc");

        assert!(matches!(
            value.apply_change_set(TextChangeSet::insert(2, 0, "x"), None, None),
            Err(TextEditError::BaseLenMismatch { .. })
        ));

        assert!(matches!(
            value.apply_change_set(TextChangeSet::delete(3, TextRange::new(2, 4)), None, None),
            Err(TextEditError::OutOfBounds { .. })
        ));

        assert!(matches!(
            value.apply_change_set(
                TextChangeSet::new(
                    3,
                    smallvec![
                        TextChange::delete(TextRange::new(0, 2)),
                        TextChange::delete(TextRange::new(1, 3)),
                    ],
                ),
                None,
                None,
            ),
            Err(TextEditError::OverlappingChanges)
        ));
    }

    #[test]
    fn maps_selection_and_composing_after_change() {
        let mut value = TextEditingValue::with_text("abcd");
        value.selection = TextSelection::new(1, 3);
        value.composing = Some(TextRange::new(2, 4));

        value
            .apply_change_set(TextChangeSet::insert(4, 1, "XYZ"), None, None)
            .unwrap();

        assert_eq!(value.text(), "aXYZbcd");
        assert_eq!(value.selection, TextSelection::new(1, 6));
        assert_eq!(value.composing, Some(TextRange::new(5, 7)));
    }
}
