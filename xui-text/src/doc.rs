use crate::{
    fontique_library::FamilyList,
    span::{Span, SpanElement},
};
use std::borrow::{Borrow, Cow};
use swash::{Setting, Stretch, Style, Weight, text::Language};

pub type Features<'a> = Cow<'a, [Setting<u16>]>;
pub type Variations<'a> = Cow<'a, [Setting<f32>]>;

#[derive(Default)]
pub struct Doc<'a> {
    pub(super) spans: Vec<Span<'a>>,
    pub(super) fragments: Vec<(usize, usize)>,
    pub(super) roots: Vec<usize>,
    pub(super) text: String,
}

impl std::fmt::Display for Doc<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, root_span) in self.roots.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            self.format_span(f, *root_span, 0)?;
        }
        Ok(())
    }
}

impl<'a> Doc<'a> {
    pub fn builder() -> DocumentBuilder<'a> {
        DocumentBuilder::default()
    }

    pub fn simple<I>(properties: I, text: &str) -> Self
    where
        I: IntoIterator,
        I::Item: Borrow<SpanStyle<'a>>,
    {
        let mut builder = DocumentBuilder::default();
        builder.enter_span(properties);
        builder.add_text(text);
        builder.leave_span();
        builder.build()
    }

    fn format_span(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        span_index: usize,
        depth: usize,
    ) -> std::fmt::Result {
        let span = &self.spans[span_index];
        let indent = "  ".repeat(depth);

        // Format span properties
        write!(f, "{}Span", indent)?;
        if !span.properties.is_empty() {
            write!(f, " [")?;
            for (i, prop) in span.properties.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                self.format_property(f, prop)?;
            }
            write!(f, "]")?;
        }

        if span.elements.is_empty() {
            writeln!(f, " {{}}")?;
            return Ok(());
        }

        writeln!(f, " {{")?;

        // Format span elements
        for element in &span.elements {
            match element {
                SpanElement::Fragment(index) => {
                    let (start, end) = self.fragments[*index];
                    let text = &self.text[start..end];
                    writeln!(f, "{}  Text: {:?}", indent, text)?;
                }
                SpanElement::Span(index) => {
                    self.format_span(f, *index, depth + 1)?;
                }
            }
        }

        writeln!(f, "{}}}", indent)?;
        Ok(())
    }

    fn format_property(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        prop: &SpanStyle,
    ) -> std::fmt::Result {
        match prop {
            SpanStyle::FamilyList(families) => {
                write!(f, "font-family: [")?;
                for (i, family) in families.families().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", family)?;
                }
                write!(f, "]")?;
            }
            SpanStyle::Size(size) => write!(f, "font-size: {}px", size)?,
            SpanStyle::Stretch(stretch) => write!(f, "font-stretch: {:?}", stretch)?,
            SpanStyle::Weight(weight) => write!(f, "font-weight: {:?}", weight)?,
            SpanStyle::Style(style) => write!(f, "font-style: {:?}", style)?,
            SpanStyle::Features(features) => {
                write!(f, "font-features: [")?;
                for (i, feature) in features.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", feature)?;
                }
                write!(f, "]")?;
            }
            SpanStyle::Variations(vars) => {
                write!(f, "font-variations: [")?;
                for (i, var) in vars.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", var)?;
                }
                write!(f, "]")?;
            }
            SpanStyle::Language(lang) => write!(f, "lang: {:?}", lang)?,
            SpanStyle::Direction(dir) => write!(f, "direction: {:?}", dir)?,
            SpanStyle::LetterSpacing(spacing) => write!(f, "letter-spacing: {}px", spacing)?,
            SpanStyle::WordSpacing(spacing) => write!(f, "word-spacing: {}px", spacing)?,
            SpanStyle::LineSpacing(spacing) => write!(f, "line-spacing: {}px", spacing)?,
            SpanStyle::Underline(enabled) => write!(f, "underline: {}", enabled)?,
            SpanStyle::UnderlineOffset(offset) => match offset {
                Some(val) => write!(f, "underline-offset: {}px", val)?,
                None => write!(f, "underline-offset: auto")?,
            },
            SpanStyle::UnderlineSize(size) => match size {
                Some(val) => write!(f, "underline-size: {}px", val)?,
                None => write!(f, "underline-size: auto")?,
            },
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum SpanStyle<'a> {
    FamilyList(FamilyList),
    Size(f32),
    Stretch(Stretch),
    Weight(Weight),
    Style(Style),
    Features(Features<'a>),
    Variations(Variations<'a>),
    Language(Language),
    Direction(Direction),
    LetterSpacing(f32),
    WordSpacing(f32),
    LineSpacing(f32),
    Underline(bool),
    UnderlineOffset(Option<f32>),
    UnderlineSize(Option<f32>),
}

impl SpanStyle<'_> {
    pub fn same_kind(&self, other: &Self) -> bool {
        use std::mem::discriminant;
        discriminant(self) == discriminant(other)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Auto,
    Ltr,
    Rtl,
}

#[derive(Default)]
pub struct DocumentBuilder<'a> {
    doc: Doc<'a>,
    spans: Vec<usize>,
}

impl<'a> DocumentBuilder<'a> {
    pub fn enter_span<I>(&mut self, properties: I) -> usize
    where
        I: IntoIterator,
        I::Item: Borrow<SpanStyle<'a>>,
    {
        let span = Span {
            properties: properties
                .into_iter()
                .map(|p| p.borrow().to_owned())
                .collect(),
            elements: Vec::new(),
        };
        let index = self.doc.spans.len();
        self.doc.spans.push(span);
        if let Some(parent) = self.spans.last() {
            self.doc.spans[*parent]
                .elements
                .push(SpanElement::Span(index));
        } else {
            self.doc.roots.push(index);
        }
        self.spans.push(index);
        index
    }

    pub fn leave_span(&mut self) {
        self.spans.pop();
    }

    pub fn add_text(&mut self, text: &str) {
        if let Some(span) = self.spans.last() {
            let index = self.doc.fragments.len();
            let start = self.doc.text.len();
            self.doc.text.push_str(text);
            let end = self.doc.text.len();
            self.doc.fragments.push((start, end));
            self.doc.spans[*span]
                .elements
                .push(SpanElement::Fragment(index));
        }
    }

    pub fn build(self) -> Doc<'a> {
        self.doc
    }
}

#[cfg(test)]
mod test {
    use crate::{
        doc::{Doc, SpanStyle},
        fontique_library::FamilyList,
    };

    #[test]
    fn test() {
        let properties = &[
            SpanStyle::FamilyList(FamilyList::new("pingfang sc")),
            SpanStyle::LineSpacing(1.25),
        ];
        let doc = Doc::simple(properties, "Hello, World!");
        println!("{}", doc);
    }
}
