//! 出现与离开带过渡的容器 [`reveal`]，以及透明的可点击区域 [`click_area`]。

use crate::anim::{Easing, Transition};
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::{Affine, Align, CursorIcon, Direction, El, Painter, Point, PointerButton, Rect, Style, Styled};

type Callback<S> = Box<dyn Fn(&mut S)>;

/// 从哪边进来。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Side {
    /// 从下方升起（面板、提示）。
    #[default]
    Below,
    /// 从上方落下（下拉的弹出框）。
    Above,
}

/// 出现时淡入并滑到位、离开时淡出的容器。用 [`reveal`] 创建，子元素纵向排列、横向拉伸。
///
/// 控件被创建后第一次绘制时开始出现：不透明度 0→1，同时从 [`crate::Motion::enter_offset`] 远处
/// 移到原位，时长 [`crate::Motion::enter`]。尺寸始终不变。
/// 设置 [`Reveal::leaving`] 后反向播放（时长 [`crate::Motion::exit`]），播完调用 [`Reveal::on_left`]——
/// 应用在回调里把它从界面里去掉。离开过程中不再接收指针事件。
///
/// 变换和透明度只影响绘制，布局从一开始就是最终位置。列表里给它加 `.key(..)`，
/// 已有的条目在重建时不会重新播放出现动画。
pub struct Reveal<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    from: Side,
    leaving: bool,
    on_left: Option<Callback<S>>,
    t: Transition,
    notified: bool,
    /// 上一帧还在动画中（决定要不要用合成层缓存）。
    moving: bool,
}

pub fn reveal<S: 'static>() -> Reveal<S> {
    Reveal {
        style: Style {
            direction: Direction::Column,
            align_items: Some(Align::Stretch),
            ..Style::default()
        },
        key: None,
        children: Vec::new(),
        from: Side::Below,
        leaving: false,
        on_left: None,
        t: Transition::default(),
        notified: false,
        moving: false,
    }
}

impl<S: 'static> Reveal<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// 从上方落下（例如从按钮下拉出来的弹出框），缩放以顶边为基准。默认从下方升起。
    pub fn from_above(mut self) -> Self {
        self.from = Side::Above;
        self
    }

    /// 开始（或取消）离开。
    pub fn leaving(mut self, leaving: bool) -> Self {
        self.leaving = leaving;
        self
    }

    /// 离开动画播完时调用。
    pub fn on_left(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_left = Some(Box::new(f));
        self
    }

    fn sign(&self) -> f32 {
        match self.from {
            Side::Below => 1.0,
            Side::Above => -1.0,
        }
    }
}

impl<S: 'static> Styled for Reveal<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Reveal<S>> for El<S> {
    fn from(mut r: Reveal<S>) -> Self {
        let style = std::mem::take(&mut r.style);
        let key = r.key.take();
        let children = std::mem::take(&mut r.children);
        let mut el = El::new(r);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for Reveal<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.leaving != self.leaving {
            self.notified = false;
            Change::PAINT
        } else {
            Change::NONE
        };
        self.leaving = new.leaving;
        self.from = new.from;
        self.on_left = new.on_left;
        change
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let d = theme.motion.enter_offset.abs();
        match self.from {
            Side::Below => bounds.outset(0.0, 0.0, d, 0.0),
            Side::Above => bounds.outset(d, 0.0, 0.0, 0.0),
        }
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let m = cx.theme().motion;
        let now = cx.now();
        let (target, duration, easing) = if self.leaving {
            (0.0, m.exit, Easing::EaseOut)
        } else {
            (1.0, m.enter, m.easing)
        };
        self.t.animate_to(target, now, duration, easing);
        self.moving = self.t.is_animating(now);
        if self.moving {
            cx.request_frame();
        } else if self.leaving && !self.notified {
            self.notified = true;
            cx.request_timer(now);
        }
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        let k = self.t.value(cx.now());
        if k >= 1.0 {
            return Affine::IDENTITY;
        }
        // 只平移不缩放：尺寸始终不变，合成层缓存的位图可以按整像素直接贴上去，不用重新采样。
        let m = cx.theme().motion;
        Affine::translate(0.0, self.sign() * m.enter_offset * (1.0 - k))
    }

    fn opacity(&self, cx: &PaintCx) -> f32 {
        self.t.value(cx.now())
    }

    fn cache_layer(&self) -> bool {
        self.moving
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if *event == Event::Timer
            && self.leaving
            && let Some(f) = &self.on_left
        {
            f(cx.app());
        }
    }

    fn intercepts_pointer(&self, _pos: Point, _bounds: Rect, _theme: &Theme) -> bool {
        self.leaving
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Group,
            ..Semantics::default()
        }
    }
}

/// 没有外观的可点击区域。用 [`click_area`] 创建，子元素纵向排列（与 [`crate::column`] 相同）。
///
/// 在它本身、或不处理点击的子元素上按下并松开时调用 [`ClickArea::on_click`]。典型用法：弹出框后面
/// 铺一层盖满窗口的 `click_area()`（`.absolute()` 加四边为 0），点弹出框外面任何地方都关闭弹出框。
/// 悬停不会触发重绘。
pub struct ClickArea<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    on_click: Option<Callback<S>>,
    pressed: bool,
}

pub fn click_area<S: 'static>() -> ClickArea<S> {
    ClickArea {
        style: Style {
            direction: Direction::Column,
            align_items: Some(Align::Start),
            ..Style::default()
        },
        key: None,
        children: Vec::new(),
        on_click: None,
        pressed: false,
    }
}

impl<S: 'static> ClickArea<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl<S: 'static> Styled for ClickArea<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<ClickArea<S>> for El<S> {
    fn from(mut c: ClickArea<S>) -> Self {
        let style = std::mem::take(&mut c.style);
        let key = c.key.take();
        let children = std::mem::take(&mut c.children);
        let mut el = El::new(c);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for ClickArea<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        self.on_click = new.on_click;
        Change::NONE
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        match event {
            Event::PointerDown {
                button: PointerButton::Left,
                ..
            } => {
                self.pressed = true;
                cx.set_handled();
            }
            Event::PointerUp {
                pos,
                button: PointerButton::Left,
            } => {
                let click = std::mem::take(&mut self.pressed) && cx.bounds().contains(*pos);
                cx.set_handled();
                if click && let Some(f) = &self.on_click {
                    f(cx.app());
                }
            }
            _ => {}
        }
    }

    fn cursor(&self) -> CursorIcon {
        CursorIcon::Default
    }
}
