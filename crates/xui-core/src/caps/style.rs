//! The style capabilities: the smallest groups of style properties a widget
//! selects.
//!
//! A group is what one kind of widget either has entirely or lacks entirely,
//! so a widget's style surface is the list of groups it names. Each group's
//! properties live in its `props` module, named like their attributes.

use std::sync::Arc;

use crate::capability::{StyleProperty, style_capability};
use xui_interface::core::{EdgeInsets, Sizing};
use xui_interface::style::{ComputedStyle, CursorIcon, Theme};
use xui_interface::styles::compute::resolve_field;
use xui_interface::styles::computed::ComputedStrokeStyle;
use xui_interface::styles::units::{
    AlignStyle, BackdropStyle, ColorStyle, ColorValue, DisplayStyle, Effect, JustifyStyle,
    LengthValue, PositionStyle, ScrollDirectionStyle, ScrollbarVisibilityStyle, ShadowStyle,
    StrokeLineStyle, StrokeStyle, StyleValue, VisibilityStyle,
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
}

style_capability! {
    /// The shadow cast by the border box.
    pub mod box_shadow: BoxShadow {
        shadow: ShadowStyle => paint.shadow,
    }
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
}

style_capability! {
    /// Backdrop filtering and layer effects.
    pub mod effects: Effects {
        backdrop: BackdropStyle => effect.backdrop,
        effects: Arc<[Effect]> => effect.effects,
    }
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
}

style_capability! {
    /// The pointer shown over the node. Changing it dirties nothing.
    pub mod cursor: Cursor {
        cursor: CursorIcon => cursor,
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
    //! Every capability must resolve exactly like the matching fields of the
    //! monolithic `StylePatch` did.

    use super::*;
    use crate::capability::StyleCapability;
    use crate::property::{Access, CapScope, Capability, Has, Slot};
    use xui_interface::WidgetUpdateFlags;
    use xui_interface::core::Color;
    use xui_interface::style::{
        EffectStylePatch, LayoutStylePatch, PaintStylePatch, ScrollStylePatch, StylePatch,
        TextStylePatch, TransformStylePatch,
    };
    use xui_interface::styles::units::{ColorToken, FilterQuality, SpacingToken};

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

    fn old(parent: &ComputedStyle, patch: &StylePatch, theme: &Theme) -> ComputedStyle {
        ComputedStyle::compute(parent, patch, theme)
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
    fn layout_groups_match_layout_patch() {
        let theme = Theme::default();
        let parent = parent(&theme);
        let patch = StylePatch {
            layout: LayoutStylePatch {
                gap: StyleValue::Value(LengthValue::Spacing(SpacingToken::Md)),
                width: StyleValue::Value(Sizing::from(120.0)),
                height: StyleValue::Inherit,
                min_width: StyleValue::Value(Sizing::from(10.0)),
                max_height: StyleValue::Initial,
                margin: StyleValue::Value(EdgeInsets::all(4.0)),
                padding: StyleValue::Value(EdgeInsets::symmetric(8.0, 2.0)),
                align: StyleValue::Value(AlignStyle::Center),
                justify: StyleValue::Value(JustifyStyle::SpaceBetween),
                position: StyleValue::Value(PositionStyle::Absolute),
                inset: StyleValue::Value(EdgeInsets::all(1.0)),
                display: StyleValue::Value(DisplayStyle::None),
                ..Default::default()
            },
            ..Default::default()
        };
        let expected = old(&parent, &patch, &theme);

        let initial = ComputedStyle::initial(&theme);
        let mut computed = parent.inherited_from(&theme);
        let layout = &patch.layout;
        BoxSize::apply(
            &box_size::Storage {
                width: layout.width,
                height: layout.height,
                min_width: layout.min_width,
                max_width: layout.max_width,
                min_height: layout.min_height,
                max_height: layout.max_height,
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );
        Margin::apply(
            &margin::Storage {
                margin: layout.margin,
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );
        Padding::apply(
            &padding::Storage {
                padding: layout.padding,
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );
        FlexLayout::apply(
            &flex_layout::Storage {
                gap: layout.gap,
                align: layout.align,
                justify: layout.justify,
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );
        Position::apply(
            &position::Storage {
                position: layout.position,
                inset: layout.inset,
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );
        Display::apply(
            &display::Storage {
                display: layout.display,
            },
            &mut computed,
            &parent,
            &initial,
            &theme,
        );

        assert_eq!(computed, expected);
        // `Inherit` on a non-inherited property keeps what the node had.
        assert_eq!(computed.layout.height, Sizing::Hug);
    }

    #[test]
    fn typography_matches_text_patch() {
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
        let patch = StylePatch {
            text: TextStylePatch {
                color: storage.color,
                font_family: storage.font_family.clone(),
                font_size: storage.font_size,
                font_weight: storage.font_weight,
                font_style: storage.font_style,
                line_height: storage.line_height,
                letter_spacing: storage.letter_spacing,
                decoration: storage.decoration,
            },
            ..Default::default()
        };

        let computed = new::<Typography>(&parent, &storage, &theme);
        assert_eq!(computed, old(&parent, &patch, &theme));
        assert_eq!(computed.text.font_weight, FontWeight::SemiBold);
        assert_eq!(
            computed.text.font_size,
            ComputedStyle::initial(&theme).text.font_size
        );
    }

    #[test]
    fn inheritance_helpers_match_style_patch() {
        let theme = Theme::default();
        let parent = parent(&theme);
        let storage = typography::Storage {
            color: StyleValue::Value(ColorValue::Color(Color::BLACK)),
            ..Default::default()
        };
        let patch = StylePatch {
            text: TextStylePatch {
                color: storage.color,
                ..Default::default()
            },
            ..Default::default()
        };
        let current = ComputedStyle::initial(&theme);

        assert_eq!(
            Typography::inherits_differently(&storage, &current, &parent),
            current.inherits_differently_from(&parent, &patch),
        );
        let mut by_cap = current.clone();
        Typography::inherit_unset(&storage, &mut by_cap, &parent);
        let mut by_patch = current.clone();
        by_patch.inherit_unset_from(&parent, &patch);
        assert_eq!(by_cap, by_patch);
        // A non-inherited capability never takes anything from the parent.
        assert!(!BoxSize::inherits_differently(
            &Default::default(),
            &current,
            &parent
        ));
    }

    #[test]
    fn paint_groups_match_paint_patch() {
        let theme = Theme::default();
        let parent = parent(&theme);
        let cases = [
            // The stroke follows the border.
            (
                StyleValue::Value(ColorStyle::from(Color::BLACK)),
                StyleValue::Value(LengthValue::Px(2.0)),
                StyleValue::Unset,
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
            ),
            (
                StyleValue::Unset,
                StyleValue::Value(LengthValue::Px(2.0)),
                StyleValue::Initial,
            ),
        ];

        for (border_color, border_width, stroke) in cases {
            let patch = StylePatch {
                paint: PaintStylePatch {
                    background: StyleValue::Value(ColorToken::Muted.into()),
                    border_color,
                    border_width,
                    border_radius: StyleValue::Value(LengthValue::Px(6.0)),
                    stroke,
                    shadow: StyleValue::Value(ShadowStyle::new().color(Color::BLACK).blur(4.0)),
                    clip: StyleValue::Value(true),
                    visibility: StyleValue::Value(VisibilityStyle::Hidden),
                },
                ..Default::default()
            };
            let paint = &patch.paint;

            let initial = ComputedStyle::initial(&theme);
            let mut computed = parent.inherited_from(&theme);
            Background::apply(
                &background::Storage {
                    background: paint.background,
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );
            CornerRadius::apply(
                &corner_radius::Storage {
                    border_radius: paint.border_radius,
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );
            Border::apply(
                &border::Storage {
                    border_color: paint.border_color,
                    border_width: paint.border_width,
                    stroke: paint.stroke,
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );
            BoxShadow::apply(
                &box_shadow::Storage {
                    shadow: paint.shadow,
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );
            Clip::apply(
                &clip::Storage { clip: paint.clip },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );
            Visibility::apply(
                &visibility::Storage {
                    visibility: paint.visibility,
                },
                &mut computed,
                &parent,
                &initial,
                &theme,
            );

            assert_eq!(computed, old(&parent, &patch, &theme), "{paint:?}");
        }
    }

    #[test]
    fn transform_effects_scroll_and_cursor_match() {
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

        let mut patch = StylePatch {
            transform: TransformStylePatch {
                translate_x: transform.translate_x,
                translate_y: transform.translate_y,
                scale_x: transform.scale_x,
                scale_y: transform.scale_y,
                rotate: transform.rotate,
                origin_x: transform.origin_x,
                origin_y: transform.origin_y,
            },
            effect: EffectStylePatch {
                backdrop: effects.backdrop.clone(),
                effects: effects.effects.clone(),
            },
            scroll: ScrollStylePatch {
                direction: scroll.scroll_direction,
                ..Default::default()
            },
            cursor: cursor.cursor,
            ..Default::default()
        };
        patch.scroll.scrollbar.width = scroll.scrollbar_width;
        patch.scroll.scrollbar.track_color = scroll.scrollbar_track_color;
        patch.scroll.scrollbar.thumb_color = scroll.scrollbar_thumb_color;
        patch.scroll.scrollbar.radius = scroll.scrollbar_radius;
        patch.scroll.scrollbar.visibility = scroll.scrollbar_visibility;

        let initial = ComputedStyle::initial(&theme);
        let mut computed = parent.inherited_from(&theme);
        Transform::apply(&transform, &mut computed, &parent, &initial, &theme);
        Effects::apply(&effects, &mut computed, &parent, &initial, &theme);
        Scroll::apply(&scroll, &mut computed, &parent, &initial, &theme);
        Cursor::apply(&cursor, &mut computed, &parent, &initial, &theme);

        assert_eq!(computed, old(&parent, &patch, &theme));
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
