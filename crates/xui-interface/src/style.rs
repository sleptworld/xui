//! The computed style — what a node shows once its declarations resolve —
//! and the theme they resolve against.
//!
//! Declarations themselves live with the nodes: each style capability in
//! `xui-core` declares its own fields in the units of [`crate::styles`] and
//! resolves them into the [`ComputedStyle`] here.

use crate::{
    Affine, Bounds, Color, EdgeInsets, FontFamily, FontStyle, FontWeight, LineHeight, Point, Size,
    StyleDiffFlags, TextDecoration, WidgetState, core::Sizing, text::TextStyle,
};

use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

pub use crate::styles::computed::*;

pub use crate::styles::units::*;

/// The pointer shape a node asks the platform for.
///
/// A style property, so that it can be state-conditioned like any other —
/// `style!(cursor: if disabled { NotAllowed } else { Pointer })` — and so that
/// `cursor={..}` works on every widget without the DSL knowing about it.
///
/// It is deliberately absent from [`ComputedStyle::diff`]: a cursor produces no
/// scene output, so changing one must not invalidate layout, paint, or text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CursorIcon {
    #[default]
    Default,
    /// The "this is clickable" hand.
    Pointer,
    /// An I-beam, for selectable text.
    Text,
    Crosshair,
    Move,
    /// Something can be picked up here.
    Grab,
    /// Something is being dragged right now.
    Grabbing,
    NotAllowed,
    Wait,
    Progress,
    Help,
    ColumnResize,
    RowResize,
    EastWestResize,
    NorthSouthResize,
    /// Draw nothing at all.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetStateMatcher {
    required: WidgetState,
    forbidden: WidgetState,
}

impl WidgetStateMatcher {
    pub fn new(required: WidgetState, forbidden: WidgetState) -> Self {
        Self {
            required,
            forbidden,
        }
    }

    pub fn all(required: WidgetState) -> Self {
        Self::new(required, WidgetState::empty())
    }

    pub fn matches(&self, state: WidgetState) -> bool {
        state.contains(self.required) && !state.intersects(self.forbidden)
    }

    pub fn matches_state_change(&self, before: WidgetState, after: WidgetState) -> bool {
        self.matches(before) != self.matches(after)
    }

    pub fn dependencies(&self) -> WidgetState {
        self.required | self.forbidden
    }
}

/// Computed Style
/// It's real
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
    // pub size: Size<Sizing>,
    pub width: Sizing,
    pub height: Sizing,
    pub min_height: Option<Sizing>,
    pub max_height: Option<Sizing>,
    pub min_width: Option<Sizing>,
    pub max_width: Option<Sizing>,
    // pub min_size: Option<Size<Sizing>>,
    // pub max_size: Option<Size<Sizing>>,
    pub margin: EdgeInsets,
    pub padding: EdgeInsets,
    pub align: AlignStyle,
    pub justify: JustifyStyle,
    pub position: PositionStyle,
    /// Explicit positioning offsets. `None` maps to Taffy's `auto` inset on
    /// every edge, which preserves the intrinsic size of absolute children.
    pub inset: Option<EdgeInsets>,
    pub display: DisplayStyle,
}

impl ComputedLayoutStyle {
    pub fn size(&self) -> Size<Sizing> {
        Size::<Sizing>::new(self.width, self.height)
    }

    pub fn min_size(&self) -> Size<Option<Sizing>> {
        Size::new(self.min_width, self.min_height)
    }

    pub fn max_size(&self) -> Size<Option<Sizing>> {
        Size::new(self.max_width, self.max_height)
    }
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
    pub visibility: VisibilityStyle,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ComputedEffectStyle {
    pub backdrop: Option<ComputedBackdropStyle>,
    pub effects: Arc<[ComputedEffect]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedScrollStyle {
    pub direction: ScrollDirectionStyle,
    pub scrollbar: ComputedScrollbarStyle,
}

impl ComputedScrollStyle {
    pub fn is_scrollable(&self) -> bool {
        self.direction != ScrollDirectionStyle::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedScrollbarStyle {
    pub width: f32,
    pub track_color: ComputedColorStyle,
    pub thumb_color: ComputedColorStyle,
    pub radius: f32,
    pub visibility: ScrollbarVisibilityStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub text: ComputedTextStyle,
    pub layout: ComputedLayoutStyle,
    pub paint: ComputedPaintStyle,
    pub transform: TransformStyle,
    pub effect: ComputedEffectStyle,
    pub scroll: ComputedScrollStyle,
    /// `None` means this node does not specify one; the resolver walks up to
    /// the nearest ancestor that does.
    ///
    /// Not inherited through [`ComputedStyle::inherited_from`] on purpose:
    /// inheriting would give every node a resolved copy to keep in step, while
    /// the walk happens only when the hover target changes.
    pub cursor: Option<CursorIcon>,
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
                width: Sizing::Hug,
                height: Sizing::Hug,
                min_height: None,
                min_width: None,
                max_height: None,
                max_width: None,
                margin: EdgeInsets::zero(),
                padding: EdgeInsets::zero(),
                align: AlignStyle::Start,
                justify: JustifyStyle::Start,
                position: PositionStyle::Relative,
                inset: None,
                display: DisplayStyle::Visible,
            },
            paint: ComputedPaintStyle {
                background: ComputedColorStyle::Solid(Color::TRANSPARENT),
                border_color: ComputedColorStyle::Solid(Color::TRANSPARENT),
                border_width: 0.0,
                border_radius: 0.0,
                stroke: None,
                shadow: None,
                clip: false,
                visibility: VisibilityStyle::Visible,
            },
            transform: TransformStyle::IDENTITY,
            effect: ComputedEffectStyle {
                backdrop: None,
                effects: Arc::from([]),
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
            cursor: None,
        }
    }

    pub fn diff(&self, other: &ComputedStyle) -> StyleDiffFlags {
        let mut flags = StyleDiffFlags::empty();

        if self.text != other.text {
            flags |= StyleDiffFlags::TEXT;
        }

        if self.layout != other.layout {
            flags |= StyleDiffFlags::LAYOUT;
        }

        let paint_without_visibility = |paint: &ComputedPaintStyle| ComputedPaintStyle {
            visibility: VisibilityStyle::Visible,
            ..*paint
        };
        if paint_without_visibility(&self.paint) != paint_without_visibility(&other.paint) {
            flags |= StyleDiffFlags::PAINT;
        }

        if self.paint.visibility != other.paint.visibility
            || self.layout.display != other.layout.display
        {
            flags |= StyleDiffFlags::VISIBILITY;
        }

        if self.transform != other.transform {
            flags |= StyleDiffFlags::TRANSFORM;
        }

        if self.effect != other.effect {
            flags |= StyleDiffFlags::EFFECT;
        }

        if self.scroll != other.scroll {
            flags |= StyleDiffFlags::SCROLL;
        }

        if !self.inherited_eq(other) {
            flags |= StyleDiffFlags::INHERITED;
        }

        // `cursor` is intentionally not compared. It has no scene output, so a
        // change must not dirty layout, paint, or text; the platform layer picks
        // the current value up on its next pull. Adding it here would make
        // hovering a button repaint it.

        flags
    }
}

/// Declares the properties a node takes from its parent when its own patch
/// leaves them unset, and derives every inheritance helper from that one list.
///
/// Adding a property here is the whole job of making it inherited: the
/// runtime only ever asks through these helpers and
/// [`StyleDiffFlags::INHERITED`], never names a field. Resolving an explicit
/// `StyleValue::Inherit` in the property's `apply_*` function is separate —
/// that is where it picks its `resolve_*` variant.
macro_rules! inherited_properties {
    ($($group:ident . $field:ident),+ $(,)?) => {
        impl ComputedStyle {
            /// The initial style with every inherited property taken from
            /// `self`: what a child starts from before its own patch applies.
            pub fn inherited_from(&self, theme: &Theme) -> Self {
                let mut computed = Self::initial(theme);
                $(computed.$group.$field = self.$group.$field.clone();)+
                computed
            }

            /// Whether both styles agree on every inherited property, which is
            /// exactly when a child resolves the same against either.
            pub fn inherited_eq(&self, other: &Self) -> bool {
                true $(&& self.$group.$field == other.$group.$field)+
            }
        }
    };
}

inherited_properties! {
    text.color,
    text.font_family,
    text.font_size,
    text.font_weight,
    text.font_style,
    text.line_height,
    text.letter_spacing,
    text.decoration,
}

/// Design tokens resolved by `ColorToken`, `SpacingToken`, `RadiusToken`, and
/// `FontSizeToken`. The defaults follow shadcn/ui's neutral palette;
/// [`Theme::light`] is the default and [`Theme::dark`] its dark counterpart.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub card: Color,
    pub card_foreground: Color,
    pub popover: Color,
    pub popover_foreground: Color,
    pub primary: Color,
    pub primary_foreground: Color,
    pub secondary: Color,
    pub secondary_foreground: Color,
    pub muted: Color,
    pub muted_foreground: Color,
    pub accent: Color,
    pub accent_foreground: Color,
    pub destructive: Color,
    pub destructive_foreground: Color,
    pub border: Color,
    pub input: Color,
    pub ring: Color,
    pub spacing_xs: f32,
    pub spacing_sm: f32,
    pub spacing_md: f32,
    pub spacing_lg: f32,
    pub spacing_xl: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    pub radius_xl: f32,
    pub font_size_sm: f32,
    pub font_size_md: f32,
    pub font_size_lg: f32,
    pub font_size_xl: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}

impl Theme {
    /// shadcn/ui "neutral", light.
    pub fn light() -> Self {
        Self {
            background: Color::hex("#ffffff"),
            foreground: Color::hex("#0a0a0a"),
            card: Color::hex("#ffffff"),
            card_foreground: Color::hex("#0a0a0a"),
            popover: Color::hex("#ffffff"),
            popover_foreground: Color::hex("#0a0a0a"),
            primary: Color::hex("#171717"),
            primary_foreground: Color::hex("#fafafa"),
            secondary: Color::hex("#f5f5f5"),
            secondary_foreground: Color::hex("#171717"),
            muted: Color::hex("#f5f5f5"),
            muted_foreground: Color::hex("#737373"),
            accent: Color::hex("#f5f5f5"),
            accent_foreground: Color::hex("#171717"),
            destructive: Color::hex("#e7000b"),
            destructive_foreground: Color::hex("#ffffff"),
            border: Color::hex("#e5e5e5"),
            input: Color::hex("#e5e5e5"),
            ring: Color::hex("#a1a1a1"),
            // Radii derive from a 10px base radius, as in shadcn/ui.
            spacing_xs: 4.0,
            spacing_sm: 8.0,
            spacing_md: 12.0,
            spacing_lg: 16.0,
            spacing_xl: 24.0,
            radius_sm: 6.0,
            radius_md: 8.0,
            radius_lg: 10.0,
            radius_xl: 14.0,
            font_size_sm: 12.0,
            font_size_md: 14.0,
            font_size_lg: 16.0,
            font_size_xl: 20.0,
        }
    }

    /// shadcn/ui "neutral", dark.
    pub fn dark() -> Self {
        Self {
            background: Color::hex("#0a0a0a"),
            foreground: Color::hex("#fafafa"),
            card: Color::hex("#171717"),
            card_foreground: Color::hex("#fafafa"),
            popover: Color::hex("#171717"),
            popover_foreground: Color::hex("#fafafa"),
            primary: Color::hex("#e5e5e5"),
            primary_foreground: Color::hex("#171717"),
            secondary: Color::hex("#262626"),
            secondary_foreground: Color::hex("#fafafa"),
            muted: Color::hex("#262626"),
            muted_foreground: Color::hex("#a1a1a1"),
            accent: Color::hex("#262626"),
            accent_foreground: Color::hex("#fafafa"),
            // shadcn uses #ff6467 at 60% for dark fills; red-600 keeps white
            // text legible on a solid fill.
            destructive: Color::hex("#dc2626"),
            destructive_foreground: Color::hex("#ffffff"),
            border: Color::rgba(1.0, 1.0, 1.0, 0.10),
            input: Color::rgba(1.0, 1.0, 1.0, 0.15),
            ring: Color::hex("#737373"),
            ..Self::light()
        }
    }

    pub fn color(&self, token: ColorToken) -> Color {
        match token {
            ColorToken::Background => self.background,
            ColorToken::Foreground | ColorToken::Text => self.foreground,
            ColorToken::Card | ColorToken::Surface => self.card,
            ColorToken::CardForeground => self.card_foreground,
            ColorToken::Popover => self.popover,
            ColorToken::PopoverForeground => self.popover_foreground,
            ColorToken::Primary => self.primary,
            ColorToken::PrimaryForeground | ColorToken::InverseText => self.primary_foreground,
            ColorToken::Secondary => self.secondary,
            ColorToken::SecondaryForeground => self.secondary_foreground,
            ColorToken::Muted | ColorToken::MutedSurface => self.muted,
            ColorToken::MutedForeground => self.muted_foreground,
            ColorToken::Accent => self.accent,
            ColorToken::AccentForeground => self.accent_foreground,
            ColorToken::Destructive => self.destructive,
            ColorToken::DestructiveForeground => self.destructive_foreground,
            ColorToken::Border => self.border,
            ColorToken::Input => self.input,
            ColorToken::Ring => self.ring,
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
            RadiusToken::Xl => self.radius_xl,
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

pub(crate) fn compute_backdrop_style(
    style: &BackdropStyle,
    theme: &Theme,
) -> ComputedBackdropStyle {
    ComputedBackdropStyle {
        filters: style
            .filters
            .iter()
            .map(|filter| compute_backdrop_filter(filter, theme))
            .collect::<Vec<_>>()
            .into(),
        opacity: style.opacity.clamp(0.0, 1.0),
        blend_mode: style.blend_mode,
        mask: compute_backdrop_mask(&style.mask, theme),
    }
}

fn compute_backdrop_filter(filter: &BackdropFilter, theme: &Theme) -> ComputedBackdropFilter {
    match filter {
        BackdropFilter::Blur {
            sigma_x,
            sigma_y,
            quality,
        } => ComputedBackdropFilter::Blur {
            sigma_x: non_negative(length_value(*sigma_x, theme)),
            sigma_y: non_negative(length_value(*sigma_y, theme)),
            quality: *quality,
        },
        BackdropFilter::Saturate(value) => ComputedBackdropFilter::Saturate(non_negative(*value)),
        BackdropFilter::Brightness(value) => {
            ComputedBackdropFilter::Brightness(non_negative(*value))
        }
        BackdropFilter::Contrast(value) => ComputedBackdropFilter::Contrast(non_negative(*value)),
        BackdropFilter::Grayscale(value) => {
            ComputedBackdropFilter::Grayscale(value.clamp(0.0, 1.0))
        }
        BackdropFilter::Sepia(value) => ComputedBackdropFilter::Sepia(value.clamp(0.0, 1.0)),
        BackdropFilter::HueRotate(value) => {
            ComputedBackdropFilter::HueRotate(normalize_radians(*value))
        }
        BackdropFilter::Invert(value) => ComputedBackdropFilter::Invert(value.clamp(0.0, 1.0)),
        BackdropFilter::ColorMatrix(matrix) => ComputedBackdropFilter::ColorMatrix(*matrix),
        BackdropFilter::Pixelate { size } => ComputedBackdropFilter::Pixelate {
            size: Size::new(
                non_negative(length_value(size.width, theme)),
                non_negative(length_value(size.height, theme)),
            ),
        },
        BackdropFilter::Refraction {
            strength,
            chromatic_aberration,
        } => ComputedBackdropFilter::Refraction {
            strength: length_value(*strength, theme),
            chromatic_aberration: length_value(*chromatic_aberration, theme).abs(),
        },
        BackdropFilter::ChromaticAberration { offset_x, offset_y } => {
            ComputedBackdropFilter::ChromaticAberration {
                offset: [
                    length_value(*offset_x, theme),
                    length_value(*offset_y, theme),
                ],
            }
        }
    }
}

fn compute_backdrop_mask(mask: &BackdropMask, theme: &Theme) -> ComputedBackdropMask {
    match mask {
        BackdropMask::None => ComputedBackdropMask::None,
        BackdropMask::Shape { shape, transform } => ComputedBackdropMask::Shape {
            shape: match shape {
                MaskShape::Rect => ComputedMaskShape::Rect,
                MaskShape::RoundedRect(radius) => {
                    ComputedMaskShape::RoundedRect(non_negative(length_value(*radius, theme)))
                }
                MaskShape::Circle => ComputedMaskShape::Circle,
                MaskShape::Ellipse => ComputedMaskShape::Ellipse,
                MaskShape::Line { from, to } => ComputedMaskShape::Line {
                    from: *from,
                    to: *to,
                },
            },
            transform: *transform,
        },
        BackdropMask::AlphaTexture { texture, transform } => ComputedBackdropMask::AlphaTexture {
            texture: texture.clone(),
            transform: *transform,
        },
    }
}

pub(crate) fn compute_effect(effect: &Effect, theme: &Theme) -> ComputedEffect {
    match effect {
        Effect::Blur {
            sigma_x,
            sigma_y,
            quality,
        } => ComputedEffect::Blur {
            sigma_x: non_negative(length_value(*sigma_x, theme)),
            sigma_y: non_negative(length_value(*sigma_y, theme)),
            quality: *quality,
        },
        Effect::DropShadow {
            color,
            offset_x,
            offset_y,
            sigma_x,
            sigma_y,
            spread,
            quality,
        } => ComputedEffect::DropShadow {
            color: color_value(*color, theme),
            offset: Point::new(
                length_value(*offset_x, theme),
                length_value(*offset_y, theme),
            ),
            sigma_x: non_negative(length_value(*sigma_x, theme)),
            sigma_y: non_negative(length_value(*sigma_y, theme)),
            spread: non_negative(length_value(*spread, theme)),
            quality: *quality,
        },
        Effect::ColorMatrix(matrix) => ComputedEffect::ColorMatrix(*matrix),
        Effect::ImageMask {
            image,
            data,
            bounds,
        } => ComputedEffect::ImageMask {
            image: image.clone(),
            data: data.clone(),
            bounds: *bounds,
        },
    }
}

fn non_negative(value: f32) -> f32 {
    if value < 0.0 { 0.0 } else { value }
}

fn normalize_radians(value: f32) -> f32 {
    if value.is_finite() {
        let normalized =
            (value + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        if normalized.abs() <= 1.0e-6 {
            0.0
        } else {
            normalized
        }
    } else {
        value
    }
}

pub(crate) fn color_value(value: ColorValue, theme: &Theme) -> Color {
    match value {
        ColorValue::Color(color) => color,
        ColorValue::Token(token) => theme.color(token),
        ColorValue::TokenAlpha(token, alpha) => {
            let color = theme.color(token);
            color.alpha(color.a * alpha)
        }
    }
}

pub(crate) fn color_style(value: ColorStyle, theme: &Theme) -> ComputedColorStyle {
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

pub(crate) fn length_value(value: LengthValue, theme: &Theme) -> f32 {
    match value {
        LengthValue::Px(value) => value,
        LengthValue::Spacing(token) => theme.spacing(token),
        LengthValue::Radius(token) => theme.radius(token),
        LengthValue::FontSize(token) => theme.font_size(token),
    }
}

fn hash_color<H: Hasher>(color: Color, state: &mut H) {
    color.r.to_bits().hash(state);
    color.g.to_bits().hash(state);
    color.b.to_bits().hash(state);
    color.a.to_bits().hash(state);
}

fn hash_f32<H: Hasher>(value: f32, state: &mut H) {
    let bits = if value == 0.0 {
        0.0f32.to_bits()
    } else {
        value.to_bits()
    };
    bits.hash(state);
}

fn hash_affine<H: Hasher>(value: Affine, state: &mut H) {
    for component in [value.xx, value.yx, value.xy, value.yy, value.dx, value.dy] {
        hash_f32(component, state);
    }
}

fn hash_rect<H: Hasher>(value: Bounds, state: &mut H) {
    for component in [value.min.x, value.min.y, value.max.x, value.max.y] {
        hash_f32(component, state);
    }
}

fn hash_color_matrix<H: Hasher>(matrix: &ColorMatrix, state: &mut H) {
    for value in matrix {
        hash_f32(*value, state);
    }
}

fn hash_point<H: Hasher>(value: Point, state: &mut H) {
    hash_f32(value.x, state);
    hash_f32(value.y, state);
}

impl Hash for ColorValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Color(color) => hash_color(*color, state),
            Self::Token(token) => token.hash(state),
            Self::TokenAlpha(token, alpha) => {
                token.hash(state);
                hash_f32(*alpha, state);
            }
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

impl Hash for MaskShape {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::RoundedRect(radius) => radius.hash(state),
            Self::Line { from, to } => {
                hash_point(*from, state);
                hash_point(*to, state);
            }
            Self::Rect | Self::Circle | Self::Ellipse => {}
        }
    }
}

impl Hash for BackdropMask {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::None => {}
            Self::Shape { shape, transform } => {
                shape.hash(state);
                hash_affine(*transform, state);
            }
            Self::AlphaTexture { texture, transform } => {
                texture.hash(state);
                hash_affine(*transform, state);
            }
        }
    }
}

impl Hash for BackdropFilter {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Blur {
                sigma_x,
                sigma_y,
                quality,
            } => {
                sigma_x.hash(state);
                sigma_y.hash(state);
                quality.hash(state);
            }
            Self::Saturate(value)
            | Self::Brightness(value)
            | Self::Contrast(value)
            | Self::Grayscale(value)
            | Self::Sepia(value)
            | Self::HueRotate(value)
            | Self::Invert(value) => hash_f32(*value, state),
            Self::ColorMatrix(matrix) => hash_color_matrix(matrix, state),
            Self::Pixelate { size } => {
                size.width.hash(state);
                size.height.hash(state);
            }
            Self::Refraction {
                strength,
                chromatic_aberration,
            } => {
                strength.hash(state);
                chromatic_aberration.hash(state);
            }
            Self::ChromaticAberration { offset_x, offset_y } => {
                offset_x.hash(state);
                offset_y.hash(state);
            }
        }
    }
}

impl Hash for BackdropStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.filters.hash(state);
        hash_f32(self.opacity, state);
        self.blend_mode.hash(state);
        self.mask.hash(state);
    }
}

impl Hash for Effect {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Blur {
                sigma_x,
                sigma_y,
                quality,
            } => {
                sigma_x.hash(state);
                sigma_y.hash(state);
                quality.hash(state);
            }
            Self::DropShadow {
                color,
                offset_x,
                offset_y,
                sigma_x,
                sigma_y,
                spread,
                quality,
            } => {
                color.hash(state);
                offset_x.hash(state);
                offset_y.hash(state);
                sigma_x.hash(state);
                sigma_y.hash(state);
                spread.hash(state);
                quality.hash(state);
            }
            Self::ColorMatrix(matrix) => hash_color_matrix(matrix, state),
            Self::ImageMask {
                image,
                data,
                bounds,
            } => {
                image.hash(state);
                data.id().hash(state);
                hash_rect(*bounds, state);
            }
        }
    }
}
