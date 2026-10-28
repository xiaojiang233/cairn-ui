//! 几何与颜色类型。除 [`DeviceRect`] 外，坐标都是**逻辑像素**。

use std::ops::{Add, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

impl Add for Point {
    type Output = Point;
    fn add(self, o: Point) -> Point {
        Point::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Point {
    type Output = Point;
    fn sub(self, o: Point) -> Point {
        Point::new(self.x - o.x, self.y - o.y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Size = Size {
        width: 0.0,
        height: 0.0,
    };

    pub const fn new(width: f32, height: f32) -> Self {
        Size { width, height }
    }
}

/// 轴对齐矩形，`(x, y)` 是左上角。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Rect { x, y, width, height }
    }

    pub fn from_ltrb(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Rect::new(left, top, right - left, bottom - top)
    }

    pub fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }

    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn is_empty(&self) -> bool {
        !(self.width > 0.0 && self.height > 0.0)
    }

    /// 包含左、上边，不包含右、下边。
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    pub fn intersection(&self, o: &Rect) -> Option<Rect> {
        let r = Rect::from_ltrb(
            self.x.max(o.x),
            self.y.max(o.y),
            self.right().min(o.right()),
            self.bottom().min(o.bottom()),
        );
        (!r.is_empty()).then_some(r)
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        Rect::from_ltrb(
            self.x.min(o.x),
            self.y.min(o.y),
            self.right().max(o.right()),
            self.bottom().max(o.bottom()),
        )
    }

    /// 向外扩张（负数为向内收缩）。
    pub fn inflate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x - dx, self.y - dy, self.width + 2.0 * dx, self.height + 2.0 * dy)
    }

    pub fn translate(&self, d: Point) -> Rect {
        Rect::new(self.x + d.x, self.y + d.y, self.width, self.height)
    }

    /// 四边分别外扩（负数为收缩）。
    pub fn outset(&self, top: f32, right: f32, bottom: f32, left: f32) -> Rect {
        Rect::from_ltrb(
            self.x - left,
            self.y - top,
            self.right() + right,
            self.bottom() + bottom,
        )
    }

    pub fn top_center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y)
    }

    pub fn bottom_center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.bottom())
    }
}

/// 只含等比缩放和平移的变换：`p' = p * scale + (dx, dy)`。
///
/// 不支持旋转和非等比缩放，所以矩形变换后仍是轴对齐矩形，像素对齐和矩形裁剪都保持简单。
/// 用于悬停上浮、按下缩小这类整块内容的动效。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub scale: f32,
    pub dx: f32,
    pub dy: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Affine::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        scale: 1.0,
        dx: 0.0,
        dy: 0.0,
    };

    pub const fn translate(dx: f32, dy: f32) -> Affine {
        Affine { scale: 1.0, dx, dy }
    }

    /// 以 `center` 为中心缩放 `k` 倍。
    pub fn scale_about(center: Point, k: f32) -> Affine {
        Affine {
            scale: k,
            dx: center.x * (1.0 - k),
            dy: center.y * (1.0 - k),
        }
    }

    pub fn is_identity(&self) -> bool {
        *self == Affine::IDENTITY
    }

    /// 先做 `self`，再做 `outer`。
    pub fn then(self, outer: Affine) -> Affine {
        Affine {
            scale: self.scale * outer.scale,
            dx: self.dx * outer.scale + outer.dx,
            dy: self.dy * outer.scale + outer.dy,
        }
    }

    pub fn apply(&self, p: Point) -> Point {
        Point::new(p.x * self.scale + self.dx, p.y * self.scale + self.dy)
    }

    pub fn apply_rect(&self, r: Rect) -> Rect {
        Rect::new(
            r.x * self.scale + self.dx,
            r.y * self.scale + self.dy,
            r.width * self.scale,
            r.height * self.scale,
        )
    }
}

/// 设备（物理像素）坐标下的整数矩形。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DeviceRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl DeviceRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        DeviceRect { x, y, width, height }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn intersection(&self, o: &DeviceRect) -> Option<DeviceRect> {
        let l = self.x.max(o.x);
        let t = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r > l && b > t).then(|| DeviceRect::new(l, t, (r - l) as u32, (b - t) as u32))
    }

    /// 把逻辑矩形换算到设备像素，向外取整，保证完全覆盖。
    pub fn cover(rect: &Rect, scale: f32) -> DeviceRect {
        let l = (rect.x * scale).floor() as i32;
        let t = (rect.y * scale).floor() as i32;
        let r = (rect.right() * scale).ceil() as i32;
        let b = (rect.bottom() * scale).ceil() as i32;
        DeviceRect::new(l, t, (r - l).max(0) as u32, (b - t).max(0) as u32)
    }
}

/// sRGB 颜色，非预乘 alpha。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color { r, g, b, a }
    }

    /// `0xRRGGBB`。
    pub const fn hex(rgb: u32) -> Self {
        Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    }

    /// 透明度用 0–1 的小数给出，例如 `Color::rgbaf(255, 255, 255, 0.08)` 是 8% 的白。
    pub const fn rgbaf(r: u8, g: u8, b: u8, a: f32) -> Self {
        Color {
            r,
            g,
            b,
            a: unit_to_u8(a),
        }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Color { a, ..self }
    }

    /// 替换透明度（0–1）。
    pub const fn with_opacity(self, a: f32) -> Self {
        Color {
            a: unit_to_u8(a),
            ..self
        }
    }

    /// 透明度乘以 `k`（0–1）。
    pub fn fade(self, k: f32) -> Self {
        Color {
            a: (self.a as f32 * k.clamp(0.0, 1.0)).round() as u8,
            ..self
        }
    }

    /// 按比例与另一个颜色混合，`t = 0` 为自身，`t = 1` 为 `other`。
    pub fn mix(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color::rgba(
            m(self.r, other.r),
            m(self.g, other.g),
            m(self.b, other.b),
            m(self.a, other.a),
        )
    }
}

const fn unit_to_u8(a: f32) -> u8 {
    let a = if a < 0.0 {
        0.0
    } else if a > 1.0 {
        1.0
    } else {
        a
    };
    (a * 255.0 + 0.5) as u8
}
