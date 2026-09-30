//! The Component Gallery: every `xui-components` control on the shadcn/ui-style
//! theme, one page per component, behind a sidebar.

use xui_components::image::image;
use xui_components::*;
use xui_core::core::Bounds;
use xui_core::prelude::*;
use xui_macros::{component, xui};

/// A gallery page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Button,
    Input,
    Select,
    Tabs,
    Image,
    VirtualList,
    Canvas,
}

impl Page {
    pub const ALL: [Page; 7] = [
        Page::Button,
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
            Page::Input => lucide_rs::icons::text_cursor_input(),
            Page::Select => lucide_rs::icons::chevrons_up_down(),
            Page::Tabs => lucide_rs::icons::layers(),
            Page::Image => lucide_rs::icons::image(),
            Page::VirtualList => lucide_rs::icons::list(),
            Page::Canvas => lucide_rs::icons::shapes(),
        }
    }
}

thread_local! {
    /// The page the gallery opens on; tests point it at each page in turn.
    pub static INITIAL_PAGE: std::cell::Cell<Page> = const { std::cell::Cell::new(Page::Button) };
}

// ---- building blocks -------------------------------------------------------

fn label_text(value: impl Into<String>, style: Style) -> ElementDesc {
    TextWidget::new(value.into())
        .style(style)
        .into_element_desc()
}

fn muted(value: impl Into<String>) -> ElementDesc {
    label_text(
        value,
        Style::new()
            .color(ColorToken::MutedForeground)
            .font_size(FontSizeToken::Md),
    )
}

fn lucide(data: IconData, size: f32) -> ElementDesc {
    icon().from_icon_data(data).size(size).into_element_desc()
}

fn hstack(gap: f32, children: Vec<ElementDesc>) -> ElementDesc {
    ContainerWidget::new()
        .style(Style::new().gap(gap).align(AlignStyle::Center))
        .flex_direction(FlexDirectionStyle::Row)
        .into_element_desc(children)
}

/// A centered row that wraps onto new lines when the preview is too narrow,
/// like shadcn's `flex flex-wrap items-center justify-center gap-2`.
fn wrap_row(gap: f32, children: Vec<ElementDesc>) -> ElementDesc {
    ContainerWidget::new()
        .style(
            Style::new()
                .gap(gap)
                .align(AlignStyle::Center)
                .justify(JustifyStyle::Center),
        )
        .flex_direction(FlexDirectionStyle::Row)
        .flex_wrap(true)
        .into_element_desc(children)
}

fn vstack(gap: f32, children: Vec<ElementDesc>) -> ElementDesc {
    ContainerWidget::new()
        .style(Style::new().gap(gap))
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(children)
}

/// A page body: examples stacked at full content width.
fn page(children: Vec<ElementDesc>) -> ElementDesc {
    ContainerWidget::new()
        .style(Style::new().gap(40.0).width(Sizing::fill()))
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(children)
}

/// A titled example: a heading over a bordered preview area, like the demo
/// blocks on ui.shadcn.com.
fn example(title: &str, preview: ElementDesc) -> ElementDesc {
    ContainerWidget::new()
        .style(Style::new().gap(12.0).width(Sizing::fill()))
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(vec![
            label_text(
                title,
                Style::new()
                    .font_size(FontSizeToken::Lg)
                    .font_weight(FontWeight::SemiBold),
            ),
            ContainerWidget::new()
                .style(
                    Style::new()
                        .width(Sizing::fill())
                        .min_height(140.0)
                        .padding(EdgeInsets::all(32.0))
                        .align(AlignStyle::Center)
                        .justify(JustifyStyle::Center)
                        .background(ColorToken::Background)
                        .border_color(ColorToken::Border)
                        .border_width(1.0)
                        .border_radius(RadiusToken::Lg),
                )
                .flex_direction(FlexDirectionStyle::Column)
                .into_element_desc(vec![preview]),
        ])
}

fn field(label: &str, control: ElementDesc, hint: Option<&str>) -> ElementDesc {
    let mut children = vec![
        label_text(
            label,
            Style::new()
                .font_size(FontSizeToken::Md)
                .font_weight(FontWeight::Medium),
        ),
        control,
    ];
    if let Some(hint) = hint {
        children.push(label_text(
            hint,
            Style::new()
                .color(ColorToken::MutedForeground)
                .font_size(FontSizeToken::Sm),
        ));
    }
    ContainerWidget::new()
        .style(Style::new().gap(8.0).width(320.0))
        .flex_direction(FlexDirectionStyle::Column)
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
                loading_indicator={Some(lucide(lucide_rs::icons::loader_circle(), 16.0))}
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
fn input_page() {
    let email = cx.use_ref(TextController::new);
    let prefilled = cx.use_ref(|| TextController::with_text("Hello, xui 你好"));
    page(vec![
        example(
            "Default",
            ContainerWidget::new()
                .style(Style::new().width(320.0))
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
                Style::new()
                    .font_size(FontSizeToken::Lg)
                    .font_weight(FontWeight::SemiBold),
            ),
            muted(description),
        ],
    )];
    children.extend(fields);
    children.push(hstack(
        0.0,
        vec![xui! { <button text={action.to_string()} variant={ButtonVariant::Primary} /> }],
    ));
    ContainerWidget::new()
        .style(
            Style::new()
                .width(Sizing::fill())
                .gap(20.0)
                .padding(EdgeInsets::all(24.0))
                .background(ColorToken::Card)
                .color(ColorToken::CardForeground)
                .border_color(ColorToken::Border)
                .border_width(1.0)
                .border_radius(RadiusToken::Xl)
                .shadow(
                    ShadowStyle::new()
                        .color(Color::rgba(0.0, 0.0, 0.0, 0.05))
                        .offset(Point::new(0.0, 1.0))
                        .blur(2.0),
                ),
        )
        .flex_direction(FlexDirectionStyle::Column)
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
    style.root = style.root.width(420.0);

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
                ContainerWidget::new()
                    .style(
                        Style::new()
                            .size(Size::fix(180.0, 120.0))
                            .background(ColorToken::Muted)
                            .border_radius(RadiusToken::Md)
                            .clip(true),
                    )
                    .into_element_desc(vec![xui! {
                        <image
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
        ContainerWidget::new()
            .style(
                Style::new()
                    .width(Sizing::fill())
                    .height(ROW_HEIGHT)
                    .padding(EdgeInsets::symmetric(16.0, 0.0))
                    .align(AlignStyle::Center)
                    .justify(JustifyStyle::SpaceBetween)
                    .border_color(ColorToken::Border)
                    .when(WidgetState::HOVERED, |style| {
                        style.background(ColorToken::Muted)
                    }),
            )
            .flex_direction(FlexDirectionStyle::Row)
            .into_element_desc(vec![
                label_text(
                    format!("Item {}", index + 1),
                    Style::new().font_size(FontSizeToken::Md),
                ),
                label_text(
                    format!("#{index:05}"),
                    Style::new()
                        .color(ColorToken::MutedForeground)
                        .font_size(FontSizeToken::Sm),
                ),
            ])
    });
    let list = ContainerWidget::new()
        .style(
            Style::new()
                .width(360.0)
                .border_color(ColorToken::Border)
                .border_width(1.0)
                .border_radius(RadiusToken::Md)
                .clip(true),
        )
        .into_element_desc(vec![xui! {
            <virtual_list
                item_count={ROW_COUNT}
                item_height={ROW_HEIGHT}
                viewport_height={320.0}
                render_item={render_item}
                style={Style::new().width(Sizing::fill()).height(320.0)}
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
            icon()
                .from_icon_data(filled_icon())
                .size(32.0)
                .style(Style::new().color(ColorToken::Primary))
                .into_element_desc(),
            icon()
                .from_icon_data(stroked_icon())
                .size(32.0)
                .style(Style::new().color(ColorToken::Foreground))
                .into_element_desc(),
            icon()
                .from_icon_data(svg_icon())
                .size(32.0)
                .style(Style::new().color(ColorToken::MutedForeground))
                .into_element_desc(),
            icon()
                .from_icon_data(lucide_rs::icons::palette())
                .size(32.0)
                .style(Style::new().color(ColorToken::Destructive))
                .into_element_desc(),
        ],
    );

    let glass = xui! {
        <z_stack style={Style::new().size(Size::fix(440.0, 220.0))}>
            <canvas controller={canvas_handle} width={440.0} height={220.0} />
            <container
                style={Style::new()
                    .size(Size::fix(310.0, 118.0))
                    .padding(EdgeInsets::all(22.0))
                    .background(Color::rgba(0.92, 0.96, 1.0, 0.16))
                    .border_color(Color::rgba(1.0, 1.0, 1.0, 0.48))
                    .border_width(1.0)
                    .border_radius(20.0)
                    .clip(true)}
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
    let mut style = Style::new()
        .width(Sizing::fill())
        .justify(JustifyStyle::Start)
        .padding(EdgeInsets::symmetric(8.0, 6.0))
        .min_height(32.0);
    if active {
        style = style
            .background(ColorToken::Accent)
            .color(ColorToken::AccentForeground);
    } else {
        style = style.color(ColorToken::MutedForeground);
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
        ContainerWidget::new()
            .style(
                Style::new()
                    .gap(8.0)
                    .padding(EdgeInsets::new(8.0, 8.0, 4.0, 20.0))
                    .align(AlignStyle::Center),
            )
            .flex_direction(FlexDirectionStyle::Row)
            .into_element_desc(vec![
                lucide(lucide_rs::icons::component(), 20.0),
                label_text(
                    "xui components",
                    Style::new()
                        .font_size(FontSizeToken::Lg)
                        .font_weight(FontWeight::SemiBold),
                ),
            ]),
        label_text(
            "Components",
            Style::new()
                .padding(EdgeInsets::new(8.0, 8.0, 0.0, 4.0))
                .color(ColorToken::MutedForeground)
                .font_size(FontSizeToken::Sm)
                .font_weight(FontWeight::Medium),
        ),
    ];
    for (page, on_select) in Page::ALL.into_iter().zip(callbacks) {
        items.push(nav_item(page, page == active, on_select));
    }
    ContainerWidget::new()
        .style(
            Style::new()
                .width(240.0)
                .height(Sizing::fill())
                .gap(2.0)
                .padding(EdgeInsets::all(12.0))
                .background(ColorToken::Card),
        )
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(items)
}

#[component]
pub fn gallery() {
    let active = cx.use_state(|| INITIAL_PAGE.with(|page| page.get()));
    let callbacks: Vec<Callback<()>> = Page::ALL
        .into_iter()
        .map(|page| cx.use_callback((), move |()| active.set(page)))
        .collect();
    let page = *active.get();

    let content: ElementDesc = match page {
        Page::Button => xui! { <button_page /> },
        Page::Input => xui! { <input_page /> },
        Page::Select => xui! { <select_page /> },
        Page::Tabs => xui! { <tabs_page /> },
        Page::Image => xui! { <image_page /> },
        Page::VirtualList => xui! { <virtual_list_page /> },
        Page::Canvas => xui! { <canvas_page /> },
    };

    let main = ContainerWidget::new()
        .style(
            Style::new()
                .size(Size::fill())
                .padding(EdgeInsets::new(48.0, 48.0, 40.0, 64.0))
                .scroll_vertical(),
        )
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(vec![
            ContainerWidget::new()
                .key(format!("page-{page:?}"))
                .style(
                    Style::new()
                        .width(Sizing::fill())
                        .max_width(760.0)
                        .gap(40.0),
                )
                .flex_direction(FlexDirectionStyle::Column)
                .into_element_desc(vec![
                    vstack(
                        8.0,
                        vec![
                            label_text(
                                page.title(),
                                Style::new().font_size(30.0).font_weight(FontWeight::Bold),
                            ),
                            label_text(
                                page.description(),
                                Style::new()
                                    .color(ColorToken::MutedForeground)
                                    .font_size(FontSizeToken::Lg),
                            ),
                        ],
                    ),
                    content,
                ]),
        ]);

    ContainerWidget::new()
        .style(
            Style::new()
                .size(Size::fill())
                .background(ColorToken::Background)
                .padding(EdgeInsets::zero().set_top(32.))
                .color(ColorToken::Foreground),
        )
        .flex_direction(FlexDirectionStyle::Row)
        .into_element_desc(vec![
            sidebar(page, callbacks),
            ContainerWidget::new()
                .style(
                    Style::new()
                        .width(1.0)
                        .height(Sizing::fill())
                        .background(ColorToken::Border),
                )
                .into_element_desc(Vec::new()),
            main,
        ])
}

#[cfg(test)]
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
                INITIAL_PAGE.with(|initial| initial.set(page));
                let mut app = App::new(gallery_component);
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
}
