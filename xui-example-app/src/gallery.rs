//! The Component Gallery: every `xui-components` control on the shadcn/ui-style
//! theme, one page per component, behind a sidebar.

use xui_components::*;
use xui_core::core::Bounds;
use xui_core::prelude::*;
use xui_macros::{component, xui};

/// A gallery page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Button,
    Spinner,
    Input,
    Select,
    Tabs,
    Image,
    VirtualList,
    Canvas,
}

impl Page {
    pub const ALL: [Page; 8] = [
        Page::Button,
        Page::Spinner,
        Page::Input,
        Page::Select,
        Page::Tabs,
        Page::Image,
        Page::VirtualList,
        Page::Canvas,
    ];

    fn title(self) -> &'static str {
        match self {
            Page::Button => "Button",
            Page::Spinner => "Spinner",
            Page::Input => "Input",
            Page::Select => "Select",
            Page::Tabs => "Tabs",
            Page::Image => "Image",
            Page::VirtualList => "Virtual List",
            Page::Canvas => "Icon & Canvas",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Page::Button => "Displays a button or a component that looks like a button.",
            Page::Spinner => "An indeterminate progress indicator that turns on its own.",
            Page::Input => "A single-line text field with IME support.",
            Page::Select => "A list of options for the user to pick from, shown in a portal.",
            Page::Tabs => "Layered sections of content, displayed one at a time.",
            Page::Image => "Loads images from assets, files, or URLs, with fit modes.",
            Page::VirtualList => "Renders only the rows near the viewport of a long list.",
            Page::Canvas => "Path and SVG icons, vector canvas scenes, and backdrop blur.",
        }
    }

    fn icon(self) -> IconData {
        match self {
            Page::Button => lucide_rs::icons::square_mouse_pointer(),
            Page::Spinner => lucide_rs::icons::loader_circle(),
            Page::Input => lucide_rs::icons::text_cursor_input(),
            Page::Select => lucide_rs::icons::chevrons_up_down(),
            Page::Tabs => lucide_rs::icons::layers(),
            Page::Image => lucide_rs::icons::image(),
            Page::VirtualList => lucide_rs::icons::list(),
            Page::Canvas => lucide_rs::icons::shapes(),
        }
    }
}

/// The page the gallery opens on, provided on the `App`; tests point it at
/// each page in turn. Without one it opens on [`Page::Button`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InitialPage(pub Page);

// ---- building blocks -------------------------------------------------------

fn label_text(value: impl Into<String>, style: StyleOf<Text>) -> ElementDesc {
    text()
        .with::<text::content>(value.into())
        .with::<text::style>(style)
        .into_element_desc(Vec::new())
}

fn muted(value: impl Into<String>) -> ElementDesc {
    label_text(
        value,
        StyleOf::new()
            .with::<container::color>(ColorToken::MutedForeground)
            .with::<container::font_size>(FontSizeToken::Md),
    )
}

fn lucide(data: IconData, size: f32) -> ElementDesc {
    icon()
        .with::<icon::data>(data)
        .with::<icon::size>(Size::fix(size, size))
        .into_element_desc(Vec::new())
}

fn hstack(gap: f32, children: Vec<ElementDesc>) -> ElementDesc {
    container()
        .with::<container::style>(
            StyleOf::new()
                .with::<container::gap>(gap)
                .with::<container::align>(AlignStyle::Center),
        )
        .with::<container::flex_direction>(FlexDirectionStyle::Row)
        .into_element_desc(children)
}

/// A centered row that wraps onto new lines when the preview is too narrow,
/// like shadcn's `flex flex-wrap items-center justify-center gap-2`.
fn wrap_row(gap: f32, children: Vec<ElementDesc>) -> ElementDesc {
    container()
        .with::<container::style>(
            StyleOf::new()
                .with::<container::gap>(gap)
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::Center),
        )
        .with::<container::flex_direction>(FlexDirectionStyle::Row)
        .with::<container::flex_wrap>(true)
        .into_element_desc(children)
}

fn vstack(gap: f32, children: Vec<ElementDesc>) -> ElementDesc {
    container()
        .with::<container::style>(StyleOf::new().with::<container::gap>(gap))
        .with::<container::flex_direction>(FlexDirectionStyle::Column)
        .into_element_desc(children)
}

/// A page body: examples stacked at full content width.
fn page(children: Vec<ElementDesc>) -> ElementDesc {
    container()
        .with::<container::style>(
            StyleOf::new()
                .with::<container::gap>(40.0)
                .with::<container::width>(Sizing::fill()),
        )
        .with::<container::flex_direction>(FlexDirectionStyle::Column)
        .into_element_desc(children)
}

/// A titled example: a heading over a bordered preview area, like the demo
/// blocks on ui.shadcn.com.
fn example(title: &str, preview: ElementDesc) -> ElementDesc {
    container()
        .with::<container::style>(
            StyleOf::new()
                .with::<container::gap>(12.0)
                .with::<container::width>(Sizing::fill()),
        )
        .with::<container::flex_direction>(FlexDirectionStyle::Column)
        .into_element_desc(vec![
            label_text(
                title,
                StyleOf::new()
                    .with::<container::font_size>(FontSizeToken::Lg)
                    .with::<container::font_weight>(FontWeight::SemiBold),
            ),
            container()
                .with::<container::style>(
                    StyleOf::new()
                        .with::<container::width>(Sizing::fill())
                        .with::<container::min_height>(140.0)
                        .with::<container::padding>(EdgeInsets::all(32.0))
                        .with::<container::align>(AlignStyle::Center)
                        .with::<container::justify>(JustifyStyle::Center)
                        .with::<container::background>(ColorToken::Background)
                        .with::<container::border_color>(ColorToken::Border)
                        .with::<container::border_width>(1.0)
                        .with::<container::border_radius>(RadiusToken::Lg),
                )
                .with::<container::flex_direction>(FlexDirectionStyle::Column)
                .into_element_desc(vec![preview]),
        ])
}

fn field(label: &str, control: ElementDesc, hint: Option<&str>) -> ElementDesc {
    let mut children = vec![
        label_text(
            label,
            StyleOf::new()
                .with::<container::font_size>(FontSizeToken::Md)
                .with::<container::font_weight>(FontWeight::Medium),
        ),
        control,
    ];
    if let Some(hint) = hint {
        children.push(label_text(
            hint,
            StyleOf::new()
                .with::<container::color>(ColorToken::MutedForeground)
                .with::<container::font_size>(FontSizeToken::Sm),
        ));
    }
    container()
        .with::<container::style>(
            StyleOf::new()
                .with::<container::gap>(8.0)
                .with::<container::width>(320.0),
        )
        .with::<container::flex_direction>(FlexDirectionStyle::Column)
        .into_element_desc(children)
}

// ---- pages -----------------------------------------------------------------

#[component]
fn button_page() {
    let clicks = cx.use_state(|| 0usize);
    let on_click = cx.use_callback(clicks, move |()| clicks.update(|count| *count += 1));
    let label = |value: &str| value.to_string();

    let variants = xui! {
        <row gap={8.0} flex_wrap={true} justify={JustifyStyle::Center}>
            <button text={label("Primary")} variant={ButtonVariant::Primary} />
            <button text={label("Secondary")} variant={ButtonVariant::Secondary} />
            <button text={label("Outline")} variant={ButtonVariant::Outline} />
            <button text={label("Ghost")} variant={ButtonVariant::Ghost} />
            <button text={label("Destructive")} variant={ButtonVariant::Danger} />
            <button text={label("Link")} variant={ButtonVariant::Link} />
        </row>
    };
    let sizes = xui! {
        <row gap={8.0} flex_wrap={true} justify={JustifyStyle::Center}>
            <button text={label("Small")} variant={ButtonVariant::Outline} size={ButtonSize::Small} />
            <button text={label("Default")} variant={ButtonVariant::Outline} />
            <button text={label("Large")} variant={ButtonVariant::Outline} size={ButtonSize::Large} />
            <button
                text={String::new()}
                variant={ButtonVariant::Outline}
                size={ButtonSize::Icon}
                leading={Some(lucide(lucide_rs::icons::plus(), 16.0))}
                accessibility_label={Some(label("Add"))}
            />
        </row>
    };
    let with_icon = xui! {
        <row gap={8.0} flex_wrap={true} justify={JustifyStyle::Center}>
            <button
                text={label("Login with Email")}
                variant={ButtonVariant::Primary}
                leading={Some(lucide(lucide_rs::icons::mail(), 16.0))}
            />
            <button
                text={label("Delete")}
                variant={ButtonVariant::Danger}
                leading={Some(lucide(lucide_rs::icons::trash_2(), 16.0))}
            />
            <button
                text={label("Continue")}
                variant={ButtonVariant::Secondary}
                trailing={Some(lucide(lucide_rs::icons::arrow_right(), 16.0))}
            />
        </row>
    };
    let states = xui! {
        <row gap={8.0} flex_wrap={true} justify={JustifyStyle::Center}>
            <button text={label("Disabled")} variant={ButtonVariant::Primary} disabled={true} />
            <button text={label("Disabled")} variant={ButtonVariant::Outline} disabled={true} />
            <button
                text={label("Please wait")}
                variant={ButtonVariant::Primary}
                loading={true}
                loading_indicator={Some(xui! { <spinner size={16.0} /> })}
            />
        </row>
    };
    let count = *clicks.get();
    let interactive = hstack(
        16.0,
        vec![
            xui! { <button text={label("Click me")} variant={ButtonVariant::Primary} on_click={Some(on_click)} /> },
            muted(format!(
                "Clicked {count} time{}",
                if count == 1 { "" } else { "s" }
            )),
        ],
    );

    page(vec![
        example("Variants", variants),
        example("Sizes", sizes),
        example("With icon", with_icon),
        example("Disabled and loading", states),
        example("Interactive", interactive),
    ])
}

#[component]
fn spinner_page() {
    let sizes = xui! {
        <row gap={24.0} align={AlignStyle::Center} justify={JustifyStyle::Center}>
            <spinner size={16.0} />
            <spinner size={24.0} />
            <spinner size={32.0} />
            <spinner size={48.0} />
        </row>
    };
    let colors = xui! {
        <row gap={24.0} align={AlignStyle::Center} justify={JustifyStyle::Center}>
            <spinner style={StyleOf::new().with::<container::color>(ColorToken::MutedForeground)} />
            <spinner style={StyleOf::new().with::<container::color>(ColorToken::Primary)} />
            <spinner style={StyleOf::new().with::<container::color>(ColorToken::Destructive)} />
        </row>
    };
    let in_context = hstack(
        8.0,
        vec![
            xui! { <spinner size={16.0} style={StyleOf::new().with::<container::color>(ColorToken::MutedForeground)} /> },
            muted("Syncing your changes"),
        ],
    );

    page(vec![
        example("Sizes", sizes),
        example("Color", colors),
        example("Inline", in_context),
    ])
}

#[component]
fn input_page() {
    let email = cx.use_ref(TextController::new);
    let prefilled = cx.use_ref(|| TextController::with_text("Hello, xui 你好"));
    page(vec![
        example(
            "Default",
            container()
                .with::<container::style>(StyleOf::new().with::<container::width>(320.0))
                .into_element_desc(vec![xui! { <input controller={email.get().clone()} /> }]),
        ),
        example(
            "With label",
            field(
                "Email",
                xui! { <input controller={prefilled.get().clone()} /> },
                Some("Click to focus; click the empty area around it to blur."),
            ),
        ),
    ])
}

fn fruits() -> Vec<DropDownItem> {
    vec![
        DropDownItem::new("apple", "Apple"),
        DropDownItem::new("banana", "Banana"),
        DropDownItem::new("blueberry", "Blueberry").disabled(true),
        DropDownItem::new("grapes", "Grapes"),
        DropDownItem::new("pineapple", "Pineapple"),
    ]
}

#[component]
fn select_page() {
    let selected = cx.use_state(|| 3usize);
    let on_change = cx.use_callback(selected, move |index| selected.set(index));
    let current = fruits()[*selected.get()].label.clone();
    let controlled = vstack(
        12.0,
        vec![
            xui! {
                <drop_down
                    items={fruits()}
                    selected={Some(*selected.get())}
                    on_change={Some(on_change)}
                    id_prefix={"gallery-select-controlled".to_string()}
                />
            },
            muted(format!("Selected: {current}")),
        ],
    );
    page(vec![
        example(
            "Default",
            xui! { <drop_down items={fruits()} id_prefix={"gallery-select".to_string()} /> },
        ),
        example("Controlled", controlled),
        example(
            "Disabled",
            xui! {
                <drop_down
                    items={fruits()}
                    disabled={true}
                    id_prefix={"gallery-select-disabled".to_string()}
                />
            },
        ),
    ])
}

/// A settings card like shadcn's tabs demo.
fn tab_card(title: &str, description: &str, fields: Vec<ElementDesc>, action: &str) -> ElementDesc {
    let mut children = vec![vstack(
        6.0,
        vec![
            label_text(
                title,
                StyleOf::new()
                    .with::<container::font_size>(FontSizeToken::Lg)
                    .with::<container::font_weight>(FontWeight::SemiBold),
            ),
            muted(description),
        ],
    )];
    children.extend(fields);
    children.push(hstack(
        0.0,
        vec![xui! { <button text={action.to_string()} variant={ButtonVariant::Primary} /> }],
    ));
    container()
        .with::<container::style>(
            StyleOf::new()
                .with::<container::width>(Sizing::fill())
                .with::<container::gap>(20.0)
                .with::<container::padding>(EdgeInsets::all(24.0))
                .with::<container::background>(ColorToken::Card)
                .with::<container::color>(ColorToken::CardForeground)
                .with::<container::border_color>(ColorToken::Border)
                .with::<container::border_width>(1.0)
                .with::<container::border_radius>(RadiusToken::Xl)
                .with::<container::shadow>(
                    ShadowStyle::new()
                        .color(Color::rgba(0.0, 0.0, 0.0, 0.05))
                        .offset(Point::new(0.0, 1.0))
                        .blur(2.0),
                ),
        )
        .with::<container::flex_direction>(FlexDirectionStyle::Column)
        .into_element_desc(children)
}

#[component]
fn tabs_page() {
    let selected = cx.use_state(|| 0usize);
    let on_change = cx.use_callback(selected, move |index| selected.set(index));
    let name = cx.use_ref(|| TextController::with_text("Pedro Duarte"));
    let username = cx.use_ref(|| TextController::with_text("@peduarte"));
    let current = cx.use_ref(TextController::new);
    let next = cx.use_ref(TextController::new);

    let items = vec![
        TabItem::new(
            "account",
            "Account",
            tab_card(
                "Account",
                "Make changes to your account here. Click save when you're done.",
                vec![
                    field(
                        "Name",
                        xui! { <input controller={name.get().clone()} /> },
                        None,
                    ),
                    field(
                        "Username",
                        xui! { <input controller={username.get().clone()} /> },
                        None,
                    ),
                ],
                "Save changes",
            ),
        ),
        TabItem::new(
            "password",
            "Password",
            tab_card(
                "Password",
                "Change your password here. After saving, you'll be logged out.",
                vec![
                    field(
                        "Current password",
                        xui! { <input controller={current.get().clone()} /> },
                        None,
                    ),
                    field(
                        "New password",
                        xui! { <input controller={next.get().clone()} /> },
                        None,
                    ),
                ],
                "Save password",
            ),
        ),
        TabItem::new("billing", "Billing", muted("Unavailable")).disabled(true),
    ];
    let mut style = TabsStyle::default();
    style.root = style.root.with::<container::width>(420.0);

    page(vec![
        example(
            "Default",
            xui! {
                <tabs
                    items={items}
                    selected={Some(*selected.get())}
                    on_change={Some(on_change)}
                    style={style}
                    id_prefix={"gallery-tabs".to_string()}
                />
            },
        ),
        muted("Focus a tab and use ←/→, Home, and End. Disabled tabs are skipped."),
    ])
}

#[component]
fn image_page() {
    let framed = |fit: ImageFit, caption: &str| {
        vstack(
            8.0,
            vec![
                container()
                    .with::<container::style>(
                        StyleOf::new()
                            .with::<container::size>(Size::fix(180.0, 120.0))
                            .with::<container::background>(ColorToken::Muted)
                            .with::<container::border_radius>(RadiusToken::Md)
                            .with::<container::clip>(true),
                    )
                    .into_element_desc(vec![xui! {
                        <xui_components::image::image
                            src={ImageSrc::AssetId(crate::xui_assets::refs::images::DEMO_PNG)}
                            width={180.0}
                            height={120.0}
                            fit={Some(fit)}
                        />
                    }]),
                muted(caption),
            ],
        )
    };
    page(vec![example(
        "Fit",
        wrap_row(
            16.0,
            vec![
                framed(ImageFit::Cover, "Cover"),
                framed(ImageFit::Contain, "Contain"),
                framed(ImageFit::Fill, "Fill"),
            ],
        ),
    )])
}

const ROW_COUNT: usize = 10_000;
const ROW_HEIGHT: f32 = 36.0;

#[component]
fn virtual_list_page() {
    let render_item: VirtualItemRenderer = cx.use_callback((), |index: usize| {
        container()
            .with::<container::style>(
                StyleOf::new()
                    .with::<container::width>(Sizing::fill())
                    .with::<container::height>(ROW_HEIGHT)
                    .with::<container::padding>(EdgeInsets::symmetric(16.0, 0.0))
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::justify>(JustifyStyle::SpaceBetween)
                    .with::<container::border_color>(ColorToken::Border)
                    .when_style(
                        WidgetStateMatcher::all(WidgetState::HOVERED),
                        StyleOf::<Container>::new()
                            .with::<container::background>(ColorToken::Muted),
                    ),
            )
            .with::<container::flex_direction>(FlexDirectionStyle::Row)
            .into_element_desc(vec![
                label_text(
                    format!("Item {}", index + 1),
                    StyleOf::new().with::<container::font_size>(FontSizeToken::Md),
                ),
                label_text(
                    format!("#{index:05}"),
                    StyleOf::new()
                        .with::<container::color>(ColorToken::MutedForeground)
                        .with::<container::font_size>(FontSizeToken::Sm),
                ),
            ])
    });
    let list = container()
        .with::<container::style>(
            StyleOf::new()
                .with::<container::width>(360.0)
                .with::<container::border_color>(ColorToken::Border)
                .with::<container::border_width>(1.0)
                .with::<container::border_radius>(RadiusToken::Md)
                .with::<container::clip>(true),
        )
        .into_element_desc(vec![xui! {
            <virtual_list
                item_count={ROW_COUNT}
                item_height={ROW_HEIGHT}
                viewport_height={320.0}
                render_item={render_item}
                style={StyleOf::new().with::<container::width>(Sizing::fill()).with::<container::height>(320.0)}
            />
        }]);
    page(vec![
        example("10,000 rows", list),
        muted("Only the rows inside the viewport, plus a small overscan, are mounted."),
    ])
}

fn filled_icon() -> IconData {
    static ICON: std::sync::OnceLock<IconData> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(12.0, 2.0))
            .line_to(Point::new(22.0, 20.0))
            .line_to(Point::new(2.0, 20.0))
            .close();
        IconData::from_fill(Rect::new(0.0, 0.0, 24.0, 24.0), path.build())
    })
    .clone()
}

fn stroked_icon() -> IconData {
    static ICON: std::sync::OnceLock<IconData> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(4.0, 12.0))
            .cubic_to(
                Point::new(4.0, 5.0),
                Point::new(20.0, 5.0),
                Point::new(20.0, 12.0),
            )
            .cubic_to(
                Point::new(20.0, 19.0),
                Point::new(4.0, 19.0),
                Point::new(4.0, 12.0),
            )
            .close();
        IconData::from_stroke(
            Rect::new(0.0, 0.0, 24.0, 24.0),
            path.build(),
            IconStroke::new(2.0)
                .cap(LineCap::Round)
                .join(LineJoin::Round),
        )
    })
    .clone()
}

fn svg_icon() -> IconData {
    static ICON: std::sync::OnceLock<IconData> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        IconData::from_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"
                     fill="none" stroke="currentColor" stroke-width="2"
                     stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="9"/>
                    <path d="M8 12l3 3 5-6"/>
                </svg>"#,
        )
        .expect("embedded SVG icon must be valid")
    })
    .clone()
}

fn demo_canvas_scene(highlighted: bool) -> VectorScene {
    let mut scene = VectorSceneBuilder::new();
    let area_color = if highlighted {
        Color::rgba(0.74, 0.32, 0.96, 0.3)
    } else {
        Color::rgba(0.18, 0.42, 0.88, 0.28)
    };
    let line_color = if highlighted {
        Color::rgb(0.88, 0.55, 1.0)
    } else {
        Color::rgb(0.35, 0.72, 1.0)
    };

    let mut background = PathBuilder::new();
    background
        .move_to(Point::new(0.0, 0.0))
        .line_to(Point::new(440.0, 0.0))
        .line_to(Point::new(440.0, 220.0))
        .line_to(Point::new(0.0, 220.0))
        .close();
    let background_color = if highlighted {
        Color::rgb(0.16, 0.055, 0.22)
    } else {
        Color::rgb(0.04, 0.06, 0.11)
    };
    scene.fill_path(
        background.build(),
        Affine::IDENTITY,
        PathFill::new(background_color),
    );

    let mut grid = PathBuilder::new();
    for x in [40.0, 120.0, 200.0, 280.0, 360.0, 420.0] {
        grid.move_to(Point::new(x, 24.0))
            .line_to(Point::new(x, 192.0));
    }
    for y in [32.0, 72.0, 112.0, 152.0, 192.0] {
        grid.move_to(Point::new(24.0, y))
            .line_to(Point::new(420.0, y));
    }
    scene.stroke_path(
        grid.build(),
        Affine::IDENTITY,
        PathStroke::new(Color::rgba(0.55, 0.67, 0.86, 0.18), 1.0),
    );

    let curve = |path: &mut PathBuilder| {
        path.cubic_to(
            Point::new(76.0, 154.0),
            Point::new(92.0, 110.0),
            Point::new(136.0, 118.0),
        )
        .cubic_to(
            Point::new(188.0, 128.0),
            Point::new(202.0, 66.0),
            Point::new(252.0, 78.0),
        )
        .cubic_to(
            Point::new(302.0, 90.0),
            Point::new(326.0, 42.0),
            Point::new(368.0, 54.0),
        )
        .cubic_to(
            Point::new(392.0, 60.0),
            Point::new(406.0, 38.0),
            Point::new(420.0, 32.0),
        );
    };

    let mut area = PathBuilder::new();
    area.move_to(Point::new(24.0, 192.0))
        .line_to(Point::new(24.0, 158.0));
    curve(&mut area);
    area.line_to(Point::new(420.0, 192.0)).close();
    scene.fill_path(area.build(), Affine::IDENTITY, PathFill::new(area_color));

    let mut caption = TextProps::new(if highlighted {
        "Stable CanvasTextId\nshapes only when text changes"
    } else {
        "Canvas text box\nshares the TextHost cache"
    });
    caption.style.color = Color::rgb(0.92, 0.96, 1.0);
    caption.style.font_size = 15.0;
    caption.style.font_weight = FontWeight::Medium;
    caption.paragraph.vertical_align = TextVerticalAlign::Middle;
    caption.text_box.max_lines = Some(2);
    caption.text_box.overflow = TextOverflow::Ellipsis;
    scene.text_box(
        CanvasTextId::new(1),
        Bounds::from_origin_size((38.0, 36.0), (190.0, 58.0)),
        caption,
    );

    let mut line = PathBuilder::new();
    line.move_to(Point::new(24.0, 158.0));
    curve(&mut line);
    scene.stroke_path(
        line.build(),
        Affine::IDENTITY,
        PathStroke::new(line_color, 4.0)
            .cap(LineCap::Round)
            .join(LineJoin::Round),
    );

    scene.build()
}

#[component]
fn canvas_page() {
    let highlighted = cx.use_state(|| false);
    let controller = cx.use_ref(|| CanvasController::with_scene(demo_canvas_scene(false)));
    let canvas_handle = controller.get().clone();
    let click_handle = canvas_handle.clone();

    let icons = wrap_row(
        24.0,
        vec![
            container()
                .with::<container::style>(
                    StyleOf::new().with::<container::color>(ColorToken::Primary),
                )
                .into_element_desc(vec![
                    icon()
                        .with::<icon::data>(filled_icon())
                        .with::<icon::size>(Size::fix(32.0, 32.0))
                        .into_element_desc(Vec::new()),
                ]),
            container()
                .with::<container::style>(
                    StyleOf::new().with::<container::color>(ColorToken::Foreground),
                )
                .into_element_desc(vec![
                    icon()
                        .with::<icon::data>(stroked_icon())
                        .with::<icon::size>(Size::fix(32.0, 32.0))
                        .into_element_desc(Vec::new()),
                ]),
            container()
                .with::<container::style>(
                    StyleOf::new().with::<container::color>(ColorToken::MutedForeground),
                )
                .into_element_desc(vec![
                    icon()
                        .with::<icon::data>(svg_icon())
                        .with::<icon::size>(Size::fix(32.0, 32.0))
                        .into_element_desc(Vec::new()),
                ]),
            container()
                .with::<container::style>(
                    StyleOf::new().with::<container::color>(ColorToken::Destructive),
                )
                .into_element_desc(vec![
                    icon()
                        .with::<icon::data>(lucide_rs::icons::palette())
                        .with::<icon::size>(Size::fix(32.0, 32.0))
                        .into_element_desc(Vec::new()),
                ]),
        ],
    );

    let glass = xui! {
        <z_stack style={StyleOf::new().with::<container::size>(Size::fix(440.0, 220.0))}>
            <canvas controller={canvas_handle} width={440.0} height={220.0} />
            <container
                style={StyleOf::new()
                    .with::<container::size>(Size::fix(310.0, 118.0))
                    .with::<container::padding>(EdgeInsets::all(22.0))
                    .with::<container::background>(Color::rgba(0.92, 0.96, 1.0, 0.16))
                    .with::<container::border_color>(Color::rgba(1.0, 1.0, 1.0, 0.48))
                    .with::<container::border_width>(1.0)
                    .with::<container::border_radius>(20.0)
                    .with::<container::clip>(true)}
                backdrop_blur={18.0}
                on_click={move |_, _| {
                    let next = !*highlighted.get();
                    click_handle.set_scene(demo_canvas_scene(next));
                    highlighted.set(next);
                    EventResult::Consumed
                }}
            >
                <column gap={7.0}>
                    <text color={Color::WHITE} font_size={20.0} font_weight={FontWeight::Medium}>
                        {"GPU Backdrop Blur"}
                    </text>
                    <text color={Color::rgba(0.92, 0.96, 1.0, 0.82)} font_size={13.0}>
                        {"Click the glass card to swap the canvas scene."}
                    </text>
                </column>
            </container>
        </z_stack>
    };

    page(vec![
        example("Path, stroke, SVG, and Lucide icons", icons),
        example("Canvas with backdrop blur", glass),
    ])
}

// ---- shell -----------------------------------------------------------------

fn nav_item(page: Page, active: bool, on_select: Callback<()>) -> ElementDesc {
    let mut style = StyleOf::new()
        .with::<container::width>(Sizing::fill())
        .with::<container::justify>(JustifyStyle::Start)
        .with::<container::padding>(EdgeInsets::symmetric(8.0, 6.0))
        .with::<container::min_height>(32.0);
    if active {
        style = style
            .with::<container::background>(ColorToken::Accent)
            .with::<container::color>(ColorToken::AccentForeground);
    } else {
        style = style.with::<container::color>(ColorToken::MutedForeground);
    }
    xui! {
        <button
            text={page.title().to_string()}
            variant={ButtonVariant::Ghost}
            size={ButtonSize::Small}
            leading={Some(lucide(page.icon(), 16.0))}
            on_click={Some(on_select)}
            style={style}
        />
    }
}

fn sidebar(active: Page, callbacks: Vec<Callback<()>>) -> ElementDesc {
    let mut items = vec![
        container()
            .with::<container::style>(
                StyleOf::new()
                    .with::<container::gap>(8.0)
                    .with::<container::padding>(EdgeInsets::new(8.0, 8.0, 4.0, 20.0))
                    .with::<container::align>(AlignStyle::Center),
            )
            .with::<container::flex_direction>(FlexDirectionStyle::Row)
            .into_element_desc(vec![
                lucide(lucide_rs::icons::component(), 20.0),
                label_text(
                    "xui components",
                    StyleOf::new()
                        .with::<container::font_size>(FontSizeToken::Lg)
                        .with::<container::font_weight>(FontWeight::SemiBold),
                ),
            ]),
        label_text(
            "Components",
            StyleOf::new()
                .with::<container::padding>(EdgeInsets::new(8.0, 8.0, 0.0, 4.0))
                .with::<container::color>(ColorToken::MutedForeground)
                .with::<container::font_size>(FontSizeToken::Sm)
                .with::<container::font_weight>(FontWeight::Medium),
        ),
    ];
    for (page, on_select) in Page::ALL.into_iter().zip(callbacks) {
        items.push(nav_item(page, page == active, on_select));
    }
    container()
        .with::<container::style>(
            StyleOf::new()
                .with::<container::width>(240.0)
                .with::<container::height>(Sizing::fill())
                .with::<container::gap>(2.0)
                .with::<container::padding>(EdgeInsets::all(12.0))
                .with::<container::background>(ColorToken::Card),
        )
        .with::<container::flex_direction>(FlexDirectionStyle::Column)
        .into_element_desc(items)
}

#[component]
pub fn gallery() {
    let initial = cx
        .use_context::<InitialPage>()
        .map_or(Page::Button, |page| page.0);
    let active = cx.use_state(|| initial);
    let callbacks: Vec<Callback<()>> = Page::ALL
        .into_iter()
        .map(|page| cx.use_callback((), move |()| active.set(page)))
        .collect();
    let page = *active.get();

    let content: ElementDesc = match page {
        Page::Button => xui! { <button_page /> },
        Page::Spinner => xui! { <spinner_page /> },
        Page::Input => xui! { <input_page /> },
        Page::Select => xui! { <select_page /> },
        Page::Tabs => xui! { <tabs_page /> },
        Page::Image => xui! { <image_page /> },
        Page::VirtualList => xui! { <virtual_list_page /> },
        Page::Canvas => xui! { <canvas_page /> },
    };

    let main = container()
        .with::<container::style>(
            StyleOf::new()
                .with::<container::size>(Size::fill())
                .with::<container::padding>(EdgeInsets::new(48.0, 48.0, 40.0, 64.0))
                .with::<container::scroll_direction>(ScrollDirectionStyle::Vertical),
        )
        .with::<container::flex_direction>(FlexDirectionStyle::Column)
        .into_element_desc(vec![
            container()
                .key(format!("page-{page:?}"))
                .with::<container::style>(
                    StyleOf::new()
                        .with::<container::width>(Sizing::fill())
                        .with::<container::max_width>(760.0)
                        .with::<container::gap>(40.0),
                )
                .with::<container::flex_direction>(FlexDirectionStyle::Column)
                .into_element_desc(vec![
                    vstack(
                        8.0,
                        vec![
                            label_text(
                                page.title(),
                                StyleOf::new()
                                    .with::<container::font_size>(30.0)
                                    .with::<container::font_weight>(FontWeight::Bold),
                            ),
                            label_text(
                                page.description(),
                                StyleOf::new()
                                    .with::<container::color>(ColorToken::MutedForeground)
                                    .with::<container::font_size>(FontSizeToken::Lg),
                            ),
                        ],
                    ),
                    content,
                ]),
        ]);

    container()
        .with::<container::style>(
            StyleOf::new()
                .with::<container::size>(Size::fill())
                .with::<container::background>(ColorToken::Background)
                .with::<container::padding>(EdgeInsets::zero().set_top(32.))
                .with::<container::color>(ColorToken::Foreground),
        )
        .with::<container::flex_direction>(FlexDirectionStyle::Row)
        .into_element_desc(vec![
            sidebar(page, callbacks),
            container()
                .with::<container::style>(
                    StyleOf::new()
                        .with::<container::width>(1.0)
                        .with::<container::height>(Sizing::fill())
                        .with::<container::background>(ColorToken::Border),
                )
                .into_element_desc(Vec::new()),
            main,
        ])
}

#[cfg(all(test, feature = "skia"))]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::BufWriter;
    use xui_core::text::TextHost;
    use xui_cosmic::CosmicEngine;
    use xui_skia::{SkiaBackend, SkiaBackendOptions};

    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 860;

    /// Renders every page headlessly in both palettes. With
    /// `XUI_GALLERY_SNAPSHOT_DIR` set, also writes `<page>-<theme>.png` there.
    #[test]
    fn every_page_renders_in_both_themes() {
        let snapshot_dir = std::env::var("XUI_GALLERY_SNAPSHOT_DIR").ok();
        // What `#[xui_macros::main]` does before `main`, so the image page
        // loads from the packed assets.
        xui_core::assets::install_asset_manager(
            crate::xui_assets::manager().expect("packed assets should mount"),
        );
        let mut runs: Vec<(String, Theme, Page, u32)> = Vec::new();
        for (theme_name, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
            for page in Page::ALL {
                runs.push((format!("{page:?}-{theme_name}"), theme.clone(), page, WIDTH));
            }
        }
        // Narrow enough that the button rows have to wrap.
        runs.push((
            "Button-light-narrow".to_string(),
            Theme::light(),
            Page::Button,
            820,
        ));
        {
            for (name, theme, page, width) in runs {
                let mut app = App::new(gallery_component);
                app.provide_context(InitialPage(page));
                app.set_theme(theme.clone());
                app.resize(Size::new(width as f32, HEIGHT as f32));
                let mut backend = SkiaBackend::<CosmicEngine>::headless(
                    1.0,
                    SkiaBackendOptions {
                        clear_color: theme.background,
                        ..SkiaBackendOptions::default()
                    },
                );
                let mut text = TextHost::new(CosmicEngine::new(1.0));
                // Images decode off the render thread, so give them time to land.
                for _ in 0..40 {
                    app.drain_async_messages();
                    if !app.is_dirty() {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        if !app.drain_async_messages() && !app.is_dirty() {
                            break;
                        }
                    }
                    app.render(&mut backend, &mut text)
                        .unwrap_or_else(|error| panic!("{page:?} should render: {error:?}"));
                }
                let pixels = backend
                    .read_pixels_rgba8()
                    .expect("pixels should be readable");
                assert_eq!(pixels.len(), (width * HEIGHT * 4) as usize);

                if let Some(dir) = &snapshot_dir {
                    let path = format!("{dir}/{name}.png");
                    let writer = BufWriter::new(File::create(path).expect("snapshot file"));
                    let mut encoder = png::Encoder::new(writer, width, HEIGHT);
                    encoder.set_color(png::ColorType::Rgba);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder
                        .write_header()
                        .expect("PNG header")
                        .write_image_data(&pixels)
                        .expect("PNG data");
                }
            }
        }
    }

    /// The spinner's turn reaches the renderer: two frames 200ms apart are
    /// different pictures, though nothing but the frame clock moved.
    #[test]
    fn the_spinner_page_moves_from_frame_to_frame() {
        let theme = Theme::light();
        let mut app = App::new(gallery_component);
        app.provide_context(InitialPage(Page::Spinner));
        app.set_theme(theme.clone());
        let backend = SkiaBackend::<CosmicEngine>::headless(
            1.0,
            SkiaBackendOptions {
                clear_color: theme.background,
                ..SkiaBackendOptions::default()
            },
        );
        let mut runtime = GuiRuntime::new(app, backend, CosmicEngine::new(1.0));
        runtime.handle_event(RuntimeEvent::Resize(Size::new(WIDTH as f32, HEIGHT as f32)));
        let t0 = std::time::Instant::now();
        let mut pixels_at = |millis: u64| {
            runtime
                .frame_at(t0 + std::time::Duration::from_millis(millis))
                .expect("frame should render");
            runtime
                .backend_mut()
                .read_pixels_rgba8()
                .expect("pixels should be readable")
        };
        pixels_at(0);
        let before = pixels_at(50);
        for millis in [100, 150, 200] {
            pixels_at(millis);
        }
        let after = pixels_at(250);
        let changed = before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(changed > 100, "only {changed} pixels changed");
    }
}

#[cfg(test)]
mod smoke_tests {
    use super::*;
    use xui_core::text::TextHost;
    use xui_cosmic::CosmicEngine;

    #[test]
    fn gallery_pages_build_scenes_in_both_themes() {
        xui_core::assets::install_asset_manager(crate::xui_assets::manager().unwrap());
        for theme in [Theme::light(), Theme::dark()] {
            for page in Page::ALL {
                let mut app = App::new(gallery_component);
                app.provide_context(InitialPage(page));
                app.set_theme(theme.clone());
                let mut backend = MockRenderBackend::default();
                let mut text = TextHost::new(CosmicEngine::new(1.0));
                for width in [1280.0, 820.0] {
                    app.resize(Size::new(width, 860.0));
                    app.render(&mut backend, &mut text).expect("gallery scene");
                    let tree = app.ui_runtime();
                    let mut pending = vec![tree.root()];
                    let mut count = 0;
                    while let Some(id) = pending.pop() {
                        let node = tree.node(id).unwrap();
                        assert!(node.layout.width().is_finite(), "{page:?}");
                        assert!(node.layout.height().is_finite(), "{page:?}");
                        pending.extend(tree.children(id));
                        count += 1;
                    }
                    assert!(count > 10, "{page:?} did not build its content");
                }
            }
        }
    }
}
