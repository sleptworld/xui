//! The style capabilities: the smallest groups of style properties a widget
//! selects.
//!
//! A group is what one kind of widget either has entirely or lacks entirely,
//! so a widget's style surface is the list of groups it names. Each group's
//! properties live in its `props` module, named like their attributes.

use std::sync::Arc;

use crate::capability::{StyleProperty, style_capability};
use xui_interface::core::{EdgeInsets, Point, Size, Sizing};
use xui_interface::style::{ComputedStyle, CursorIcon, Theme};
use xui_interface::styles::compute::resolve_field;
use xui_interface::styles::computed::ComputedStrokeStyle;
use xui_interface::styles::units::{
    AlignStyle, BackdropStyle, ColorStyle, ColorValue, DisplayStyle, Effect, JustifyStyle,
    LengthValue, PositionStyle, ScrollDirectionStyle, ScrollbarVisibilityStyle, ShadowStyle,
    ScrollbarStyle, StrokeLineStyle, StrokeStyle, StyleValue, TransformStyle, VisibilityStyle,
};
use xui_interface::{FontFamily, FontStyle, FontWeight, LineHeight, TextDecoration};

style_capability! {
    /// The border-box size and its bounds.
    pub mod box_size: BoxSize {
        width: Sizing => layout.width,
        height: Sizing => layout.height,
        min_width: Sizing => layout.min_width,
        max_width: Sizing => layout.max_width,
        min_height: Sizing => layout.min_height,
        max_height: Sizing => layout.max_height,
    }
    extra(box_size_shorthands { size, min_size, max_size })
}

style_capability! {
    /// Space outside the border box.
    pub mod margin: Margin {
        margin: EdgeInsets => layout.margin,
    }
}

style_capability! {
    /// Space between the border box and the content.
    pub mod padding: Padding {
        padding: EdgeInsets => layout.padding,
    }
}

style_capability! {
    /// How a container arranges its children along and across its axis.
    pub mod flex_layout: FlexLayout {
        gap: LengthValue => layout.gap,
        align: AlignStyle => layout.align,
        justify: JustifyStyle => layout.justify,
    }
}

style_capability! {
    /// Taking the node out of flow and offsetting it.
    pub mod position: Position {
        position: PositionStyle => layout.position,
        inset: EdgeInsets => layout.inset,
    }
}

style_capability! {
    /// Whether the node takes part in layout at all.
    pub mod display: Display {
        display: DisplayStyle => layout.display,
    }
}

style_capability! {
    /// Text appearance. Every property is inherited.
    pub mod typography: Typography {
        color: ColorValue => text.color [inherited],
        font_family: FontFamily => text.font_family [inherited],
        font_size: LengthValue => text.font_size [inherited],
        font_weight: FontWeight => text.font_weight [inherited],
        font_style: FontStyle => text.font_style [inherited],
        line_height: LineHeight => text.line_height [inherited],
        letter_spacing: LengthValue => text.letter_spacing [inherited],
        decoration: TextDecoration => text.decoration [inherited],
    }
}

style_capability! {
    /// The fill behind the content.
    pub mod background: Background {
        background: ColorStyle => paint.background,
    }
}

style_capability! {
    /// Rounding of the border box, shared by the background, border and clip.
    pub mod corner_radius: CornerRadius {
        border_radius: LengthValue => paint.border_radius,
    }
}

style_capability! {
    /// The line around the border box.
    pub mod border: Border {
        border_color: ColorStyle => paint.border_color,
        border_width: LengthValue => paint.border_width,
        /// Left unset, a visible `border_color` + `border_width` pair is
        /// drawn as a solid stroke.
        stroke: StrokeStyle => paint.stroke [resolve = resolve_stroke],
    }
    extra(border_shorthands { stroke_style })
}

style_capability! {
    /// The shadow cast by the border box.
    pub mod box_shadow: BoxShadow {
        shadow: ShadowStyle => paint.shadow,
    }
    extra(box_shadow_shorthands { box_shadow })
}

style_capability! {
    /// Clipping children to the border box.
    pub mod clip: Clip {
        clip: bool => paint.clip,
    }
}

style_capability! {
    /// Whether a laid-out node is drawn.
    pub mod visibility: Visibility {
        visibility: VisibilityStyle => paint.visibility,
    }
}

style_capability! {
    /// The visual transform applied after layout.
    pub mod transform: Transform {
        translate_x: f32 => transform.translate.x,
        translate_y: f32 => transform.translate.y,
        scale_x: f32 => transform.scale.x,
        scale_y: f32 => transform.scale.y,
        /// Clockwise, in radians.
        rotate: f32 => transform.rotate,
        /// Normalized within the laid-out bounds.
        origin_x: f32 => transform.origin.x,
        origin_y: f32 => transform.origin.y,
    }
    extra(transform_shorthands { transform, translate, scale, scale_xy, transform_origin })
}

style_capability! {
    /// Backdrop filtering and layer effects.
    pub mod effects: Effects {
        backdrop: BackdropStyle => effect.backdrop,
        effects: Arc<[Effect]> => effect.effects,
    }
    extra(effects_shorthands { backdrop_blur })
}

style_capability! {
    /// Scrolling and the scrollbar.
    pub mod scroll: Scroll {
        scroll_direction: ScrollDirectionStyle => scroll.direction,
        scrollbar_width: LengthValue => scroll.scrollbar.width,
        scrollbar_track_color: ColorStyle => scroll.scrollbar.track_color,
        scrollbar_thumb_color: ColorStyle => scroll.scrollbar.thumb_color,
        scrollbar_radius: LengthValue => scroll.scrollbar.radius,
        scrollbar_visibility: ScrollbarVisibilityStyle => scroll.scrollbar.visibility,
    }
    extra(scroll_shorthands { scrollbar_style })
}

style_capability! {
    /// The pointer shown over the node. Changing it dirties nothing.
    pub mod cursor: Cursor {
        cursor: CursorIcon => cursor,
    }
}

/// A write-only property setting several of one capability's fields at once.
macro_rules! cap_shorthand {
    ($(#[$meta:meta])* $name:ident : $cap:ty, $value:ty, |$storage:ident, $v:ident| $body:expr) => {
        $(#[$meta])*
        pub enum $name {}

        impl crate::property::Property for $name {
            type Scope = crate::property::CapScope<$cap>;
            type Requirement = crate::property::NotRequired;
            type Value = $value;

            fn set($storage: &mut <$cap as crate::property::Capability>::Storage, $v: $value) {
                $body;
            }
        }
    };
}

/// [`cap_shorthand!`], accepting anything `Into` its value.
macro_rules! into_shorthand {
    ($(#[$meta:meta])* $name:ident : $cap:ty, $value:ty, |$storage:ident, $v:ident| $body:expr) => {
        cap_shorthand!($(#[$meta])* $name: $cap, $value, |$storage, $v| $body);

        impl<T: Into<$value>> crate::property::Accepts<T> for $name {
            fn accept(value: T) -> $value {
                value.into()
            }
        }
    };
}

#[allow(non_camel_case_types)]
mod box_size_shorthands {
    use super::*;

    into_shorthand! {
        /// Width and height at once.
        size: BoxSize, Size<Sizing>, |s, size| {
            s.width = StyleValue::Value(size.width);
            s.height = StyleValue::Value(size.height);
        }
    }
    into_shorthand! {
        /// Minimum width and height at once.
        min_size: BoxSize, Size<Sizing>, |s, size| {
            s.min_width = StyleValue::Value(size.width);
            s.min_height = StyleValue::Value(size.height);
        }
    }
    into_shorthand! {
        /// Maximum width and height at once.
        max_size: BoxSize, Size<Sizing>, |s, size| {
            s.max_width = StyleValue::Value(size.width);
            s.max_height = StyleValue::Value(size.height);
        }
    }
}

#[allow(non_camel_case_types)]
mod transform_shorthands {
    use super::*;

    into_shorthand! {
        /// Every transform field at once.
        transform: Transform, TransformStyle, |s, t| {
            s.translate_x = StyleValue::Value(t.translate.x);
            s.translate_y = StyleValue::Value(t.translate.y);
            s.scale_x = StyleValue::Value(t.scale.x);
            s.scale_y = StyleValue::Value(t.scale.y);
            s.rotate = StyleValue::Value(t.rotate);
            s.origin_x = StyleValue::Value(t.origin.x);
            s.origin_y = StyleValue::Value(t.origin.y);
        }
    }
    into_shorthand! {
        translate: Transform, Point, |s, p| {
            s.translate_x = StyleValue::Value(p.x);
            s.translate_y = StyleValue::Value(p.y);
        }
    }
    cap_shorthand! {
        /// The same scale on both axes.
        scale: Transform, f32, |s, k| {
            s.scale_x = StyleValue::Value(k);
            s.scale_y = StyleValue::Value(k);
        }
    }
    impl crate::property::Accepts<f32> for scale {
        fn accept(value: f32) -> f32 {
            value
        }
    }
    into_shorthand! {
        scale_xy: Transform, Point, |s, p| {
            s.scale_x = StyleValue::Value(p.x);
            s.scale_y = StyleValue::Value(p.y);
        }
    }
    into_shorthand! {
        /// Normalized within the laid-out bounds; `(0.5, 0.5)` is the centre.
        transform_origin: Transform, Point, |s, p| {
            s.origin_x = StyleValue::Value(p.x);
            s.origin_y = StyleValue::Value(p.y);
        }
    }
}

#[allow(non_camel_case_types)]
mod border_shorthands {
    use super::*;

    cap_shorthand! {
        /// A stroke from its colour, width and line style.
        stroke_style: Border, (ColorStyle, LengthValue, StrokeLineStyle), |s, v| {
            s.stroke = StyleValue::Value(StrokeStyle {
                color: v.0,
                width: v.1,
                line_style: v.2,
            });
        }
    }
    impl<C: Into<ColorStyle>, W: Into<LengthValue>> crate::property::Accepts<(C, W, StrokeLineStyle)>
        for stroke_style
    {
        fn accept(v: (C, W, StrokeLineStyle)) -> (ColorStyle, LengthValue, StrokeLineStyle) {
            (v.0.into(), v.1.into(), v.2)
        }
    }
}

#[allow(non_camel_case_types)]
mod box_shadow_shorthands {
    use super::*;

    cap_shorthand! {
        /// A shadow from its colour, offset, blur and spread.
        box_shadow: BoxShadow, (ColorValue, Point, LengthValue, LengthValue), |s, v| {
            s.shadow = StyleValue::Value(ShadowStyle {
                color: v.0,
                offset: v.1,
                blur: v.2,
                spread: v.3,
            });
        }
    }
    impl<C, B, S> crate::property::Accepts<(C, Point, B, S)> for box_shadow
    where
        C: Into<ColorValue>,
        B: Into<LengthValue>,
        S: Into<LengthValue>,
    {
        fn accept(v: (C, Point, B, S)) -> (ColorValue, Point, LengthValue, LengthValue) {
            (v.0.into(), v.1, v.2.into(), v.3.into())
        }
    }
}

#[allow(non_camel_case_types)]
mod effects_shorthands {
    use super::*;

    into_shorthand! {
        /// A backdrop blurred by `sigma`.
        backdrop_blur: Effects, LengthValue, |s, sigma| {
            s.backdrop = StyleValue::Value(BackdropStyle::blur(sigma));
        }
    }
}

#[allow(non_camel_case_types)]
mod scroll_shorthands {
    use super::*;

    into_shorthand! {
        /// Every scrollbar field at once.
        scrollbar_style: Scroll, ScrollbarStyle, |s, bar| {
            s.scrollbar_width = StyleValue::Value(bar.width);
            s.scrollbar_track_color = StyleValue::Value(bar.track_color);
            s.scrollbar_thumb_color = StyleValue::Value(bar.thumb_color);
            s.scrollbar_radius = StyleValue::Value(bar.radius);
            s.scrollbar_visibility = StyleValue::Value(bar.visibility);
        }
    }
}

/// An unset stroke follows the border declared alongside it.
fn resolve_stroke(
    value: &StyleValue<StrokeStyle>,
    target: &mut ComputedStyle,
    parent: &ComputedStyle,
    initial: &ComputedStyle,
    theme: &Theme,
) {
    let paint = &mut target.paint;
    if matches!(value, StyleValue::Unset | StyleValue::Inherit) {
        if paint.border_width > 0.0 && paint.border_color.is_visible() {
            paint.stroke = Some(ComputedStrokeStyle {
                color: paint.border_color,
                width: paint.border_width,
                line_style: StrokeLineStyle::Solid,
            });
        }
        return;
    }
    resolve_field(
        value,
        <border::props::stroke as StyleProperty>::INHERITED,
        &mut paint.stroke,
        &parent.paint.stroke,
        &initial.paint.stroke,
        theme,
    );
}

#[cfg(test)]
mod tests {
    //! Each capability resolves its declarations into the computed style:
    //! values, tokens through the theme, `Initial`, `Inherit` and `Unset`.

    use super::*;
    use crate::capability::StyleCapability;
    use crate::property::{Access, CapScope, Capability, Has, Slot};
    use xui_interface::WidgetUpdateFlags;
    use xui_interface::core::Color;
    use xui_interface::styles::units::{ColorToken, FilterQuality, SpacingToken};
    use xui_interface::{ComputedColorStyle, ComputedStrokeStyle};

    fn parent(theme: &Theme) -> ComputedStyle {
        let mut parent = ComputedStyle::initial(theme);
        parent.text.color = Color::rgb(0.2, 0.4, 0.6);
        parent.text.font_size = 22.0;
        parent.text.font_weight = FontWeight::SemiBold;
        parent.layout.width = Sizing::Fill;
        parent.layout.height = Sizing::Fill;
        parent.paint.background = Color::WHITE.into();
        parent
    }

    fn new<C: StyleCapability>(
        parent: &ComputedStyle,
        storage: &C::Storage,
        theme: &Theme,
    ) -> ComputedStyle {
        let initial = ComputedStyle::initial(theme);
        let mut computed = parent.inherited_from(theme);
        C::apply(storage, &mut computed, parent, &initial, theme);
        computed
    }

    #[test]
    fn layout_groups_resolve_their_declarations() {
        let theme = Theme::default();
        let parent = parent(&theme);
        let initial = ComputedStyle::initial(&theme);
        let mut computed = parent.inherited_from(&theme);
        BoxSize::apply(
            &box_size::Storage {
                width: StyleValue::Value(Sizing::from(120.0)),
                height: StyleValue::Inherit,
                min_width: StyleValue::Value(Sizing::from(10.0)),
                max_height: StyleValue::Initial,
                ..Default::default()
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );
        Margin::apply(
            &margin::Storage {
                margin: StyleValue::Value(EdgeInsets::all(4.0)),
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );
        Padding::apply(
            &padding::Storage {
                padding: StyleValue::Value(EdgeInsets::symmetric(8.0, 2.0)),
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );
        FlexLayout::apply(
            &flex_layout::Storage {
                gap: StyleValue::Value(LengthValue::Spacing(SpacingToken::Md)),
                align: StyleValue::Value(AlignStyle::Center),
                justify: StyleValue::Value(JustifyStyle::SpaceBetween),
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );
        Position::apply(
            &position::Storage {
                position: StyleValue::Value(PositionStyle::Absolute),
                inset: StyleValue::Value(EdgeInsets::all(1.0)),
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );
        Display::apply(
            &display::Storage {
                display: StyleValue::Value(DisplayStyle::None),
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );

        let layout = &computed.layout;
        assert_eq!(layout.width, Sizing::from(120.0));
        // `Inherit` on a non-inherited property keeps what the node had.
        assert_eq!(layout.height, Sizing::Hug);
        assert_eq!(layout.min_width, Some(Sizing::from(10.0)));
        assert_eq!(layout.max_height, initial.layout.max_height);
        assert_eq!(layout.margin, EdgeInsets::all(4.0));
        assert_eq!(layout.padding, EdgeInsets::symmetric(8.0, 2.0));
        assert_eq!(layout.gap, theme.spacing(SpacingToken::Md));
        assert_eq!(layout.align, AlignStyle::Center);
        assert_eq!(layout.justify, JustifyStyle::SpaceBetween);
        assert_eq!(layout.position, PositionStyle::Absolute);
        assert_eq!(layout.inset, Some(EdgeInsets::all(1.0)));
        assert_eq!(layout.display, DisplayStyle::None);
        // Nothing it did not declare moved.
        assert_eq!(computed.text, parent.inherited_from(&theme).text);
        assert_eq!(computed.paint, initial.paint);
    }

    #[test]
    fn typography_resolves_tokens_and_keywords() {
        let theme = Theme::default();
        let parent = parent(&theme);
        let storage = typography::Storage {
            color: StyleValue::Value(ColorValue::Token(ColorToken::Primary)),
            font_family: StyleValue::Unset,
            font_size: StyleValue::Initial,
            font_weight: StyleValue::Inherit,
            font_style: StyleValue::Value(FontStyle::Italic),
            line_height: StyleValue::Value(LineHeight::Px(30.0)),
            letter_spacing: StyleValue::Value(LengthValue::Px(0.5)),
            decoration: StyleValue::Unset,
        };

        let computed = new::<Typography>(&parent, &storage, &theme);
        let text = &computed.text;
        assert_eq!(text.color, theme.color(ColorToken::Primary));
        assert_eq!(text.font_family, parent.text.font_family);
        assert_eq!(text.font_size, ComputedStyle::initial(&theme).text.font_size);
        assert_eq!(text.font_weight, FontWeight::SemiBold);
        assert_eq!(text.font_style, FontStyle::Italic);
        assert_eq!(text.line_height, LineHeight::Px(30.0));
        assert_eq!(text.letter_spacing, 0.5);
        assert_eq!(text.decoration, parent.text.decoration);
    }

    #[test]
    fn unset_inherited_properties_follow_the_parent() {
        let theme = Theme::default();
        let parent = parent(&theme);
        let storage = typography::Storage {
            color: StyleValue::Value(ColorValue::Color(Color::BLACK)),
            ..Default::default()
        };
        let current = ComputedStyle::initial(&theme);

        // The parent's font size differs, and the size is left unset.
        assert!(Typography::inherits_differently(&storage, &current, &parent));
        let mut synced = current.clone();
        Typography::inherit_unset(&storage, &mut synced, &parent);
        assert_eq!(synced.text.font_size, parent.text.font_size);
        assert_eq!(synced.text.font_weight, parent.text.font_weight);
        // The colour is the node's own, so the parent's does not reach it.
        assert_eq!(synced.text.color, current.text.color);
        assert!(!Typography::inherits_differently(&storage, &synced, &parent));
        // A non-inherited capability never takes anything from the parent.
        assert!(!BoxSize::inherits_differently(
            &Default::default(),
            &current,
            &parent
        ));
    }

    #[test]
    fn paint_groups_resolve_and_the_stroke_follows_the_border() {
        let theme = Theme::default();
        let parent = parent(&theme);
        let border = |color: Color, width: f32| ComputedStrokeStyle {
            color: ComputedColorStyle::Solid(color),
            width,
            line_style: StrokeLineStyle::Solid,
        };
        let cases = [
            // The stroke follows the border.
            (
                StyleValue::Value(ColorStyle::from(Color::BLACK)),
                StyleValue::Value(LengthValue::Px(2.0)),
                StyleValue::Unset,
                Some(border(Color::BLACK, 2.0)),
            ),
            // An explicit stroke wins over the border.
            (
                StyleValue::Value(ColorStyle::from(Color::BLACK)),
                StyleValue::Value(LengthValue::Px(2.0)),
                StyleValue::Value(StrokeStyle {
                    color: Color::WHITE.into(),
                    width: LengthValue::Px(3.0),
                    line_style: StrokeLineStyle::Dashed,
                }),
                Some(ComputedStrokeStyle {
                    color: ComputedColorStyle::Solid(Color::WHITE),
                    width: 3.0,
                    line_style: StrokeLineStyle::Dashed,
                }),
            ),
            // An invisible explicit stroke computes to none.
            (
                StyleValue::Unset,
                StyleValue::Unset,
                StyleValue::Value(StrokeStyle {
                    color: Color::WHITE.into(),
                    width: LengthValue::Px(0.0),
                    line_style: StrokeLineStyle::Solid,
                }),
                None,
            ),
            // A width without a visible colour draws nothing.
            (
                StyleValue::Unset,
                StyleValue::Value(LengthValue::Px(2.0)),
                StyleValue::Initial,
                None,
            ),
        ];

        for (border_color, border_width, stroke, expected_stroke) in cases {
            let initial = ComputedStyle::initial(&theme);
            let mut computed = parent.inherited_from(&theme);
            Background::apply(
                &background::Storage {
                    background: StyleValue::Value(ColorToken::Muted.into()),
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );
            CornerRadius::apply(
                &corner_radius::Storage {
                    border_radius: StyleValue::Value(LengthValue::Px(6.0)),
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );
            Border::apply(
                &border::Storage {
                    border_color,
                    border_width,
                    stroke,
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );
            BoxShadow::apply(
                &box_shadow::Storage {
                    shadow: StyleValue::Value(ShadowStyle::new().color(Color::BLACK).blur(4.0)),
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );
            Clip::apply(
                &clip::Storage {
                    clip: StyleValue::Value(true),
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );
            Visibility::apply(
                &visibility::Storage {
                    visibility: StyleValue::Value(VisibilityStyle::Hidden),
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );

            let paint = &computed.paint;
            assert_eq!(
                paint.background,
                ComputedColorStyle::Solid(theme.color(ColorToken::Muted))
            );
            assert_eq!(paint.border_radius, 6.0);
            assert_eq!(paint.stroke, expected_stroke, "{border_color:?} {border_width:?}");
            let shadow = paint.shadow.as_ref().expect("a shadow");
            assert_eq!(shadow.blur, 4.0);
            assert!(paint.clip);
            assert_eq!(paint.visibility, VisibilityStyle::Hidden);
        }
    }

    #[test]
    fn transform_effects_scroll_and_cursor_resolve() {
        let theme = Theme::default();
        let parent = parent(&theme);
        let transform = transform::Storage {
            translate_x: StyleValue::Value(3.0),
            translate_y: StyleValue::Value(-2.0),
            scale_x: StyleValue::Value(1.5),
            scale_y: StyleValue::Initial,
            rotate: StyleValue::Value(0.25),
            origin_x: StyleValue::Value(0.0),
            origin_y: StyleValue::Unset,
        };
        let effects = effects::Storage {
            backdrop: StyleValue::Value(BackdropStyle::blur(LengthValue::Px(8.0))),
            effects: StyleValue::Value(Arc::from([Effect::Blur {
                sigma_x: LengthValue::Px(2.0),
                sigma_y: LengthValue::Spacing(SpacingToken::Sm),
                quality: FilterQuality::High,
            }])),
        };
        let scroll = scroll::Storage {
            scroll_direction: StyleValue::Value(ScrollDirectionStyle::Vertical),
            scrollbar_width: StyleValue::Value(LengthValue::Px(6.0)),
            scrollbar_track_color: StyleValue::Unset,
            scrollbar_thumb_color: StyleValue::Value(ColorToken::Border.into()),
            scrollbar_radius: StyleValue::Initial,
            scrollbar_visibility: StyleValue::Value(ScrollbarVisibilityStyle::Always),
        };
        let cursor = cursor::Storage {
            cursor: StyleValue::Value(CursorIcon::Pointer),
        };

        let initial = ComputedStyle::initial(&theme);
        let mut computed = parent.inherited_from(&theme);
        Transform::apply(&transform, &mut computed, &parent, &initial, &theme);
        Effects::apply(&effects, &mut computed, &parent, &initial, &theme);
        Scroll::apply(&scroll, &mut computed, &parent, &initial, &theme);
        Cursor::apply(&cursor, &mut computed, &parent, &initial, &theme);

        let t = &computed.transform;
        assert_eq!((t.translate.x, t.translate.y), (3.0, -2.0));
        assert_eq!((t.scale.x, t.scale.y), (1.5, initial.transform.scale.y));
        assert_eq!(t.rotate, 0.25);
        assert_eq!((t.origin.x, t.origin.y), (0.0, initial.transform.origin.y));
        assert!(computed.effect.backdrop.is_some());
        assert_eq!(computed.effect.effects.len(), 1);
        assert_eq!(computed.scroll.direction, ScrollDirectionStyle::Vertical);
        let bar = &computed.scroll.scrollbar;
        assert_eq!(bar.width, 6.0);
        assert_eq!(bar.track_color, initial.scroll.scrollbar.track_color);
        assert_eq!(
            bar.thumb_color,
            ComputedColorStyle::Solid(theme.color(ColorToken::Border))
        );
        assert_eq!(bar.radius, initial.scroll.scrollbar.radius);
        assert_eq!(bar.visibility, ScrollbarVisibilityStyle::Always);
        assert_eq!(computed.cursor, Some(CursorIcon::Pointer));
    }

    #[test]
    fn merge_layers_declarations() {
        let mut base = box_size::Storage {
            width: StyleValue::Value(Sizing::from(10.0)),
            height: StyleValue::Value(Sizing::from(20.0)),
            ..Default::default()
        };
        let over = box_size::Storage {
            width: StyleValue::Value(Sizing::Fill),
            height: StyleValue::Unset,
            min_width: StyleValue::Initial,
            ..Default::default()
        };
        BoxSize::merge(&mut base, &over);
        assert_eq!(base.width, StyleValue::Value(Sizing::Fill));
        assert_eq!(base.height, StyleValue::Value(Sizing::from(20.0)));
        assert_eq!(base.min_width, StyleValue::Initial);
    }

    /// A node that selected `BoxSize` and `Typography` only.
    #[derive(Default)]
    struct Node {
        size: box_size::Storage,
        text: typography::Storage,
    }

    impl Has<BoxSize> for Node {
        fn get(&self) -> &box_size::Storage {
            &self.size
        }
        fn get_mut(&mut self) -> &mut box_size::Storage {
            &mut self.size
        }
    }

    impl Has<Typography> for Node {
        fn get(&self) -> &typography::Storage {
            &self.text
        }
        fn get_mut(&mut self) -> &mut typography::Storage {
            &mut self.text
        }
    }

    impl<C: Capability> Access<CapScope<C>> for Node
    where
        Node: Has<C>,
    {
        fn access(&self) -> &C::Storage {
            Has::<C>::get(self)
        }
        fn access_mut(&mut self) -> &mut C::Storage {
            Has::<C>::get_mut(self)
        }
    }

    #[test]
    fn properties_are_typed_slots() {
        use box_size::props::width;
        use typography::props::{color, font_size};

        let mut node = Node::default();
        let width_slot = Slot::<width>::new();
        // Anything `Into` the declared unit is accepted, wrapped as a value.
        assert_eq!(
            width_slot.update(&mut node, 100.0),
            WidgetUpdateFlags::STYLE_TARGET
        );
        assert_eq!(
            width_slot.update(&mut node, 100.0),
            WidgetUpdateFlags::empty()
        );
        // So is the declaration itself.
        assert!(width_slot.apply(&mut node, StyleValue::Inherit));
        assert!(Slot::<color>::new().apply(&mut node, ColorToken::Primary));
        assert!(Slot::<font_size>::new().apply(&mut node, SpacingToken::Lg));

        assert_eq!(node.size.width, StyleValue::Inherit);
        assert_eq!(
            node.text.color,
            StyleValue::Value(ColorValue::Token(ColorToken::Primary))
        );
        assert_eq!(
            node.text.font_size,
            StyleValue::Value(LengthValue::Spacing(SpacingToken::Lg))
        );

        // A local named like a property still binds next to its import.
        let width = 3.0_f32;
        assert_eq!(width, 3.0);
    }
}
