# UiRuntime API 设计

状态：**已实施**（2026-10-01）。实际落地与提案的差异见 §8。依据 `crates/xui-core/src/ui_runtime/` 与其全部生产调用方（`component.rs`、
`event_system/`、`app.rs`、`runtime.rs`）的现状整理。

## 0. 目标与术语

| 目标 | 手段 |
|---|---|
| 公私分明 | 按**调用方**切分 API 面；外部 crate 看不到 `UiRuntime`；子系统字段对 `ui_runtime` 外不可见 |
| 减少重复 | 生命周期、标脏、树挂接各只有一处实现；删掉无人消费的产物 |
| 减少非必要检查 | "id 是否存活" 只在**边界**检查一次，内部一律按前置条件处理 |
| 内部高效 | 稠密子系统直接索引；不再为每次挂接重建整条 taffy 子列表；去掉 O(n²) 路径 |
| 增加预设 | ① 前置条件契约（`# Preconditions` + `debug_assert!`）② 常用组合的预设常量 |

**存活 id（live id）**：当前 `HostTree` 中存在的 `NodeId`。
**边界**：`NodeId` 可能已失效的入口——排队后才消费的 id、用户回调持有的 id、公开查询。

## 1. id 的两种来源 —— 检查放在哪

| 来源 | 是否可能失效 | 规则 |
|---|---|---|
| `HostTree` 遍历得到（`children` / `parent` / `subtree` / `ancestors`） | 否 | 不检查 |
| reconciler 持有的宿主 id（`create_node` 返回值） | 否（reconciler 负责在删除后不再使用） | `debug_assert!` |
| `EventState` 的 hover / capture、`FocusManager` 的 focus | 否：`remove_subtree` → `InteractionSystem::remove` 已清空 | 不检查（删掉 `dispatcher.rs` 中的 `.filter(contains)`） |
| 帧内脏队列（state/shape/style/subtree/inheritance/canvas） | **是**：删除节点时不清队列 | **排空时过滤一次** |
| `CanvasInvalidator`、`ScrollRequestQueue`、clamped scrolls | **是**：外部句柄异步写入 | 排空时过滤一次 |
| 事件回调发出的 `EventRequest::Focus/CapturePointer(node)` | **是**：用户可持有旧 id | 处理请求时检查一次 |
| 公开查询 `node(id)` 等 | 是 | 返回 `Option`，一次检查 |

据此，**内部函数一律不再做存活检查**，包括 `mark_work`、`recompute_node_style`、
`recompute_subtree_styles`、`clear_work_subtree`、`rebuild_subtree_dirty`、
`sync_effective_taffy_style`、`sync_inheritance_against`、`hit_test_from`、`visual_layout`（内部版）。

排空统一走一个辅助函数，队列里的失效 id 在这里被丢弃，且只丢弃一次：

```rust
impl UiRuntime {
    /// Drops ids removed after they were queued. The one place a queued id is checked.
    fn live(&self, ids: Vec<NodeId>) -> impl Iterator<Item = NodeId> + '_ {
        ids.into_iter().filter(|id| self.hosts.contains_key(*id))
    }
}
```

不采用"删除时清队列"：那要对每个被删节点扫描所有队列（O(删除数 × 队列长度)），
而排空时过滤是 O(队列长度)。

## 2. 子系统契约：稠密 vs 稀疏

现状的根源问题：每个存活宿主**必然**有 style / layout / render binding，但这些子系统的访问器全返回
`Option`，于是 pipeline 生产代码里有约 60 处 `expect` / `invariant!` 在反复证明同一件事。

### 稠密子系统（每个存活宿主恰有一项）

`HostTree` 数据、`StyleSystem`、`LayoutTree`、`RenderSystem` 的 binding。

契约：**以存活 id 为前置条件，直接返回引用**，缺失即 bug（`SecondaryMap` 索引自带 panic）。

```rust
// StyleSystem
fn computed(&self, id: NodeId) -> &ComputedStyle;
fn effective(&self, id: NodeId) -> &ComputedStyle;
fn styles(&self, id: NodeId) -> (&ComputedStyle, &ComputedStyle);
fn is_initialized(&self, id: NodeId) -> bool;

// LayoutTree —— 以宿主 id 为键，taffy id 不再外泄
fn node(&self, id: NodeId) -> &LayoutNode;
fn node_mut(&mut self, id: NodeId) -> &mut LayoutNode;
fn style(&self, id: NodeId) -> &tf::Style;
fn set_style(&mut self, id: NodeId, style: tf::Style) -> bool;   // 返回是否变化
fn invalidate(&mut self, id: NodeId);                            // 原 mark_dirty(taffy)，无 Result

// RenderSystem
fn binding(&self, id: NodeId) -> HostRenderBinding;              // Copy
```

`LayoutTree` 的 `TaffyResult` 只在 taffy trait 实现内部需要；对 pipeline 的接口去掉 `Result`
（`InvalidInputNode` 在存活前提下不可能发生），这样可以删掉约 8 处 `.expect("failed to ...")`。

需要"可能不存在"语义时，只有边界上的 `UiRuntime::node(id) -> Option<NodeView>` 这一处。

### 稀疏子系统（只有部分节点有）

`InteractionSystem`、`AnchorSystem`、`text_nodes` / `canvas_nodes` / `canvases_wanting_repaint`。

契约：`get -> Option` 表示"**有没有这个特性**"，而不是"节点是否存在"。
`InteractionSystem::handlers()` 已是正确示范（空 handler 是正常情况）。

建议把三个 `SparseSecondaryMap<NodeId, ()>` 索引收进一个 `NodeIndex` 结构，
让 `create_node` / `remove_subtree` 各只调用一次 `index.insert(id, kind)` / `index.remove(id)`。

## 3. 预设

### 3.1 工作标记预设

pipeline 中重复出现的组合（生产代码计数）：

```rust
bitflags! {
    pub(crate) struct HostWorkFlags: u8 {
        // …原有位…

        /// 子节点列表变了：父节点重新同步树与布局。（9 处）
        const CHILDREN_CHANGED = Self::SYNC_TREE.bits() | Self::RECALC_LAYOUT.bits();
        /// 换了父节点或脱离树：继承上下文变了。（5 处）
        const REPARENTED = Self::RECALC_STYLE_SUBTREE.bits() | Self::RECALC_LAYOUT.bits();
        /// 几何变了：重新布局并重绘。（5 处）
        const RELAYOUT = Self::RECALC_LAYOUT.bits() | Self::REBUILD_PAINT.bits();
        /// 新建节点的首帧工作。（2 处）
        const MOUNT = Self::RECALC_STYLE.bits() | Self::RELAYOUT.bits();
    }
}
```

### 3.2 前置条件契约

所有内部 `fn` 用统一格式声明前提，并在 debug 构建里断言。release 构建里不产生任何开销：

```rust
/// # Preconditions
/// - `id` is live.
#[inline]
fn mark_work(&mut self, id: NodeId, flags: HostWorkFlags) {
    debug_assert!(self.hosts.contains_key(id), "mark_work: {id:?} is not live");
    let node = &mut self.hosts[id];
    let added = flags & !node.work;
    if added.is_empty() { return; }
    node.work |= flags;
    // …
}
```

`strict-invariants` 仍然用于跨子系统的一致性（例如 host ↔ taffy 父子关系），
**不再**用于"这个 id 还在吗"——那部分改由 `debug_assert!` 负责。

## 4. `UiRuntime` 的 API 面（按调用方）

`UiRuntime` 整体降为 `pub(crate)`，并从 `prelude` 移除。外部 crate 的生产代码中没有任何调用；
`xui-components/virtual_list.rs` 的测试改用 `App` 上的只读查询（见 4.6）。

实现文件按 API 面拆分，每个文件一个 `impl UiRuntime` 块，替代现在 6400 行的 `pipeline.rs`：

```
ui_runtime/
  mod.rs        UiRuntime 结构、NodeView、RenderFrame
  commit.rs     4.1  fiber 提交
  query.rs      4.2  只读查询
  input.rs      4.3  事件写回
  frame.rs      4.4  帧驱动
  config.rs     4.5  宿主配置
  style_pass.rs / layout_pass.rs / paint_pass.rs / scroll.rs   私有流水线（pub(super) 及以下）
  host_tree.rs style.rs layout/ render.rs interaction.rs anchor.rs state.rs   子系统
```

可见性规则：4.1–4.5 为 `pub(crate)`；流水线步骤为 `pub(super)` 或私有；
子系统字段为 `pub(super)`（只有 `ui_runtime` 模块内可见）。

### 4.1 Fiber 提交（`component.rs`）—— 唯一能改树结构的入口

```rust
fn create_node(&mut self, key: Option<Key>, props_hash: u64, widget: WidgetI,
               interaction: Option<HostInteraction>) -> NodeId;
fn update_node(&mut self, id: NodeId, parts: …);                 // 原 update_widget_node_from_parts
fn place(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>);   // 合并 append_child / insert_before
fn remove_subtree(&mut self, id: NodeId);

fn mount_overlay_entry(…) -> Result<OverlayEntryId, OverlayModelError>;
fn update_overlay_entry(…) -> Result<(), OverlayModelError>;
fn unmount_overlay_entry(…) -> Result<NodeId, OverlayModelError>;
fn set_overlay_entry_dismiss(…) -> Result<(), OverlayModelError>;
fn set_anchor(&mut self, node: NodeId, anchor: Option<(Option<NodeId>, AnchorPlacement)>);

fn root(&self) -> NodeId;
fn root_overlayer(&self) -> NodeId;
fn measure_key(&self, id: NodeId) -> tf::NodeId;                // 原 layout_node_id，仅 measure 回调用
```

前置条件：所有 id 存活；`place` 的 `child` 不是 root 或 root_overlayer；`before`（若有）是 `parent` 的子节点。
这些都只用 `debug_assert!`，不再静默返回——静默返回会把 reconciler 的 bug 藏起来。

**`place` 的唯一实现**：

```rust
fn place(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
    // 应用内容永远排在 overlayer 之前 —— root 的唯一特例，集中在这里。
    let before = before.or((parent == self.root).then_some(self.root_overlayer));
    let old_parent = self.hosts.parent(child);
    if let Some(old) = old_parent {
        self.hosts.detach(child);
        self.layout_tree.detach(child);                 // 增量，见 §5.2
        self.mark_work(old, HostWorkFlags::CHILDREN_CHANGED);
    }
    self.hosts.insert(parent, child, before);
    self.layout_tree.insert(parent, child, before);     // 增量
    if old_parent != Some(parent) {
        self.mark_work(child, HostWorkFlags::REPARENTED);
    }
    self.mark_work(parent, HostWorkFlags::CHILDREN_CHANGED);
}
```

**overlayer 子节点同步**：`set_children` 只剩 overlayer 一个调用方，改为私有
`sync_overlayer_children(order: &[NodeId])`，由 4.1 中的三个 overlay 函数调用。
它比较新旧顺序，对差集调用 `place` / `detach`，不再做去重、root 特判或自环过滤。

### 4.2 只读查询（`event_system/`、`runtime.rs`）

```rust
// 边界：可能收到失效 id
fn contains(&self, id: NodeId) -> bool;
fn node(&self, id: NodeId) -> Option<NodeView<'_>>;

// 前置条件：id 存活
fn parent(&self, id: NodeId) -> Option<NodeId>;
fn children(&self, id: NodeId) -> Children<'_>;
fn event_path(&self, target: NodeId) -> EventPath;           // 见 §5.4
fn visual_layout(&self, id: NodeId) -> Bounds;               // 不再返回 Option
fn to_local(&self, id: NodeId, viewport: Point) -> Point;

fn hit_test(&self, point: Point) -> Option<NodeId>;
fn scrollbar_hit(&self, point: Point) -> Option<ScrollbarHit>;
fn dismissable_overlay(&self) -> Option<(NodeId, DismissHandler)>;
fn overlay_contains_pointer(&self, visual_root: NodeId, point: Point) -> bool;

// 焦点 / 指针
fn focused_node(&self) -> Option<NodeId>;
fn hovered_node(&self) -> Option<NodeId>;
fn pointer_capture_node(&self) -> Option<NodeId>;
fn is_focusable(&self, id: NodeId) -> bool;
fn is_sequentially_focusable(&self, id: NodeId) -> bool;
fn tab_index(&self, id: NodeId) -> Option<i32>;
fn resolved_cursor(&self) -> CursorIcon;
fn resolve_local_shortcut(&self, …) -> Option<…>;

// 分发辅助
fn handlers(&self, id: NodeId) -> &EventHandlers;            // 原 node_and_handlers 拆为 node + handlers
fn has_raw_event_listeners(&self) -> bool;
fn node_reads_raw_events(&self, id: NodeId) -> bool;
fn has_drag_callbacks(&self, id: NodeId) -> bool;
fn scroll_metrics(&self, id: NodeId) -> Option<ScrollMetrics>;   // None = 不可滚动，而非"不存在"
fn scrollbar_part(&self, id: NodeId, axis: ScrollbarAxis) -> Option<ScrollbarPart>;
```

### 4.3 事件写回（`event_system/`）

```rust
fn set_widget_state_flag(&mut self, id: NodeId, flag: WidgetState, enabled: bool);
fn request_update(&mut self, id: NodeId, flags: WidgetUpdateFlags);   // 原 pub mark_dirty
fn set_scroll_offset(&mut self, id: NodeId, offset: Point) -> bool;
fn scroll_node_to(&mut self, id: NodeId, target: Point) -> Option<AppliedScroll>;
fn focus_manager_mut(&mut self) -> &mut FocusManager;
fn event_state_mut(&mut self) -> &mut EventState;
```

前置条件：id 存活（它们来自命中测试或 `event_path`）。
`EventRequest::{Focus, CapturePointer}` 中的 id 属于边界，在 `dispatcher.rs` 处理请求时检查一次。

> `focus_manager_mut` / `event_state_mut` 把两个子系统整体交给了事件系统。它们在第一阶段先保留；
> 事件系统和 runtime 的边界应当另起一份设计。

### 4.4 帧驱动（`app.rs`、`runtime.rs`）

```rust
fn begin_frame(&mut self, time: FrameTime);
fn frame_time(&self) -> FrameTime;
fn tick_style_animations(&mut self) -> bool;
fn tick_tickers(&mut self);
fn tick_animating_canvases(&mut self);
fn update_tree<T: TextBackend>(&mut self, viewport: Size<f32>, text: &mut TextHost<T>);
fn build_render_frame(…) -> Result<RenderFrame, RenderFrameError>;
fn finish_render_frame(&mut self);
fn drain_node_lifecycle_events(&mut self) -> Vec<NodeLifecycleEvent>;

fn dispatch_event<T: TextBackend>(…) -> …;

// 滚动请求（外部句柄写入，属边界 —— 排空时过滤）
fn scroll_request_queue(&self) -> ScrollRequestQueue;
fn has_pending_scroll_requests(&self) -> bool;
fn apply_scroll_requests(&mut self) -> Vec<AppliedScroll>;
fn take_clamped_scrolls(&mut self) -> Vec<AppliedScroll>;

// 调度判断
fn is_dirty(&self) -> bool;
fn has_running_style_animations(&self) -> bool;
fn tickers(&self) -> &TickerRegistry;
fn canvas_invalidator(&self) -> CanvasInvalidator;
fn mark_subtree_layout_dirty(&mut self, id: NodeId);
```

计数器 `update_visits` / `inheritance_visits` / `layout_passes` / `repaint_passes`
收进 `pub(crate) stats: FrameStats`，不再是 `pub` 字段。

### 4.5 宿主配置（`app.rs`）

```rust
fn theme(&self) -> &Theme;
fn set_theme(&mut self, theme: Theme);
fn set_scale_factor(&mut self, scale: f32);
fn window_visible(&self) -> bool;
fn set_window_visible(&mut self, visible: bool);
fn set_gpu_context(&mut self, context: CanvasGpuContext);
```

### 4.6 对外（`App` 上的只读视图）

外部 crate 需要检查树时（测试、调试工具）通过 `App`，而不是直接拿 `UiRuntime`：

```rust
impl App {
    pub fn root(&self) -> NodeId;
    pub fn node(&self, id: NodeId) -> Option<NodeView<'_>>;
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_;
    pub fn hit_test(&self, point: Point) -> Option<NodeId>;
}
```

`NodeView` 保持 `pub`。

## 5. 内部效率

### 5.1 `HostTree`

- `children` / `subtree` / `walk` / `ancestors` / `remove` 统一以存活 id 为前置条件，
  去掉 `contains_key(...).then_some(...)` 以及 `remove` 里的检查。
- `insert(parent, child, before: Option<NodeId>)` 合并 `append_child` 和 `insert_before`。
- `assert_attachable` 的环检测会遍历 ancestors（O(深度)），降级为 `debug_assert!`。
- `set_children`、`position`、`iter` 里的 `filter_map`（data 是稠密的）可以删除。
- （可选，需要用 `frame_bench` 测量）`nodes` 和 `data` 两张表合并为一张 `SlotMap<NodeId, Entry { link, data }>`，
  这样每次访问可以少一次代数校验。

### 5.2 Taffy 子节点：从整表重建改为增量

现状：每次挂接都要调用 `sync_taffy_children(parent)`，收集父节点的全部子节点再整体 `set_children`。
依次挂载 n 个子节点就是 O(n²)，`place` 里对旧父节点还要再重建一次。

提案：`LayoutTree` 提供与 `HostTree` 对称的 `insert(parent, child, before)` / `detach(child)`，
每次只改一项。`sync_taffy_children` 只保留给 `sync_overlayer_children`。

> 更彻底的方案是让 `LayoutView` 直接从 `HostTree` 读取拓扑，彻底去掉 taffy 里重复的父子关系。
> 但 taffy 的 `get_child_id(index)` 在链表上是 O(n)，需要先确认各布局算法是否依赖随机访问，暂不纳入本提案。

### 5.3 删除 `Moved` 事件

`NodeLifecycleEvent::Moved` 没有任何消费者（xui-skia 只处理 `Removed`）。
删除它和 `record_node_move` 之后，`place` / `sync_overlayer_children` 就不需要为了它去算 `position()`（每次都是 O(兄弟数)）。

### 5.4 查询路径

- `visual_layout` 每向上走一个祖先都要做 3 次 `Option` 查询；改为稠密索引后变成直接访问，
  也不再需要 `?` 提前返回。
- `event_path` 每次分发都会分配一个 `Vec`。改为返回 `SmallVec<[NodeId; 16]>`（实际深度一般在 16 以内）。
- `hit_test_from` 中的两个 `invariant!` 直接删除。

### 5.5 `remove_subtree`

- 前置条件：存活，且不是 root 或 root_overlayer（`debug_assert!`）。
- 子树只收集一次；每个节点调用一次 `drop_node_data(id)`，统一清理所有子系统和 `NodeIndex`。
- `raw_event_listeners` 的计数直接读 `hosts[id].reads_raw_events`，不需要 `get().is_some_and`。
- 父节点的 taffy 子列表通过 `layout_tree.detach(id)` 更新。

## 6. 删除清单

| 项 | 原因 |
|---|---|
| `render_scene`、`frame_properties`、`frame_properties_mut` | 0 处调用 |
| `to_content_local`、`clear_work`、`compute_layout_if_needed`、`attach` | 0 处调用 |
| `remove_child`、`remove_from_parent`、`clear_children` | 0 处生产调用 |
| `scroll_node_by`、`scroll_single_node_by` | 0 处调用（滚动走 `scroll_node_to`） |
| `accessibility` | 只有测试在用（测试改走 `node(id)` / render scene） |
| `append_child`、`insert_before`、`set_children`（pub） | 由 `place` / `sync_overlayer_children` 取代 |
| `component.rs` 的 `sync_host_children*` | 死代码 |
| `NodeLifecycleEvent::Moved`、`record_node_move` | 无消费者 |
| `node_and_handlers` | 拆为 `node` + `handlers` |
| 降为私有：`compute_layout`、`repaint_if_needed`、`invalidate_canvas`、`is_animating`、`has_animating_canvases`、`listens_for` | 只在 pipeline 内部使用 |

## 7. 实施顺序

每一步都要保持 `cargo test -p xui-core`、`cargo test -p xui-core --features strict-invariants`
和 `cargo test -p xui-components --lib --tests` 通过。

1. **删死代码**（§6 前半部分，加上 `Moved`）——纯删除，风险最低。
2. **加预设**：`HostWorkFlags` 组合常量，替换所有重复出现的组合。
3. **子系统契约**：稠密访问器改为非 `Option`，删掉对应的 `expect` / `invariant!`；`LayoutTree` 改为以宿主 id 为键。
4. **收拢检查**：加 `live()` 排空辅助函数；删掉内部函数的存活检查；dispatcher 去掉多余的过滤。
5. **合并挂接**：`place` + `HostTree::insert` + `LayoutTree` 的增量 insert/detach；实现 `sync_overlayer_children`。
6. **拆文件并收窄可见性**：按 §4 拆分 `pipeline.rs`；`UiRuntime` 降为 `pub(crate)`，从 `prelude` 移除；补上 `App` 的只读视图；迁移 virtual_list 测试。
7. 用 `frame_bench` 对比步骤 3–5 前后的数据（只作参考，不作为通过条件）。

## 8. 实施结果与偏差

与提案不同的地方：

- **对外 API（§4.6）**：没有在 `App` 上加转发方法，因为那会重复一遍 API。`UiRuntime` 仍可从外部命名，但 `pub` 只剩
  `root` / `children` / `parent` / `node` / `contains` / `hit_test` 这几个只读查询，其余都是 `pub(crate)`；
  `ui_runtime` 模块降为 `pub(crate)`；`RenderFrame` 移出 `prelude`；`App::ui_runtime_mut` 改为仅测试可见。
- **`place` 的 `before`**：reconciler 查找下一个兄弟节点时，会越过 Portal 找到挂在 overlayer 下的宿主，
  所以 `before` 不一定是 `parent` 的子节点。这里保留了一次 O(1) 的检查：不是子节点就追加到末尾，并在文档中写明，没有改成 `debug_assert!`。
- **`sync_overlayer_children`**：提交过程中，overlay 模型的更新和宿主的删除分属不同阶段，
  所以它会短暂地列出已经删除的 visual root。这里按边界输入处理，过滤一次（有测试覆盖）。
- **`set_anchor`**：Portal 释放一个已经换掉的 visual root 时，传进来的 id 可能已经失效，所以这里保留边界检查。
- **`LayoutTree`**：改为以宿主 id 为键，taffy 的 `NodeId` 直接取宿主 id 的 FFI 位，去掉了 taffy 内部的 slotmap 和一层映射。
  几何数据（`LayoutNode`）和 taffy 的节点状态放在两张独立的 `SecondaryMap` 里。
  最初把两者合成一个条目，结果 hover 基准回退了 9%：命中测试逐节点遍历时要跨过 taffy 很大的 style 和 cache。拆开后反而比基线快 29%。
- **额外删除**：`FiberArena` 里有一个只含一个节点的 `TaffyTree`，外加 `HostState::taffy_node`。
  它们唯一的用途，是拿 runtime 布局树的 id 去这个无关的树上 `remove`，属于死代码，而且 id 空间对不上。
- **未做**：`NodeIndex` 合并（`drop_node` 已把清理集中到一处，收益不大），以及 `HostTree` 两张表合并（§5.1 的可选项）。

提案之外顺带修掉的效率问题：

- `dispatch_user_handlers` 以前先构建 `NodeView`（要走一遍祖先链）再判断有没有 handler，现在先判断。
- `to_local` 从递归、每层构建两个 `NodeView` 的 O(深度²)，改为一次迭代遍历。
- `AnchorSystem::offset` 在没有任何锚点时直接返回，不再做哈希查找。
- `recompute_node_style` 不再克隆父节点的 `ComputedStyle`。
- `HostTree` 的遍历迭代器不再经过 `&dyn` 虚调用。
- `mark_subtree_layout_dirty` 不再为每一层递归分配一个 `Vec`。

### 8.1 transform 统一计算（B′）

- **原来的问题。** 节点的 paint transform 由三样输入决定：effective 样式里的 transform、`anchor_offset`、布局尺寸（transform origin 是按尺寸的比例算的）。以前有 6 个地方各自当场调用 `sync_effective_transform`：`tick`、样式 pass 的两个分支、`set_anchor`、锚点放置、渲染 pass。结果同一帧里可能重复计算，初始化分支还会拿布局之前的 0×0 尺寸算一次。
- **现在的做法。** 新增 `HostWorkFlags::SYNC_TRANSFORM`，为此把 `HostWorkFlags` 从 u8 扩到 u16。
  - 样式差异里的 `TRANSFORM`（`from_style_diff`）会打上这个标记。
  - 锚点加入、退出或偏移变化时打上它。
  - `sync_layout` 发现尺寸变化时打上它。
  - `MOUNT` 预设包含它。
  - 渲染同步遍历（`sync_render_dirty_subtree`）对带这个标记的节点只计算一次 transform。这一步和 scene 同步（`SCENE`）互相独立，所以只改了 transform 的节点不会触发 scene 重建。
- **时机变化。** `tick_style_animations` 之后，`FrameProperties` 里的 transform 要等下一次 `update_tree` 才更新。在这段时间里没有代码读它：命中测试和 `to_local` 读的是 `anchor_offset`。
- **验证。**
  - `FrameStats::transform_syncs` 计数。新增测试 `a_transform_is_derived_once_a_frame_from_the_settled_size`：同一帧里 transform 和尺寸一起变化时，只计算一次，而且用的是布局之后的尺寸。
  - 同一时段 A/B 交替基准（每边 4 轮）：所有用例在 ±3% 以内，性能持平；这项改动的收益在于结构。

## 9. 基准对比（`frame_bench`，release，三次取最优）

| 用例（6403 个宿主） | 之前 | 之后 |
|---|---|---|
| hover（`interaction_frame_cost`） | 0.0267 ms | 0.0190 ms（−29%） |
| 动画帧 `update_tree` | 0.0457 ms | 0.0304 ms（−33%） |
| 继承过渡 `update_tree` | 0.0470 ms | 0.0315 ms（−33%） |
| 结构变更 total | 3.89 ms | 3.98 ms（+2%，噪声范围） |
| 组件重建（`component_rebuild_cost`） | 4.92 ms | 4.94 ms（持平） |

小规模用例的变化都在 ±5% 以内。
