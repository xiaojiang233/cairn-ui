//! 窗口背景场景：整窗口不透明的底图，也是玻璃面板背后模糊图像的来源。
//!
//! 场景只在窗口尺寸、缩放因子或场景本身变化时重画一次，缓存成整窗口的像素图；
//! 之后每次局部重绘直接从缓存拷贝，不重新计算渐变。

use std::fmt::Debug;

use crate::geom::{Color, Point, Rect, Size};
use crate::image::Image;
use crate::painter::{Brush, Painter};

/// 窗口背景。实现者必须把 `size`（逻辑像素）范围内画满不透明的颜色。
pub trait Scene: Debug + 'static {
    fn paint(&self, p: &mut dyn Painter, size: Size);
}

/// 场景里的一个光斑：圆心按窗口尺寸的比例定位，再加一个逻辑像素偏移（可以落在窗口外）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    /// 圆心相对窗口的位置，`(0, 0)` 左上角，`(1, 1)` 右下角。
    pub anchor: (f32, f32),
    pub offset: Point,
    /// 半径（逻辑像素），光斑在这个距离上衰减到透明。
    pub radius: f32,
    /// 圆心处的颜色（含透明度）。
    pub color: Color,
}

/// 夜景：深墨蓝底色、边缘压暗，叠几个柔和的径向光斑。
#[derive(Clone, Debug, PartialEq)]
pub struct NightScene {
    pub base: Color,
    /// 四角压暗到的颜色。
    pub edge: Color,
    pub spots: Vec<Spot>,
}

impl Default for NightScene {
    /// 右上角一个月光蓝光斑、左下角一个深蓝光斑。
    fn default() -> Self {
        NightScene {
            base: Color::hex(0x0B1322),
            edge: Color::hex(0x060A13),
            spots: vec![
                Spot {
                    anchor: (1.0, 0.0),
                    offset: Point::new(60.0, -80.0),
                    radius: 560.0,
                    color: Color::rgbaf(124, 156, 196, 0.28),
                },
                Spot {
                    anchor: (0.0, 1.0),
                    offset: Point::new(-40.0, 60.0),
                    radius: 520.0,
                    color: Color::rgbaf(51, 81, 122, 0.35),
                },
            ],
        }
    }
}

impl NightScene {
    /// 在默认夜景上再加一个靠近中下方的暖色光斑。
    pub fn with_warm_glow(mut self) -> Self {
        self.spots.push(Spot {
            anchor: (0.62, 1.0),
            offset: Point::new(0.0, 120.0),
            radius: 460.0,
            color: Color::rgbaf(228, 184, 99, 0.12),
        });
        self
    }
}

/// 柔和衰减的光斑色标：比线性衰减更像光晕，避免出现“圆锥”感的边界。
fn spot_stops(c: Color) -> [(f32, Color); 5] {
    [
        (0.0, c),
        (0.25, c.fade(0.78)),
        (0.5, c.fade(0.42)),
        (0.75, c.fade(0.13)),
        (1.0, c.with_alpha(0)),
    ]
}

impl Scene for NightScene {
    fn paint(&self, p: &mut dyn Painter, size: Size) {
        let all = Rect::new(0.0, 0.0, size.width, size.height);
        p.fill_rect(all, self.base);
        // 暗角：中心保持底色，四角压到 edge。
        let center = all.center();
        let reach = (size.width * size.width + size.height * size.height).sqrt() / 2.0;
        if reach > 0.0 {
            p.fill(
                all,
                0.0,
                &Brush::radial(
                    center,
                    reach,
                    [
                        (0.0, self.edge.with_alpha(0)),
                        (0.45, self.edge.with_alpha(0)),
                        (1.0, self.edge),
                    ],
                ),
            );
        }
        for s in &self.spots {
            if s.radius <= 0.0 {
                continue;
            }
            let c = Point::new(
                s.anchor.0 * size.width + s.offset.x,
                s.anchor.1 * size.height + s.offset.y,
            );
            p.fill(all, 0.0, &Brush::radial(c, s.radius, spot_stops(s.color)));
        }
    }
}

/// 图片背景：按“覆盖”方式铺满窗口（保持比例、居中裁切），再盖一层暗色，保证上面的文字可读。
#[derive(Clone, Debug)]
pub struct ImageScene {
    pub image: Image,
    /// 盖在图片上的颜色，默认 `rgba(6, 10, 19, 0.6)`。
    pub dim: Color,
    /// 图片后面的底色（图片本身半透明时可见）。
    pub base: Color,
}

impl ImageScene {
    pub fn new(image: Image) -> Self {
        ImageScene {
            image,
            dim: Color::rgbaf(6, 10, 19, 0.6),
            base: Color::hex(0x0B1322),
        }
    }
}

impl Scene for ImageScene {
    fn paint(&self, p: &mut dyn Painter, size: Size) {
        let all = Rect::new(0.0, 0.0, size.width, size.height);
        p.fill_rect(all, self.base);
        let (iw, ih) = (self.image.width() as f32, self.image.height() as f32);
        if iw > 0.0 && ih > 0.0 {
            let k = (size.width / iw).max(size.height / ih);
            let (w, h) = (iw * k, ih * k);
            let dest = Rect::new((size.width - w) / 2.0, (size.height - h) / 2.0, w, h);
            p.draw_image(&self.image, dest, 1.0);
        }
        p.fill_rect(all, self.dim);
    }
}
