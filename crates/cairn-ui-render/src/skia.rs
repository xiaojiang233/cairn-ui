//! 基于 tiny-skia 的 CPU 渲染实现。
//!
//! [`Canvas`] 持有整窗口大小的画布。重绘时只处理脏区：每个脏区先画进一块同样大小的临时
//! 像素图（天然被裁剪到脏区内），再整块拷回画布。这样小范围更新（如光标闪烁）不需要
//! 为整个窗口生成裁剪蒙版，大背景填充也只会处理脏区那一小块。
//!
//! 背景分三层缓存，只在窗口尺寸、缩放因子或场景变化时重算：
//! 1. 场景（[`Scene`]）画成的整窗口像素图，每次重绘脏区时先从这里拷贝底色；
//! 2. 场景降采样、模糊、提高饱和度后的低分辨率图像，玻璃面板用 [`Brush::Backdrop`] 取样，
//!    取样时双线性插值放大回原尺寸；
//! 3. 256×256 的噪点块，每个脏区画完后按窗口绝对坐标叠加，所以局部重绘没有接缝。

use std::rc::Rc;

use cosmic_text::SwashContent;
use tiny_skia::{
    FillRule, FilterQuality, IntSize, LineCap, LinearGradient, Mask, Paint, Path, PathBuilder, Pattern, Pixmap,
    PixmapMut, RadialGradient, Shader, SpreadMode, Stroke, Transform,
};

use crate::effects::{self, NOISE_SIZE, ShadowCache, rrect_coverage};
use crate::geom::{Affine, Color, DeviceRect, Point, Rect, Size};
use crate::image::Image;
use crate::painter::{Brush, GradientStop, Painter, Shadow};
use crate::scene::Scene;
use crate::text::{TextLayout, TextSystem};

/// 窗口背景。
#[derive(Clone, Debug)]
pub enum Background {
    Solid(Color),
    Scene(Rc<dyn Scene>),
}

/// 背景模糊（玻璃效果）的参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackdropStyle {
    /// 高斯模糊的标准差（逻辑像素），语义同 CSS `backdrop-filter: blur()`。
    pub blur: f32,
    /// 饱和度倍数，语义同 CSS `saturate()`。
    pub saturation: f32,
    /// 先缩小这么多倍再模糊。模糊半径很大时看不出差别，计算量小得多。
    pub downsample: u32,
}

impl Default for BackdropStyle {
    fn default() -> Self {
        BackdropStyle {
            blur: 40.0,
            saturation: 1.8,
            downsample: 4,
        }
    }
}

/// 整窗口画布：RGBA8、预乘 alpha、行优先紧密排列。
pub struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    scratch: Vec<u8>,
    background: Background,
    backdrop_style: BackdropStyle,
    noise_amount: f32,
    noise: Vec<u8>,
    /// 场景与模糊背景缓存对应的 (宽, 高, 缩放因子的位模式)。`None` 表示需要重算。
    layers_key: Option<(u32, u32, u32)>,
    scene: Option<Pixmap>,
    backdrop: Option<Pixmap>,
    shadows: ShadowCache,
    /// 阴影缓存对应的缩放因子（位模式）。
    shadow_scale: Option<u32>,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            scratch: Vec::new(),
            background: Background::Solid(Color::TRANSPARENT),
            backdrop_style: BackdropStyle::default(),
            noise_amount: 0.0,
            noise: Vec::new(),
            layers_key: None,
            scene: None,
            backdrop: None,
            shadows: ShadowCache::default(),
            shadow_scale: None,
        }
    }
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
        self.layers_key = None;
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

    /// 设置窗口背景。之后需要整窗重画。
    pub fn set_background(&mut self, background: Background) {
        self.background = background;
        self.layers_key = None;
    }

    pub fn set_backdrop_style(&mut self, style: BackdropStyle) {
        if style != self.backdrop_style {
            self.backdrop_style = style;
            self.layers_key = None;
        }
    }

    /// 全局噪点的不透明度（0 关闭）。例如 0.025 是 2.5%。
    pub fn set_noise(&mut self, amount: f32) {
        self.noise_amount = amount.clamp(0.0, 1.0);
    }

    /// 窗口尺寸、缩放或场景变了：重画场景，重算模糊背景。
    fn ensure_layers(&mut self, scale: f32, text: &mut TextSystem) {
        let key = (self.width, self.height, scale.to_bits());
        if self.layers_key == Some(key) {
            return;
        }
        if self.shadow_scale != Some(key.2) {
            // 阴影蒙版按物理尺寸缓存，换了缩放因子后旧的都用不上了。
            self.shadows.clear();
            self.shadow_scale = Some(key.2);
        }
        self.layers_key = Some(key);
        self.scene = None;
        self.backdrop = None;
        let Background::Scene(scene) = &self.background else {
            return;
        };
        let scene = scene.clone();
        let Some(mut pixmap) = Pixmap::new(self.width, self.height) else {
            return;
        };
        {
            let res = Resources {
                backdrop: None,
                background: Color::BLACK,
                shadows: &mut self.shadows,
            };
            let mut p = SkiaPainter::new(pixmap.as_mut(), (0, 0), scale, text, res);
            scene.paint(&mut p, Size::new(self.width as f32 / scale, self.height as f32 / scale));
        }
        let st = self.backdrop_style;
        let f = st.downsample.max(1) as usize;
        let (mut buf, lw, lh) = effects::downsample_rgba(pixmap.data(), self.width as usize, self.height as usize, f);
        effects::gaussian_blur(&mut buf, lw, lh, 4, st.blur * scale / f as f32);
        effects::saturate(&mut buf, st.saturation);
        let data: Vec<u8> = buf.iter().map(|&v| ((v as u32 + 128) / 257) as u8).collect();
        self.backdrop = IntSize::from_wh(lw as u32, lh as u32).and_then(|s| Pixmap::from_vec(data, s));
        self.scene = Some(pixmap);
    }

    /// 重画 `region`（物理像素，窗口坐标）。`region` 内原有的像素被完全替换：
    /// 先铺背景（纯色或场景），再执行 `f`，最后叠噪点。`f` 里画的内容被裁剪到 `region` 内。
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
        self.ensure_layers(scale, text);
        let (w, h) = (region.width as usize, region.height as usize);
        let len = w * h * 4;
        if self.scratch.len() < len {
            self.scratch.resize(len, 0);
        }
        let stride = self.width as usize * 4;
        let (x0, y0) = (region.x as usize, region.y as usize);
        let solid = match &self.background {
            Background::Solid(c) => *c,
            Background::Scene(_) => Color::BLACK,
        };
        match &self.scene {
            Some(scene) => {
                let src = scene.data();
                for row in 0..h {
                    let s = (y0 + row) * stride + x0 * 4;
                    self.scratch[row * w * 4..(row + 1) * w * 4].copy_from_slice(&src[s..s + w * 4]);
                }
            }
            None => {
                let px = premultiply(solid);
                for c in self.scratch[..len].chunks_exact_mut(4) {
                    c.copy_from_slice(&px);
                }
            }
        }
        {
            let pixmap =
                PixmapMut::from_bytes(&mut self.scratch[..len], region.width, region.height).expect("脏区尺寸非零");
            let factor = self.backdrop_style.downsample.max(1) as f32;
            let res = Resources {
                backdrop: self.backdrop.as_ref().map(|b| (b, factor)),
                background: solid,
                shadows: &mut self.shadows,
            };
            let mut painter = SkiaPainter::new(pixmap, (region.x, region.y), scale, text, res);
            f(&mut painter);
        }
        if self.noise_amount > 0.0 {
            if self.noise.is_empty() {
                self.noise = effects::noise_tile();
            }
            apply_noise(&mut self.scratch[..len], region, &self.noise, self.noise_amount);
        }
        for row in 0..h {
            let src = &self.scratch[row * w * 4..(row + 1) * w * 4];
            let start = (y0 + row) * stride + x0 * 4;
            self.pixels[start..start + w * 4].copy_from_slice(src);
        }
    }
}

fn premultiply(c: Color) -> [u8; 4] {
    let a = c.a as u32;
    [
        mul255(c.r as u32, a) as u8,
        mul255(c.g as u32, a) as u8,
        mul255(c.b as u32, a) as u8,
        c.a,
    ]
}

/// 按窗口绝对坐标把噪点块叠加到一个脏区上（source-over，噪点灰度为颜色，`amount` 为不透明度）。
fn apply_noise(buf: &mut [u8], region: DeviceRect, tile: &[u8], amount: f32) {
    let a = (amount * 65536.0) as i32;
    let w = region.width as usize;
    for row in 0..region.height as usize {
        let ty = (region.y as usize + row) % NOISE_SIZE;
        let tile_row = &tile[ty * NOISE_SIZE..(ty + 1) * NOISE_SIZE];
        let line = &mut buf[row * w * 4..(row + 1) * w * 4];
        for (col, px) in line.chunks_exact_mut(4).enumerate() {
            let n = tile_row[(region.x as usize + col) % NOISE_SIZE] as i32;
            for c in &mut px[..3] {
                let v = *c as i32;
                *c = (v + (((n - v) * a + 32768) >> 16)).clamp(0, 255) as u8;
            }
        }
    }
}

/// 画家需要的共享资源。
struct Resources<'a> {
    /// 模糊背景与它的降采样倍数。
    backdrop: Option<(&'a Pixmap, f32)>,
    /// 没有场景时 [`Brush::Backdrop`] 退化成的颜色。
    background: Color,
    shadows: &'a mut ShadowCache,
}

/// 一层裁剪。
struct Clip {
    /// 外接矩形（局部物理坐标），已与上一层相交。
    rect: DeviceRect,
    /// 圆角等非矩形裁剪的逐像素覆盖率（尺寸同整块像素图）。`None` 表示裁剪区就是 `rect`。
    mask: Option<Rc<Mask>>,
    /// `mask` 在 `rect` 之外已经全是 0（本层就是用它建的），可以直接交给 tiny-skia。
    mask_exact: bool,
    /// 交给 tiny-skia 画路径用的蒙版（`rect` 与 `mask` 的交集），按需生成。
    path_mask: Option<Mask>,
    /// 就是整块像素图：画路径时不需要蒙版。
    full: bool,
}

impl Clip {
    fn new(rect: DeviceRect, mask: Option<Rc<Mask>>, full: bool) -> Clip {
        Clip {
            rect,
            mask,
            mask_exact: false,
            path_mask: None,
            full,
        }
    }
}

/// 局部物理坐标下的浮点矩形（左、上、右、下）。
#[derive(Clone, Copy, Debug)]
struct DRect {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}

impl DRect {
    fn width(&self) -> f32 {
        self.r - self.l
    }

    fn height(&self) -> f32 {
        self.b - self.t
    }

    /// 向外取整的整数外接矩形。
    fn cover(&self) -> DeviceRect {
        let l = self.l.floor() as i32;
        let t = self.t.floor() as i32;
        let r = self.r.ceil() as i32;
        let b = self.b.ceil() as i32;
        DeviceRect::new(l, t, (r - l).max(0) as u32, (b - t).max(0) as u32)
    }
}

/// 圆角矩形内部“肯定完全覆盖”的两块区域（横条、竖条），逐像素循环时用来跳过。
struct Interior {
    a: DRect,
    b: DRect,
}

impl Interior {
    fn new(s: DRect, radius: f32, margin: f32) -> Interior {
        let m = margin + 1.0;
        Interior {
            a: DRect {
                l: s.l + m,
                t: s.t + radius + m,
                r: s.r - m,
                b: s.b - radius - m,
            },
            b: DRect {
                l: s.l + radius + m,
                t: s.t + m,
                r: s.r - radius - m,
                b: s.b - m,
            },
        }
    }

    /// 像素中心纵坐标为 `y` 的那一行里可以跳过的像素列区间 `[from, to)`。
    fn span(&self, y: f32) -> (i32, i32) {
        let pick = if y >= self.a.t && y <= self.a.b {
            &self.a
        } else if y >= self.b.t && y <= self.b.b {
            &self.b
        } else {
            return (0, 0);
        };
        let from = (pick.l - 0.5).ceil() as i32;
        let to = (pick.r - 0.5).floor() as i32 + 1;
        if to > from { (from, to) } else { (0, 0) }
    }
}

/// tiny-skia 实现的 [`Painter`]。坐标先经过变换栈、换算到物理像素、对齐像素网格，
/// 再平移到当前临时像素图的局部坐标。
pub struct SkiaPainter<'a> {
    pixmap: PixmapMut<'a>,
    /// 临时像素图左上角在窗口物理坐标中的位置。
    origin: (i32, i32),
    scale: f32,
    text: &'a mut TextSystem,
    res: Resources<'a>,
    /// 变换栈，栈顶是当前（已复合的）变换。
    transforms: Vec<Affine>,
    /// 裁剪栈，第 0 层是整块像素图。
    clips: Vec<Clip>,
}

impl<'a> SkiaPainter<'a> {
    fn new(
        pixmap: PixmapMut<'a>,
        origin: (i32, i32),
        scale: f32,
        text: &'a mut TextSystem,
        res: Resources<'a>,
    ) -> Self {
        let full = DeviceRect::new(0, 0, pixmap.width(), pixmap.height());
        SkiaPainter {
            pixmap,
            origin,
            scale,
            text,
            res,
            transforms: Vec::new(),
            clips: vec![Clip::new(full, None, true)],
        }
    }

    fn xf(&self) -> Affine {
        self.transforms.last().copied().unwrap_or(Affine::IDENTITY)
    }

    /// 逻辑长度 → 物理长度的倍数（含变换的缩放）。
    fn k(&self) -> f32 {
        self.xf().scale * self.scale
    }

    /// 当前变换不含缩放：边缘对齐像素网格，平移量取整到整像素。
    /// 有缩放（按下缩小之类的短暂动效）时按实际位置抗锯齿绘制。
    fn snapped(&self) -> bool {
        (self.xf().scale - 1.0).abs() < 1e-4
    }

    fn clip(&self) -> &Clip {
        self.clips.last().expect("裁剪栈不为空")
    }

    fn map_point(&self, p: Point) -> (f32, f32) {
        let a = self.xf();
        let s = self.scale;
        let (ox, oy) = (self.origin.0 as f32, self.origin.1 as f32);
        if self.snapped() {
            (p.x * s + (a.dx * s).round() - ox, p.y * s + (a.dy * s).round() - oy)
        } else {
            let k = a.scale * s;
            (p.x * k + a.dx * s - ox, p.y * k + a.dy * s - oy)
        }
    }

    /// 逻辑矩形 → 局部物理坐标。不缩放时四条边分别四舍五入到像素网格：相邻矩形共享同一条边时
    /// 取整结果一致，不会出现缝隙或重叠；非空矩形至少占 1 个物理像素。
    fn map_rect(&self, r: Rect) -> Option<DRect> {
        if r.is_empty() {
            return None;
        }
        let a = self.xf();
        let s = self.scale;
        let (ox, oy) = (self.origin.0 as f32, self.origin.1 as f32);
        if self.snapped() {
            let tx = (a.dx * s).round() - ox;
            let ty = (a.dy * s).round() - oy;
            let l = (r.x * s).round();
            let t = (r.y * s).round();
            let rr = (r.right() * s).round().max(l + 1.0);
            let b = (r.bottom() * s).round().max(t + 1.0);
            Some(DRect {
                l: l + tx,
                t: t + ty,
                r: rr + tx,
                b: b + ty,
            })
        } else {
            let k = a.scale * s;
            let (tx, ty) = (a.dx * s - ox, a.dy * s - oy);
            Some(DRect {
                l: r.x * k + tx,
                t: r.y * k + ty,
                r: r.right() * k + tx,
                b: r.bottom() * k + ty,
            })
        }
    }

    /// 给 tiny-skia 画路径用的裁剪蒙版。`Err(())` 表示裁剪区为空，什么都不用画。
    fn path_mask(clips: &mut [Clip], w: u32, h: u32) -> Result<Option<&Mask>, ()> {
        let clip = clips.last_mut().expect("裁剪栈不为空");
        if clip.rect.is_empty() {
            return Err(());
        }
        if clip.full {
            return Ok(None);
        }
        if clip.mask_exact {
            return Ok(clip.mask.as_deref());
        }
        if clip.path_mask.is_none() {
            let r = clip.rect;
            let mut m = match &clip.mask {
                Some(m) => (**m).clone(),
                None => {
                    let mut m = Mask::new(w, h).ok_or(())?;
                    let data = m.data_mut();
                    for y in r.y..r.bottom() {
                        let row = y as usize * w as usize;
                        data[row + r.x as usize..row + r.right() as usize].fill(255);
                    }
                    m
                }
            };
            if clip.mask.is_some() {
                // 继承来的圆角蒙版：再把本层矩形外的部分清零。
                let data = m.data_mut();
                for y in 0..h as i32 {
                    let row = &mut data[y as usize * w as usize..(y as usize + 1) * w as usize];
                    if y < r.y || y >= r.bottom() {
                        row.fill(0);
                    } else {
                        row[..r.x as usize].fill(0);
                        row[r.right() as usize..].fill(0);
                    }
                }
            }
            clip.path_mask = Some(m);
        }
        Ok(clip.path_mask.as_ref())
    }

    fn fill_path(&mut self, path: &Path, rule: FillRule, paint: &Paint) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Ok(mask) = Self::path_mask(&mut self.clips, w, h) else {
            return;
        };
        self.pixmap.fill_path(path, paint, rule, Transform::identity(), mask);
    }

    /// 不抗锯齿地填充整数矩形（已对齐像素网格），裁剪到当前裁剪矩形。只在裁剪区没有蒙版时用。
    fn fill_device_rect(&mut self, d: DRect, color: Color) {
        let c = self.clip().rect;
        let l = (d.l as i32).max(c.x);
        let t = (d.t as i32).max(c.y);
        let r = (d.r as i32).min(c.right());
        let b = (d.b as i32).min(c.bottom());
        if l >= r || t >= b || color.a == 0 {
            return;
        }
        if let Some(rect) = tiny_skia::Rect::from_ltrb(l as f32, t as f32, r as f32, b as f32) {
            let mut paint = Paint::default();
            paint.set_color_rgba8(color.r, color.g, color.b, color.a);
            paint.anti_alias = false;
            self.pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    /// 可以走“整数矩形、无蒙版”快速路径。
    fn rect_fast_path(&self, radius_px: f32) -> bool {
        self.snapped() && radius_px < 0.5 && self.clip().mask.is_none()
    }

    fn stops(stops: &[GradientStop]) -> Vec<tiny_skia::GradientStop> {
        stops
            .iter()
            .map(|s| {
                let c = s.color;
                tiny_skia::GradientStop::new(s.offset, tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a))
            })
            .collect()
    }

    /// 笔刷对应的 tiny-skia 着色器。完全透明时返回 `None`。
    fn paint_for(&self, brush: &Brush) -> Option<Paint<'a>> {
        let solid = |c: Color| -> Option<Paint<'a>> {
            if c.a == 0 {
                return None;
            }
            let mut p = Paint::default();
            p.set_color_rgba8(c.r, c.g, c.b, c.a);
            Some(p)
        };
        let shader: Shader<'a> = match brush {
            Brush::Solid(c) => return solid(*c),
            Brush::Linear { start, end, stops } => {
                let (x0, y0) = self.map_point(*start);
                let (x1, y1) = self.map_point(*end);
                match LinearGradient::new(
                    tiny_skia::Point::from_xy(x0, y0),
                    tiny_skia::Point::from_xy(x1, y1),
                    Self::stops(stops),
                    SpreadMode::Pad,
                    Transform::identity(),
                ) {
                    Some(s) => s,
                    None => return solid(stops.last()?.color),
                }
            }
            Brush::Radial { center, radius, stops } => {
                let (cx, cy) = self.map_point(*center);
                let c = tiny_skia::Point::from_xy(cx, cy);
                match RadialGradient::new(
                    c,
                    0.0,
                    c,
                    radius * self.k(),
                    Self::stops(stops),
                    SpreadMode::Pad,
                    Transform::identity(),
                ) {
                    Some(s) => s,
                    None => return solid(stops.last()?.color),
                }
            }
            Brush::Backdrop => match self.res.backdrop {
                // 模糊背景与窗口对齐：低分辨率像素 (i, j) 覆盖窗口物理像素 [f·i, f·(i+1))。
                Some((pixmap, f)) => Pattern::new(
                    pixmap.as_ref(),
                    SpreadMode::Pad,
                    FilterQuality::Bilinear,
                    1.0,
                    Transform::from_row(f, 0.0, 0.0, f, -self.origin.0 as f32, -self.origin.1 as f32),
                ),
                None => return solid(self.res.background),
            },
        };
        Some(Paint {
            shader,
            ..Paint::default()
        })
    }

    /// 外阴影 / 内阴影的公共实现：逐像素按覆盖率混合，形状外挖空（外阴影）或形状内保留（内阴影）。
    fn shadow(&mut self, rect: Rect, radius: f32, sh: &Shadow, inset: bool) {
        if sh.color.a == 0 {
            return;
        }
        let Some(r) = self.map_rect(rect) else { return };
        let k = self.k();
        let rad = (radius * k).max(0.0);
        let grow = if inset { -sh.spread * k } else { sh.spread * k };
        let (dx, dy) = (sh.offset.x * k, sh.offset.y * k);
        let s = DRect {
            l: r.l - grow + dx,
            t: r.t - grow + dy,
            r: r.r + grow + dx,
            b: r.b + grow + dy,
        };
        let shape_ok = s.width() > 0.0 && s.height() > 0.0;
        if !inset && !shape_ok {
            return;
        }
        let srad = (rad + grow).max(0.0);
        let sigma = (sh.blur * 0.5 * k).max(0.0);
        let mask = (shape_ok && sigma >= 0.5).then(|| {
            self.res.shadows.get(
                s.width().round().max(1.0) as usize,
                s.height().round().max(1.0) as usize,
                srad,
                sigma,
            )
        });
        let (mx, my) = match &mask {
            Some(m) => (s.l.round() as i32 - m.pad as i32, s.t.round() as i32 - m.pad as i32),
            None => (0, 0),
        };
        let area = if inset {
            r.cover()
        } else {
            match &mask {
                Some(m) => DeviceRect::new(mx, my, m.width as u32, m.height as u32),
                None => DRect {
                    l: s.l - 1.0,
                    t: s.t - 1.0,
                    r: s.r + 1.0,
                    b: s.b + 1.0,
                }
                .cover(),
            }
        };
        let clip = self.clip();
        let Some(area) = area.intersection(&clip.rect) else {
            return;
        };
        let clip_mask = clip.mask.clone();
        let skip = if inset {
            let margin = mask.as_ref().map(|m| m.pad as f32).unwrap_or(0.0);
            Interior::new(s, srad, margin)
        } else {
            Interior::new(r, rad, 0.0)
        };
        // 形状 S 在像素 (px, py) 的（模糊后）覆盖率。
        let shape = |px: i32, py: i32, x: f32, y: f32| -> f32 {
            if !shape_ok {
                return 0.0;
            }
            match &mask {
                Some(m) => {
                    let (u, v) = (px - mx, py - my);
                    if u < 0 || v < 0 || u >= m.width as i32 || v >= m.height as i32 {
                        0.0
                    } else {
                        m.data[v as usize * m.width + u as usize] as f32 / 255.0
                    }
                }
                None => rrect_coverage(x, y, s.l, s.t, s.r, s.b, srad),
            }
        };
        let c = sh.color;
        let (cr, cg, cb, ca) = (c.r as u32, c.g as u32, c.b as u32, c.a as f32);
        let w = self.pixmap.width() as usize;
        let data = self.pixmap.data_mut();
        for py in area.y..area.bottom() {
            let y = py as f32 + 0.5;
            let (s0, s1) = skip.span(y);
            for px in area.x..area.right() {
                if px >= s0 && px < s1 {
                    continue;
                }
                let x = px as f32 + 0.5;
                let cov_r = rrect_coverage(x, y, r.l, r.t, r.r, r.b, rad);
                let a = if inset {
                    if cov_r <= 0.0 {
                        continue;
                    }
                    cov_r * (1.0 - shape(px, py, x, y))
                } else {
                    if cov_r >= 1.0 {
                        continue;
                    }
                    shape(px, py, x, y) * (1.0 - cov_r)
                };
                let mut a8 = (a * ca + 0.5) as u32;
                if let Some(m) = &clip_mask {
                    a8 = mul255(a8, m.data()[py as usize * w + px as usize] as u32);
                }
                if a8 == 0 {
                    continue;
                }
                let i = (py as usize * w + px as usize) * 4;
                blend_px(&mut data[i..i + 4], cr, cg, cb, a8);
            }
        }
    }
}

fn rrect_path(d: DRect, radius: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    push_rounded_rect(&mut pb, d.l, d.t, d.r, d.b, radius);
    pb.finish()
}

fn push_rounded_rect(pb: &mut PathBuilder, l: f32, t: f32, r: f32, b: f32, radius: f32) {
    let rad = radius.min((r - l) / 2.0).min((b - t) / 2.0).max(0.0);
    if rad < 0.01 {
        pb.move_to(l, t);
        pb.line_to(r, t);
        pb.line_to(r, b);
        pb.line_to(l, b);
        pb.close();
        return;
    }
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

    fn fill(&mut self, rect: Rect, radius: f32, brush: &Brush) {
        let Some(d) = self.map_rect(rect) else { return };
        let rad = radius * self.k();
        if let Brush::Solid(c) = brush
            && self.rect_fast_path(rad)
        {
            return self.fill_device_rect(d, *c);
        }
        let Some(paint) = self.paint_for(brush) else { return };
        if let Some(path) = rrect_path(d, rad) {
            self.fill_path(&path, FillRule::Winding, &paint);
        }
    }

    fn stroke(&mut self, rect: Rect, radius: f32, width: f32, brush: &Brush) {
        let Some(o) = self.map_rect(rect) else { return };
        let wpx = if width <= 0.0 {
            1.0
        } else if self.snapped() {
            (width * self.scale).round().max(1.0)
        } else {
            width * self.k()
        };
        let i = DRect {
            l: o.l + wpx,
            t: o.t + wpx,
            r: o.r - wpx,
            b: o.b - wpx,
        };
        let rad = radius * self.k();
        if i.l >= i.r || i.t >= i.b {
            // 边框比矩形还粗：直接填满。
            let Some(paint) = self.paint_for(brush) else { return };
            if let Some(path) = rrect_path(o, rad) {
                self.fill_path(&path, FillRule::Winding, &paint);
            }
            return;
        }
        if let Brush::Solid(c) = brush
            && self.rect_fast_path(rad)
        {
            self.fill_device_rect(DRect { b: i.t, ..o }, *c);
            self.fill_device_rect(DRect { t: i.b, ..o }, *c);
            self.fill_device_rect(
                DRect {
                    l: o.l,
                    t: i.t,
                    r: i.l,
                    b: i.b,
                },
                *c,
            );
            self.fill_device_rect(
                DRect {
                    l: i.r,
                    t: i.t,
                    r: o.r,
                    b: i.b,
                },
                *c,
            );
            return;
        }
        let Some(paint) = self.paint_for(brush) else { return };
        // 外轮廓减内轮廓（奇偶填充），比描边更容易对齐像素。
        let mut pb = PathBuilder::new();
        push_rounded_rect(&mut pb, o.l, o.t, o.r, o.b, rad);
        push_rounded_rect(&mut pb, i.l, i.t, i.r, i.b, (rad - wpx).max(0.0));
        if let Some(path) = pb.finish() {
            self.fill_path(&path, FillRule::EvenOdd, &paint);
        }
    }

    fn draw_shadow(&mut self, rect: Rect, radius: f32, shadow: &Shadow) {
        self.shadow(rect, radius, shadow, false);
    }

    fn draw_inner_shadow(&mut self, rect: Rect, radius: f32, shadow: &Shadow) {
        self.shadow(rect, radius, shadow, true);
    }

    fn draw_text(&mut self, text: &TextLayout, origin: Point, color: Color) {
        let clip = self.clip();
        if clip.rect.is_empty() || color.a == 0 {
            return;
        }
        let clip_rect = clip.rect;
        let clip_mask = clip.mask.clone();
        let a = self.xf();
        let s = self.scale;
        let k = a.scale * s;
        // 文字框原点的窗口物理坐标。不缩放时纵向对齐到整像素，横向保留小数交给 cosmic-text 的亚像素分档。
        let (ox, oy) = if self.snapped() {
            (
                origin.x * s + (a.dx * s).round(),
                (origin.y * s).round() + (a.dy * s).round(),
            )
        } else {
            (origin.x * k + a.dx * s, origin.y * k + a.dy * s)
        };
        let ts = &mut *self.text;
        let (px, py) = self.origin;
        let mask = clip_mask.as_deref().map(|m| m.data());
        for run in text.buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let pg = glyph.physical((ox, oy + run.line_y * k), k);
                let Some(image) = ts.glyphs.get_image(&mut ts.fonts, pg.cache_key) else {
                    continue;
                };
                let x = pg.x + image.placement.left - px;
                let y = pg.y - image.placement.top - py;
                blend_glyph(&mut self.pixmap, image, x, y, clip_rect, mask, color);
            }
        }
    }

    fn draw_image(&mut self, image: &Image, rect: Rect, opacity: f32) {
        let Some(d) = self.map_rect(rect) else { return };
        let (iw, ih) = (image.width() as f32, image.height() as f32);
        if iw <= 0.0 || ih <= 0.0 || opacity <= 0.0 {
            return;
        }
        let shader = Pattern::new(
            image.pixmap.as_ref(),
            SpreadMode::Pad,
            FilterQuality::Bilinear,
            opacity,
            Transform::from_row(d.width() / iw, 0.0, 0.0, d.height() / ih, d.l, d.t),
        );
        let paint = Paint {
            shader,
            ..Paint::default()
        };
        if let Some(path) = rrect_path(d, 0.0) {
            self.fill_path(&path, FillRule::Winding, &paint);
        }
    }

    fn draw_line(&mut self, from: Point, to: Point, width: f32, color: Color) {
        if color.a == 0 {
            return;
        }
        let (x0, y0) = self.map_point(from);
        let (x1, y1) = self.map_point(to);
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        let Some(path) = pb.finish() else { return };
        let stroke = Stroke {
            width: if width <= 0.0 { 1.0 } else { width * self.k() },
            line_cap: LineCap::Round,
            ..Stroke::default()
        };
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.r, color.g, color.b, color.a);
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Ok(mask) = Self::path_mask(&mut self.clips, w, h) else {
            return;
        };
        self.pixmap
            .stroke_path(&path, &paint, &stroke, Transform::identity(), mask);
    }

    fn fill_polygon(&mut self, points: &[Point], color: Color) {
        if points.len() < 3 || color.a == 0 {
            return;
        }
        let mut pb = PathBuilder::new();
        for (i, p) in points.iter().enumerate() {
            let (x, y) = self.map_point(*p);
            if i == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        pb.close();
        let Some(path) = pb.finish() else { return };
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.r, color.g, color.b, color.a);
        self.fill_path(&path, FillRule::Winding, &paint);
    }

    fn push_rounded_clip(&mut self, rect: Rect, radius: f32) {
        let parent = self.clip();
        let parent_rect = parent.rect;
        let parent_mask = parent.mask.clone();
        let parent_full = parent.full;
        let empty = Clip::new(DeviceRect::new(parent_rect.x, parent_rect.y, 0, 0), None, false);
        let Some(d) = self.map_rect(rect) else {
            self.clips.push(empty);
            return;
        };
        let rad = radius * self.k();
        let Some(bbox) = d.cover().intersection(&parent_rect) else {
            self.clips.push(empty);
            return;
        };
        if self.snapped() && rad < 0.5 {
            // 整数矩形：不需要新蒙版，继承上一层的（如果有）。
            let full = parent_full && bbox == parent_rect;
            self.clips.push(Clip::new(bbox, parent_mask, full));
            return;
        }
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let mut mask = match parent_mask {
            Some(m) => (*m).clone(),
            None => {
                let Some(mut m) = Mask::new(w, h) else {
                    self.clips.push(empty);
                    return;
                };
                let data = m.data_mut();
                for y in parent_rect.y..parent_rect.bottom() {
                    let row = y as usize * w as usize;
                    data[row + parent_rect.x as usize..row + parent_rect.right() as usize].fill(255);
                }
                m
            }
        };
        if let Some(path) = rrect_path(d, rad) {
            mask.intersect_path(&path, FillRule::Winding, true, Transform::identity());
        }
        self.clips.push(Clip {
            rect: bbox,
            mask: Some(Rc::new(mask)),
            mask_exact: true,
            path_mask: None,
            full: false,
        });
    }

    fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
    }

    fn push_transform(&mut self, t: Affine) {
        let cur = self.xf();
        self.transforms.push(t.then(cur));
    }

    fn pop_transform(&mut self) {
        self.transforms.pop();
    }
}

/// `a * b / 255`，四舍五入。
#[inline]
fn mul255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

/// 纯色按透明度 `a`（0–255）source-over 混合进一个预乘像素。
#[inline]
fn blend_px(dst: &mut [u8], r: u32, g: u32, b: u32, a: u32) {
    let inv = 255 - a;
    dst[0] = (mul255(r, a) + mul255(dst[0] as u32, inv)) as u8;
    dst[1] = (mul255(g, a) + mul255(dst[1] as u32, inv)) as u8;
    dst[2] = (mul255(b, a) + mul255(dst[2] as u32, inv)) as u8;
    dst[3] = (a + mul255(dst[3] as u32, inv)) as u8;
}

/// 把一个字形位图按 source-over 混合进像素图（像素图是预乘 alpha）。
/// `mask` 是当前裁剪层的逐像素覆盖率（与像素图同尺寸），没有时只按 `clip` 矩形裁剪。
fn blend_glyph(
    pixmap: &mut PixmapMut,
    image: &cosmic_text::SwashImage,
    x: i32,
    y: i32,
    clip: DeviceRect,
    mask: Option<&[u8]>,
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
    let clip_at = |px: i32, py: i32| {
        mask.map(|m| m[py as usize * stride + px as usize] as u32)
            .unwrap_or(255)
    };
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
                    let a = mul255(mul255(cov, ca), clip_at(px, py));
                    if a == 0 {
                        continue;
                    }
                    let i = (dst_row + px as usize) * 4;
                    blend_px(&mut dst[i..i + 4], cr, cg, cb, a);
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
                    let a = mul255(mul255(image.data[s + 3] as u32, ca), clip_at(px, py));
                    if a == 0 {
                        continue;
                    }
                    let i = (dst_row + px as usize) * 4;
                    blend_px(
                        &mut dst[i..i + 4],
                        image.data[s] as u32,
                        image.data[s + 1] as u32,
                        image.data[s + 2] as u32,
                        a,
                    );
                }
            }
        }
        // 请求的是灰度格式，不会出现子像素蒙版。
        SwashContent::SubpixelMask => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::FontSetup;

    fn texts() -> TextSystem {
        // 不加载任何字体：这些测试不画字。
        TextSystem::new(&FontSetup::default())
    }

    fn px(c: &Canvas, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * c.width() + x) * 4) as usize;
        c.pixels()[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn outer_shadow_is_knocked_out_and_inner_edge_is_one_pixel() {
        let mut text = texts();
        for scale in [1.0f32, 1.25, 1.5] {
            let mut c = Canvas::new();
            c.set_background(Background::Solid(Color::WHITE));
            let (w, h) = ((200.0 * scale) as u32, (200.0 * scale) as u32);
            c.resize(w, h);
            let rect = Rect::new(50.0, 50.0, 100.0, 60.0);
            c.paint_region(DeviceRect::new(0, 0, w, h), scale, &mut text, |p| {
                p.draw_shadow(rect, 10.0, &Shadow::new(0.0, 16.0, 40.0, Color::rgbaf(0, 0, 0, 0.5)));
                let hl = p.hairline();
                p.draw_inner_shadow(rect, 10.0, &Shadow::new(0.0, hl, 0.0, Color::BLACK));
            });
            let s = |v: f32| (v * scale) as u32;
            // 矩形内部（离边缘远处）没有被阴影弄脏：外阴影挖空了形状本身。
            assert_eq!(px(&c, s(100.0), s(80.0)), [255, 255, 255, 255], "scale {scale}");
            // 矩形正下方有阴影。
            let below = px(&c, s(100.0), s(120.0));
            assert!(below[0] < 230, "scale {scale}: {below:?}");
            // 顶部内边：正好一行物理像素被画黑，下一行不受影响。
            let top = (50.0 * scale).round() as u32;
            assert_eq!(px(&c, s(100.0), top), [0, 0, 0, 255], "scale {scale}");
            assert_eq!(px(&c, s(100.0), top + 1), [255, 255, 255, 255], "scale {scale}");
        }
    }

    #[test]
    fn backdrop_brush_is_window_aligned_and_transforms_snap() {
        #[derive(Debug)]
        struct Halves;
        impl Scene for Halves {
            fn paint(&self, p: &mut dyn Painter, size: Size) {
                p.fill_rect(
                    Rect::new(0.0, 0.0, size.width / 2.0, size.height),
                    Color::rgb(200, 0, 0),
                );
                p.fill_rect(
                    Rect::new(size.width / 2.0, 0.0, size.width / 2.0, size.height),
                    Color::rgb(0, 0, 200),
                );
            }
        }
        let mut text = texts();
        let mut c = Canvas::new();
        c.set_background(Background::Scene(Rc::new(Halves)));
        c.set_backdrop_style(BackdropStyle {
            blur: 0.0,
            saturation: 1.0,
            downsample: 1,
        });
        c.resize(100, 50);
        // 两个脏区分别画：结果应和整窗一次画完一致（背景按窗口坐标取样）。
        let draw = |p: &mut dyn Painter| {
            p.push_transform(Affine::translate(0.4, 0.0));
            p.fill(Rect::new(10.0, 10.0, 80.0, 30.0), 0.0, &Brush::Backdrop);
            p.pop_transform();
        };
        c.paint_region(DeviceRect::new(0, 0, 50, 50), 1.0, &mut text, draw);
        c.paint_region(DeviceRect::new(50, 0, 50, 50), 1.0, &mut text, draw);
        assert_eq!(px(&c, 20, 20), [200, 0, 0, 255]);
        assert_eq!(px(&c, 80, 20), [0, 0, 200, 255]);
        // 不含缩放的平移按整像素取整：0.4 像素的平移不产生半透明边缘。
        assert_eq!(px(&c, 10, 20), [200, 0, 0, 255]);
    }

    #[test]
    fn rounded_clip_masks_fills_and_shadows() {
        let mut text = texts();
        let mut c = Canvas::new();
        c.set_background(Background::Solid(Color::WHITE));
        c.resize(100, 100);
        c.paint_region(DeviceRect::new(0, 0, 100, 100), 1.0, &mut text, |p| {
            p.push_rounded_clip(Rect::new(0.0, 0.0, 100.0, 100.0), 40.0);
            p.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::BLACK);
            p.pop_clip();
        });
        // 角落在圆角外，保持白色；中心被填黑。
        assert_eq!(px(&c, 1, 1), [255, 255, 255, 255]);
        assert_eq!(px(&c, 50, 50), [0, 0, 0, 255]);
    }
}
