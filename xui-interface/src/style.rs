use std::{
    hash::{Hash, Hasher},
    time::Duration,
};

use crate::{
    core::Sizing, event::EventTrigger, text::TextStyle, Color, EdgeInsets, FontFamily, FontStyle,
    FontWeight, LineHeight, Point, Size, TextDecoration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleValue<T> {
    Unset,
    Inherit,
    Initial,
    Value(T),
}

impl<T> Default for StyleValue<T> {
    fn default() -> Self {
        Self::Unset
    }
}

impl<T> StyleValue<T> {
    pub fn value(value: T) -> Self {
        Self::Value(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorToken {
    Text,
    InverseText,
    Background,
    Surface,
    MutedSurface,
    Border,
    Primary,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorValue {
    Color(Color),
    Token(ColorToken),
}

impl From<Color> for ColorValue {
    fn from(value: Color) -> Self {
        Self::Color(value)
    }
}

impl From<ColorToken> for ColorValue {
    fn from(value: ColorToken) -> Self {
        Self::Token(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorStyle {
    Solid(ColorValue),
    LinearGradient(LinearGradientStyle),
    RadialGradient(RadialGradientStyle),
}

impl Default for ColorStyle {
    fn default() -> Self {
        Self::Solid(Color::TRANSPARENT.into())
    }
}

impl From<ColorValue> for ColorStyle {
    fn from(value: ColorValue) -> Self {
        Self::Solid(value)
    }
}

impl From<Color> for ColorStyle {
    fn from(value: Color) -> Self {
        Self::Solid(value.into())
    }
}

impl From<ColorToken> for ColorStyle {
    fn from(value: ColorToken) -> Self {
        Self::Solid(value.into())
    }
}

impl ColorStyle {
    pub fn solid(color: impl Into<ColorValue>) -> Self {
        Self::Solid(color.into())
    }

    pub fn linear_gradient(
        start: Point,
        end: Point,
        from: impl Into<ColorValue>,
        to: impl Into<ColorValue>,
    ) -> Self {
        Self::LinearGradient(LinearGradientStyle::new(start, end, from, to))
    }

    pub fn radial_gradient(
        center: Point,
        radius: impl Into<LengthValue>,
        from: impl Into<ColorValue>,
        to: impl Into<ColorValue>,
    ) -> Self {
        Self::RadialGradient(RadialGradientStyle::new(center, radius, from, to))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    pub edge: EdgeInsets,
    pub color: ComputedColorStyle,
    pub line_style: StrokeLineStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub color: Color,
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrokeLineStyle {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeStyle {
    pub color: ColorStyle,
    pub width: LengthValue,
    pub line_style: StrokeLineStyle,
}

impl StrokeStyle {
    pub fn new(color: impl Into<ColorStyle>, width: impl Into<LengthValue>) -> Self {
        Self {
            color: color.into(),
            width: width.into(),
            line_style: StrokeLineStyle::Solid,
        }
    }

    pub fn line_style(mut self, line_style: StrokeLineStyle) -> Self {
        self.line_style = line_style;
        self
    }

    pub fn dashed(mut self) -> Self {
        self.line_style = StrokeLineStyle::Dashed;
        self
    }

    pub fn dotted(mut self) -> Self {
        self.line_style = StrokeLineStyle::Dotted;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearGradientStyle {
    pub start: Point,
    pub end: Point,
    pub from: ColorValue,
    pub to: ColorValue,
}

impl LinearGradientStyle {
    pub fn new(
        start: Point,
        end: Point,
        from: impl Into<ColorValue>,
        to: impl Into<ColorValue>,
    ) -> Self {
        Self {
            start,
            end,
            from: from.into(),
            to: to.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialGradientStyle {
    pub center: Point,
    pub radius: LengthValue,
    pub from: ColorValue,
    pub to: ColorValue,
}

impl RadialGradientStyle {
    pub fn new(
        center: Point,
        radius: impl Into<LengthValue>,
        from: impl Into<ColorValue>,
        to: impl Into<ColorValue>,
    ) -> Self {
        Self {
            center,
            radius: radius.into(),
            from: from.into(),
            to: to.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpacingToken {
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RadiusToken {
    Sm,
    Md,
    Lg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontSizeToken {
    Sm,
    Md,
    Lg,
    Xl,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthValue {
    Px(f32),
    Spacing(SpacingToken),
    Radius(RadiusToken),
    FontSize(FontSizeToken),
}

impl From<f32> for LengthValue {
    fn from(value: f32) -> Self {
        Self::Px(value)
    }
}

impl From<SpacingToken> for LengthValue {
    fn from(value: SpacingToken) -> Self {
        Self::Spacing(value)
    }
}

impl From<RadiusToken> for LengthValue {
    fn from(value: RadiusToken) -> Self {
        Self::Radius(value)
    }
}

impl From<FontSizeToken> for LengthValue {
    fn from(value: FontSizeToken) -> Self {
        Self::FontSize(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlexDirectionStyle {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScrollDirectionStyle {
    #[default]
    None,
    Horizontal,
    Vertical,
    Both,
}

impl ScrollDirectionStyle {
    pub fn allows_horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    pub fn allows_vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    pub fn is_scrollable(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScrollbarVisibilityStyle {
    #[default]
    Auto,
    Always,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarStyle {
    pub width: LengthValue,
    pub track_color: ColorStyle,
    pub thumb_color: ColorStyle,
    pub radius: LengthValue,
    pub visibility: ScrollbarVisibilityStyle,
}

impl Default for ScrollbarStyle {
    fn default() -> Self {
        Self {
            width: LengthValue::Px(8.0),
            track_color: ColorStyle::Solid(Color::TRANSPARENT.into()),
            thumb_color: ColorStyle::Solid(Color::rgba(0.0, 0.0, 0.0, 0.35).into()),
            radius: LengthValue::Px(4.0),
            visibility: ScrollbarVisibilityStyle::Auto,
        }
    }
}

impl ScrollbarStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn width(mut self, width: impl Into<LengthValue>) -> Self {
        self.width = width.into();
        self
    }

    pub fn track_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.track_color = color.into();
        self
    }

    pub fn thumb_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.thumb_color = color.into();
        self
    }

    pub fn radius(mut self, radius: impl Into<LengthValue>) -> Self {
        self.radius = radius.into();
        self
    }

    pub fn visibility(mut self, visibility: ScrollbarVisibilityStyle) -> Self {
        self.visibility = visibility;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowStyle {
    pub color: ColorValue,
    pub offset: Point,
    pub blur: LengthValue,
    pub spread: LengthValue,
}

impl Default for ShadowStyle {
    fn default() -> Self {
        Self {
            color: Color::TRANSPARENT.into(),
            offset: Point::new(0.0, 0.0),
            blur: LengthValue::Px(0.0),
            spread: LengthValue::Px(0.0),
        }
    }
}

impl ShadowStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn color(mut self, color: impl Into<ColorValue>) -> Self {
        self.color = color.into();
        self
    }

    pub fn offset(mut self, offset: Point) -> Self {
        self.offset = offset;
        self
    }

    pub fn blur(mut self, blur: impl Into<LengthValue>) -> Self {
        self.blur = blur.into();
        self
    }

    pub fn spread(mut self, spread: impl Into<LengthValue>) -> Self {
        self.spread = spread.into();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlignStyle {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JustifyStyle {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextStylePatch {
    pub color: StyleValue<ColorValue>,
    pub font_family: StyleValue<FontFamily>,
    pub font_size: StyleValue<LengthValue>,
    pub font_weight: StyleValue<FontWeight>,
    pub font_style: StyleValue<FontStyle>,
    pub line_height: StyleValue<LineHeight>,
    pub letter_spacing: StyleValue<LengthValue>,
    pub decoration: StyleValue<TextDecoration>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutStylePatch {
    // pub flex_direction: StyleValue<FlexDirectionStyle>,
    pub gap: StyleValue<LengthValue>,
    pub size: StyleValue<Size<Sizing>>,
    pub min_size: StyleValue<Size<Sizing>>,
    pub max_size: StyleValue<Size<Sizing>>,
    pub margin: StyleValue<EdgeInsets>,
    pub padding: StyleValue<EdgeInsets>,
    pub align: StyleValue<AlignStyle>,
    pub justify: StyleValue<JustifyStyle>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaintStylePatch {
    pub background: StyleValue<ColorStyle>,
    pub border_color: StyleValue<ColorStyle>,
    pub border_width: StyleValue<LengthValue>,
    pub border_radius: StyleValue<LengthValue>,
    pub stroke: StyleValue<StrokeStyle>,
    pub shadow: StyleValue<ShadowStyle>,
    pub clip: StyleValue<bool>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScrollStylePatch {
    pub direction: StyleValue<ScrollDirectionStyle>,
    pub scrollbar: StyleValue<ScrollbarStyle>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Style {
    pub text: TextStylePatch,
    pub layout: LayoutStylePatch,
    pub paint: PaintStylePatch,
    pub scroll: ScrollStylePatch,
}

impl Style {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn color(mut self, color: impl Into<ColorValue>) -> Self {
        self.text.color = StyleValue::Value(color.into());
        self
    }

    pub fn font_family(mut self, font_family: FontFamily) -> Self {
        self.text.font_family = StyleValue::Value(font_family);
        self
    }

    pub fn font_size(mut self, font_size: impl Into<LengthValue>) -> Self {
        self.text.font_size = StyleValue::Value(font_size.into());
        self
    }

    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.text.font_weight = StyleValue::Value(font_weight);
        self
    }

    pub fn font_style(mut self, font_style: FontStyle) -> Self {
        self.text.font_style = StyleValue::Value(font_style);
        self
    }

    pub fn line_height(mut self, line_height: LineHeight) -> Self {
        self.text.line_height = StyleValue::Value(line_height);
        self
    }

    pub fn letter_spacing(mut self, letter_spacing: impl Into<LengthValue>) -> Self {
        self.text.letter_spacing = StyleValue::Value(letter_spacing.into());
        self
    }

    pub fn decoration(mut self, decoration: TextDecoration) -> Self {
        self.text.decoration = StyleValue::Value(decoration);
        self
    }

    // pub fn flex_direction(mut self, flex_direction: FlexDirectionStyle) -> Self {
    //     self.layout.flex_direction = StyleValue::Value(flex_direction);
    //     self
    // }

    pub fn gap(mut self, gap: impl Into<LengthValue>) -> Self {
        self.layout.gap = StyleValue::Value(gap.into());
        self
    }

    pub fn size(mut self, size: Size<Sizing>) -> Self {
        self.layout.size = StyleValue::Value(size);
        self
    }

    pub fn width(mut self, width: Sizing) -> Self {
        let size = match self.layout.size {
            StyleValue::Value(mut size) => {
                size.width = width;
                size
            }
            _ => Size::<Sizing>::new(width, Sizing::Hug),
        };
        self.layout.size = StyleValue::Value(size);
        self
    }

    pub fn height(mut self, height: Sizing) -> Self {
        let size = match self.layout.size {
            StyleValue::Value(mut size) => {
                size.height = height;
                size
            }
            _ => Size::<Sizing>::new(Sizing::Hug, height),
        };
        self.layout.size = StyleValue::Value(size);
        self
    }

    pub fn min_size(mut self, size: Size<Sizing>) -> Self {
        self.layout.min_size = StyleValue::Value(size);
        self
    }

    pub fn max_size(mut self, size: Size<Sizing>) -> Self {
        self.layout.max_size = StyleValue::Value(size);
        self
    }

    pub fn margin(mut self, margin: EdgeInsets) -> Self {
        self.layout.margin = StyleValue::Value(margin);
        self
    }

    pub fn padding(mut self, padding: EdgeInsets) -> Self {
        self.layout.padding = StyleValue::Value(padding);
        self
    }

    pub fn align(mut self, align: AlignStyle) -> Self {
        self.layout.align = StyleValue::Value(align);
        self
    }

    pub fn justify(mut self, justify: JustifyStyle) -> Self {
        self.layout.justify = StyleValue::Value(justify);
        self
    }

    pub fn background(mut self, color: impl Into<ColorStyle>) -> Self {
        self.paint.background = StyleValue::Value(color.into());
        self
    }

    pub fn border_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.paint.border_color = StyleValue::Value(color.into());
        self
    }

    pub fn border_width(mut self, width: impl Into<LengthValue>) -> Self {
        self.paint.border_width = StyleValue::Value(width.into());
        self
    }

    pub fn border_radius(mut self, radius: impl Into<LengthValue>) -> Self {
        self.paint.border_radius = StyleValue::Value(radius.into());
        self
    }

    pub fn stroke(mut self, stroke: StrokeStyle) -> Self {
        self.paint.stroke = StyleValue::Value(stroke);
        self
    }

    pub fn stroke_style(
        mut self,
        color: impl Into<ColorStyle>,
        width: impl Into<LengthValue>,
        line_style: StrokeLineStyle,
    ) -> Self {
        self.paint.stroke =
            StyleValue::Value(StrokeStyle::new(color, width).line_style(line_style));
        self
    }

    pub fn no_stroke(mut self) -> Self {
        self.paint.stroke = StyleValue::Initial;
        self
    }

    pub fn shadow(mut self, shadow: ShadowStyle) -> Self {
        self.paint.shadow = StyleValue::Value(shadow);
        self
    }

    pub fn box_shadow(
        mut self,
        color: impl Into<ColorValue>,
        offset: Point,
        blur: impl Into<LengthValue>,
        spread: impl Into<LengthValue>,
    ) -> Self {
        self.paint.shadow = StyleValue::Value(
            ShadowStyle::new()
                .color(color)
                .offset(offset)
                .blur(blur)
                .spread(spread),
        );
        self
    }

    pub fn no_shadow(mut self) -> Self {
        self.paint.shadow = StyleValue::Initial;
        self
    }

    pub fn clip(mut self, clip: bool) -> Self {
        self.paint.clip = StyleValue::Value(clip);
        self
    }

    pub fn scroll_direction(mut self, direction: ScrollDirectionStyle) -> Self {
        self.scroll.direction = StyleValue::Value(direction);
        self
    }

    pub fn scroll_vertical(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::Vertical)
    }

    pub fn scroll_horizontal(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::Horizontal)
    }

    pub fn scroll_both(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::Both)
    }

    pub fn no_scroll(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::None)
    }

    pub fn scrollbar(mut self, scrollbar: ScrollbarStyle) -> Self {
        self.scroll.scrollbar = StyleValue::Value(scrollbar);
        self
    }

    pub fn scrollbar_width(mut self, width: impl Into<LengthValue>) -> Self {
        let scrollbar = match self.scroll.scrollbar {
            StyleValue::Value(scrollbar) => scrollbar.width(width),
            _ => ScrollbarStyle::new().width(width),
        };
        self.scroll.scrollbar = StyleValue::Value(scrollbar);
        self
    }

    pub fn scrollbar_track_color(mut self, color: impl Into<ColorStyle>) -> Self {
        let scrollbar = match self.scroll.scrollbar {
            StyleValue::Value(scrollbar) => scrollbar.track_color(color),
            _ => ScrollbarStyle::new().track_color(color),
        };
        self.scroll.scrollbar = StyleValue::Value(scrollbar);
        self
    }

    pub fn scrollbar_thumb_color(mut self, color: impl Into<ColorStyle>) -> Self {
        let scrollbar = match self.scroll.scrollbar {
            StyleValue::Value(scrollbar) => scrollbar.thumb_color(color),
            _ => ScrollbarStyle::new().thumb_color(color),
        };
        self.scroll.scrollbar = StyleValue::Value(scrollbar);
        self
    }

    pub fn scrollbar_radius(mut self, radius: impl Into<LengthValue>) -> Self {
        let scrollbar = match self.scroll.scrollbar {
            StyleValue::Value(scrollbar) => scrollbar.radius(radius),
            _ => ScrollbarStyle::new().radius(radius),
        };
        self.scroll.scrollbar = StyleValue::Value(scrollbar);
        self
    }

    pub fn scrollbar_visibility(mut self, visibility: ScrollbarVisibilityStyle) -> Self {
        let scrollbar = match self.scroll.scrollbar {
            StyleValue::Value(scrollbar) => scrollbar.visibility(visibility),
            _ => ScrollbarStyle::new().visibility(visibility),
        };
        self.scroll.scrollbar = StyleValue::Value(scrollbar);
        self
    }

    pub fn merge(&mut self, other: &Style) {
        merge_text(&mut self.text, &other.text);
        merge_layout(&mut self.layout, &other.layout);
        merge_paint(&mut self.paint, &other.paint);
        merge_scroll(&mut self.scroll, &other.scroll);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationEasing {
    Linear,
    QuadIn,
    QuadOut,
    QuadInOut,
    CubicIn,
    CubicOut,
    CubicInOut,
    QuartIn,
    QuartOut,
    QuartInOut,
    QuintIn,
    QuintOut,
    QuintInOut,
    SineIn,
    SineOut,
    SineInOut,
}

impl Default for AnimationEasing {
    fn default() -> Self {
        Self::Linear
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimationTransition {
    pub duration: Duration,
    pub delay: Duration,
    pub easing: AnimationEasing,
}

impl AnimationTransition {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            delay: Duration::ZERO,
            easing: AnimationEasing::default(),
        }
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn ease(mut self, easing: AnimationEasing) -> Self {
        self.easing = easing;
        self
    }
}

impl Default for AnimationTransition {
    fn default() -> Self {
        Self::new(Duration::ZERO)
    }
}

#[derive(Debug, Clone, PartialEq, Hash)]
pub struct StyleAnimation {
    pub trigger: EventTrigger,
    pub style: Style,
    pub transition: AnimationTransition,
}

impl StyleAnimation {
    pub fn new(trigger: EventTrigger, style: Style, transition: AnimationTransition) -> Self {
        Self {
            trigger,
            style,
            transition,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Hash)]
pub struct AnimatedStyle {
    pub base: Style,
    pub animations: Vec<StyleAnimation>,
}

impl AnimatedStyle {
    pub fn new(base: Style) -> Self {
        Self {
            base,
            animations: Vec::new(),
        }
    }

    pub fn animation(
        mut self,
        trigger: EventTrigger,
        style: Style,
        transition: AnimationTransition,
    ) -> Self {
        self.animations
            .push(StyleAnimation::new(trigger, style, transition));
        self
    }

    pub fn on_hover(self, style: Style, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnHover, style, transition)
    }

    pub fn on_hover_start(self, style: Style, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnHoverStart, style, transition)
    }

    pub fn on_hover_end(self, style: Style, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnHoverEnd, style, transition)
    }

    pub fn on_press(self, style: Style, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnPress, style, transition)
    }

    pub fn on_focus(self, style: Style, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnFocus, style, transition)
    }

    pub fn on_click(self, style: Style, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnClick, style, transition)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WidgetState {
    pub hovered: bool,
    pub pressed: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedTextStyle {
    pub color: Color,
    pub font_family: FontFamily,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub line_height: LineHeight,
    pub letter_spacing: f32,
    pub decoration: TextDecoration,
}

impl From<TextStyle> for ComputedTextStyle {
    fn from(style: TextStyle) -> Self {
        Self {
            color: style.color,
            font_family: style.font_family,
            font_size: style.font_size,
            font_weight: style.font_weight,
            font_style: style.font_style,
            line_height: style.line_height,
            letter_spacing: style.letter_spacing,
            decoration: style.decoration,
        }
    }
}

impl From<&TextStyle> for ComputedTextStyle {
    fn from(style: &TextStyle) -> Self {
        Self {
            color: style.color,
            font_family: style.font_family.clone(),
            font_size: style.font_size,
            font_weight: style.font_weight,
            font_style: style.font_style,
            line_height: style.line_height,
            letter_spacing: style.letter_spacing,
            decoration: style.decoration,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedLayoutStyle {
    pub flex_direction: FlexDirectionStyle,
    pub gap: f32,
    pub size: Option<Size<Sizing>>,
    pub min_size: Option<Size<Sizing>>,
    pub max_size: Option<Size<Sizing>>,
    pub margin: EdgeInsets,
    pub padding: EdgeInsets,
    pub align: AlignStyle,
    pub justify: JustifyStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedPaintStyle {
    pub background: ComputedColorStyle,
    pub border_color: ComputedColorStyle,
    pub border_width: f32,
    pub border_radius: f32,
    pub stroke: Option<ComputedStrokeStyle>,
    pub shadow: Option<ComputedShadowStyle>,
    pub clip: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedScrollStyle {
    pub direction: ScrollDirectionStyle,
    pub scrollbar: ComputedScrollbarStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedScrollbarStyle {
    pub width: f32,
    pub track_color: ComputedColorStyle,
    pub thumb_color: ComputedColorStyle,
    pub radius: f32,
    pub visibility: ScrollbarVisibilityStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComputedColorStyle {
    Solid(Color),
    LinearGradient(ComputedLinearGradientStyle),
    RadialGradient(ComputedRadialGradientStyle),
}

impl Default for ComputedColorStyle {
    fn default() -> Self {
        Self::Solid(Color::TRANSPARENT)
    }
}

impl ComputedColorStyle {
    pub fn is_visible(self) -> bool {
        match self {
            Self::Solid(color) => color.a > 0.0,
            Self::LinearGradient(gradient) => gradient.from.a > 0.0 || gradient.to.a > 0.0,
            Self::RadialGradient(gradient) => gradient.from.a > 0.0 || gradient.to.a > 0.0,
        }
    }

    pub fn solid_color(self) -> Option<Color> {
        match self {
            Self::Solid(color) => Some(color),
            Self::LinearGradient(_) | Self::RadialGradient(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedLinearGradientStyle {
    pub start: Point,
    pub end: Point,
    pub from: Color,
    pub to: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedRadialGradientStyle {
    pub center: Point,
    pub radius: f32,
    pub from: Color,
    pub to: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedStrokeStyle {
    pub color: ComputedColorStyle,
    pub width: f32,
    pub line_style: StrokeLineStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedShadowStyle {
    pub color: Color,
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub text: ComputedTextStyle,
    pub layout: ComputedLayoutStyle,
    pub paint: ComputedPaintStyle,
    pub scroll: ComputedScrollStyle,
}

impl ComputedStyle {
    pub fn initial(theme: &Theme) -> Self {
        Self {
            text: ComputedTextStyle {
                color: theme.color(ColorToken::Text),
                font_family: FontFamily::System,
                font_size: theme.font_size(FontSizeToken::Md),
                font_weight: FontWeight::Normal,
                font_style: FontStyle::Normal,
                line_height: LineHeight::Normal,
                letter_spacing: 0.0,
                decoration: TextDecoration::default(),
            },
            layout: ComputedLayoutStyle {
                flex_direction: FlexDirectionStyle::Column,
                gap: 0.0,
                size: None,
                min_size: None,
                max_size: None,
                margin: EdgeInsets::ZERO,
                padding: EdgeInsets::ZERO,
                align: AlignStyle::Start,
                justify: JustifyStyle::Start,
            },
            paint: ComputedPaintStyle {
                background: ComputedColorStyle::Solid(Color::TRANSPARENT),
                border_color: ComputedColorStyle::Solid(Color::TRANSPARENT),
                border_width: 0.0,
                border_radius: 0.0,
                stroke: None,
                shadow: None,
                clip: false,
            },
            scroll: ComputedScrollStyle {
                direction: ScrollDirectionStyle::None,
                scrollbar: ComputedScrollbarStyle {
                    width: 8.0,
                    track_color: ComputedColorStyle::Solid(Color::TRANSPARENT),
                    thumb_color: ComputedColorStyle::Solid(Color::rgba(0.0, 0.0, 0.0, 0.35)),
                    radius: 4.0,
                    visibility: ScrollbarVisibilityStyle::Auto,
                },
            },
        }
    }

    pub fn apply(&mut self, parent: &ComputedStyle, patch: &Style, theme: &Theme) {
        apply_text(&mut self.text, &parent.text, &patch.text, theme);
        apply_layout(&mut self.layout, &patch.layout, theme);
        apply_paint(&mut self.paint, &patch.paint, theme);
        apply_scroll(&mut self.scroll, &patch.scroll, theme);
    }

    pub fn inherited_from(&self, theme: &Theme) -> Self {
        let mut next = Self::initial(theme);
        next.text = self.text.clone();
        next
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub text: Color,
    pub inverse_text: Color,
    pub background: Color,
    pub surface: Color,
    pub muted_surface: Color,
    pub border: Color,
    pub primary: Color,
    pub spacing_xs: f32,
    pub spacing_sm: f32,
    pub spacing_md: f32,
    pub spacing_lg: f32,
    pub spacing_xl: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    pub font_size_sm: f32,
    pub font_size_md: f32,
    pub font_size_lg: f32,
    pub font_size_xl: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            text: Color::BLACK,
            inverse_text: Color::WHITE,
            background: Color::WHITE,
            surface: Color::GRAY_100,
            muted_surface: Color::GRAY_300,
            border: Color::GRAY_300,
            primary: Color::BLUE_500,
            spacing_xs: 4.0,
            spacing_sm: 8.0,
            spacing_md: 12.0,
            spacing_lg: 16.0,
            spacing_xl: 24.0,
            radius_sm: 2.0,
            radius_md: 4.0,
            radius_lg: 8.0,
            font_size_sm: 12.0,
            font_size_md: 14.0,
            font_size_lg: 16.0,
            font_size_xl: 20.0,
        }
    }
}

impl Theme {
    pub fn color(&self, token: ColorToken) -> Color {
        match token {
            ColorToken::Text => self.text,
            ColorToken::InverseText => self.inverse_text,
            ColorToken::Background => self.background,
            ColorToken::Surface => self.surface,
            ColorToken::MutedSurface => self.muted_surface,
            ColorToken::Border => self.border,
            ColorToken::Primary => self.primary,
        }
    }

    pub fn spacing(&self, token: SpacingToken) -> f32 {
        match token {
            SpacingToken::Xs => self.spacing_xs,
            SpacingToken::Sm => self.spacing_sm,
            SpacingToken::Md => self.spacing_md,
            SpacingToken::Lg => self.spacing_lg,
            SpacingToken::Xl => self.spacing_xl,
        }
    }

    pub fn radius(&self, token: RadiusToken) -> f32 {
        match token {
            RadiusToken::Sm => self.radius_sm,
            RadiusToken::Md => self.radius_md,
            RadiusToken::Lg => self.radius_lg,
        }
    }

    pub fn font_size(&self, token: FontSizeToken) -> f32 {
        match token {
            FontSizeToken::Sm => self.font_size_sm,
            FontSizeToken::Md => self.font_size_md,
            FontSizeToken::Lg => self.font_size_lg,
            FontSizeToken::Xl => self.font_size_xl,
        }
    }
}

fn merge_text(target: &mut TextStylePatch, other: &TextStylePatch) {
    merge_value(&mut target.color, &other.color);
    merge_value(&mut target.font_family, &other.font_family);
    merge_value(&mut target.font_size, &other.font_size);
    merge_value(&mut target.font_weight, &other.font_weight);
    merge_value(&mut target.font_style, &other.font_style);
    merge_value(&mut target.line_height, &other.line_height);
    merge_value(&mut target.letter_spacing, &other.letter_spacing);
    merge_value(&mut target.decoration, &other.decoration);
}

fn merge_layout(target: &mut LayoutStylePatch, other: &LayoutStylePatch) {
    // merge_value(&mut target.flex_direction, &other.flex_direction);
    merge_value(&mut target.gap, &other.gap);
    merge_value(&mut target.size, &other.size);
    merge_value(&mut target.min_size, &other.min_size);
    merge_value(&mut target.max_size, &other.max_size);
    merge_value(&mut target.margin, &other.margin);
    merge_value(&mut target.padding, &other.padding);
    merge_value(&mut target.align, &other.align);
    merge_value(&mut target.justify, &other.justify);
}

fn merge_paint(target: &mut PaintStylePatch, other: &PaintStylePatch) {
    merge_value(&mut target.background, &other.background);
    merge_value(&mut target.border_color, &other.border_color);
    merge_value(&mut target.border_width, &other.border_width);
    merge_value(&mut target.border_radius, &other.border_radius);
    merge_value(&mut target.stroke, &other.stroke);
    merge_value(&mut target.shadow, &other.shadow);
    merge_value(&mut target.clip, &other.clip);
}

fn merge_scroll(target: &mut ScrollStylePatch, other: &ScrollStylePatch) {
    merge_value(&mut target.direction, &other.direction);
    merge_value(&mut target.scrollbar, &other.scrollbar);
}

fn merge_value<T: Clone>(target: &mut StyleValue<T>, other: &StyleValue<T>) {
    if !matches!(other, StyleValue::Unset) {
        *target = other.clone();
    }
}

fn apply_text(
    target: &mut ComputedTextStyle,
    parent: &ComputedTextStyle,
    patch: &TextStylePatch,
    theme: &Theme,
) {
    target.color = resolve_color(
        patch.color,
        target.color,
        parent.color,
        theme.color(ColorToken::Text),
        theme,
    );
    target.font_family = resolve_clone(
        &patch.font_family,
        &target.font_family,
        &parent.font_family,
        &FontFamily::System,
    );
    target.font_size = resolve_length(
        patch.font_size,
        target.font_size,
        parent.font_size,
        theme.font_size(FontSizeToken::Md),
        theme,
    );
    target.font_weight = resolve_copy(
        patch.font_weight,
        target.font_weight,
        parent.font_weight,
        FontWeight::Normal,
    );
    target.font_style = resolve_copy(
        patch.font_style,
        target.font_style,
        parent.font_style,
        FontStyle::Normal,
    );
    target.line_height = resolve_copy(
        patch.line_height,
        target.line_height,
        parent.line_height,
        LineHeight::Normal,
    );
    target.letter_spacing = resolve_length(
        patch.letter_spacing,
        target.letter_spacing,
        parent.letter_spacing,
        0.0,
        theme,
    );
    target.decoration = resolve_copy(
        patch.decoration,
        target.decoration,
        parent.decoration,
        TextDecoration::default(),
    );
}

fn apply_layout(target: &mut ComputedLayoutStyle, patch: &LayoutStylePatch, theme: &Theme) {
    let initial = ComputedStyle::initial(theme).layout;
    // target.flex_direction = resolve_copy_no_inherit(
    //     patch.flex_direction,
    //     target.flex_direction,
    //     initial.flex_direction,
    // );
    target.gap = resolve_length_no_inherit(patch.gap, target.gap, initial.gap, theme);
    target.size = resolve_optional_size_no_inherit(patch.size, target.size, initial.size);
    target.min_size =
        resolve_optional_size_no_inherit(patch.min_size, target.min_size, initial.min_size);
    target.max_size =
        resolve_optional_size_no_inherit(patch.max_size, target.max_size, initial.max_size);
    target.margin = resolve_copy_no_inherit(patch.margin, target.margin, initial.margin);
    target.padding = resolve_copy_no_inherit(patch.padding, target.padding, initial.padding);
    target.align = resolve_copy_no_inherit(patch.align, target.align, initial.align);
    target.justify = resolve_copy_no_inherit(patch.justify, target.justify, initial.justify);
}

fn apply_paint(target: &mut ComputedPaintStyle, patch: &PaintStylePatch, theme: &Theme) {
    let initial = ComputedStyle::initial(theme).paint;
    target.background = resolve_color_style_no_inherit(
        patch.background,
        target.background,
        initial.background,
        theme,
    );
    target.border_color = resolve_color_style_no_inherit(
        patch.border_color,
        target.border_color,
        initial.border_color,
        theme,
    );
    target.border_width = resolve_length_no_inherit(
        patch.border_width,
        target.border_width,
        initial.border_width,
        theme,
    );
    target.border_radius = resolve_length_no_inherit(
        patch.border_radius,
        target.border_radius,
        initial.border_radius,
        theme,
    );
    target.stroke = resolve_stroke_no_inherit(
        patch.stroke,
        target.stroke,
        initial.stroke,
        target.border_color,
        target.border_width,
        theme,
    );
    target.shadow = resolve_shadow_no_inherit(patch.shadow, target.shadow, initial.shadow, theme);
    target.clip = resolve_copy_no_inherit(patch.clip, target.clip, initial.clip);
}

fn apply_scroll(target: &mut ComputedScrollStyle, patch: &ScrollStylePatch, theme: &Theme) {
    let initial = ComputedStyle::initial(theme).scroll;
    target.direction =
        resolve_copy_no_inherit(patch.direction, target.direction, initial.direction);
    target.scrollbar =
        resolve_scrollbar_no_inherit(patch.scrollbar, target.scrollbar, initial.scrollbar, theme);
}

fn resolve_scrollbar_no_inherit(
    value: StyleValue<ScrollbarStyle>,
    current: ComputedScrollbarStyle,
    initial: ComputedScrollbarStyle,
    theme: &Theme,
) -> ComputedScrollbarStyle {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => ComputedScrollbarStyle {
            width: length_value(value.width, theme),
            track_color: color_style(value.track_color, theme),
            thumb_color: color_style(value.thumb_color, theme),
            radius: length_value(value.radius, theme),
            visibility: value.visibility,
        },
    }
}

fn resolve_stroke_no_inherit(
    value: StyleValue<StrokeStyle>,
    current: Option<ComputedStrokeStyle>,
    initial: Option<ComputedStrokeStyle>,
    border_color: ComputedColorStyle,
    border_width: f32,
    theme: &Theme,
) -> Option<ComputedStrokeStyle> {
    match value {
        StyleValue::Unset | StyleValue::Inherit => {
            if border_width > 0.0 && border_color.is_visible() {
                Some(ComputedStrokeStyle {
                    color: border_color,
                    width: border_width,
                    line_style: StrokeLineStyle::Solid,
                })
            } else {
                current
            }
        }
        StyleValue::Initial => initial,
        StyleValue::Value(value) => {
            let color = color_style(value.color, theme);
            (length_value(value.width, theme) > 0.0 && color.is_visible()).then_some(
                ComputedStrokeStyle {
                    color,
                    width: length_value(value.width, theme),
                    line_style: value.line_style,
                },
            )
        }
    }
}

fn resolve_shadow_no_inherit(
    value: StyleValue<ShadowStyle>,
    current: Option<ComputedShadowStyle>,
    initial: Option<ComputedShadowStyle>,
    theme: &Theme,
) -> Option<ComputedShadowStyle> {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => Some(ComputedShadowStyle {
            color: color_value(value.color, theme),
            offset: value.offset,
            blur: length_value(value.blur, theme),
            spread: length_value(value.spread, theme),
        }),
    }
}

fn resolve_color(
    value: StyleValue<ColorValue>,
    current: Color,
    inherited: Color,
    initial: Color,
    theme: &Theme,
) -> Color {
    match value {
        StyleValue::Unset => current,
        StyleValue::Inherit => inherited,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => color_value(value, theme),
    }
}

fn resolve_color_style_no_inherit(
    value: StyleValue<ColorStyle>,
    current: ComputedColorStyle,
    initial: ComputedColorStyle,
    theme: &Theme,
) -> ComputedColorStyle {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => color_style(value, theme),
    }
}

fn color_value(value: ColorValue, theme: &Theme) -> Color {
    match value {
        ColorValue::Color(color) => color,
        ColorValue::Token(token) => theme.color(token),
    }
}

fn color_style(value: ColorStyle, theme: &Theme) -> ComputedColorStyle {
    match value {
        ColorStyle::Solid(color) => ComputedColorStyle::Solid(color_value(color, theme)),
        ColorStyle::LinearGradient(gradient) => {
            ComputedColorStyle::LinearGradient(ComputedLinearGradientStyle {
                start: gradient.start,
                end: gradient.end,
                from: color_value(gradient.from, theme),
                to: color_value(gradient.to, theme),
            })
        }
        ColorStyle::RadialGradient(gradient) => {
            ComputedColorStyle::RadialGradient(ComputedRadialGradientStyle {
                center: gradient.center,
                radius: length_value(gradient.radius, theme),
                from: color_value(gradient.from, theme),
                to: color_value(gradient.to, theme),
            })
        }
    }
}

fn resolve_length(
    value: StyleValue<LengthValue>,
    current: f32,
    inherited: f32,
    initial: f32,
    theme: &Theme,
) -> f32 {
    match value {
        StyleValue::Unset => current,
        StyleValue::Inherit => inherited,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => length_value(value, theme),
    }
}

fn resolve_length_no_inherit(
    value: StyleValue<LengthValue>,
    current: f32,
    initial: f32,
    theme: &Theme,
) -> f32 {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => length_value(value, theme),
    }
}

fn length_value(value: LengthValue, theme: &Theme) -> f32 {
    match value {
        LengthValue::Px(value) => value,
        LengthValue::Spacing(token) => theme.spacing(token),
        LengthValue::Radius(token) => theme.radius(token),
        LengthValue::FontSize(token) => theme.font_size(token),
    }
}

fn resolve_copy<T: Copy>(value: StyleValue<T>, current: T, inherited: T, initial: T) -> T {
    match value {
        StyleValue::Unset => current,
        StyleValue::Inherit => inherited,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => value,
    }
}

fn resolve_clone<T: Clone>(value: &StyleValue<T>, current: &T, inherited: &T, initial: &T) -> T {
    match value {
        StyleValue::Unset => current.clone(),
        StyleValue::Inherit => inherited.clone(),
        StyleValue::Initial => initial.clone(),
        StyleValue::Value(value) => value.clone(),
    }
}

fn resolve_copy_no_inherit<T: Copy>(value: StyleValue<T>, current: T, initial: T) -> T {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => value,
    }
}

fn resolve_optional_size_no_inherit(
    value: StyleValue<Size<Sizing>>,
    current: Option<Size<Sizing>>,
    initial: Option<Size<Sizing>>,
) -> Option<Size<Sizing>> {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => Some(value),
    }
}

fn hash_color<H: Hasher>(color: Color, state: &mut H) {
    color.r.to_bits().hash(state);
    color.g.to_bits().hash(state);
    color.b.to_bits().hash(state);
    color.a.to_bits().hash(state);
}

fn hash_edge_insets<H: Hasher>(value: EdgeInsets, state: &mut H) {
    value.left.to_bits().hash(state);
    value.right.to_bits().hash(state);
    value.top.to_bits().hash(state);
    value.bottom.to_bits().hash(state);
}

fn hash_size<H: Hasher>(value: Size<Sizing>, state: &mut H) {
    value.width.hash(state);
    value.height.hash(state);
}

fn hash_point<H: Hasher>(value: Point, state: &mut H) {
    value.x.to_bits().hash(state);
    value.y.to_bits().hash(state);
}

impl Hash for ColorValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Color(color) => hash_color(*color, state),
            Self::Token(token) => token.hash(state),
        }
    }
}

impl Hash for ColorStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Solid(color) => color.hash(state),
            Self::LinearGradient(gradient) => gradient.hash(state),
            Self::RadialGradient(gradient) => gradient.hash(state),
        }
    }
}

impl Hash for LinearGradientStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_point(self.start, state);
        hash_point(self.end, state);
        self.from.hash(state);
        self.to.hash(state);
    }
}

impl Hash for RadialGradientStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_point(self.center, state);
        self.radius.hash(state);
        self.from.hash(state);
        self.to.hash(state);
    }
}

impl Hash for LengthValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Px(value) => value.to_bits().hash(state),
            Self::Spacing(token) => token.hash(state),
            Self::Radius(token) => token.hash(state),
            Self::FontSize(token) => token.hash(state),
        }
    }
}

impl Hash for Style {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.layout.hash(state);
        self.paint.hash(state);
        self.scroll.hash(state);
    }
}

impl Hash for TextStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.color.hash(state);
        self.font_family.hash(state);
        self.font_size.hash(state);
        self.font_weight.hash(state);
        self.font_style.hash(state);
        self.line_height.hash(state);
        self.letter_spacing.hash(state);
        self.decoration.hash(state);
    }
}

impl Hash for LayoutStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // self.flex_direction.hash(state);
        self.gap.hash(state);
        hash_style_value_size(&self.size, state);
        hash_style_value_size(&self.min_size, state);
        hash_style_value_size(&self.max_size, state);
        hash_style_value_edge_insets(&self.margin, state);
        hash_style_value_edge_insets(&self.padding, state);
    }
}

impl Hash for PaintStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.background.hash(state);
        self.border_color.hash(state);
        self.border_width.hash(state);
        self.border_radius.hash(state);
        hash_style_value_stroke(&self.stroke, state);
        hash_style_value_shadow(&self.shadow, state);
        self.clip.hash(state);
    }
}

impl Hash for ScrollStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.direction.hash(state);
        hash_style_value_scrollbar(&self.scrollbar, state);
    }
}

fn hash_style_value_size<H: Hasher>(value: &StyleValue<Size<Sizing>>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        hash_size(*value, state);
    }
}

fn hash_style_value_edge_insets<H: Hasher>(value: &StyleValue<EdgeInsets>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        hash_edge_insets(*value, state);
    }
}

fn hash_style_value_stroke<H: Hasher>(value: &StyleValue<StrokeStyle>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        value.color.hash(state);
        value.width.hash(state);
        value.line_style.hash(state);
    }
}

fn hash_style_value_shadow<H: Hasher>(value: &StyleValue<ShadowStyle>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        value.color.hash(state);
        hash_point(value.offset, state);
        value.blur.hash(state);
        value.spread.hash(state);
    }
}

fn hash_style_value_scrollbar<H: Hasher>(value: &StyleValue<ScrollbarStyle>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        value.width.hash(state);
        value.track_color.hash(state);
        value.thumb_color.hash(state);
        value.radius.hash(state);
        value.visibility.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_text_properties_flow_from_parent() {
        let theme = Theme::default();
        let mut parent = ComputedStyle::initial(&theme);
        parent.apply(
            &ComputedStyle::initial(&theme),
            &Style::new().color(Color::BLUE_500).font_size(20.0),
            &theme,
        );

        let mut child = parent.inherited_from(&theme);
        child.apply(&parent, &Style::new(), &theme);

        assert_eq!(child.text.color, Color::BLUE_500);
        assert_eq!(child.text.font_size, 20.0);
        assert_eq!(
            child.paint.background,
            ComputedColorStyle::Solid(Color::TRANSPARENT)
        );
        assert_eq!(child.layout.padding, EdgeInsets::ZERO);
    }

    #[test]
    fn initial_resets_inheritable_values() {
        let theme = Theme::default();
        let mut parent = ComputedStyle::initial(&theme);
        parent.apply(
            &ComputedStyle::initial(&theme),
            &Style::new().font_size(20.0),
            &theme,
        );

        let mut patch = Style::new();
        patch.text.font_size = StyleValue::Initial;
        let mut child = parent.inherited_from(&theme);
        child.apply(&parent, &patch, &theme);

        assert_eq!(child.text.font_size, theme.font_size(FontSizeToken::Md));
    }

    #[test]
    fn tokens_resolve_from_theme() {
        let mut theme = Theme::default();
        theme.primary = Color::rgb(0.2, 0.3, 0.4);
        theme.spacing_lg = 18.0;

        let initial = ComputedStyle::initial(&theme);
        let mut computed = initial.inherited_from(&theme);
        computed.apply(
            &initial,
            &Style::new()
                .background(ColorToken::Primary)
                .gap(SpacingToken::Lg),
            &theme,
        );

        assert_eq!(
            computed.paint.background,
            ComputedColorStyle::Solid(Color::rgb(0.2, 0.3, 0.4))
        );
        assert_eq!(computed.layout.gap, 18.0);
    }

    #[test]
    fn width_and_height_create_size_patch_when_unset() {
        let width_style = Style::new().width(Sizing::fix(300.0));
        assert_eq!(
            width_style.layout.size,
            StyleValue::Value(Size::<Sizing>::new(Sizing::fix(300.0), Sizing::Hug))
        );

        let height_style = Style::new().height(Sizing::Fill);
        assert_eq!(
            height_style.layout.size,
            StyleValue::Value(Size::<Sizing>::new(Sizing::Hug, Sizing::Fill))
        );
    }

    #[test]
    fn gradient_color_styles_resolve_tokens() {
        let mut theme = Theme::default();
        theme.primary = Color::rgb(0.2, 0.3, 0.4);
        theme.background = Color::WHITE;

        let initial = ComputedStyle::initial(&theme);
        let mut computed = initial.inherited_from(&theme);
        computed.apply(
            &initial,
            &Style::new().background(ColorStyle::linear_gradient(
                Point::new(0.0, 0.0),
                Point::new(1.0, 1.0),
                ColorToken::Primary,
                ColorToken::Background,
            )),
            &theme,
        );

        assert_eq!(
            computed.paint.background,
            ComputedColorStyle::LinearGradient(ComputedLinearGradientStyle {
                start: Point::new(0.0, 0.0),
                end: Point::new(1.0, 1.0),
                from: Color::rgb(0.2, 0.3, 0.4),
                to: Color::WHITE,
            })
        );
    }

    #[test]
    fn shadow_style_resolves_from_theme_and_can_reset() {
        let mut theme = Theme::default();
        theme.primary = Color::rgba(0.1, 0.2, 0.3, 0.4);
        theme.spacing_sm = 10.0;

        let initial = ComputedStyle::initial(&theme);
        let mut computed = initial.inherited_from(&theme);
        computed.apply(
            &initial,
            &Style::new().box_shadow(
                ColorToken::Primary,
                Point::new(2.0, 4.0),
                SpacingToken::Sm,
                3.0,
            ),
            &theme,
        );

        let shadow = computed.paint.shadow.unwrap();
        assert_eq!(shadow.color, Color::rgba(0.1, 0.2, 0.3, 0.4));
        assert_eq!(shadow.offset, Point::new(2.0, 4.0));
        assert_eq!(shadow.blur, 10.0);
        assert_eq!(shadow.spread, 3.0);

        computed.apply(&initial, &Style::new().no_shadow(), &theme);

        assert_eq!(computed.paint.shadow, None);
    }

    #[test]
    fn animated_style_records_event_triggered_style_animation() {
        let transition = AnimationTransition::new(Duration::from_millis(120))
            .delay(Duration::from_millis(20))
            .ease(AnimationEasing::CubicOut);
        let animated = AnimatedStyle::new(Style::new().background(Color::BLACK))
            .on_hover(Style::new().background(Color::WHITE), transition);

        assert_eq!(
            animated.base.paint.background,
            StyleValue::Value(ColorStyle::Solid(ColorValue::Color(Color::BLACK)))
        );
        assert_eq!(animated.animations.len(), 1);
        assert_eq!(animated.animations[0].trigger, EventTrigger::OnHover);
        assert_eq!(animated.animations[0].transition, transition);
        assert_eq!(
            animated.animations[0].style.paint.background,
            StyleValue::Value(ColorStyle::Solid(ColorValue::Color(Color::WHITE)))
        );
    }
}
