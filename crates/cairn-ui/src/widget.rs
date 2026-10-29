//! 控件 trait 与回调上下文。
//!
//! 一般使用者只需要 [`crate::label`]、[`crate::button`] 这些现成控件；
//! 想写自己的控件时实现 [`Widget`]，再用 [`crate::El::new`] 包成元素。

use std::any::{Any, TypeId};
use std::time::{Duration, Instant};

use crate::host::Host;
use crate::theme::Theme;
use crate::{
    Affine, CursorIcon, ImeEvent, KeyEvent, Modifiers, Painter, Point, PointerButton, Rect, ScrollDelta, Size, Style,
    TextSystem,
};
use cairn_ui_layout::MeasureInput;

/// 控件更新后需要做的工作。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// 尺寸可能变了，需要重新测量和布局（隐含重绘）。
    pub layout: bool,
    /// 外观变了，需要重绘。
    pub paint: bool,
}

impl Change {
    pub const NONE: Change = Change {
        layout: false,
        paint: false,
    };
    pub const PAINT: Change = Change {
        layout: false,
        paint: true,
    };
    pub const LAYOUT: Change = Change {
        layout: true,
        paint: true,
    };

    pub fn is_none(&self) -> bool {
        !self.layout && !self.paint
    }
}

impl std::ops::BitOr for Change {
    type Output = Change;
    fn bitor(self, o: Change) -> Change {
        Change {
            layout: self.layout || o.layout,
            paint: self.paint || o.paint,
        }
    }
}

impl std::ops::BitOrAssign for Change {
    fn bitor_assign(&mut self, o: Change) {
        *self = *self | o;
    }
}

/// 分发给控件的事件。坐标是逻辑像素、窗口坐标系。
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// 指针进入 / 离开本控件或它的任何子元素（与 DOM 的 `mouseenter` / `mouseleave` 相同：
    /// 指针从父控件移到子控件上时，父控件**不会**收到 `PointerLeave`）。
    /// 框架不会因此自动重绘，有悬停效果的控件应在这里调用 [`EventCx::request_paint`]。
    PointerEnter,
    PointerLeave,
    PointerMove {
        pos: Point,
    },
    /// `count` 是连击次数：1 单击，2 双击，3 三击……
    PointerDown {
        pos: Point,
        button: PointerButton,
        count: u32,
    },
    PointerUp {
        pos: Point,
        button: PointerButton,
    },
    Wheel {
        delta: ScrollDelta,
    },
    Key(KeyEvent),
    Ime(ImeEvent),
    FocusGained,
    FocusLost,
    /// 窗口本身获得 / 失去焦点（只发给当前有焦点的控件）。
    WindowFocusChanged(bool),
    /// [`EventCx::request_timer`] 请求的时间到了。
    Timer,
}

/// 无障碍语义角色。现在还没有接入 accesskit，先在控件上把信息备好。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Role {
    #[default]
    Generic,
    Group,
    Label,
    Button,
    TextInput,
    Slider,
    /// 滚动容器（[`crate::scroll`]）。
    ScrollArea,
    Checkbox,
    Tab,
    ProgressBar,
    ListItem,
}

/// 控件的语义信息，用于以后接入无障碍接口，也方便测试里按文字查找控件。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Semantics {
    pub role: Role,
    /// 名称：按钮文字、标签文字、输入框的占位提示等。
    pub label: Option<String>,
    /// 值：输入框的内容、滑块的数值等。
    pub value: Option<String>,
    pub disabled: bool,
}

/// 保留式控件。
///
/// 同一个控件实例在多次 `view` 调用之间一直存在（由 [`crate::WidgetId`] 标识）；
/// 每次重新构建界面时，新生成的同类型描述通过 [`Widget::update`] 合并进来。
pub trait Widget<S: 'static>: Any {
    /// 用新的描述更新自己：拷贝外部给定的属性，保留内部状态（悬停、动画、光标位置、排版缓存等），
    /// 并报告需要重新布局还是只需重绘。
    fn update(&mut self, cx: &mut LayoutCx, new: Self) -> Change
    where
        Self: Sized;

    /// 布局前按主题补全样式里的占位值，例如内边距是 [`crate::Insets::AUTO`] 时换成主题给的默认值。
    /// 剩下没补的占位值按 0 处理。
    fn resolve_style(&self, _style: &mut Style, _theme: &Theme) {}

    /// 测量内容尺寸（不含内边距）。只对没有子元素的控件调用。
    fn measure(&mut self, _cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        Size::ZERO
    }

    /// 每次布局完成后调用，告知最终的边框盒与内容区（窗口坐标）。
    fn arrange(&mut self, _cx: &mut LayoutCx, _bounds: Rect, _content: Rect) {}

    /// 紧接在 [`Widget::arrange`] 之后、子元素定位之前调用（只对有子元素的控件）：告知子元素占据的范围，
    /// 供滚动容器据此限制滚动量。
    ///
    /// `content_size` 是从本控件边框盒左上角算起的范围，与 CSS 的 `scrollWidth` / `scrollHeight` 相同：
    /// 每个可见子元素的“布局位置 + 尺寸 + 右 / 下外边距”的最大值，再加上本控件自己的右 / 下内边距。
    /// 窗口尺寸或内容变化后都会重新调用。
    fn arrange_content(&mut self, _cx: &mut LayoutCx, _content_size: Size) {}

    /// 所有子元素的整体平移量：子元素的窗口位置 = 布局位置 − 这个值。滚动容器返回当前的滚动量。
    ///
    /// 这平移的是子元素**真实的**边框盒，不是只改绘制：命中测试、事件坐标、输入法候选框、焦点框和脏区
    /// 因此始终一致。滚动量改变后用 [`EventCx::request_arrange`] 通知框架重新定位子元素。
    fn child_offset(&self) -> Point {
        Point::ZERO
    }

    /// 本控件在任何状态下可能画到的范围（窗口坐标）：阴影、外发光、焦点框、悬停上浮都要算进去。
    /// 用于计算脏区和跳过画不到的控件。默认是边框盒（框架另外留了 4 像素余量给抗锯齿和焦点框）。
    fn visual_bounds(&self, bounds: Rect, _theme: &Theme) -> Rect {
        bounds
    }

    /// 绘制之前调用，先于 [`Widget::transform`] 和 [`Widget::paint`]：按 [`PaintCx`] 里的当前状态
    /// （悬停、按下、焦点……）重设过渡的目标，还在过渡中就 [`PaintCx::request_frame`]。
    ///
    /// 过渡的目标不在事件里设，而是在这里按状态推导，所以状态由什么引起都不会漏掉；
    /// 又因为先于 `transform` 和 `paint`，它们读到的就是本帧最新的过渡值——时长为 0
    /// （系统关闭了动画）时，状态变化后的第一帧就已经是终态，不会要等下一帧。
    fn animate(&mut self, _cx: &mut PaintCx) {}

    /// 本控件及其子元素的绘制变换（悬停上浮、按下缩小），默认不变换。只影响绘制，不影响布局和点击判定。
    /// 只读取 [`Widget::animate`] 已经更新过的过渡值。
    fn transform(&self, _cx: &PaintCx) -> Affine {
        Affine::IDENTITY
    }

    /// 本控件及其子元素整体的不透明度（0–1），用于淡入淡出，默认 1。小于 1 时整棵子树先画进离屏图层、
    /// 再按这个值混合，叠在一起的子元素不会互相透出。只影响绘制，不影响布局和点击判定。
    /// 只读取 [`Widget::animate`] 已经更新过的过渡值。
    fn opacity(&self, _cx: &PaintCx) -> f32 {
        1.0
    }

    /// 返回 `true` 时，本控件及其子树被画成一张位图缓存起来，之后的帧只把这张位图按
    /// [`Widget::transform`]、[`Widget::opacity`] 合成上去，不再重画里面的内容——
    /// 整块面板淡入、滑入这类大面积动画因此每帧只需一次贴图。
    /// 子树内容有任何变化（状态更新、重绘请求、换主题）时缓存自动重画。只应在动画进行时返回 `true`，
    /// 静止时缓存的意义不大，还会占内存。
    fn cache_layer(&self) -> bool {
        false
    }

    /// 返回 `Some(圆角半径)` 时，子元素被裁剪到本控件的边框盒内（绘制和点击判定都是）。
    fn clip_children(&self, _theme: &Theme) -> Option<f32> {
        None
    }

    /// 返回 `Some(h)` 时，布局引擎把本节点的高度固定为 `h`（不增长、不收缩），子元素超出的部分
    /// 由 [`Widget::clip_children`] 裁掉。用于高度随动画变化的容器（[`crate::collapse`]）：它每帧返回新的高度，
    /// 并通过定时器事件请求重新布局，下面的兄弟元素就会跟着移动。默认 `None`，高度由样式和子元素决定。
    fn animated_height(&self) -> Option<f32> {
        None
    }

    /// 窗口坐标 `pos`（已在 `bounds` 内）处的指针是否归本控件，而不是它下面的子元素：返回 `true`
    /// 时命中测试在本控件停下，不再看子元素。用于画在子元素上面的东西（滚动条）。
    fn intercepts_pointer(&self, _pos: Point, _bounds: Rect, _theme: &Theme) -> bool {
        false
    }

    /// 画自己（在子元素下面）。
    fn paint(&mut self, cx: &mut PaintCx, painter: &mut dyn Painter);

    /// 子元素画完之后调用，画盖在子元素上面的东西（滚动条等）。
    fn paint_over(&mut self, _cx: &mut PaintCx, _painter: &mut dyn Painter) {}

    fn event(&mut self, _cx: &mut EventCx<S>, _event: &Event) {}

    /// 能否获得键盘焦点（点击或 Tab）。
    fn focusable(&self) -> bool {
        false
    }

    /// 获得焦点时是否打开输入法。
    fn wants_ime(&self) -> bool {
        false
    }

    /// 输入法候选框应当避开的区域（窗口坐标），通常是光标所在的那一行。
    fn ime_area(&self, _bounds: Rect, _content: Rect) -> Option<Rect> {
        None
    }

    fn cursor(&self) -> CursorIcon {
        CursorIcon::Default
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 类型擦除后的控件：在不知道具体类型的情况下完成“同类型就更新，否则重建”。
pub(crate) trait AnyWidget<S: 'static>: Widget<S> {
    fn update_any(&mut self, cx: &mut LayoutCx, new: Box<dyn AnyWidget<S>>) -> Result<Change, Box<dyn AnyWidget<S>>>;
}

impl<S: 'static, W: Widget<S>> AnyWidget<S> for W {
    fn update_any(&mut self, cx: &mut LayoutCx, new: Box<dyn AnyWidget<S>>) -> Result<Change, Box<dyn AnyWidget<S>>> {
        // 注意要对解引用后的 trait 对象取 type_id，而不是对 Box 本身。
        if (*new).type_id() != TypeId::of::<W>() {
            return Err(new);
        }
        let new: Box<dyn Any> = new;
        let new = new.downcast::<W>().expect("类型已检查");
        Ok(self.update(cx, *new))
    }
}

/// 更新、测量、布局完成时可用的上下文。
pub struct LayoutCx<'a> {
    pub(crate) text: &'a mut TextSystem,
    pub(crate) theme: &'a Theme,
}

impl<'a> LayoutCx<'a> {
    pub fn text(&mut self) -> &mut TextSystem {
        self.text
    }

    pub fn theme(&self) -> &'a Theme {
        self.theme
    }
}

/// 绘制时可用的上下文。
pub struct PaintCx<'a> {
    pub(crate) bounds: Rect,
    pub(crate) content: Rect,
    pub(crate) hovered: bool,
    pub(crate) focused: bool,
    pub(crate) focus_visible: bool,
    pub(crate) window_focused: bool,
    pub(crate) window_maximized: bool,
    pub(crate) theme: &'a Theme,
    pub(crate) now: Instant,
    pub(crate) frame_requested: bool,
    pub(crate) timer: Option<Instant>,
}

impl<'a> PaintCx<'a> {
    /// 边框盒（窗口坐标）。
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// 内容区：边框盒减去内边距。
    pub fn content(&self) -> Rect {
        self.content
    }

    /// 指针在本控件或它的子元素上。
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// 焦点是通过键盘（Tab）得到的，应当画焦点框。
    pub fn focus_visible(&self) -> bool {
        self.focused && self.focus_visible
    }

    pub fn window_focused(&self) -> bool {
        self.window_focused
    }

    /// 窗口处于最大化状态（窗口按钮据此显示“最大化”或“还原”）。
    pub fn window_maximized(&self) -> bool {
        self.window_maximized
    }

    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// 本帧的时间点，动画按它取值。同一帧里所有控件拿到的是同一个值。
    pub fn now(&self) -> Instant {
        self.now
    }

    /// 动画还没结束：请框架在下一帧（按显示器刷新率）重画本控件。
    /// 每帧都要重新请求；不再请求时动画帧自然停止，事件循环回到空闲等待。
    pub fn request_frame(&mut self) {
        self.frame_requested = true;
    }

    /// 在 `at` 时刻给本控件发一次 [`Event::Timer`]。动画在绘制时推进、却要在结束时通知应用
    /// （例如退场动画播完后调用回调）时用：事件里才能调用应用的回调。
    pub fn request_timer(&mut self, at: Instant) {
        self.timer = Some(at);
    }
}

/// 事件处理时控件对框架的请求，由运行时在分发后统一处理。
#[derive(Default)]
pub(crate) struct EventOutput {
    pub handled: bool,
    pub change: Change,
    pub damage: Vec<Rect>,
    pub app_touched: bool,
    pub request_focus: bool,
    pub timer: Option<Instant>,
    /// 子元素的位置需要重新计算（滚动量变了），但不需要重新测量。
    pub arrange: bool,
}

/// 事件处理时可用的上下文。
pub struct EventCx<'a, S> {
    pub(crate) bounds: Rect,
    pub(crate) content: Rect,
    pub(crate) state: &'a mut S,
    pub(crate) text: &'a mut TextSystem,
    pub(crate) theme: &'a Theme,
    pub(crate) host: &'a mut dyn Host,
    pub(crate) focused: bool,
    pub(crate) captured: bool,
    pub(crate) window_focused: bool,
    pub(crate) modifiers: Modifiers,
    pub(crate) scale: f32,
    pub(crate) blink: Option<Duration>,
    pub(crate) now: Instant,
    pub(crate) out: EventOutput,
}

impl<'a, S> EventCx<'a, S> {
    /// 访问应用状态，通常用来调用应用传进来的回调。调用后框架会重新构建界面。
    pub fn app(&mut self) -> &mut S {
        self.out.app_touched = true;
        self.state
    }

    /// 标记事件已处理，不再冒泡给父控件。
    pub fn set_handled(&mut self) {
        self.out.handled = true;
    }

    /// 本控件需要重绘（重画 [`Widget::visual_bounds`] 范围）。
    pub fn request_paint(&mut self) {
        self.out.change.paint = true;
    }

    /// 只重绘一小块区域（窗口坐标），例如光标闪烁。
    pub fn request_paint_rect(&mut self, rect: Rect) {
        self.out.damage.push(rect);
    }

    /// 本控件的尺寸可能变了。
    pub fn request_layout(&mut self) {
        self.out.change = Change::LAYOUT;
    }

    /// 子元素的位置需要重新计算，但尺寸没变、不用重新测量（例如 [`Widget::child_offset`] 变了）。
    /// 比 [`EventCx::request_layout`] 轻得多：不跑布局引擎，只重新定位各节点并记录移动了的区域。
    /// 不隐含重绘，需要的话另外调用 [`EventCx::request_paint`]。
    pub fn request_arrange(&mut self) {
        self.out.arrange = true;
    }

    pub fn request_focus(&mut self) {
        self.out.request_focus = true;
    }

    /// 在 `at` 时刻收到一次 [`Event::Timer`]。每个控件只保留最近一次请求。
    pub fn request_timer(&mut self, at: Instant) {
        self.out.timer = Some(at);
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    pub fn content(&self) -> Rect {
        self.content
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// 本控件正独占指针事件：它处理了按下事件，到松开之前所有指针事件都先交给它。
    /// 拖动中的控件可以据此发现独占已经丢了（例如窗口失去焦点，松开事件不会再来）。
    /// 注意松开事件到达时独占已经解除，这里返回 `false`。
    pub fn is_captured(&self) -> bool {
        self.captured
    }

    pub fn window_focused(&self) -> bool {
        self.window_focused
    }

    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// 当前时间。动画、定时器都应以它为准（测试里是可控的虚拟时钟）。
    pub fn now(&self) -> Instant {
        self.now
    }

    /// 系统设置的文本光标闪烁间隔。`None` 表示不闪烁。
    pub fn caret_blink_interval(&self) -> Option<Duration> {
        self.blink
    }

    /// 动画帧间隔（显示器刷新间隔）。用 [`EventCx::request_timer`] 逐帧驱动动画时，
    /// 下一帧排在 `now() + frame_interval()`。
    pub fn frame_interval(&self) -> Duration {
        self.host.frame_interval()
    }

    pub fn text(&mut self) -> &mut TextSystem {
        self.text
    }

    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// 开始用鼠标拖动窗口。用于关闭了系统标题栏（[`crate::App::decorations`]）时自己画的标题栏，
    /// 在左键按下的事件里调用。
    pub fn drag_window(&mut self) {
        self.host.drag_window();
    }

    pub fn minimize_window(&mut self) {
        self.host.minimize_window();
    }

    /// 最大化与还原之间切换。
    pub fn toggle_maximize_window(&mut self) {
        self.host.toggle_maximize_window();
    }

    pub fn is_window_maximizable(&self) -> bool {
        self.host.is_window_maximizable()
    }

    pub fn is_window_maximized(&self) -> bool {
        self.host.is_window_maximized()
    }

    /// 关闭窗口，结束 [`crate::App::run`]。
    pub fn close_window(&mut self) {
        self.host.exit();
    }

    pub fn clipboard_get(&mut self) -> Option<String> {
        self.host.clipboard_get()
    }

    pub fn clipboard_set(&mut self, text: &str) {
        self.host.clipboard_set(text);
    }

    pub(crate) fn layout_cx(&mut self) -> LayoutCx<'_> {
        LayoutCx {
            text: &mut *self.text,
            theme: self.theme,
        }
    }
}
