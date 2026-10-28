# 更新日志

## 0.1.0 — 第一阶段：基础框架

首个版本，只支持 Windows（MSVC）。

- 工作区：`cairn-ui`（主 crate）、`cairn-ui-platform`、`cairn-ui-render`、`cairn-ui-layout`，示例 `examples/hello`。
- 声明式写法、保留式控件树：`view(&self) -> impl View<Self>` 描述界面，回调直接修改应用状态，之后重跑 `view` 并与控件树比对，只重新布局 / 重绘变化的控件；支持 `key`。
- 控件：`column`、`row`、`label`（自动换行）、`button`、`text_input`（光标移动、删除、选区、剪贴板、双击选词、中文输入法预编辑与候选框定位）。
- 布局：taffy flexbox 子集，`Styled` 修饰方法（尺寸、内外边距、`fill_width`、`grow`、对齐、隐藏）。
- 渲染：`Painter` trait 与 tiny-skia 实现，脏区重绘，像素网格对齐；cosmic-text 排版，系统字体，字形缓存。
- DPI：Per-Monitor V2，逻辑像素布局、物理像素渲染，运行中缩放变化自动整窗重画；调试用 `CAIRN_UI_SCALE_FACTOR`。
- 事件与焦点：命中测试、冒泡、指针独占、悬停、点击与 Tab 焦点切换、键盘焦点框、光标闪烁定时器。
- 后台线程通过 `Proxy::update` 修改状态。
- `cairn_ui::testing::Headless`：无窗口运行界面，用于自动化测试与截图。
- `examples/hello` release 体积 1.92 MiB。
