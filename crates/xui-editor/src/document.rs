//! The text: a rope, and a stable block id per line.

use std::cell::RefCell;

use ropey::Rope;
use rustc_hash::FxHashMap;
use unicode_segmentation::UnicodeSegmentation;
use xui_core::prelude::{BlockChange, BlockId};

/// A place in the document: a line, and a byte offset into that line's text
/// (which does not include its line break). Ordered by line, then byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub byte: usize,
}

impl Pos {
    pub const fn new(line: usize, byte: usize) -> Self {
        Self { line, byte }
    }
}

/// Text as a rope of lines. Each line is one block of the surface, with an id
/// that stays the same while the line is edited, so the surface reshapes only
/// the lines that changed. Line breaks are `\n`; `\r\n` and `\r` become `\n`
/// on the way in.
#[derive(Debug, Clone)]
pub struct Document {
    rope: Rope,
    /// The id of each line, in order.
    ids: Vec<BlockId>,
    /// Where each id is in `ids`, rebuilt after lines are added or removed.
    index: RefCell<Option<FxHashMap<BlockId, usize>>>,
    next_id: u64,
}

impl Document {
    pub fn new(text: &str) -> Self {
        let rope = Rope::from_str(&normalize_breaks(text));
        let lines = rope.len_lines();
        Self {
            rope,
            ids: (0..lines as u64).map(BlockId).collect(),
            index: RefCell::new(None),
            next_id: lines as u64,
        }
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// At least one: an empty document is one empty line.
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Line `line`'s text, without its line break.
    pub fn line(&self, line: usize) -> String {
        let mut text = self.rope.line(line).to_string();
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    /// The length of line `line` in bytes, without its line break.
    pub fn line_len(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        let len = slice.len_bytes();
        if len > 0 && slice.byte(len - 1) == b'\n' {
            len - 1
        } else {
            len
        }
    }

    pub fn id(&self, line: usize) -> BlockId {
        self.ids[line]
    }

    /// The line `id` is now, if it is still in the document.
    pub fn line_of(&self, id: BlockId) -> Option<usize> {
        let mut index = self.index.borrow_mut();
        let index = index.get_or_insert_with(|| {
            self.ids
                .iter()
                .enumerate()
                .map(|(line, id)| (*id, line))
                .collect()
        });
        index.get(&id).copied()
    }

    pub fn start(&self) -> Pos {
        Pos::new(0, 0)
    }

    pub fn end(&self) -> Pos {
        let line = self.line_count() - 1;
        Pos::new(line, self.line_len(line))
    }

    /// `pos` moved onto the document: onto a line that exists, inside it, and
    /// onto a character boundary.
    pub fn clamp(&self, pos: Pos) -> Pos {
        let line = pos.line.min(self.line_count() - 1);
        let text = self.line(line);
        let mut byte = pos.byte.min(text.len());
        while !text.is_char_boundary(byte) {
            byte -= 1;
        }
        Pos::new(line, byte)
    }

    /// `pos` as a byte offset into the whole text.
    pub fn offset(&self, pos: Pos) -> usize {
        let pos = self.clamp(pos);
        self.rope.line_to_byte(pos.line) + pos.byte
    }

    /// The position of byte `offset` of the whole text.
    pub fn pos(&self, offset: usize) -> Pos {
        let offset = offset.min(self.rope.len_bytes());
        let line = self.rope.byte_to_line(offset);
        Pos::new(line, offset - self.rope.line_to_byte(line))
    }

    /// The text from `start` to `end`, in either order.
    pub fn slice(&self, start: Pos, end: Pos) -> String {
        let (start, end) = ordered(start, end);
        let (start, end) = (self.offset(start), self.offset(end));
        let (start, end) = (self.rope.byte_to_char(start), self.rope.byte_to_char(end));
        self.rope.slice(start..end).to_string()
    }

    /// The position one grapheme before `pos`: the end of the line above at a
    /// line's start, and `pos` itself at the document's start.
    pub fn prev_grapheme(&self, pos: Pos) -> Pos {
        let pos = self.clamp(pos);
        if pos.byte == 0 {
            return if pos.line == 0 {
                pos
            } else {
                Pos::new(pos.line - 1, self.line_len(pos.line - 1))
            };
        }
        let text = self.line(pos.line);
        let byte = text[..pos.byte]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(byte, _)| byte);
        Pos::new(pos.line, byte)
    }

    /// The position one grapheme after `pos`, across line breaks.
    pub fn next_grapheme(&self, pos: Pos) -> Pos {
        let pos = self.clamp(pos);
        let text = self.line(pos.line);
        if pos.byte >= text.len() {
            return if pos.line + 1 >= self.line_count() {
                pos
            } else {
                Pos::new(pos.line + 1, 0)
            };
        }
        let byte = text[pos.byte..]
            .grapheme_indices(true)
            .nth(1)
            .map_or(text.len(), |(byte, _)| pos.byte + byte);
        Pos::new(pos.line, byte)
    }

    /// The start of the word before `pos`, skipping whitespace and
    /// punctuation in between; the end of the line above at a line's start.
    pub fn prev_word(&self, pos: Pos) -> Pos {
        let pos = self.clamp(pos);
        if pos.byte == 0 {
            return self.prev_grapheme(pos);
        }
        let text = self.line(pos.line);
        let byte = text[..pos.byte]
            .split_word_bound_indices()
            .rev()
            .find(|(_, word)| is_word(word))
            .map_or(0, |(byte, _)| byte);
        Pos::new(pos.line, byte)
    }

    /// The end of the word after `pos`; the start of the line below at a
    /// line's end.
    pub fn next_word(&self, pos: Pos) -> Pos {
        let pos = self.clamp(pos);
        let text = self.line(pos.line);
        if pos.byte >= text.len() {
            return self.next_grapheme(pos);
        }
        let byte = text[pos.byte..]
            .split_word_bound_indices()
            .find(|(_, word)| is_word(word))
            .map_or(text.len(), |(byte, word)| pos.byte + byte + word.len());
        Pos::new(pos.line, byte)
    }

    /// The word around `pos`, or the single run of non-word characters there.
    pub fn word_at(&self, pos: Pos) -> (Pos, Pos) {
        let pos = self.clamp(pos);
        let text = self.line(pos.line);
        for (byte, word) in text.split_word_bound_indices() {
            if (byte..byte + word.len()).contains(&pos.byte) {
                return (
                    Pos::new(pos.line, byte),
                    Pos::new(pos.line, byte + word.len()),
                );
            }
        }
        (pos, pos)
    }

    /// The leading whitespace of line `line`.
    pub fn indent(&self, line: usize) -> String {
        self.line(line)
            .chars()
            .take_while(|ch| *ch == ' ' || *ch == '\t')
            .collect()
    }

    /// Replaces `start..end` with `text`, records the block changes that
    /// makes in `changes`, and returns where the inserted text ends.
    pub(crate) fn replace(
        &mut self,
        start: Pos,
        end: Pos,
        text: &str,
        changes: &mut Vec<BlockChange>,
    ) -> Pos {
        let (start, end) = ordered(self.clamp(start), self.clamp(end));
        let text = normalize_breaks(text);
        let from = self.rope.byte_to_char(self.offset(start));
        let to = self.rope.byte_to_char(self.offset(end));
        self.rope.remove(from..to);
        self.rope.insert(from, &text);

        // The first line keeps its id; the lines the range covered after it go,
        // and each new line break makes a line after it.
        for id in self.ids.drain(start.line + 1..end.line + 1) {
            changes.push(BlockChange::Removed { id });
        }
        changes.push(BlockChange::Text {
            id: self.ids[start.line],
        });
        let breaks = text.matches('\n').count();
        if breaks > 0 {
            let first = self.next_id;
            self.next_id += breaks as u64;
            let new_ids = (first..first + breaks as u64).map(BlockId);
            self.ids
                .splice(start.line + 1..start.line + 1, new_ids.clone());
            for (offset, id) in new_ids.enumerate() {
                changes.push(BlockChange::Inserted {
                    index: start.line + 1 + offset,
                    id,
                });
            }
        }
        if breaks > 0 || end.line > start.line {
            *self.index.get_mut() = None;
        }
        end_of(start, &text)
    }
}

/// Where `text` ends when it is inserted at `start`.
pub(crate) fn end_of(start: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        None => Pos::new(start.line, start.byte + text.len()),
        Some(last) => Pos::new(
            start.line + text.matches('\n').count(),
            text.len() - last - 1,
        ),
    }
}

pub(crate) fn ordered(a: Pos, b: Pos) -> (Pos, Pos) {
    if a <= b { (a, b) } else { (b, a) }
}

pub(crate) fn normalize_breaks(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n").into()
    } else {
        text.into()
    }
}

fn is_word(text: &str) -> bool {
    text.chars().any(|ch| ch.is_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_keeps_the_first_line_id_and_reports_each_block() {
        let mut doc = Document::new("one\ntwo\nthree");
        let (one, two, three) = (doc.id(0), doc.id(1), doc.id(2));
        let mut changes = Vec::new();
        // "one" ends, "two" goes, "three" loses "th": "onAB\nCDree".
        let end = doc.replace(Pos::new(0, 2), Pos::new(2, 2), "AB\nCD", &mut changes);
        assert_eq!(doc.text(), "onAB\nCDree");
        assert_eq!(end, Pos::new(1, 2));
        assert_eq!(doc.id(0), one);
        assert_eq!(doc.line_count(), 2);
        let new = doc.id(1);
        assert_eq!(
            changes,
            [
                BlockChange::Removed { id: two },
                BlockChange::Removed { id: three },
                BlockChange::Text { id: one },
                BlockChange::Inserted { index: 1, id: new },
            ]
        );
        assert_eq!(doc.line_of(new), Some(1));
        assert_eq!(doc.line_of(two), None);
    }

    #[test]
    fn breaks_are_normalized_and_lines_exclude_them() {
        let doc = Document::new("a\r\nb\rc\n");
        assert_eq!(doc.line_count(), 4);
        assert_eq!(doc.line(0), "a");
        assert_eq!(doc.line_len(2), 1);
        assert_eq!(doc.line(3), "");
        assert_eq!(doc.end(), Pos::new(3, 0));
    }

    #[test]
    fn motions_cross_lines_and_respect_graphemes() {
        let doc = Document::new("aé👍🏽 b\nfoo_bar baz");
        assert_eq!(doc.next_grapheme(Pos::new(0, 1)), Pos::new(0, 3));
        // The thumbs up with its skin tone is one grapheme of 8 bytes.
        assert_eq!(doc.next_grapheme(Pos::new(0, 3)), Pos::new(0, 11));
        assert_eq!(doc.prev_grapheme(Pos::new(1, 0)), Pos::new(0, 13));
        assert_eq!(doc.next_word(Pos::new(1, 0)), Pos::new(1, 7));
        assert_eq!(doc.prev_word(Pos::new(1, 11)), Pos::new(1, 8));
        assert_eq!(
            doc.word_at(Pos::new(1, 2)),
            (Pos::new(1, 0), Pos::new(1, 7))
        );
    }
}
