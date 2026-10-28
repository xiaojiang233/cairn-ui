# cairn-ui

轻量、保留式的 Rust GUI 库。设计目标是作为库被别的应用直接依赖，所以重视清晰的公开 API、可控的体积和平台隔离。

当前是 **0.1.0 / 第一阶段：基础框架**：Windows 窗口，标签、按钮、能输入中文的单行文本框，弹性布局，脏区重绘，DPI 适配。还没有滚动、图片、列表等，API 在 0.x 期间可能变化。

```rust
use cairn_ui::prelude::*;

#[derive(Default)]
struct Hello {
    clicks: u32,
    name: String,
}

impl Hello {
    fn view(&self) -> impl View<Self> {
        let name = self.name.trim();
        let greeting = match (self.clicks, name.is_empty()) {
            (0, _) => "你好，cairn-ui！点一下按钮试试。".to_string(),
            (n, true) => format!("按钮被点了 {n} 次"),
            (n, false) => format!("{name}，按钮被点了 {n} 次"),
        };

        column()
            .padding(24.0)
            .gap(14.0)
            .child(label(greeting).font_size(18.0))
            .child(
                row()
                    .gap(8.0)
                    .child(button("点我").on_click(|app: &mut Self| app.clicks += 1))
                    .child(button("清空名字").on_click(|app: &mut Self| app.name.clear())),
            )
            .child(
                text_input(&self.name)
                    .placeholder("输入你的名字（支持中文输入法）")
                    .fill_width()
                    .on_change(|app: &mut Self, text| app.name = text),
            )
    }
}

fn main() -> cairn_ui::Result<()> {
    App::new(Hello::view)
        .title("cairn-ui · hello")
        .size(460.0, 220.0)
        .run(Hello::default())
}
```

这就是 [`examples/hello`](examples/hello/src/main.rs) 的全部代码。

## 特点

- **声明式写法，保留式实现**：像 Compose 一样用嵌套调用描述界面，但没有 `remember`、重组、快照状态那一套；状态就是普通结构体字段。
- **只重画变化的地方**：界面描述与保留的控件树比对，只有真正变化的控件重新布局 / 重绘；点击按钮约重画 13% 的窗口面积，光标闪烁只重画 11×30 像素。
- **DPI**：布局用逻辑像素，渲染用物理像素并对齐像素网格；100%、105% 这类非整数缩放、125%、150% 都清晰；窗口换到缩放不同的显示器时自动按新缩放重画。
- **中文输入法**：预编辑文本内联显示（带下划线），候选框跟随光标、不遮挡输入框。
- **小**：`examples/hello` release + strip 后 **1.92 MiB**。不内嵌字体，用系统字体。
- **平台隔离**：所有操作系统相关代码在 `cairn-ui-platform` 里，上层 crate 没有一行 Windows 专有调用。

## 构建与运行

需要 Rust 1.89+（cosmic-text 0.19 的要求），目前只支持 Windows（MSVC 工具链）。

```bash
cargo run -p hello --release
```

```bash
cargo test --workspace
```

调试用环境变量：

| 变量 | 作用 |
| --- | --- |
| `CAIRN_UI_SCALE_FACTOR=1.5` | 忽略系统缩放，强制使用给定缩放因子。用来在不改系统设置的情况下看 125% / 150% 的效果；**不能**代替真实的系统缩放测试（不会走跨显示器的 DPI 变化流程）。 |
| `CAIRN_SNAPSHOT_DIR=路径` | 运行 `cargo test -p cairn-ui --test headless` 时把各缩放下的无窗口截图写成 PNG。 |

## 写法与状态模型

1. **应用持有状态**：一个普通结构体，例如上面的 `Hello`。
2. **`view` 描述界面**：`fn view(&self) -> impl View<Self>`，根据当前状态返回一棵元素树。元素只是轻量描述，构建很便宜。
3. **控件通过回调通知应用**：`on_click`、`on_change` 等回调直接拿到 `&mut` 状态，应用在回调里改状态。
4. **改完自动更新**：回调返回后，框架重新调用 `view`，把新的元素树和**保留着的控件树**逐个比对——同位置（或同 `key`）、同类型的控件就地更新并保留内部状态（悬停、光标、选区、排版缓存），只有真正变了的控件被标记为需要重新布局或重绘，最后只重画这些区域。

没有信号、`remember` 之类的响应式机制：`view` 是状态的普通函数，状态变了就整体重跑一遍。

和 Compose 的对照：

| Compose | cairn-ui |
| --- | --- |
| `@Composable fun Greeting()` | 返回 `impl View<S>` 的普通函数 / 方法 |
| `Column(Modifier.padding(16.dp)) { … }` | `column().padding(16.0).child(…)` |
| `var n by remember { mutableStateOf(0) }` | 状态结构体里的普通字段 |
| `onClick = { n++ }` | `.on_click(|app: &mut Self| app.n += 1)` |
| 重组（按读取的状态精确重跑） | 整体重跑 `view`，再与保留的控件树比对 |

两点 Rust 带来的差别：

- 内联闭包要写参数类型 `|app: &mut Self|`，这是 Rust 闭包推导的限制；也可以直接传方法：`.on_click(Self::increment)`。
- 回调里只能改状态，不能借用 `&self`；需要的值请 `move` 进闭包（例如列表项的下标）。

### 文本框是“受控”的

`text_input(&self.name)` 传入当前值，`on_change` 把用户的修改写回状态。设置了 `on_change` 时以应用的值为准：界面重建时如果应用给的值和框里的内容不同（例如点“清空名字”），框里的内容会被替换。没设置 `on_change` 时，只有应用给的值本身变了才覆盖。输入法正在组字的文本（预编辑）不属于值，提交后才通过 `on_change` 进入状态。

### 列表与 key

子元素默认按出现顺序与旧控件匹配。列表会增删、重排时给元素加 `.key(id)`，控件状态（比如某个输入框里的光标）就会跟着数据走，而不是跟着位置走。

### 后台线程

```rust
App::new(MyApp::view).run_with(|proxy| MyApp::new(proxy))
```

`proxy.update(|app: &mut MyApp| app.progress = 0.5)` 可以在任何线程调用，修改在主线程执行，随后界面自动重建。

## 控件与布局

| 构建函数 | 主要方法 |
| --- | --- |
| `column()` / `row()` | `child`、`children`、`gap`、`align`（交叉轴）、`justify`（主轴）、`wrap`、`background`、`corner_radius` |
| `label(text)` | `font_size`、`weight` / `bold`、`color`；宽度不够时自动换行 |
| `button(text)` | `on_click`、`enabled` |
| `text_input(value)` | `placeholder`、`on_change`、`on_submit`（回车） |

所有元素都有布局修饰方法（`Styled` trait，单位逻辑像素）：`width`、`height`、`size`、`min_width` / `max_width` / `min_height` / `max_height`、`fill_width` / `fill_height`（占满父容器内容区）、`grow`（分配剩余空间）、`padding`、`margin`、`align_self`、`hidden`、`key`。

`column()` 的子元素默认靠左、不拉伸（与 Compose 一致）；`row()` 的子元素默认纵向居中。外观参数在 `Theme` 里，用 `App::theme` 设置。

文本框支持：←/→（加 Ctrl 按词）、Shift 扩展选区、Home/End、Backspace/Delete（加 Ctrl 按词删除）、Ctrl+A/C/X/V、鼠标点击定位、拖动选择、双击选词、三击全选；光标、删除都以字素簇为单位（emoji、组合字符不会被拆开）。

控件之间：点击可获得焦点的控件获得焦点，点别处失去焦点；Tab / Shift+Tab 按树顺序切换；通过键盘获得焦点的按钮显示焦点框，Enter / 空格触发点击。

写自己的控件：实现 `Widget<S>` trait（`update`、`measure`、`paint`、`event` 等），用 `El::new(widget)` 包成元素。详见 [docs/architecture.md](docs/architecture.md)。

## 工作区结构

```
crates/
├─ cairn-ui-platform/   winit 窗口与事件循环、softbuffer 上屏、IME、DPI、剪贴板、字体目录、系统参数
├─ cairn-ui-render/     Painter trait + tiny-skia 实现、文字排版与字形缓存（cosmic-text）
├─ cairn-ui-layout/     taffy 的薄封装
└─ cairn-ui/            控件树与比对、命中测试、焦点、事件分发、主题、内置控件（用户依赖这个）
examples/
└─ hello/               演示程序
docs/
└─ architecture.md      设计说明、平台相关代码清单、Linux 移植要点
```

`cairn-ui` 依赖下面三个 crate 并按需 re-export，使用者看不到 winit、tiny-skia、taffy、cosmic-text 的类型。下面三个 crate 互相不依赖。

## 体积

`examples/hello`，`cargo build -p hello --release`（`opt-level = "z"`、`lto = "fat"`、`codegen-units = 1`、`panic = "abort"`、`strip = true`）：**2,012,672 字节（1.92 MiB）**。

控制体积的做法：

- 所有依赖关掉默认特性：tiny-skia 去掉 `png-format`（png + flate2 + miniz），cosmic-text 去掉 `fontconfig`，taffy 只留 `flexbox`。
- 不内嵌字体。
- cosmic-text 0.19 自己依赖 skrifa 0.40，而 swash 允许 `>=0.31.1, <=0.44`，Cargo 默认会再拉一份 0.44（连带 read-fonts、font-types 各两份）。本仓库的 `Cargo.lock` 已经统一到 0.40，省约 91 KB。**依赖 cairn-ui 的应用**可以在自己的项目里执行同样的命令：

  ```bash
  cargo update -p skrifa@0.44.0 --precise 0.40.0
  ```

## 验证情况

| 项目 | 状态 |
| --- | --- |
| 单元测试（布局、文本编辑、脏区合并）与无窗口集成测试 | `cargo test --workspace` 全部通过 |
| 无窗口集成测试覆盖 | 点击 / 悬停 / 焦点 / Tab、键盘编辑、选区、剪贴板、回车提交、受控值、按 Windows 上 winit 实际产生的序列模拟输入法事件、候选框位置、光标闪烁、运行中改缩放、各操作的重绘面积 |
| 真实窗口，系统缩放 105%（Windows 11，2560×1440 单显示器） | 运行并截图检查：首帧、点击按钮、文本框输入、调整窗口大小、真实鼠标的悬停 / 按下效果 |
| 125% / 150% | 用 `CAIRN_UI_SCALE_FACTOR` 在真实窗口中运行并截图，另有无窗口截图；**没有**在系统缩放真正设为 125% / 150% 的环境中运行过 |
| 中文输入法 | 作者在 Windows 11 上手动测试，工作正常 |
| 窗口在不同缩放的显示器间移动 | **未验证**（测试机只有一块显示器）；对应代码路径（缩放变化 → 整窗按新缩放重画）有无窗口测试 |
| Linux / macOS | **没有编译过** |

## 已知限制

- 没有滚动容器、图片、列表虚拟化、弹出层、多窗口。
- 文本框没有撤销 / 重做；拖选到边缘时不会自动滚动。
- 只有亮色主题；文字是灰度抗锯齿，混合时未做伽马校正（深底浅字会偏细）。
- 无障碍：控件已有稳定 id（`WidgetId`）和语义信息（`Semantics`），还没接 accesskit。
- 每次状态变化整体重跑 `view`，很大的列表以后需要按需构建或缓存子树。
- 非 Windows 平台的剪贴板与系统参数是占位实现。

## 许可证

MIT 或 Apache-2.0，任选其一：[LICENSE-MIT](LICENSE-MIT)、[LICENSE-APACHE](LICENSE-APACHE)。
