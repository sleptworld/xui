//! Selections: one or more ranges, each with a fixed end and a moving one.

use crate::document::{Pos, ordered};

/// A range from `anchor`, where it started, to `head`, where the caret is.
/// Equal ends are a caret.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    pub anchor: Pos,
    pub head: Pos,
    /// The x a run of up/down moves keeps returning to, in content
    /// coordinates. Cleared by any other move.
    pub goal_x: Option<f32>,
}

impl Selection {
    pub fn caret(at: Pos) -> Self {
        Self {
            anchor: at,
            head: at,
            goal_x: None,
        }
    }

    pub fn new(anchor: Pos, head: Pos) -> Self {
        Self {
            anchor,
            head,
            goal_x: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn start(&self) -> Pos {
        ordered(self.anchor, self.head).0
    }

    pub fn end(&self) -> Pos {
        ordered(self.anchor, self.head).1
    }
}

/// Every selection, in document order and not overlapping, and which one is
/// primary: the one the viewport follows and the IME composes at.
#[derive(Debug, Clone, PartialEq)]
pub struct Selections {
    list: Vec<Selection>,
    primary: usize,
}

impl Selections {
    pub fn single(selection: Selection) -> Self {
        Self {
            list: vec![selection],
            primary: 0,
        }
    }

    pub fn primary(&self) -> Selection {
        self.list[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Selection> {
        self.list.iter()
    }

    /// Builds from `list`, sorting it and merging ranges that overlap.
    /// `primary` indexes `list`.
    pub fn from_list(list: Vec<Selection>, primary: usize) -> Self {
        let mut selections = Self {
            primary: primary.min(list.len().saturating_sub(1)),
            list,
        };
        selections.normalize();
        selections
    }

    /// Adds `selection` and makes it primary.
    pub fn add(&mut self, selection: Selection) {
        self.list.push(selection);
        self.primary = self.list.len() - 1;
        self.normalize();
    }

    /// Only the primary selection.
    pub fn collapse(&mut self) {
        let primary = self.primary();
        *self = Self::single(primary);
    }

    pub(crate) fn map(&mut self, mut map: impl FnMut(Selection) -> Selection) {
        for selection in &mut self.list {
            *selection = map(*selection);
        }
        self.normalize();
    }

    pub(crate) fn list(&self) -> &[Selection] {
        &self.list
    }

    fn normalize(&mut self) {
        if self.list.is_empty() {
            self.list.push(Selection::caret(Pos::default()));
            self.primary = 0;
            return;
        }
        let primary = self.list[self.primary];
        let mut indexed: Vec<(usize, Selection)> = self.list.drain(..).enumerate().collect();
        indexed.sort_by_key(|(_, selection)| (selection.start(), selection.end()));
        let mut merged: Vec<(bool, Selection)> = Vec::with_capacity(indexed.len());
        for (index, selection) in indexed {
            let is_primary = index == self.primary;
            if let Some((was_primary, last)) = merged.last_mut()
                && (selection.start() < last.end()
                    || selection.start() == last.start()
                    || (selection.is_empty() && selection.start() == last.end()))
            {
                // Overlapping, or the same caret twice: one range covering
                // both, facing the way the primary faces.
                let start = last.start().min(selection.start());
                let end = last.end().max(selection.end());
                let forward = if is_primary {
                    primary.head >= primary.anchor
                } else {
                    last.head >= last.anchor
                };
                *last = if forward {
                    Selection::new(start, end)
                } else {
                    Selection::new(end, start)
                };
                *was_primary |= is_primary;
                continue;
            }
            merged.push((is_primary, selection));
        }
        self.primary = merged
            .iter()
            .position(|(is_primary, _)| *is_primary)
            .unwrap_or(0);
        self.list = merged.into_iter().map(|(_, selection)| selection).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_selections_merge_and_keep_the_primary() {
        let selections = Selections::from_list(
            vec![
                Selection::new(Pos::new(0, 5), Pos::new(0, 9)),
                Selection::caret(Pos::new(2, 0)),
                Selection::new(Pos::new(0, 0), Pos::new(0, 6)),
                Selection::caret(Pos::new(2, 0)),
            ],
            2,
        );
        assert_eq!(selections.len(), 2);
        assert_eq!(selections.primary_index(), 0);
        assert_eq!(selections.primary().start(), Pos::new(0, 0));
        assert_eq!(selections.primary().end(), Pos::new(0, 9));
    }
}
