# 多窗口：运行时与 shell（第 2 步）

状态：**提案**（2026-11-25），未实施。

本文只覆盖多窗口的第 2 步：`xui-core` 的 `App` / `GuiRuntime` / reactive runtime 与
`xui-shell` 如何从"一次运行一个窗口"变成"一个应用多个窗口"。其余步骤只在需要时提到：

| 步骤 | 内容 | 本文 |
|---|---|---|
| 1 | 平台契约带上 `WindowId`（`xui-interface::platform`） | 只列出第 2 步需要的形状，见 §9 |
| **2** | **`App` 管多个 `UiRuntime`；`Shell` 拆成应用级与窗口级** | **全文** |
| 3–5 | `xui-macos`、`xui-winit`、`xui-windows` 接上 | 不涉及；第 2 步落地时宿主仍是单窗口，见 §10 |
| 6 | 越出窗口边界的弹层（子窗口菜单、下拉） | 不涉及 |

## 0. 现状：单窗口假设在哪里

| 位置 | 假设 |
|---|---|
| `xui-core/src/app.rs` `App` | 一个 `ui_runtime`、一个 `event_translator`、一个 `size`、一个 `PlatformHandle`；`App::mount` 挂到 `ui_runtime.root()` |
| `xui-core/src/reactive/runtime.rs` | `flush` / `run` / `mount` / `unmount` / `hot_reload` 都只接一个 `&mut UiRuntime`；`Owner::Root(NodeId)` 和 `Owner::Portal { layer }` 隐含了同一棵树 |
| `xui-core/src/system.rs` | `window::{Focused, Visible, Minimized, Fullscreen, Maximized}`、`viewport::{LogicalSize, ScaleFactor}` 是**应用级** keyed context（`provide_keyed`） |
| `xui-core/src/runtime.rs` `GuiRuntime` | 一个 `App`、一个 `backend`、一个 `TextHost`、一个 `platform_output` |
| `xui-core/src/platform.rs` `Bridge` | 标题、surface、window_state 与命令队列、菜单栏、请求表混在一起 |
| `xui-shell/src/shell.rs` `Shell` | 一个 `Arc<W>` + 一个 `GuiRuntime`；输入状态（modifiers、buttons、光标位置）、首帧显示、可见性、定时重绘都是单份 |
| `xui-interface/src/widget.rs` `NodeId` | 每个 `HostTree` 自己的 `SlotMap` 键——两棵树会发出**相同的** `NodeId` |
| `xui-core/src/text/cache.rs` `TextHost` | `owners: FxHashMap<NodeId, OwnerCache>`，按 `NodeId` 记账 |

不受影响的：`UiRuntime` 内部各 pass、render graph、布局、样式、`slot`（phase 是 thread-local，
多个 `UiRuntime` 在同一线程上顺序执行即可）、绝大多数组件。

## 1. 决定一览

| # | 决定 | 理由（详见各节） |
|---|---|---|
| D1 | **一个 `App`、一个 reactive runtime 管全部窗口** | `Signal` 挂在 runtime 的依赖图上；一窗一 App 就无法共享状态 |
| D2 | **每个窗口一个 `UiRuntime`**，`UiRuntime` 仍是单根 | 不动约 150 处 `root` / `root_overlayer` 的使用；焦点、hover、capture、overlay 天然按窗口隔离 |
| D3 | **`NodeId` 在整个 `App` 内唯一**：各 `HostTree` 共用一个分配器 | `TextHost`、后端、节点回复都按 `NodeId` 记账；共用分配器让它们不必知道窗口 |
| D4 | **窗口是组件**：`window(..) { .. }` 块，实现为 portal 的变体 | 窗口生命周期 = 挂载生命周期；context、signal 自然穿过窗口 |
| D5 | 窗口级 system context 由**窗口作用域**提供，不再是应用级 | 两个窗口的 `LogicalSize`、`Focused` 不同 |
| D6 | `TextHost`、frame clock、tokio、主题共享；**后端每窗口一个** | 布局与缩放无关，可共享；surface 必然按窗口 |
| D7 | `Shell` 拆成 `Shell`（应用级）+ `WindowShell`（窗口级） | 输入状态、首帧显示、可见性是窗口的；命令、退出策略是应用的 |
| D8 | **菜单栏两级**：应用级默认 + 窗口级 `WindowMenu`，显示哪份由宿主决定 | macOS 菜单随 key window 变、无窗口时仍要有菜单；Windows 菜单挂在 hwnd 上 |

## 2. D1 为什么是一个 App

`Signal::new(self.graph.insert(), value)`（`reactive/runtime.rs:494`）：signal 是某个
runtime 的图中的一个节点，订阅、调度、memo 层级都在这个图里。两个 `App` 意味着两张图，
一个窗口里的 handler 写另一个窗口读的 signal 没有路径可走，只能另造一套跨图同步。

一个 App 之下：
- `app.signal(..)`、`provide_context`、`context!()`、`resource!` 不变，跨窗口共享是默认行为；
- 一次 flush 可能同时弄脏几个窗口——这只影响"哪些窗口需要重绘"，见 §7。

## 3. D2/D3 窗口状态与 `NodeId`

### 3.1 `App` 的新形状

**部分实施**（2026-11-25）：`crates/xui-core/src/windows.rs` 有 `Windows` 与 `WindowState { ui, event_translator, size }`，
`App` 的这三个字段已换成 `windows`，目前一律经 `windows.main()` / `main_mut()` 访问，行为不变。
`App` 的方法还没有加 `window: WindowId` 参数，`PlatformHandle` 还没拆，`attached` / `owner` / `contexts` 字段还没加——
这些随 reactive 改造（§4.2）和窗口级 context（§5）一起做。下面是目标形状。

```rust
new_key_type! { pub struct WindowId; }   // xui-interface::platform

pub struct App {
    reactive: reactive::Runtime,
    windows: Windows,                 // 新
    tokio_runtime: TokioRuntime,
    finished: mpsc::Receiver<..>,
    platform: AppPlatform,            // 应用级：命令队列、请求表、默认菜单栏、capabilities、appearance
    theme: ThemeHandle,
    shortcuts: ShortcutManager,       // 应用级快捷键，解析到 key window
    #[cfg(feature = "hotpatch")] hot: HotRoot,
}

pub(crate) struct Windows {
    nodes: Rc<RefCell<SlotMap<NodeId, WindowId>>>,  // D3：共用分配器，值是节点所属窗口
    map: SlotMap<WindowId, WindowState>,
    main: WindowId,
    key: Option<WindowId>,            // 最后获得焦点的窗口，应用快捷键发往它
}

pub(crate) struct WindowState {
    ui: UiRuntime,
    event_translator: EventTranslator,
    size: Size<f32>,
    platform: WindowPlatform,         // 窗口级：标题、菜单（WindowMenu）、surface、window_state；它发出的命令都盖上 WindowId
    attached: bool,                   // 宿主是否已为它建好原生窗口（§8）
    owner: Option<InstanceId>,        // 挂载它的 window 块；主窗口为 None
    contexts: WindowContexts,         // D5
}
```

`App` 上现有的单窗口方法（`resize`、`set_scale_factor`、`dispatch_event`、`render`、
`set_window_visible` …）都加上 `window: WindowId` 参数；为了让单窗口调用方和测试不改，
保留只作用于 `windows.main` 的同名便捷方法，或者全部改为显式传参——实施时二选一，倾向后者，
调用方只有 `GuiRuntime` 和测试。

### 3.2 `NodeId` 共用分配器（D3）

**已实施**（2026-11-25）：`ui_runtime/host_tree.rs` 的 `NodeIds`（`Rc<RefCell<SlotMap<NodeId, WindowId>>>`），
`HostTree::with_ids`，`UiRuntime::with_node_ids`。`UiRuntime::new()` 仍给自己一个独立分配器，
所以现在的单窗口应用和全部测试照旧；第 2 步的 `Windows` 改用 `with_node_ids` 传共享的那一个。
`HostTree` 被 drop 时把仍持有的 id 还给分配器。

原来 `HostTree::nodes` 是 `SlotMap<NodeId, HostNode>`。改为：

```rust
struct HostTree {
    alloc: Rc<RefCell<SlotMap<NodeId, WindowId>>>,  // 所有窗口共用
    window: WindowId,
    nodes: SecondaryMap<NodeId, HostNode>,
    ..
}
```

- 建节点：`alloc.insert(self.window)` 拿键，再 `nodes.insert(key, node)`；删节点两边都删。
- `NodeId` 在整个 App 内唯一，于是：
  - `TextHost` 共享不需要改键（`owners` 仍按 `NodeId`）；
  - 后端的 `handle_node_lifecycle` 只收到自己窗口的事件，键不会撞；
  - `NodeReply { node, .. }` 可以用 `alloc[node]` 找到窗口，不必另存；
  - `debug_assert!(alloc[id] == self.window)` 可以放在 `UiRuntime` 的入口，抓"拿 A 窗口的 id 去 B 窗口用"这类 bug。
- 代价：`SecondaryMap` 按全局槽位下标索引，每个窗口的 `nodes` 长度跟着全局最大下标走。
  窗口数很少、节点数在万级，可以接受。

考虑过、不采用的方案：

| 方案 | 不采用的原因 |
|---|---|
| 每窗口一个 `TextHost` | 字体 shaping 缓存与 residency 预算被切成 N 份；同一段文字在两个窗口要排两次 |
| 所有按 `NodeId` 记账的地方改成 `(WindowId, NodeId)` | `TextHost`、后端、event system、平台回复全部改键，改动面最大 |
| 一个 `UiRuntime` 多个根 | 见 D2：要重写 `root` 相关的约 150 处，以及焦点、overlay、hit test 的全部按根拆分 |

## 4. D4 `window` 块

### 4.1 写法

与 `portal` 并列的一种模板节点：

```rust
xui! {
    column {
        button(on_click: move |cx| settings_open.set(cx, true)) { "Settings…" }
        if settings_open.get() {
            window(
                title: "Settings",
                size: (480.0, 360.0),
                resizable: false,
                on_close_requested: move |cx| settings_open.set(cx, false),
            ) {
                SettingsPane(..)
            }
        }
    }
}
```

- 块的内容挂在**新窗口**的 `UiRuntime` 根下；块本身在父模板里不占宿主节点（和 portal 一样）。
- 挂载 ⇔ 窗口存在：块挂载时创建 `WindowState` 并发出 `OpenWindow`，卸载时卸掉内容、发出 `CloseWindow`、删除 `WindowState`。
- 选项（title、size、resizable、decorations、…，即现有的 `WindowOptions`）与 portal 一样**挂载时读取一次**；之后要改标题等，用窗口自己的 handle（§6.2）。与 `docs/reactive.md` 里 portal 选项的已知限制相同。

### 4.2 reactive runtime 改动

**部分实施**（2026-11-25）：`flush` / `run` / `mount*` / `unmount` / `order_rows` / `hot_reload`、
`BlockCx`、`Children` 与 `PendingMount` 的闭包都已改为接 `&mut Windows`；`Mounted.window` 在挂载时由
`window_at(owner)` 得出（`Owner::Root(host)` 经共享分配器查 host 所在的窗口，其余继承所在 scope 的），
用到树的地方按它取 `windows.ui_mut(window)`。`Owner::Root` 保持 `Root(NodeId)`——窗口可由 host 查得，
不必另存。`Owner::Window`、`mount_window` 与 `window` 块尚未做（需要 `xui!` 支持新的节点种类）。

```rust
pub(crate) enum Owner {
    Root { window: WindowId, host: NodeId },          // 原 Root(NodeId)
    Child { instance: InstanceId, node: u16 },
    Row { instance: InstanceId, node: u16 },
    Portal { instance: InstanceId, layer: NodeId },
    Window { instance: InstanceId, window: WindowId }, // 新：window 块的内容
}

struct Mounted {
    window: WindowId,   // 新：挂载时从 owner 继承；Window owner 取它自己的 window
    ..
}
```

- 现在所有带 `ui: &mut UiRuntime` 的函数（`flush`、`run`、`mount*`、`unmount`、`order_rows`、`hot_reload`、
  `BlockCx`、`HostCx`、`pass.rs` 里的 `MountChildren`）改为接 `&mut Windows`，在用到树的那一刻按
  `self.instances[id].window` 取 `&mut UiRuntime`。这一步是机械的，但改动面最大，签名改动遍及
  `reactive/runtime.rs`、`pass.rs`、`template.rs`。
- `HostCx` 仍只持有一个 `&mut UiRuntime`（一次 render binding 只写自己窗口的宿主），借用不冲突。
- `BlockCx` 需要 `&mut Windows`：一个块的行里可能有 `window` 块，挂载它要新建 `WindowState`。
- `mount_window(id, node)` 仿照 `mount_portal`（`reactive/runtime.rs:780`）：建窗口状态 → 以
  `Owner::Window` 挂内容 → 记入 `Mounted.windows: Vec<MountedWindow { node, window }>` 以便卸载。
- `parent_host` 遇到 `Owner::Window { window, .. }` 返回该窗口的 `ui.root()`。
- 卸载顺序：先卸内容（窗口内组件的 effect cleanup 照常跑），再删 `WindowState`、发 `CloseWindow`。
  `WindowState` 删除后仍在排队的 effect 由 `schedule_pending` 里已有的"实例已卸载就丢弃"过滤掉，不需要新检查。

### 4.3 窗口内的 portal

portal 挂在**它所在窗口**的 `root_overlayer` 下：`mount_portal` 里的 `ui` 就是 `instances[id].window`
对应的那一个，代码不变。下拉、tooltip、dialog 仍然在各自窗口内。

## 5. D5 窗口级 context

`window::*` 和 `viewport::*` 七个 key 从应用级改为窗口级：

- `WindowState.contexts` 存这七个 key 的 signal，由 `App` 在对应窗口的事件里写（原来写 `provide_keyed` 的地方改为写窗口的）。
- `Contexts::find`（`reactive/runtime.rs:452`）沿 owner 链向上，走到 `Owner::Root { window, .. }` 或
  `Owner::Window { window, .. }` 时，先查该窗口的 `contexts`，再查应用级。
- 其他 context（主题、`Appearance`、`app.provide_context` 提供的）保持应用级，所有窗口看到同一份。
- `Appearance` 暂按应用级处理（系统外观）；macOS 允许单窗口覆盖外观，需要时再把它挪进窗口级。

新增一个窗口级 key：`window::Id: WindowId`，组件用它拿自己所在的窗口（给 §6.2 的 handle 用）。

## 6. 事件、命令、平台 handle

### 6.1 事件路由

`RuntimeEvent` 分成两类：

```rust
pub enum RuntimeEvent {
    Window(WindowId, WindowEvent),   // Resize、Input、Drag、ScaleFactor、Visible、Minimized、WindowState、CloseRequested、Attached
    Command { window: Option<WindowId>, command, timestamp },  // 菜单命令：发往菜单所属的窗口（§6.3）
    Reply(RequestId, PlatformReply), // 按请求表找回调；节点回复按 alloc[node] 找窗口
    Capabilities(..), Appearance(..), RedrawRequested, Exit,
}
```

- 输入只进它所属窗口的 `UiRuntime` + `EventTranslator`；hover、capture、焦点、IME 互不干扰。
- **key window**：收到某窗口的 `WindowFocus` 时设为 `windows.key`。应用级快捷键（`ShortcutManager`）
  发往 key window 的焦点节点，无焦点时发往该窗口的 command root。局部快捷键（`resolve_local_shortcut`）
  照旧在事件所在窗口解析。菜单命令的路由见 §6.3。
- 事件处理前的 `rebuild_sync_if_needed`（flush）是全局的；`update_tree` 只对事件所在窗口做。

### 6.2 平台 handle 拆分

`PlatformHandle` 现在一个 `Bridge` 装了全部（`platform.rs:30`）。拆成：

| | `AppPlatform`（应用级） | `WindowPlatform`（每窗口） |
|---|---|---|
| 状态 | 默认菜单栏、capabilities、appearance | 标题、菜单（`WindowMenu`）、surface、window_state |
| 命令 | 剪贴板、打开 URL、对话框、请求表 | 窗口命令（全屏、移动、尺寸……），自动盖上 `WindowId` |

- 两者共用一个命令队列（保持全局顺序），队列元素是 `(Target, PlatformCommand)`，`Target = App | Window(WindowId)`。
- 对外 API：`app.platform()` 返回应用级；`app.window(id).platform()` 返回窗口级；
  组件里经 `context!(window::Id)` 拿 id。`EventContext::send_platform` / `request_platform`
  自动用事件所在窗口——`TextInputWidget` 的剪贴板路径不用改。
- `PlatformOutput` 同样拆：`AppOutput { menu_bar }` + 每窗口 `WindowOutput { title, menu, cursor, text_input, surface }`，各自 diff。

### 6.3 菜单栏（D8）

菜单栏的语义是**"这个窗口处于活动状态时适用的菜单"**；画在屏幕顶部还是窗口内，是宿主的事。

各平台的事实：

- **macOS**：`NSApp.mainMenu` 全局只有一个，但内容应随 key window 变（文档窗口与设置窗口的菜单项、
  启用状态不同），切换 key window 时换 `mainMenu` 是常规做法。没有任何窗口时（`QuitPolicy::Explicit`）
  菜单栏仍在，至少要有 About、Quit、新建窗口。
- **Windows**：`HMENU` 由 `SetMenu(hwnd, ..)` 挂在某个窗口上，本来就是每窗口的。
- **Linux**（KDE 等全局菜单）：每个窗口经 DBus 各自导出。

所以有两级：

```rust
// 应用级：默认值，也是没有窗口时的菜单
app.platform().set_menu_bar(Some(bar));

// 窗口级
pub enum WindowMenu {
    Inherit,        // 用应用级的（默认）
    Hidden,         // 这个窗口不要菜单
    Own(MenuBar),   // 自己的
}
app.window(id).platform().set_menu(WindowMenu::Own(bar));
```

- **shell 只传递，不解析**：应用级的经 `PlatformHost::set_menu_bar`、窗口级的经
  `PlatformWindow::set_menu` 交给宿主，各自 diff。`xui-shell` 里不出现平台判断。
- **宿主的解析**：

  | | macOS | Windows |
  |---|---|---|
  | 显示哪份 | key window 的有效菜单；无 key window 时显示应用级的。宿主自己的应用菜单（About/Hide/Quit）始终在首位 | 每个窗口挂它自己的有效菜单 |
  | `Own(bar)` | `bar` | `bar` |
  | `Inherit` | 应用级的 | **只有主窗口**挂应用级的；其他窗口不挂——否则设置窗口、检查器都挂着整条主菜单，不合 Windows 习惯 |
  | `Hidden` | 同 `Inherit`：菜单栏不能为空 | 不挂 |

  主窗口默认 `Inherit`，所以现在单窗口应用的 `set_menu_bar` 在两个平台上行为都不变。
- **命令路由**：菜单命令发往**菜单所属的窗口**。宿主在 `ShellEvent::Command` 里填 `WindowId`：
  Windows 上是收到 `WM_COMMAND` 的那个 hwnd；macOS 上是选菜单时的 key window。
  `window == None`（macOS 上无窗口时选了应用级菜单）发往主窗口的 command root；主窗口也不存在时，
  发往应用级的命令处理器（`App::on_command`，新增，给"新建窗口"这类无窗口也要响应的命令用）。
  窗口内仍按现有规则：焦点节点开始冒泡，无焦点到该窗口的 command root。
- `enabled` / `checked` 照旧由应用写进 `MenuBar` 数据；窗口切换时宿主换上那个窗口的菜单，状态随之切换，不需要 AppKit 的菜单验证回调。

## 7. 帧

- **一次 frame 是"应用 settle + 若干窗口绘制"**：
  1. 全局：`drain_async_messages`、到期 wake、tickers、`reactive.flush`、主题。
  2. 每个 *attached、可见、脏* 的窗口：`update_tree(size, text)` → lifecycle → draw 到它自己的后端。
- `App::is_dirty` 变为 `dirty_windows() -> impl Iterator<Item = WindowId>`；
  一个窗口的 handler 写的 signal 弄脏另一个窗口时，shell 由此知道要向那个原生窗口 `request_redraw`。
- frame clock 全局一份。全部窗口都不可见时才 `suspend`；只有部分可见时，不可见窗口的
  `UiRuntime::set_window_visible(false)` 照旧让它不为动画请求帧。
- 宿主的重绘是按原生窗口来的（macOS 每个 view 一个 display link 回调）。`Shell::redraw(window)`
  先做第 1 步（若本轮尚未做过），再只画这个窗口；其余脏窗口通过 `request_redraw` 排到它们自己的回调里。
  这样不需要把多个窗口的 present 同步到同一时刻。

## 8. 窗口生命周期

```
window 块挂载
  → WindowState { attached: false }，内容立即挂载（组件可以先跑起来）
  → 命令 OpenWindow(id, WindowOptions)
宿主建好原生窗口（隐藏）、建后端
  → WindowEvent::Attached { size, scale_factor }   // attached = true，写 viewport context
  → 首帧画完 → 显示（沿用现有 pending_first_present 逻辑，按窗口）
用户点关闭
  → WindowEvent::CloseRequested
  → 有 on_close_requested：调用它，通常由它把挂载条件置 false → 卸载 → CloseWindow
  → 没有：隐藏窗口（window::Visible = false），状态保留；下次再挂载同一个块之前它都在
window 块卸载
  → 卸内容 → 删 WindowState → 命令 CloseWindow(id) → 宿主销毁原生窗口与后端
```

- `attached == false` 的窗口不做 `update_tree`、不绘制、不收输入；尺寸未知时跑布局没有意义。
- **主窗口**由宿主在启动时创建，`App::new` 时 `windows.main` 已存在且 `owner == None`，
  `App::mount` 挂在它的根下——`#[xui::main]` 和所有现有应用不变。
- **主窗口的关闭**和**退出策略**放在 `ShellOptions`：

  ```rust
  pub enum QuitPolicy {
      MainWindowClosed,   // 默认，即现在的行为
      LastWindowClosed,
      Explicit,           // 只有 RuntimeEvent::Exit 或应用发出 Quit 才退出（macOS 习惯）
  }
  ```

## 9. 需要第 1 步提供的契约

第 2 步依赖、但属于 `xui-interface::platform` 的部分（第 1 步先落地、行为不变）。

**已完成**（2026-11-25），只加词汇，没有任何调用方发出或处理它们：

- `WindowId`：`slotmap` 键，在 `platform/window.rs`，由第 2 步的 `Windows` 分配；
- `PhysicalSize`、`WindowSize`、`WindowOptions`、`MacOsWindowOptions` 从 `xui-shell` 挪到 `xui-interface::platform`，
  `xui-shell` 原路径重新导出，宿主不用改；
- `PlatformCommand::OpenWindow { window, options }`、`CloseWindow(WindowId)`；`PlatformCommand` 本就是
  `#[non_exhaustive]`，宿主的 `perform` 落到 `Unsupported`；
- `HostCapabilities::MULTI_WINDOW`；
- `WindowMenu`（§6.3），在 `platform/menu.rs`。

**推到第 2 步**——这些的形状取决于 `Shell` 怎么拆，提前定只会再改一次：

- `ShellEvent` 带 `WindowId`：窗口事件是在 `ShellEvent` 里带 id，还是 `dispatch_event(window, at, event)` 带参，随 §10 定；
  `Command` 带 `Option<WindowId>` 同理；
- 命令队列元素 `(Target, PlatformCommand)`：在 `xui-core::platform` 内部，随 §6.2 的拆分一起做。

## 10. D7 `xui-shell` 拆分

```rust
pub struct Shell<H: PlatformHost, B, T> {
    host: Arc<H>,                                   // 应用级：perform 应用命令、菜单栏、建窗口
    runtime: GuiRuntime<T>,                         // App + TextHost + frame clock；不再含后端
    windows: HashMap<WindowId, WindowShell<H::Window, B>>,
    make_backend: Box<dyn FnMut(&Arc<H::Window>) -> Result<B, ..>>,  // 现在的 FnOnce 工厂拆成 app 一次 + 后端每窗口
    options: ShellOptions,
    last_app_output: AppOutput,
}

struct WindowShell<W, B> {
    window: Arc<W>,
    backend: B,
    modifiers, pointer_buttons, last_cursor_position,
    live_resize, occluded, pending_first_present, timed_redraw,
    last_output: WindowOutput,
}

pub trait PlatformHost {
    type Window: PlatformWindow;
    fn open_window(&self, id: WindowId, options: &WindowOptions) -> Result<Arc<Self::Window>, ..>;
    fn close_window(&self, id: WindowId);
    fn set_menu_bar(&self, menu_bar: Option<&MenuBar>) {}   // 应用级默认菜单（§6.3）
    fn perform(&self, command: PlatformCommand) -> Performed { Performed::Unsupported }
    fn capabilities(&self) -> HostCapabilities;
}
```

- `PlatformWindow` 保持按窗口；`set_menu_bar` 改为 `set_menu(&WindowMenu)`，默认空实现；`perform` 只收窗口命令。
- `shell.rs` 现有的逻辑大多原样搬进 `WindowShell`（输入合成、首帧显示、可见性、定时重绘、窗口菜单 diff），
  应用级的只剩命令排空、回复轮次（`MAX_COMMAND_ROUNDS`）、默认菜单栏 diff、退出策略。
- `ShellHandle` 的可重入队列元素带 `WindowId`。
- **兼容**：提供一个 `SingleWindowHost<W>` 适配器，把只实现了 `PlatformWindow` 的宿主包成
  `PlatformHost`，`open_window` 返回 `Unsupported`、`capabilities().multi_window = false`；
  它把应用级菜单与主窗口的 `WindowMenu` 按 §6.3 的规则合成一份，交给窗口现有的菜单实现，
  所以 `xui-macos` 的 `menu.rs` 在第 2 步不用动。
  第 2 步落地时三个宿主都走这个适配器，行为与现在一致；在不支持的宿主上 `window` 块
  挂载但不 attach（`window::Visible == false`），应用可以据 capability 退回窗口内 dialog。

## 11. 测试

全部可在无窗口环境下跑（`xui-shell` 的测试宿主 + `MockRenderBackend`），并在
`--features strict-invariants` 下保持绿色：

- 共享状态：主窗口按钮写 signal，第二个窗口里读它的组件重新 render，且只有第二个窗口被标脏；
- 隔离：两个窗口各自的焦点、hover、capture、IME 互不影响；Tab 不跨窗口；
- `NodeId`：两个窗口建同样多的节点，键不重复；`debug_assert` 抓到跨窗口误用；
- 路由：菜单命令进它所带 `WindowId` 那个窗口的焦点节点，`None` 时进主窗口或 `App::on_command`；节点回复回到发请求的窗口；
- 菜单：应用级与窗口级各自 diff、分别到达 `PlatformHost` 与 `PlatformWindow`；`SingleWindowHost` 合成的结果符合 §6.3 的表；
- context：`viewport::LogicalSize` 在两个窗口不同；主题切换两个窗口都重算样式；
- 生命周期：`window` 块挂载 → `OpenWindow`；attach 前不绘制；`CloseRequested` 有/无回调两种；卸载 → `CloseWindow`，effect cleanup 都跑了；
- 退出策略三种；
- 窗口内 portal 挂在自己窗口的 overlayer 下；
- 单窗口宿主（`SingleWindowHost`）下现有 shell 测试原样通过，`window` 块不 attach；
- hotpatch（cfg）：`hot_reload` 覆盖所有窗口，主窗口根变化时整体重挂不丢第二个窗口的 `WindowState` 以外的状态。

## 12. 未决问题

1. **关闭请求的默认行为**：无回调时"隐藏并保留"还是"拒绝关闭"？前者贴近 macOS 设置窗口的习惯，但窗口会在应用不知情时变成不可见。
2. **窗口选项的更新**：挂载时读一次（与 portal 相同）是否够用，还是 `title` 这类选项要做成随 signal 更新的绑定。
3. **模态窗口**：`window(modal: true)` 是否在本步内——它需要禁用父窗口输入（macOS sheet / Win32 owner window），牵涉宿主，倾向放到第 3 步。
4. **`Appearance` 是否窗口级**（§5）。
5. **macOS 上的 `Hidden`**：现在等同 `Inherit`。是否要更细——比如去掉应用自己的菜单、只留宿主的应用菜单和 Window 菜单——视面板类窗口的需要再定。
6. **GPU 设备共享**：多个 Vello 后端共用一个 `wgpu::Device`（已有 `VelloBackend::new_with_device`）以及共享 glyph atlas，属于后端，不在本步，但 `make_backend` 的签名要允许传入共享资源。
