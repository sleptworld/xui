//! The Component Gallery: every `xui-components` control on the shadcn/ui-style
//! theme, one page per component, behind a sidebar.

use xui_components::*;
use xui_core::core::Bounds;
use xui_core::prelude::*;
use xui_components::image::ImageSrc;
use xui_macros::component;

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

fn heading_style() -> StyleOf<Text> {
    StyleOf::new()
        .with::<container::font_size>(FontSizeToken::Lg)
        .with::<container::font_weight>(FontWeight::SemiBold)
}

fn lucide(data: IconData, size: f32) -> Children {
    Children::component::<LucideIcon>(LucideIconProps { data, size })
}

/// An icon at a square size, as content for a button's `leading`/`trailing`.
#[component]
fn lucide_icon(data: &IconData, size: &f32) {
    view! { icon(data: data.clone(), size: Size::fix(*size, *size)) }
}

#[component]
fn muted(#[source] text: &String) {
    view! {
        xui_core::prelude::text(
            text.clone(),
            style: StyleOf::new()
                .with::<container::color>(ColorToken::MutedForeground)
                .with::<container::font_size>(FontSizeToken::Md),
        )
    }
}

#[component]
fn hstack(gap: &f32, children: &Children) {
    view! {
        container(
            style: StyleOf::new().with::<container::gap>(*gap).with::<container::align>(AlignStyle::Center),
            flex_direction: FlexDirectionStyle::Row,
        ) {
            slot(children)
        }
    }
}

/// A centered row that wraps onto new lines when the preview is too narrow,
/// like shadcn's `flex flex-wrap items-center justify-center gap-2`.
#[component]
fn wrap_row(gap: &f32, children: &Children) {
    view! {
        container(
            style: StyleOf::new()
                .with::<container::gap>(*gap)
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::Center),
            flex_direction: FlexDirectionStyle::Row,
            flex_wrap: true,
        ) {
            slot(children)
        }
    }
}

#[component]
fn vstack(gap: &f32, children: &Children) {
    view! {
        container(style: StyleOf::new().with::<container::gap>(*gap), flex_direction: FlexDirectionStyle::Column) {
            slot(children)
        }
    }
}

/// A page body: examples stacked at full content width.
#[component]
fn page_body(children: &Children) {
    view! {
        container(
            style: StyleOf::new().with::<container::gap>(40.0).with::<container::width>(Sizing::fill()),
            flex_direction: FlexDirectionStyle::Column,
        ) {
            slot(children)
        }
    }
}

/// A titled example: a heading over a bordered preview area, like the demo
/// blocks on ui.shadcn.com.
#[component]
fn example(title: &String, children: &Children) {
    view! {
        container(
            style: StyleOf::new().with::<container::gap>(12.0).with::<container::width>(Sizing::fill()),
            flex_direction: FlexDirectionStyle::Column,
        ) {
            text(title.clone(), style: heading_style())
            container(
                style: StyleOf::new()
                    .with::<container::width>(Sizing::fill())
                    .with::<container::min_height>(140.0)
                    .with::<container::padding>(EdgeInsets::all(32.0))
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::justify>(JustifyStyle::Center)
                    .with::<container::background>(ColorToken::Background)
                    .with::<container::border_color>(ColorToken::Border)
                    .with::<container::border_width>(1.0)
                    .with::<container::border_radius>(RadiusToken::Lg),
                flex_direction: FlexDirectionStyle::Column,
            ) {
                slot(children)
            }
        }
    }
}

/// A labelled control, with an optional hint under it.
#[component]
#[defaults(hint = None::<String>)]
fn field(label: &String, hint: &Option<String>, children: &Children) {
    view! {
        container(
            style: StyleOf::new().with::<container::gap>(8.0).with::<container::width>(320.0),
            flex_direction: FlexDirectionStyle::Column,
        ) {
            text(
                label.clone(),
                style: StyleOf::new()
                    .with::<container::font_size>(FontSizeToken::Md)
                    .with::<container::font_weight>(FontWeight::Medium),
            )
            slot(children)
            if let Some(hint) = hint.clone() {
                text(
                    hint.clone(),
                    style: StyleOf::new()
                        .with::<container::color>(ColorToken::MutedForeground)
                        .with::<container::font_size>(FontSizeToken::Sm),
                )
            }
        }
    }
}

// ---- pages -----------------------------------------------------------------

#[component]
fn button_page() {
    let clicks: usize = state!(0);
    view! {
        page_body {
            example(title: "Variants") {
                row(gap: 8.0, flex_wrap: true, justify: JustifyStyle::Center) {
                    button(text: "Primary", variant: ButtonVariant::Primary)
                    button(text: "Secondary", variant: ButtonVariant::Secondary)
                    button(text: "Outline", variant: ButtonVariant::Outline)
                    button(text: "Ghost", variant: ButtonVariant::Ghost)
                    button(text: "Destructive", variant: ButtonVariant::Danger)
                    button(text: "Link", variant: ButtonVariant::Link)
                }
            }
            example(title: "Sizes") {
                row(gap: 8.0, flex_wrap: true, justify: JustifyStyle::Center) {
                    button(text: "Small", variant: ButtonVariant::Outline, size: ButtonSize::Small)
                    button(text: "Default", variant: ButtonVariant::Outline)
                    button(text: "Large", variant: ButtonVariant::Outline, size: ButtonSize::Large)
                    button(
                        text: String::new(),
                        variant: ButtonVariant::Outline,
                        size: ButtonSize::Icon,
                        leading: lucide(lucide_rs::icons::plus(), 16.0),
                        accessibility_label: Some("Add".to_string()),
                    )
                }
            }
            example(title: "With icon") {
                row(gap: 8.0, flex_wrap: true, justify: JustifyStyle::Center) {
                    button(
                        text: "Login with Email",
                        variant: ButtonVariant::Primary,
                        leading: lucide(lucide_rs::icons::mail(), 16.0),
                    )
                    button(text: "Delete", variant: ButtonVariant::Danger, leading: lucide(lucide_rs::icons::trash_2(), 16.0))
                    button(
                        text: "Continue",
                        variant: ButtonVariant::Secondary,
                        trailing: lucide(lucide_rs::icons::arrow_right(), 16.0),
                    )
                }
            }
            example(title: "Disabled and loading") {
                row(gap: 8.0, flex_wrap: true, justify: JustifyStyle::Center) {
                    button(text: "Disabled", variant: ButtonVariant::Primary, disabled: true)
                    button(text: "Disabled", variant: ButtonVariant::Outline, disabled: true)
                    button(
                        text: "Please wait",
                        variant: ButtonVariant::Primary,
                        loading: true,
                        loading_indicator: view! { spinner(size: 16.0) },
                    )
                }
            }
            example(title: "Interactive") {
                hstack(gap: 16.0) {
                    button(text: "Click me", variant: ButtonVariant::Primary, on_click: |cx| clicks.update(cx, |count| *count += 1))
                    muted(text: format!("Clicked {clicks} time{}", if *clicks == 1 { "" } else { "s" }))
                }
            }
        }
    }
}

#[component]
fn spinner_page() {
    view! {
        page_body {
            example(title: "Sizes") {
                row(gap: 24.0, align: AlignStyle::Center, justify: JustifyStyle::Center) {
                    spinner(size: 16.0)
                    spinner(size: 24.0)
                    spinner(size: 32.0)
                    spinner(size: 48.0)
                }
            }
            example(title: "Color") {
                row(gap: 24.0, align: AlignStyle::Center, justify: JustifyStyle::Center) {
                    spinner(style: StyleOf::new().with::<container::color>(ColorToken::MutedForeground))
                    spinner(style: StyleOf::new().with::<container::color>(ColorToken::Primary))
                    spinner(style: StyleOf::new().with::<container::color>(ColorToken::Destructive))
                }
            }
            example(title: "Inline") {
                hstack(gap: 8.0) {
                    spinner(size: 16.0, style: StyleOf::new().with::<container::color>(ColorToken::MutedForeground))
                    muted(text: "Syncing your changes")
                }
            }
        }
    }
}

#[component]
fn input_page() {
    let email: TextController = TextController::new();
    let prefilled: TextController = TextController::with_text("Hello, xui 你好");
    view! {
        page_body {
            example(title: "Default") {
                container(style: StyleOf::new().with::<container::width>(320.0)) {
                    input(controller: Some(email.clone()))
                }
            }
            example(title: "With label") {
                field(label: "Email", hint: Some("Click to focus; click the empty area around it to blur.".to_string())) {
                    input(controller: Some(prefilled.clone()))
                }
            }
        }
    }
}

#[component]
fn select_page() {
    let selected: usize = state!(3);
    view! {
        page_body {
            example(title: "Default") {
                drop_down(items: fruits(), id_prefix: "gallery-select")
            }
            example(title: "Controlled") {
                vstack(gap: 12.0) {
                    drop_down(
                        items: fruits(),
                        selected: Some(*selected),
                        on_change: |cx, index| selected.set(cx, index),
                        id_prefix: "gallery-select-controlled",
                    )
                    muted(text: format!("Selected: {}", fruits()[*selected].label))
                }
            }
            example(title: "Disabled") {
                drop_down(items: fruits(), disabled: true, id_prefix: "gallery-select-disabled")
            }
        }
    }
}

/// A settings card like shadcn's tabs demo.
#[component]
fn tab_card(title: &String, description: &String, action: &String, children: &Children) {
    view! {
        container(
            style: StyleOf::new()
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
            flex_direction: FlexDirectionStyle::Column,
        ) {
            vstack(gap: 6.0) {
                text(title.clone(), style: heading_style())
                muted(text: description.clone())
            }
            slot(children)
            hstack(gap: 0.0) {
                button(text: action.clone(), variant: ButtonVariant::Primary)
            }
        }
    }
}

/// The tab panel with two labelled inputs.
#[component]
fn form_tab(
    title: &String,
    description: &String,
    action: &String,
    first_label: &String,
    first: &TextController,
    second_label: &String,
    second: &TextController,
) {
    view! {
        tab_card(title: title.clone(), description: description.clone(), action: action.clone()) {
            field(label: first_label.clone()) { input(controller: Some(first.clone())) }
            field(label: second_label.clone()) { input(controller: Some(second.clone())) }
        }
    }
}

#[component]
fn tabs_page() {
    let selected: usize = state!(0);
    let items: Vec<TabItem> = vec![
        TabItem::new(
            "account",
            "Account",
            Children::component::<FormTab>(FormTabProps {
                title: "Account".into(),
                description: "Make changes to your account here. Click save when you're done.".into(),
                action: "Save changes".into(),
                first_label: "Name".into(),
                first: TextController::with_text("Pedro Duarte"),
                second_label: "Username".into(),
                second: TextController::with_text("@peduarte"),
            }),
        ),
        TabItem::new(
            "password",
            "Password",
            Children::component::<FormTab>(FormTabProps {
                title: "Password".into(),
                description: "Change your password here. After saving, you'll be logged out.".into(),
                action: "Save password".into(),
                first_label: "Current password".into(),
                first: TextController::new(),
                second_label: "New password".into(),
                second: TextController::new(),
            }),
        ),
        TabItem::new("billing", "Billing", "Unavailable").disabled(true),
    ];
    let mut style: TabsStyle = TabsStyle::default();
    style.root = style.root.with::<container::width>(420.0);
    view! {
        page_body {
            example(title: "Default") {
                tabs(
                    items: items.clone(),
                    selected: Some(*selected),
                    on_change: |cx, index| selected.set(cx, index),
                    style: style.clone(),
                    id_prefix: "gallery-tabs",
                )
            }
            muted(text: "Focus a tab and use ←/→, Home, and End. Disabled tabs are skipped.")
        }
    }
}

/// One fit mode of the demo image, with its caption.
#[component]
fn framed(fit: &ImageFit, caption: &String) {
    view! {
        vstack(gap: 8.0) {
            container(
                style: StyleOf::new()
                    .with::<container::size>(Size::fix(180.0, 120.0))
                    .with::<container::background>(ColorToken::Muted)
                    .with::<container::border_radius>(RadiusToken::Md)
                    .with::<container::clip>(true),
            ) {
                xui_components::image::image(
                    src: ImageSrc::AssetId(crate::xui_assets::refs::images::DEMO_PNG),
                    width: 180.0f32,
                    height: 120.0f32,
                    fit: Some(*fit),
                )
            }
            muted(text: caption.clone())
        }
    }
}

#[component]
fn image_page() {
    view! {
        page_body {
            example(title: "Fit") {
                wrap_row(gap: 16.0) {
                    framed(fit: ImageFit::Cover, caption: "Cover")
                    framed(fit: ImageFit::Contain, caption: "Contain")
                    framed(fit: ImageFit::Fill, caption: "Fill")
                }
            }
        }
    }
}

const ROW_COUNT: usize = 10_000;
const ROW_HEIGHT: f32 = 36.0;

/// A row of the long list.
#[component]
fn list_item(index: &usize) {
    view! {
        container(
            style: StyleOf::new()
                .with::<container::width>(Sizing::fill())
                .with::<container::height>(ROW_HEIGHT)
                .with::<container::padding>(EdgeInsets::symmetric(16.0, 0.0))
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::SpaceBetween)
                .with::<container::border_color>(ColorToken::Border)
                .when_style(
                    WidgetStateMatcher::all(WidgetState::HOVERED),
                    StyleOf::<Container>::new().with::<container::background>(ColorToken::Muted),
                ),
            flex_direction: FlexDirectionStyle::Row,
        ) {
            text(format!("Item {}", *index + 1), style: StyleOf::new().with::<container::font_size>(FontSizeToken::Md))
            text(
                format!("#{index:05}"),
                style: StyleOf::new()
                    .with::<container::color>(ColorToken::MutedForeground)
                    .with::<container::font_size>(FontSizeToken::Sm),
            )
        }
    }
}

#[component]
fn virtual_list_page() {
    let render_item: VirtualItemRenderer =
        VirtualItemRenderer::new(|index: usize| Children::component::<ListItem>(ListItemProps { index }));
    view! {
        page_body {
            example(title: "10,000 rows") {
                container(
                    style: StyleOf::new()
                        .with::<container::width>(360.0)
                        .with::<container::border_color>(ColorToken::Border)
                        .with::<container::border_width>(1.0)
                        .with::<container::border_radius>(RadiusToken::Md)
                        .with::<container::clip>(true),
                ) {
                    virtual_list(
                        item_count: ROW_COUNT,
                        item_height: ROW_HEIGHT,
                        viewport_height: 320.0,
                        render_item: render_item.clone(),
                        style: StyleOf::new().with::<container::width>(Sizing::fill()).with::<container::height>(320.0),
                    )
                }
            }
            muted(text: "Only the rows inside the viewport, plus a small overscan, are mounted.")
        }
    }
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

/// An icon in a color.
#[component]
fn tinted_icon(data: &IconData, color: &ColorToken) {
    view! {
        container(style: StyleOf::new().with::<container::color>(*color)) {
            icon(data: data.clone(), size: Size::fix(32.0, 32.0))
        }
    }
}

#[component]
fn canvas_page() {
    let highlighted: bool = state!(false);
    let controller: CanvasController = CanvasController::with_scene(demo_canvas_scene(false));
    view! {
        page_body {
            example(title: "Path, stroke, SVG, and Lucide icons") {
                wrap_row(gap: 24.0) {
                    tinted_icon(data: filled_icon(), color: ColorToken::Primary)
                    tinted_icon(data: stroked_icon(), color: ColorToken::Foreground)
                    tinted_icon(data: svg_icon(), color: ColorToken::MutedForeground)
                    tinted_icon(data: lucide_rs::icons::palette(), color: ColorToken::Destructive)
                }
            }
            example(title: "Canvas with backdrop blur") {
                z_stack(style: StyleOf::new().with::<container::size>(Size::fix(440.0, 220.0))) {
                    canvas(controller: controller.clone(), width: 440.0, height: 220.0)
                    container(
                        style: StyleOf::new()
                            .with::<container::size>(Size::fix(310.0, 118.0))
                            .with::<container::padding>(EdgeInsets::all(22.0))
                            .with::<container::background>(Color::rgba(0.92, 0.96, 1.0, 0.16))
                            .with::<container::border_color>(Color::rgba(1.0, 1.0, 1.0, 0.48))
                            .with::<container::border_width>(1.0)
                            .with::<container::border_radius>(20.0)
                            .with::<container::clip>(true),
                        backdrop_blur: 18.0,
                        on_click: |cx| {
                            let next = !highlighted.get();
                            controller.set_scene(demo_canvas_scene(next));
                            highlighted.set(cx, next);
                            EventResult::Consumed
                        },
                    ) {
                        column(gap: 7.0) {
                            text("GPU Backdrop Blur", color: Color::WHITE, font_size: 20.0, font_weight: FontWeight::Medium)
                            text(
                                "Click the glass card to swap the canvas scene.",
                                color: Color::rgba(0.92, 0.96, 1.0, 0.82),
                                font_size: 13.0,
                            )
                        }
                    }
                }
            }
        }
    }
}

// ---- shell -----------------------------------------------------------------

#[component]
pub fn gallery() {
    let initial: Page = cx.try_context::<InitialPage>().map_or(Page::Button, |page| page.get().0);
    let active: Page = state!(initial);
    let nav_style: StyleOf<Container> = StyleOf::new()
        .with::<container::width>(Sizing::fill())
        .with::<container::justify>(JustifyStyle::Start)
        .with::<container::padding>(EdgeInsets::symmetric(8.0, 6.0))
        .with::<container::min_height>(32.0);
    view! {
        container(
            style: StyleOf::new()
                .with::<container::size>(Size::fill())
                .with::<container::background>(ColorToken::Background)
                .with::<container::padding>(EdgeInsets::zero().set_top(32.))
                .with::<container::color>(ColorToken::Foreground),
            flex_direction: FlexDirectionStyle::Row,
        ) {
            container(
                style: StyleOf::new()
                    .with::<container::width>(240.0)
                    .with::<container::height>(Sizing::fill())
                    .with::<container::gap>(2.0)
                    .with::<container::padding>(EdgeInsets::all(12.0))
                    .with::<container::background>(ColorToken::Card),
                flex_direction: FlexDirectionStyle::Column,
            ) {
                container(
                    style: StyleOf::new()
                        .with::<container::gap>(8.0)
                        .with::<container::padding>(EdgeInsets::new(8.0, 8.0, 4.0, 20.0))
                        .with::<container::align>(AlignStyle::Center),
                    flex_direction: FlexDirectionStyle::Row,
                ) {
                    icon(data: lucide_rs::icons::component(), size: Size::fix(20.0, 20.0))
                    text("xui components", style: heading_style())
                }
                text(
                    "Components",
                    style: StyleOf::new()
                        .with::<container::padding>(EdgeInsets::new(8.0, 8.0, 0.0, 4.0))
                        .with::<container::color>(ColorToken::MutedForeground)
                        .with::<container::font_size>(FontSizeToken::Sm)
                        .with::<container::font_weight>(FontWeight::Medium),
                )
                for page in Page::ALL {
                    button(
                        text: page.title().to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Small,
                        leading: lucide(page.icon(), 16.0),
                        on_click: |cx| active.set(cx, *page),
                        style: if *active == *page {
                            nav_style
                                .clone()
                                .with::<container::background>(ColorToken::Accent)
                                .with::<container::color>(ColorToken::AccentForeground)
                        } else {
                            nav_style.clone().with::<container::color>(ColorToken::MutedForeground)
                        },
                    )
                    .key(*page)
                }
            }
            container(
                style: StyleOf::new()
                    .with::<container::width>(1.0)
                    .with::<container::height>(Sizing::fill())
                    .with::<container::background>(ColorToken::Border),
            )
            container(
                style: StyleOf::new()
                    .with::<container::size>(Size::fill())
                    .with::<container::padding>(EdgeInsets::new(48.0, 48.0, 40.0, 64.0))
                    .with::<container::scroll_direction>(ScrollDirectionStyle::Vertical),
                flex_direction: FlexDirectionStyle::Column,
            ) {
                container(
                    style: StyleOf::new()
                        .with::<container::width>(Sizing::fill())
                        .with::<container::max_width>(760.0)
                        .with::<container::gap>(40.0),
                    flex_direction: FlexDirectionStyle::Column,
                ) {
                    vstack(gap: 8.0) {
                        text(
                            active.title(),
                            style: StyleOf::new()
                                .with::<container::font_size>(30.0)
                                .with::<container::font_weight>(FontWeight::Bold),
                        )
                        text(
                            active.description(),
                            style: StyleOf::new()
                                .with::<container::color>(ColorToken::MutedForeground)
                                .with::<container::font_size>(FontSizeToken::Lg),
                        )
                    }
                    match *active {
                        Page::Button => button_page(),
                        Page::Spinner => spinner_page(),
                        Page::Input => input_page(),
                        Page::Select => select_page(),
                        Page::Tabs => tabs_page(),
                        Page::Image => image_page(),
                        Page::VirtualList => virtual_list_page(),
                        Page::Canvas => canvas_page(),
                    }
                }
            }
        }
    }
}

/// What the app starts with: the gallery, at the root.
pub fn gallery_root(app: &mut App) {
    app.mount::<Gallery>(GalleryProps {});
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
                let mut app = App::new(|_| {});
                app.provide_context(InitialPage(page));
                app.mount::<Gallery>(GalleryProps {});
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
        let mut app = App::new(|_| {});
        app.provide_context(InitialPage(Page::Spinner));
        app.mount::<Gallery>(GalleryProps {});
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
                let mut app = App::new(|_| {});
                app.provide_context(InitialPage(page));
                app.mount::<Gallery>(GalleryProps {});
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
