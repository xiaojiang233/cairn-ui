//! 简单的几何图标：用画线、画多边形、画圆拼出来，不依赖 SVG 或图标字体。

use crate::{Painter, Point, Rect};

/// 内置图标。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// 实心小圆点。
    Dot,
    Plus,
    Minus,
    Close,
    Check,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    ChevronDown,
    /// 播放 / 启动（实心三角）。
    Play,
    /// 三条横线。
    Menu,
    Search,
}

impl Icon {
    /// 英文名称，用作无障碍名称的默认值。
    pub fn name(self) -> &'static str {
        match self {
            Icon::Dot => "dot",
            Icon::Plus => "add",
            Icon::Minus => "remove",
            Icon::Close => "close",
            Icon::Check => "check",
            Icon::ChevronLeft => "back",
            Icon::ChevronRight => "forward",
            Icon::ChevronUp => "up",
            Icon::ChevronDown => "down",
            Icon::Play => "play",
            Icon::Menu => "menu",
            Icon::Search => "search",
        }
    }
}

/// 在正方形 `rect` 里画图标。线宽随图标大小变化（16 像素的图标约 1.6 像素）。
pub fn paint_icon(p: &mut dyn Painter, icon: Icon, rect: Rect, color: crate::Color) {
    let d = rect.width.min(rect.height);
    let c = rect.center();
    let w = (d * 0.1).max(1.25);
    let at = |x: f32, y: f32| Point::new(c.x + x * d, c.y + y * d);
    let line = |p: &mut dyn Painter, a: Point, b: Point| p.draw_line(a, b, w, color);
    match icon {
        Icon::Dot => {
            let r = d * 0.25;
            p.fill_rounded_rect(Rect::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r), r, color);
        }
        Icon::Plus => {
            line(p, at(-0.36, 0.0), at(0.36, 0.0));
            line(p, at(0.0, -0.36), at(0.0, 0.36));
        }
        Icon::Minus => line(p, at(-0.36, 0.0), at(0.36, 0.0)),
        Icon::Close => {
            line(p, at(-0.28, -0.28), at(0.28, 0.28));
            line(p, at(0.28, -0.28), at(-0.28, 0.28));
        }
        Icon::Check => {
            line(p, at(-0.32, 0.02), at(-0.1, 0.24));
            line(p, at(-0.1, 0.24), at(0.34, -0.22));
        }
        Icon::ChevronLeft => {
            line(p, at(0.1, -0.3), at(-0.18, 0.0));
            line(p, at(-0.18, 0.0), at(0.1, 0.3));
        }
        Icon::ChevronRight => {
            line(p, at(-0.1, -0.3), at(0.18, 0.0));
            line(p, at(0.18, 0.0), at(-0.1, 0.3));
        }
        Icon::ChevronUp => {
            line(p, at(-0.3, 0.1), at(0.0, -0.18));
            line(p, at(0.0, -0.18), at(0.3, 0.1));
        }
        Icon::ChevronDown => {
            line(p, at(-0.3, -0.1), at(0.0, 0.18));
            line(p, at(0.0, 0.18), at(0.3, -0.1));
        }
        Icon::Play => p.fill_polygon(&[at(-0.22, -0.32), at(0.34, 0.0), at(-0.22, 0.32)], color),
        Icon::Menu => {
            for y in [-0.24, 0.0, 0.24] {
                line(p, at(-0.34, y), at(0.34, y));
            }
        }
        Icon::Search => {
            let r = d * 0.26;
            let o = at(-0.08, -0.08);
            p.stroke_rect(Rect::new(o.x - r, o.y - r, 2.0 * r, 2.0 * r), r, w, color);
            line(p, at(0.14, 0.14), at(0.36, 0.36));
        }
    }
}
