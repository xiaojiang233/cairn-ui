//! 按钮类控件共用的小工具：按下 / 点击判定、过渡驱动、悬停上浮与按下缩小的变换、焦点框。

use std::time::Duration;

use crate::anim::{Easing, Transition};
use crate::theme::Theme;
use crate::widget::{Event, EventCx, PaintCx};
use crate::{Affine, Key, NamedKey, Painter, PointerButton, Rect, Shadow};

/// 在控件上按下、并在控件上松开才算一次点击；有键盘焦点时 Enter / 空格也算。返回这次事件是否构成点击。
pub(crate) fn press_event<S>(pressed: &mut bool, enabled: bool, cx: &mut EventCx<S>, event: &Event) -> bool {
    match event {
        Event::PointerEnter | Event::PointerLeave => {
            cx.request_paint();
            false
        }
        Event::PointerDown {
            button: PointerButton::Left,
            ..
        } if enabled => {
            *pressed = true;
            cx.request_paint();
            cx.set_handled();
            false
        }
        Event::PointerUp {
            pos,
            button: PointerButton::Left,
        } => {
            let mut click = false;
            if *pressed {
                *pressed = false;
                cx.request_paint();
                click = cx.bounds().contains(*pos);
            }
            cx.set_handled();
            click
        }
        Event::Key(k)
            if k.pressed
                && !k.repeat
                && enabled
                && matches!(k.key, Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space)) =>
        {
            cx.set_handled();
            true
        }
        Event::FocusLost if *pressed => {
            *pressed = false;
            cx.request_paint();
            false
        }
        _ => false,
    }
}

/// 把过渡的目标设为 `on`（1 或 0）。还在过渡中就请求下一帧。在 [`crate::Widget::animate`] 里调用。
pub(crate) fn drive(t: &mut Transition, cx: &mut PaintCx, on: bool, duration: Duration, easing: Easing) {
    let now = cx.now();
    t.animate_to(if on { 1.0 } else { 0.0 }, now, duration, easing);
    if t.is_animating(now) {
        cx.request_frame();
    }
}

/// 悬停、按下、键盘焦点三个过渡。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Interaction {
    pub hover: Transition,
    pub press: Transition,
    pub focus: Transition,
}

/// 三个过渡在某一时刻的值。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Amounts {
    pub hover: f32,
    pub press: f32,
    pub focus: f32,
}

impl Interaction {
    /// 按当前状态重设三个过渡的目标。在 [`crate::Widget::animate`] 里调用。
    pub fn retarget(&mut self, cx: &mut PaintCx, hovered: bool, pressed: bool) {
        let m = cx.theme().motion;
        let focus_visible = cx.focus_visible();
        drive(&mut self.hover, cx, hovered, m.hover, m.easing);
        drive(&mut self.press, cx, pressed, m.press, m.press_easing);
        drive(&mut self.focus, cx, focus_visible, m.focus, m.easing);
    }

    /// 三个过渡此刻的值。
    pub fn amounts(&self, cx: &PaintCx) -> Amounts {
        let now = cx.now();
        Amounts {
            hover: self.hover.value(now),
            press: self.press.value(now),
            focus: self.focus.value(now),
        }
    }
}

/// 悬停上浮 `lift`、按下缩小到 `press_scale` 的变换。按下时不再上浮。
pub(crate) fn lift_press(bounds: Rect, lift: f32, press_scale: f32, a: Amounts) -> Affine {
    let scale = 1.0 - (1.0 - press_scale) * a.press;
    let up = lift * a.hover * (1.0 - a.press);
    let mut t = Affine::IDENTITY;
    if (scale - 1.0).abs() > 1e-4 {
        t = Affine::scale_about(bounds.center(), scale);
    }
    if up.abs() > 1e-4 {
        t = t.then(Affine::translate(0.0, -up));
    }
    t
}

/// 两个阴影各方向外扩的最大值：上、右、下、左。
pub(crate) fn max_extent(a: &Shadow, b: &Shadow) -> [f32; 4] {
    let (x, y) = (a.extent(), b.extent());
    [x[0].max(y[0]), x[1].max(y[1]), x[2].max(y[2]), x[3].max(y[3])]
}

/// 键盘焦点框，`amount`（0–1）控制透明度，用于渐显。
pub(crate) fn focus_ring(p: &mut dyn Painter, theme: &Theme, bounds: Rect, radius: f32, amount: f32) {
    if amount <= 0.0 {
        return;
    }
    let f = theme.focus_ring;
    p.stroke_rect(
        bounds.inflate(f.offset, f.offset),
        radius + f.offset,
        f.width,
        f.color.fade(amount),
    );
}

/// 焦点框画到控件外多远。
pub(crate) fn focus_ring_extent(theme: &Theme) -> f32 {
    theme.focus_ring.offset + theme.focus_ring.width
}
