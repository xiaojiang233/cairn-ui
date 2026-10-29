//! 简单的几何图标：用画线、画多边形、画圆拼出来，不依赖 SVG 或图标字体。

use crate::{Color, Painter, Point, Rect};

/// 内置图标。都是线条风格（少数是实心的小图形），颜色由调用方给出。
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
    /// 立方体。
    Cube,
    /// 图片：画框里有山和太阳。
    Image,
    Sun,
    Globe,
    Camera,
    Trash,
    Download,
    Folder,
    /// 房子。
    Home,
    Gamepad,
    /// 收纳箱。
    Package,
    /// 齿轮（设置）。
    Gear,
    Rocket,
    User,
    /// 循环箭头（刷新、更新）。
    Refresh,
    Copy,
    /// 在外部打开（方框加斜向箭头）。
    ExternalLink,
    /// 圆圈里的 i。
    Info,
    /// 三角形里的感叹号。
    Warning,
    /// 实心方块（停止）。
    Stop,
    /// 横向三个点。
    More,
    /// 芯片（内存）。
    Chip,
    /// 杯子。
    Cup,
    /// 窗口按钮：最小化、最大化、还原。线条比其他图标细，与系统窗口按钮一致。
    WindowMinimize,
    WindowMaximize,
    WindowRestore,
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
            Icon::Cube => "cube",
            Icon::Image => "image",
            Icon::Sun => "sun",
            Icon::Globe => "globe",
            Icon::Camera => "camera",
            Icon::Trash => "delete",
            Icon::Download => "download",
            Icon::Folder => "folder",
            Icon::Home => "home",
            Icon::Gamepad => "gamepad",
            Icon::Package => "package",
            Icon::Gear => "settings",
            Icon::Rocket => "launch",
            Icon::User => "account",
            Icon::Refresh => "refresh",
            Icon::Copy => "copy",
            Icon::ExternalLink => "open externally",
            Icon::Info => "info",
            Icon::Warning => "warning",
            Icon::Stop => "stop",
            Icon::More => "more",
            Icon::Chip => "memory",
            Icon::Cup => "cup",
            Icon::WindowMinimize => "minimize",
            Icon::WindowMaximize => "maximize",
            Icon::WindowRestore => "restore",
        }
    }
}

/// 在正方形 `rect` 里画图标。线宽随图标大小变化（16 像素的图标约 1.6 像素）。
pub fn paint_icon(p: &mut dyn Painter, icon: Icon, rect: Rect, color: Color) {
    let d = rect.width.min(rect.height);
    let c = rect.center();
    let w = (d * 0.1).max(1.25);
    let at = |x: f32, y: f32| Point::new(c.x + x * d, c.y + y * d);
    let line = |p: &mut dyn Painter, a: Point, b: Point| p.draw_line(a, b, w, color);
    // 依次连接各点（单位坐标）。
    let poly = |p: &mut dyn Painter, pts: &[(f32, f32)], width: f32| {
        for s in pts.windows(2) {
            p.draw_line(at(s[0].0, s[0].1), at(s[1].0, s[1].1), width, color);
        }
    };
    let ring = |p: &mut dyn Painter, x: f32, y: f32, r: f32, width: f32| {
        let o = at(x, y);
        let r = r * d;
        p.stroke_rect(Rect::new(o.x - r, o.y - r, 2.0 * r, 2.0 * r), r, width, color);
    };
    let disc = |p: &mut dyn Painter, x: f32, y: f32, r: f32| {
        let o = at(x, y);
        let r = r * d;
        p.fill_rounded_rect(Rect::new(o.x - r, o.y - r, 2.0 * r, 2.0 * r), r, color);
    };
    match icon {
        Icon::Dot => disc(p, 0.0, 0.0, 0.25),
        Icon::Plus => {
            line(p, at(-0.36, 0.0), at(0.36, 0.0));
            line(p, at(0.0, -0.36), at(0.0, 0.36));
        }
        Icon::Minus => line(p, at(-0.36, 0.0), at(0.36, 0.0)),
        Icon::Close => {
            line(p, at(-0.28, -0.28), at(0.28, 0.28));
            line(p, at(0.28, -0.28), at(-0.28, 0.28));
        }
        Icon::Check => poly(p, &[(-0.32, 0.02), (-0.1, 0.24), (0.34, -0.22)], w),
        Icon::ChevronLeft => poly(p, &[(0.1, -0.3), (-0.18, 0.0), (0.1, 0.3)], w),
        Icon::ChevronRight => poly(p, &[(-0.1, -0.3), (0.18, 0.0), (-0.1, 0.3)], w),
        Icon::ChevronUp => poly(p, &[(-0.3, 0.1), (0.0, -0.18), (0.3, 0.1)], w),
        Icon::ChevronDown => poly(p, &[(-0.3, -0.1), (0.0, 0.18), (0.3, -0.1)], w),
        Icon::Play => p.fill_polygon(&[at(-0.22, -0.32), at(0.34, 0.0), at(-0.22, 0.32)], color),
        Icon::Menu => {
            for y in [-0.24, 0.0, 0.24] {
                line(p, at(-0.34, y), at(0.34, y));
            }
        }
        Icon::Search => {
            ring(p, -0.08, -0.08, 0.26, w);
            line(p, at(0.14, 0.14), at(0.36, 0.36));
        }
        Icon::Cube => {
            let hex = [
                (0.0, -0.4),
                (0.35, -0.2),
                (0.35, 0.2),
                (0.0, 0.4),
                (-0.35, 0.2),
                (-0.35, -0.2),
                (0.0, -0.4),
            ];
            poly(p, &hex, w);
            poly(p, &[(-0.35, -0.2), (0.0, 0.0), (0.35, -0.2)], w);
            line(p, at(0.0, 0.0), at(0.0, 0.4));
        }
        Icon::Image => {
            let o = at(-0.38, -0.3);
            p.stroke_rect(Rect::new(o.x, o.y, 0.76 * d, 0.6 * d), 0.08 * d, w, color);
            poly(
                p,
                &[(-0.26, 0.18), (-0.07, -0.04), (0.06, 0.09), (0.15, 0.0), (0.27, 0.18)],
                w,
            );
            disc(p, 0.15, -0.13, 0.06);
        }
        Icon::Sun => {
            ring(p, 0.0, 0.0, 0.17, w);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let (s, co) = a.sin_cos();
                line(p, at(co * 0.29, s * 0.29), at(co * 0.4, s * 0.4));
            }
        }
        Icon::Globe => {
            ring(p, 0.0, 0.0, 0.38, w);
            let o = at(-0.15, -0.38);
            p.stroke_rect(Rect::new(o.x, o.y, 0.3 * d, 0.76 * d), 0.15 * d, w, color);
            line(p, at(-0.38, 0.0), at(0.38, 0.0));
        }
        Icon::Camera => {
            let o = at(-0.4, -0.22);
            p.stroke_rect(Rect::new(o.x, o.y, 0.8 * d, 0.54 * d), 0.08 * d, w, color);
            poly(p, &[(-0.14, -0.22), (-0.08, -0.33), (0.08, -0.33), (0.14, -0.22)], w);
            ring(p, 0.0, 0.05, 0.13, w);
        }
        Icon::Trash => {
            line(p, at(-0.36, -0.24), at(0.36, -0.24));
            poly(p, &[(-0.12, -0.24), (-0.1, -0.36), (0.1, -0.36), (0.12, -0.24)], w);
            poly(p, &[(-0.27, -0.24), (-0.22, 0.38), (0.22, 0.38), (0.27, -0.24)], w);
            line(p, at(-0.08, -0.08), at(-0.07, 0.22));
            line(p, at(0.08, -0.08), at(0.07, 0.22));
        }
        Icon::Download => {
            line(p, at(0.0, -0.38), at(0.0, 0.12));
            poly(p, &[(-0.18, -0.06), (0.0, 0.12), (0.18, -0.06)], w);
            poly(p, &[(-0.36, 0.12), (-0.36, 0.36), (0.36, 0.36), (0.36, 0.12)], w);
        }
        Icon::Folder => poly(
            p,
            &[
                (-0.4, 0.32),
                (-0.4, -0.3),
                (-0.12, -0.3),
                (-0.04, -0.2),
                (0.4, -0.2),
                (0.4, 0.32),
                (-0.4, 0.32),
            ],
            w,
        ),
        Icon::Home => {
            poly(p, &[(-0.42, -0.04), (0.0, -0.4), (0.42, -0.04)], w);
            poly(p, &[(-0.3, -0.08), (-0.3, 0.36), (0.3, 0.36), (0.3, -0.08)], w);
            poly(p, &[(-0.08, 0.36), (-0.08, 0.1), (0.08, 0.1), (0.08, 0.36)], w);
        }
        Icon::Gamepad => {
            let o = at(-0.42, -0.22);
            p.stroke_rect(Rect::new(o.x, o.y, 0.84 * d, 0.48 * d), 0.2 * d, w, color);
            line(p, at(-0.3, 0.02), at(-0.1, 0.02));
            line(p, at(-0.2, -0.08), at(-0.2, 0.12));
            disc(p, 0.14, -0.03, 0.05);
            disc(p, 0.26, 0.07, 0.05);
        }
        Icon::Package => {
            let o = at(-0.4, -0.34);
            p.stroke_rect(Rect::new(o.x, o.y, 0.8 * d, 0.2 * d), 0.04 * d, w, color);
            poly(p, &[(-0.33, -0.14), (-0.33, 0.36), (0.33, 0.36), (0.33, -0.14)], w);
            line(p, at(-0.1, 0.03), at(0.1, 0.03));
        }
        Icon::Gear => {
            // 六个齿从环里伸出来、与环连成一体，小尺寸下也看得出是齿轮。
            for k in 0..6 {
                let a = std::f32::consts::FRAC_PI_6 + k as f32 * std::f32::consts::FRAC_PI_3;
                let (s, co) = a.sin_cos();
                p.draw_line(at(co * 0.24, s * 0.24), at(co * 0.4, s * 0.4), w * 2.1, color);
            }
            ring(p, 0.0, 0.0, 0.27, w * 1.5);
            ring(p, 0.0, 0.0, 0.08, w);
        }
        Icon::Rocket => {
            // 先按“竖直朝上”设计，再整体顺时针转 45°，让火箭朝右上方飞。
            let (s, co) = std::f32::consts::FRAC_PI_4.sin_cos();
            let rot = |(x, y): (f32, f32)| (x * co - y * s, x * s + y * co);
            let rp = |p: &mut dyn Painter, pts: &[(f32, f32)]| {
                let pts: Vec<(f32, f32)> = pts.iter().map(|&q| rot(q)).collect();
                poly(p, &pts, w);
            };
            rp(
                p,
                &[
                    (0.0, -0.44),
                    (0.12, -0.27),
                    (0.13, 0.1),
                    (-0.13, 0.1),
                    (-0.12, -0.27),
                    (0.0, -0.44),
                ],
            );
            rp(p, &[(-0.13, -0.02), (-0.26, 0.14), (-0.13, 0.1)]);
            rp(p, &[(0.13, -0.02), (0.26, 0.14), (0.13, 0.1)]);
            rp(p, &[(0.0, 0.18), (0.0, 0.32)]);
            let (x, y) = rot((0.0, -0.15));
            ring(p, x, y, 0.06, w);
        }
        Icon::User => {
            ring(p, 0.0, -0.15, 0.16, w);
            arc(p, at(0.0, 0.4), 0.3 * d, 200.0, 340.0, w, color);
        }
        Icon::Refresh => {
            let (from, to, r) = (300.0f32, 600.0f32, 0.3 * d);
            arc(p, c, r, from, to, w, color);
            let t = to.to_radians();
            let (sn, cs) = t.sin_cos();
            let tip = Point::new(c.x + r * cs, c.y + r * sn);
            let (tx, ty) = (-sn, cs);
            let (a, b) = (0.15 * d, 0.11 * d);
            p.draw_line(
                tip,
                Point::new(tip.x - tx * a + cs * b, tip.y - ty * a + sn * b),
                w,
                color,
            );
            p.draw_line(
                tip,
                Point::new(tip.x - tx * a - cs * b, tip.y - ty * a - sn * b),
                w,
                color,
            );
        }
        Icon::Copy => {
            let o = at(-0.18, -0.12);
            p.stroke_rect(Rect::new(o.x, o.y, 0.54 * d, 0.52 * d), 0.08 * d, w, color);
            poly(p, &[(-0.36, 0.2), (-0.36, -0.36), (0.18, -0.36)], w);
        }
        Icon::ExternalLink => {
            poly(
                p,
                &[
                    (0.04, -0.36),
                    (-0.36, -0.36),
                    (-0.36, 0.36),
                    (0.36, 0.36),
                    (0.36, -0.04),
                ],
                w,
            );
            line(p, at(-0.04, 0.04), at(0.36, -0.36));
            poly(p, &[(0.12, -0.36), (0.36, -0.36), (0.36, -0.12)], w);
        }
        Icon::Info => {
            ring(p, 0.0, 0.0, 0.4, w);
            disc(p, 0.0, -0.18, 0.055);
            line(p, at(0.0, -0.04), at(0.0, 0.22));
        }
        Icon::Warning => {
            poly(p, &[(0.0, -0.38), (0.4, 0.33), (-0.4, 0.33), (0.0, -0.38)], w);
            line(p, at(0.0, -0.1), at(0.0, 0.09));
            disc(p, 0.0, 0.2, 0.05);
        }
        Icon::Stop => {
            let o = at(-0.24, -0.24);
            p.fill_rounded_rect(Rect::new(o.x, o.y, 0.48 * d, 0.48 * d), 0.06 * d, color);
        }
        Icon::More => {
            for x in [-0.28, 0.0, 0.28] {
                disc(p, x, 0.0, 0.065);
            }
        }
        Icon::Chip => {
            let o = at(-0.24, -0.24);
            p.stroke_rect(Rect::new(o.x, o.y, 0.48 * d, 0.48 * d), 0.06 * d, w, color);
            for i in [-0.11, 0.11] {
                line(p, at(i, -0.24), at(i, -0.38));
                line(p, at(i, 0.24), at(i, 0.38));
                line(p, at(-0.24, i), at(-0.38, i));
                line(p, at(0.24, i), at(0.38, i));
            }
        }
        Icon::Cup => {
            poly(
                p,
                &[(-0.3, -0.1), (-0.26, 0.3), (0.16, 0.3), (0.2, -0.1), (-0.3, -0.1)],
                w,
            );
            arc(p, at(0.22, 0.08), 0.11 * d, 270.0, 450.0, w, color);
            line(p, at(-0.12, -0.38), at(-0.12, -0.22));
            line(p, at(0.04, -0.38), at(0.04, -0.22));
        }
        Icon::WindowMinimize | Icon::WindowMaximize | Icon::WindowRestore => {
            let thin = (d * 0.09).max(1.0);
            match icon {
                Icon::WindowMinimize => p.draw_line(at(-0.5, 0.0), at(0.5, 0.0), thin, color),
                Icon::WindowMaximize => {
                    let o = at(-0.5, -0.5);
                    p.stroke_rect(Rect::new(o.x, o.y, d, d), 0.12 * d, thin, color);
                }
                _ => {
                    let o = at(-0.5, -0.3);
                    p.stroke_rect(Rect::new(o.x, o.y, 0.8 * d, 0.8 * d), 0.1 * d, thin, color);
                    poly(
                        p,
                        &[(-0.3, -0.3), (-0.3, -0.5), (0.5, -0.5), (0.5, 0.3), (0.3, 0.3)],
                        thin,
                    );
                }
            }
        }
    }
}

/// 圆弧：圆心 `c`、半径 `r`，从 `from` 度顺时针画到 `to` 度（0 度指向右，屏幕坐标 y 向下）。
/// 用短线段拼成，每段不超过 15 度。
fn arc(p: &mut dyn Painter, c: Point, r: f32, from: f32, to: f32, width: f32, color: Color) {
    let n = ((to - from).abs() / 15.0).ceil().max(1.0) as usize;
    let pt = |deg: f32| {
        let (s, co) = deg.to_radians().sin_cos();
        Point::new(c.x + r * co, c.y + r * s)
    };
    let mut prev = pt(from);
    for i in 1..=n {
        let next = pt(from + (to - from) * i as f32 / n as f32);
        p.draw_line(prev, next, width, color);
        prev = next;
    }
}
