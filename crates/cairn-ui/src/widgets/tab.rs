//! 标签页 / 可选中的切换按钮。

use cairn_ui_layout::MeasureInput;

use crate::anim::Transition;
use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{Interaction, drive_state, focus_ring, focus_ring_extent, press_event};
use crate::widgets::icon::{Icon, paint_icon};
use crate::{CursorIcon, FontFamily, Insets, Painter, Point, Rect, Size, Style, TextLayout, TextStyle};

type ClickHandler<S> = Box<dyn Fn(&mut S)>;

/// 标签页按钮：图标 + 文字 + 可选的数字徽标，有“选中”状态。用 [`tab`] 创建。
///
/// 一组 `tab` 放进 [`crate::row`] 就是标签栏；单独用就是可以保持按下状态的切换按钮（例如工具栏上的
/// “当前打开的面板”）。选中状态由应用给出（[`Tab::selected`]），点击只调用 [`Tab::on_click`]。
/// 外观来自主题的 [`Theme::tab`]：未选中时没有底色，悬停时淡淡一层，选中时底色、边框和文字都加亮，
/// 徽标换成强调色。
pub struct Tab<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    label: String,
    icon: Option<Icon>,
    badge: Option<String>,
    selected: bool,
    enabled: bool,
    on_click: Option<ClickHandler<S>>,
    pressed: bool,
    anim: Interaction,
    sel: Transition,
    /// 画过一次了（之后状态变化才有过渡）。
    started: bool,
    text: Option<TextLayout>,
    badge_text: Option<TextLayout>,
}

pub fn tab<S: 'static>(label: impl Into<String>) -> Tab<S> {
    Tab {
        style: Style {
            padding: Insets::AUTO,
            ..Style::default()
        },
        key: None,
        label: label.into(),
        icon: None,
        badge: None,
        selected: false,
        enabled: true,
        on_click: None,
        pressed: false,
        anim: Interaction::default(),
        sel: Transition::default(),
        started: false,
        text: None,
        badge_text: None,
    }
}

element!(Tab<S>);

impl<S: 'static> Tab<S> {
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// 文字后面的小徽标，通常是数量。
    pub fn badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    fn badge_padding(theme: &Theme) -> f32 {
        theme.tab.gap
    }
}

impl<S: 'static> Widget<S> for Tab<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.label != self.label || new.icon != self.icon || new.badge != self.badge {
            change = Change::LAYOUT;
        }
        if new.selected != self.selected || new.enabled != self.enabled {
            change |= Change::PAINT;
        }
        if !new.enabled {
            self.pressed = false;
        }
        self.label = new.label;
        self.icon = new.icon;
        self.badge = new.badge;
        self.selected = new.selected;
        self.enabled = new.enabled;
        self.on_click = new.on_click;
        change
    }

    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        if style.padding.is_auto() {
            style.padding = theme.tab.padding;
        }
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let theme = cx.theme();
        let look = theme.tab;
        let style = TextStyle {
            font_size: theme.typography.body.size,
            line_height: theme.typography.line_height,
            weight: look.weight,
            family: FontFamily::SansSerif,
        };
        let small = TextStyle {
            font_size: theme.typography.caption.size,
            line_height: theme.typography.line_height,
            weight: look.weight,
            family: FontFamily::SansSerif,
        };
        set_layout(&mut self.text, cx, &self.label, &style);
        match &self.badge {
            Some(b) => set_layout(&mut self.badge_text, cx, b, &small),
            None => self.badge_text = None,
        }
        let mut w = 0.0;
        let mut h: f32 = 0.0;
        let mut parts = 0;
        if self.icon.is_some() {
            w += look.icon_size;
            h = h.max(look.icon_size);
            parts += 1;
        }
        if !self.label.is_empty()
            && let Some(t) = &self.text
        {
            w += t.size().width;
            h = h.max(t.size().height);
            parts += 1;
        }
        if let Some(t) = &self.badge_text {
            let pad = Self::badge_padding(theme);
            w += t.size().width + pad * 2.0;
            h = h.max(t.size().height);
            parts += 1;
        }
        if parts > 1 {
            w += look.gap * (parts - 1) as f32;
        }
        Size::new(w, h)
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let e = focus_ring_extent(theme);
        bounds.inflate(e, e)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let hovered = self.enabled && cx.is_hovered();
        self.anim.retarget(cx, hovered, self.pressed && hovered);
        let m = cx.theme().motion;
        drive_state(&mut self.sel, cx, self.selected, m.press, m.easing, &mut self.started);
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = theme.tab;
        let a = self.anim.amounts(cx);
        let s = self.sel.value(cx.now());
        let b = cx.bounds();
        let r = theme.radius.control;
        let opacity = if self.enabled { 1.0 } else { theme.disabled_opacity };

        let fill = look
            .fill
            .mix(look.fill_hover, a.hover * (1.0 - s))
            .mix(look.fill_selected, s);
        if fill.a > 0 {
            p.fill_rounded_rect(b, r, fill.fade(opacity));
        }
        if look.border_selected.a > 0 && s > 0.0 {
            p.stroke_rect(b, r, 0.0, look.border_selected.fade(s * opacity));
        }
        let fg = look
            .text
            .mix(look.text_hover, a.hover)
            .mix(look.text_selected, s)
            .fade(opacity);

        let c = cx.content();
        let mid = |h: f32| c.y + (c.height - h) / 2.0;
        let mut x = c.x;
        if let Some(icon) = self.icon {
            let i = look.icon_size;
            paint_icon(p, icon, Rect::new(x, mid(i), i, i), fg);
            x += i + look.gap;
        }
        if !self.label.is_empty()
            && let Some(t) = &self.text
        {
            p.draw_text(t, Point::new(x, mid(t.size().height)), fg);
            x += t.size().width + look.gap;
        }
        if let Some(t) = &self.badge_text {
            let pad = Self::badge_padding(theme);
            let ts = t.size();
            let pill = Rect::new(x, mid(ts.height), ts.width + pad * 2.0, ts.height);
            let bg = look.badge_fill.mix(look.badge_fill_selected, s).fade(opacity);
            p.fill_rounded_rect(pill, pill.height / 2.0, bg);
            let tc = look.badge_text.mix(look.badge_text_selected, s).fade(opacity);
            p.draw_text(t, Point::new(pill.x + pad, pill.y), tc);
        }
        focus_ring(p, theme, b, r, a.focus);
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if press_event(&mut self.pressed, self.enabled, cx, event)
            && let Some(f) = &self.on_click
        {
            f(cx.app());
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
            role: Role::Tab,
            label: Some(if self.label.is_empty() {
                self.icon.map(|i| i.name().to_string()).unwrap_or_default()
            } else {
                self.label.clone()
            }),
            value: self.selected.then(|| "selected".into()),
            disabled: !self.enabled,
        }
    }
}

/// 复用已有的排版对象，只在文字或样式变了时重排。
pub(crate) fn set_layout(slot: &mut Option<TextLayout>, cx: &mut LayoutCx, text: &str, style: &TextStyle) {
    match slot {
        Some(l) => {
            l.set_style(cx.text(), style);
            l.set_text(cx.text(), text);
        }
        None => *slot = Some(TextLayout::new(cx.text(), text, style)),
    }
}
