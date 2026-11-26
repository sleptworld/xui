# 宿主能力的两个层级：系统组件与系统服务

状态：设计稿，未实现。建立在 [`shell-api.md`](shell-api.md) 的四条通道和请求生命周期、
[`multi-window.md`](multi-window.md) 的按窗口划分之上。

## 两类能力

app 从宿主那里要的东西分两个层级，不能放进同一个接口：

| | 系统组件 | 系统服务 |
| --- | --- | --- |
| 例子 | 右键菜单、消息框、文件对话框，以后还有颜色/字体选择器、分享面板 | 通知、Dock/任务栏角标与进度、托盘图标、打开文件/URL、退出确认，以后还有全局快捷键 |
| 归属 | 代表**某个窗口**出现，挂在一次交互上 | 属于 **app**，与窗口无关；所有窗口都关了也照样存在 |
| 谁决定呈现 | app 决定何时、何处出现，用户操作完就结束 | 系统决定：显示与否、分组、勿扰模式、是否留在通知中心 |
| 生命周期 | 调用方一直在场等结果；调用方卸载就取消 | 可能比调用方、甚至比进程活得久（点击通知会冷启动 app） |
| 前提 | 无 | 往往需要身份和权限：bundle id、AUMID、用户授权 |
| 不支持时 | 可以退回自绘（自绘菜单、自绘对话框） | 没有等价的退回方式（toast 不是通知） |
| 形状 | 请求 → 回复，从组件发起 | 状态（diff）、命令、请求，再加上**宿主主动发起的 app 事件** |

通知横幅虽然也是系统画的，但 app 只是把一份数据交给通知中心，不拥有、也不控制它的呈现，
所以它是服务而不是组件。

## 结构上的后果

现在所有平台动作都经 `PlatformWindow::perform` 下发，`PlatformHandle` 发出的每条命令
都带着窗口 id（`Bridge::commands: Vec<(WindowId, PlatformCommand)>`）。宿主侧只有
`PlatformWindow` 和 `WindowOpener`，**没有 app 级的接口**。

- **系统组件放在这条路上是对的**：它本来就是窗口的。
- **系统服务不行**：没有窗口时它该经谁发？发通知时随便选一个窗口，那个窗口关闭后，
  点击通知的事件又该回到哪里？

所以分成两条通道：

```
                 runtime (xui-core)                     host (xui-shell trait)
系统组件   PlatformHandle（按窗口）  ── PlatformRequest ──►  PlatformWindow::perform
系统服务   ServiceHandle（按 app）   ── ServiceRequest  ──►  PlatformApp::perform     ← 新增
                                     ◄─ AppEvent ─────────  ShellEvent::App          ← 新增
                                     ◄─ AppOutput（diff）─  PlatformApp::sync         ← 新增
```

两条通道分开以后，类型系统就能保证没有窗口的宿主接口不会被要求弹出菜单，
窗口接口也不会被要求发通知。

---

# 第一部分：系统组件

## 组件够不到宿主

现在只有 widget 能通过 `EventContext::request_platform` 发请求，回复被投递给节点。
`#[component]` 写的组件只能自己捕获一份 `App::platform()` 的克隆，回调里也没有 `Cx`，
写不了 signal（`shell-api.md` 里写着 "No component-facing hook"）。
自绘菜单和原生菜单的切换就发生在组件这一层，所以先补上这一块。

**平台句柄作为窗口 context。** 每个窗口本来就有一份 `PlatformHandle`，把它和
`window::Focused` 等一起在 `provide_window_defaults` 里提供出去：

```rust
let platform: PlatformHandle = context!(window::Platform);
```

组件拿到的就是它所在窗口的句柄，多窗口下不需要额外处理。

**带 `Cx` 的请求，随组件取消。**

```rust
impl PlatformHandle {
    /// 回复时以一个 handler Cx 调用 `on_reply`，回调里可以写 signal。
    /// 返回的 guard 被 drop 时取消请求（发出 `PlatformCommand::Cancel`）。
    pub fn request_with(
        &self,
        cx: &Cx<'_>,
        request: PlatformRequest,
        on_reply: impl FnOnce(&mut Cx<'_>, PlatformReply) + 'static,
    ) -> PendingRequest;
}
```

`Cx` 只持有一个可克隆的 `Notifier`，所以回复到达时用 `Cx::handler(&notifier)`
重建一个 Cx 即可，和 `SizeObserver::reactive` 的做法相同。组件把 `PendingRequest`
存在一个 typed `let` 字段里，实例销毁时字段随之 drop，请求就取消了。
这满足了 `shell-api.md` 第 6 节"调用方卸载即 Cancel"的约定，不需要改宏。

## 右键菜单

### 现状

- **接口：** `PlatformCommand::ShowContextMenu { menu, at }` 是一条**命令**。选中的项以命令事件返回，取消菜单时什么都不发。
- **macOS：** 回复 `Unsupported`，排在 `shell-api.md` 的 phase 2。
- **组件：** `xui-components` 的 `context_menu` 是自绘的 portal，它的 `MenuItem { id, label, disabled }`
  和 `xui_interface::platform::MenuItem` 重名。

### 改成请求，和消息框、文件对话框并列

```rust
PlatformRequest::ContextMenu { menu: Arc<Menu>, at: Option<Point> }
PlatformReply::MenuChoice(Option<MenuPath>)   // None：被取消

/// 被选中项在 `Menu::entries` 中的下标路径，子菜单逐级向下。
pub struct MenuPath(pub Vec<u16>);
```

改成请求的理由有三条：

1. **受控的 `open` 需要知道"取消"。**
2. **组件要的是"选了哪一项"。** `CommandId` 是 `&'static str`，表达不了"删除第 5 行"这种运行时才知道的项。
3. **能复用请求的生命周期。** 组件卸载或 `open` 被置为 `false` 时，`Cancel` 让宿主收起菜单
   （AppKit 用 `cancelTracking`，Win32 用 `EndMenu`）。

**带命令的项。** `MenuItem.command` 改为 `Option<CommandId>`。选中带命令的项时，
由运行时在投递回复的同时，从发起请求的节点派发这个命令。待处理请求里本来就存着这份
`Arc<Menu>`，所以宿主只需返回路径。"复制、粘贴"这类标准项因此和快捷键、菜单栏走同一条路。

**`at: None`** 表示键盘触发（`ContextMenuTrigger::Keyboard`），菜单放在焦点节点附近。
节点的边界由运行时填入请求。

### 宿主实现要点

- **macOS。** `popUpMenuPositioningItem:atLocation:inView:` 会进入一个嵌套的 tracking run loop，
  不能在 `perform` 里同步调用，因为此时 `Shell` 正被借用，run loop 里的事件会重入。
  做法是：`perform` 返回 `Performed::Pending`，把弹出操作 `dispatch_async` 到主队列；
  弹出返回后，经 `ShellHandle::submit` 发出 `ShellEvent::Reply`。
  还要检查 display link 是否注册在 common modes 下，否则菜单打开期间动画会停。
- **Windows。** `TrackPopupMenuEx(TPM_RETURNCMD)` 同样是模态的，用 `PostMessage` 推迟执行，原理相同。
- **winit。** 回复 `Unsupported`，组件退回自绘。
- **能力位：** 用已有的 `NATIVE_CONTEXT_MENU`。

### 组件

```rust
context_menu(trigger: .., items: .., presentation: MenuPresentation::Auto)  // Auto | Native | Drawn
```

- **`Auto`：** 宿主支持时走原生，否则用自绘 portal。`Native` 在不支持的宿主上同样退回自绘。
- **条目：** 扩展为 `Item / Separator / Submenu`，原生和自绘共用同一份数据。
  条目带图标或自定义行内容时，`Auto` 选择自绘。
- **重名：** 组件的 `MenuItem` 改名，并提供到平台 `MenuEntry` 的 `From` 转换。
- **原生流程：** `on_context_menu` → `request_with`，guard 存在字段里 →
  回复 `Some(path)` 时调用 `on_select` 并关闭，`None` 时只关闭 → 受控 `open` 变为 `false` 时 drop guard。

---

# 第二部分：系统服务

## app 级通道

### 宿主侧

```rust
// xui-shell
pub trait PlatformApp {
    fn capabilities(&self) -> ServiceCapabilities { ServiceCapabilities::NONE }
    /// 和 `PlatformWindow::perform` 同一套 `Performed` 约定（Done/Reply/Pending/Unsupported）。
    fn perform(&self, command: ServiceCommand) -> Performed { Performed::Unsupported }
    /// app 级状态的 diff 结果：角标、托盘等。
    fn sync(&self, output: &AppOutput, previous: &AppOutput) {}
}
```

- 和 `WindowOpener` 一样由宿主交给 `Shell`（`Shell::with_app(..)`），不提供时等于全部不支持。
- 宿主的 app delegate（macOS 的 `NSApplicationDelegate`、`UNUserNotificationCenterDelegate`）
  经 `ShellHandle::submit(ShellEvent::App(..))` 上报事件，不经过任何窗口。

### 运行时侧

```rust
// App::services() -> ServiceHandle：不带窗口 id，全 app 共享
let services: ServiceHandle = context!(app::Services);

services.request_with(cx, ServiceRequest::Notify(n), |cx, reply| ..);   // 请求
services.send(ServiceCommand::RemoveNotifications(None));              // 命令
services.set_badge(Some("3".into()));                                  // 状态，diff
```

`ServiceRequest` 和 `PlatformRequest` 是两个独立的枚举，保证服务不会被发到窗口接口上，反过来也一样。
请求 id、`Cancel` 和回复投递沿用同一套机制。实现上，`Bridge` 里加一条不带窗口的队列即可。

### app 事件

```rust
#[non_exhaustive]
pub enum AppEvent {
    Notification(NotificationResponse),
    /// 文件关联打开、拖到 Dock 图标（`application:openURLs:`）。
    OpenUrls(Vec<String>),
    /// 没有可见窗口时点了 Dock 图标（`applicationShouldHandleReopen`）。
    Reopen,
    StatusItemClicked,
}
```

```rust
fn root(app: &mut App) {
    let last_action = app.signal(None::<String>);
    app.on_app_event(move |cx, event| {
        if let AppEvent::Notification(response) = event {
            last_action.set(cx, response.action.clone());
        }
    });
    app.mount::<Main>(MainProps { last_action: last_action.read_only() });
}
```

- **只有一个 app 级 handler**，在 `AppRoot` 里注册，经 `App::signal` 把结果交给组件树。
  通知被点击时组件可能都还没挂载，所以让组件自己订阅是错误的模型。
- **冷启动时缓冲。** handler 注册之前到达的事件由 `App` 暂存，注册时按顺序补发。
  "点通知启动 app"就是这个场景。

## 通知

### 类型

```rust
pub struct Notification {
    /// 由 app 指定；同 id 再发即替换。
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,        // 只有 macOS 显示
    pub body: Option<String>,
    pub thread: Option<String>,          // 分组
    pub actions: Vec<NotificationAction>,
    pub sound: bool,
    /// 原样带回；必须能跨进程重启存活，所以是字符串。
    pub payload: Option<String>,
}

pub struct NotificationAction {
    pub id: String,
    pub label: String,
    pub kind: ActionKind,                // Button | TextReply { placeholder }
    pub destructive: bool,
}

pub struct NotificationResponse {
    pub id: String,
    pub action: Option<String>,          // None：点击了通知本身
    pub reply_text: Option<String>,
    pub payload: Option<String>,
}

ServiceRequest::NotificationPermission { prompt: bool } → ServiceReply::Permission(Granted | Denied | NotDetermined | Provisional)
ServiceRequest::Notify(Notification)                     → ServiceReply::Done | Failed | Unsupported
ServiceCommand::RemoveNotifications(Option<Vec<String>>) // None：全部移除
AppEvent::Notification(NotificationResponse)
```

- **不做组件**，也**不自动退回 toast**：两者语义不同，宿主不支持时返回 `Unsupported`，由应用决定怎么做。
- **前台展示：** macOS 默认在 app 处于前台时不显示通知，由 `willPresentNotification` 决定。
  宿主选项 `MacRunnerOptions::notifications_in_foreground` 控制这一行为，默认显示。

### 平台前提，最大的风险

- **macOS。** `UNUserNotificationCenter` 要求进程是一个带 bundle identifier 的 `.app`，
  裸跑 `cargo run` 时会直接异常。宿主必须先检查 `NSBundle.mainBundle.bundleIdentifier`，
  有才声明 `NOTIFICATIONS`。开发阶段要能测试，需要 `cargo xui run` 产出一个开发用的 `.app` 包装
  （`Info.plist`，加上指向 target 下二进制的 symlink），这是 `xui-cli` 的一项新工作。
- **Windows。** 需要 AUMID。非打包 app 要有一个带 AUMID 的开始菜单快捷方式，
  点击激活还需要 COM activator，做到冷启动更麻烦。`xui-windows` 从未构建过，放到最后。
- **Linux（winit）。** 通过 zbus 调用 `org.freedesktop.Notifications`，激活事件只在进程运行期间收得到。

## 其他服务（只定形状）

| 服务 | 形状 |
| --- | --- |
| Dock 角标 / 任务栏计数 | `AppOutput.badge`，状态，diff |
| Dock / 任务栏进度 | `AppOutput.progress`。Windows 任务栏的进度按窗口显示，宿主映射到主窗口 |
| 托盘 / 状态栏图标 | `AppOutput.status_item { icon, tooltip, menu }`；菜单项沿用 `CommandId`，点击图标走 `AppEvent::StatusItemClicked` |
| 打开文件/URL、Reopen | `AppEvent` |
| 退出确认 | 现在由宿主的 `should_terminate` 自己决定；改为 `AppEvent::QuitRequested`，由 app 回复是否退出 |
| 全局快捷键、单实例 | 涉及系统权限和进程间通信，暂不设计 |

## 现有命令里哪些其实是服务

`OpenUrl`、`RevealPath`、剪贴板读写都不依赖窗口，按上面的标准属于服务。
`Bridge.menu_bar` 本来就已经是 app 级状态。这些**这一轮不迁移**：它们今天能用，
迁移只会让所有宿主都改一遍。等 `PlatformApp` 落地后，再决定是否挪过去并保留兼容的别名。

## 实施顺序

1. **组件访问宿主**（`window::Platform` context、`request_with`、`PendingRequest`）。
   右键菜单和以后所有系统组件都靠它，与平台无关，用 mock 宿主测试。
2. **右键菜单**：改成请求，加上组件的 `presentation` 属性和 macOS 实现，smoke 示例加一步。
3. **app 级通道**：`PlatformApp`、`ServiceHandle`、`AppEvent`、`App::on_app_event` 及冷启动缓冲，
   先用 `Reopen` / `OpenUrls` 验证，这两项不需要 bundle。
4. **通知**：先做 `cargo xui` 的开发用 `.app` 包装，再做类型、macOS 实现和权限请求。
5. **角标、进度、托盘**，然后是 Windows 和 winit。

第 1、2 步只涉及系统组件，可以独立交付；第 3 步起是服务。

## 待决定

1. **菜单项的标识方式**：下标路径（本稿的选择），还是给 `MenuItem` 加 `tag: u64`？
2. **`MenuItem.command` 改为 `Option` 后**，菜单栏里没有命令的项怎么处理？
3. **组件 `MenuItem` 的新名字**：`ContextMenuItem` 还是 `MenuOption`？
4. **app 事件**只交给 `AppRoot` 里的一个 handler（本稿的选择），还是也开放给组件订阅？
5. **开发用 `.app` 包装**：在 `cargo xui run` 里默认开启，还是需要显式 `--bundle`？
6. **`OpenUrl`、`RevealPath`、剪贴板**以后是否迁到服务通道？
