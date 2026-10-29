//! 进度条与加载指示。

use std::time::Instant;

use cairn_ui_layout::MeasureInput;

use crate::anim::{Easing, Transition};
use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::{Brush, Painter, Point, Rect, Size, Style};

/// 进度条。用 [`progress_bar`] 创建；宽度由布局决定（通常 `.fill_width()`），高度取主题的 `progress.height`。
///
/// 数值是 0–1 的完成比例，变化时按 [`crate::Motion::progress`] 平滑过渡；`None` 表示进度未知，
/// 一小段在轨道上来回移动。外观来自主题的 [`Theme::progress`]。
pub struct ProgressBar {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    value: Option<f32>,
    shown: Transition,
    started: bool,
    epoch: Option<Instant>,
}

pub fn progress_bar(value: Option<f32>) -> ProgressBar {
    ProgressBar {
        style: Style::default(),
        key: None,
        value: value.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }),
        shown: Transition::default(),
        started: false,
        epoch: None,
    }
}

element!(ProgressBar);

impl<S: 'static> Widget<S> for ProgressBar {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        if new.value == self.value {
            return Change::NONE;
        }
        self.value = new.value;
        Change::PAINT
    }

    fn measure(&mut self, cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        Size::new(input.known_width.unwrap_or(0.0), cx.theme().progress.height)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        match self.value {
            Some(v) => {
                self.epoch = None;
                if !self.started {
                    self.started = true;
                    self.shown.set(v);
                } else {
                    let m = cx.theme().motion;
                    self.shown.animate_to(v, now, m.progress, Easing::EaseOut);
                }
                if self.shown.is_animating(now) {
                    cx.request_frame();
                }
            }
            None => {
                self.epoch.get_or_insert(now);
                cx.request_frame();
            }
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = &theme.progress;
        let b = cx.content();
        let h = look.height.min(b.height);
        let track = Rect::new(b.x, b.y + (b.height - h) / 2.0, b.width, h);
        let r = h / 2.0;
        p.fill_rounded_rect(track, r, look.track);
        let (x0, x1) = match (self.value, self.epoch) {
            (Some(_), _) => (0.0, self.shown.value(cx.now())),
            (None, Some(epoch)) => {
                let period = look.indeterminate_period.as_secs_f32().max(0.1);
                let t = (cx.now().saturating_duration_since(epoch).as_secs_f32() / period).fract();
                // 往返：0→1→0，两端放慢。
                let k = Easing::EASE_IN_OUT.apply(1.0 - (2.0 * t - 1.0).abs());
                let w = look.indeterminate_width.clamp(0.05, 1.0);
                let x = k * (1.0 - w);
                (x, x + w)
            }
            (None, None) => (0.0, 0.0),
        };
        let w = (x1 - x0) * track.width;
        if w <= 0.0 {
            return;
        }
        let bar = Rect::new(track.x + x0 * track.width, track.y, w.max(h), h);
        let brush = Brush::linear(
            Point::new(bar.x, bar.y),
            Point::new(bar.right(), bar.y),
            [(0.0, look.fill), (1.0, look.fill_end)],
        );
        p.push_rounded_clip(track, r);
        p.fill(bar, r, &brush);
        p.pop_clip();
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::ProgressBar,
            value: self.value.map(|v| format!("{:.0}%", v * 100.0)),
            ..Semantics::default()
        }
    }
}

/// 转圈的加载指示。用 [`spinner`] 创建，直径、颜色、速度取主题的 `progress.spinner*`。
pub struct Spinner {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    epoch: Option<Instant>,
}

pub fn spinner() -> Spinner {
    Spinner {
        style: Style::default(),
        key: None,
        epoch: None,
    }
}

element!(Spinner);

/// 一圈的辐条数。
const SPOKES: usize = 10;

impl<S: 'static> Widget<S> for Spinner {
    fn update(&mut self, _cx: &mut LayoutCx, _new: Self) -> Change {
        Change::NONE
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let d = cx.theme().progress.spinner_size;
        Size::new(d, d)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        self.epoch.get_or_insert(cx.now());
        cx.request_frame();
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme: &Theme = cx.theme();
        let look = &theme.progress;
        let b = cx.content();
        let d = look.spinner_size.min(b.width).min(b.height);
        let c = b.center();
        let period = look.spinner_period.as_secs_f32().max(0.1);
        let elapsed = self
            .epoch
            .map(|e| cx.now().saturating_duration_since(e).as_secs_f32())
            .unwrap_or(0.0);
        let head = ((elapsed / period).fract() * SPOKES as f32) as usize;
        let w = (d * 0.11).max(1.25);
        for i in 0..SPOKES {
            let a = i as f32 / SPOKES as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let (s, co) = a.sin_cos();
            // 从“头”往后逐渐变淡。
            let age = (head + SPOKES - i) % SPOKES;
            let alpha = 1.0 - age as f32 / SPOKES as f32 * 0.85;
            let (r0, r1) = (d * 0.25, d * 0.47);
            p.draw_line(
                Point::new(c.x + co * r0, c.y + s * r0),
                Point::new(c.x + co * r1, c.y + s * r1),
                w,
                look.spinner.fade(alpha),
            );
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::ProgressBar,
            label: Some("loading".into()),
            ..Semantics::default()
        }
    }
}
