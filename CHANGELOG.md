# 更新日志

## 未发布

**窗口**

- 无边框窗口：`App::decorations(false)`。控件 `drag_area()`（拖动窗口、双击最大化 / 还原）和 `window_controls()`（最小化 / 最大化 / 关闭按钮）；窗口四周 6 像素是缩放边，带对应的鼠标形状。Windows 上保留系统阴影与圆角。
- 拖入文件：`App::on_file_drop`，收到悬停、取消、放下三种事件。
- 运行时换主题：`App::theme_from` 在每次状态变化后询问是否有新主题，换主题时保留控件树。

**绘制**

- 矢量路径：`Path`（直线、二次与三次贝塞尔、圆、椭圆、圆角矩形）和 SVG 路径数据解析 `Path::parse`（`M L H V C S Q T A Z` 的大小写形式，圆弧换成三次贝塞尔）；`Painter::fill_path` / `stroke_path`，描边样式 `StrokeStyle`（端点、拐角、虚线）。路径用自己的设计坐标，画的时候给一个 `Affine` 映射到窗口。
- 透明度图层：`Painter::push_layer` / `pop_layer`，一组东西整体淡入淡出。
- 合成层缓存：控件返回 `Widget::cache_layer() == true` 时，整棵子树只画一次进位图，之后动画只移动 / 淡入淡出那张位图。
- 圆角矩形填充、玻璃背景、位图与图层的混合走专门的快路径（整数对齐时按行直接写像素，四个圆角按距离估算覆盖率），一个玻璃面板的整块重画约 17 ms → 5 ms（1178×736、opt-level 2，`cargo test --release -p cairn-ui-render bench_ -- --ignored --nocapture` 可复现）。
- 文字覆盖率按文字亮度校正，浅色字压在深色底上不再偏细。
- 环境变量 `CAIRN_UI_FRAME_STATS=1`：每帧把重建 / 布局 / 缓存 / 绘制各阶段耗时和重画面积打到标准错误。

**控件与布局**

- 新控件：`checkbox`、`tab`（图标 + 文字 + 徽标，可选中）、`list_item`、`progress_bar`、`spinner`、`divider`、`tag`、`icon` / `icon_button`、`reveal`（出现 / 离开带位移和淡入淡出）、`click_area`（透明点击区，用来做点外面关闭）。内置图标二十多个，用线条画。
- 绝对定位：`Styled::absolute` 加 `top` / `right` / `bottom` / `left`，浮层、弹出框用；`Styled::shrink`。
- `label().truncate()` 超出宽度显示省略号；`text_input().icon()`；`panel().bar()` / `popover()`。
- 主题增加 `bar`、`popover`、`divider`、`caption`、`checkbox`、`tab`、`progress`、`list_item`、`tag` 的样式，`Motion` 增加 `enter` / `exit` / `enter_offset` / `progress`。

**测试**

- `cairn_ui::testing::Headless` 增加窗口按钮的检查（最小化 / 最大化 / 关闭）。

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
