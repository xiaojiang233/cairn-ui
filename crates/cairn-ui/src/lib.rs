//! # cairn-ui
//!
//! 轻量、保留式的 Rust GUI 库。
//!
//! ## 写法
//!
//! 应用状态是一个普通结构体；`view` 方法根据状态描述界面，控件回调直接拿到 `&mut` 状态去修改：
//!
//! ```no_run
//! use cairn_ui::prelude::*;
//!
//! #[derive(Default)]
//! struct Hello { clicks: u32 }
//!
//! impl Hello {
//!     fn view(&self) -> impl View<Self> {
//!         column()
//!             .padding(24.0)
//!             .gap(12.0)
//!             .child(label(format!("点了 {} 次", self.clicks)))
//!             .child(button("点我").on_click(|app: &mut Self| app.clicks += 1))
//!     }
//! }
//!
//! fn main() -> cairn_ui::Result<()> {
//!     App::new(Hello::view).title("hello").run(Hello::default())
//! }
//! ```
//!
//! ## 状态与界面怎么连接
//!
//! 1. 应用持有全部状态（`Hello`）。
//! 2. 控件通过回调通知应用（`on_click`、`on_change` 收到 `&mut Hello`），应用在回调里直接改状态。
//! 3. 回调返回后，框架重新调用 `view` 得到新的元素描述，与**保留着的控件树**逐个比对：
//!    同位置（或同 [`Styled::key`]）、同类型的控件就地更新并保留内部状态（悬停、光标、选区、排版缓存），
//!    只有真正变了的控件被标记为需要重新布局或重绘，最后只重画这些区域。
//! 4. 后台线程用 [`Proxy::update`] 投递修改，效果同第 2、3 步（见 [`App::run_with`]）。
//!
//! 没有信号、`remember` 之类的响应式机制：`view` 是状态的普通函数，每次状态变化后整体重跑一遍。
//! 元素只是轻量的描述，重跑很便宜；昂贵的工作（排版、布局、绘制）只发生在变化的控件上。
//!
//! 闭包参数需要写类型（`|app: &mut Self| ...`），这是 Rust 类型推导的限制；
//! 也可以传方法：`.on_click(Self::increment)`。
//!
//! ## 坐标与缩放
//!
//! 所有对外接口用逻辑像素。渲染按物理像素进行：矩形边缘对齐到像素网格，文字按
//! “逻辑字号 × 缩放因子”光栅化。窗口移到缩放不同的显示器上时自动按新缩放重画。

mod anim;
mod app;
mod damage;
mod element;
mod host;
mod text_edit;
mod theme;
mod tree;
mod ui;
mod widget;
mod widgets;

pub mod testing;

pub use anim::{Easing, Transition};
pub use app::{App, Proxy, ViewFn};
pub use element::{El, Styled, View};
pub use theme::{
    ButtonStyle, FocusRing, IconButtonStyle, InputStyle, Motion, PanelStyle, Radii, ScrollStyle, SliderStyle, Spacing,
    Theme, TypeStyle, Typography,
};
pub use tree::WidgetId;
pub use widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
pub use widgets::{
    Button, Flex, Icon, IconButton, Label, Panel, Scroll, Slider, TextInput, TextRole, button, column, icon_button,
    label, paint_icon, panel, row, scroll, slider, text_input,
};

pub use cairn_ui_layout::{Align, AvailableSpace, Direction, Insets, Justify, Length, MeasureInput, Style};
pub use cairn_ui_platform::{
    CursorIcon, ImeEvent, Key, KeyEvent, Modifiers, NamedKey, PixelRect, PointerButton, ScrollDelta,
};
pub use cairn_ui_render::{
    Affine, BackdropStyle, Background, Brush, Canvas, Color, DeviceRect, FontFamily, FontSetup, GradientStop, Image,
    ImageScene, NightScene, Painter, Point, Rect, Scene, Shadow, Size, Spot, TextLayout, TextStyle, TextSystem,
};

/// 常用导入：`use cairn_ui::prelude::*;`
pub mod prelude {
    pub use crate::{
        Align, App, Color, El, Icon, Insets, Justify, Proxy, Styled, Theme, View, button, column, icon_button, label,
        panel, row, scroll, slider, text_input,
    };
}

/// cairn-ui 的错误类型。
#[derive(Debug)]
pub struct Error(cairn_ui_platform::Error);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<cairn_ui_platform::Error> for Error {
    fn from(e: cairn_ui_platform::Error) -> Self {
        Error(e)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
