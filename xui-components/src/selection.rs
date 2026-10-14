//! Reusable enabled-item navigation and buffered text matching.
//!
//! [`SelectionModel`] normalizes indices, skips disabled choices, supports optional
//! wrapping, and caches case-folded labels. [`TypeaheadState`] buffers text with an
//! expiry window. [`SelectionOrientation`] and [`SelectionActivationMode`] configure
//! component keyboard behavior.

use std::time::{Duration, Instant};

/// The axis used for layout and directional keyboard navigation.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum SelectionOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Whether keyboard focus automatically activates a choice or waits for activation.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum SelectionActivationMode {
    #[default]
    Automatic,
    Manual,
}

/// An identified navigation choice with a searchable label and disabled state.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct SelectionItem {
    pub id: String,
    pub label: String,
    pub disabled: bool,
}

impl SelectionItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>, disabled: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled,
        }
    }
}

/// Enabled-item navigation and case-folded label matching with optional wrapping.
#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct SelectionModel {
    items: Vec<SelectionItem>,
    wrap: bool,
    normalized_labels: Vec<String>,
}

impl SelectionModel {
    pub fn new(items: impl IntoIterator<Item = SelectionItem>) -> Self {
        let items: Vec<SelectionItem> = items.into_iter().collect();
        let normalized_labels = items.iter().map(|item| item.label.to_lowercase()).collect();
        Self {
            items,
            normalized_labels,
            wrap: true,
        }
    }

    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    pub fn normalize(&self, requested: usize) -> Option<usize> {
        self.items
            .get(requested)
            .filter(|item| !item.disabled)
            .map(|_| requested)
            .or_else(|| self.first())
    }

    pub fn first(&self) -> Option<usize> {
        self.items.iter().position(|item| !item.disabled)
    }

    pub fn last(&self) -> Option<usize> {
        self.items.iter().rposition(|item| !item.disabled)
    }

    pub fn adjacent(&self, current: usize, direction: isize) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        let mut candidate = current.min(self.items.len() - 1);
        for _ in 0..self.items.len() {
            candidate = if direction < 0 {
                match candidate.checked_sub(1) {
                    Some(index) => index,
                    None if self.wrap => self.items.len() - 1,
                    None => return None,
                }
            } else if candidate + 1 < self.items.len() {
                candidate + 1
            } else if self.wrap {
                0
            } else {
                return None;
            };
            if !self.items[candidate].disabled {
                return Some(candidate);
            }
        }
        None
    }

    pub fn typeahead(&self, query: &str, after: Option<usize>) -> Option<usize> {
        let query = query.trim().to_lowercase();
        if query.is_empty() || self.items.is_empty() {
            return None;
        }
        let start = after.map_or(0, |index| (index % self.items.len() + 1) % self.items.len());
        (0..self.items.len())
            .map(|offset| (start + offset) % self.items.len())
            .find(|index| {
                let item = &self.items[*index];
                !item.disabled && self.normalized_labels[*index].starts_with(&query)
            })
    }
}

/// A time-limited text buffer for successive typeahead keystrokes.
#[derive(Clone, Debug, Default)]
pub struct TypeaheadState {
    query: String,
    last_input: Option<Instant>,
}

impl TypeaheadState {
    pub const TIMEOUT: Duration = Duration::from_millis(500);

    pub fn push(
        &mut self,
        model: &SelectionModel,
        text: &str,
        current: Option<usize>,
        now: Instant,
    ) -> Option<usize> {
        if self
            .last_input
            .is_none_or(|last| now.saturating_duration_since(last) > Self::TIMEOUT)
        {
            self.query.clear();
        }
        self.query.push_str(text);
        self.last_input = Some(now);
        model.typeahead(&self.query, current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> SelectionModel {
        SelectionModel::new([
            SelectionItem::new("a", "Alpha", false),
            SelectionItem::new("b", "Beta", true),
            SelectionItem::new("c", "Charlie", false),
        ])
    }

    #[test]
    fn navigation_wraps_and_skips_disabled_items() {
        let model = model();
        assert_eq!(model.adjacent(0, 1), Some(2));
        assert_eq!(model.adjacent(2, 1), Some(0));
        assert_eq!(model.adjacent(0, -1), Some(2));
    }

    #[test]
    fn typeahead_is_case_insensitive_and_skips_disabled_items() {
        let model = model();
        assert_eq!(model.typeahead("cH", Some(0)), Some(2));
        assert_eq!(model.typeahead("be", None), None);
    }

    #[test]
    fn typeahead_buffer_expires() {
        let model = model();
        let start = Instant::now();
        let mut state = TypeaheadState::default();
        assert_eq!(state.push(&model, "a", None, start), Some(0));
        assert_eq!(
            state.push(&model, "c", None, start + TypeaheadState::TIMEOUT),
            None
        );
        assert_eq!(
            state.push(
                &model,
                "c",
                None,
                start
                    + TypeaheadState::TIMEOUT
                    + TypeaheadState::TIMEOUT
                    + Duration::from_millis(1)
            ),
            Some(2)
        );
    }
}

pub(crate) const PREVIOUS_ITEM: xui_core::prelude::CommandId =
    xui_core::prelude::CommandId("xui.selection.previous");
pub(crate) const NEXT_ITEM: xui_core::prelude::CommandId =
    xui_core::prelude::CommandId("xui.selection.next");
pub(crate) const FIRST_ITEM: xui_core::prelude::CommandId =
    xui_core::prelude::CommandId("xui.selection.first");
pub(crate) const LAST_ITEM: xui_core::prelude::CommandId =
    xui_core::prelude::CommandId("xui.selection.last");
pub(crate) fn selection_shortcuts()
-> [(xui_core::prelude::Shortcut, xui_core::prelude::CommandId); 4] {
    use xui_core::prelude::*;
    [
        (
            Shortcut::named(NamedKey::ArrowUp).allow_repeat(true),
            PREVIOUS_ITEM,
        ),
        (
            Shortcut::named(NamedKey::ArrowDown).allow_repeat(true),
            NEXT_ITEM,
        ),
        (Shortcut::named(NamedKey::Home), FIRST_ITEM),
        (Shortcut::named(NamedKey::End), LAST_ITEM),
    ]
}
pub(crate) fn selection_target(
    model: &SelectionModel,
    index: usize,
    command: xui_core::prelude::CommandId,
) -> Option<usize> {
    match command {
        PREVIOUS_ITEM => model.adjacent(index, -1),
        NEXT_ITEM => model.adjacent(index, 1),
        FIRST_ITEM => model.first(),
        LAST_ITEM => model.last(),
        _ => None,
    }
}
