#import "operations-template.typ": *
#show: manual

#v(24mm)
#text(size: 11pt, fill: accent, weight: "bold")[XUI · 开发与维护手册]
#v(10mm)
#text(size: 38pt, weight: "bold")[Render Compile]
#v(3mm)
#text(size: 26pt, weight: "bold")[增量编译操作手册]
#v(9mm)
#text(size: 14pt, fill: muted)[从保留场景，到可复用绘制结构，再到每帧合成]
#v(14mm)
#note[本手册的实现基线][
  2026-10-12，仓库 HEAD：`e972c54`，并包含当前工作区未提交的 render 改动。重点是 `rebuild_dirty_paths`、`NodeItemLayout`、移除日志与增量 dirty 摘要。后续源码变动时，应重新核对本文条件和示例。
]
#v(10mm)
面向渲染管线维护者、Widget 与动画开发者、后端实现者。手册提供决策规则、操作步骤、代码示例、数据结构解释、正确性约束与验证入口。

#v(1fr)
#line(length: 100%, stroke: 1pt + accent)
#text(size: 9pt, fill: muted)[本地 Typst 操作手册模板 · 可编辑源码 · 无在线模板依赖]

#pagebreak()
#outline(title: [阅读导航], indent: 1em, depth: 1)
#v(12pt)
#note[按任务阅读][
  初次接触：第 1 至 3 章。维护编译器：第 4 至 8 章。接入动画与后端：第 9 至 11 章。实际操作与排障：第 12 至 14 章。源码索引与更新文档：第 15 章。
]

#chapter[
= 架构目标与边界
== 编译器究竟编译什么
`SceneCompiler` 把后端无关的 `RenderScene` 编译成 `CompiledScene`。它确定绘制顺序、图元所在空间、裁剪链、Layer 是否形成独立 Picture，并为隔离 Layer 准备静态 render program。这里的 compile 发生在运行时，不等于 Rust 编译，也不等于 shader 编译。

新的核心思想是：编译结果与 Scene 一样长期保留。每次更新先判断变化是否会影响结构；普通属性变化原地更新实体，结构变化沿 dirty 路径组织绘制条目，干净子树复用旧结果。

== 两种 compile，三个生命周期
#table(columns: (1.1fr, 1.35fr, 1.45fr),
  [阶段], [输入与输出], [生命周期],
  [`SceneCompiler`], [`RenderScene` → `CompiledScene`], [跨帧保留，按变化更新],
  [`compile_layer`], [静态 Layer 样式 → `LayerProgram`], [样式相同则复用静态 IR],
  [`FrameBuilder::build`], [编译场景 + viewport + properties → `BuiltFrame`], [本帧世界坐标、可见性与合成实例],
  [`LayerProgram::instantiate`], [帧几何 → 可执行 pass 与纹理槽], [由图程序结合具体几何展开],
)

== 数据怎样流动
```text
Widget / layout / paint
        |
        v
RenderScene -- SceneCompiler --> CompiledScene
                                     |
Viewport + FrameProperties ----------+
                                     v
                                  BuiltFrame
                                     |
                         Backend / graph planning
                                     v
                               Surface / present
```
Scene 是“内容与关系”；CompiledScene 是“组织好的绘制结构”；BuiltFrame 是“这一帧的执行数据”。不要把编译器当作负责创建 GPU 纹理的对象，纹理与栅格缓存仍属于后端。

== 新思路解决的问题
旧路径即使复用实体 ID，结构更新仍会付出全场景元数据遍历或清理扫描。当前路径用子树摘要定位变化，用绘制区间复用干净分支，用移除日志删除实体，避免把每个结构变化都变成全树重建。
]

#chapter[
= 对象模型与身份
== RenderScene 的五种节点
#table(columns: (1fr, 1.3fr, 1.7fr),
  [节点], [作用], [编译后对应物],
  [`Group`], [保存有序孩子集合], [展开到当前 Picture 的 items],
  [`Primitive`], [Shape、Vector、Image、Text], [`CompiledPrimitive`],
  [`Transform`], [单个孩子的局部坐标空间], [`CompiledSpatialNode`],
  [`Clip`], [单个孩子的裁剪条件], [`CompiledClip` 与裁剪父链],
  [`Layer`], [效果、缓存、合成与隔离边界], [展平内容，或生成 `CompiledPicture`],
)
Scene 具有一个根 Group。编译器另设 synthetic root spatial 与 root picture，把所有普通绘制内容放到可消费的根结构中。Layer 与 Transform、Clip 是单孩子容器；给 Group 使用 `append_child`，给单孩子容器使用 `set_child`。

== 源节点与编译实体不能混用
`RenderNodeId` 标识源场景节点。`PrimitiveId`、`SpatialNodeId`、`CompiledClipId`、`PictureId` 标识编译实体。两层都使用带代际的 slotmap key，但它们不是同一种身份。

CompiledScene 的四个 source 索引维护源节点到实体的映射。可通过 `primitive_for_source`、`spatial_for_source`、`clip_for_source`、`picture_for_source` 查询。非结构更新通常保持 ID；结构更新也尽量复用仍存活、仍对应相同角色的实体。

#note[身份的适用范围][
  ID 稳定不表示字段不变，也不表示 GPU 内容自动可复用。新建 SceneCompiler、删除后重建节点，或 Layer 隔离切换，均可能改变相应实体身份。缓存还必须检查内容与放置版本。
]

== Picture 的绘制列表
`CompiledPicture.items` 保存 `Primitive(PrimitiveId)` 或 `Picture(PictureId)`，顺序就是绘制顺序。Transform、Clip 和普通 Group 不占据独立绘制条目，它们改变后代图元的空间或裁剪关系。

`placement_spatial` 与 `placement_clip` 描述一个 Picture 放进父 Picture 时的位置与裁剪。Picture 自己的内容和它放置到父层的实例是两个层次，这是缓存内容与动态合成分离的基础。

== Widget 侧的保留输出
内部 `SceneWriter` 按位置和节点种类协调 Widget 的声明式绘制输出，兼容节点可复用，过期孩子会删除。它是 crate 内部工具；应用通常通过 Widget/style/runtime 接口工作，不应直接依赖该私有类型。
]

#chapter[
= 一帧的完整操作顺序
== Runtime 的标准入口
`UiRuntime::build_render_frame` 先取得 scene dirty snapshot、properties snapshot 与 viewport。首次编译、dirty 队列非空或存在移除日志时，才调用 SceneCompiler。Scene 无变化、properties 无变化且 viewport 相同时，返回 `None`。

Viewport 或动态属性单独改变，会生成新的 BuiltFrame，但不要求重新编译 Scene。FrameBuilder 之前先用 `FrameProperties::validate` 检查动态目标的节点种类。

== 成功呈现后再确认
+ 更新 Scene 或 FrameProperties。
+ 取得与本帧对应的两种 snapshot。
+ 必要时 compile，再 validate 与 build。
+ 后端执行 begin_frame、submit、end_frame。
+ 当 `did_present()` 为真时，runtime 调用 `finish_render_frame`。
+ 以本帧 snapshot acknowledge scene 与 properties，并记录 viewport。

#note[确认是消费完成的边界][
  compile 成功不等于帧已经呈现。后端错误或未呈现时，不应提前清空 dirty。当前 App 的标准流程已经使用 `did_present()` 控制确认。
]

== 编译与快照的对应关系
SceneCompiler 根据当前 source revision 与已编译 revision 判断是否需要工作；再根据 snapshot.nodes 找到候选节点，用各类 epoch 筛出真正的新变化。调用方必须让 snapshot 与本次 source 更新匹配，不能随意传空 snapshot 编译已经修改的 Scene。

当前 API 假设调用期间通过 Rust 借用规则保持 Scene 稳定。snapshot 主要用于跨 build/present 的消费确认，不是整个场景内容的不可变副本。

== 新变化怎样免于被旧帧清掉
Scene acknowledge 只清理 newest epoch 不晚于 snapshot revision 的节点。构帧后又发生更新的节点仍保留 dirty。properties acknowledge 也以 snapshot revision 为界，新动态值仍然待消费。removed 日志只消费该 snapshot 对应的前缀。

== 高级接口的使用限制
`take_dirty_nodes`、`clear_node_dirty` 和全量 `rebuild_subtree_dirty` 是低层工具，不应替代普通帧循环的 snapshot/acknowledge。直接清标记会绕过正常摘要维护；批量特殊编辑需要额外恢复摘要的一致性。
]

#chapter[
= Dirty 分类与路径选择
== 七种变化标记
#table(columns: (1fr, 1.35fr, 1.7fr),
  [标记], [典型触发], [当前编译路径],
  [`TOPOLOGY`], [插入、移除、换父或重排], [结构更新],
  [`VISIBILITY`], [显隐切换], [结构更新],
  [`GEOMETRY`], [图元尺寸、局部 transform], [已有实体原地更新],
  [`PAINT`], [颜色、图像或文本绘制属性], [图元内容原地更新],
  [`CLIP`], [裁剪形状变化], [已有 clip 原地更新],
  [`COMPOSITE`], [Layer opacity、transform 等], [原地更新；隔离切换例外],
  [`EFFECT`], [效果、缓存策略、强制离屏等], [原地更新；隔离切换例外],
)

实际操作应调用 Scene 的 mutation 方法，由它们比较新旧值并产生标记。相同值通常返回 `false`，避免无意义地推进 revision。

== structural 不是只看一个 bit
`requires_structural_rebuild` 的规则：首次编译必定全建；TOPOLOGY 或 VISIBILITY 必定结构更新；Primitive、Transform、Clip 缺少已有编译实体时也需结构更新；Layer 则比较当前 `requires_isolation()` 和是否已有 Picture。

所以默认 Layer 的 opacity 从 1 改成 0.5，即使标记只是 COMPOSITE，也会从展平变成隔离。已隔离 Layer 从 0.5 改成 0.6，则通常只改 descriptor。

== revision 与 epoch 的职责
Scene revision 是全局更新序号，RenderEpochs 分别记录 topology、geometry、paint、composite、effect 的最近序号。CLIP 和 VISIBILITY 共用 geometry epoch。`dirty_since` 把 epoch 与已编译 revision 比较，过滤已处理但尚未确认的 dirty。

`ContentVersion` 包括 topology、geometry、paint、dynamic，刻意把 Layer 自身 effect 和 composite 分开保存。版本是变化追踪信息，不是内容哈希。

== 决策练习
按钮换颜色：PAINT，更新 primitive。滚动改变 Scene transform：GEOMETRY，更新 spatial 并刷新相关 bounds 索引。列表增行：TOPOLOGY，重组条目。透明度动画写 properties：Scene 不 dirty，直接 build。Blur 参数改变但 Layer 仍隔离：更新静态 program，通常不重组 Picture 图。
]

#chapter[
= Dirty 摘要与移除日志
== 自身 dirty 与子树 dirty
每个 RenderNode 保存 `dirty` 和 `subtree_dirty`。前者表示自己改变了什么，后者是自己和后代变化标记的并集。编译器通过后者判断一个分支是否可以整块复用。

`mark_dirty` 更新自身 epoch 和 dirty，首次入队时加入 `dirty_nodes`，再向父链传播摘要。某位祖先已经包含全部待传播 bit 时，传播停止；它上方已经接收过同一类摘要。

```text
Root: TOPOLOGY
  A: empty       -> 可尝试复用
  B: TOPOLOGY    -> 继续向下
    B1: empty    -> 可尝试复用
    B2: TOPOLOGY -> 实际变化
  C: empty       -> 可尝试复用
```

== acknowledge 后摘要怎样回落
节点 dirty 按 snapshot revision 清理后，`refresh_subtree_dirty` 收集 snapshot 中仍存在的节点及其祖先，并对共同祖先去重，再按从子到父的顺序重新计算“自身 dirty + 直接孩子摘要”。每个受影响节点只计算一次；未受影响的子树直接贡献已有摘要，无需递归进入。

它仍会读取受影响节点的直接孩子，但同一宽 Group 不会因多个脏孩子而重复扫描。成本为受影响节点数量加上这些节点的直接孩子数量之和，避免逐个脏孩子向上回刷造成的平方级开销。snapshot 后的新 dirty 仍保留；即使节点自身因新变化不能清理，也会刷新其摘要，以去掉已删除孩子留下的标记。

== 移除与脱离是两种操作
`detach` 让节点离开当前树，但节点仍存在；`set_visible(false)` 也保留源节点。当前增量路径允许它们的编译实体暂时保留，只是当前绘制列表不再引用它们。

`remove_subtree` 真正删除源实体，并把被删除的节点 ID 加入 `removed`。编译器入口首先调用 `drop_removed_sources`，清理四种实体、source 索引与 node_layout。这个动作先于 revision 快速返回，因此已脱离子树的删除也能被处理。

#note[生命周期操作建议][
  临时藏起或准备再挂载：detach/visibility。永久销毁：remove_subtree。删除时也清理对应动态 properties；直接控制低层管线可用 `remove_source`，它不会替你递归枚举整个子树。
]

== 什么情况下仍全量清理
首次编译用 full walk 与 live-set sweep 建立完整结果。后续 dirty-path 编译主要靠移除日志清理，避免每次扫描所有 retained 实体。隐去的内容可能占用内存直到实际删除或再次走访，这是当前策略的取舍。
]

#chapter[
= 结构增量编译的核心算法
== 一次更新的入口
首次编译调用 `rebuild_structure`，已有编译结果的结构变化调用 `rebuild_dirty_paths`。编译器用 `self.compiled.take()` 取得原对象继续改写，避免克隆全部 SlotMap 和 source 索引。

增量路径先取走 root picture 的旧 items 与 spans，重置 synthetic root，然后从 root 走访。走访不是无条件递归每个后代：遇到可复用子树时复制旧区间并立即返回。

== NodeItemLayout 记录什么
#table(columns: (1.15fr, 2.85fr),
  [字段], [含义],
  [`parent`], [上次放置此节点时的源父节点],
  [`offset / count`], [相对父节点首条目的偏移，以及该子树在当前 Picture 内贡献的条目数],
  [`local_bounds`], [子树在放置空间里的 paint bounds],
  [`boundable`], [能否保守地给该子树确定边界],
  [`bounds_valid`], [记录的 bounds 是否仍可作为复用依据],
  [`generation`], [最近一次放置这个节点的 walk],
  [`children_generation`], [最近一次实际放置其孩子的 walk],
)

== 什么条件下复制旧区间
+ 调用上下文能找到可信的 `old_start`。
+ 该节点 `subtree_dirty` 为空。
+ 存在旧 NodeItemLayout，且 `bounds_valid` 为真。
+ 旧 items 中对应的 range 仍然有效。

满足时 `copy_previous_items` 复制绘制条目与属于子树内部的 spans，调整新 start/offset，并复用旧 local_bounds。图元与空间实体不重新创建，子树后代也不逐个访问。

== 什么条件下退回 walk
节点有变化、换了父亲、旧范围无法定位、布局代际不匹配或 bounds 失效时，继续 walk。即使当前节点不能复用，后代仍可逐个尝试复用，不必自动重编整个分支。

== 复杂度应怎样理解
遍历工作集中在 dirty 路径和需检查的兄弟入口；但 root 等 Picture 的 Vec 仍在组织新列表，复制条目成本与被复制 item 数有关，spans 还需调整与排序。当前是“跳过干净子树的节点处理”，并非持久化绘制列表的零拷贝 splice。
]

#chapter[
= 重排、换父与 generation
== 为什么不缓存绝对数组下标
设 A 有 3 个 items，B 有 2 个，C 有 4 个。原列表中 C 从 5 开始。B 新增 1 个条目后，C 从 6 开始。C 的图元完全没变，但绝对位置变了。offset 的用途就是在父范围被重新放置时，重新推导孩子的位置。

```text
旧 Root: [ A A A | B B   | C C C C ]
新 Root: [ A A A | B B B | C C C C ]
                           ^ 从旧区间复制，写到新 start
```

== 两个 generation 为什么缺一不可
假设 A 整块复用。编译器更新 A 自己的 generation 和位置，却没有进入 A 的后代。因此 A.children_generation 必须保留，表示孩子的 offset 仍属于之前那次实际 walk。

下次 A 内部有变化，要定位孩子旧范围时，检查孩子的 `parent == A`，且孩子 generation 等于 A.children_generation。两者都成立，才用父的旧起点加孩子 offset。

#note[避免一个常见错误][
  整块复用时不能把所有孩子当作已经重新放置，也不能随意把 children_generation 改成当前 walk。否则后续更新会误把过期区间当作当前区间，导致漏画或顺序错乱。
]

== 换父与隔离切换
换父后旧 parent 不匹配，旧 range 定位失效，编译器重新 walk。这也会更新 inherited spatial/clip。不要仅因为 `subtree_dirty` 为空，就复用一个换了放置上下文的子树。

Layer 新增隔离时，孩子内容从父 Picture 转入新 Picture。Layer 失去隔离时，移除它的旧 Picture，将孩子展开到父 Picture。旧列表来自不同 Picture，不能直接按原下标搬用；当前代码在这些边界主动放弃旧 base。

== 维护此算法的约束
每个子树在其 Picture 中的贡献必须连续；父子相对 offset 必须对应正确的 generation；span 的 range 调整必须与 item 复制一致；isolated Picture 的孩子列表从本 Picture 的 0 开始。更改 walk 顺序或引入新容器类型时，应逐项重新验证。
]

#chapter[
= Bounds 与整段可见性裁剪
== 两份边界索引
NodeItemLayout 的 bounds 用于结构复用；CompiledItemSpan 的 bounds 用于 FrameBuilder 裁剪。它们目的不同，不能只更新一份然后认为全部优化安全。

Span 保存 spatial、local_bounds、start、end。条目范围为半开区间，spans 按 start 排列，包围 span 先于内部 span。FrameBuilder 检查一个整段在 viewport 外时，可以直接跳到 end，省掉内部图元的空间解析与逐项裁剪。

== 局部空间是关键
Span bounds 位于对应 spatial 自身 transform 应用之前。动态修改该 spatial 的 transform 时，FrameBuilder 用新世界矩阵转换 bounds，所以该 span 仍然可用。

若动态 transform 位于它的后代，祖先 bounds 已经折叠过旧后代位置，不能继续依赖它。FrameBuilder 从动态 transform 的父 spatial 向上标记 stale spans；这些祖先失去整段裁剪机会，回退到后续项级处理。

== 非结构更新的失效处理
`apply_non_structural_updates` 对非纯 COMPOSITE 更新调用 `invalidate_layout_bounds`，把节点及祖先的复用 bounds 标为无效。这样后来结构更新时，不会直接搬用过期边界。

当有 GEOMETRY 更新时，当前实现调用 `refresh_span_bounds`，依据现有图元与局部矩阵重算 spans。这个步骤可能遍历已有 Picture、span 和相关图元；几何更新不能一概描述为恒定开销。

== 效果边界为什么保守
隔离 Picture 的效果可能画到内容 bounds 外。因此其 walk 返回 `boundable = false`，阻止包围它的普通 span 用内容边界裁掉效果。离屏 Layer 内容也不简单套用根 viewport 的逐项裁剪，它仍需提供给后端计划阶段。

#note[扩展 Primitive 时检查 paint_bounds][
  新的 stroke、shadow、滤镜或绘制属性若改变 paint_bounds，必须检查 dirty 分类与 span 刷新触发。当前 span 刷新以 GEOMETRY 为触发，而部分绘制变化会归到 PAINT。不要把“PAINT 一定不改变边界”当成普遍约束；扩展时应增加边界回归测试。
]
]

#chapter[
= 动态属性与动画接入
== 两种 transform 的区别
`FrameProperties::set_transform` 覆盖某个 Transform 节点的局部矩阵，FrameBuilder 用它替代 Scene 的 local_transform，再与父矩阵组合。它不是在旧矩阵后追加一个 delta。

`DynamicComposite.transform` 改变隔离 Layer 完成后如何合成到父层，不改变该 Layer 内容绘制所用的世界 transform。两者影响的版本与缓存层次也不同。

== 选择正确的入口
#table(columns: (1.4fr, 2.6fr),
  [意图], [操作],
  [修改 Scene 的稳定基准位置], [`scene.update_transform(id, matrix)`],
  [只改变本帧某个空间的位置], [`properties.set_transform(id, matrix)`],
  [动画整层透明度/合成位置], [`properties.set_composite(layer, value)`],
  [回到 Scene 的基准值], [`clear_transform` 或 `clear_composite`],
  [源节点生命周期结束], [`remove_source` 清理动态覆盖],
)

== 接入步骤
+ 保存目标源节点 RenderNodeId，Transform 动画目标必须是 Transform。
+ Layer composite 动画目标必须已经是隔离 Layer；如需长期保持隔离，可在 descriptor 上设 force_offscreen。
+ 每个采样时刻只更新 properties；相同值不会推进 properties revision。
+ 取得 properties snapshot，validate 后用已有 CompiledScene build。
+ 成功呈现后 acknowledge 对应 snapshot。

`validate` 对树外目标忽略，对附着但种类错误的目标报错。手动低层调用时应保留这一步；它不是任意目标节点都能动画的通用变换系统。

== 内容与放置版本
动态空间 transform 会进入绘制项 ContentVersion.dynamic。动态 composite 则进入 Layer 实例 PlacementVersion.dynamic。隔离层内容不变时，可以由后端选择复用内容并只更新放置；是否真正命中栅格缓存取决于后端策略。

#note[使用范围][
  此低层路径证明动画可以跳过 SceneCompiler。高层 style/keyframe 是否走该路径，应另外查看 runtime 的同步逻辑；不要默认所有 transform 动画都已经自动绕过 Scene 更新与布局。
]
]

#chapter[
= Layer 隔离与静态图程序
== requires_isolation 的完整条件
以下任意一项成立，当前 Layer 就需要独立 Picture：force_offscreen；BackdropIsolation::Isolate；非空 effects；存在 backdrop_style；cache_policy 不为 None；非默认 blend_mode 或 composite operator；opacity 不为 1；composite transform 不为 identity。

只设置 cache_key 不会单独触发隔离。隔离是保守策略，当前还没有证明“把整体 opacity 分摊到单个 primitive”后再自动展平的优化。

== LayerProgram 如何复用
SceneCompiler 为隔离 Picture 保留 BuiltLayerProgram，即静态 `Arc<LayerProgram>` 与场景拥有的外部 mask 绑定。`reusable_render_program` 比较实际 graph style，再决定复用、重绑或重新编译。

#table(columns: (1.4fr, 2.6fr),
  [变化], [结果],
  [静态 style 和 bindings 都相同], [复用已有 bound program],
  [静态 style 相同，mask 资源绑定改变], [保留 Arc program，创建新的绑定],
  [静态 graph style 改变], [重新 compile_layer 并 bind],
  [动态 Layer opacity/transform 改变], [保留静态 IR，在帧实例中提供值],
)

静态 graph descriptor 包含 backdrop 配置、effects、合成 blend/operator 与工作色彩空间。Layer 自身的动态 composite opacity/transform 被排除在静态 fingerprint 外；backdrop 配置中的 opacity 仍是该静态描述的一部分。

== 编译后的图在哪里执行
`LayerProgram` 表达规范化的资源与操作。instantiate 结合 LayerGeometry 计算边界、展开 pass、分配抽象 transient texture slots。Backend 再把这些逻辑资源落实为平台纹理和执行命令。核心 graph crate 为纯 Rust、无 GPU 依赖。

== cache_key 校验
显式 cache key 在同一附着 Scene 中不能重复。当前 Scene 维护拥有显式 key 的 live Layer 数量，少于 2 时跳过碰撞检查；首次编译或 TOPOLOGY/EFFECT 变化且存在碰撞可能时，才进行检查。它仍可能是一次全树遍历，不能视为免费步骤。
]

#chapter[
= FrameBuilder 与后端消费
== build 负责本帧的哪些计算
FrameBuilder 从 root Picture 出发，按序消费 items，解析空间父链与动态覆盖，解析 clip 父链，计算世界 bounds，应用 viewport culling，并建立 BuiltLayer、BuiltLayerInstance 与 BuiltDraw。

一次 build 内以 spatial_cache 和 clip_cache 避免重复解析公共父链。它们是本帧的计算缓存，不等于 SceneCompiler 的跨帧 retained 结构。

== 内容与实例的对应关系
BuiltLayer 是内容：items、content_bounds、render_bounds、content_version、effect_version、cache 策略。BuiltLayerInstance 是放置：composite、clip、world_bounds、placement_version、静态 render_program 与 destination_prefix。

父 clip 作用于 Picture 放置，而不是被提前烘焙进可缓存内容。这样父层裁剪或合成位置改变时，后端可以按版本判断是否保留原内容。

== Backdrop 的 destination 不能用最终画面
Backdrop 或显式 destination read 需要看到“该操作执行前”的已有画面。BuiltFrame 用 SurfacePrefix 表示某层前 item_count 个条目，再用 CompositePrefix 串联父背景和局部前缀。

这是一份逻辑的有序 destination 状态，不是已分配的纹理。两个相邻玻璃层各自观察的 prefix 不同，后端不能把它们都绑定到同一份最终父画面。

BackdropIsolation 控制后代能否观察 Layer 之外的 destination 历史；它不会禁止当前 Layer 自己的 backdrop 读取合成前的父背景。

== 后端实现步骤
+ 保持 BuiltItem 的顺序，Draw 与 Layer 的交错不可丢失。
+ 根据 BuiltLayer 内容版本与 effect_version 判断各级缓存是否可复用。
+ 按 BuiltLayerInstance 处理放置、裁剪与合成参数。
+ 若图程序声明 destination read，解析对应 prefix。
+ 结合几何实例化或计划 graph，落实资源与 pass。
+ 呈现成功后通过 did_present 报告真实结果。

不要只凭 cache ID 或静态 program fingerprint 判定整层像素未变。内容、效果、动态放置以及 destination 依赖都可能使执行结果改变。
]

#chapter[
= 最小工作示例
== 构造、编译与构帧
以下为可执行示例，另存于同目录 `example.rs`。它只构建 CPU 侧帧；没有窗口，也不模拟成功呈现后的 runtime 确认。

```rust
use xui_core::render::{
    FrameBuilder, FrameProperties, Primitive, RenderScene,
    SceneCompiler, Shape, ShapePrimitive,
};
use xui_interface::{Affine, Bounds, Color, ComputedColorStyle};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut scene = RenderScene::new();
    let space = scene.insert_transform(Affine::IDENTITY);
    let node = scene.insert_primitive(Primitive::Shape(
        ShapePrimitive {
            bounds: Bounds::from_origin_size(
                (0.0, 0.0), (40.0, 20.0)),
            shape: Shape::Rect,
            fill: Some(ComputedColorStyle::Solid(Color::BLACK)),
            stroke: None,
            shadow: None,
        },
    ));
    scene.append_child(scene.root(), space)?;
    scene.set_child(space, Some(node))?;

    let mut compiler = SceneCompiler::new();
    let snapshot = scene.dirty_snapshot();
    compiler.compile(&scene, &snapshot)?;
    let compiled = compiler.compiled_scene().unwrap();
    let revision = compiled.scene_revision();

    let mut properties = FrameProperties::default();
    properties.set_transform(space, Affine::translate(30.0, 0.0));
    properties.validate(&scene)?;
    let mut builder = FrameBuilder::new();
    let viewport = Bounds::from_origin_size(
        (0.0, 0.0), (200.0, 100.0));
    let frame = builder.build(compiled, viewport, &properties)?;
    assert_eq!(frame.scene_revision, revision);
    assert_eq!(frame.layers[0].items.len(), 1);
    Ok(())
}
```
== 接到真实帧循环
在上述 build 前获取 properties snapshot；后端成功 present 后，确认 scene snapshot 与 properties snapshot。示例中的 frame 只用于检查，故没有 acknowledge。实际应用优先使用 App/runtime 的现成帧循环。
]

#chapter[
= 常见操作配方
== 配方 A：更新图元颜色或尺寸
+ 保留 RenderNodeId，生成新 Primitive。
+ 调用 `update_primitive`，由 Primitive.diff 分类。
+ 取得 dirty snapshot 并 compile。
+ 用 source 查询编译实体 ID，应在非结构更新中保持稳定。
+ build 后确认颜色、bounds 与可见性；成功呈现后 acknowledge。

若新绘制属性扩大 paint bounds，应额外做第 8 章的边界测试。不能仅用“ID 没变”证明更新正确。

== 配方 B：插入、重排与换父
插入 Group 孩子用 append_child/insert_child；重排用 reorder_child 或 set_children；换父使用 Scene 的合法树操作。不要重建全部源节点来表达一次小变动，否则 retained 身份与可复用区间都容易丢失。

验证 A/B/C 三分支：只改 B，A/C 的实体 ID 保留，最终 items 顺序等于全新编译结果。再修改此前整块复用过的 A 的深层孩子，验证 generation 的二次更新路径。

== 配方 C：整层淡入淡出
先建立隔离 Layer，例如 force_offscreen 为 true；把内容挂在它的 child 下。compile 一次后，用 set_composite 写 `DynamicComposite { opacity: Some(value), transform: None }`。validate、build、present、acknowledge；动画结束 clear_composite 回到 descriptor 基准值。

若长期保持 force_offscreen，透明度回到 1 也不会展平。若之后移除 force_offscreen 且不再满足其他隔离条件，就会发生一次结构更新。

== 配方 D：隐藏、恢复与永久销毁
短暂隐藏调用 set_visible；恢复可重新 walk 并复用适用实体。永久销毁调用 remove_subtree，并逐个清理子树动态覆盖。下一次 compile 会先消费 removal log，然后调整当前绘制结构。

== 配方 E：更换效果与 mask
已有隔离 Layer 用 update_layer_descriptor 更新 effects 或 backdrop。检查静态 style 是否改变、资源 bindings 是否改变、是否仍隔离。前两项影响 program 复用，最后一项决定 Picture 结构路径，三者不能混为一谈。

== 配方 F：只有 viewport 改变
保留 Scene 与 properties，直接 build 新 viewport。确认根层裁剪和离屏 Layer 计划仍正确。runtime 会记录新的 viewport；下次三种输入均未变时可以不构帧。
]

#chapter[
= 验证、性能观察与排障
== 建议先运行的已有测试
以下命令验证当前实现，不要求为了阅读手册修改源码：
```sh
cargo test -p xui-core render::compiler --lib
cargo test -p xui-core render::frame_builder --lib
cargo test -p xui-core render::scene --lib
cargo test -p xui-core render::properties --lib
cargo test -p xui-render-graph
```
compiler 中随机差分测试使用 24 个 seed，每个 150 步编辑。它比较增量与从头编译的规范化结构，检查 span 能覆盖对应内容，并断言确实发生过区间复制。不要比较两份独立编译结果的 slotmap ID 来判断等价。

== 针对新改动增加的检查
修改 dirty-path 算法：连续重排、换父、隐藏恢复、移除脱离节点、隔离切换、复用后深层更新。修改 bounds：与禁用 spans 的 build 比较绘制摘要，并测内容从屏幕外移回视口。修改动态属性：确认 scene_revision 不变，但相应 dynamic version 和世界位置变化。

== 性能观察的正确口径
分别测 Scene mutation、compile、frame build、graph plan、raster 和 present。记录访问节点数、复制 items 数、刷新 spans 数、program 重编译/重绑次数以及层缓存命中。不要用后端帧率单独证明 compile 优化，也不要把测试中的 COPIED_ITEMS 当作公开运行时统计 API。

#table(columns: (1.2fr, 2.8fr),
  [现象], [优先检查],
  [变化后没有新帧], [mutation 是否生效、snapshot 是否对应当前 revision、是否提前 acknowledge],
  [重排后顺序错误], [offset/count、parent、两种 generation 与旧 Picture 来源],
  [图元移动回视口后漏画], [span bounds 是否刷新，动态后代是否使祖先 span 失效],
  [动画意外改变内容位置], [是否误用 spatial transform 代替 Layer composite transform],
  [隔离层动画报错], [目标是否附着且 requires_isolation 为真],
  [内存保留超过预期], [节点是否只 detach/hidden，而没有 remove_subtree],
  [Backdrop 看到错误背景], [destination prefix 的时间顺序及 isolation 边界],
)

== 编译失败怎样恢复
重复 cache key、无效 graph style、资源 binding 错误或 Scene 错误会返回 SceneCompileError。结构重建失败时半写入的结果被丢弃，下一次从 full rebuild 开始。非结构更新不要假设具有同样的原子回滚保证；修正输入后重试，必要时用新 compiler 做全建基准。
]

#chapter[
= 源码索引与维护清单
== 从入口到细节的阅读路线
以下路径相对于仓库根目录，可用函数名定位，避免文档依赖会漂移的行号。
#table(columns: (1.8fr, 2.2fr),
  [源码路径], [阅读目标],
  [`crates/xui-core/src/ui_runtime/frame.rs`], [build_render_frame、finish_render_frame],
  [`crates/xui-core/src/render/scene.rs`], [RenderDirty、RenderEpochs、mark_dirty、acknowledge、removed],
  [`crates/xui-core/src/render/compiler.rs`], [compile、requires_structural_rebuild、rebuild_dirty_paths、walk_node],
  [`crates/xui-core/src/render/compiled_scene.rs`], [NodeItemLayout、CompiledItemSpan、四种实体],
  [`crates/xui-core/src/render/frame_builder.rs`], [build、span_is_offscreen、resolve_spatial、build_child_picture],
  [`crates/xui-core/src/render/properties.rs`], [validate、动态属性与版本确认],
  [`crates/xui-core/src/render/layer.rs`], [requires_isolation 的完整规则],
  [`crates/xui-core/src/render/render_graph.rs`], [静态 descriptor、style 比较与资源绑定],
  [`crates/xui-core/src/render/built.rs`], [内容、放置与逻辑 destination 数据模型],
  [`crates/xui-core/src/render/destination.rs`], [有序 composite prefixes],
  [`crates/xui-render-graph/src/`], [compiler、program、binding、plan 与资源生命周期],
)

== 修改后应重新核对的六条约束
+ dirty 标记、epochs、队列和 subtree 摘要必须描述同一次变化。
+ 复制区间必须有可信旧放置，不能只检查“干净”。
+ 相同 source 的 retained 身份与内容版本分别正确。
+ bounds 优化失效时允许退回慢路径，不能漏画。
+ 静态 program、资源 bindings、内容和合成放置分开更新。
+ acknowledge 对应成功消费的 snapshot，保留后续新变化。

== 重新生成手册
在仓库根目录执行：
```sh
typst compile docs/render-compile/manual.typ \
  output/pdf/xui-render-compile-manual.pdf
```
模板使用系统 PingFang SC 与 Menlo 字体；其他系统可在 operations-template.typ 中改为已安装的中文与等宽字体。生成后渲染 PDF 页面对表格、代码、页眉和页码做视觉检查。

本文描述的是已存在的代码行为；性能观察指标和扩展测试是维护建议，并不表示 runtime 已提供所有计数器或所有动画均自动采用动态属性路径。
]
