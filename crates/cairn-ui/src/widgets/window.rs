//! 关闭系统标题栏（[`crate::App::decorations`]）后自己画标题栏用的两个部件：
//! 可拖动窗口的区域 [`drag_area`]，以及最小化 / 最大化 / 关闭按钮 [`window_controls`]。

use cairn_ui_layout::MeasureInput;

use crate::anim::Transition;
use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::drive;
use crate::widgets::icon::{Icon, paint_icon};
use crate::{Align, Direction, El, Justify, Painter, Point, PointerButton, Rect, Size, Style, Styled};

/// 可拖动窗口的区域。用 [`drag_area`] 创建，子元素横向排列、纵向居中（与 [`crate::row`] 相同）。
///
/// 在它本身、或者在不处理按下事件的子元素（文字、图标）上按下左键会拖动窗口（系统的移动循环，
/// 可以拖到屏幕边缘贴靠）；双击切换最大化。按钮、输入框这类自己处理按下事件的子元素照常工作。
pub struct DragArea<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
}

pub fn drag_area<S: 'static>() -> DragArea<S> {
    DragArea {
        style: Style {
            direction: Direction::Row,
            align_items: Some(Align::Center),
            ..Style::default()
        },
        key: None,
        children: Vec::new(),
    }
}

impl<S: 'static> DragArea<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = gap;
        self
    }

    pub fn justify(mut self, justify: Justify) -> Self {
        self.style.justify = justify;
        self
    }
}

impl<S: 'static> Styled for DragArea<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<DragArea<S>> for El<S> {
    fn from(mut d: DragArea<S>) -> Self {
        let style = std::mem::take(&mut d.style);
        let key = d.key.take();
        let children = std::mem::take(&mut d.children);
        let mut el = El::new(d);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for DragArea<S> {
    fn update(&mut self, _cx: &mut LayoutCx, _new: Self) -> Change {
        Change::NONE
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        // 不标记为已处理：系统的移动循环会吞掉松开事件，标记了的话运行时会一直等这次按下的松开，
        // 把之后的指针事件都独占给这里。
        if let Event::PointerDown {
            button: PointerButton::Left,
            count,
            ..
        } = event
        {
            if *count >= 2 {
                cx.toggle_maximize_window();
            } else {
                cx.drag_window();
            }
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Group,
            ..Semantics::default()
        }
    }
}

/// 窗口按钮：最小化、最大化（最大化时显示为“还原”）、关闭，从左到右排成一行。
/// 用 [`window_controls`] 创建，尺寸和外观来自主题的 [`Theme::caption`]。
///
/// 和系统的窗口按钮一样不参与 Tab 焦点切换。关闭按钮调用 [`EventCx::close_window`]，即结束 [`crate::App::run`]。
pub struct WindowControls {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    /// 指针下的按钮、按下的按钮（0 最小化，1 最大化 / 还原，2 关闭）。
    hovered: Option<usize>,
    pressed: Option<usize>,
    fade: [Transition; 3],
}

pub fn window_controls() -> WindowControls {
    WindowControls {
        style: Style::default(),
        key: None,
        hovered: None,
        pressed: None,
        fade: Default::default(),
    }
}

element!(WindowControls);

impl WindowControls {
    fn index_at(&self, bounds: Rect, pos: Point, theme: &Theme) -> Option<usize> {
        if !bounds.contains(pos) {
            return None;
        }
        Some((((pos.x - bounds.x) / theme.caption.width) as usize).min(2))
    }
}

impl<S: 'static> Widget<S> for WindowControls {
    fn update(&mut self, _cx: &mut LayoutCx, _new: Self) -> Change {
        Change::NONE
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let c = &cx.theme().caption;
        Size::new(c.width * 3.0, c.height)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let m = cx.theme().motion;
        // 悬停底色出现得快、消失得慢一点，和系统按钮的手感接近。
        for (i, t) in self.fade.iter_mut().enumerate() {
            drive(t, cx, self.hovered == Some(i), m.press, m.easing);
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let c = &theme.caption;
        let b = cx.bounds();
        let icons = [
            Icon::WindowMinimize,
            if cx.window_maximized() {
                Icon::WindowRestore
            } else {
                Icon::WindowMaximize
            },
            Icon::Close,
        ];
        for (i, icon) in icons.into_iter().enumerate() {
            let cell = Rect::new(b.x + c.width * i as f32, b.y, c.width, c.height);
            let h = self.fade[i].value(cx.now());
            let pressed = self.pressed == Some(i) && self.hovered == Some(i);
            let (fill, fill_pressed, icon_hover) = if i == 2 {
                (c.close_fill_hover, c.close_fill_pressed, c.close_icon_hover)
            } else {
                (c.fill_hover, c.fill_pressed, c.icon_hover)
            };
            if h > 0.0 {
                let f = if pressed { fill_pressed } else { fill };
                p.fill_rounded_rect(cell, c.radius, f.fade(h));
            }
            let s = c.icon_size;
            let r = Rect::new(cell.x + (cell.width - s) / 2.0, cell.y + (cell.height - s) / 2.0, s, s);
            paint_icon(p, icon, r, c.icon.mix(icon_hover, h));
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        let theme = cx.theme();
        match event {
            Event::PointerMove { pos } => {
                let i = self.index_at(cx.bounds(), *pos, theme);
                if i != self.hovered {
                    self.hovered = i;
                    cx.request_paint();
                }
            }
            Event::PointerLeave => {
                self.hovered = None;
                cx.request_paint();
            }
            Event::PointerDown {
                pos,
                button: PointerButton::Left,
                ..
            } => {
                self.pressed = self.index_at(cx.bounds(), *pos, theme);
                cx.request_paint();
                cx.set_handled();
            }
            Event::PointerUp {
                pos,
                button: PointerButton::Left,
            } => {
                let target = self.pressed.take();
                cx.request_paint();
                cx.set_handled();
                if target.is_some() && target == self.index_at(cx.bounds(), *pos, theme) {
                    // 最小化 / 最大化之后指针多半已经不在按钮上，先清掉悬停，等下一次移动再算。
                    self.hovered = None;
                    match target {
                        Some(0) => cx.minimize_window(),
                        Some(1) => cx.toggle_maximize_window(),
                        _ => cx.close_window(),
                    }
                }
            }
            _ => {}
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Group,
            label: Some("window controls".into()),
            ..Semantics::default()
        }
    }
}
