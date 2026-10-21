//! Call-site static template recipe. A production template can include paths,
//! static nodes and slot descriptors; this POC uses a statically stored
//! initializer function rather than any heap-allocated template AST.
use std::marker::PhantomData;
use crate::nodes::{NodeDef, NodeDraft};

pub struct Template<N: NodeDef, Input> {
    pub id: &'static str,
    build: fn(Input) -> NodeDraft<N>,
    _marker: PhantomData<fn(Input) -> N>,
}

impl<N: NodeDef, Input> Template<N, Input> {
    pub const fn new(id: &'static str, build: fn(Input) -> NodeDraft<N>) -> Self {
        Self { id, build, _marker: PhantomData }
    }
    pub fn mount(&self, dynamic: Input) -> NodeDraft<N> {
        (self.build)(dynamic)
    }
}
