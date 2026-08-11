use slotmap::new_key_type;

new_key_type! {
    pub struct NodeId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetType {
    Text,
    TextInput,
    Canvas,
    Label,
    Button,
    Column,
    Row,
    Container,
    ZStack,
    StyleScope,
    ScrollScope,
    Image,
    Icon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key(fixedstr::str64);

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Self(value.into())
    }
}

pub trait Component<Context, Output> {
    fn render(&mut self, cx: &mut Context) -> Output;
}

impl<F, Context, Output> Component<Context, Output> for F
where
    F: FnMut(&mut Context) -> Output,
{
    fn render(&mut self, cx: &mut Context) -> Output {
        self(cx)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeLifecycleEvent {
    Created(NodeId),
    Moved {
        id: NodeId,
        old_parent: Option<NodeId>,
        new_parent: Option<NodeId>,
        old_position: usize,
        new_position: usize,
    },
    Removed(NodeId),
}
