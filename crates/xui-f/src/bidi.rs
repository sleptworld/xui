use std::ops::Range;

use self_cell::self_cell;
use smallvec::{SmallVec, smallvec};
use unicode_bidi::{BidiClass, BidiInfo, bidi_class};
use xui_interface::TextContent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VisualRun {
    pub range: Range<usize>,
    pub level: u8,
}

#[derive(Debug, Clone)]
pub(crate) struct VisualOrder {
    pub base_rtl: bool,
    pub runs: SmallVec<[VisualRun; 1]>,
}

self_cell!(
    struct ResolverCell {
        owner: TextContent,

        #[covariant]
        dependent: BidiInfo,
    }

    impl {Debug}
);

/// Bidi levels of one text. `None` is text that is left-to-right throughout,
/// which needs no analysis.
#[derive(Debug)]
pub(crate) struct Resolver {
    cell: Option<ResolverCell>,
}

impl Resolver {
    pub fn new(text: &str) -> Self {
        Self {
            cell: (!left_to_right(text)).then(|| {
                ResolverCell::new(TextContent::copy_from(text), |text| {
                    BidiInfo::new(text.as_str(), None)
                })
            }),
        }
    }

    pub fn from_content(text: TextContent) -> Self {
        Self {
            cell: (!left_to_right(text.as_str()))
                .then(|| ResolverCell::new(text, |text| BidiInfo::new(text.as_str(), None))),
        }
    }

    pub fn resolve(&self, line: Range<usize>) -> VisualOrder {
        match &self.cell {
            Some(cell) => resolve_with(cell.borrow_dependent(), line),
            None => VisualOrder {
                base_rtl: false,
                runs: if line.is_empty() {
                    SmallVec::new()
                } else {
                    smallvec![VisualRun {
                        range: line,
                        level: 0,
                    }]
                },
            },
        }
    }
}

/// Whether every character of `text` resolves to level 0.
///
/// Without a right-to-left strong character, an Arabic number or an explicit
/// embedding, override or isolate, the paragraph level is 0 and no rule
/// raises any character above it: European numbers follow the paragraph's
/// left-to-right start of sequence (W7), and neutrals sit between
/// left-to-right types (N1). All such characters are outside ASCII.
fn left_to_right(text: &str) -> bool {
    text.is_ascii()
        || !text.chars().any(|character| {
            !character.is_ascii()
                && matches!(
                    bidi_class(character),
                    BidiClass::R
                        | BidiClass::AL
                        | BidiClass::AN
                        | BidiClass::LRE
                        | BidiClass::RLE
                        | BidiClass::LRO
                        | BidiClass::RLO
                        | BidiClass::PDF
                        | BidiClass::LRI
                        | BidiClass::RLI
                        | BidiClass::FSI
                        | BidiClass::PDI
                )
        })
}

fn resolve_with(info: &BidiInfo<'_>, line: Range<usize>) -> VisualOrder {
    if line.is_empty() {
        return VisualOrder {
            base_rtl: false,
            runs: SmallVec::new(),
        };
    }
    let Some(paragraph) = info
        .paragraphs
        .iter()
        .find(|paragraph| line.start >= paragraph.range.start && line.end <= paragraph.range.end)
        .or_else(|| info.paragraphs.first())
    else {
        return VisualOrder {
            base_rtl: false,
            runs: smallvec![VisualRun {
                range: line,
                level: 0,
            }],
        };
    };
    let (levels, ranges) = info.visual_runs(paragraph, line);
    let runs = ranges
        .into_iter()
        .map(|range| VisualRun {
            level: levels.get(range.start).map_or(0, |level| level.number()),
            range,
        })
        .collect();
    VisualOrder {
        base_rtl: paragraph.level.is_rtl(),
        runs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_text_is_split_and_visually_reordered() {
        let text = "abc אבג 123";
        let order = Resolver::new(text).resolve(0..text.len());
        assert!(!order.base_rtl);
        assert!(order.runs.len() >= 3);
        assert!(order.runs.iter().any(|run| run.level % 2 == 1));
        assert_eq!(
            order.runs.iter().map(|run| run.range.len()).sum::<usize>(),
            text.len()
        );
    }

    #[test]
    fn left_to_right_text_matches_full_analysis() {
        for text in [
            "plain ascii 123, (done).",
            "café — naïve 価格 ¥100 €5",
            "a\u{2028}b\nc 中文",
            "",
        ] {
            let fast = Resolver::new(text);
            assert!(fast.cell.is_none(), "{text:?}");
            let full = ResolverCell::new(TextContent::copy_from(text), |text| {
                BidiInfo::new(text.as_str(), None)
            });
            for line in [0..text.len(), 0..text.len() / 2] {
                let line = text.floor_char_boundary(line.start)..text.floor_char_boundary(line.end);
                let expected = resolve_with(full.borrow_dependent(), line.clone());
                let actual = fast.resolve(line);
                assert_eq!(actual.base_rtl, expected.base_rtl, "{text:?}");
                assert_eq!(actual.runs, expected.runs, "{text:?}");
            }
        }
        for text in ["abc אבג", "x \u{0661}\u{0662}", "a\u{2067}b\u{2069}"] {
            assert!(Resolver::new(text).cell.is_some(), "{text:?}");
        }
    }

    #[test]
    fn rtl_paragraph_reports_rtl_base() {
        let text = "שלום עולם";
        assert!(Resolver::new(text).resolve(0..text.len()).base_rtl);
    }
}
