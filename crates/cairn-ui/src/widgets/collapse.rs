//! 展开 / 收起带高度过渡的容器 [`collapse`]。

use crate::anim::{Easing, Transition};
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::{Align, Direction, El, Painter, Rect, Size, Style, Styled};

type Callback<S> = Box<dyn Fn(&mut S)>;

/// 展开 / 收起时高度平滑变化的容器，用于下拉列表、折叠面板。用 [`collapse`] 创建，
/// 子元素纵向排列、横向拉伸。
///
/// `open` 为 `true` 时高度从 0 长到子元素的完整高度，同时淡入；为 `false` 时反向收起。高度变化是真的
/// 参与布局：它下面的兄弟元素随着一起移动，而不是等动画播完才跳一下。展开完成之后高度重新交给子元素决定，
/// 内容再有增减（例如异步加载出更多条目）也会正常撑开。
///
/// 收起时子元素仍然留在界面树里：应用要在收起动画结束（[`Collapse::on_collapsed`]）之后再把它从
/// `view` 里去掉，否则收起的过程就看不到了。新创建的容器如果 `open` 已经是 `true`，也会播一遍展开动画。
/// 时长取主题的 [`crate::Motion::expand`]，收起比展开快 25%。
pub struct Collapse<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    open: bool,
    on_collapsed: Option<Callback<S>>,
    t: Transition,
    /// 子元素完整展开时的高度。每次布局后由 `arrange_content` 更新。
    natural: f32,
    /// 上一次绘制时的展开比例（0–1）；下一次布局用它算高度。
    k: f32,
    /// 收起完成的通知已经发过了。
    notified: bool,
}

pub fn collapse<S: 'static>(open: bool) -> Collapse<S> {
    Collapse {
        style: Style {
            direction: Direction::Column,
            align_items: Some(Align::Stretch),
            ..Style::default()
        },
        key: None,
        children: Vec::new(),
        open,
        on_collapsed: None,
        t: Transition::default(),
        natural: 0.0,
        k: 0.0,
        notified: false,
    }
}

impl<S: 'static> Collapse<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// 收起动画播完时调用。应用在回调里把它从界面里去掉。
    pub fn on_collapsed(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_collapsed = Some(Box::new(f));
        self
    }

    /// 动画还没结束，或者已经收起：高度由动画决定。
    fn height_driven(&self) -> bool {
        !(self.open && self.k >= 1.0)
    }
}

impl<S: 'static> Styled for Collapse<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Collapse<S>> for El<S> {
    fn from(mut c: Collapse<S>) -> Self {
        let style = std::mem::take(&mut c.style);
        let key = c.key.take();
        let children = std::mem::take(&mut c.children);
        // 子元素放进一个不收缩的内层容器：外层高度被压到 0 时，内层仍然保持完整高度，
        // 外层从 `arrange_content` 里就能知道“完全展开要多高”。
        let inner: El<S> = crate::column()
            .align(Align::Stretch)
            .shrink(0.0)
            .children(children)
            .into();
        let mut el = El::new(c);
        el.style = style;
        el.key = key;
        el.children = vec![inner];
        el
    }
}

impl<S: 'static> Widget<S> for Collapse<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        if new.open != self.open {
            self.notified = false;
        }
        let change = if new.open != self.open {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.open = new.open;
        self.on_collapsed = new.on_collapsed;
        change
    }

    fn arrange_content(&mut self, _cx: &mut LayoutCx, content: Size) {
        self.natural = content.height;
    }

    /// 收起时高度是 0，空矩形不会产生脏区，动画也就没机会开始（动画是在绘制时推进的）；至少留 1 像素高。
    fn visual_bounds(&self, bounds: Rect, _theme: &Theme) -> Rect {
        Rect::new(bounds.x, bounds.y, bounds.width, bounds.height.max(1.0))
    }

    fn animated_height(&self) -> Option<f32> {
        self.height_driven().then(|| self.natural * self.k)
    }

    fn clip_children(&self, _theme: &Theme) -> Option<f32> {
        self.height_driven().then_some(0.0)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        let m = cx.theme().motion;
        let (target, duration) = if self.open {
            (1.0, m.expand)
        } else {
            (0.0, m.expand.mul_f32(0.75))
        };
        self.t.animate_to(target, now, duration, Easing::EaseOut);
        let k = self.t.value(now).clamp(0.0, 1.0);
        let changed = (k - self.k).abs() > 1e-4;
        self.k = k;
        if self.t.is_animating(now) {
            // 每一帧都要重新布局（高度在变），布局请求走定时器事件。
            cx.request_frame();
            cx.request_timer(now);
        } else if changed || (!self.open && !self.notified) {
            // 刚停下：再布局一次（展开后恢复自动高度），收起的话顺便通知应用。
            cx.request_timer(now);
        }
    }

    fn opacity(&self, _cx: &PaintCx) -> f32 {
        self.k
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if *event != Event::Timer {
            return;
        }
        cx.request_layout();
        cx.request_paint();
        if !self.open && self.k <= 0.0 && !self.notified {
            self.notified = true;
            if let Some(f) = &self.on_collapsed {
                f(cx.app());
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
