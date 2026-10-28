//! 按钮。

use cairn_ui_layout::MeasureInput;

use crate::element::element;
use crate::theme::{ButtonStyle, Theme};
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{Interaction, focus_ring, focus_ring_extent, lift_press, max_extent, press_event};
use crate::widgets::icon::{Icon, paint_icon};
use crate::{Affine, CursorIcon, FontFamily, Insets, Painter, Point, Rect, Shadow, Size, Style, TextLayout, TextStyle};

type ClickHandler<S> = Box<dyn Fn(&mut S)>;

/// 文字按钮。用 [`button`] 创建，[`Button::on_click`] 设置点击回调。
///
/// 鼠标在按钮上按下、并在按钮上松开才算一次点击；有键盘焦点时 Enter / 空格也会触发。
/// 外观来自主题的 [`Theme::button`]，[`Button::accent`] 用 [`Theme::accent_button`]。
pub struct Button<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    label: String,
    icon: Option<Icon>,
    on_click: Option<ClickHandler<S>>,
    enabled: bool,
    accent: bool,
    pressed: bool,
    anim: Interaction,
    text: Option<TextLayout>,
}

pub fn button<S: 'static>(label: impl Into<String>) -> Button<S> {
    Button {
        style: Style {
            padding: Insets::AUTO,
            ..Style::default()
        },
        key: None,
        label: label.into(),
        icon: None,
        on_click: None,
        enabled: true,
        accent: false,
        pressed: false,
        anim: Interaction::default(),
        text: None,
    }
}

element!(Button<S>);

impl<S: 'static> Button<S> {
    /// 点击时调用，参数是应用状态。
    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// 禁用后不响应点击、不能获得焦点，并显示为灰色。
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// 强调按钮（主题的 `accent_button`，Nocturne 下是香槟金）。一页最多用一两个。
    pub fn accent(mut self) -> Self {
        self.accent = true;
        self
    }

    /// 在文字前面加一个图标。
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    fn click(&self, cx: &mut EventCx<S>) {
        if let Some(f) = &self.on_click {
            f(cx.app());
        }
    }

    fn look<'t>(&self, theme: &'t Theme) -> &'t ButtonStyle {
        if self.accent {
            &theme.accent_button
        } else {
            &theme.button
        }
    }

    /// 图标边长和图标与文字的间距（没有文字时不留间距）。
    fn icon_metrics(&self, theme: &Theme) -> (f32, f32) {
        let look = self.look(theme);
        (look.icon_size, if self.label.is_empty() { 0.0 } else { look.icon_gap })
    }
}

impl<S: 'static> Widget<S> for Button<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.label != self.label || new.icon != self.icon || new.accent != self.accent {
            change = Change::LAYOUT;
        }
        if new.enabled != self.enabled {
            change |= Change::PAINT;
            self.pressed = false;
        }
        self.label = new.label;
        self.icon = new.icon;
        self.accent = new.accent;
        self.enabled = new.enabled;
        self.on_click = new.on_click;
        change
    }

    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        if style.padding.is_auto() {
            style.padding = self.look(theme).padding;
        }
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let theme = cx.theme();
        let style = TextStyle {
            font_size: theme.typography.body.size,
            line_height: theme.typography.line_height,
            weight: self.look(theme).weight,
            family: FontFamily::SansSerif,
        };
        let icon = self.icon.map(|_| self.icon_metrics(theme));
        match &mut self.text {
            Some(l) => {
                l.set_style(cx.text(), &style);
                l.set_text(cx.text(), &self.label);
            }
            None => self.text = Some(TextLayout::new(cx.text(), &self.label, &style)),
        }
        let mut size = self.text.as_ref().map(|l| l.size()).unwrap_or_default();
        if self.label.is_empty() {
            size.width = 0.0;
        }
        if let Some((i, gap)) = icon {
            size.width += i + gap;
            size.height = size.height.max(i);
        }
        size
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let look = self.look(theme);
        let g = max_extent(&look.glow, &look.glow_hover);
        let ring = focus_ring_extent(theme);
        bounds.outset(
            g[0].max(ring) + look.lift,
            g[1].max(ring),
            g[2].max(ring),
            g[3].max(ring),
        )
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let hovered = self.enabled && cx.is_hovered();
        self.anim.retarget(cx, hovered, self.pressed && hovered);
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        let look = self.look(cx.theme());
        lift_press(cx.bounds(), look.lift, look.press_scale, self.anim.amounts(cx))
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let a = self.anim.amounts(cx);
        let theme = cx.theme();
        let look = self.look(theme);
        let b = cx.bounds();
        let r = theme.radius.control;
        let fg = if self.enabled {
            let glow = look.glow.lerp(&look.glow_hover, a.hover);
            p.draw_shadow(b, r, &glow);
            p.fill_rounded_rect(
                b,
                r,
                look.fill.mix(look.fill_hover, a.hover).mix(look.fill_pressed, a.press),
            );
            if look.top_highlight.a > 0 {
                p.draw_inner_shadow(b, r, &Shadow::new(0.0, p.hairline(), 0.0, look.top_highlight));
            }
            let border = look.border.mix(look.border_hover, a.hover);
            if border.a > 0 {
                p.stroke_rect(b, r, 0.0, border);
            }
            look.text
        } else {
            p.fill_rounded_rect(b, r, look.disabled_fill);
            if look.disabled_border.a > 0 {
                p.stroke_rect(b, r, 0.0, look.disabled_border);
            }
            look.disabled_text
        };

        let c = cx.content();
        let text_size = match &self.text {
            Some(t) if !self.label.is_empty() => t.size(),
            _ => Size::ZERO,
        };
        let icon = self.icon.map(|i| {
            let (size, gap) = self.icon_metrics(theme);
            (i, size, gap)
        });
        let total = text_size.width + icon.map(|(_, s, g)| s + g).unwrap_or(0.0);
        let mut x = c.x + (c.width - total) / 2.0;
        if let Some((i, s, g)) = icon {
            paint_icon(p, i, Rect::new(x, c.y + (c.height - s) / 2.0, s, s), fg);
            x += s + g;
        }
        if let Some(text) = &self.text
            && !self.label.is_empty()
        {
            p.draw_text(text, Point::new(x, c.y + (c.height - text_size.height) / 2.0), fg);
        }
        focus_ring(p, theme, b, r, a.focus);
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if press_event(&mut self.pressed, self.enabled, cx, event) {
            self.click(cx);
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
            role: Role::Button,
            label: Some(if self.label.is_empty() {
                self.icon.map(|i| i.name().to_string()).unwrap_or_default()
            } else {
                self.label.clone()
            }),
            value: None,
            disabled: !self.enabled,
        }
    }
}
