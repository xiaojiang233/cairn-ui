//! 控件 trait 与回调上下文。
//!
//! 一般使用者只需要 [`crate::label`]、[`crate::button`] 这些现成控件；
//! 想写自己的控件时实现 [`Widget`]，再用 [`crate::El::new`] 包成元素。

use std::any::{Any, TypeId};
use std::time::{Duration, Instant};

use crate::host::Host;
use crate::theme::Theme;
use crate::{
    CursorIcon, ImeEvent, KeyEvent, Modifiers, Painter, Point, PointerButton, Rect, ScrollDelta, Size, TextSystem,
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
    /// 指针移入 / 移出本控件（只发给最深层的那个控件）。框架不会因此自动重绘，
    /// 有悬停效果的控件应在这里调用 [`EventCx::request_paint`]。
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
}

/// 控件的语义信息，用于以后接入无障碍接口，也方便测试里按文字查找控件。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Semantics {
    pub role: Role,
    /// 名称：按钮文字、标签文字、输入框的占位提示等。
    pub label: Option<String>,
    /// 值：输入框的内容等。
    pub value: Option<String>,
    pub disabled: bool,
}

/// 保留式控件。
///
/// 同一个控件实例在多次 `view` 调用之间一直存在（由 [`crate::WidgetId`] 标识）；
/// 每次重新构建界面时，新生成的同类型描述通过 [`Widget::update`] 合并进来。
pub trait Widget<S: 'static>: Any {
    /// 用新的描述更新自己：拷贝外部给定的属性，保留内部状态（悬停、光标位置、排版缓存等），
    /// 并报告需要重新布局还是只需重绘。
    fn update(&mut self, cx: &mut LayoutCx, new: Self) -> Change
    where
        Self: Sized;

    /// 测量内容尺寸（不含内边距）。只对没有子元素的控件调用。
    fn measure(&mut self, _cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        Size::ZERO
    }

    /// 每次布局完成后调用，告知最终的边框盒与内容区（窗口坐标）。
    fn arrange(&mut self, _cx: &mut LayoutCx, _bounds: Rect, _content: Rect) {}

    fn paint(&mut self, cx: &mut PaintCx, painter: &mut dyn Painter);

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

impl LayoutCx<'_> {
    pub fn text(&mut self) -> &mut TextSystem {
        self.text
    }

    pub fn theme(&self) -> &Theme {
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
    pub(crate) theme: &'a Theme,
}

impl PaintCx<'_> {
    /// 边框盒（窗口坐标）。
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// 内容区：边框盒减去内边距。
    pub fn content(&self) -> Rect {
        self.content
    }

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

    pub fn theme(&self) -> &Theme {
        self.theme
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
    pub(crate) window_focused: bool,
    pub(crate) modifiers: Modifiers,
    pub(crate) scale: f32,
    pub(crate) blink: Option<Duration>,
    pub(crate) out: EventOutput,
}

impl<S> EventCx<'_, S> {
    /// 访问应用状态，通常用来调用应用传进来的回调。调用后框架会重新构建界面。
    pub fn app(&mut self) -> &mut S {
        self.out.app_touched = true;
        self.state
    }

    /// 标记事件已处理，不再冒泡给父控件。
    pub fn set_handled(&mut self) {
        self.out.handled = true;
    }

    /// 本控件需要重绘。
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

    pub fn window_focused(&self) -> bool {
        self.window_focused
    }

    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// 系统设置的文本光标闪烁间隔。`None` 表示不闪烁。
    pub fn caret_blink_interval(&self) -> Option<Duration> {
        self.blink
    }

    pub fn text(&mut self) -> &mut TextSystem {
        self.text
    }

    pub fn theme(&self) -> &Theme {
        self.theme
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
