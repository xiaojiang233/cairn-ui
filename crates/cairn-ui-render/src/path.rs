//! 矢量路径：直线、二次与三次贝塞尔曲线组成的轮廓，可以从 SVG 路径数据解析。
//!
//! 路径用自己的“设计坐标”描述（例如 0–64 的方框），画的时候由调用方给出一个 [`Affine`]
//! 把它映射到窗口里——同一条路径可以画成任意大小，边缘始终是矢量抗锯齿的。

use std::f32::consts::PI;
use std::fmt;

use crate::geom::{Affine, Point, Rect};

/// 路径的一段。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    /// 开始一条新的子路径。
    Move(Point),
    Line(Point),
    /// 二次贝塞尔：控制点、终点。
    Quad(Point, Point),
    /// 三次贝塞尔：两个控制点、终点。
    Cubic(Point, Point, Point),
    /// 闭合当前子路径。
    Close,
}

/// 一条矢量路径。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    segs: Vec<Seg>,
}

/// 圆弧用三次贝塞尔近似时的控制点系数（四分之一圆）。
const KAPPA: f32 = 0.552_284_8;

impl Path {
    pub fn new() -> Self {
        Path::default()
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    pub fn segments(&self) -> &[Seg] {
        &self.segs
    }

    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.segs.push(Seg::Move(Point::new(x, y)));
        self
    }

    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.segs.push(Seg::Line(Point::new(x, y)));
        self
    }

    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) -> &mut Self {
        self.segs.push(Seg::Quad(Point::new(cx, cy), Point::new(x, y)));
        self
    }

    pub fn cubic_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> &mut Self {
        self.segs
            .push(Seg::Cubic(Point::new(c1x, c1y), Point::new(c2x, c2y), Point::new(x, y)));
        self
    }

    pub fn close(&mut self) -> &mut Self {
        self.segs.push(Seg::Close);
        self
    }

    /// 矩形（顺时针）。
    pub fn rect(r: Rect) -> Path {
        let mut p = Path::new();
        p.move_to(r.x, r.y)
            .line_to(r.right(), r.y)
            .line_to(r.right(), r.bottom())
            .line_to(r.x, r.bottom())
            .close();
        p
    }

    /// 圆角矩形（顺时针），半径超过边长一半时按一半算。
    pub fn rounded_rect(r: Rect, radius: f32) -> Path {
        let rad = radius.min(r.width / 2.0).min(r.height / 2.0).max(0.0);
        if rad < 0.001 {
            return Path::rect(r);
        }
        let k = rad * (1.0 - KAPPA);
        let (l, t, rr, b) = (r.x, r.y, r.right(), r.bottom());
        let mut p = Path::new();
        p.move_to(l + rad, t)
            .line_to(rr - rad, t)
            .cubic_to(rr - k, t, rr, t + k, rr, t + rad)
            .line_to(rr, b - rad)
            .cubic_to(rr, b - k, rr - k, b, rr - rad, b)
            .line_to(l + rad, b)
            .cubic_to(l + k, b, l, b - k, l, b - rad)
            .line_to(l, t + rad)
            .cubic_to(l, t + k, l + k, t, l + rad, t)
            .close();
        p
    }

    /// 椭圆（顺时针，从右端点开始）。
    pub fn ellipse(center: Point, rx: f32, ry: f32) -> Path {
        let (cx, cy) = (center.x, center.y);
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        let mut p = Path::new();
        p.move_to(cx + rx, cy)
            .cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry)
            .cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy)
            .cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry)
            .cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy)
            .close();
        p
    }

    pub fn circle(center: Point, radius: f32) -> Path {
        Path::ellipse(center, radius, radius)
    }

    /// 全部点（含控制点）的外接矩形，没有任何点时为空矩形。
    /// 曲线可能比它小，但一定不会超出它。
    pub fn bounds(&self) -> Rect {
        let mut min = (f32::MAX, f32::MAX);
        let mut max = (f32::MIN, f32::MIN);
        let mut any = false;
        let mut add = |p: &Point| {
            any = true;
            min = (min.0.min(p.x), min.1.min(p.y));
            max = (max.0.max(p.x), max.1.max(p.y));
        };
        for s in &self.segs {
            match s {
                Seg::Move(a) | Seg::Line(a) => add(a),
                Seg::Quad(a, b) => {
                    add(a);
                    add(b);
                }
                Seg::Cubic(a, b, c) => {
                    add(a);
                    add(b);
                    add(c);
                }
                Seg::Close => {}
            }
        }
        if any {
            Rect::from_ltrb(min.0, min.1, max.0, max.1)
        } else {
            Rect::ZERO
        }
    }

    /// 变换后的副本。
    pub fn transformed(&self, at: Affine) -> Path {
        let segs = self
            .segs
            .iter()
            .map(|s| match s {
                Seg::Move(a) => Seg::Move(at.apply(*a)),
                Seg::Line(a) => Seg::Line(at.apply(*a)),
                Seg::Quad(a, b) => Seg::Quad(at.apply(*a), at.apply(*b)),
                Seg::Cubic(a, b, c) => Seg::Cubic(at.apply(*a), at.apply(*b), at.apply(*c)),
                Seg::Close => Seg::Close,
            })
            .collect();
        Path { segs }
    }

    /// 解析 SVG 路径数据（`<path>` 的 `d` 属性）。
    ///
    /// 支持 `M m L l H h V v C c S s Q q T t A a Z z`，数字之间的分隔符、省略的重复命令、
    /// 紧凑的圆弧标志位（`a1 1 0 011 1`）都按 SVG 规范处理。圆弧会换成三次贝塞尔。
    pub fn parse(d: &str) -> Result<Path, PathParseError> {
        Parser { s: d.as_bytes(), i: 0 }.run()
    }
}

/// 路径数据解析失败。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathParseError {
    /// 出错位置（字节偏移）。
    pub offset: usize,
    pub message: &'static str,
}

impl fmt::Display for PathParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "路径数据第 {} 个字符处出错：{}", self.offset, self.message)
    }
}

impl std::error::Error for PathParseError {}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err<T>(&self, message: &'static str) -> Result<T, PathParseError> {
        Err(PathParseError {
            offset: self.i,
            message,
        })
    }

    fn skip_sep(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<f32, PathParseError> {
        self.skip_sep();
        let start = self.i;
        if self.i < self.s.len() && (self.s[self.i] == b'+' || self.s[self.i] == b'-') {
            self.i += 1;
        }
        let int = self.digits();
        let mut frac = 0;
        if self.i < self.s.len() && self.s[self.i] == b'.' {
            self.i += 1;
            frac = self.digits();
        }
        if int + frac == 0 {
            self.i = start;
            return self.err("需要一个数字");
        }
        if self.i < self.s.len() && (self.s[self.i] == b'e' || self.s[self.i] == b'E') {
            let save = self.i;
            self.i += 1;
            if self.i < self.s.len() && (self.s[self.i] == b'+' || self.s[self.i] == b'-') {
                self.i += 1;
            }
            if self.digits() == 0 {
                // 不是指数（后面没有数字），退回去。
                self.i = save;
            }
        }
        match std::str::from_utf8(&self.s[start..self.i])
            .ok()
            .and_then(|t| t.parse::<f32>().ok())
        {
            Some(v) => Ok(v),
            None => self.err("数字格式不对"),
        }
    }

    fn flag(&mut self) -> Result<bool, PathParseError> {
        self.skip_sep();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => self.err("圆弧标志位必须是 0 或 1"),
        }
    }

    fn point(&mut self, rel: bool, cur: Point) -> Result<Point, PathParseError> {
        let (x, y) = (self.number()?, self.number()?);
        Ok(if rel {
            Point::new(cur.x + x, cur.y + y)
        } else {
            Point::new(x, y)
        })
    }

    fn run(mut self) -> Result<Path, PathParseError> {
        let mut path = Path::new();
        let mut cur = Point::ZERO;
        let mut start = Point::ZERO;
        // 上一条曲线的控制点（S / T 用它做镜像）。
        let mut last_cubic: Option<Point> = None;
        let mut last_quad: Option<Point> = None;
        let mut cmd: Option<u8> = None;
        loop {
            self.skip_sep();
            let Some(&c) = self.s.get(self.i) else { break };
            if c.is_ascii_alphabetic() {
                cmd = Some(c);
                self.i += 1;
                if c == b'Z' || c == b'z' {
                    path.close();
                    cur = start;
                    last_cubic = None;
                    last_quad = None;
                    cmd = None;
                    continue;
                }
            } else if cmd.is_none() {
                return self.err("路径数据必须以命令开头");
            }
            let Some(c) = cmd else {
                return self.err("多余的数字");
            };
            let rel = c.is_ascii_lowercase();
            match c.to_ascii_uppercase() {
                b'M' => {
                    let p = self.point(rel, cur)?;
                    path.move_to(p.x, p.y);
                    cur = p;
                    start = p;
                    // 后面跟着的坐标对是隐含的 L。
                    cmd = Some(if rel { b'l' } else { b'L' });
                    last_cubic = None;
                    last_quad = None;
                }
                b'L' => {
                    let p = self.point(rel, cur)?;
                    path.line_to(p.x, p.y);
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                b'H' => {
                    let x = self.number()?;
                    cur = Point::new(if rel { cur.x + x } else { x }, cur.y);
                    path.line_to(cur.x, cur.y);
                    last_cubic = None;
                    last_quad = None;
                }
                b'V' => {
                    let y = self.number()?;
                    cur = Point::new(cur.x, if rel { cur.y + y } else { y });
                    path.line_to(cur.x, cur.y);
                    last_cubic = None;
                    last_quad = None;
                }
                b'C' => {
                    let (a, b, p) = (self.point(rel, cur)?, self.point(rel, cur)?, self.point(rel, cur)?);
                    path.cubic_to(a.x, a.y, b.x, b.y, p.x, p.y);
                    cur = p;
                    last_cubic = Some(b);
                    last_quad = None;
                }
                b'S' => {
                    let a = reflect(last_cubic, cur);
                    let (b, p) = (self.point(rel, cur)?, self.point(rel, cur)?);
                    path.cubic_to(a.x, a.y, b.x, b.y, p.x, p.y);
                    cur = p;
                    last_cubic = Some(b);
                    last_quad = None;
                }
                b'Q' => {
                    let (a, p) = (self.point(rel, cur)?, self.point(rel, cur)?);
                    path.quad_to(a.x, a.y, p.x, p.y);
                    cur = p;
                    last_quad = Some(a);
                    last_cubic = None;
                }
                b'T' => {
                    let a = reflect(last_quad, cur);
                    let p = self.point(rel, cur)?;
                    path.quad_to(a.x, a.y, p.x, p.y);
                    cur = p;
                    last_quad = Some(a);
                    last_cubic = None;
                }
                b'A' => {
                    let (rx, ry, rot) = (self.number()?, self.number()?, self.number()?);
                    let (large, sweep) = (self.flag()?, self.flag()?);
                    let p = self.point(rel, cur)?;
                    arc_to(&mut path, cur, rx, ry, rot, large, sweep, p);
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                _ => return self.err("不认识的命令"),
            }
        }
        Ok(path)
    }
}

/// 上一个控制点关于当前点的镜像；上一段不是同类曲线时就是当前点本身。
fn reflect(ctrl: Option<Point>, cur: Point) -> Point {
    match ctrl {
        Some(c) => Point::new(2.0 * cur.x - c.x, 2.0 * cur.y - c.y),
        None => cur,
    }
}

/// 两个向量之间的有向夹角。
fn angle(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
    (ux * vy - uy * vx).atan2(ux * vx + uy * vy)
}

/// SVG 圆弧（端点参数）换成三次贝塞尔，追加到 `path`。算法见 SVG 规范附录 F.6。
#[allow(clippy::too_many_arguments)]
fn arc_to(path: &mut Path, from: Point, rx: f32, ry: f32, rotation: f32, large: bool, sweep: bool, to: Point) {
    if from == to {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-6 || ry < 1e-6 {
        path.line_to(to.x, to.y);
        return;
    }
    let phi = rotation.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let (dx2, dy2) = ((from.x - to.x) / 2.0, (from.y - to.y) / 2.0);
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;
    // 半径太小够不着终点时按比例放大。
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let (rx2, ry2) = (rx * rx, ry * ry);
    let num = rx2 * ry2 - rx2 * y1p * y1p - ry2 * x1p * x1p;
    let den = rx2 * y1p * y1p + ry2 * x1p * x1p;
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let coef = if den.abs() < 1e-12 {
        0.0
    } else {
        sign * (num / den).max(0.0).sqrt()
    };
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * -(ry * x1p / rx);
    let cx = cos_phi * cxp - sin_phi * cyp + (from.x + to.x) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (from.y + to.y) / 2.0;
    let (ux, uy) = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let (vx, vy) = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut dtheta = angle(ux, uy, vx, vy);
    if !sweep && dtheta > 0.0 {
        dtheta -= 2.0 * PI;
    } else if sweep && dtheta < 0.0 {
        dtheta += 2.0 * PI;
    }
    // 每段不超过 90°。
    let n = ((dtheta.abs() / (PI / 2.0)) - 1e-4).ceil().max(1.0) as usize;
    let delta = dtheta / n as f32;
    let t = 4.0 / 3.0 * (delta / 4.0).tan();
    let map = |x: f32, y: f32| {
        Point::new(
            cos_phi * rx * x - sin_phi * ry * y + cx,
            sin_phi * rx * x + cos_phi * ry * y + cy,
        )
    };
    for i in 0..n {
        let a1 = theta1 + delta * i as f32;
        let a2 = a1 + delta;
        let (s1, c1) = a1.sin_cos();
        let (s2, c2) = a2.sin_cos();
        let p1 = map(c1 - t * s1, s1 + t * c1);
        let p2 = map(c2 + t * s2, s2 - t * c2);
        // 最后一段落在精确的终点上，避免误差累积。
        let p3 = if i + 1 == n { to } else { map(c2, s2) };
        path.cubic_to(p1.x, p1.y, p2.x, p2.y, p3.x, p3.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn parses_absolute_relative_and_implicit_commands() {
        let p = Path::parse("M10 10 L20 10 20 20 z m5 5 h10 v-3").unwrap();
        assert_eq!(
            p.segments(),
            &[
                Seg::Move(pt(10.0, 10.0)),
                Seg::Line(pt(20.0, 10.0)),
                Seg::Line(pt(20.0, 20.0)),
                Seg::Close,
                // z 之后相对坐标从子路径起点算起。
                Seg::Move(pt(15.0, 15.0)),
                Seg::Line(pt(25.0, 15.0)),
                Seg::Line(pt(25.0, 12.0)),
            ]
        );
    }

    #[test]
    fn parses_compact_numbers() {
        let p = Path::parse("M0,0L10-5.5.5 1e1,2E-1,3").unwrap();
        assert_eq!(
            p.segments(),
            &[
                Seg::Move(pt(0.0, 0.0)),
                Seg::Line(pt(10.0, -5.5)),
                Seg::Line(pt(0.5, 10.0)),
                Seg::Line(pt(0.2, 3.0))
            ]
        );
    }

    #[test]
    fn smooth_curves_reflect_the_previous_control_point() {
        let p = Path::parse("M0 0 C0 10 10 10 10 0 S20 -10 20 0").unwrap();
        // 第二段的第一个控制点是 (10, 10) 关于 (10, 0) 的镜像。
        assert_eq!(
            p.segments()[2],
            Seg::Cubic(pt(10.0, -10.0), pt(20.0, -10.0), pt(20.0, 0.0))
        );
        let q = Path::parse("M0 0 Q5 10 10 0 T20 0").unwrap();
        assert_eq!(q.segments()[2], Seg::Quad(pt(15.0, -10.0), pt(20.0, 0.0)));
    }

    #[test]
    fn arcs_become_cubics_ending_exactly_at_the_target() {
        // 半圆：两段 90° 的曲线。
        let p = Path::parse("M0 0 A10 10 0 0 1 20 0").unwrap();
        let segs = p.segments();
        assert_eq!(segs.len(), 3);
        match segs[2] {
            Seg::Cubic(_, _, end) => assert_eq!(end, pt(20.0, 0.0)),
            other => panic!("{other:?}"),
        }
        // 弧的最高点离弦 10 个单位（圆心在 (10, 0)，半径 10）。
        let b = p.bounds();
        assert!(b.height > 9.0 && b.height < 15.0, "{b:?}");
        // 紧凑写法：标志位和坐标连在一起。
        let q = Path::parse("M0 0a10 10 0 0120 0").unwrap();
        assert_eq!(q.segments().len(), 3);
        // 终点重合的圆弧什么也不画；半径为 0 退化成直线。
        assert_eq!(Path::parse("M5 5 A3 3 0 0 0 5 5").unwrap().segments().len(), 1);
        assert_eq!(
            Path::parse("M0 0 A0 5 0 0 0 4 4").unwrap().segments()[1],
            Seg::Line(pt(4.0, 4.0))
        );
    }

    #[test]
    fn reports_errors_with_position() {
        assert!(Path::parse("10 10").is_err());
        assert!(Path::parse("M10").is_err());
        assert!(Path::parse("M0 0 X1 1").is_err());
        assert_eq!(Path::parse("M0 0 L5 x").unwrap_err().offset, 8);
        assert!(Path::parse("").unwrap().is_empty());
    }

    #[test]
    fn shapes_and_transform() {
        let r = Path::rounded_rect(Rect::new(0.0, 0.0, 10.0, 6.0), 100.0);
        // 圆角半径被限制为短边的一半。
        assert_eq!(r.bounds(), Rect::new(0.0, 0.0, 10.0, 6.0));
        let c = Path::circle(pt(5.0, 5.0), 2.0).transformed(Affine {
            scale: 2.0,
            dx: 1.0,
            dy: 0.0,
        });
        assert_eq!(c.bounds(), Rect::new(7.0, 6.0, 8.0, 8.0));
    }
}
