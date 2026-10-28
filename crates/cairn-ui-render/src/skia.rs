//! 基于 tiny-skia 的 CPU 渲染实现。
//!
//! [`Canvas`] 持有整窗口大小的画布。重绘时只处理脏区：每个脏区先画进一块同样大小的临时
//! 像素图（天然被裁剪到脏区内），再整块拷回画布。这样小范围更新（如光标闪烁）不需要
//! 为整个窗口生成裁剪蒙版，大背景填充也只会处理脏区那一小块。

use cosmic_text::SwashContent;
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, PixmapMut, Transform};

use crate::geom::{Color, DeviceRect, Point, Rect};
use crate::painter::Painter;
use crate::text::{TextLayout, TextSystem};

/// 整窗口画布：RGBA8、预乘 alpha、行优先紧密排列。
#[derive(Default)]
pub struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    scratch: Vec<u8>,
}

impl Canvas {
    pub fn new() -> Self {
        Self::default()
    }

    /// 调整尺寸（物理像素）。尺寸变化时内容被清空，需要整窗重画。返回尺寸是否变化。
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        if (width, height) == (self.width, self.height) {
            return false;
        }
        self.width = width;
        self.height = height;
        self.pixels.clear();
        self.pixels.resize(width as usize * height as usize * 4, 0);
        true
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// 重画 `region`（物理像素，窗口坐标）。`f` 里画的内容会被裁剪到 `region` 内，
    /// `region` 内原有的像素被完全替换（临时像素图初始是透明的，调用方应先画背景）。
    pub fn paint_region(
        &mut self,
        region: DeviceRect,
        scale: f32,
        text: &mut TextSystem,
        f: impl FnOnce(&mut dyn Painter),
    ) {
        let bounds = DeviceRect::new(0, 0, self.width, self.height);
        let Some(region) = region.intersection(&bounds) else {
            return;
        };
        let (w, h) = (region.width as usize, region.height as usize);
        let len = w * h * 4;
        if self.scratch.len() < len {
            self.scratch.resize(len, 0);
        }
        let scratch = &mut self.scratch[..len];
        scratch.fill(0);
        {
            let pixmap = PixmapMut::from_bytes(scratch, region.width, region.height).expect("脏区尺寸非零");
            let mut painter = SkiaPainter {
                pixmap,
                origin: (region.x, region.y),
                scale,
                text,
                clips: vec![DeviceRect::new(0, 0, region.width, region.height)],
                mask: None,
            };
            f(&mut painter);
        }
        let stride = self.width as usize * 4;
        for row in 0..h {
            let src = &self.scratch[row * w * 4..(row + 1) * w * 4];
            let start = (region.y as usize + row) * stride + region.x as usize * 4;
            self.pixels[start..start + w * 4].copy_from_slice(src);
        }
    }
}

/// tiny-skia 实现的 [`Painter`]。坐标先换算到物理像素、对齐像素网格，
/// 再平移到当前临时像素图的局部坐标。
pub struct SkiaPainter<'a> {
    pixmap: PixmapMut<'a>,
    /// 临时像素图左上角在窗口物理坐标中的位置。
    origin: (i32, i32),
    scale: f32,
    text: &'a mut TextSystem,
    /// 裁剪栈（局部物理坐标）。第 0 个是整块临时像素图。
    clips: Vec<DeviceRect>,
    /// 当前裁剪区不是整块像素图时，画路径要用的蒙版（按裁剪区缓存）。
    mask: Option<(DeviceRect, Mask)>,
}

/// 局部物理坐标下的左上右下。
#[derive(Clone, Copy)]
struct Ltrb {
    l: i32,
    t: i32,
    r: i32,
    b: i32,
}

impl<'a> SkiaPainter<'a> {
    fn clip(&self) -> DeviceRect {
        *self.clips.last().expect("裁剪栈不为空")
    }

    /// 逻辑矩形 → 局部物理坐标，四条边分别四舍五入到像素网格。
    /// 相邻矩形共享同一条边时取整结果一致，不会出现缝隙或重叠。
    /// 非空矩形至少占 1 个物理像素。
    fn snap(&self, r: Rect) -> Option<Ltrb> {
        if r.is_empty() {
            return None;
        }
        let s = self.scale;
        let (ox, oy) = self.origin;
        let mut l = (r.x * s).round() as i32;
        let mut t = (r.y * s).round() as i32;
        let mut rr = (r.right() * s).round() as i32;
        let mut b = (r.bottom() * s).round() as i32;
        if rr <= l {
            rr = l + 1;
        }
        if b <= t {
            b = t + 1;
        }
        l -= ox;
        rr -= ox;
        t -= oy;
        b -= oy;
        Some(Ltrb { l, t, r: rr, b })
    }

    fn paint(color: Color, anti_alias: bool) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color_rgba8(color.r, color.g, color.b, color.a);
        p.anti_alias = anti_alias;
        p
    }

    fn fill_device(&mut self, d: Ltrb, color: Color) {
        let c = self.clip();
        let l = d.l.max(c.x);
        let t = d.t.max(c.y);
        let r = d.r.min(c.right());
        let b = d.b.min(c.bottom());
        if l >= r || t >= b || color.a == 0 {
            return;
        }
        if let Some(rect) = tiny_skia::Rect::from_ltrb(l as f32, t as f32, r as f32, b as f32) {
            self.pixmap
                .fill_rect(rect, &Self::paint(color, false), Transform::identity(), None);
        }
    }

    /// 确保 `self.mask` 是当前裁剪区对应的蒙版。
    fn ensure_clip_mask(&mut self, c: DeviceRect) {
        if matches!(&self.mask, Some((rect, _)) if *rect == c) {
            return;
        }
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let mut mask = Mask::new(w, h).expect("像素图尺寸非零");
        let data = mask.data_mut();
        for y in c.y..c.bottom() {
            let row = y as usize * w as usize;
            data[row + c.x as usize..row + c.right() as usize].fill(255);
        }
        self.mask = Some((c, mask));
    }

    fn fill_path(&mut self, path: &tiny_skia::Path, rule: FillRule, color: Color) {
        let c = self.clip();
        if color.a == 0 || c.is_empty() {
            return;
        }
        let paint = Self::paint(color, true);
        // 裁剪区就是整块像素图时不需要蒙版（像素图边界本身就是裁剪）。
        let needs_mask = c != self.clips[0];
        if needs_mask {
            self.ensure_clip_mask(c);
        }
        let mask = if needs_mask {
            self.mask.as_ref().map(|(_, m)| m)
        } else {
            None
        };
        self.pixmap.fill_path(path, &paint, rule, Transform::identity(), mask);
    }
}

fn push_rounded_rect(pb: &mut PathBuilder, l: f32, t: f32, r: f32, b: f32, radius: f32) {
    let rad = radius.min((r - l) / 2.0).min((b - t) / 2.0).max(0.0);
    // 用三次贝塞尔近似四分之一圆。
    let k = rad * (1.0 - 0.552_284_8);
    pb.move_to(l + rad, t);
    pb.line_to(r - rad, t);
    pb.cubic_to(r - k, t, r, t + k, r, t + rad);
    pb.line_to(r, b - rad);
    pb.cubic_to(r, b - k, r - k, b, r - rad, b);
    pb.line_to(l + rad, b);
    pb.cubic_to(l + k, b, l, b - k, l, b - rad);
    pb.line_to(l, t + rad);
    pb.cubic_to(l, t + k, l + k, t, l + rad, t);
    pb.close();
}

impl Painter for SkiaPainter<'_> {
    fn scale(&self) -> f32 {
        self.scale
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        if let Some(d) = self.snap(rect) {
            self.fill_device(d, color);
        }
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        let Some(d) = self.snap(rect) else { return };
        let rad = radius * self.scale;
        if rad < 0.5 {
            return self.fill_device(d, color);
        }
        let mut pb = PathBuilder::new();
        push_rounded_rect(&mut pb, d.l as f32, d.t as f32, d.r as f32, d.b as f32, rad);
        if let Some(path) = pb.finish() {
            self.fill_path(&path, FillRule::Winding, color);
        }
    }

    fn stroke_rect(&mut self, rect: Rect, radius: f32, width: f32, color: Color) {
        let Some(o) = self.snap(rect) else { return };
        let wpx = ((width * self.scale).round() as i32).max(1);
        let i = Ltrb {
            l: o.l + wpx,
            t: o.t + wpx,
            r: o.r - wpx,
            b: o.b - wpx,
        };
        if i.l >= i.r || i.t >= i.b {
            // 边框比矩形还粗：直接填满。
            return self.fill_rounded_rect(rect, radius, color);
        }
        let rad = radius * self.scale;
        if rad < 0.5 {
            self.fill_device(Ltrb { b: i.t, ..o }, color);
            self.fill_device(Ltrb { t: i.b, ..o }, color);
            self.fill_device(
                Ltrb {
                    l: o.l,
                    t: i.t,
                    r: i.l,
                    b: i.b,
                },
                color,
            );
            self.fill_device(
                Ltrb {
                    l: i.r,
                    t: i.t,
                    r: o.r,
                    b: i.b,
                },
                color,
            );
            return;
        }
        // 外轮廓减内轮廓（奇偶填充），比描边更容易对齐像素。
        let mut pb = PathBuilder::new();
        push_rounded_rect(&mut pb, o.l as f32, o.t as f32, o.r as f32, o.b as f32, rad);
        push_rounded_rect(
            &mut pb,
            i.l as f32,
            i.t as f32,
            i.r as f32,
            i.b as f32,
            (rad - wpx as f32).max(0.0),
        );
        if let Some(path) = pb.finish() {
            self.fill_path(&path, FillRule::EvenOdd, color);
        }
    }

    fn draw_text(&mut self, text: &TextLayout, origin: Point, color: Color) {
        let clip = self.clip();
        if clip.is_empty() || color.a == 0 {
            return;
        }
        let s = self.scale;
        let ox = origin.x * s;
        // 纵向对齐到整像素，横向保留小数交给 cosmic-text 的亚像素分档。
        let oy = (origin.y * s).round();
        let ts = &mut *self.text;
        let (px, py) = self.origin;
        for run in text.buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let pg = glyph.physical((ox, oy + run.line_y * s), s);
                let Some(image) = ts.glyphs.get_image(&mut ts.fonts, pg.cache_key) else {
                    continue;
                };
                let x = pg.x + image.placement.left - px;
                let y = pg.y - image.placement.top - py;
                blend_glyph(&mut self.pixmap, image, x, y, clip, color);
            }
        }
    }

    fn push_clip(&mut self, rect: Rect) {
        let c = self.clip();
        let next = self
            .snap(rect)
            .and_then(|d| DeviceRect::new(d.l, d.t, (d.r - d.l) as u32, (d.b - d.t) as u32).intersection(&c))
            .unwrap_or(DeviceRect::new(c.x, c.y, 0, 0));
        self.clips.push(next);
    }

    fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
    }
}

/// `a * b / 255`，四舍五入。
#[inline]
fn mul255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

/// 把一个字形位图按 source-over 混合进像素图（像素图是预乘 alpha）。
fn blend_glyph(
    pixmap: &mut PixmapMut,
    image: &cosmic_text::SwashImage,
    x: i32,
    y: i32,
    clip: DeviceRect,
    color: Color,
) {
    let (w, h) = (image.placement.width as i32, image.placement.height as i32);
    let x0 = x.max(clip.x);
    let y0 = y.max(clip.y);
    let x1 = (x + w).min(clip.right());
    let y1 = (y + h).min(clip.bottom());
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let stride = pixmap.width() as usize;
    let dst = pixmap.data_mut();
    let (cr, cg, cb, ca) = (color.r as u32, color.g as u32, color.b as u32, color.a as u32);
    match image.content {
        SwashContent::Mask => {
            for py in y0..y1 {
                let src_row = ((py - y) * w) as usize;
                let dst_row = py as usize * stride;
                for px in x0..x1 {
                    let cov = image.data[src_row + (px - x) as usize] as u32;
                    if cov == 0 {
                        continue;
                    }
                    let a = mul255(cov, ca);
                    let inv = 255 - a;
                    let i = (dst_row + px as usize) * 4;
                    dst[i] = (mul255(cr, a) + mul255(dst[i] as u32, inv)) as u8;
                    dst[i + 1] = (mul255(cg, a) + mul255(dst[i + 1] as u32, inv)) as u8;
                    dst[i + 2] = (mul255(cb, a) + mul255(dst[i + 2] as u32, inv)) as u8;
                    dst[i + 3] = (a + mul255(dst[i + 3] as u32, inv)) as u8;
                }
            }
        }
        SwashContent::Color => {
            // 彩色字形（emoji）：自带颜色，按非预乘 RGBA 处理，只套用文字颜色的透明度。
            for py in y0..y1 {
                let src_row = ((py - y) * w) as usize;
                let dst_row = py as usize * stride;
                for px in x0..x1 {
                    let s = (src_row + (px - x) as usize) * 4;
                    let a = mul255(image.data[s + 3] as u32, ca);
                    if a == 0 {
                        continue;
                    }
                    let inv = 255 - a;
                    let i = (dst_row + px as usize) * 4;
                    for c in 0..3 {
                        dst[i + c] = (mul255(image.data[s + c] as u32, a) + mul255(dst[i + c] as u32, inv)) as u8;
                    }
                    dst[i + 3] = (a + mul255(dst[i + 3] as u32, inv)) as u8;
                }
            }
        }
        // 请求的是灰度格式，不会出现子像素蒙版。
        SwashContent::SubpixelMask => {}
    }
}
