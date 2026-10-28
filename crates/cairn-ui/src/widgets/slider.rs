//! 滑块：在一个数值范围内用指针拖动或键盘调节。

use cairn_ui_layout::{AvailableSpace, MeasureInput};

use crate::anim::Transition;
use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{drive, focus_ring, focus_ring_extent};
use crate::{CursorIcon, Key, NamedKey, Painter, PointerButton, Rect, Shadow, Size, Style};

/// 没有指定宽度时的宽度（逻辑像素）。
const DEFAULT_WIDTH: f32 = 200.0;
/// 没有设置步长时，方向键每次移动整个范围的这一比例。
const KEY_FRACTION: f32 = 0.01;
/// PageUp / PageDown 是方向键步长的这么多倍。
const PAGE_STEPS: f32 = 10.0;

type ChangeHandler<S> = Box<dyn Fn(&mut S, f32)>;

/// 滑块。用 [`slider`] 创建，默认范围 0–1。
///
/// 和 [`crate::TextInput`] 一样是“受控”的：传入当前值，用 [`Slider::on_change`] 接收用户的调整并写回状态。
/// 设置了 `on_change` 时以应用给的值为准；没设置时只在应用给的值本身变化时才覆盖。
///
/// 操作：在轨道上按下，滑块跳到指针处并开始拖动；按在滑块上则保持抓取位置。有焦点时 ←/↓ 减、→/↑ 加
/// （有步长按步长，否则每次 1%），PageUp / PageDown 为 10 倍，Home / End 到两端。
pub struct Slider<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    /// 应用给的值，用来判断应用有没有改过它。
    value: f32,
    /// 正在显示的值：拖动时先于应用变化。
    shown: f32,
    min: f32,
    max: f32,
    step: Option<f32>,
    on_change: Option<ChangeHandler<S>>,
    enabled: bool,
    dragging: bool,
    /// 按下时指针相对滑块中心的横向偏移：按在滑块上时保持它，按在轨道上时为 0（滑块直接跳到指针处）。
    grab: f32,
    /// 拖动时滑块外圈的环，以及键盘焦点框。
    ring: Transition,
    focus: Transition,
}

pub fn slider<S: 'static>(value: f32) -> Slider<S> {
    Slider {
        style: Style::default(),
        key: None,
        value,
        shown: value,
        min: 0.0,
        max: 1.0,
        step: None,
        on_change: None,
        enabled: true,
        dragging: false,
        grab: 0.0,
        ring: Transition::default(),
        focus: Transition::default(),
    }
}

element!(Slider<S>);

impl<S: 'static> Slider<S> {
    /// 取值范围，默认 0 到 1。`min` 大于 `max` 时两者对调；含 NaN 或无穷大的参数被忽略。
    pub fn range(mut self, min: f32, max: f32) -> Self {
        if min.is_finite() && max.is_finite() {
            (self.min, self.max) = if min <= max { (min, max) } else { (max, min) };
        }
        self
    }

    /// 步长：拖动和键盘调节得到的值都落在 `min + n × step` 上。非正数或 NaN 表示连续取值。
    pub fn step(mut self, step: f32) -> Self {
        self.step = (step.is_finite() && step > 0.0).then_some(step);
        self
    }

    /// 值被用户调节时调用，参数是应用状态和新值。
    pub fn on_change(mut self, f: impl Fn(&mut S, f32) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// 禁用后不能拖动、不能获得焦点。
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn clamp(&self, v: f32) -> f32 {
        if v.is_nan() {
            self.min
        } else {
            v.clamp(self.min, self.max)
        }
    }

    /// 正在显示的值（限制在范围内）。
    fn display(&self) -> f32 {
        self.clamp(self.shown)
    }

    /// 限制在范围内，并对齐到步长。
    fn snap(&self, v: f32) -> f32 {
        let v = self.clamp(v);
        match self.step {
            Some(s) => self.clamp(self.min + ((v - self.min) / s).round() * s),
            None => v,
        }
    }

    /// 键盘每次调节的量。
    fn unit(&self) -> f32 {
        self.step.unwrap_or((self.max - self.min) * KEY_FRACTION)
    }

    /// 值在范围内的位置，0–1。
    fn fraction(&self) -> f32 {
        let span = self.max - self.min;
        if span > 0.0 {
            (self.display() - self.min) / span
        } else {
            0.0
        }
    }

    /// 滑块中心的横向可移动范围：两端各留半个滑块，滑块始终在控件范围内。
    fn travel(bounds: Rect, theme: &Theme) -> (f32, f32) {
        let r = theme.slider.knob_size / 2.0;
        let left = bounds.x + r;
        (left, (bounds.right() - r).max(left))
    }

    fn knob_x(&self, bounds: Rect, theme: &Theme) -> f32 {
        let (a, b) = Self::travel(bounds, theme);
        a + (b - a) * self.fraction()
    }

    /// 控件高度：滑块加上外圈的环（或键盘焦点框），上下对称。
    fn height(theme: &Theme) -> f32 {
        let l = &theme.slider;
        l.knob_size + 2.0 * (l.knob_ring_gap + l.knob_ring_width).max(focus_ring_extent(theme))
    }

    /// 把值设为 `v`（先对齐步长），变了就重画并通知应用。
    fn set(&mut self, cx: &mut EventCx<S>, v: f32) {
        let v = self.snap(v);
        if v == self.display() {
            return;
        }
        self.shown = v;
        cx.request_paint();
        if let Some(f) = &self.on_change {
            f(cx.app(), v);
        }
    }

    /// 指针（窗口坐标）横向位于 `x` 时对应的值。
    fn value_at(&self, x: f32, bounds: Rect, theme: &Theme) -> f32 {
        let (a, b) = Self::travel(bounds, theme);
        let t = if b > a {
            ((x - a) / (b - a)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.min + t * (self.max - self.min)
    }

    fn key(&mut self, cx: &mut EventCx<S>, key: &Key) {
        let unit = self.unit();
        let target = match key {
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) => self.display() - unit,
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowUp) => self.display() + unit,
            Key::Named(NamedKey::PageDown) => self.display() - PAGE_STEPS * unit,
            Key::Named(NamedKey::PageUp) => self.display() + PAGE_STEPS * unit,
            Key::Named(NamedKey::Home) => self.min,
            Key::Named(NamedKey::End) => self.max,
            _ => return,
        };
        cx.set_handled();
        self.set(cx, target);
    }
}

impl<S: 'static> Widget<S> for Slider<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.min != self.min || new.max != self.max || new.step != self.step || new.enabled != self.enabled {
            change = Change::PAINT;
        }
        if !new.enabled {
            self.dragging = false;
        }
        (self.min, self.max, self.step, self.enabled) = (new.min, new.max, new.step, new.enabled);
        let controlled = new.on_change.is_some();
        if (controlled && self.clamp(new.value) != self.display()) || (!controlled && new.value != self.value) {
            self.shown = new.value;
            change = Change::PAINT;
        }
        self.value = new.value;
        self.on_change = new.on_change;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        let theme = cx.theme();
        let width = match input.available_width {
            // 容器不够宽时允许收缩到两个滑块宽。
            AvailableSpace::MinContent => theme.slider.knob_size * 2.0,
            _ => DEFAULT_WIDTH,
        };
        Size::new(width, Self::height(theme))
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        // 滑块的阴影和焦点框会画到控件范围外（滑块在两端时尤其明显）。
        let l = &theme.slider;
        let ring = focus_ring_extent(theme);
        let e = l.knob_shadow.extent();
        let r = l.knob_size / 2.0;
        let strip = Rect::new(bounds.x, bounds.center().y - r, bounds.width, l.knob_size);
        strip
            .outset(e[0].max(ring), e[1].max(ring), e[2].max(ring), e[3].max(ring))
            .union(&bounds)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let motion = &cx.theme().motion;
        let focus_visible = cx.focus_visible();
        let dragging = self.enabled && self.dragging;
        drive(&mut self.ring, cx, dragging, motion.press, motion.press_easing);
        drive(&mut self.focus, cx, focus_visible, motion.focus, motion.easing);
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = &theme.slider;
        let now = cx.now();
        let (ring, focus) = (self.ring.value(now), self.focus.value(now));
        let k = if self.enabled { 1.0 } else { theme.disabled_opacity };

        let b = cx.bounds();
        let (kr, cy) = (look.knob_size / 2.0, b.center().y);
        let knob_x = self.knob_x(b, theme);

        // 轨道：药丸形，已填充部分从左端到滑块中心。
        let th = look.track_height;
        let track = Rect::new(b.x, cy - th / 2.0, b.width, th);
        p.fill_rounded_rect(track, th / 2.0, look.track.fade(k));
        if look.track_border.a > 0 {
            p.stroke_rect(track, th / 2.0, 0.0, look.track_border.fade(k));
        }
        p.fill_rounded_rect(
            Rect::new(track.x, track.y, knob_x - track.x, th),
            th / 2.0,
            look.fill.fade(k),
        );

        // 滑块：阴影、实心圆、细边。
        let knob = Rect::new(knob_x - kr, cy - kr, look.knob_size, look.knob_size);
        let shadow = Shadow {
            color: look.knob_shadow.color.fade(k),
            ..look.knob_shadow
        };
        p.draw_shadow(knob, kr, &shadow);
        p.fill_rounded_rect(knob, kr, look.knob.fade(k));
        if look.knob_border.a > 0 {
            p.stroke_rect(knob, kr, 0.0, look.knob_border.fade(k));
        }
        if ring > 0.0 {
            let o = look.knob_ring_gap + look.knob_ring_width;
            p.stroke_rect(
                knob.inflate(o, o),
                kr + o,
                look.knob_ring_width,
                look.knob_ring.fade(ring * k),
            );
        }
        focus_ring(p, theme, knob, kr, focus);
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        match event {
            Event::PointerDown {
                pos,
                button: PointerButton::Left,
                ..
            } if self.enabled => {
                let (bounds, theme) = (cx.bounds(), cx.theme());
                let kr = theme.slider.knob_size / 2.0;
                let dx = pos.x - self.knob_x(bounds, theme);
                let dy = pos.y - bounds.center().y;
                self.grab = if dx * dx + dy * dy <= kr * kr { dx } else { 0.0 };
                self.dragging = true;
                cx.set_handled();
                cx.request_paint();
                let v = self.value_at(pos.x - self.grab, bounds, theme);
                self.set(cx, v);
            }
            Event::PointerMove { pos } if self.dragging => {
                let v = self.value_at(pos.x - self.grab, cx.bounds(), cx.theme());
                self.set(cx, v);
            }
            Event::PointerUp {
                button: PointerButton::Left,
                ..
            } => {
                if self.dragging {
                    self.dragging = false;
                    cx.request_paint();
                }
                cx.set_handled();
            }
            Event::FocusLost | Event::WindowFocusChanged(false) if self.dragging => {
                self.dragging = false;
                cx.request_paint();
            }
            Event::Key(k) if k.pressed && self.enabled => self.key(cx, &k.key),
            _ => {}
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
            role: Role::Slider,
            label: None,
            value: Some(self.display().to_string()),
            disabled: !self.enabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_step_and_stays_in_range() {
        let s = slider::<()>(0.0).range(10.0, 20.0).step(2.5);
        assert_eq!(s.snap(12.4), 12.5);
        assert_eq!(s.snap(-5.0), 10.0);
        assert_eq!(s.snap(99.0), 20.0);
        // 终点不在网格上：指针拖到最右端得到最大的网格点 0.9，要到 1.0 只能按 End。
        let s = slider::<()>(0.0).range(0.0, 1.0).step(0.3);
        assert!((s.snap(1.0) - 0.9).abs() < 1e-6);
        // 连续取值：只限制范围。
        let s = slider::<()>(0.5);
        assert_eq!(s.snap(0.123), 0.123);
        assert_eq!(s.snap(f32::NAN), 0.0);
        // 反了的范围对调，非法参数忽略。
        let s = slider::<()>(0.0).range(5.0, 1.0).range(f32::NAN, 3.0);
        assert_eq!((s.min, s.max), (1.0, 5.0));
    }
}
