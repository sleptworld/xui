//! Styling a line's text: keywords, strings, numbers, comments.

use rustc_hash::FxHashMap;
use xui_core::prelude::{TextRun, TextRunStyle};

/// Styles one line at a time. Called again for a line only when its text
/// changes, so it may be as slow as a line of text allows. Runs may overlap
/// or come in any order; they are put in order, earlier ones winning.
///
/// Line-at-a-time cannot see a comment or string that spans lines; a
/// highlighter that needs that (tree-sitter, say) keeps its own parse and
/// tells the editor which lines it restyled (`Editor::restyle_lines`).
pub trait Highlighter {
    fn highlight(&self, line: &str) -> Vec<TextRun>;
}

impl<F: Fn(&str) -> Vec<TextRun>> Highlighter for F {
    fn highlight(&self, line: &str) -> Vec<TextRun> {
        self(line)
    }
}

/// A highlighter for C-like languages: keywords by name, quoted strings,
/// numbers, and a line comment marker.
#[derive(Debug, Clone, Default)]
pub struct KeywordHighlighter {
    keywords: FxHashMap<String, TextRunStyle>,
    line_comment: Option<(String, TextRunStyle)>,
    string: Option<TextRunStyle>,
    number: Option<TextRunStyle>,
}

impl KeywordHighlighter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn keywords<'a>(
        mut self,
        words: impl IntoIterator<Item = &'a str>,
        style: TextRunStyle,
    ) -> Self {
        for word in words {
            self.keywords.insert(word.to_owned(), style.clone());
        }
        self
    }

    pub fn line_comment(mut self, marker: impl Into<String>, style: TextRunStyle) -> Self {
        self.line_comment = Some((marker.into(), style));
        self
    }

    pub fn strings(mut self, style: TextRunStyle) -> Self {
        self.string = Some(style);
        self
    }

    pub fn numbers(mut self, style: TextRunStyle) -> Self {
        self.number = Some(style);
        self
    }
}

impl Highlighter for KeywordHighlighter {
    fn highlight(&self, line: &str) -> Vec<TextRun> {
        let mut runs = Vec::new();
        let bytes = line.as_bytes();
        let mut index = 0;
        while index < line.len() {
            let rest = &line[index..];
            if let Some((marker, style)) = &self.line_comment
                && !marker.is_empty()
                && rest.starts_with(marker.as_str())
            {
                runs.push(TextRun::new(index..line.len(), style.clone()));
                break;
            }
            let ch = rest.chars().next().unwrap_or(' ');
            if (ch == '"' || ch == '\'')
                && let Some(style) = &self.string
            {
                let end = string_end(bytes, index, bytes[index]);
                runs.push(TextRun::new(index..end, style.clone()));
                index = end;
                continue;
            }
            if ch.is_alphanumeric() || ch == '_' {
                let end = index
                    + rest
                        .char_indices()
                        .find(|(_, ch)| !(ch.is_alphanumeric() || *ch == '_'))
                        .map_or(rest.len(), |(offset, _)| offset);
                let word = &line[index..end];
                if ch.is_ascii_digit() {
                    if let Some(style) = &self.number {
                        runs.push(TextRun::new(index..end, style.clone()));
                    }
                } else if let Some(style) = self.keywords.get(word) {
                    runs.push(TextRun::new(index..end, style.clone()));
                }
                index = end;
                continue;
            }
            index += ch.len_utf8();
        }
        runs
    }
}

/// The byte after the string opened by `quote` at `start`, or the line's end.
fn string_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            byte if byte == quote => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use xui_core::prelude::Color;

    use super::*;

    #[test]
    fn keywords_strings_numbers_and_comments() {
        let style = |r| TextRunStyle::new().color(Color::rgba(r, 0.0, 0.0, 1.0));
        let highlighter = KeywordHighlighter::new()
            .keywords(["fn", "let"], style(0.1))
            .strings(style(0.2))
            .numbers(style(0.3))
            .line_comment("//", style(0.4));
        let runs = highlighter.highlight(r#"let x = "a\"b" + 42; // fn"#);
        let ranges: Vec<_> = runs.iter().map(|run| run.range.clone()).collect();
        assert_eq!(ranges, [0..3, 8..14, 17..19, 21..26]);
    }
}
