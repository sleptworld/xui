use swash::shape::ShapeContext;

use crate::{
    bidi::BidiResolver,
    doc::{Direction, Doc},
    library::{FamilyList, FontContext},
    par::{BuilderState, Session},
    span::{Span, SpanElement},
};

pub struct Engine {
    pub(crate) font_ctx: FontContext,
    pub(crate) bidi: BidiResolver,
    pub(crate) scx: ShapeContext,
    pub(crate) state: BuilderState,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            scx: ShapeContext::new(),
            bidi: BidiResolver::new(),
            font_ctx: FontContext::default(),
            state: BuilderState::default(),
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

mod test {
    use crate::{
        doc::{Doc, SpanStyle},
        engine::Engine,
        library::FamilyList,
    };

    #[test]
    fn test() {
        let mut engine = Engine::new();
        let mut session = engine.start(crate::doc::Direction::Ltr, 2.0, 0);
        let properties = &[SpanStyle::FamilyList(FamilyList::new("pingfang sc"))];
        let doc = Doc::simple(properties, "Hello, World");
        session.process(&doc);
        let par = session.finish(None);
    }
}
