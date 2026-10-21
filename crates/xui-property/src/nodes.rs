//! Handwritten output of #[widget] / #[component].
use crate::abilities::{
    Background, BoxLayout, CapScope, Decoration, FontSize, TextColor, Typography, Width,
};
use crate::capability::{CapSet, Has, Style, StyleSet};
use crate::caps::{ActionCaps, ButtonCaps, EmptyCaps, ImageCaps, TextCaps};
use crate::core::{Dirty, ParentStyle, StyleValue, Theme, WidgetState};
use crate::interaction::{Focus, Interaction, InteractionEnabled, OnClick, TabIndex};
use crate::managed::{GlobalShortcut, ShortcutAbility};
use crate::property::{Access, ManagedProperty, Property, ResolveAttr, Scope, SlotProperty};
use std::marker::PhantomData;

pub struct HostKind;
pub struct ComponentKind;

pub trait NodeDef {
    type Props;
    type Caps: StyleSet;
    type Kind;
}

pub struct PropsScope<N>(PhantomData<fn() -> N>);
impl<N: NodeDef> Scope for PropsScope<N> {
    type Storage = N::Props;
}

/// Complete declarative description, NOT a mounted widget instance.
/// A required-props Builder precedes this stage.
pub struct NodeDraft<N: NodeDef> {
    pub props: N::Props,
    pub config: <N::Caps as CapSet>::Config,
    pub style: Style<N::Caps>,
    _node: PhantomData<fn() -> N>,
}

impl<N: NodeDef> NodeDraft<N> {
    pub fn new(props: N::Props) -> Self {
        Self {
            props,
            config: Default::default(),
            style: Style::new(),
            _node: PhantomData,
        }
    }
    pub fn set<P: Property>(&mut self, value: P::Value)
    where
        Self: Access<P::Scope>,
    {
        P::replace(<Self as Access<P::Scope>>::access_mut(self), value);
    }
    pub fn resolve_style(
        &self,
        state: WidgetState,
        parent: &ParentStyle,
        theme: &Theme,
    ) -> crate::capability::Resolved<<N::Caps as StyleSet>::Computed> {
        self.style.resolve(&self.config, state, parent, theme)
    }
}

impl<N: NodeDef> Access<PropsScope<N>> for NodeDraft<N> {
    fn access(&self) -> &N::Props {
        &self.props
    }
    fn access_mut(&mut self) -> &mut N::Props {
        &mut self.props
    }
}
impl<N, C> Access<CapScope<C>> for NodeDraft<N>
where
    N: NodeDef,
    C: crate::capability::Capability,
    <N::Caps as CapSet>::Config: Has<C>,
{
    fn access(&self) -> &C::Config {
        <<N::Caps as CapSet>::Config as Has<C>>::get(&self.config)
    }
    fn access_mut(&mut self) -> &mut C::Config {
        <<N::Caps as CapSet>::Config as Has<C>>::get_mut(&mut self.config)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextProps {
    pub content: String,
    pub selectable: bool,
}
pub struct Text;
impl NodeDef for Text {
    type Props = TextProps;
    type Caps = TextCaps;
    type Kind = HostKind;
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageProps {
    pub source: String,
}
pub struct Image;
impl NodeDef for Image {
    type Props = ImageProps;
    type Caps = ImageCaps;
    type Kind = HostKind;
}
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonProps {
    pub label: String,
}
pub struct Button;
impl NodeDef for Button {
    type Props = ButtonProps;
    type Caps = ButtonCaps;
    type Kind = HostKind;
}
#[derive(Clone, Debug, PartialEq)]
pub struct ActionProps {
    pub id: usize,
}
pub struct Action;
impl NodeDef for Action {
    type Props = ActionProps;
    type Caps = ActionCaps;
    type Kind = HostKind;
}

#[derive(Clone, Debug, PartialEq)]
pub struct CounterProps {
    pub title: String,
    pub count: usize,
}
pub struct Counter;
impl NodeDef for Counter {
    type Props = CounterProps;
    type Caps = EmptyCaps;
    type Kind = ComponentKind;
}

pub struct TextContent;

impl Property for TextContent {
    type Scope = PropsScope<Text>;
    type Value = String;
    fn get(p: &TextProps) -> &String {
        &p.content
    }

    fn replace(p: &mut TextProps, v: String) {
        p.content = v;
    }
}

impl SlotProperty for TextContent {
    const HINT: Dirty = Dirty(Dirty::TEXT.0 | Dirty::LAYOUT.0);
    fn same(a: &String, b: &String) -> bool {
        a == b
    }
}

pub struct TextSelectable;
impl Property for TextSelectable {
    type Scope = PropsScope<Text>;
    type Value = bool;
    fn get(p: &TextProps) -> &bool {
        &p.selectable
    }
    fn replace(p: &mut TextProps, v: bool) {
        p.selectable = v;
    }
}
impl SlotProperty for TextSelectable {
    const HINT: Dirty = Dirty::EVENT;
    fn same(a: &bool, b: &bool) -> bool {
        a == b
    }
}

pub struct ImageSource;
impl Property for ImageSource {
    type Scope = PropsScope<Image>;
    type Value = String;
    fn get(p: &ImageProps) -> &String {
        &p.source
    }
    fn replace(p: &mut ImageProps, v: String) {
        p.source = v;
    }
}
impl SlotProperty for ImageSource {
    const HINT: Dirty = Dirty::PAINT;
    fn same(a: &String, b: &String) -> bool {
        a == b
    }
}

pub struct ButtonLabel;
impl Property for ButtonLabel {
    type Scope = PropsScope<Button>;
    type Value = String;
    fn get(p: &ButtonProps) -> &String {
        &p.label
    }
    fn replace(p: &mut ButtonProps, v: String) {
        p.label = v;
    }
}
impl SlotProperty for ButtonLabel {
    const HINT: Dirty = Dirty::TEXT;
    fn same(a: &String, b: &String) -> bool {
        a == b
    }
}
impl ManagedProperty for ButtonLabel {
    type Resource = bool; // Whether this mounted node owns a registered a11y label.
    fn on_commit(
        old: Option<&Self::Value>,
        new: Option<&Self::Value>,
        owned: &mut Self::Resource,
        ctx: &mut crate::runtime::CommitCtx<'_>,
    ) {
        match new {
            Some(label) if !*owned || old != Some(label) => {
                ctx.services
                    .accessibility
                    .set_label(ctx.node, label.clone());
                *owned = true;
            }
            None if *owned => {
                ctx.services.accessibility.remove_label(ctx.node);
                *owned = false;
            }
            _ => {}
        }
    }
}

pub struct CounterTitle;
impl Property for CounterTitle {
    type Scope = PropsScope<Counter>;
    type Value = String;
    fn get(p: &CounterProps) -> &String {
        &p.title
    }
    fn replace(p: &mut CounterProps, v: String) {
        p.title = v;
    }
}
impl SlotProperty for CounterTitle {
    const HINT: Dirty = Dirty::COMPONENT_PROPS;
    fn same(a: &String, b: &String) -> bool {
        a == b
    }
}

pub struct CounterCount;
impl Property for CounterCount {
    type Scope = PropsScope<Counter>;
    type Value = usize;
    fn get(p: &CounterProps) -> &usize {
        &p.count
    }
    fn replace(p: &mut CounterProps, v: usize) {
        p.count = v;
    }
}

impl SlotProperty for CounterCount {
    const HINT: Dirty = Dirty::COMPONENT_PROPS;
    fn same(a: &usize, b: &usize) -> bool {
        a == b
    }
}

pub mod attr {
    pub struct Width;
    pub struct Background;
    pub struct Color;
    pub struct FontSize;
    pub struct OnClick;
    pub struct InteractionEnabled;
    pub struct TabIndex;
    pub struct GlobalShortcut;
    pub struct Content;
    pub struct Selectable;
    pub struct Source;
    pub struct Label;
    pub struct Title;
    pub struct Count;
}

// Capability-generated blanket mappings. No mapping for an unsupported cap.
impl<N> ResolveAttr<attr::Width> for NodeDraft<N>
where
    N: NodeDef,
    <N::Caps as CapSet>::Config: Has<BoxLayout>,
{
    type Property = Width;
}

impl<N> ResolveAttr<attr::Background> for NodeDraft<N>
where
    N: NodeDef,
    <N::Caps as CapSet>::Config: Has<Decoration>,
{
    type Property = Background;
}
impl<N> ResolveAttr<attr::Color> for NodeDraft<N>
where
    N: NodeDef,
    <N::Caps as CapSet>::Config: Has<Typography>,
{
    type Property = TextColor;
}
impl<N> ResolveAttr<attr::FontSize> for NodeDraft<N>
where
    N: NodeDef,
    <N::Caps as CapSet>::Config: Has<Typography>,
{
    type Property = FontSize;
}
impl<N> ResolveAttr<attr::OnClick> for NodeDraft<N>
where
    N: NodeDef,
    <N::Caps as CapSet>::Config: Has<Interaction>,
{
    type Property = OnClick;
}
impl<N> ResolveAttr<attr::InteractionEnabled> for NodeDraft<N>
where
    N: NodeDef,
    <N::Caps as CapSet>::Config: Has<Interaction>,
{
    type Property = InteractionEnabled;
}
impl<N> ResolveAttr<attr::TabIndex> for NodeDraft<N>
where
    N: NodeDef,
    <N::Caps as CapSet>::Config: Has<Focus>,
{
    type Property = TabIndex;
}
impl<N> ResolveAttr<attr::GlobalShortcut> for NodeDraft<N>
where
    N: NodeDef,
    <N::Caps as CapSet>::Config: Has<ShortcutAbility>,
{
    type Property = GlobalShortcut;
}
impl ResolveAttr<attr::Content> for NodeDraft<Text> {
    type Property = TextContent;
}
impl ResolveAttr<attr::Selectable> for NodeDraft<Text> {
    type Property = TextSelectable;
}
impl ResolveAttr<attr::Source> for NodeDraft<Image> {
    type Property = ImageSource;
}
impl ResolveAttr<attr::Label> for NodeDraft<Button> {
    type Property = ButtonLabel;
}
impl ResolveAttr<attr::Title> for NodeDraft<Counter> {
    type Property = CounterTitle;
}
impl ResolveAttr<attr::Count> for NodeDraft<Counter> {
    type Property = CounterCount;
}

/// What a typed xui! expansion can emit once the target type is resolved.
pub fn set_mapped<N, Key>(
    node: &mut NodeDraft<N>,
    value: <<NodeDraft<N> as ResolveAttr<Key>>::Property as Property>::Value,
) where
    N: NodeDef,
    NodeDraft<N>: ResolveAttr<Key>
        + Access<<<NodeDraft<N> as ResolveAttr<Key>>::Property as Property>::Scope>,
{
    node.set::<<NodeDraft<N> as ResolveAttr<Key>>::Property>(value);
}

impl NodeDraft<Text> {
    pub fn text(content: impl Into<String>) -> Self {
        Self::new(TextProps {
            content: content.into(),
            selectable: false,
        })
    }
}
impl NodeDraft<Image> {
    pub fn image(source: impl Into<String>) -> Self {
        Self::new(ImageProps {
            source: source.into(),
        })
    }
}
impl NodeDraft<Button> {
    pub fn button(label: impl Into<String>) -> Self {
        Self::new(ButtonProps {
            label: label.into(),
        })
    }
}
impl NodeDraft<Action> {
    pub fn action(id: usize) -> Self {
        Self::new(ActionProps { id })
    }
}
impl NodeDraft<Counter> {
    pub fn counter(title: impl Into<String>, count: usize) -> Self {
        Self::new(CounterProps {
            title: title.into(),
            count,
        })
    }
}
impl<N: NodeDef> NodeDraft<N>
where
    <N::Caps as CapSet>::Config: Has<BoxLayout>,
{
    pub fn width(mut self, width: impl Into<crate::core::Sizing>) -> Self {
        self.set::<Width>(StyleValue::Value(width.into()));
        self
    }
}
