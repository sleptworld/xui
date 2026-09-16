use std::ops::Range;

use self_cell::self_cell;
use unicode_bidi::BidiInfo;
use xui_interface::TextContent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VisualRun {
    pub range: Range<usize>,
    pub level: u8,
}

#[derive(Debug, Clone)]
pub(crate) struct VisualOrder {
    pub base_rtl: bool,
    pub runs: Vec<VisualRun>,
}

self_cell!(
    struct ResolverCell {
        owner: TextContent,

        #[covariant]
        dependent: BidiInfo,
    }

    impl {Debug}
);

#[derive(Debug)]
pub(crate) struct Resolver {
    cell: ResolverCell,
}

impl Resolver {
    pub fn new(text: &str) -> Self {
        Self::from_content(TextContent::copy_from(text))
    }

    pub fn from_content(text: TextContent) -> Self {
        Self {
            cell: ResolverCell::new(text, |text| BidiInfo::new(text.as_str(), None)),
        }
    }

    pub fn resolve(&self, line: Range<usize>) -> VisualOrder {
        resolve_with(self.cell.borrow_dependent(), line)
    }
}

fn resolve_with(info: &BidiInfo<'_>, line: Range<usize>) -> VisualOrder {
    if line.is_empty() {
        return VisualOrder {
            base_rtl: false,
            runs: Vec::new(),
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
            runs: vec![VisualRun {
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
    fn rtl_paragraph_reports_rtl_base() {
        let text = "שלום עולם";
        assert!(Resolver::new(text).resolve(0..text.len()).base_rtl);
    }
}
