//! 圆形图标按钮。

use cairn_ui_layout::MeasureInput;

use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{Interaction, focus_ring, focus_ring_extent, lift_press, press_event};
use crate::widgets::icon::{Icon, paint_icon};
use crate::{Affine, Color, CursorIcon, Insets, Painter, Rect, Shadow, Size, Style};

type ClickHandler<S> = Box<dyn Fn(&mut S)>;

/// 圆形图标按钮。用 [`icon_button`] 创建。直径、颜色来自主题的 [`Theme::icon_button`]。
pub struct IconButton<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    icon: Icon,
    label: Option<String>,
    color: Option<Color>,
    on_click: Option<ClickHandler<S>>,
    enabled: bool,
    pressed: bool,
    anim: Interaction,
}

pub fn icon_button<S: 'static>(icon: Icon) -> IconButton<S> {
    IconButton {
        style: Style {
            padding: Insets::ZERO,
            ..Style::default()
        },
        key: None,
        icon,
        label: None,
        color: None,
        on_click: None,
        enabled: true,
        pressed: false,
        anim: Interaction::default(),
    }
}

element!(IconButton<S>);

impl<S: 'static> IconButton<S> {
    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// 无障碍名称（读屏软件念的文字）。默认是图标的英文名。
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// 图标颜色，默认取主题的 `icon_button.icon`。
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

impl<S: 'static> Widget<S> for IconButton<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.icon != self.icon || new.color != self.color || new.enabled != self.enabled {
            change = Change::PAINT;
        }
        if new.enabled != self.enabled {
            self.pressed = false;
        }
        self.icon = new.icon;
        self.label = new.label;
        self.color = new.color;
        self.enabled = new.enabled;
        self.on_click = new.on_click;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let d = cx.theme().icon_button.size;
        Size::new(d, d)
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let e = focus_ring_extent(theme);
        bounds.inflate(e, e)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let hovered = self.enabled && cx.is_hovered();
        self.anim.retarget(cx, hovered, self.pressed && hovered);
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        lift_press(
            cx.bounds(),
            0.0,
            cx.theme().icon_button.press_scale,
            self.anim.amounts(cx),
        )
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let a = self.anim.amounts(cx);
        let theme = cx.theme();
        let look = &theme.icon_button;
        let b = cx.bounds();
        let r = b.width.min(b.height) / 2.0;
        // 没有专门的“禁用”配色，整体按主题给的不透明度淡出。
        let k = if self.enabled { 1.0 } else { theme.disabled_opacity };
        p.fill_rounded_rect(b, r, look.fill.mix(look.fill_hover, a.hover).fade(k));
        if look.top_highlight.a > 0 {
            p.draw_inner_shadow(b, r, &Shadow::new(0.0, p.hairline(), 0.0, look.top_highlight.fade(k)));
        }
        let border = look.border.mix(look.border_hover, a.hover).fade(k);
        if border.a > 0 {
            p.stroke_rect(b, r, 0.0, border);
        }
        let s = look.icon_size;
        let c = b.center();
        let color = self.color.unwrap_or(look.icon).fade(k);
        paint_icon(p, self.icon, Rect::new(c.x - s / 2.0, c.y - s / 2.0, s, s), color);
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
            role: Role::Button,
            label: Some(self.label.clone().unwrap_or_else(|| self.icon.name().to_string())),
            value: None,
            disabled: !self.enabled,
        }
    }
}
