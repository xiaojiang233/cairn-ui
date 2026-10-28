# cairn-ui 设计说明

本文描述 0.1.0 的内部结构，供以后扩展和移植时参考。使用方式见仓库根目录的 README。

## 1. 分层

```
            ┌──────────────────────────────────────────────┐
  应用代码   │  状态结构体 + view() + 回调                      │
            └──────────────────────┬───────────────────────┘
            ┌──────────────────────▼───────────────────────┐
  cairn-ui  │  App / Ui 运行时：控件树、比对、事件分发、焦点、  │
            │  脏区、主题、内置控件、无窗口测试                 │
            └───────┬──────────────────┬───────────────┬───┘
   cairn-ui-platform│     cairn-ui-render│   cairn-ui-layout│
   winit + softbuffer│ tiny-skia + cosmic-text│        taffy │
```

- 三个下层 crate 互不依赖，只被 `cairn-ui` 使用。每层只暴露自己定义的类型：
  - platform：物理像素的事件、`PixelRect`、`Surface::present(rgba, …)`、`FontConfig`、系统参数；
  - render：逻辑像素的 `Point` / `Size` / `Rect` / `Color`、`Painter`、`TextLayout`、`Canvas`；
  - layout：`Style`、`Layout`、`MeasureInput`。
- `cairn-ui` 负责在这些类型之间换算（例如物理像素 ÷ 缩放因子 = 逻辑像素），并 re-export 用户需要的部分。

## 2. 一次交互的完整流程

以“点击按钮，标签文字变化”为例：

1. winit 收到 `WM_LBUTTONDOWN` / `UP`，platform 转成 `Event::PointerButton`（位置取最近一次 `PointerMoved`，物理像素）。
2. `Ui::handle` 换算成逻辑坐标，命中测试找到最深层的控件，沿父链冒泡分发 `PointerDown`；处理了按下事件的控件在松开前独占指针事件（按钮按住后移出、拖选文字都依赖这个）。
3. 按钮在 `PointerUp` 且指针仍在按钮内时调用 `on_click(cx.app())`。`EventCx::app()` 会把“需要重建界面”的标记置上。
4. 本批事件处理完后 `Ui::after`：
   1. 重新调用 `view(&state)` 得到新元素树，与控件树比对（见第 3 节）；
   2. 有节点需要重新布局就跑 taffy，更新各节点的包围盒，位置或尺寸变化的节点新旧区域都计入脏区；
   3. 同步输入法状态和候选框位置、定时器；
   4. 有脏区就 `request_redraw`。
5. winit 发 `RedrawRequested`，`Ui::render` 把脏区换算成物理像素区域，逐块重画到画布，`Surface::present` 只把这些区域拷进 softbuffer 缓冲区并提交。

## 3. 控件树与比对

- 元素 `El<S>`：一个控件对象（`Box<dyn Widget<S>>`）+ 布局样式 + 子元素 + 可选 key。`view` 每次都构建一棵新的元素树。
- 控件树：`slotmap` 存储节点，`WidgetId` 是带代数的稳定标识（控件存在期间不变，删除后不复用），每个节点对应一个 taffy 节点。
- 比对规则（`tree.rs`）：
  - 子元素有 key 的按 key 匹配旧节点，没有 key 的按出现顺序依次匹配没有 key 的旧节点；
  - 匹配上且具体类型相同 → `Widget::update(cx, new)`：控件拷贝新描述里的属性、保留内部状态，返回 `Change { layout, paint }`；
  - 类型不同或没有匹配 → 删掉旧子树、新建；剩下没匹配上的旧节点删除（其区域计入脏区，焦点 / 悬停 / 定时器里对它们的引用被清掉）；
  - 样式（`Style`）有变化就更新 taffy 节点。
- `Widget::update` 的返回值决定代价：文字变了返回 `LAYOUT`（重新测量 → 重新布局 → 重绘），只改颜色返回 `PAINT`，只换了回调返回 `NONE`。
- 类型擦除用的是 `AnyWidget`：先比较 `TypeId`，相同再 `downcast`，所以同一位置换了控件类型会正确重建。

## 4. 脏区

- 记录（`damage.rs`）：逻辑像素矩形，最多 8 个；新矩形与已有矩形相交（含 2 像素内贴近）就合并，超过上限合成一个外接矩形。窗口尺寸或缩放变化时整窗重画。
- 来源：控件 `update` / 事件处理返回的 `Change`、`EventCx::request_paint_rect`（光标闪烁只报光标那一小块）、布局后包围盒的变化、节点删除。悬停变化本身不产生脏区，有悬停效果的控件在 `PointerEnter` / `Leave` 里自己请求重绘，避免鼠标划过大容器时整块重画。
- 重画（`ui.rs`、`render/skia.rs`）：每个脏区外扩 4 个逻辑像素（覆盖抗锯齿边缘和画在控件外面的焦点框），向外取整到物理像素，重叠的再合并。每块区域先画进一块同样大小的临时像素图（天然被裁剪到该区域），按树顺序重画所有与它相交的控件，再拷回整窗画布。这样大背景填充只处理脏区那一小块，也不需要为整个窗口生成裁剪蒙版。
- 上屏（`platform/present.rs`）：softbuffer 的 Win32 后端只有一个 DIB 缓冲区，提交后 `age() == 1`，内容保留。此时只把脏区从 RGBA 画布转成 `0RGB` 写进缓冲区并 `present_with_damage`；缓冲区是新的（`age() == 0`，刚创建或尺寸变了）或者系统要求重绘（没有脏区）时整帧提交。

## 5. 坐标、DPI 与像素对齐

- 对外接口和布局一律用逻辑像素。taffy 的取整被关掉，取整放到渲染阶段按物理像素做。
- `SkiaPainter` 把矩形的四条边分别乘以缩放因子后四舍五入。相邻矩形共享同一条边时取整结果一致，不会出现缝隙或重叠；非空矩形至少占 1 个物理像素。
- 边框宽度取整到整数个物理像素（至少 1），边框用“外轮廓减内轮廓”的奇偶填充，比描边容易对齐。文本光标 1 个物理像素宽（150% 及以上为 2 个）。
- winit 在 Windows 上启用 Per-Monitor DPI Awareness V2。窗口创建尺寸按逻辑像素给出；`ScaleFactorChanged` 时更新缩放、清空字形缓存、整窗重画。winit 默认按系统建议调整窗口尺寸以保持逻辑尺寸，随后的 `Resized` 更新画布大小。
- 非整数缩放下窗口物理尺寸是整数，逻辑尺寸会有小数（460 × 1.052 取整为 484，484 / 1.052 = 460.04），布局会有不到 0.1 像素的差别，这是预期行为。
- 窗口先隐藏创建，画好第一帧再显示，避免闪白。

## 6. 文字

- 字体加载（`render/text.rs`）：平台层提供字体目录、按语言排好的默认字体候选（Windows 上中文环境为 Microsoft YaHei UI → Microsoft YaHei → Segoe UI）和系统语言；渲染层用 fontdb 扫描目录，取第一个存在的候选作为无衬线默认字体，交给 cosmic-text。字体回退（缺字时换字体）用 cosmic-text 自带的按平台、按文字系统的回退表，简体中文环境下汉字回退到 Microsoft YaHei UI。
- 加载在后台线程里和创建窗口并行进行。测试机上 release 构建扫描 250 个字体面约 40–70 ms。
- 排版在逻辑像素下进行：cosmic-text 默认不做度量取整，字形位置是小数，与分辨率无关，所以缩放变化不需要重新排版。光栅化时 `LayoutGlyph::physical(offset, scale)` 按“逻辑字号 × 缩放因子”生成字形缓存键（含横向亚像素分档），用 swash 光栅化成灰度蒙版。
- 字形缓存：cosmic-text 的 `SwashCache`（以字体、字形、物理字号、亚像素分档为键）。缩放变化时清空。
- 混合：灰度蒙版按 source-over 直接混合进预乘 alpha 的画布；彩色字形（emoji）按非预乘 RGBA 处理。没有做伽马校正。
- `TextLayout` 提供命中测试（点击位置 → 字节位置）、光标位置、选区矩形，单行文本框和以后的多行编辑都用它。

## 7. 输入、焦点与命中测试

- 命中测试：从根节点深度优先，后画的子节点优先；子节点可能超出父节点，所以不先判断父节点是否包含点。隐藏的子树跳过。
- 键盘事件发给有焦点的控件并冒泡；没人处理的 Tab / Shift+Tab 用来切换焦点（按树顺序循环）。
- 点击时把焦点给命中点上最近的可获得焦点的祖先；点到别处清除焦点。焦点通过键盘获得时 `focus_visible` 为真，按钮据此画焦点框。
- 定时器：每个控件最多一个待触发的定时器（`EventCx::request_timer`），平台层只设一个最近的唤醒时间点（`ControlFlow::WaitUntil`）。光标闪烁间隔读系统设置（Windows 上 `GetCaretBlinkTime`，用户关闭闪烁时光标常亮）。
- 双击判定的时间和距离同样读系统设置。

## 8. 输入法（Windows）

winit 0.30 在 Windows 上走 IMM32 接口（微软拼音等 TSF 输入法通过系统的兼容层工作）。读 winit 源码得到的事件序列：

- `WM_IME_STARTCOMPOSITION` → `Ime::Enabled`；每次组字开始都会发，**不代表焦点变化**。
- `WM_IME_COMPOSITION`（`GCS_COMPSTR`）→ `Ime::Preedit(text, Some((a, b)))`：`a == b` 是组字光标位置，`a < b` 是当前正在转换的子句。
- 提交：`Preedit("", None)` 紧接着 `Commit(text)`。
- `WM_IME_ENDCOMPOSITION` → `Ime::Disabled`。
- 组字过程中被输入法处理的按键以 `NamedKey::Process`（`VK_PROCESSKEY`）出现，文本框必须忽略，否则会把回车、退格当成编辑操作。
- winit 处理 `WM_IME_SETCONTEXT` 时去掉了 `ISC_SHOWUICOMPOSITIONWINDOW`，输入法不画自己的组字窗口，预编辑文本由应用内联绘制；候选框仍由输入法显示。

cairn-ui 的处理（`widgets/text_input.rs`、`ui.rs`）：

- 文本框获得焦点 → `set_ime_allowed(true)`，失去焦点 → `false`（winit 内部是 `ImmAssociateContextEx`）。
- 预编辑文本插在光标处显示、画细下划线，正在转换的子句画粗下划线；它不进入文本框的值，提交后才通过 `on_change` 通知应用。开始组字时如有选区先删除选区。
- 每批事件处理后同步候选框位置 `set_ime_cursor_area`：横坐标取光标（组字时取组字起点，候选框不随拼音左右跳），纵向取整个输入框的高度。winit 用 `CFS_EXCLUDE` 传给输入法，候选框会避开这块区域，显示在输入框下方而不是盖住它。坐标按物理像素给出，重复值由平台层缓存去重。

## 9. 无障碍预留

每个控件有稳定的 `WidgetId`（`to_raw()` 可得 `u64`，可直接作为 accesskit 的节点 id）和 `Widget::semantics()`（角色、名称、值、是否禁用）。目前只有无窗口测试用它按文字查找控件；接入 accesskit 时需要在平台层加适配器、在运行时把控件树转成 accesskit 的树并处理动作请求。

## 10. 测试

`cairn_ui::testing::Headless` 用同一套运行时，只把平台服务换成记录型的假实现：可以模拟点击、按键、输入法事件，读出输入法开关、候选框位置、剪贴板、定时器，按文字或角色查找控件，得到每帧重画的区域，把画布编码成 PNG。`crates/cairn-ui/tests/headless.rs` 是例子。注意它使用系统字体，结果依赖当前系统安装的字体。

## 11. 平台相关代码清单

| 位置 | 内容 | 是否 Windows 专有 |
| --- | --- | --- |
| `cairn-ui-platform/src/lib.rs` | winit 事件循环、窗口创建、`Context`（重绘请求、光标形状、输入法开关与候选框、定时唤醒、退出、跨线程唤醒）、缩放覆盖环境变量 | 否（经 winit 跨平台） |
| `cairn-ui-platform/src/present.rs` | softbuffer 上屏、RGBA → 0RGB、按缓冲区年龄决定局部 / 整帧提交 | 否（经 softbuffer 跨平台） |
| `cairn-ui-platform/src/convert.rs` | winit 事件 → 本库事件 | 否 |
| `cairn-ui-platform/src/event.rs` | 事件类型；`Modifiers::shortcut()` / `word_jump()` 用 `cfg!(target_vendor = "apple")` 区分 Ctrl 与 Command / Option | 否 |
| `cairn-ui-platform/src/sys/mod.rs` | 按 `cfg(windows)` 选择实现 | — |
| `cairn-ui-platform/src/sys/windows.rs` | 剪贴板（`CF_UNICODETEXT`）、光标闪烁间隔、双击时间与距离、字体目录（`%WINDIR%\Fonts`、`%LOCALAPPDATA%\Microsoft\Windows\Fonts`）、按语言的默认字体 | **是** |
| `cairn-ui-platform/src/sys/fallback.rs` | 其他平台的占位实现 | 占位，未编译过 |
| `cairn-ui-platform/Cargo.toml` | Linux 目标下打开 winit 的 x11 / wayland 与 softbuffer 的对应后端 | — |

`cairn-ui`、`cairn-ui-render`、`cairn-ui-layout` 里没有任何 `cfg`（测试除外）或操作系统调用。

## 12. 做 Linux 时要改的地方

1. **先编译运行**：`Cargo.toml` 里 x11 / wayland 特性已经配好，但从没编译过。注意 winit 的 `wayland-csd-adwaita`（客户端标题栏）没有打开，GNOME Wayland 下窗口可能没有标题栏，需要决定是否打开（会增加体积）。
2. **剪贴板**（`sys`）：X11 的 selection 或 Wayland 的 data-device。可以自己实现，也可以用现成 crate，注意体积。
3. **字体**（`sys::font_dirs`、`default_font_families`）：现在是写死的目录和 Noto / DejaVu 字体名。更可靠的做法是走 fontconfig（fontdb 有 `fontconfig` 特性，可以只在 Linux 目标下打开；或者解析 `fc-match` 的结果）。
4. **系统参数**：光标闪烁、双击时间可以读 GTK / GNOME 设置（`gtk-cursor-blink-time`、`gtk-double-click-time`）。
5. **输入法**：winit 在 X11 上走 XIM、在 Wayland 上走 text-input-v3。X11 上 `set_ime_cursor_area` 只支持位置不支持区域。需要实测 fcitx5、ibus 的预编辑与候选框位置，以及 Enabled / Disabled 的语义是否与 Windows 一致（文本框代码按“Disabled 时清掉预编辑”处理，应该兼容）。
6. **DPI**：Wayland 分数缩放（winit 支持 `wp_fractional_scale`）与 X11（`Xft.dpi`）都由 winit 换算成缩放因子，上层不用改；需要实测。
7. **上屏**：softbuffer 的 Wayland 后端可能是双缓冲（`age() == 2`），现在的实现遇到 `age != 1` 就整帧提交，正确但多拷一些。想优化可以记住最近两帧的脏区并合并。
8. **快捷键约定**：`Modifiers::shortcut()` 在 Linux 上就是 Ctrl，不用改；macOS 要另外检查。
