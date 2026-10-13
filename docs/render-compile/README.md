# Render Compile 操作手册

以 2026-10-12 的当前工作区为基线：HEAD `e972c54`，包含尚未提交的 render 增量编译改动。

- [手册 PDF](../../output/pdf/xui-render-compile-manual.pdf)：17 页，15 章。
- [Typst 正文](manual.typ)：可编辑内容。
- [操作手册模板](operations-template.typ)：本地模板，含封面、目录、章节、提示框、表格与页码样式，无在线包依赖。
- [Rust 示例](example.rs)：CPU 侧构帧与动态 transform 示例，可复制到依赖 `xui-core`、`xui-interface` 的 Cargo 项目中作为入口运行。

## 生成 PDF

在仓库根目录运行：

```sh
mkdir -p output/pdf
typst compile docs/render-compile/manual.typ output/pdf/xui-render-compile-manual.pdf
```

默认字体为 PingFang SC、Arial、Menlo。其他系统请在模板中换成已安装的中文和等宽字体。字体变化可能改变页数；生成后请重新检查页面。

## 内容范围

架构与对象模型、帧生命周期、dirty 分类与 epochs、移除日志、dirty-path 编译、相对区间与 generation、bounds 与 spans、动态属性、隔离与静态 program、FrameBuilder 与 destination prefixes、最小示例、操作配方、测试和排障、源码索引。

## 本次验证

- `cargo test -p xui-core render:: --lib`：58 项通过。
- `cargo test -p xui-render-graph`：40 项通过。
- `example.rs`：使用当前工作区编译产物编译并运行成功。
- Typst 编译成功；PDF 全部页面渲染并检查，代码页与源码索引页单独复核。

以上是文档生成时的验证记录，并不取代后续代码更新后的测试。
