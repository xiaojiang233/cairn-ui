//! 复选框。

use cairn_ui_layout::MeasureInput;

use crate::anim::Transition;
use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{Interaction, drive_state, focus_ring, focus_ring_extent, press_event};
use crate::widgets::icon::{Icon, paint_icon};
use crate::{CursorIcon, FontFamily, Painter, Point, Rect, Size, Style, TextLayout, TextStyle};

type ChangeHandler<S> = Box<dyn Fn(&mut S, bool)>;

/// 复选框，可以带一段文字。用 [`checkbox`] 创建。
///
/// 是“受控”的：勾选状态由应用给出（`checkbox(self.enabled)`），点击时 [`Checkbox::on_change`]
/// 收到新状态，应用改了自己的字段，界面重建后框里才变。点文字也算点框；有焦点时空格 / 回车切换。
/// 外观来自主题的 [`Theme::checkbox`]，勾选后的底色通常就是强调色。
pub struct Checkbox<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    checked: bool,
    label: String,
    enabled: bool,
    on_change: Option<ChangeHandler<S>>,
    pressed: bool,
    anim: Interaction,
    check: Transition,
    /// 画过一次了（之后状态变化才有过渡）。
    started: bool,
    text: Option<TextLayout>,
}

pub fn checkbox<S: 'static>(checked: bool) -> Checkbox<S> {
    Checkbox {
        style: Style::default(),
        key: None,
        checked,
        label: String::new(),
        enabled: true,
        on_change: None,
        pressed: false,
        anim: Interaction::default(),
        check: Transition::default(),
        started: false,
        text: None,
    }
}

element!(Checkbox<S>);

impl<S: 'static> Checkbox<S> {
    /// 框右边的文字。
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 点击时收到切换后的状态。
    pub fn on_change(mut self, f: impl Fn(&mut S, bool) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn box_rect(&self, content: Rect, theme: &Theme) -> Rect {
        let s = theme.checkbox.size;
        Rect::new(content.x, content.y + (content.height - s) / 2.0, s, s)
    }
}

impl<S: 'static> Widget<S> for Checkbox<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.label != self.label {
            change = Change::LAYOUT;
        }
        if new.checked != self.checked || new.enabled != self.enabled {
            change |= Change::PAINT;
        }
        if !new.enabled {
            self.pressed = false;
        }
        self.checked = new.checked;
        self.label = new.label;
        self.enabled = new.enabled;
        self.on_change = new.on_change;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let theme = cx.theme();
        let look = &theme.checkbox;
        if self.label.is_empty() {
            self.text = None;
            return Size::new(look.size, look.size);
        }
        let style = TextStyle {
            font_size: theme.typography.body.size,
            line_height: theme.typography.line_height,
            weight: theme.typography.body.weight,
            family: FontFamily::SansSerif,
        };
        match &mut self.text {
            Some(l) => {
                l.set_style(cx.text(), &style);
                l.set_text(cx.text(), &self.label);
            }
            None => self.text = Some(TextLayout::new(cx.text(), &self.label, &style)),
        }
        let t = self.text.as_ref().map(|l| l.size()).unwrap_or_default();
        Size::new(look.size + look.label_gap + t.width, t.height.max(look.size))
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let e = focus_ring_extent(theme);
        bounds.inflate(e, e)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let hovered = self.enabled && cx.is_hovered();
        self.anim.retarget(cx, hovered, self.pressed && hovered);
        let m = cx.theme().motion;
        drive_state(&mut self.check, cx, self.checked, m.press, m.easing, &mut self.started);
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = &theme.checkbox;
        let a = self.anim.amounts(cx);
        let k = self.check.value(cx.now());
        let b = self.box_rect(cx.content(), theme);
        let r = look.radius;
        let opacity = if self.enabled { 1.0 } else { theme.disabled_opacity };

        p.fill_rounded_rect(b, r, look.fill.mix(look.fill_hover, a.hover).fade(opacity));
        if k < 1.0 {
            p.stroke_rect(
                b,
                r,
                0.0,
                look.border.mix(look.border_hover, a.hover).fade(opacity * (1.0 - k)),
            );
        }
        if k > 0.0 {
            let fill = look.checked_fill.mix(look.checked_fill_hover, a.hover);
            p.fill_rounded_rect(b, r, fill.fade(opacity * k));
            paint_icon(
                p,
                Icon::Check,
                b.inflate(-b.width * 0.12, -b.height * 0.12),
                look.check.fade(opacity * k),
            );
        }
        if let Some(text) = &self.text {
            let c = cx.content();
            let t = text.size();
            let color = theme.typography.body.color.fade(opacity);
            p.draw_text(
                text,
                Point::new(b.right() + look.label_gap, c.y + (c.height - t.height) / 2.0),
                color,
            );
        }
        focus_ring(p, theme, b, r, a.focus);
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if press_event(&mut self.pressed, self.enabled, cx, event)
            && let Some(f) = &self.on_change
        {
            f(cx.app(), !self.checked);
        }
    }

    fn focusable(&self) -> bool {
        self.enabled
    }

    fn cursor(&self) -> CursorIcon {
        if self.enabled {
            CursorIcon::Pointer
        } else {
            CursorIcon::Default
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Checkbox,
            label: (!self.label.is_empty()).then(|| self.label.clone()),
            value: Some(if self.checked { "checked" } else { "unchecked" }.into()),
            disabled: !self.enabled,
        }
    }
}
