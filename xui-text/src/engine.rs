use std::collections::HashMap;
use std::sync::Arc;

use swash::shape::ShapeContext;
use swash::{Style, Weight};
use xui_interface::{
    Color, ComputedTextStyle, FontFamily, FontStyle as XuiFontStyle, FontWeight as XuiFontWeight,
    LineHeight, Size, TextDecoration, TextLayoutConstraints, TextMeasurer,
};

use crate::{
    bidi::BidiResolver,
    doc::{Direction, Doc, SpanStyle},
    library::{FamilyList, FontContext},
    line_breaker::Alignment,
    par::{BuilderState, Par, Session},
    span::{Span, SpanElement},
};

pub trait TextLayouter: TextMeasurer {
    fn layout_text(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Arc<Par>;
}

pub struct Engine {
    pub(crate) font_ctx: FontContext,
    pub(crate) bidi: BidiResolver,
    pub(crate) scx: ShapeContext,
    pub(crate) state: BuilderState,
    layout_cache: HashMap<TextLayoutKey, Arc<Par>>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            scx: ShapeContext::new(),
            bidi: BidiResolver::new(),
            font_ctx: FontContext::default(),
            state: BuilderState::default(),
            layout_cache: HashMap::new(),
        }
    }

    pub fn start<'a>(
        &'a mut self,
        dir: Direction,
        // lang: Option<swash::text::Language>,
        scale: f32,
        offset: usize,
    ) -> Session<'a> {
        self.state.clear();
        self.state.begin(dir, None, scale, offset);
        let default_family = FamilyList::new("system-ui, sans-serif");
        let default_font = self.font_ctx.register_group(
            default_family.names(),
            default_family.key(),
            Default::default(),
        );
        if let Some(root) = self.state.spans.first_mut() {
            root.font_family = default_family;
            root.font = default_font;
        }
        Session {
            engine: self,
            dir_depth: 0,
            scale,
            needs_bidi: false,
            last_offset: offset,
            dir: dir,
        }
    }

    pub fn measure_text_style(&mut self, text: &str, style: &ComputedTextStyle) -> Size {
        self.measure_text_style_with_constraints(text, style, TextLayoutConstraints::UNBOUNDED)
    }

    pub fn measure_text_style_with_constraints(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Size {
        size_for_par(&self.layout_text(text, style, constraints))
    }

    fn layout_text_uncached(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Par {
        let styles = span_styles_for_style(style);
        let doc = Doc::simple(styles.iter(), text);
        self.layout_doc_with_constraints(&doc, constraints)
    }

    pub fn layout_doc(&mut self, doc: &Doc<'_>) -> Par {
        self.layout_doc_with_constraints(doc, TextLayoutConstraints::UNBOUNDED)
    }

    pub fn layout_doc_with_constraints(
        &mut self,
        doc: &Doc<'_>,
        constraints: TextLayoutConstraints,
    ) -> Par {
        let mut session = self.start(Direction::Auto, 1.0, 0);
        session.process(doc);
        let mut par = session.finish(None);
        par.break_lines()
            .break_remaining(max_advance(constraints), Alignment::Start);
        par
    }

    pub fn measure_doc(&mut self, doc: &Doc<'_>) -> Size {
        let par = self.layout_doc(doc);
        size_for_par(&par)
    }
}

impl TextLayouter for Engine {
    fn layout_text(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Arc<Par> {
        let key = TextLayoutKey::new(text, style, constraints);
        if let Some(par) = self.layout_cache.get(&key) {
            return Arc::clone(par);
        }

        let par = Arc::new(self.layout_text_uncached(text, style, constraints));
        self.layout_cache.insert(key, Arc::clone(&par));
        par
    }
}

impl TextMeasurer for Engine {
    fn measure_text(&mut self, text: &str, style: &ComputedTextStyle) -> Size {
        self.measure_text_style(text, style)
    }

    fn measure_text_with_constraints(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Size {
        self.measure_text_style_with_constraints(text, style, constraints)
    }
}

fn max_advance(constraints: TextLayoutConstraints) -> f32 {
    constraints
        .max_width
        .filter(|width| width.is_finite())
        .map(|width| width.max(0.0))
        .unwrap_or(f32::MAX)
}

fn size_for_par(par: &Par) -> Size {
    let mut width: f32 = 0.0;
    let mut height = 0.0;
    for line in par.lines() {
        width = width.max(line.advance_without_trailing_whitespace());
        height += line.size();
    }

    Size::new(width, height)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextLayoutKey {
    text: Arc<str>,
    style: TextStyleKey,
    constraints: TextLayoutConstraintsKey,
}

impl TextLayoutKey {
    fn new(text: &str, style: &ComputedTextStyle, constraints: TextLayoutConstraints) -> Self {
        Self {
            text: Arc::from(text),
            style: TextStyleKey::from(style),
            constraints: TextLayoutConstraintsKey::from(constraints),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TextLayoutConstraintsKey {
    max_width: Option<F32Key>,
}

impl From<TextLayoutConstraints> for TextLayoutConstraintsKey {
    fn from(constraints: TextLayoutConstraints) -> Self {
        Self {
            max_width: constraints.max_width.map(|width| F32Key(width.to_bits())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextStyleKey {
    color: ColorKey,
    font_family: FontFamily,
    font_size: F32Key,
    font_weight: XuiFontWeight,
    font_style: XuiFontStyle,
    line_height: LineHeightKey,
    letter_spacing: F32Key,
    decoration: TextDecoration,
}

impl From<&ComputedTextStyle> for TextStyleKey {
    fn from(style: &ComputedTextStyle) -> Self {
        Self {
            color: ColorKey::from(style.color),
            font_family: style.font_family.clone(),
            font_size: F32Key(style.font_size.to_bits()),
            font_weight: style.font_weight,
            font_style: style.font_style,
            line_height: LineHeightKey::from(style.line_height),
            letter_spacing: F32Key(style.letter_spacing.to_bits()),
            decoration: style.decoration,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ColorKey {
    r: F32Key,
    g: F32Key,
    b: F32Key,
    a: F32Key,
}

impl From<Color> for ColorKey {
    fn from(color: Color) -> Self {
        Self {
            r: F32Key(color.r.to_bits()),
            g: F32Key(color.g.to_bits()),
            b: F32Key(color.b.to_bits()),
            a: F32Key(color.a.to_bits()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct F32Key(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LineHeightKey {
    Normal,
    Px(F32Key),
    Em(F32Key),
}

impl From<LineHeight> for LineHeightKey {
    fn from(line_height: LineHeight) -> Self {
        match line_height {
            LineHeight::Normal => Self::Normal,
            LineHeight::Px(value) => Self::Px(F32Key(value.to_bits())),
            LineHeight::Em(value) => Self::Em(F32Key(value.to_bits())),
        }
    }
}

fn span_styles_for_style(style: &ComputedTextStyle) -> Vec<SpanStyle<'static>> {
    let mut styles = Vec::with_capacity(7);
    styles.push(SpanStyle::FamilyList(family_list(&style.font_family)));
    styles.push(SpanStyle::Size(style.font_size));
    styles.push(SpanStyle::Weight(font_weight(style.font_weight)));
    styles.push(SpanStyle::Style(font_style(style.font_style)));
    styles.push(SpanStyle::LineSpacing(line_spacing(
        style.line_height,
        style.font_size,
    )));
    styles.push(SpanStyle::LetterSpacing(style.letter_spacing));
    styles.push(SpanStyle::Underline(style.decoration.underline));
    styles
}

fn family_list(family: &FontFamily) -> FamilyList {
    match family {
        FontFamily::System => FamilyList::new("system-ui, sans-serif"),
        FontFamily::Named(name) => FamilyList::new(name),
        FontFamily::Stack(names) => FamilyList::new(&names.join(", ")),
    }
}

fn font_weight(weight: XuiFontWeight) -> Weight {
    match weight {
        XuiFontWeight::Thin => Weight::THIN,
        XuiFontWeight::ExtraLight => Weight::EXTRA_LIGHT,
        XuiFontWeight::Light => Weight::LIGHT,
        XuiFontWeight::Normal => Weight::NORMAL,
        XuiFontWeight::Medium => Weight::MEDIUM,
        XuiFontWeight::SemiBold => Weight::SEMI_BOLD,
        XuiFontWeight::Bold => Weight::BOLD,
        XuiFontWeight::ExtraBold => Weight::EXTRA_BOLD,
        XuiFontWeight::Black => Weight::BLACK,
        XuiFontWeight::Number(value) => Weight(value.clamp(1, 1000)),
    }
}

fn font_style(style: XuiFontStyle) -> Style {
    match style {
        XuiFontStyle::Normal => Style::Normal,
        XuiFontStyle::Italic => Style::Italic,
        XuiFontStyle::Oblique => Style::from_degrees(14.0),
    }
}

fn line_spacing(line_height: LineHeight, font_size: f32) -> f32 {
    match line_height {
        LineHeight::Normal => 1.0,
        LineHeight::Px(px) => {
            if font_size > 0.0 {
                px / font_size
            } else {
                1.0
            }
        }
        LineHeight::Em(em) => em,
    }
}

impl<'a> Session<'a> {
    pub fn process(&mut self, doc: &Doc) {
        for root in &doc.roots {
            let span = &doc.spans[*root];
            self.layout_span(span, doc);
        }
    }

    fn layout_span(&mut self, span: &Span, doc: &Doc) {
        self.push_span(&span.properties);
        for e in &span.elements {
            match e {
                SpanElement::Span(i) => self.layout_span(&doc.spans[*i], doc),
                SpanElement::Fragment(i) => {
                    let (start, end) = doc.fragments[*i];
                    if start < end {
                        if let Some(s) = doc.text.get(start..end) {
                            self.add_text(s);
                        }
                    }
                }
            }
        }
        self.pop_span();
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use xui_interface::{ComputedTextStyle, TextLayoutConstraints, TextMeasurer, TextStyle};

    use crate::{
        doc::{Doc, SpanStyle},
        engine::{Engine, TextLayouter},
        library::FamilyList,
    };

    fn text_style() -> ComputedTextStyle {
        TextStyle::default().into()
    }

    #[test]
    fn test() {
        let mut engine = Engine::new();
        let mut session = engine.start(crate::doc::Direction::Ltr, 2.0, 0);
        let properties = &[SpanStyle::FamilyList(FamilyList::new("pingfang sc"))];
        let doc = Doc::simple(properties, "Hello, World");
        session.process(&doc);
        let _par = session.finish(None);
    }

    #[test]
    fn layout_text_reuses_cached_par_for_same_props() {
        let mut engine = Engine::new();
        let style = text_style();

        let first = engine.layout_text("Hello, cache", &style, TextLayoutConstraints::UNBOUNDED);
        let second = engine.layout_text("Hello, cache", &style, TextLayoutConstraints::UNBOUNDED);

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(engine.layout_cache.len(), 1);
    }

    #[test]
    fn layout_text_uses_style_in_cache_key() {
        let mut engine = Engine::new();
        let style = text_style();
        let mut larger = style.clone();
        larger.font_size += 1.0;
        let mut spaced = style.clone();
        spaced.letter_spacing = 1.0;

        let first = engine.layout_text("Hello, cache", &style, TextLayoutConstraints::UNBOUNDED);
        let larger = engine.layout_text("Hello, cache", &larger, TextLayoutConstraints::UNBOUNDED);
        let spaced = engine.layout_text("Hello, cache", &spaced, TextLayoutConstraints::UNBOUNDED);

        assert!(!Arc::ptr_eq(&first, &larger));
        assert!(!Arc::ptr_eq(&first, &spaced));
        assert_eq!(engine.layout_cache.len(), 3);
    }

    #[test]
    fn layout_text_uses_constraints_in_cache_key() {
        let mut engine = Engine::new();
        let style = text_style();

        let wide = engine.layout_text(
            "Hello, cache constraints",
            &style,
            TextLayoutConstraints::max_width(500.0),
        );
        let narrow = engine.layout_text(
            "Hello, cache constraints",
            &style,
            TextLayoutConstraints::max_width(50.0),
        );
        let wide_again = engine.layout_text(
            "Hello, cache constraints",
            &style,
            TextLayoutConstraints::max_width(500.0),
        );

        assert!(Arc::ptr_eq(&wide, &wide_again));
        assert!(!Arc::ptr_eq(&wide, &narrow));
        assert_eq!(engine.layout_cache.len(), 2);
    }

    #[test]
    fn measure_text_populates_layout_cache() {
        let mut engine = Engine::new();
        let style = text_style();

        let _ = engine.measure_text("Measured once", &style);
        let first = engine.layout_text("Measured once", &style, TextLayoutConstraints::UNBOUNDED);
        let second = engine.layout_text("Measured once", &style, TextLayoutConstraints::UNBOUNDED);

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(engine.layout_cache.len(), 1);
    }
}
