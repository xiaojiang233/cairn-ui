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
    FillRule, FilterQuality, IntSize, LineCap, LineJoin, LinearGradient, Mask, Paint, Path, PathBuilder, Pattern,
    Pixmap, PixmapMut, RadialGradient, Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

use crate::effects::{self, NOISE_SIZE, ShadowCache, rrect_coverage};
use crate::geom::{Affine, Color, DeviceRect, Point, Rect, Size};
use crate::image::Image;
use crate::painter::{self, Brush, GradientStop, Painter, Shadow, StrokeStyle};
use crate::path::{Path as VPath, Seg};
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
            blur: 34.0,
            saturation: 1.0,
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
    /// 模糊背景按窗口分辨率放大后的整幅图(不透明)。玻璃面板填充时直接拷贝它,不用每次重新做双线性采样。
    backdrop_full: Option<Pixmap>,
    shadows: ShadowCache,
    /// 阴影缓存对应的缩放因子（位模式）。
    shadow_scale: Option<u32>,
    /// 透明度图层用过的缓冲区，留着下一帧复用（动画期间每帧都要用）。
    layer_pool: Vec<Vec<u8>>,
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
            backdrop_full: None,
            shadows: ShadowCache::default(),
            shadow_scale: None,
            layer_pool: Vec::new(),
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
        self.backdrop_full = None;
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
                backdrop_full: None,
                background: Color::BLACK,
                shadows: &mut self.shadows,
                layer_pool: &mut self.layer_pool,
            };
            let mut p = SkiaPainter::new(pixmap.as_mut(), (0, 0), scale, text, res);
            scene.paint(&mut p, Size::new(self.width as f32 / scale, self.height as f32 / scale));
        }
        let st = self.backdrop_style;
        let f = st.downsample.max(1) as usize;
        let (mut buf, lw, lh) = effects::downsample_rgba(pixmap.data(), self.width as usize, self.height as usize, f);
        if st.blur > 0.0 {
            // 采用 CairnPhi 的 9 抽头可分离高斯模糊算法与暗调衰减
            let effective_passes = (st.blur * scale / 17.0).clamp(1.0, 6.0);
            let passes = effective_passes.floor() as u32;
            let spread = 1.0 + effective_passes.fract();
            effects::cairn_phi_gaussian_blur(&mut buf, lw, lh, 4, passes, spread);
            effects::adjust_brightness(&mut buf, 0.95);
        }
        effects::saturate(&mut buf, st.saturation);
        let data: Vec<u8> = buf.iter().map(|&v| ((v as u32 + 128) / 257) as u8).collect();
        self.backdrop = IntSize::from_wh(lw as u32, lh as u32).and_then(|s| Pixmap::from_vec(data, s));
        self.backdrop_full = self
            .backdrop
            .as_ref()
            .and_then(|low| upsample_backdrop(low, f, self.width as usize, self.height as usize));
        self.scene = Some(pixmap);
    }

    /// 把一组绘制画进一张透明的离屏位图（物理像素矩形 `rect`，窗口坐标），用作合成层缓存：
    /// 动画期间只移动、淡入淡出这张位图，不必每帧重画里面的内容。
    /// 玻璃（[`Brush::Backdrop`]）按位图在窗口里的原位置取背景。
    pub fn render_layer(
        &mut self,
        rect: DeviceRect,
        scale: f32,
        text: &mut TextSystem,
        f: impl FnOnce(&mut dyn Painter),
    ) -> Option<Image> {
        let bounds = DeviceRect::new(0, 0, self.width, self.height);
        let rect = rect.intersection(&bounds)?;
        self.ensure_layers(scale, text);
        let mut pixmap = Pixmap::new(rect.width, rect.height)?;
        {
            let factor = self.backdrop_style.downsample.max(1) as f32;
            let solid = match &self.background {
                Background::Solid(c) => *c,
                Background::Scene(_) => Color::BLACK,
            };
            let res = Resources {
                backdrop: self.backdrop.as_ref().map(|b| (b, factor)),
                backdrop_full: self.backdrop_full.as_ref(),
                background: solid,
                shadows: &mut self.shadows,
                layer_pool: &mut self.layer_pool,
            };
            let mut painter = SkiaPainter::new(pixmap.as_mut(), (rect.x, rect.y), scale, text, res);
            f(&mut painter);
        }
        Some(Image { pixmap })
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
                backdrop_full: self.backdrop_full.as_ref(),
                background: solid,
                shadows: &mut self.shadows,
                layer_pool: &mut self.layer_pool,
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
    /// 模糊背景放大到窗口分辨率的整幅图。
    backdrop_full: Option<&'a Pixmap>,
    /// 没有场景时 [`Brush::Backdrop`] 退化成的颜色。
    background: Color,
    shadows: &'a mut ShadowCache,
    layer_pool: &'a mut Vec<Vec<u8>>,
}

/// 一层透明度图层：与像素图同尺寸、同坐标的离屏缓冲（预乘 RGBA，初始全透明）。
/// 尺寸相同，裁剪蒙版就能原样沿用。
struct Layer {
    buf: Vec<u8>,
    opacity: f32,
    /// 合成时要处理的范围（局部物理坐标）。
    area: DeviceRect,
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
    /// 透明度图层栈。非空时所有绘制都落在栈顶的图层上。
    layers: Vec<Layer>,
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
            layers: Vec::new(),
        }
    }

    /// 当前的绘制目标：栈顶的透明度图层，没有图层时是像素图本身。
    /// 只借用这两个字段，调用方可以同时借用裁剪栈、文字系统等其他字段。
    fn target<'b>(pixmap: &'b mut PixmapMut<'a>, layers: &'b mut [Layer]) -> PixmapMut<'b> {
        let (w, h) = (pixmap.width(), pixmap.height());
        match layers.last_mut() {
            Some(l) => PixmapMut::from_bytes(&mut l.buf, w, h).expect("图层与像素图同尺寸"),
            None => PixmapMut::from_bytes(pixmap.data_mut(), w, h).expect("尺寸不变"),
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

    /// 矢量路径经 `at` 和当前变换，换成局部物理像素坐标的 tiny-skia 路径。
    fn device_path(&self, path: &VPath, at: Affine) -> Option<Path> {
        let mut pb = PathBuilder::new();
        let map = |p: &Point| self.map_point(at.apply(*p));
        for seg in path.segments() {
            match seg {
                Seg::Move(a) => {
                    let (x, y) = map(a);
                    pb.move_to(x, y);
                }
                Seg::Line(a) => {
                    let (x, y) = map(a);
                    pb.line_to(x, y);
                }
                Seg::Quad(a, b) => {
                    let ((x1, y1), (x, y)) = (map(a), map(b));
                    pb.quad_to(x1, y1, x, y);
                }
                Seg::Cubic(a, b, c) => {
                    let ((x1, y1), (x2, y2), (x, y)) = (map(a), map(b), map(c));
                    pb.cubic_to(x1, y1, x2, y2, x, y);
                }
                Seg::Close => pb.close(),
            }
        }
        pb.finish()
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

    fn fill_skia_path(&mut self, path: &Path, rule: FillRule, paint: &Paint) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Ok(mask) = Self::path_mask(&mut self.clips, w, h) else {
            return;
        };
        Self::target(&mut self.pixmap, &mut self.layers).fill_path(path, paint, rule, Transform::identity(), mask);
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
            Self::target(&mut self.pixmap, &mut self.layers).fill_rect(rect, &paint, Transform::identity(), None);
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

    /// 把预乘的位图按整像素位置、透明度 `opacity` source-over 混合进当前目标，裁剪到当前裁剪区。
    fn blit(&mut self, image: &Image, x: i32, y: i32, opacity: f32) {
        let clip = self.clip();
        let mask = clip.mask.clone();
        let (iw, ih) = (image.width() as i32, image.height() as i32);
        let Some(area) = DeviceRect::new(x, y, iw as u32, ih as u32).intersection(&clip.rect) else {
            return;
        };
        let a = (opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
        let w = self.pixmap.width() as usize;
        let src = image.pixmap.data();
        let mut target = Self::target(&mut self.pixmap, &mut self.layers);
        let dst = target.data_mut();
        for py in area.y..area.bottom() {
            let s0 = (((py - y) * iw + (area.x - x)) as usize) * 4;
            let d0 = (py as usize * w + area.x as usize) * 4;
            let n = area.width as usize * 4;
            match &mask {
                None => over_row(&mut dst[d0..d0 + n], &src[s0..s0 + n], a),
                Some(m) => {
                    // 圆角蒙版:每个像素的不透明度还要乘上蒙版覆盖率。
                    let mrow = &m.data()[py as usize * w + area.x as usize..][..area.width as usize];
                    for (i, &cov) in mrow.iter().enumerate() {
                        let k = mul255(a, cov as u32);
                        let (s, d) = (&src[s0 + i * 4..s0 + i * 4 + 4], &mut dst[d0 + i * 4..d0 + i * 4 + 4]);
                        d.copy_from_slice(&over_px(px_at(s), px_at(d), k).to_le_bytes());
                    }
                }
            }
        }
    }

    /// 圆角矩形填充的快路径。条件:整数对齐、当前裁剪没有蒙版,笔刷是纯色、竖直渐变或模糊背景。
    /// 中间的大块逐行直接写像素(模糊背景是整行拷贝),只有四个圆角按到圆心的距离估算覆盖率、逐像素混合。
    /// 比走 tiny-skia 的着色器管线快一个数量级,玻璃面板整块填充都靠它。返回是否已经画完。
    fn fill_rrect_fast(&mut self, d: DRect, rad: f32, brush: &Brush) -> bool {
        if self.clip().mask.is_some() {
            return false;
        }
        let cr = self.clip().rect;
        let (l, t, r, b) = (
            d.l.round() as i32,
            d.t.round() as i32,
            d.r.round() as i32,
            d.b.round() as i32,
        );
        if r - l < 2 || b - t < 2 {
            return false;
        }
        let rad = rad.min(((r - l) / 2) as f32).min(((b - t) / 2) as f32).max(0.0);
        let c = rad.ceil() as i32;

        enum Source<'s> {
            Solid(u32),
            /// 竖直渐变:起点终点的纵坐标(局部物理像素)和色标。
            Rows(f32, f32, &'s [GradientStop]),
            Backdrop(&'s Pixmap),
        }
        let source = match brush {
            Brush::Solid(color) => {
                if color.a == 0 {
                    return true;
                }
                let p = premultiply(*color);
                Source::Solid(u32::from_le_bytes(p))
            }
            Brush::Linear { start, end, stops } => {
                let ((x0, y0), (x1, y1)) = (self.map_point(*start), self.map_point(*end));
                if (x0 - x1).abs() > 0.01 || (y1 - y0).abs() < 0.01 || stops.is_empty() {
                    return false;
                }
                Source::Rows(y0, y1, stops)
            }
            Brush::Backdrop => match self.res.backdrop_full {
                Some(full) => Source::Backdrop(full),
                None => return false,
            },
            Brush::Radial { .. } => return false,
        };
        let (ox, oy) = self.origin;
        let w = self.pixmap.width() as usize;
        let res = &self.res;
        let mut target = Self::target(&mut self.pixmap, &mut self.layers);
        let data = target.data_mut();
        // 一行里 [x0, x1) 这一段用笔刷颜色(覆盖率 cov)铺上去。
        let paint_span = |data: &mut [u8], y: i32, x0: i32, x1: i32, cov: u32| {
            if x1 <= x0 {
                return;
            }
            let row = &mut data[(y as usize * w + x0 as usize) * 4..(y as usize * w + x1 as usize) * 4];
            match &source {
                Source::Solid(color) => fill_row_color(row, *color, cov),
                Source::Rows(y0, y1, stops) => fill_row_color(
                    row,
                    gradient_color(stops, ((y as f32 + 0.5 - y0) / (y1 - y0)).clamp(0.0, 1.0)),
                    cov,
                ),
                Source::Backdrop(full) => {
                    let fw = full.width() as usize;
                    let (gx, gy) = ((x0 + ox) as usize, (y + oy) as usize);
                    let src = &full.data()[(gy * fw + gx) * 4..(gy * fw + gx + (x1 - x0) as usize) * 4];
                    if cov >= 255 {
                        row.copy_from_slice(src);
                    } else {
                        over_row(row, src, cov);
                    }
                }
            }
        };
        let _ = res;
        let (top, bottom) = (t.max(cr.y), b.min(cr.bottom()));
        for y in top..bottom {
            let edge = y - t < c || b - 1 - y < c;
            let (mx0, mx1) = if edge { (l + c, r - c) } else { (l, r) };
            paint_span(data, y, mx0.max(cr.x), mx1.min(cr.right()), 255);
            if !edge {
                continue;
            }
            // 圆角带里两头的 c 列:覆盖率按像素中心到圆心的距离算。
            let py = y as f32 + 0.5;
            let dy = ((t as f32 + rad) - py).max(py - (b as f32 - rad)).max(0.0);
            for x in (l..(l + c).min(r)).chain((r - c).max(l + c)..r) {
                if x < cr.x || x >= cr.right() {
                    continue;
                }
                let px = x as f32 + 0.5;
                let dx = ((l as f32 + rad) - px).max(px - (r as f32 - rad)).max(0.0);
                let cov = (0.5 + rad - dx.hypot(dy)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    paint_span(data, y, x, x + 1, (cov * 255.0).round() as u32);
                }
            }
        }
        true
    }

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
        let mut target = Self::target(&mut self.pixmap, &mut self.layers);
        let data = target.data_mut();
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
        if self.snapped() && rad >= 0.5 && self.fill_rrect_fast(d, rad, brush) {
            return;
        }
        let Some(paint) = self.paint_for(brush) else { return };
        if let Some(path) = rrect_path(d, rad) {
            self.fill_skia_path(&path, FillRule::Winding, &paint);
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
                self.fill_skia_path(&path, FillRule::Winding, &paint);
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
            self.fill_skia_path(&path, FillRule::EvenOdd, &paint);
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
        let mut target = Self::target(&mut self.pixmap, &mut self.layers);
        let lut = coverage_lut(color);
        for run in text.buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let mut pg = glyph.physical((ox, oy + run.line_y * k), k);
                pg.cache_key.flags |= cosmic_text::CacheKeyFlags::DISABLE_HINTING;
                let Some(image) = ts.glyphs.get_image(&mut ts.fonts, pg.cache_key) else {
                    continue;
                };
                let x = pg.x + image.placement.left - px;
                let y = pg.y - image.placement.top - py;
                blend_glyph(&mut target, image, (x, y), clip_rect, mask, color, &lut);
            }
        }
    }

    fn draw_image(&mut self, image: &Image, rect: Rect, opacity: f32) {
        let Some(d) = self.map_rect(rect) else { return };
        let (iw, ih) = (image.width() as f32, image.height() as f32);
        if iw <= 0.0 || ih <= 0.0 || opacity <= 0.0 {
            return;
        }
        // 1:1 贴到整像素位置（合成层缓存的常见情况）：逐像素按透明度混合，不走重新采样的着色器。
        if self.snapped() && (d.width() - iw).abs() < 0.5 && (d.height() - ih).abs() < 0.5 {
            self.blit(image, d.l.round() as i32, d.t.round() as i32, opacity);
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
            self.fill_skia_path(&path, FillRule::Winding, &paint);
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
        Self::target(&mut self.pixmap, &mut self.layers).stroke_path(
            &path,
            &paint,
            &stroke,
            Transform::identity(),
            mask,
        );
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
        self.fill_skia_path(&path, FillRule::Winding, &paint);
    }

    fn fill_path(&mut self, path: &VPath, at: Affine, rule: painter::FillRule, brush: &Brush) {
        let Some(paint) = self.paint_for(brush) else { return };
        let Some(path) = self.device_path(path, at) else { return };
        let rule = match rule {
            painter::FillRule::NonZero => FillRule::Winding,
            painter::FillRule::EvenOdd => FillRule::EvenOdd,
        };
        self.fill_skia_path(&path, rule, &paint);
    }

    fn stroke_path(&mut self, path: &VPath, at: Affine, stroke: &StrokeStyle, brush: &Brush) {
        let Some(paint) = self.paint_for(brush) else { return };
        let Some(path) = self.device_path(path, at) else { return };
        // 线宽和虚线长度都在路径坐标系里，先按变换的缩放、再按屏幕缩放换成物理像素。
        let k = at.scale * self.k();
        let dash = (stroke.dash.len() >= 2 && stroke.dash.len().is_multiple_of(2))
            .then(|| StrokeDash::new(stroke.dash.iter().map(|d| d * k).collect(), stroke.dash_offset * k))
            .flatten();
        let skia = Stroke {
            width: if stroke.width <= 0.0 { 1.0 } else { stroke.width * k },
            miter_limit: stroke.miter_limit,
            line_cap: match stroke.cap {
                painter::LineCap::Butt => LineCap::Butt,
                painter::LineCap::Round => LineCap::Round,
                painter::LineCap::Square => LineCap::Square,
            },
            line_join: match stroke.join {
                painter::LineJoin::Miter => LineJoin::Miter,
                painter::LineJoin::Round => LineJoin::Round,
                painter::LineJoin::Bevel => LineJoin::Bevel,
            },
            dash,
        };
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Ok(mask) = Self::path_mask(&mut self.clips, w, h) else {
            return;
        };
        Self::target(&mut self.pixmap, &mut self.layers).stroke_path(&path, &paint, &skia, Transform::identity(), mask);
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

    fn push_layer(&mut self, rect: Rect, opacity: f32) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let len = w as usize * h as usize * 4;
        let mut buf = self.res.layer_pool.pop().unwrap_or_default();
        buf.clear();
        buf.resize(len, 0);
        // 图层内容只可能落在当前裁剪区里；`rect` 只是提示（子元素的阴影之类可能画到它外面），不用来缩小范围。
        let _ = rect;
        let area = self.clip().rect;
        self.layers.push(Layer {
            buf,
            opacity: opacity.clamp(0.0, 1.0),
            area,
        });
    }

    fn pop_layer(&mut self) {
        let Some(layer) = self.layers.pop() else { return };
        let a = (layer.opacity * 255.0).round() as u32;
        if a > 0 && !layer.area.is_empty() {
            let w = self.pixmap.width() as usize;
            let mut target = Self::target(&mut self.pixmap, &mut self.layers);
            let dst = target.data_mut();
            let r = layer.area;
            let n = r.width as usize * 4;
            for y in r.y..r.bottom() {
                let i = (y as usize * w + r.x as usize) * 4;
                over_row(&mut dst[i..i + n], &layer.buf[i..i + n], a);
            }
        }
        self.res.layer_pool.push(layer.buf);
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

/// 两路并行的 `x * k / 255`（四舍五入）:`px` 的四个通道各占一个字节。
#[inline(always)]
fn scale_px(px: u32, k: u32) -> u32 {
    let rb = (px & 0x00FF_00FF) * k + 0x0080_0080;
    let ag = ((px >> 8) & 0x00FF_00FF) * k + 0x0080_0080;
    let rb = ((rb + ((rb >> 8) & 0x00FF_00FF)) >> 8) & 0x00FF_00FF;
    let ag = (ag + ((ag >> 8) & 0x00FF_00FF)) & 0xFF00_FF00;
    rb | ag
}

/// 逐通道饱和相加。
#[inline(always)]
fn add_px(a: u32, b: u32) -> u32 {
    let sat = |v: u32| (v | (((v >> 8) & 0x0001_0001) * 255)) & 0x00FF_00FF;
    let rb = (a & 0x00FF_00FF) + (b & 0x00FF_00FF);
    let ag = ((a >> 8) & 0x00FF_00FF) + ((b >> 8) & 0x00FF_00FF);
    sat(rb) | (sat(ag) << 8)
}

/// 预乘像素 `src` 先乘以不透明度 `k`（0–255），再 source-over 到预乘像素 `dst` 上。
/// 小端字节序下 alpha 在最高字节。没有分支，编译器能把外层循环向量化。
#[inline(always)]
fn over_px(src: u32, dst: u32, k: u32) -> u32 {
    let s = scale_px(src, k);
    add_px(s, scale_px(dst, 255 - (s >> 24)))
}

#[inline(always)]
fn px_at(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// 一行预乘 RGBA 按不透明度 `k`（0–255）source-over 到另一行上。两行等长。
fn over_row(dst: &mut [u8], src: &[u8], k: u32) {
    if k >= 255 {
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            let sp = px_at(s);
            d.copy_from_slice(&add_px(sp, scale_px(px_at(d), 255 - (sp >> 24))).to_le_bytes());
        }
    } else {
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            d.copy_from_slice(&over_px(px_at(s), px_at(d), k).to_le_bytes());
        }
    }
}

/// 用同一个预乘颜色铺满一段，按 `cov`（0–255）的覆盖率混合。
fn fill_row_color(dst: &mut [u8], color: u32, cov: u32) {
    let s = if cov >= 255 { color } else { scale_px(color, cov) };
    if s >> 24 == 255 {
        let bytes = s.to_le_bytes();
        for d in dst.chunks_exact_mut(4) {
            d.copy_from_slice(&bytes);
        }
    } else {
        let a = 255 - (s >> 24);
        for d in dst.chunks_exact_mut(4) {
            d.copy_from_slice(&add_px(s, scale_px(px_at(d), a)).to_le_bytes());
        }
    }
}

/// 把低分辨率的模糊背景（每个像素盖住窗口里 `f × f` 个像素）双线性放大成整个窗口大小的不透明图。
/// 采样规则与 [`Brush::Backdrop`] 的着色器一致（像素中心对齐、边缘取最近的像素）。
fn upsample_backdrop(low: &Pixmap, f: usize, w: usize, h: usize) -> Option<Pixmap> {
    let (lw, lh) = (low.width() as usize, low.height() as usize);
    if lw == 0 || lh == 0 || f == 0 {
        return None;
    }
    let src = low.data();
    let mut out = Pixmap::new(w as u32, h as u32)?;
    // (左邻、右邻、右邻的权重 0–256),水平方向只和列有关,先算好。
    let axis = |n: usize, limit: usize| -> Vec<(usize, usize, u32)> {
        (0..n)
            .map(|i| {
                let u = ((i as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (limit - 1) as f32);
                let i0 = u.floor() as usize;
                (i0, (i0 + 1).min(limit - 1), ((u - i0 as f32) * 256.0) as u32)
            })
            .collect()
    };
    let xs = axis(w, lw);
    let ys = axis(h, lh);
    let dst = out.data_mut();
    for (y, &(j0, j1, fy)) in ys.iter().enumerate() {
        let (row0, row1) = (
            &src[j0 * lw * 4..(j0 + 1) * lw * 4],
            &src[j1 * lw * 4..(j1 + 1) * lw * 4],
        );
        let drow = &mut dst[y * w * 4..(y + 1) * w * 4];
        for (x, &(i0, i1, fx)) in xs.iter().enumerate() {
            for c in 0..4 {
                let top = row0[i0 * 4 + c] as u32 * (256 - fx) + row0[i1 * 4 + c] as u32 * fx;
                let bottom = row1[i0 * 4 + c] as u32 * (256 - fx) + row1[i1 * 4 + c] as u32 * fx;
                drow[x * 4 + c] = ((top * (256 - fy) + bottom * fy + 32768) >> 16) as u8;
            }
        }
    }
    Some(out)
}

/// 渐变在 `t`（0–1）处的预乘颜色。色标之间在非预乘空间里插值（与 tiny-skia 一致）。
fn gradient_color(stops: &[GradientStop], t: f32) -> u32 {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return 0;
    };
    let c = if t <= first.offset {
        first.color
    } else if t >= last.offset {
        last.color
    } else {
        let i = stops
            .windows(2)
            .position(|w| t >= w[0].offset && t <= w[1].offset)
            .unwrap_or(0);
        let (a, b) = (&stops[i], &stops[i + 1]);
        let span = (b.offset - a.offset).max(1e-6);
        a.color.mix(b.color, (t - a.offset) / span)
    };
    let a = c.a as u32;
    u32::from_le_bytes([
        mul255(c.r as u32, a) as u8,
        mul255(c.g as u32, a) as u8,
        mul255(c.b as u32, a) as u8,
        c.a,
    ])
}

/// 文字覆盖率的校正表。
///
/// 混合在 sRGB 空间里做（没有转到线性空间），浅色字压在深色底上时，半覆盖的边缘像素偏暗，
/// 笔画看起来比实际细；深色字在浅底上则相反。这里按文字颜色的亮度把覆盖率提一提：
/// 白字相当于 `覆盖率^(1/1.8)`（接近线性空间混合的结果），黑字不变。
fn coverage_lut(color: Color) -> [u8; 256] {
    let lum = (0.2126 * color.r as f32 + 0.7152 * color.g as f32 + 0.0722 * color.b as f32) / 255.0;
    let t = ((lum - 0.35) / 0.65).clamp(0.0, 1.0);
    let exp = 1.0 - t * (1.0 - 1.0 / 1.8);
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = ((i as f32 / 255.0).powf(exp) * 255.0 + 0.5) as u8;
    }
    lut
}

/// 把一个字形位图按 source-over 混合进像素图（像素图是预乘 alpha）。
/// `mask` 是当前裁剪层的逐像素覆盖率（与像素图同尺寸），没有时只按 `clip` 矩形裁剪。
fn blend_glyph(
    pixmap: &mut PixmapMut,
    image: &cosmic_text::SwashImage,
    (x, y): (i32, i32),
    clip: DeviceRect,
    mask: Option<&[u8]>,
    color: Color,
    lut: &[u8; 256],
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
                    let cov = lut[image.data[src_row + (px - x) as usize] as usize] as u32;
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

    /// 手动跑的耗时测量:`cargo test --release -p cairn-ui-render bench_ -- --ignored --nocapture`。
    /// 用 1178 × 736、1 倍缩放的窗口,量一帧里各种操作大概要多久(ms,取多次的平均)。
    #[test]
    #[ignore = "耗时测量,手动跑"]
    fn bench_paint_costs() {
        use std::time::Instant;
        #[derive(Debug)]
        struct Sky;
        impl Scene for Sky {
            fn paint(&self, p: &mut dyn Painter, size: Size) {
                let r = Rect::new(0.0, 0.0, size.width, size.height);
                p.fill(
                    r,
                    0.0,
                    &Brush::vertical(r, Color::rgb(70, 130, 200), Color::rgb(120, 170, 110)),
                );
            }
        }
        let mut text = texts();
        let mut c = Canvas::new();
        c.set_background(Background::Scene(Rc::new(Sky)));
        c.set_backdrop_style(BackdropStyle {
            blur: 34.0,
            saturation: 1.3,
            downsample: 4,
        });
        c.set_noise(0.02);
        c.resize(1178, 736);
        let region = DeviceRect::new(168, 36, 842, 590);
        let panel = Rect::new(168.0, 36.0, 842.0, 590.0);
        // 一张和面板一样大的位图,模拟合成层缓存。
        let mut cache_px = vec![0u8; 842 * 590 * 4];
        for (i, px) in cache_px.chunks_exact_mut(4).enumerate() {
            let a = if i % 7 == 0 { 120 } else { 220 };
            px.copy_from_slice(&[(i % 200) as u8 / 2, 40, 60, a]);
        }
        let image = Image::from_rgba8(842, 590, cache_px).unwrap();

        let mut time = |name: &str, f: &mut dyn FnMut(&mut dyn Painter)| {
            // 先预热一次(阴影蒙版、字形之类的缓存)。
            c.paint_region(region, 1.0, &mut text, |p| f(p));
            let n = 20;
            let t = Instant::now();
            for _ in 0..n {
                c.paint_region(region, 1.0, &mut text, |p| f(p));
            }
            println!("{name:<34} {:6.2} ms", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        };
        let glass = |p: &mut dyn Painter, b: Rect| {
            p.draw_shadow(b, 20.0, &Shadow::new(0.0, 20.0, 60.0, Color::rgbaf(0, 0, 0, 0.3)));
            p.fill(b, 20.0, &Brush::Backdrop);
            p.fill_rounded_rect(b, 20.0, Color::rgbaf(250, 251, 253, 0.55));
            p.fill(
                b,
                20.0,
                &Brush::vertical(b, Color::WHITE.with_opacity(0.2), Color::WHITE.with_alpha(0)),
            );
            p.draw_inner_shadow(b, 20.0, &Shadow::new(0.0, 1.0, 0.0, Color::WHITE.with_opacity(0.5)));
            p.stroke_rect(b, 20.0, 0.0, Color::WHITE.with_opacity(0.3));
        };
        time("空(背景拷贝 + 噪点 + 回写)", &mut |_| {});
        time("blit 缓存位图 不透明度 1.0", &mut |p| {
            p.draw_image(&image, panel, 1.0)
        });
        time("blit 缓存位图 不透明度 0.6", &mut |p| {
            p.draw_image(&image, panel, 0.6)
        });
        time("blit 缓存位图 平移 + 0.6", &mut |p| {
            p.push_transform(Affine::translate(0.0, 9.0));
            p.draw_image(&image, panel, 0.6);
            p.pop_transform();
        });
        time("玻璃面板(阴影+背景+填充+渐变+边)", &mut |p| {
            glass(p, panel)
        });
        time("  其中:仅阴影", &mut |p| {
            p.draw_shadow(panel, 20.0, &Shadow::new(0.0, 20.0, 60.0, Color::rgbaf(0, 0, 0, 0.3)))
        });
        time("  其中:仅背景模糊填充", &mut |p| {
            p.fill(panel, 20.0, &Brush::Backdrop)
        });
        time("  其中:仅纯色圆角填充", &mut |p| {
            p.fill_rounded_rect(panel, 20.0, Color::rgbaf(250, 251, 253, 0.55))
        });
        time("  其中:仅竖直渐变填充", &mut |p| {
            p.fill(
                panel,
                20.0,
                &Brush::vertical(panel, Color::WHITE.with_opacity(0.2), Color::WHITE.with_alpha(0)),
            )
        });
        time("  其中:仅两条内阴影", &mut |p| {
            p.draw_inner_shadow(panel, 20.0, &Shadow::new(0.0, 1.0, 0.0, Color::WHITE.with_opacity(0.5)));
            p.draw_inner_shadow(
                panel,
                20.0,
                &Shadow::new(0.0, -1.0, 0.0, Color::BLACK.with_opacity(0.2)),
            );
        });
        time("圆角裁剪 + 大填充", &mut |p| {
            p.push_rounded_clip(panel, 20.0);
            p.fill_rect(panel, Color::rgb(30, 60, 90));
            p.pop_clip();
        });
        time("透明度图层(整块)", &mut |p| {
            p.push_layer(panel, 0.6);
            p.fill_rect(panel, Color::rgb(30, 60, 90));
            p.pop_layer();
        });
        time("20 个圆角小块(列表行)", &mut |p| {
            for i in 0..20 {
                p.fill_rounded_rect(
                    Rect::new(200.0, 60.0 + i as f32 * 28.0, 700.0, 24.0),
                    10.0,
                    Color::rgbaf(255, 255, 255, 0.1),
                );
            }
        });
        // 对照:tiny-skia 自己的 draw_pixmap(整数平移、最近邻)贴同一张位图要多久。
        {
            let mut target = Pixmap::new(1178, 736).unwrap();
            let t = Instant::now();
            for _ in 0..20 {
                let paint = tiny_skia::PixmapPaint {
                    opacity: 0.6,
                    blend_mode: tiny_skia::BlendMode::SourceOver,
                    quality: FilterQuality::Nearest,
                };
                target.draw_pixmap(168, 36, image.pixmap.as_ref(), &paint, Transform::identity(), None);
            }
            println!(
                "{:<34} {:6.2} ms",
                "tiny-skia draw_pixmap 0.6",
                t.elapsed().as_secs_f64() * 1000.0 / 20.0
            );
        }
        let sq = crate::path::Path::parse("M4 4 L60 4 L60 60 L4 60 Z M20 20 L20 44 L44 44 L44 20 Z").unwrap();
        time("40 条矢量路径(模拟 10 个图标)", &mut |p| {
            for i in 0..40 {
                let at = Affine {
                    scale: 0.6,
                    dx: 200.0 + (i % 10) as f32 * 40.0,
                    dy: 60.0 + (i / 10) as f32 * 40.0,
                };
                p.fill_path(
                    &sq,
                    at,
                    crate::painter::FillRule::EvenOdd,
                    &Brush::Solid(Color::rgb(200, 150, 60)),
                );
            }
        });
        time("10 个渐变填充的矢量路径", &mut |p| {
            for i in 0..10 {
                let at = Affine {
                    scale: 0.6,
                    dx: 200.0 + (i % 10) as f32 * 40.0,
                    dy: 60.0,
                };
                let b = Brush::linear(
                    Point::new(at.dx, at.dy),
                    Point::new(at.dx, at.dy + 38.0),
                    [(0.0, Color::rgb(250, 220, 150)), (1.0, Color::rgb(200, 150, 60))],
                );
                p.fill_path(&sq, at, crate::painter::FillRule::EvenOdd, &b);
            }
        });
    }

    #[test]
    fn vector_paths_fill_and_stroke_at_any_scale() {
        use crate::painter::{FillRule as Rule, LineCap as Cap, StrokeStyle};
        use crate::path::Path as Vector;
        let mut text = texts();
        for scale in [1.0f32, 1.5] {
            let mut c = Canvas::new();
            c.set_background(Background::Solid(Color::WHITE));
            let (w, h) = ((100.0 * scale) as u32, (100.0 * scale) as u32);
            c.resize(w, h);
            // 设计坐标 0–10 的方框映射到窗口里 (20, 20) 起的 40 × 40。
            let at = Affine {
                scale: 4.0,
                dx: 20.0,
                dy: 20.0,
            };
            // 外圈顺时针、内圈逆时针：奇偶规则下内圈被挖空。
            let ring = Vector::parse("M0 0H10V10H0Z M3 3V7H7V3Z").unwrap();
            let line = Vector::parse("M0 12 H10").unwrap();
            c.paint_region(DeviceRect::new(0, 0, w, h), scale, &mut text, |p| {
                p.fill_path(&ring, at, Rule::EvenOdd, &Brush::Solid(Color::rgb(200, 0, 0)));
                p.stroke_path(
                    &line,
                    at,
                    &StrokeStyle::new(1.0).cap(Cap::Butt),
                    &Brush::Solid(Color::rgb(0, 0, 200)),
                );
            });
            let s = |v: f32| (v * scale) as u32;
            // 圆环的边：红色；中心被挖空：仍是白色。
            assert_eq!(px(&c, s(23.0), s(40.0)), [200, 0, 0, 255], "scale {scale}");
            assert_eq!(px(&c, s(40.0), s(40.0)), [255, 255, 255, 255], "scale {scale}");
            // 线宽 1 个设计单位 = 4 个逻辑像素：线在 y = 20 + 12 × 4 = 68 附近，上下各 2 像素。
            assert_eq!(px(&c, s(40.0), s(68.0)), [0, 0, 200, 255], "scale {scale}");
            assert_eq!(px(&c, s(40.0), s(60.0)), [255, 255, 255, 255], "scale {scale}");
        }
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
