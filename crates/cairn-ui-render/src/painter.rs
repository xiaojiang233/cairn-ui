//! 与具体渲染后端无关的绘图接口。

use crate::geom::{Affine, Color, Point, Rect};
use crate::image::Image;
use crate::text::TextLayout;

/// 渐变上的一个色标。`offset` 取 0–1。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}

/// 填充方式。
///
/// 渐变插值在非预乘颜色上进行：要渐隐到透明时，终点用同一颜色、透明度为 0
/// （`c.with_alpha(0)`），而不是 [`Color::TRANSPARENT`]，否则中间会发灰。
#[derive(Clone, Debug, PartialEq)]
pub enum Brush {
    Solid(Color),
    /// 线性渐变，`start` / `end` 是窗口坐标（逻辑像素），受当前变换影响。
    Linear {
        start: Point,
        end: Point,
        stops: Vec<GradientStop>,
    },
    /// 径向渐变，`center` 是窗口坐标（逻辑像素）。
    Radial {
        center: Point,
        radius: f32,
        stops: Vec<GradientStop>,
    },
    /// 窗口背景（场景）经过模糊和饱和度增强后的图像，始终与窗口对齐，不随变换移动——
    /// 玻璃面板移动时“透过”它看到的是背后不同位置的场景。没有场景时等同于窗口背景色。
    Backdrop,
}

impl From<Color> for Brush {
    fn from(c: Color) -> Self {
        Brush::Solid(c)
    }
}

impl Brush {
    pub fn linear(start: Point, end: Point, stops: impl IntoIterator<Item = (f32, Color)>) -> Brush {
        Brush::Linear {
            start,
            end,
            stops: collect_stops(stops),
        }
    }

    /// 从 `rect` 顶边到底边的两色线性渐变。
    pub fn vertical(rect: Rect, top: Color, bottom: Color) -> Brush {
        Brush::linear(rect.top_center(), rect.bottom_center(), [(0.0, top), (1.0, bottom)])
    }

    pub fn radial(center: Point, radius: f32, stops: impl IntoIterator<Item = (f32, Color)>) -> Brush {
        Brush::Radial {
            center,
            radius,
            stops: collect_stops(stops),
        }
    }
}

fn collect_stops(stops: impl IntoIterator<Item = (f32, Color)>) -> Vec<GradientStop> {
    stops
        .into_iter()
        .map(|(offset, color)| GradientStop { offset, color })
        .collect()
}

/// 阴影，语义同 CSS `box-shadow`。
///
/// - `blur` 是模糊半径（CSS 的第三个长度），高斯标准差为 `blur / 2`。
/// - 外阴影只画在形状外面（与 CSS 一致：盒子本身的区域被挖掉），所以半透明的玻璃底下不会透出阴影。
/// - 内阴影只画在形状里面；`blur = 0`、偏移 1 个物理像素就是常见的顶部高光 / 底部暗边。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shadow {
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

impl Shadow {
    pub const NONE: Shadow = Shadow {
        offset: Point::ZERO,
        blur: 0.0,
        spread: 0.0,
        color: Color::TRANSPARENT,
    };

    pub const fn new(dx: f32, dy: f32, blur: f32, color: Color) -> Shadow {
        Shadow {
            offset: Point::new(dx, dy),
            blur,
            spread: 0.0,
            color,
        }
    }

    pub const fn spread(self, spread: f32) -> Shadow {
        Shadow { spread, ..self }
    }

    pub fn is_none(&self) -> bool {
        self.color.a == 0
    }

    /// 在两个阴影之间插值（偏移、模糊、扩展、颜色都按比例），用于悬停动效。
    pub fn lerp(&self, other: &Shadow, t: f32) -> Shadow {
        let t = t.clamp(0.0, 1.0);
        let f = |a: f32, b: f32| a + (b - a) * t;
        // 一端是“无阴影”时只渐变透明度，几何形状取有阴影的那一端。
        let (geo_a, geo_b) = match (self.is_none(), other.is_none()) {
            (true, false) => (other, other),
            (false, true) => (self, self),
            _ => (self, other),
        };
        Shadow {
            offset: Point::new(f(geo_a.offset.x, geo_b.offset.x), f(geo_a.offset.y, geo_b.offset.y)),
            blur: f(geo_a.blur, geo_b.blur),
            spread: f(geo_a.spread, geo_b.spread),
            color: self.color.mix(other.color, t),
        }
    }

    /// 外阴影最多能画到形状外多远（逻辑像素）：上、右、下、左。
    pub fn extent(&self) -> [f32; 4] {
        if self.is_none() {
            return [0.0; 4];
        }
        let r = self.blur * 1.5 + self.spread.max(0.0);
        [
            (r - self.offset.y).max(0.0),
            (r + self.offset.x).max(0.0),
            (r + self.offset.y).max(0.0),
            (r - self.offset.x).max(0.0),
        ]
    }
}

/// 控件用来画自己的接口。
///
/// 所有坐标都是**逻辑像素**（窗口坐标系）；实现负责乘以缩放因子换算到物理像素，
/// 并把矩形边缘对齐到像素网格，保证 100% / 125% / 150% 下边缘都清晰。
///
/// 线宽约定：`width <= 0` 表示**细线**，恰好 1 个物理像素（[`Painter::hairline`] 给出对应的逻辑长度）。
///
/// 目前的实现是 [`crate::SkiaPainter`]（tiny-skia，CPU）。以后换 GPU 后端（如 vello）
/// 只需要再实现一遍这个 trait，控件代码不用动。
pub trait Painter {
    /// 缩放因子（物理像素 / 逻辑像素），不含 [`Painter::push_transform`] 的缩放。
    fn scale(&self) -> f32;

    /// 1 个物理像素对应的逻辑长度。
    fn hairline(&self) -> f32 {
        1.0 / self.scale()
    }

    /// 填充圆角矩形（`radius = 0` 为直角）。
    fn fill(&mut self, rect: Rect, radius: f32, brush: &Brush);

    /// 沿 `rect` 内侧画边框，边框完全落在 `rect` 里面（与 CSS border 一致）。
    /// `radius` 是外轮廓的圆角半径。没有变换时线宽取整到整数个物理像素（至少 1）。
    fn stroke(&mut self, rect: Rect, radius: f32, width: f32, brush: &Brush);

    /// 圆角矩形的外阴影，只画在矩形外面。
    fn draw_shadow(&mut self, rect: Rect, radius: f32, shadow: &Shadow);

    /// 圆角矩形的内阴影，只画在矩形里面。
    fn draw_inner_shadow(&mut self, rect: Rect, radius: f32, shadow: &Shadow);

    /// 画一段已排版好的文字。`origin` 是文字框的左上角。
    fn draw_text(&mut self, text: &TextLayout, origin: Point, color: Color);

    /// 把图片拉伸画进 `rect`。
    fn draw_image(&mut self, image: &Image, rect: Rect, opacity: f32);

    /// 圆头线段，用于画简单图标。
    fn draw_line(&mut self, from: Point, to: Point, width: f32, color: Color);

    /// 填充多边形（自动闭合），用于画简单图标。
    fn fill_polygon(&mut self, points: &[Point], color: Color);

    /// 之后的绘制只落在这个圆角矩形与当前裁剪区的交集内，直到对应的 [`Painter::pop_clip`]。
    fn push_rounded_clip(&mut self, rect: Rect, radius: f32);

    fn pop_clip(&mut self);

    /// 之后的绘制先经过 `t` 变换（与已有的变换复合），直到对应的 [`Painter::pop_transform`]。
    fn push_transform(&mut self, t: Affine);

    fn pop_transform(&mut self);

    // ------------------------------------------------------------ 便捷方法

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.fill(rect, 0.0, &Brush::Solid(color));
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        self.fill(rect, radius, &Brush::Solid(color));
    }

    fn stroke_rect(&mut self, rect: Rect, radius: f32, width: f32, color: Color) {
        self.stroke(rect, radius, width, &Brush::Solid(color));
    }

    fn push_clip(&mut self, rect: Rect) {
        self.push_rounded_clip(rect, 0.0);
    }
}
