use crate::doc::SpanStyle;

pub struct Span<'a> {
    pub properties: Vec<SpanStyle<'a>>,
    pub elements: Vec<SpanElement>,
}

impl<'a> Span<'a> {
    pub fn set_property(&mut self, property: &SpanStyle<'a>) {
        for prop in &mut self.properties {
            if prop.same_kind(property) {
                *prop = property.to_owned();
                return;
            }
        }
        self.properties.push(property.to_owned());
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SpanElement {
    Fragment(usize),
    Span(usize),
}
