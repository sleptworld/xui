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
    RootOverlayer,
    Image,
    Icon,
    Grid,
}

/// Controls whether a widget can receive focus.
///
/// `Auto` preserves the widget's built-in behavior. An explicit tab index also
/// makes an `Auto` widget focusable, matching the behavior authors generally
/// expect from `tab_index`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Focusability {
    #[default]
    Auto,
    Focusable,
    NotFocusable,
}

/// Focus metadata shared by every host widget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FocusProperties {
    pub focusability: Focusability,
    /// Negative values allow programmatic focus but exclude the widget from
    /// sequential keyboard navigation. Non-negative values participate in the
    /// tab order; positive values are visited before zero/document-order items.
    pub tab_index: Option<i32>,
}

impl FocusProperties {
    pub const fn focusable(mut self, focusable: bool) -> Self {
        self.focusability = if focusable {
            Focusability::Focusable
        } else {
            Focusability::NotFocusable
        };
        self
    }

    pub const fn tab_index(mut self, tab_index: i32) -> Self {
        self.tab_index = Some(tab_index);
        self
    }
}

/// Platform-neutral accessibility roles exposed by xui's host tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessibilityRole {
    Generic,
    Button,
    Checkbox,
    Group,
    Heading,
    Image,
    Label,
    Link,
    List,
    ListItem,
    Radio,
    RadioGroup,
    Switch,
    Tab,
    TabList,
    TabPanel,
    Text,
    TextField,
}

/// Platform-neutral accessibility metadata attached to a host widget.
///
/// Backends can consume this data to build their native accessibility tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct AccessibilityProperties {
    pub role: Option<AccessibilityRole>,
    pub id: Option<String>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub selected: Option<bool>,
    pub disabled: Option<bool>,
    pub controls: Option<String>,
    pub labelled_by: Option<String>,
}

impl AccessibilityProperties {
    pub fn role(mut self, role: AccessibilityRole) -> Self {
        self.role = Some(role);
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = Some(disabled);
        self
    }

    pub fn controls(mut self, id: impl Into<String>) -> Self {
        self.controls = Some(id.into());
        self
    }

    pub fn labelled_by(mut self, id: impl Into<String>) -> Self {
        self.labelled_by = Some(id.into());
        self
    }
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
