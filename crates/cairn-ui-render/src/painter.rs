//! 与具体渲染后端无关的绘图接口。

use crate::geom::{Color, Point, Rect};
use crate::text::TextLayout;

/// 控件用来画自己的接口。
///
/// 所有坐标都是**逻辑像素**（窗口坐标系）；实现负责乘以缩放因子换算到物理像素，
/// 并把矩形边缘对齐到像素网格，保证 100% / 125% / 150% 下边缘都清晰。
///
/// 目前的实现是 [`crate::SkiaPainter`]（tiny-skia，CPU）。以后换 GPU 后端（如 vello）
/// 只需要再实现一遍这个 trait，控件代码不用动。
pub trait Painter {
    /// 缩放因子（物理像素 / 逻辑像素）。
    fn scale(&self) -> f32;

    fn fill_rect(&mut self, rect: Rect, color: Color);

    fn fill_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color);

    /// 沿 `rect` 内侧画边框，边框完全落在 `rect` 里面（与 CSS border 一致）。
    /// `radius` 是外轮廓的圆角半径，0 为直角。线宽会取整到整数个物理像素（至少 1）。
    fn stroke_rect(&mut self, rect: Rect, radius: f32, width: f32, color: Color);

    /// 画一段已排版好的文字。`origin` 是文字框的左上角。
    fn draw_text(&mut self, text: &TextLayout, origin: Point, color: Color);

    /// 之后的绘制只落在 `rect` 与当前裁剪区的交集内，直到对应的 [`Painter::pop_clip`]。
    fn push_clip(&mut self, rect: Rect);

    fn pop_clip(&mut self);
}
