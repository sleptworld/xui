# 草案：arena signal

状态：草案，暂缓。目前保留 `Rc` 方案：微基准显示读写开销同一量级（见「成本」），不需要的部分已经清理掉：
`Tasks` 归 `Runtime` 直接持有、删除 `ResourceState`、context 改用 `Box` 并沿 owner 链查找、`Expr` 改用 `Box`。
下文「现状」指清理之前的代码。若日后采纳，结论并入 `docs/reactive.md`（英文）。

## 动机

`reactive/` 里的 `Rc`/`RefCell` 几乎都出自同一个原因：**事件 handler 是存放在
widget 里的 `'static` 闭包，由 `UiRuntime` 的事件分发调用，而这时拿不到
`reactive::Runtime`**（两者是 `App` 的并列字段）。于是：

| 现状 | 为什么需要 |
|---|---|
| `Signal/Memo/ReadSignal(Rc<SourceCell<T>>)`，值在 `RefCell<T>` | handler 只能通过 `&self` 写，handle 要被闭包捕获 |
| `Notifier(Rc<RefCell<Vec<SourceId>>>, Rc<RefCell<Tasks>>)` | handler 无法访问 graph，只能往共享队列里记写入 |
| `ResourceState(Rc<Cell<u64>>)` | 实例字段和 future 的回调闭包共享"代数" |
| `Context = (TypeId, Rc<dyn Any>)` | `ReadSignal<T>` 不是 `Copy`，只能类型擦除后共享 |
| ticker 回调、`SizeObserver::reactive`、`DismissHandler` 捕获 `Rc<instance>` + `Notifier` | 都是在 `UiRuntime` 内部、`Runtime` 之外被调用 |

代价：

- "持有 `read()` 时 `set`"会在运行期 panic，而不是编译不过。
- 每个 source 都要一份 `Rc` 头和 `RefCell` 标志。
- 每个 handler 都要克隆 `Rc<Scope>` 和 `Notifier`。

## 目标与非目标

目标：

- `reactive/` 里不再有 `RefCell`/`Cell`。
- handle 变成 `Copy`。
- 读写冲突交给借用检查器。

不变：

- 编译期依赖分析、`Template`/`EffectSpec`、lane 顺序、flush 语义。
- `Rc<Scope>`：只读共享，不含 `RefCell`，没有问题。
- `Callback`、`Children`：靠 `Rc::ptr_eq` 判断同一性，并且会被多次挂载。

## 核心：`Store`

值从 handle 里搬到 `Runtime` 持有的 arena：

```rust
/// Every source's value, and what was written since the last flush.
pub struct Store {
    values: SlotMap<SourceId, Box<dyn Any>>,
    written: Vec<SourceId>,
    /// `resource!` futures waiting for the runtime to start them.
    spawns: Vec<Spawn>,
}

pub struct Runtime {
    store: Store,
    graph: Graph, // SecondaryMap<SourceId, SourceRecord>: subscribers, version, level
    instances: SlotMap<InstanceId, Mounted>,
    tasks: Tasks, // owned directly, no RefCell
    ..
}
```

`Store` 和 `Graph` 分开有两个原因：

1. handler 只拿到 `&mut Store`，graph 和 instances 仍然只对 `Runtime` 可见。
2. 分开的字段可以同时借用，`Runtime` 内部能同时持有 `&mut store` 和 `&mut graph`。

由 `Store` 分配 `SourceId`，`Graph` 用 `SecondaryMap` 跟随。

## Handle

```rust
pub struct Signal<T>     { id: SourceId, _t: PhantomData<fn() -> T> } // Copy, Send, Sync
pub struct Memo<T>       { .. }                                        // same
pub struct ReadSignal<T> { .. }                                        // same

impl<T: 'static> ReadSignal<T> {
    pub fn read<'s>(self, from: &'s impl Reads) -> &'s T;
    pub fn get(self, from: &impl Reads) -> T where T: Clone;
}
impl<T: 'static> Signal<T> {
    pub fn set(self, cx: &mut Cx<'_>, value: T);
    pub fn update(self, cx: &mut Cx<'_>, f: impl FnOnce(&mut T));
}

/// `Store`, `Cx`, `RenderCx`: anything a value can be read from.
pub trait Reads { fn store(&self) -> &Store; }

pub struct Cx<'a> { store: &'a mut Store }
```

- 读：一次 slotmap 下标访问、一次代数比较、一次 `TypeId` 比较（`downcast_ref`）。现在是解引用 `Rc` 再检查 `RefCell` 标志，开销同一量级。
- 写：原地修改值，再 `written.push(id)`。handler 内部依然"写后可读"。
- 已卸载的 handle：读会 panic，提示 "read after its scope unmounted"，另提供 `try_read -> Option<&T>`；写是 no-op，和现在 graph 的行为一致。generational key 保证不会出现 ABA。
- `PhantomData<fn() -> T>` 让 handle 无论 `T` 是什么都满足 `Send + Sync + Copy`。值本身仍然只在主线程。

借用检查现在能拦住旧的运行期 panic：

```rust
let v = count.read(cx);
count.set(cx, 1); // error[E0502]: `*cx` is also borrowed as immutable
```

## 各 lane 的读写

| 代码 | 现在 | arena |
|---|---|---|
| tracked prelude（render、block、resource） | `let g = this.x.read(); let x = &*g;` | `let x: &T = this.x.read(__store);` |
| memo | `Self::__memo_x(&*this.a.read(), ..)` 后 `__replace(cx, v)` | `let v = Self::__memo_x(this.a.read(store), ..);` 借用结束后 `store.replace_if_changed(this.x, v)` |
| handler、`effect!`、`ticker!`（Handles 模式） | `let x = &this.x; x.get()` | `let x = this.x;`（复制），`x.get(cx)` |

- **Render lane** 用单独的 `RenderCx { store: &Store, ui: &mut UiRuntime, hosts, instance }`。它对 store 只读，可以和 `&mut UiRuntime` 同时存在。"render binding 不能写 source"从约定变成类型约束。
- **Memo lane**：`__memo_x` 已经是"参数借用、返回所有权"的形状，读借用在调用返回时结束，之后再写。生成代码几乎不用改。
- **Block lane**：`BlockCx` 持有 `&mut Runtime`（其中包括 store）。现在宏生成的是 `__block.each(scope, <用户的迭代器表达式>, ..)`。如果迭代器借用了 store（例如 `items.iter().cloned()`），就和 `&mut BlockCx` 冲突。`each`/`each_indexed` 因此先把 `(key, item)` 收集到 `Vec` 再做协调，每次运行 `for` 块多一次 `Vec` 分配。`arm` 的 `data` 本来就是有所有权的值，在调用前算好即可。

## 从 `Runtime` 外部写入

| 入口 | 现在 | arena |
|---|---|---|
| 事件 handler | 闭包捕获 `Rc<Scope>` 和 `Notifier` | `App::dispatch_event` 把 `&mut self.reactive.store` 传给 `UiRuntime::dispatch_event`，再一路传到 `dispatch_user_handlers`，按节点重新借用后放进 `EventContext { store: Option<&'a mut Store> }` |
| overlay dismiss | `DismissHandler(Rc<dyn Fn(DismissReason)>)` 捕获 `Notifier` | `Rc<dyn Fn(&mut Cx, DismissReason)>`，`dismiss_overlay_if` 从同一次分发拿 store |
| `size!()` | layout 中途调用 `SizeObserver` 的 `notify` 闭包 | observer 持有 `Signal<Option<Size<f32>>>`（`Copy`），layout 把 `(signal, size)` 推进 `UiRuntime` 的队列，`App` 在 flush 前统一写入 store |
| `ticker!` | `TickerRegistry` 持有捕获 `Rc<instance>` + `Notifier` 的 `Box<dyn FnMut>` | 改由 `Runtime::tick(ui, frame)` 驱动，运行各实例的 tick slot；registry 只记录哪些 ticker 在运行（给 `is_running` 和唤醒时间用）。原来在 `frame.rs` 里调用的 `self.tickers.tick(frame)` 移到 `App` |
| `resource!` | `cx.__notifier().tasks().borrow_mut().start(..)` | `cx.__resource(value, future)` 推一个 `Spawn { target, future, apply }` 到 `store.spawns`，flush 时由 `Runtime` 交给自己持有的 `Tasks` |
| `App::update`、`Runtime::batch` | `Cx::handler(&notifier)` | `Cx::new(&mut self.store)` |

handler 的生成代码。`ecx` 和 `cx` 必须是互不相交的借用，因为函数体会同时用到两者：

```rust
draft.__with_on(table.on_click(), move |__arg, __ecx| {
    let __cx = &mut Cx::new(__ecx.take_store()); // leaves None in ecx
    let count = count;                            // copied handle, captured by value
    /* body: count.update(__cx, ..), __ecx.request_focus(), .. */
})
```

`App::dispatch_event` 的签名不变，现有约 50 处调用不受影响。不涉及 reactive 的 widget handler 不会调用 `take_store`。

### `resource!` 的代数

`Tasks` 维护 `latest: HashMap<SourceId, u64>`，以目标 signal 为键：

- 启动时记录新的 ticket；
- `finish` 时只有 ticket 仍是最新才写入。

`ResourceState` 和实例上的隐藏字段可以一起删掉。

## Context

`ReadSignal<T>` 变成 `Copy` 之后，context 不再需要类型擦除的共享指针：

```rust
pub(crate) type Context = (TypeId, SourceId);      // was (TypeId, Rc<dyn Any>)
contexts: HashMap<TypeId, SourceId>,                // was (Rc<dyn Any>, Rc<dyn Any>)
```

`context::<T>()` 先比对 `TypeId`，再用 `ReadSignal::<T>::from_id(id)` 构造 handle。读和写共用同一个 id。

另一个独立的改进：`context::<T>()` 被调用时沿 owner 链查找，不再在每次 mount 时由 `contexts_at` 复制整条链。

## 其他调整

- `Expr.compute`：`Rc<dyn Fn() -> T>` 改为 `Box<dyn Fn(&Store) -> T>`。
- `Derived.run`：改为 `Box<dyn Fn(&mut Store)>`。
- `BuildHost<I>`：`fn(&Rc<I>, &Notifier) -> HostDraft` 改为 `fn(&Rc<I>, &Store) -> HostDraft`。
- `NodeKind::Portal`：不再需要 `&Notifier`。
- `Notifier` 整体删除。
- 测试里的 `runtime.instance::<C>(id).unwrap().count.get()` 改为 `.count.get(runtime.store())`。
- `ReadSignal::constant`（在任何 runtime 之外凭空造一个 source）在工作区里没有调用者，删除。

## 迁移后剩下的

| 保留 | 原因 |
|---|---|
| `Rc<Scope>`、`Row.parent: Rc<P>` | 只读共享，没有内部可变性。handler 只用到 handle 时，宏可以直接复制 handle，不再克隆 `Rc`（可选优化） |
| `Rc<RowParts>`、`BlockState.parts: Rc<dyn Any>` | 同一个块的各行共享不可变模板 |
| `Callback`、`Children` | 同一性语义，多处共享 |
| `Tasks` 的 `Arc<Mutex<..>>` wake | 跨线程 |

`reactive/` 里的 `RefCell`/`Cell` 降为 0。

## 语义与 API 变化

- `x.read()`、`x.get()`、`x.peek()` 都要传入 `cx` 或 store。tracked 代码里名字本来就绑定成值，不受影响；受影响的只有 handler、`effect!`、`ticker!` 和测试。
- `impl Display for Signal` 删除（`Debug` 只打印 id）。handler 里写 `format!("{count}")` 要改成 `count.read(cx)`。
- 读已卸载的 handle 会 panic。原来因为 `Rc` 保活，能读到最后一个值。
- handle 变成 `Send + Sync + Copy`。
- 写之后仍然要等到 flush 才通知订阅者，这一点不变。

## 成本

微基准（`i64` 值，release，M 系列 Mac，单位 ns/op，取 5 次中的最好值）：

| n、访问顺序 | 读 `Rc<RefCell>` | 读 arena `downcast_ref` | 读 arena 内联 `TypeId` | 写 + 记录 `Rc<RefCell>` | 写 + 记录 arena |
|---|---|---|---|---|---|
| 1k 顺序 | 1.0 | 1.9 | 1.0 | 2.1 | 1.9 |
| 100k 顺序 | 0.9 | 1.9 | 1.5 | 2.4 | 2.4 |
| 100k 随机 | 1.6 | 3.7 | 3.1 | 3.2 | 3.7 |
| 1M 随机 | 8.6 | 30.2 | 20.2 | 23.2 | 28.1 |

- 读比现在慢 1–3.5 倍，原因有两个：
  - 要多走一层间接：slot → `Box` → 值，而现在是 `Rc` → 值。缓存未命中时这层间接就是多一次 miss。
  - `downcast_ref` 是一次虚调用。
- 把 `TypeId` 存在 slot 里、读时比对后直接转指针，可以消掉虚调用。
- 写和现在持平。
- 和一次 effect 的其余开销相比，比如 `patch_node`、调度、之后的 layout 和 paint，读多出的这 1–2 ns 可以忽略。
- 同一个实例的 source 是连续插入的，在 arena 里槽位相邻。基准里的随机顺序没有体现这一点。

其他：

- 内存：每个 source 少一份 `Rc` 头（16 B）和 `RefCell` 标志；每个值仍然要一次 `Box` 分配，和现在的 `Rc` 分配一一对应。以后可以按类型做 slab 池。
- `for` 块每次运行多一次 `Vec` 分配。
- `xui-components` 和 `xui-example-app` 还没有迁到 `#[component]`（目前 0 处使用），API 变更只影响 xui-core 的测试和已移植的 shell/skia/macos 示例。现在改的成本最低。

## 迁移步骤

每一步都能单独编译，`cargo test -p xui-core` 保持通过。

1. **一次性完成：Store、`Copy` handle 和全部写入入口**。handler 离开 notifier 就没法写，所以这一步拆不开：
   - `Store`、新的 handle、`Cx`/`RenderCx`、删除 `Notifier`；
   - 改宏的 prelude、memo、handler 代码生成，以及 `for` 块先收集；
   - `EventContext` 加上 store，`DismissHandler`、size 队列、`Runtime::tick`；
   - 更新测试。
2. ~~`Tasks` 收归 `Runtime`，删除 `ResourceState`。~~ 已在 `Rc` 方案下完成。
3. context 改成 `(TypeId, SourceId)`。（按 owner 链查找已完成。）
4. 可选：handler 只用到 handle 时不再捕获 `Rc<Scope>`。

## 考虑过但放弃的方案

- **thread-local store**（Leptos/Sycamore 的做法）：handle 能变成 `Copy`，但 `RefCell` 只是挪进了 thread-local，借用错误仍在运行期；同一线程上有多个 `App`（例如测试）时还要做作用域隔离。
- **延迟执行的 handler**（handler 只入队，由 `Runtime` 之后再跑）：handler 必须同步返回 `Flow`，并且要用到 `EventContext`。
- **把 store 放进 `UiRuntime`**：render lane 要同时持有 `&Store` 和 `&mut UiRuntime`，到处都得拆分借用 `UiRuntime` 的内部字段。改为由 `Runtime` 持有 store，事件分发时借出，边界更清楚。

## 待定

1. `EventContext` 持有 `&mut reactive::Store` 会让 `event_system` 依赖 `reactive`。两者都在 xui-core 里，直接用具体类型还是擦除成 `&mut dyn Any`？
2. 读 API 的形式：`count.get(cx)`、`cx.get(count)` 还是 `cx[count]`？
3. 读已卸载的 handle：panic 还是返回 `Option`？
