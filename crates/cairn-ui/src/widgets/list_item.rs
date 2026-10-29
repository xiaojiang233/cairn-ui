//! 列表行。

use crate::anim::Transition;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{Interaction, drive_state, focus_ring, focus_ring_extent, press_event};
use crate::{Align, CursorIcon, Direction, El, Insets, Painter, Rect, Style, Styled};

type ClickHandler<S> = Box<dyn Fn(&mut S)>;

/// 列表里的一行：横向排列子元素（纵向居中），悬停时铺一层淡淡的底色。用 [`list_item`] 创建。
///
/// 设置了 [`ListItem::on_click`] 的行可以点击、可以获得焦点；行里的按钮、复选框自己处理点击，
/// 点它们不会触发整行的点击。[`ListItem::selected`] 把行标成选中（底色和边框更亮）。
/// 外观来自主题的 [`Theme::list_item`]，圆角取 `radius.control`。
pub struct ListItem<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    selected: bool,
    on_click: Option<ClickHandler<S>>,
    pressed: bool,
    anim: Interaction,
    sel: Transition,
    /// 画过一次了（之后状态变化才有过渡）。
    started: bool,
}

pub fn list_item<S: 'static>() -> ListItem<S> {
    ListItem {
        style: Style {
            direction: Direction::Row,
            align_items: Some(Align::Center),
            padding: Insets::AUTO,
            gap: f32::NAN,
            ..Style::default()
        },
        key: None,
        children: Vec::new(),
        selected: false,
        on_click: None,
        pressed: false,
        anim: Interaction::default(),
        sel: Transition::default(),
        started: false,
    }
}

impl<S: 'static> ListItem<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// 子元素间距，默认取主题的 `list_item.gap`。
    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = gap;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 点击整行时调用。设置后行可以获得焦点，Enter / 空格也会触发。
    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl<S: 'static> Styled for ListItem<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<ListItem<S>> for El<S> {
    fn from(mut l: ListItem<S>) -> Self {
        let style = std::mem::take(&mut l.style);
        let key = l.key.take();
        let children = std::mem::take(&mut l.children);
        let mut el = El::new(l);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for ListItem<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.selected != self.selected || new.on_click.is_some() != self.on_click.is_some() {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.selected = new.selected;
        self.on_click = new.on_click;
        if self.on_click.is_none() {
            self.pressed = false;
        }
        change
    }

    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        if style.padding.is_auto() {
            style.padding = theme.list_item.padding;
        }
        if style.gap.is_nan() {
            style.gap = theme.list_item.gap;
        }
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let e = focus_ring_extent(theme);
        bounds.inflate(e, e)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let hovered = cx.is_hovered();
        self.anim.retarget(cx, hovered, self.pressed && hovered);
        let m = cx.theme().motion;
        drive_state(&mut self.sel, cx, self.selected, m.press, m.easing, &mut self.started);
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = &theme.list_item;
        let a = self.anim.amounts(cx);
        let s = self.sel.value(cx.now());
        let (b, r) = (cx.bounds(), theme.radius.control);
        let mut fill = look.fill.mix(look.fill_hover, a.hover).mix(look.fill_selected, s);
        if self.on_click.is_some() {
            fill = fill.mix(look.fill_pressed, a.press);
        }
        if fill.a > 0 {
            p.fill_rounded_rect(b, r, fill);
        }
        if look.border_selected.a > 0 && s > 0.0 {
            p.stroke_rect(b, r, 0.0, look.border_selected.fade(s));
        }
        focus_ring(p, theme, b, r, a.focus);
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if self.on_click.is_none() {
            if matches!(event, Event::PointerEnter | Event::PointerLeave) {
                cx.request_paint();
            }
            return;
        }
        if press_event(&mut self.pressed, true, cx, event)
            && let Some(f) = &self.on_click
        {
            f(cx.app());
        }
    }

    fn focusable(&self) -> bool {
        self.on_click.is_some()
    }

    fn cursor(&self) -> CursorIcon {
        if self.on_click.is_some() {
            CursorIcon::Pointer
        } else {
            CursorIcon::Default
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::ListItem,
            value: self.selected.then(|| "selected".into()),
            ..Semantics::default()
        }
    }
}
