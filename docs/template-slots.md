# 静态 Template 与动态 Slot 设计

状态：**阶段 0、1、2、3 已实施**（2026-10-19）。阶段 3 没有按 §6 的见证方案做，实际设计见 §12；其余为草案；实施记录见 §11。依据 `crates/xui-macros/src/{element,view}.rs`、
`crates/xui-core/src/{element,dsl,widgets,component,fiber}.rs` 与
`crates/xui-core/src/ui_runtime/commit.rs` 的现状整理；数字来自 `frame_bench`（release）。

## 0. 目标与结论

目标：利用 `xui!` / `view!` 在编译期看到的 UI 结构，把"每次重渲染都重建一遍、再整体 diff"
变成"静态部分只建一次，动态部分按固定位置直接 patch"，同时保留 Fiber/Lanes 负责组件生命周期与调度。

结论：

1. **方向成立，且瓶颈正在这里。** 未变化的子树重渲染，比 memo 跳过慢约 46 倍（§1）；
   而 commit 之后的样式、布局、绘制已经是增量的，几乎不花时间。所以收益集中在
   **render → reconcile → commit** 这一段，不在 Layout/Paint。
2. **第一步不是 Template，而是拆开 `WidgetI` 的"描述"与"实例"。** 现在两者是同一个
   `Rc<RefCell<Widgets>>`，任何可共享的静态描述都会和活节点互相别名（§2.2）。
3. **分阶段做，每阶段单独测量、单独可回滚：**

| 阶段 | 内容 | 依赖 | 风险 | 预期收益 |
|---|---|---|---|---|
| 0 | 低垂果实：修 handler 丢失 bug，删 fiber 里只写不读的拷贝，补基准 | — | 低 | 每次 commit 少一份 `ComputedStyle` 拷贝/每 host |
| 1 | 描述/实例分离（`WidgetProto`），handler 改为替换、不再 diff | 0 | 中 | 解锁 2–4；`use_memo` 缓存的 host 元素可走指针相等跳过 |
| 2 | 静态子树提升（hoisting），`ElementDesc::Hoisted` 整树跳过 | 1 | 低–中 | 静态子树逼近 memo 跳过的开销 |
| 3 | 类型化 Slot 与 Patch：动态属性按位置比较、直接写实例 | 1 | 中 | 去掉整份 `Style` 构造、哈希、比较 |
| 4 | Block Template：一个 fiber 持有一组 host（条件性） | 2、3 | 高 | 去掉静态节点的 fiber/WorkNode；只在 2、3 之后测量仍不够时做 |
| S | Signal 直接绑定：属性订阅信号，绕过组件 render | 1 | 中 | 热点数值（计数、进度、动画值）不再重渲染组件 |

## 1. 现状测量

`cargo test -p xui-core --release --lib frame_bench -- --ignored --nocapture --test-threads=1`

`memoized_subtree_bailout`：根组件状态变化，整棵行列表重渲染（rebuilt）对比用 `use_memo`
持有子元素、走指针相等跳过（memoized）。每行 4 个 host。

| 行数 | host 数 | rebuilt (ms) | memoized (ms) | 倍数 |
|---:|---:|---:|---:|---:|
| 25 | 103 | 0.163 | 0.0048 | 34× |
| 100 | 403 | 0.643 | 0.0155 | 41× |
| 400 | 1603 | 2.935 | 0.0638 | 46× |
| 1600 | 6403 | 4.844 | 0.2555 | —（按预算切片，3 帧完成，单帧不可比） |

即**每个未变化的 host 每次重渲染约 1.8µs**。下游对照：

| 基准 | 1600 行 |
|---|---|
| `idle_frame_is_free` | 0.0001 ms |
| `interaction_frame_cost`（hover） | 0.018 ms |
| `inherited_transition_frame_cost` | 0.030 ms |
| `structural_frame_cost`（增删一个子树） | 0.589 ms |

`WidgetUpdateFlags` → `HostWorkFlags`、`StyleDiffFlags` → `HostWorkFlags` 已经把"哪个属性变了 →
哪一段要重做"做成了位标记（`ui_runtime/state.rs` 的 `from_widget_update` / `from_style_diff`），
render-compile 也是增量的。所以提案里"属性失效标记减少无关 Layout/Paint"这一半**已经有了**；
本设计的收益来自 commit 之前。

### 1.1 一个未变 host 每次重渲染做了什么

| 步骤 | 位置 | 成本 |
|---|---|---|
| 调用 `tag()` + 每个属性的 builder 方法，构造 `Style`（`StylePatch` 六组 + `rules: Vec` + `RefCell<Vec>` 缓存） | `dsl.rs` `StyleProps` | 构造 + 分配 |
| `WidgetI::new` → `Rc<RefCell<Widgets>>`；`children: Vec<ElementDesc>` | `widgets.rs` `widget_element_desc` | 2 次分配 |
| `props_hash`：哈希整份 `Style` + `InteractionProperties` | `component.rs` `prepare_element` | 与样式大小成正比 |
| `take_host_interaction`：从 widget 里拿走 handlers | 同上 | 改写描述 |
| 每个父节点：`old_children` Vec、`used` Vec、key 表 `FxHashMap`（即使没有 key） | `reconcile_children` | 3 次分配/父节点 |
| 每个子节点一个 `WorkNode` | `alloc_wip_node` | 推入 WIP 竞技场的 `Vec`（摊销，不单独分配） |
| 若 hash 变：`update_from` 再整体比较一次 `Style`，然后 clone | `widgets/*.rs` | 比较 + 拷贝 |
| `freeze_host_state`：克隆 `target_style`（整份 `ComputedStyle`）、taffy `Style`、`WidgetI` 到 fiber | `component.rs` | **每个 host 每次 commit**，见 §3.2 |

## 2. 与现有设计的三个约束

### 2.1 宏是"故意无知"的

`element.rs` 的模块文档写明：宏不认识任何 tag 和属性名，`<tag attr={v}>` 只是
`tag().attr(v)`，合法性全部交给类型系统。所以宏**不知道**：

- `<button>` 是 host 还是组件（都只是一个返回 builder 的函数）；
- `.gap(8.0)` 是样式、布局、交互属性，还是 handler；
- 哪些属性影响布局、哪些只影响绘制。

Dioxus、Solid、Vue 能在编译期切出 Template，是因为它们的宏认识 HTML 词汇。这里要么破坏这条原则，
要么**把词汇知识留在类型系统里，宏只做语法判断**。本设计选后者：

- 阶段 2 只用语法判断"这个表达式是不是字面量/常量"；
- 阶段 3 通过方法解析拿到每个属性的类型见证（witness），宏仍然只生成 `.xxx(...)` 形式的调用。

这样"新增样式属性、组件、host 控件都不用改 `xui-macros`"的性质保持不变。

### 2.2 `WidgetI` 既是描述又是实例

- `WidgetI` 是 `Rc<RefCell<Widgets>>`（`widgets.rs`）；
- `create_node` 把收到的 `WidgetI` **原样**存成活节点的 widget（`commit.rs` → `HostData::new`）；
- `TextInputWidget` 把实例状态放在同一个结构里：`scroll_x`、`drag`、`ime_session`、`focused`、`caret_on`……
  并在 `handle_event` 里修改；
- `prepare_element` 调 `take_host_interaction`，会改写描述本身。

因此，一份被缓存、被多个位置共享的"静态描述"会：

1. 在同一个 callsite 渲染出多个实例时（列表），**多个活节点共享同一个 widget**；
2. 再次 reconcile 时，`update_from(&self, next)` 对同一个 `RefCell` 先 `borrow_mut` 再 `borrow`，**直接 panic**；
3. 第二次 prepare 拿不到 handlers（第一次已经 take 走了）。

今天 `use_memo` 缓存 host 元素时也会踩到其中一部分，只是 `props_hash` 恰好相等时不走 `update_from`，被掩盖了。
所以描述/实例分离（阶段 1）是任何 Template 方案的前提。

### 2.3 一个 fiber 对应一个 host 的假设遍布 commit

`find_host_sibling_for_wip`、`first_host_in_wip_subtree`、`commit_deletion`、
`commit_placement_subtree`、Portal 锚定（"Portal 写在其下的最近 host"）、`freeze_work_tree`
都假设 host fiber 与 `NodeId` 一一对应。完整的 Block Template（一个 fiber 管多个 host）要改所有这些路径，
是全部方案里成本最大的一块，所以放在阶段 4，并设进入条件。

## 3. 阶段 0：低垂果实

### 3.1 修复：只改了闭包捕获值时，新 handler 丢失（已验证）

`props_hash` 只哈希样式与 `InteractionProperties`，不包含 `EventHandlers`；`prepared_needs_update`
只看 `props_hash`；`interaction_system.update` 只在 `UPDATE` 时调用。于是：

```rust
let captured = *n.get();
container().style(fixed).on_click(move |_, _| { SEEN.push(captured); ... })
```

`n` 从 0 变 5 后点击，记录到的是 `[0, 0]`，期望 `[0, 5]`。让 `background` 也依赖 `captured`，
结果就变成 `[0, 5]`，说明重渲染确实发生了，丢的只是 handler。

修法：复用 host fiber 时，handlers 总是交给交互系统替换，且**不产生任何 `WidgetUpdateFlags`**
（替换 handler 不需要重做样式、布局、绘制）。这也是阶段 3 里 "handler slot 只替换、不比较" 的雏形。
已另起会话处理（"Fix stale event handlers on re-render"），回归测试放在 `ui_runtime/tests.rs`。

### 3.2 删掉 fiber `HostState` 里只写不读的拷贝

`fiber::HostState` 有 `widget`、`style`、`computed_style`、`layout`、`previous_layout`，
`freeze_host_state` 每次 commit 为 WIP 树里的**每个 host** 重新填写，其中 `computed_style` 是
`arena_node.target_style.clone()`。全仓库只读 `node_id` 和 `props_hash`
（`component.rs` 的 `from_current`、`from_prepared`、`prepared_needs_update`、`commit_deletion`）。

动作：`HostState` 只留 `node_id`、`props_hash`（阶段 1 后换成 `proto`）。预计降低 rebuilt 基准里的 commit 段。

### 3.3 `reconcile_children` 的分配

- 没有任何 key 时不建 `old_key_map`（大多数子列表没有 key）；
- `used` 用 `SmallVec<[bool; 16]>` 或位集；
- `old_children` 复用 `WorkInProgress` 上的一个 scratch Vec。

### 3.4 补基准

在 `frame_bench.rs` 增加 `template_row_rebuild`：每行一个静态容器 + 一个静态标签 + 一个依赖状态的文本，
根状态变化导致整列表重渲染。它是阶段 2、3 的主要衡量对象：

- 下界 = `memoized_subtree_bailout` 的 memoized；
- 上界 = 今天的 rebuilt；
- 每个阶段报告它落在两者之间的位置。

## 4. 阶段 1：描述 / 实例分离

### 4.1 类型

```rust
// widgets.rs
/// What the DSL built: immutable, shareable across renders and instances.
#[derive(Clone, Debug)]
pub struct WidgetProto(Rc<Widgets>);

/// A live host's widget: owned by exactly one `HostData`, mutated by events.
#[derive(Clone, Debug)]
pub struct WidgetI { widget: Rc<RefCell<Widgets>> }   // 不变，但只在 ui_runtime 内创建

impl WidgetProto {
    pub(crate) fn instantiate(&self) -> WidgetI;          // 挂载时 clone 一份成为实例
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool;
    pub(crate) fn interaction(&self) -> Option<HostInteraction>; // clone，不再 take
}

// element.rs
pub struct WidgetDesc {
    pub widget: WidgetProto,          // 原来是 WidgetI
    pub children: Vec<ElementDesc>,
}
```

- 目前没有一个控件结构派生 `Clone`，而且 `TextInputWidget` 的实例状态（`drag`、`ime_session`……）
  本来就不该被复制。所以 `instantiate` 不用 `Clone`，而是每个控件提供
  `fn instantiate(&self) -> Self`：拷贝 DSL 写入的部分，实例状态取初始值。它和现有的
  `update_from(&mut self, next: &Self)` 是一对，由 `define_widgets!` 分派到 `Widgets` 上。
- `EventHandlers` 也没有 `Clone`，但里面的 `Handler<E>` 是 `Rc<RefCell<..>>`，给 `Entry`、
  `EventHandlers` 派生 `Clone` 只是引用计数加一。
- 长期看，应该把各控件拆成 `Props`（DSL 写的）与 `State`（实例自己的），`instantiate` / `update_from`
  都只碰 `Props`。阶段 1 先不拆，只加 `instantiate`，改动面最小（见 §10.2）。

### 4.2 行为变化

| 位置 | 现在 | 之后 |
|---|---|---|
| `prepare_element` | `take_host_interaction()` 改写描述 | `proto.interaction()` 克隆 |
| `create_node` | 存入收到的 `WidgetI` | 存入 `proto.instantiate()` |
| `update_node` | `node.widget.update_from(&widget)` | `node.widget.update_from(&proto)`（借用不同的对象，不可能再 panic） |
| `prepared_needs_update` | 只比较 `props_hash` | 先 `proto.ptr_eq(committed)` → 相等则无 UPDATE、不算 hash；否则照旧比较 hash |
| handlers | 只在 UPDATE 时提交（§3.1 的 bug） | 每次复用都替换，零 flag；`ptr_eq` 时可跳过 |
| `fiber::HostState` | 存实例 `WidgetI` | 存上次提交的 `WidgetProto`（用于 `ptr_eq`） |

### 4.3 要改的文件

`widgets.rs`（`WidgetProto`、`widget_element_desc`、`WidgetI` 的公开面）、各 `widgets/*.rs`（`instantiate`）、
`element.rs`、`component.rs`（prepare / reconcile / commit / freeze）、`fiber.rs`（`HostState`）、
`ui_runtime/commit.rs`（`create_node` / `update_node` 签名）、`event_system/callbacks.rs`（`Clone`）、
测试里直接构造 `WidgetI` 的辅助函数（`ui_runtime/tests.rs` 的 `create_host` / `update_host`、`frame_bench.rs`）。

### 4.4 验收

- §3.1 的回归测试通过；
- 新增：同一个缓存的 host 元素同时挂在两个位置，修改其中一个实例的状态（如 TextInput 输入）不影响另一个；
- 新增：`use_memo` 缓存的 host 元素重渲染时不进入 `update_from`；
- `cargo test -p xui --features strict-invariants` 通过；
- `frame_bench` 全部断言通过，rebuilt 不回退。

## 5. 阶段 2：静态子树提升

### 5.1 判定（纯语法，在 `xui-macros`）

一个元素是**静态的**，当且仅当：

- 每个属性值都是静态表达式；
- body 为空，或 `Content(静态表达式)`，或 `Children` 且每个孩子都是静态元素；
- 子列表里没有 `if` / `for` / `match` / `let` / `..splice`；
- tag 路径没有泛型参数（`tag::<T>()` 不提升）。

**静态表达式**（`syn::Expr`），第一版保守：

| 允许 | 例 |
|---|---|
| 字面量、`-字面量` | `12.0`、`"Inbox"`、`-1` |
| 括号、元组、数组、`&`、`as` 包裹的静态表达式 | `(8.0, 4.0)` |
| 至少两段、末段大写开头的路径（常量、枚举变体） | `Color::BLACK`、`AlignStyle::Center` |
| 单段全大写路径（模块常量） | `GAP` |
| 不允许：首段是 `Self` 或单个大写字母的路径 | `T::DEFAULT`（可能是外层泛型，`thread_local` 里会报 E0401） |
| 不允许：闭包、块、方法调用、函数调用、宏、单段小写路径 | `move \|_\| ..`、`count`、`Color::hex("#fff")` |

是否把**关联构造函数调用**（`Color::hex("#fff")`、`EdgeInsets::all(16.0)`、`Style::new().width(16.0)`）
也算静态，是开放问题（§10.1）：它们在实际代码里很常见，但宏无法证明其纯度。
处理办法：先用保守规则落地，再统计 `xui-components` 与 example app 里两种规则各能覆盖多少元素，用数据决定。

handler 永远不会被提升：它的值是闭包或局部变量，按上表不是静态表达式，宏不需要知道 `on_` 前缀。

### 5.2 展开

```rust
view! {
    column(gap: 12.0) {
        text("Inbox").font_size(20.0)
        text(count.to_string())
    }
}
```

`column` 有动态孩子，不提升；第一个 `text` 是静态的：

```rust
__xui_children.push({
    ::std::thread_local! {
        static __XUI_HOISTED: ::std::rc::Rc<#xui_core::ElementDesc> = ::std::rc::Rc::new(
            #xui_core::dsl::ElementBody::build(
                #xui_core::dsl::Content("Inbox"),
                text().font_size(20.0),
            )
        );
    }
    #xui_core::ElementDesc::Hoisted(__XUI_HOISTED.with(::std::rc::Rc::clone))
});
```

- 用 `thread_local!` 是因为 `ElementDesc` 含 `Rc`；UI 跑在单线程上，每个线程一份。
- 只提升**极大**静态子树（静态元素的静态孩子不再单独包一层）。
- 根元素整体静态时也提升，组件函数整体返回同一个 `Rc`。
- span：初始化表达式沿用原属性的 span，保证 rustc 的诊断仍然指向原 token；`tests/ui/*.stderr` 需要重新生成并逐个核对。

### 5.3 运行时

```rust
// element.rs
pub enum ElementDesc {
    Host(WidgetDesc),
    Component(ComponentDesc),
    Portal(PortalDesc),
    /// A subtree the macro proved static: built once per call site.
    Hoisted(Rc<ElementDesc>),
}

// fiber.rs — Node 增加
pub hoisted: Option<Rc<ElementDesc>>,   // 持有 Rc，防止地址被复用造成误判
```

reconcile：

1. `prepare_element(Hoisted(rc))`：key、tag 取自 `rc` 内部；`PreparedElement` 记下 `hoisted: Some(rc)`。
2. 复用旧 fiber 且 `Rc::ptr_eq(old.hoisted, rc)` → **无 effect、无 work**，lanes 照常从 scheduler 取。
   这样 `needs_begin_work` 为假，整棵子树既不 begin、也不 clone children；`freeze_work_tree` 走
   `collect_current_child_ids`。成本 O(1)，与 memo 跳过同一条路径。
3. 其他情况（首次挂载、或换成另一个 `Rc`，如 `if` 两个分支各有一个静态子树）：把 `rc` 内部
   浅拆成普通的 Host / Component / Portal 照常处理，孩子从 `rc` 里 clone。

组件 tag 的静态提升：`button(label: "OK")` 被提升后，每次拿到的是同一个 props `Rc`，
命中 `should_render_component` 已有的指针相等跳过。组件仍会因为自己的 state、context 重渲染；
只读外部可变全局（thread_local 等）的组件会变"粘"——与 `React.memo` 同样的语义，需要写进 `docs/components.md`。

### 5.4 验收

- 新增宏测试：静态判定表的每一行（含反例）；
- 新增运行时测试：提升的子树在父组件重渲染时不进入 begin_work；`if` 切换两个提升分支时正确替换；
  一个 callsite 在列表里挂载 N 次，N 个实例互不影响（依赖阶段 1）；
- `template_row_rebuild`：静态部分的开销接近 memoized。

## 6. 阶段 3：类型化 Slot 与 Patch

阶段 2 只照顾"整棵都静态"的子树。现实里常见的是"结构和大部分属性静态，一两个属性动态"，
例如 `row(gap: 8.0, background: color) { .. }`。阶段 3 处理这种元素。

### 6.1 属性见证（不让宏认识属性名）

`dsl.rs` 的 `style_props!` 表是样式词汇的唯一来源。在表里给每个属性加一列"组"，并为每个属性生成：

```rust
pub mod attr {
    /// Zero-sized witness for one attribute.
    pub struct Background;
}

pub trait Attr<W> {
    /// The value after `Into` conversion, so a slot stores one concrete type.
    type Value: 'static;
    /// What changing this attribute invalidates on the widget.
    const DIRTY: WidgetUpdateFlags;
    fn convert<V: AttrInput<Self>>(v: V) -> Self::Value;
    fn apply(widget: W, value: Self::Value) -> W;              // 挂载：等同 builder 方法
    fn patch(widget: &mut W, value: &Self::Value) -> WidgetUpdateFlags; // 更新：只写一个字段
    fn same(a: &Self::Value, b: &Self::Value) -> bool;         // 样式属性用 PartialEq
}

pub trait StyleProps: Styled {
    // 现有的 builder 方法不变，另外每个属性一个隐藏的见证方法：
    #[doc(hidden)]
    fn __xui_attr_background(&self) -> attr::Background { attr::Background }
}
```

宏对动态属性 `background: color` 生成的是 `__xui_base.__xui_attr_background()`，
由**方法解析**决定用哪个见证：控件的固有方法优先于 trait 方法，所以 `IconWidget::color`（表示着色）
只要同时提供固有的 `__xui_attr_color`，就会覆盖 `StyleProps` 的版本，规则和今天的 builder 方法完全一致。
控件自己的非样式属性（`text` 的内容、`image` 的来源、`flex_direction`……）用一个
`#[derive(Attrs)]` 或现有的 `define_widgets!` 生成同样的见证。

`DIRTY` 的取值：

| 属性 | `DIRTY` |
|---|---|
| 任何 `Style` 属性 | `STYLE_TARGET`（下游 `StyleDiffFlags` 已能细分布局/绘制/变换，不在这里重复） |
| 文本内容、段落属性 | `TEXT_SHAPE \| LAYOUT_INPUT \| PAINT_OUTPUT` |
| `flex_direction`、`flex_wrap` | `LAYOUT_INPUT` |
| 只影响绘制的控件属性（图片、图标数据、canvas 控制器） | `PAINT_OUTPUT` |
| handler | 空：只替换，不比较 |
| key | 不是 slot：key 变化走结构路径 |

### 6.2 Slot 元组与模板元素

宏把一个元素的属性分成两组：静态属性在 callsite 的 `thread_local` 里构造一次，得到基础 `WidgetProto`；
动态属性按书写顺序编号，组成一个类型化的 slot 元组：

```rust
// 宏为 row(gap: 8.0, background: color, on_click: h) 生成（示意）：
let __xui_base = __XUI_BASE.with(Clone::clone);           // gap 已经应用
let __xui_slots = (
    #xui_core::dsl::slot(__xui_base.__xui_attr_background(), color),
    #xui_core::dsl::slot(__xui_base.__xui_attr_on_click(), h),
);
#xui_core::ElementDesc::Templated(#xui_core::TemplatedDesc::new(__xui_base, __xui_slots, children))
```

```rust
pub struct Slot<A: Attr<W>, W> { value: A::Value, .. }

/// Implemented for tuples of `Slot`s (by macro_rules, up to 16).
pub trait SlotSet: 'static {
    fn apply(&self, base: &WidgetProto) -> WidgetI;                 // 挂载
    fn patch(&self, prev: &dyn Any, inst: &mut Widgets) -> WidgetUpdateFlags; // 更新
}
```

### 6.3 reconcile / commit

复用旧 fiber，且旧 fiber 的基础 `WidgetProto` 与本次 `ptr_eq`（同一个 callsite）：

- **不算 `props_hash`、不整体比较 `Style`、不 clone 基础描述**；
- `slots.patch(prev_slots, instance)`：逐个 slot 调 `A::same`，不同就 `A::patch` 并累加 `A::DIRTY`；
- 累加结果为空且孩子没有工作 → 不产生 UPDATE；
- fiber 保存本次 slot 元组（`Box<dyn SlotSet>`），作为下一次的 `prev`。

基础描述不同（换了 callsite）→ 回到普通路径。

阶段 3 保留"一个 host 一个 fiber"，所以 §2.3 的那些路径都不用动。

### 6.4 验收

- 只有一个动态属性的元素，重渲染时不调用 `props_hash`、不调用 `update_from`（用计数器断言）；
- 属性覆盖：`IconWidget` 的 `color` 动态时走着色而不是文字颜色；
- `template_row_rebuild` 的动态行开销显著低于 rebuilt；
- trybuild：错写属性名的诊断仍然指向属性，并且仍然是 rustc 的 "no method named … did you mean"
  （见证方法是 `#[doc(hidden)]` 的 `__xui_attr_*`，需要确认不会出现在建议里，必要时保留原 builder 调用做诊断）。

## 7. 阶段 4：Block Template（条件性）

**进入条件**：阶段 2、3 完成后，`template_row_rebuild` 中 `WorkNode` 分配与 fiber 遍历仍然占重渲染时间的一半以上。
否则不做。

结构：`view!` / `xui!` 在展开处为一棵元素树生成一个真正的 `static Template`（`#[component]`
只生成 props、builder 和 tag，不参与）。动态区域（`if` / `for` / `match` / `..splice`、组件、Portal）
成为"锚点"，其内容仍然是普通 fiber 子列表。

节点的种类和静态属性**不用枚举表示**，而是表示成宏生成的函数指针：函数体就是今天的
`tag().attr(v)...` 调用，第一次用到时在本线程运行一次，得到 `WidgetProto`。slot 的种类也不用枚举，
而是记在 §6 的类型化 `SlotSet` 元组里。

```rust
pub struct Template {
    pub nodes: &'static [TemplateNode],      // 先序
    pub slots: &'static [u16],               // 第 i 个 slot 所在的节点；种类在 SlotSet 的类型里
}

pub struct TemplateNode {
    pub parent: Option<u16>,
    pub kind: TemplateNodeKind,
}

pub enum TemplateNodeKind {
    /// 静态部分：`tag()` + 静态属性 + 静态内容。函数指针可以放进 `static`。
    Host(fn() -> WidgetProto),
    /// 组件、Portal、控制流、splice：内容是普通的 fiber 子列表。
    Anchor,
}
```

例如：

```rust
view! {
    column(gap: 12.0) {
        text("Inbox").font_size(20.0)
        text(count.to_string())
        button(text: "OK", on_click: on_ok)      // xui-components 里的 #[component]
    }
}
```

展开为（示意）：

```rust
fn __n0() -> WidgetProto { WidgetProto::of(column().gap(12.0)) }
fn __n1() -> WidgetProto { WidgetProto::of_content(text().font_size(20.0), "Inbox") }
fn __n2() -> WidgetProto { WidgetProto::of(text()) }

static __XUI_TEMPLATE: Template = Template {
    nodes: &[
        TemplateNode { parent: None,    kind: TemplateNodeKind::Host(__n0) },
        TemplateNode { parent: Some(0), kind: TemplateNodeKind::Host(__n1) },
        TemplateNode { parent: Some(0), kind: TemplateNodeKind::Host(__n2) },
        TemplateNode { parent: Some(0), kind: TemplateNodeKind::Anchor },
    ],
    slots: &[2],
};

// 每次 render 只产生这些：
TemplateDesc::new(
    &__XUI_TEMPLATE,
    (content_slot(__n2_witness, count.to_string()),),      // SlotSet
    [button().text("OK").on_click(on_ok).into_element(NoChildren)], // 锚点内容
)
```

一个附带的好处：静态部分在独立的 `fn` 里，它**不可能**捕获局部变量或外层泛型参数。
宏的语法判定（§5.1）万一把引用了局部变量的表达式当成静态，结果是编译错误（E0425 / E0401），
而不是一个悄悄不更新的 UI。

### 7.1 为什么不用封闭枚举

另一种写法是 `NodeKind::{Column, Row, Text, Button}`、`StaticAttr::{Gap(f32), FontSize(f32)}`、
`SlotKind::{TextContent, OnClick}`，整个模板都是 const 数据。在这个代码库里行不通：

| 问题 | 原因 |
|---|---|
| 宏得认识全部词汇 | `column` 只是返回 `ContainerWidget` 的函数，`.gap` 是 `StyleProps` 的方法；每加一个控件或样式属性都要同时改 `xui-macros`，打破 §2.1 |
| `Button` 不是 host | `xui-components` 的 `button` 是 `#[component]`；宏从语法上分不出 host 和组件，`NodeKind` 无法正确分类 |
| 第三方控件、固有方法覆盖 | 应用自己定义的控件进不了枚举；`IconWidget::color`（着色）和 `StyleProps::color`（文字颜色）同名不同义，枚举只能二选一 |
| 值无法 const 构造 | `gap` 接受 `impl Into<LengthValue>`（可以是 token，不只是 `f32`）；`Style` 含 `Vec` 和 `RefCell`；`Color::hex(..)` 不是 `const fn` |
| 诊断变差 | 现在错写属性名得到 rustc 的 "no method named … did you mean"；枚举方案要宏自己报错 |
| 性能并不更好 | 按 `SlotKind` `match` 分派仍是运行时分支；§6 的 `SlotSet` 元组是单态化的，patch 是静态分派 |

函数指针方案的运行时布局与枚举方案相同（节点数组 + 父下标 + slot 下标），差别只是静态部分
"每个线程第一次用到时构造一次"，而不是编译期常量——这是一次性的成本。

// 一个模板实例只占一个 fiber：
pub struct TemplateState {
    pub template: &'static Template,
    pub hosts: SmallVec<[NodeId; 8]>,        // 下标 → NodeId
    pub slots: Box<dyn SlotSet>,
}
```

需要改动、并要逐个补测试的路径：`find_host_sibling_for_wip` / `first_host_in_wip_subtree`
（一个 fiber 现在可能贡献多个 host，并且锚点的孩子挂在模板内部的某个 host 下）、`commit_deletion`
（删除整组）、`commit_placement_subtree`、Portal 锚定的"最近 host"、`freeze_work_tree`。焦点、无障碍、
命中测试都在 host 侧，不受影响。

由于 `Template` 是 `&'static`，阶段 1 的"描述 / 实例分离"在这里是硬性前提：模板只能持有描述。

## 8. 独立线：Signal 直接绑定

现在 `State::set` → `enqueue_hook_update` → 调度整个所属组件重渲染。对"一个数字在变、UI 结构不变"的热点
（计数、进度条、实时数据），即使有阶段 2、3，也要跑一遍组件函数。

设计：

```rust
/// A value that host attributes can bind to directly.
pub struct Signal<T: 'static> { .. }      // hook：cx.use_signal(|| 0.0)

// DSL：属性值是 Signal 时，宏不需要知道——由 `AttrInput` 的一个实现决定
row(width: progress)   // progress: Signal<f32>
```

- commit 时，绑定属性把 `(NodeId, patch fn)` 注册到信号的订阅表；
- `Signal::set` 在 Event / Effect 阶段写值（符合 `slot` 的阶段守卫），把订阅者放进一个绑定更新队列；
- 帧开始、`update_tree` 之前排空队列：按 CLAUDE.md 的约定**在排空处用 `live()` 过滤一次**失效 id，
  对每个存活节点调用该属性的 `Attr::patch` 并 `request_update(id, A::DIRTY)`；
- 组件函数里**读取**信号值（而不是把信号交给属性）时，把组件登记为订阅者，退化成普通的重渲染，保证正确性。

这与 `CanvasInvalidator`、`ScrollRequestQueue` 已有的"外部句柄写入、排空时过滤"模式一致。依赖阶段 1
（patch 只能写实例）和阶段 3 的 `Attr`（patch 函数与 `DIRTY`）。

## 9. 静态样式共享与绘制命令缓存

- **静态样式共享**：阶段 2、3 之后，静态 `Style` 留在 callsite 的 `WidgetProto` 里，被所有实例共享，
  构造开销归零。`ComputedStyle` 的驻留（interning）收益取决于样式解析阶段的开销，而 1600 行 hover 只要
  0.018ms，暂不立项；等阶段 3 后用 `frame_bench` 再看。
- **绘制命令缓存**：render-compile 已经增量（见 `docs/render-compile/`），未变化的节点不重建场景。
  不在本设计范围内。

## 10. 开放问题

1. **关联构造函数是否视为静态**（§5.1）。需要先统计覆盖率。可选的折中：保守规则 + 显式标注
   （如 `#[static]` 写在元素前），由用户为 `Color::hex(..)` 一类表达式担保纯度。
2. **阶段 1 是否顺手把控件拆成 `Props` / `State`。** 拆了之后 `instantiate` 只构造默认状态、`update_from`
   只拷 props，语义更清楚，但改动面大（每个控件）。建议放在阶段 1 之后、阶段 3 之前单独做。
3. **`xui!`（JSX 语法）是否同样启用。** 两种语法共享 `Element::expand`，判定与展开天然一致；
   `xui!` 没有控制流孩子，提升得更彻底。
4. **调试体验。** 提升后的元素在 `thread_local` 初始化里构造，panic 回溯会多一层。
   可以给 `ElementDesc::Hoisted` 加 `#[track_caller]` 式的 callsite 记录，供 `diagnostics.rs` 输出。
5. **热重载/样式主题切换。** 静态值里不含主题解析结果（主题在样式阶段解析），所以切换主题不需要失效
   提升的描述；需要用一个测试固定这一点。

## 11. 实施记录

### 11.1 阶段 0、1（2026-10-19）

`template_row_rebuild`（每次状态变化到树稳定的总时间，中位数，ms）：

| 行数 | 模式 | 改动前 | 阶段 0 后 | 阶段 1 后 |
|---:|---|---:|---:|---:|
| 400 | unchanged | 1.762 | 1.469 | 1.518 |
| 400 | one_dynamic | 2.830 | 2.638 | 2.765 |
| 1600 | unchanged | 9.604 | 7.761 | 7.588 |
| 1600 | one_dynamic | 15.999 | 14.529 | 14.238 |

阶段 0 带来约 15–20% 的下降；阶段 1 是为后续阶段铺路，不以提速为目标，前后差异在测量噪声内。

与 §3、§4 原计划的出入：

- **§3.1**：没有把 handler 并入 hash，而是在没有 `UPDATE` 的复用路径上调用
  `UiRuntime::replace_handlers`（`commit_handlers_if_host`），不标记任何工作。本次新建的 host
  （`is_uncommited`）已经随 widget 拿到 handler，跳过。
- **§3.2**：`HostState` 只剩 `node_id`、`props_hash`，以及阶段 1 加入的 `proto`。原来的
  `freeze_host_state` 还会为每个 host 构造一次 `NodeView`（其中包含 `visual_layout` 的计算），
  现在改用 `host_props_hash`，而且只在两边都没有 hash 时才读取。
- **§3.3**：`old_children`、`used` 改为内联的 `SmallVec`。真正每次都分配的是
  `SmallVec::with_capacity(20)`：容量超过内联的 8，每个父节点都会溢出到堆上；现在改用 `SmallVec::new()`。
  key 表没动：没有 key 时 `collect` 出来的空 `FxHashMap` 本来就不分配。
- **§4.1**：`instantiate` 按控件逐个手写（各控件都没有 `Clone`）。`TextInputWidget` 用内部 controller 时，
  会按当前文本新建一个；`CanvasWidget` 共享 controller 句柄；`RootOverlayerWidget` 返回一个新的 overlayer
  （实际上不会走到）。
- **§4.2**：`WidgetProto::take_host_interaction` 在描述没有被其他人持有时（`Rc::get_mut` 成功）
  直接移走 handler 和交互属性；被 memo 等持有时才克隆。这样常见路径没有新增的克隆。
  `UiRuntime::update_node` 保留给测试使用，reconciler 调用 `update_node_from(&Widgets)`。
- 新增回归测试（`ui_runtime/tests.rs` 的 `rerendered_handlers` 模块）：只有 handler 变化时，新 handler 能够生效、
  移除的 handler 不再触发；memo 缓存的 host 在父组件重渲染后 handler 仍然有效（改动前第一次 commit
  会把共享描述里的 handler 拿走）；同一份描述挂载两次，得到两个独立的 controller。

### 11.2 阶段 2（2026-10-19）

实现位置：判定与展开在 `crates/xui-macros/src/hoist.rs`（`Element::expand_value` 调用）；运行时为
`ElementDesc::Hoisted(Rc<Hoisted>)`（`element.rs`）、`dsl::hoisted`，以及 `component.rs` 的
`prepare_hoisted_element` / `is_committed_hoisted`。

`template_row_rebuild`（中位数，ms）。`view_*` 两行用 `view!` 和常量写同一张卡片（少了 padding，
因为 `EdgeInsets` 没有 const 构造）：`view_unchanged` 整行是静态的，整行提升；`view_one_dynamic`
只有标签被提升，卡片和取值文本每次都重建。

| 行数 | unchanged | one_dynamic | view_unchanged | view_one_dynamic |
|---:|---:|---:|---:|---:|
| 25 | 0.281 | 0.183 | 0.025 | 0.295 |
| 100 | 0.673 | 0.818 | 0.045 | 0.608 |
| 400 | 1.577 | 2.809 | 0.147 | 2.429 |
| 1600 | 7.944 | 14.389 | 0.586 | 13.413 |

整棵静态的子树比重建快约 13 倍，接近 memo 跳过；但只要行里有一个动态值，收益只剩下被提升的那部分
（这里约 7%）。后者正是阶段 3 要处理的情形。

**覆盖率**（§10.1 的统计，保守规则；"放宽"指同时允许 `Type::f(静态参数)` 和静态接收者上的方法调用）：

| crate | 宏根元素 | 元素 | 保守规则下被提升 | 放宽后 |
|---|---:|---:|---:|---:|
| `xui-components` | 111 | 143 | 9（6%） | 9（6%） |
| `xui-example-app` | 147 | 207 | 24（11%） | 31（14%） |

组件库里每次宏调用平均只有约 1.3 个元素，属性几乎都来自 props 或局部变量，所以放宽规则几乎不增加覆盖。
结论：保持保守规则，不为 `Color::hex(..)` 这类调用担保纯度。

与 §5 原计划的出入：

- 静态判定：单段路径只要首字母大写且多于一个字符就算常量（`GAP`、`None`）；单个大写字母、以 `Self`
  或单个大写字母开头的路径一律不算。`as` 只允许转换到原始类型。`..expr` 拼接、`if`/`for`/`match`/`let`
  孩子都使整个元素不静态。
- `Hoisted` 在构造时就算好 host 的 `props_hash`，复用路径不再算哈希。孩子留在共享分配里，
  只有挂载或换成另一个提升子树时才 clone（`HostWork::hoisted`、`take_work_nodes`）。
- 复用旧 fiber 且 `Rc::ptr_eq` 时，直接用 `WorkNode::from_current` 搬过当前子树，只保留 `MOVE`，
  与组件 bailout 走同一条路径；子树里有排期工作时照常下降。`fiber::HostState::hoisted` 持有 `Rc`。
- 提升的组件、Portal 按普通元素准备；组件拿到的是同一个 props 分配，已有的指针相等 bailout 生效。
  语义（与 `React.memo` 相同）写进了 `docs/components.md` 的 Performance behavior。
- 新增 `ElementDesc::resolved()`，供检查元素形状的代码（测试）看穿 `Hoisted`。
- 测试：`xui-macros` 的判定表（含反例）；`component.rs` 的 `hoist_tests`（一次分配、重渲染只用 2 个
  work node、一个调用点挂载两次得到两个 host、两个提升分支之间切换、提升子树内部组件自己的更新仍会执行）；
  `tests/dsl.rs` 的提升与不提升各一例。trybuild 的 `.stderr` 没有变化。

### 11.3 阶段 3 的设计缺口（未实施）

§6 的见证方法 `__xui_attr_*` 由方法解析选出，这要求**每个 tag 的每个属性**都有见证，否则展开后的代码
编译不过。宏又分不出 tag 是 host 还是组件（§2.1），所以无法只对 host 生成见证调用：

1. `#[component]` / `component_fn!` 生成的 builder、手写的 builder（`PortalDesc`、`ContextProvider`、
   `xui-components` 里不经 `#[component]` 的那些）都没有见证。哪怕 slot 路径对它们没有意义，
   只要宏为它们发出 `__xui_attr_label()` 这样的调用，就会编译失败。
2. Rust 的两个分支都要过类型检查，所以"运行时再走回退路径"做不到；必须在类型层面有一个总能成立的回退。
3. 不经见证、直接比较动态值也行不通：属性值常常是借用（`text(&name)`），转换成 `'static` 的 slot 值需要
   知道属性的目标类型，而这正是见证提供的信息；用 autoref 按 `T: 'static` 做特化也不可行
   （trait 选择不看生命周期，结果是借用检查错误，不会回退）。

可选方向：

- **A. 名字哈希见证 + autoref 回退。** 宏把属性名哈希成 `u64`，生成 `<W as HasAttr<ID>>` 查询，并用 autoref
  特化在 `W` 没有该见证时回退到普通路径。这样总能编译，但见证的 patch 语义必须手工与 builder 方法保持一致
  （固有方法覆盖 trait 方法的规则也要在 impl 里重复一遍），新增属性时漏写见证只会悄悄失去优化。
- **B. 调用点显式开启。** 例如 `view!(template) { .. }`，由作者保证其中的 tag 都是带见证的 host；
  编译错误由开启者承担。改动面小，但优化是选择性的。
- **C. 跳过阶段 3，直接做 §8 的 Signal 绑定。** 对"一个值在变"的热点，绕过组件重渲染比 slot patch 更彻底，
  而且不需要宏认识属性。

### 11.4 阶段 3（2026-10-19）

按 §12 实施。`template_row_rebuild`（5 次运行取每行最小值，ms；`view_*` 两行用 `view!` 写同一张卡片，
少了 padding）：

| 行数 | unchanged | one_dynamic | view_unchanged | view_one_dynamic | 阶段 2 时的 view_one_dynamic |
|---:|---:|---:|---:|---:|---:|
| 25 | 0.093 | 0.153 | 0.010 | 0.085 | 0.295 |
| 100 | 0.366 | 0.583 | 0.038 | 0.314 | 0.608 |
| 400 | 1.513 | 2.628 | 0.146 | 1.278 | 2.429 |
| 1600 | 7.544 | 13.733 | 0.587 | 7.581 | 13.413 |

每行都改一个文本的模板化版本，比同样写法的 builder 版本快约 45%，和"什么都不变"的 builder 版本持平；剩下的
主要是 1600 个文本重新排版的下游工作。

实施中发现并修掉的一个回退：`Style` 有 2480 字节，最初把样式片段内联在 `TemplateSlots` 里，使 `ElementDesc`、
`WorkNode`、fiber `Node` 全部膨胀到 2.7–3.2KB，连普通 builder 路径也变慢了一倍左右。现在片段和 handler 都装箱，
`TemplatedDesc` 是 144 字节，与 `ComponentDesc` 相同，`ElementDesc` 仍是 152 字节。

## 12. 阶段 3 的实际设计

§6 的见证方案卡在宏分不出 host 和组件（§11.3）。原型还确认了另外两点：把动态值先放进元组再决定走哪条路，
不写参数类型的 handler 闭包（`move |_, _| ..`）就推断不出 `for<'a>` 签名（`implementation of FnMut is not
general enough`）；用辅助闭包重放前缀链能恢复其他推断，但恢复不了这个，因为 rustc 只从被调用函数自己声明的
约束推断高阶闭包签名。Dioxus 的 `rsx!` 用命名规则区分元素和组件，并把事件闭包直接传给每事件一个的类型化函数
（`dioxus_elements::events::onclick::call_with_explicit_closure`），这里照此重新设计。

### 12.1 命名约定

组件仍用 snake_case 定义，`#[component]` 生成的 tag 函数改为 PascalCase：`fn date_picker` 写作 `DatePicker(..)`。
host tag 保持小写。于是宏只看语法就知道小写 tag 是 host。别名同样改为 PascalCase（`Select`、`ComboBox`、
`Sheet`）。

### 12.2 词汇表：`xui-schema`

一个只有数据的 crate，`xui-macros` 和 `xui-core`（测试）都依赖它：

- `HOST_TAGS`：会被模板化的 tag、它们构造的控件和 body 含义。目前是 `container`、`row`、`column`、`center`、
  `z_stack`、`grid`、`text`。`canvas`、`text_input` 的构造函数自带每实例状态，`image`、`icon` 有与样式属性
  同名而含义不同的固有方法，都按原样构造；第三方小写 tag 也是。
- `STYLE_PROPS`：`style_props!` 表里所有"只赋值字段"的样式属性。`scrollbar` 不在其中，它整体赋值
  `ScrollbarStylePatch`（未设置的字段也一起赋），与合并不等价。
- `ORDERED_STYLE_ATTRS`：`style`、`scrollbar`，它们整体改写样式，位置有意义。

`dsl::schema_tests` 核对 `STYLE_PROPS` 与表项、`HOST_TAGS` 与构造函数实际返回的控件。

### 12.3 展开

对一个小写、在 `HOST_TAGS` 里、且不能整体提升的元素，宏逐个属性按书写顺序分类：

| 属性 | 去向 |
|---|---|
| `key` | 静态 → 基础描述；动态 → key slot |
| `on_*` | `dsl::handlers::on_*(值)`：每事件一个函数，约束与 `EventProps` 方法相同，闭包从这里得到高阶签名 |
| `STYLE_PROPS` 中的属性 | 第一个动态样式属性之前的静态属性 → 基础描述；从它开始的样式属性（不论静态动态）按顺序经 `dsl::fragment::名字(style, 值)` 链到一个空 `Style` 上 |
| `style` / `scrollbar` | 只能是静态，且必须在所有动态样式属性之前，否则整个元素按原样构造 |
| 其他属性 | 静态 → 基础描述；动态 → 整个元素按原样构造 |
| `text` 的内容 | 静态 → 基础描述（`.text(..)`）；动态 → 内容 slot |

`dsl::fragment` 由 `style_props!` 从同一张表生成，参数与 builder 方法相同，值的类型推断与今天一样；样式 setter 只
赋值字段，所以"空 `Style` 上按顺序设置、再合并到基础样式上"等于"在控件上按顺序调用"。求值顺序是先 body、再按
书写顺序的属性，与原来的 builder 链一致。

基础描述放在调用点的 `thread_local` 里，用 `dsl::host::<控件类型>(tag()…)` 构造：小写 tag 若解析到别的东西，
在这里是类型错误，而不是被 slot 误读。

### 12.4 运行时

`ElementDesc::Templated(TemplatedDesc)`：基础描述的 `Rc`、key、`TemplateSlots { style: Option<Box<Style>>,
content: Option<TextContent> }`、handler、children。

- 复用的旧 fiber 来自同一个调用点（基础描述 `Rc::ptr_eq`）：只比较 slot；有变化才 UPDATE，提交时由
  `UiRuntime::patch_node` 把变化的 slot 写进活实例（样式 = 基础样式合并片段，内容直接赋值），报告
  `STYLE_TARGET` 或 `LAYOUT_INPUT | PAINT_OUTPUT`。handler 每次渲染照常替换。
- 其他情况（新挂载、换了调用点）：`TemplateBase::build` 展开成完整描述，走原来的创建/更新路径。
- fiber 记下调用点和 slot（`HostState::template`）。打过补丁的 host 不再保留描述（`proto = None`），
  `props_hash` 置为 `STALE_PROPS_HASH`，之后同一位置换成普通元素时一定会更新。
- `TemplatedDesc::to_widget_desc()` / `ElementDesc::to_host()` 给出 builder 链会得到的普通描述，测试用它检查等价性。

### 12.5 测试

- `xui-macros` 的 `template::tests`：哪些写法模板化、哪些按原样构造；`style` / `scrollbar` 的顺序规则。
- `tests/dsl.rs`：模板化元素与对应 builder 链的 `props_hash`、key、children 相同（静态在前、重叠属性的先后、
  动态 key、交互属性加 handler）；动态的非词汇属性按原样构造。
- `ui_runtime/tests.rs` 的 `templated_hosts`：变化的 slot 写进同一个 host；不写参数类型的 handler 每次渲染都生效；
  打过补丁的 host 被与首次构造值相同的普通元素覆盖时仍会更新（去掉 `STALE_PROPS_HASH` 时这个测试失败）。

