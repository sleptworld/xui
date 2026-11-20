# 主题设计

状态：**已实施**（2026-11-21）。依据 `xui-interface/src/style.rs`、`styles/units.rs`、`xui-core/src/ui_runtime/config.rs`、
`xui-core/src/app.rs`、`xui-animation` 与 `xui-components` 的现状整理。

## 0. 目标

| 目标 | 手段 |
|---|---|
| 应用能加自己的角色 | 内置与应用 token 是同一种东西：命名常量 |
| 一种机制 | 颜色、间距、圆角、字号都用 `Token<T>`；去掉别名 |
| 应用内切换主题 | 主题由运行时持有，事件回调经 context 中的 `ThemeHandle` 请求切换，下一帧生效 |
| 依赖主题的值随之更新 | token 在样式计算时解析，画布在绘制时读主题；组件不因换主题重新运行 |
| 跟随系统 | `ThemeMode::System` 按 `Appearance::color_scheme` 选亮色或暗色主题 |
| 主题可以是数据 | token 以名字为身份，主题文件的键就是 token 名 |

## 1. 现状

- `Theme` 是定长结构体：19 个颜色字段，spacing / radius / font size 各 4–5 档。`ColorToken` 等是枚举，应用无法扩展。
- `ColorToken` 有 4 个别名（`Text`、`Surface`、`MutedSurface`、`InverseText`），全仓共 21 处使用。
- `ColorValue` 只有 `Color`、`Token`、`TokenAlpha`；两色混合无法表达。token 之间的动画是离散跳变（`xui-animation` 中
  `ColorValue::interpolate` 只插值两个字面色）。
- 主题在 `UiRuntime` 中，只能经 `App::set_theme(&mut self)` 修改。事件回调拿到的 `Cx` 只能写信号，因此应用内无法切换；组件也读不到主题。
- `Appearance` 由 platform 保存，不驱动主题，也不触发组件更新（见 `shell-api.md`）。

用量：`ColorToken::*` 约 200 处，`SpacingToken` / `RadiusToken` / `FontSizeToken` 约 65 处，直接读 `Theme` 方法的生产代码不到 15 处，
几乎全部在 `xui-components`。

## 2. Token

```rust
/// A named design value. The name is the token's identity: declare each token
/// once, as a constant, and refer to that constant everywhere.
#[derive(Debug, Clone, Copy)]
pub struct Token<T> {
    pub name: &'static str,
    /// What a theme that does not set this token resolves it to.
    pub fallback: T,
}

impl<T> Token<T> {
    pub const fn new(name: &'static str, fallback: T) -> Self;
}

// PartialEq / Eq / Hash compare `name` only.
```

- **身份是名字。** 相等与哈希只看 `name`，因此 `Token<Color>` 是 `Eq + Hash`，样式缓存不必哈希浮点数。同名的两个 token 视为同一个；
  同名不同默认值是声明错误。
- **默认值随 token 声明。** 主题没有设置某个 token 时取 `fallback`，解析总有结果。主题只写需要覆盖的部分。
- **命名**：小写，`.` 分层，词内用 `-`。xui 内置 token 不带前缀；应用与库以自己的名字为前缀（`tsradar.selected-text`）。
- `Color` 增加 `pub const fn from_rgb_u32(0xRRGGBB)`，内置常量的默认值因此可以写成 `const`。

### 2.1 内置 token

放在 `xui_interface::tokens`，按种类分模块，并由 prelude 导出：

```rust
use xui::tokens::{color, spacing, radius, font_size};

.background(color::CARD).color(color::MUTED_FOREGROUND)
.padding(spacing::MD).radius(radius::SM).font_size(font_size::LG)
```

颜色（默认值即现在的 `Theme::light()`，取自 shadcn/ui neutral）：

| 常量 | 名字 | 用途 | 现状对应 |
|---|---|---|---|
| `BACKGROUND` | `background` | 窗口底色 | `Background` |
| `FOREGROUND_STRONG` | `foreground.strong` | 标题、强调文字 | 新增 |
| `FOREGROUND` | `foreground` | 正文 | `Foreground`、`Text` |
| `MUTED_FOREGROUND` | `muted.foreground` | 说明、占位、未激活标签 | `MutedForeground` |
| `FOREGROUND_FAINT` | `foreground.faint` | 禁用、分隔符等最弱一级文字 | 新增 |
| `CARD` / `CARD_FOREGROUND` | `card` / `card.foreground` | 卡片、面板 | `Card`、`Surface` / `CardForeground` |
| `POPOVER` / `POPOVER_FOREGROUND` | `popover` / `popover.foreground` | 浮层 | 同名 |
| `PRIMARY` / `PRIMARY_FOREGROUND` | `primary` / `primary.foreground` | 主按钮 | `Primary` / `PrimaryForeground`、`InverseText` |
| `SECONDARY` / `SECONDARY_FOREGROUND` | `secondary` / `secondary.foreground` | 次按钮 | 同名 |
| `MUTED` | `muted` | 弱化的面：tab 列表、骨架屏 | `Muted`、`MutedSurface` |
| `ACCENT` / `ACCENT_FOREGROUND` | `accent` / `accent.foreground` | 悬停、高亮 | 同名 |
| `DESTRUCTIVE` / `DESTRUCTIVE_FOREGROUND` | `destructive` / `destructive.foreground` | 危险操作 | 同名 |
| `SELECTION` / `SELECTION_FOREGROUND` | `selection` / `selection.foreground` | 选中的行、文字选区 | 新增 |
| `BORDER` | `border` | 分隔线、边框 | `Border` |
| `INPUT` | `input` | 表单控件边框 | `Input` |
| `RING` | `ring` | 焦点环 | `Ring` |

前景分四级：`foreground.strong` > `foreground` > `muted.foreground` > `foreground.faint`，对应 SwiftUI 的分层前景色。

长度：

| 模块 | 常量 | 名字 | 默认值 |
|---|---|---|---|
| `spacing` | `XS` `SM` `MD` `LG` `XL` | `spacing.xs` … `spacing.xl` | 4 8 12 16 24 |
| `radius` | `SM` `MD` `LG` `XL` | `radius.sm` … `radius.xl` | 6 8 10 14 |
| `font_size` | `SM` `MD` `LG` `XL` | `font-size.sm` … `font-size.xl` | 12 14 16 20 |

### 2.2 应用 token

与内置的写法、用法完全相同：

```rust
pub const SELECTED_TEXT: Token<Color> =
    Token::new("tsradar.selected-text", Color::from_rgb_u32(0xa9c4ee));

view! { text(name).color(SELECTED_TEXT) }
```

## 3. 样式值

```rust
/// A color before the theme resolves it.
pub enum ColorRef {
    Color(Color),
    /// A token with its resolved alpha multiplied by the factor.
    Token(Token<Color>, f32),
}

pub enum ColorValue {
    One(ColorRef),
    /// `a` mixed toward `b` by `t` in 0..=1, like CSS `color-mix()`.
    Mix(ColorRef, ColorRef, f32),
}

pub enum LengthValue {
    Px(f32),
    Token(Token<f32>),
}
```

- `Color`、`Token<Color>` 都实现 `Into<ColorRef>`、`Into<ColorValue>`、`Into<ColorStyle>`；`Token<f32>` 实现 `Into<LengthValue>`。
  `.color(color::PRIMARY)`、`.background(Color::WHITE)` 照常可写。
- `Token<Color>::alpha(a) -> ColorValue` 取代 `ColorToken::alpha` 与 `TokenAlpha`；`ColorValue::alpha(a)` 对任意颜色值乘透明度，
  禁用态整体减半即用它。
- `Token<Color>::mix(other, t) -> ColorValue` 用于派生色，例如 `color::ACCENT.mix(color::PRIMARY, 0.2)`。
- **动画**：`ColorValue::interpolate(One(a), One(b), t)` 得到 `Mix(a, b, t)`，悬停等 token 之间的过渡因此连续，不再跳变。
  只有一端已是 `Mix` 时才退回离散切换。

## 4. Theme

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: String,
    /// Whether this is a light or a dark theme, for code that must choose
    /// between assets, such as an icon set or a viewport's clear color.
    pub scheme: ColorScheme,
    colors: HashMap<String, Color>,
    lengths: HashMap<String, f32>,
}

impl Theme {
    /// An empty theme: every token resolves to its fallback.
    pub fn new(name: impl Into<String>, scheme: ColorScheme) -> Self;
    /// shadcn/ui neutral.
    pub fn light() -> Self;
    pub fn dark() -> Self;

    pub fn get<T: ThemeValue>(&self, token: Token<T>) -> T;
    pub fn set<T: ThemeValue>(&mut self, token: Token<T>, value: T);
    pub fn with<T: ThemeValue>(self, token: Token<T>, value: T) -> Self;
}

/// The value kinds a theme stores: `Color` and `f32`.
pub trait ThemeValue: Copy + sealed::Sealed { /* selects the table */ }
```

- `light()` 就是 `Theme::new("Light", ColorScheme::Light)`，因为内置 token 的默认值已是亮色；`dark()` 覆盖全部内置颜色。
- 表的键是 `String`，以 `&str` 查找；代码构造与文件加载共用同一张表。每次解析查一次哈希表，表的大小在几十项。
- 现有 `Theme::color / spacing / radius / font_size` 统一为 `get`。`ComputedStyle::initial` 取 `color::FOREGROUND` 与 `font_size::MD`。

## 5. 运行时：持有、选择、读取

### 5.1 应用启动时

```rust
pub enum ThemeMode { System, Light, Dark }

pub struct Themes {
    pub light: Rc<Theme>,
    pub dark: Rc<Theme>,
}

app.set_themes(Themes { light, dark });   // 默认 Theme::light() / Theme::dark()
app.set_theme_mode(ThemeMode::System);    // 默认 System
```

`App::set_theme` 删除。只要一套主题的应用用 `Themes::single(theme)`，把 `light` 与 `dark` 设成同一个。
`App::theme()` 是当前主题，`App::theme_handle()` 是组件经 context 拿到的同一个 `ThemeHandle`。

### 5.2 主题是环境，不是响应式状态

主题与缩放比例、窗口尺寸一样属于应用的环境：设置它记下一个请求，下一帧生效，不经过信号。依赖主题的工作只发生在值被解析
或绘制的时候——token 在样式计算时解析，画布的绘制函数经 `canvas.theme()` 读取——组件不会因换主题而重新运行。

运行时以应用级 context 提供两样东西：

| context | 类型 | 用途 |
|---|---|---|
| `ThemeHandle` | 句柄，整个应用生命周期内不变 | 事件回调切换模式或替换主题；非响应式地读模式与当前主题 |
| `Appearance` | 系统外观 | 由 `RuntimeEvent::Appearance` 更新，读者随之更新 |

```rust
pub struct ThemeHandle(/* Rc<RefCell<..>> */);

impl ThemeHandle {
    pub fn mode(&self) -> ThemeMode;
    pub fn themes(&self) -> Themes;
    pub fn current(&self) -> Rc<Theme>;
    pub fn set_mode(&self, mode: ThemeMode);    // 记下请求，下一帧生效
    pub fn set_themes(&self, themes: Themes);
}

let theme: ThemeHandle = context!();
// in a handler
theme.read().set_mode(ThemeMode::Dark);
```

显示当前模式的设置界面（系统／亮色／暗色）由应用用自己的状态保存选择，在回调里同时写它并调用 `set_mode`：这个选择本就是
应用的偏好，保存与恢复也由应用负责。

### 5.3 选择当前主题

句柄上有未处理的请求时 `App::is_dirty()` 为真，宿主据此请求一帧。`App` 在每次样式计算之前（`rebuild_sync_if_needed`，
flush 之后，effect 发出的请求也赶得上同一帧）取走请求，由（`mode`，`themes`，`appearance.color_scheme`）选出主题，与当前的
比指针；不同时交给 `UiRuntime::set_theme`：

1. 重设默认样式，根节点标记 `RESTYLE_TREE`，整棵树重新计算样式并重新布局（长度 token 可能变）；
2. 以绘制函数作画的画布与文本表面重绘；保留模式的场景不读主题，不重建。

`App::set_themes`、`App::set_theme_mode` 与外观事件当场选择，不等下一帧。`UiRuntime` 持有 `Rc<Theme>`，画布每次绘制取用的是
同一份主题。

## 6. 主题文件

```toml
name = "Radar Dark"
scheme = "dark"

[colors]
background = "#0d0e0f"
"foreground.strong" = "#e8e9ea"
"tsradar.selected-text" = "#a9c4ee"

[lengths]
"radius.sm" = 4
```

`xui::theme::parse(&str) -> Result<Theme, ThemeFileError>` 解析上述格式：`name` 与 `scheme`（`"light"` 或 `"dark"`）必填，
`[colors]` 与 `[lengths]` 可省，其下的键是 token 名；颜色接受 `#rgb`、`#rrggbb`、`#rrggbbaa`（`Color::parse_hex`），长度写整数或
小数均可。未知的顶层键、写错的 `scheme` 与颜色都报错，颜色错误指出是哪个键。

主题文件放在资源包中，经 `load_asset::<ThemeAsset>(id)` 读取。调试构建下 `dev_directory` 挂载源目录，再次读取并交给
`ThemeHandle::set_themes` 即可在运行中看到修改，不必重新构建。解析放在 `xui-core`，`xui-interface` 只多了
`Color::parse_hex` 与按名字设置的 `Theme::insert`，不依赖 TOML。

## 7. 迁移

| 现状 | 改为 |
|---|---|
| `ColorToken::Primary` 等 | `color::PRIMARY` 等 |
| `ColorToken::Text` / `Surface` / `MutedSurface` / `InverseText` | `color::FOREGROUND` / `CARD` / `MUTED` / `PRIMARY_FOREGROUND` |
| `ColorToken::X.alpha(a)`、`ColorValue::TokenAlpha` | `color::X.alpha(a)` |
| `ColorValue::Color(c)` / `Token(t)` | `c.into()` / `t.into()` |
| `LengthValue::Spacing(SpacingToken::Md)` 等 | `spacing::MD.into()` 等 |
| `theme.color(t)` / `spacing` / `radius` / `font_size` | `theme.get(t)` |
| `App::set_theme(theme)` | `App::set_themes` + `App::set_theme_mode` |
| `XUI_THEME=dark`（示例应用） | `set_theme_mode(ThemeMode::Dark)` |

改动集中在 `xui-interface`（类型）、`xui-core`（解析、运行时、context）、`xui-animation`（插值）、`xui-components`（约 270 处替换）
与示例应用。`xui-property` 自有的 `Theme` 与此无关，不动。

## 8. 实施顺序

1. **数据模型**：`Token<T>`、内置 token、新 `Theme`、`ColorRef` / `ColorValue` / `LengthValue`、`Color::from_rgb_u32`；
   迁移 `xui-components` 与示例应用；token 动画插值。此步之后外观与现在一致。
2. **运行时**：`Themes`、`ThemeMode`、`ThemeHandle` 与 `Appearance` 两个 context；当前主题的选择；画布失效；macOS 上报外观。
3. **主题文件**：`theme::parse`、`ThemeAsset`、`Color::parse_hex`、`Theme::insert`。

## 9. 限制

- 主题漏设某个 token 时静默取默认值，主题文件里拼错的键也静默无效：token 不登记，`Theme` 不知道有哪些名字。需要时可以加一个
  调试构建下的检查，列出文件中未被任何已解析 token 使用过的键。
- 一棵树只有一个当前主题，不支持像 Flutter `Theme(data:, child:)` 那样给子树换主题。
- 组件不能响应式地读当前主题。要按深浅选不同资源（如一套图标）时，先用 token 着色；确有需要再提供当前 `ColorScheme`
  的 context。`Appearance` 是系统外观，模式选定亮或暗时与当前主题不一致，不能替代。
- 主题切换本身没有过渡动画；以后可由 `Theme` 的逐项插值实现。
- 只有 macOS 宿主上报系统外观，且只报深浅：内容视图在 `viewDidChangeEffectiveAppearance` 中发出
  `ShellEvent::AppearanceChanged`，`MacWindow::appearance` 给出启动时的外观（`cargo run -p xui-macos --example appearance`
  检查这条链路）。winit 与 Windows 宿主仍返回默认值，那里的 `ThemeMode::System` 总是亮色。
- `Appearance` 的 `accent`、`high_contrast` 暂不使用；以后可分别映射到 `color::PRIMARY` / `color::RING` 与一套高对比主题。
