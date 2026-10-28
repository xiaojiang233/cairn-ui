//! 过渡动画：缓动曲线，以及随时间从一个值过渡到另一个值的 [`Transition`]。自己实现，不依赖动画库。
//!
//! 用法：控件为每种状态（悬停、按下、焦点……）保存一个 `Transition`，事件里用
//! [`Transition::animate_to`] 设定目标，绘制时用 [`Transition::value`] 取当前值去混合主题里的
//! 两端颜色 / 尺寸；还在过渡中就调用 [`crate::PaintCx::request_frame`] 要下一帧。

use std::time::{Duration, Instant};

/// 缓动曲线。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    Linear,
    /// CSS `ease-out`，即 `cubic-bezier(0, 0, 0.58, 1)`。
    EaseOut,
    /// CSS `cubic-bezier(x1, y1, x2, y2)`。`x1`、`x2` 会被限制在 0–1，保证曲线随时间单调前进。
    CubicBezier(f32, f32, f32, f32),
}

impl Easing {
    /// `cubic-bezier(0.16, 1, 0.3, 1)`：起步快、收尾很长，没有回弹。
    pub const SMOOTH: Easing = Easing::CubicBezier(0.16, 1.0, 0.3, 1.0);

    /// 时间进度 `t`（0–1）对应的数值进度。
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match *self {
            Easing::Linear => t,
            Easing::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
            Easing::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, t),
        }
    }
}

/// 起点 (0, 0)、终点 (1, 1) 的三次贝塞尔：先解 x(s) = t，再求 y(s)。
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    let bez = |s: f32, p1: f32, p2: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * p1 + 3.0 * u * s * s * p2 + s * s * s
    };
    let slope = |s: f32, p1: f32, p2: f32| {
        let u = 1.0 - s;
        3.0 * u * u * p1 + 6.0 * u * s * (p2 - p1) + 3.0 * s * s * (1.0 - p2)
    };
    // 先用牛顿迭代，导数太小或跑出区间时退回二分法。
    let mut s = t;
    for _ in 0..8 {
        let err = bez(s, x1, x2) - t;
        if err.abs() < 1e-5 {
            return bez(s, y1, y2);
        }
        let d = slope(s, x1, x2);
        if d.abs() < 1e-6 {
            break;
        }
        s -= err / d;
        if !(0.0..=1.0).contains(&s) {
            break;
        }
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    s = t;
    for _ in 0..40 {
        let x = bez(s, x1, x2);
        if (x - t).abs() < 1e-5 {
            break;
        }
        if x < t {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) / 2.0;
    }
    bez(s, y1, y2)
}

/// 随时间过渡的数值（通常在 0 和 1 之间，表示“悬停程度”这类状态）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    from: f32,
    to: f32,
    start: Option<Instant>,
    duration: Duration,
    easing: Easing,
}

impl Default for Transition {
    fn default() -> Self {
        Transition::new(0.0)
    }
}

impl Transition {
    /// 停在 `value` 的过渡。
    pub const fn new(value: f32) -> Self {
        Transition {
            from: value,
            to: value,
            start: None,
            duration: Duration::ZERO,
            easing: Easing::Linear,
        }
    }

    /// `now` 时刻的值。
    pub fn value(&self, now: Instant) -> f32 {
        let Some(start) = self.start else { return self.to };
        if self.duration.is_zero() {
            return self.to;
        }
        let t = now.saturating_duration_since(start).as_secs_f32() / self.duration.as_secs_f32();
        if t >= 1.0 {
            return self.to;
        }
        self.from + (self.to - self.from) * self.easing.apply(t)
    }

    /// 最终会停在的值。
    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.start
            .is_some_and(|s| now.saturating_duration_since(s) < self.duration && self.from != self.to)
    }

    /// 从 `now` 时刻的当前值开始，用 `duration` 过渡到 `to`。目标没变时什么也不做，返回 `false`。
    /// `duration` 为 0 时直接跳到目标。
    pub fn animate_to(&mut self, to: f32, now: Instant, duration: Duration, easing: Easing) -> bool {
        if to == self.to {
            return false;
        }
        self.from = self.value(now);
        self.to = to;
        self.start = Some(now);
        self.duration = duration;
        self.easing = easing;
        true
    }

    /// 立即跳到 `value`，没有过渡。
    pub fn set(&mut self, value: f32) {
        *self = Transition::new(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_curve_is_monotonic_and_front_loaded() {
        let e = Easing::SMOOTH;
        let mut last = 0.0;
        for i in 0..=100 {
            let y = e.apply(i as f32 / 100.0);
            assert!(y >= last - 1e-4 && y <= 1.0 + 1e-4, "t = {i}%: {y}");
            last = y;
        }
        // 这条曲线前段很快：10% 时间走完将近一半，30% 时已过 85%，没有超过终点（不回弹）。
        assert!(e.apply(0.1) > 0.45 && e.apply(0.1) < 0.55, "{}", e.apply(0.1));
        assert!(e.apply(0.3) > 0.85, "{}", e.apply(0.3));
        assert!((e.apply(1.0) - 1.0).abs() < 1e-6);
        // 线性贝塞尔等于线性。
        let lin = Easing::CubicBezier(0.25, 0.25, 0.75, 0.75);
        for t in [0.1f32, 0.3, 0.5, 0.9] {
            assert!((lin.apply(t) - t).abs() < 1e-3);
        }
    }

    #[test]
    fn transition_retargets_from_current_value() {
        let t0 = Instant::now();
        let d = Duration::from_millis(100);
        let mut tr = Transition::new(0.0);
        assert!(tr.animate_to(1.0, t0, d, Easing::Linear));
        assert!(!tr.animate_to(1.0, t0, d, Easing::Linear), "同一目标不重新开始");
        let mid = t0 + Duration::from_millis(50);
        assert!((tr.value(mid) - 0.5).abs() < 1e-3);
        assert!(tr.is_animating(mid));
        // 中途反向：从 0.5 开始往回走。
        tr.animate_to(0.0, mid, d, Easing::Linear);
        assert!((tr.value(mid) - 0.5).abs() < 1e-3);
        assert!((tr.value(mid + d) - 0.0).abs() < 1e-6);
        assert!(!tr.is_animating(mid + d));
        // 时长为 0（系统关闭了动画）：立即到位。
        tr.animate_to(1.0, mid, Duration::ZERO, Easing::Linear);
        assert_eq!(tr.value(mid), 1.0);
        assert!(!tr.is_animating(mid));
    }
}
