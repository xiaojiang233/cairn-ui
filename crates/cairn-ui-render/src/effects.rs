//! 自己实现的像素算法：圆角矩形覆盖率、盒式模糊（近似高斯）、降采样、饱和度、噪点、阴影蒙版缓存。
//!
//! 都是对普通切片的操作，不依赖 tiny-skia，便于单独测试。

use std::collections::HashMap;
use std::rc::Rc;

/// 像素中心 `(px, py)` 处圆角矩形的覆盖率（0–1，边缘约 1 像素宽的抗锯齿过渡）。
/// 坐标、半径都是物理像素。
#[inline]
pub(crate) fn rrect_coverage(px: f32, py: f32, l: f32, t: f32, r: f32, b: f32, radius: f32) -> f32 {
    let hw = (r - l) * 0.5;
    let hh = (b - t) * 0.5;
    if hw <= 0.0 || hh <= 0.0 {
        return 0.0;
    }
    let rad = radius.min(hw).min(hh).max(0.0);
    let qx = (px - (l + hw)).abs() - (hw - rad);
    let qy = (py - (t + hh)).abs() - (hh - rad);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    let sdf = (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - rad;
    (0.5 - sdf).clamp(0.0, 1.0)
}

/// 用 `n` 次盒式模糊近似标准差为 `sigma` 的高斯模糊时，每次的盒子半径。
/// 算法见 W. Jarosz《Fast Image Convolutions》以及 Ivan Kutskir 的整理。
pub(crate) fn box_radii(sigma: f32, n: usize) -> Vec<usize> {
    if sigma < 0.5 {
        return vec![0; n];
    }
    let nf = n as f32;
    let ideal = (12.0 * sigma * sigma / nf + 1.0).sqrt();
    let mut wl = ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let wlf = wl as f32;
    let m = ((12.0 * sigma * sigma - nf * wlf * wlf - 4.0 * nf * wlf - 3.0 * nf) / (-4.0 * wlf - 4.0)).round() as i32;
    (0..n as i32)
        .map(|i| (if i < m { wl } else { wu } as usize - 1) / 2)
        .collect()
}

/// 一维盒式模糊（边缘按最近像素延伸），从 `src` 读、写到 `dst`，二者长度相同。
fn box_line(src: &[u16], dst: &mut [u16], radius: usize) {
    let n = src.len();
    if radius == 0 || n == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let r = radius as isize;
    let last = n as isize - 1;
    let at = |i: isize| src[i.clamp(0, last) as usize] as u32;
    let width = 2 * radius as u32 + 1;
    let half = width / 2;
    let mut sum: u32 = (-r..=r).map(at).sum();
    for (i, d) in dst.iter_mut().enumerate() {
        *d = ((sum + half) / width) as u16;
        let i = i as isize;
        sum = sum + at(i + r + 1) - at(i - r);
    }
}

/// 对交错存储、每像素 `channels` 个分量的 16 位图像做近似高斯模糊（3 次盒式模糊，横竖各一遍）。
pub(crate) fn gaussian_blur(data: &mut [u16], width: usize, height: usize, channels: usize, sigma: f32) {
    if width == 0 || height == 0 {
        return;
    }
    let radii = box_radii(sigma, 3);
    if radii.iter().all(|&r| r == 0) {
        return;
    }
    let mut a = vec![0u16; width.max(height)];
    let mut b = vec![0u16; width.max(height)];
    for &radius in &radii {
        if radius == 0 {
            continue;
        }
        // 横向
        for y in 0..height {
            let row = y * width * channels;
            for c in 0..channels {
                for x in 0..width {
                    a[x] = data[row + x * channels + c];
                }
                box_line(&a[..width], &mut b[..width], radius);
                for x in 0..width {
                    data[row + x * channels + c] = b[x];
                }
            }
        }
        // 纵向
        for x in 0..width {
            for c in 0..channels {
                for y in 0..height {
                    a[y] = data[(y * width + x) * channels + c];
                }
                box_line(&a[..height], &mut b[..height], radius);
                for y in 0..height {
                    data[(y * width + x) * channels + c] = b[y];
                }
            }
        }
    }
}

/// CairnPhi 的 9 抽头可分离高斯模糊（利用双线性插值采样 5 次等价于 9 抽头高斯滤波）。
/// `passes` 为水平与垂直滤波的迭代次数，`spread` 为采样步进扩展因子。
pub fn cairn_phi_gaussian_blur(
    data: &mut [u16],
    width: usize,
    height: usize,
    channels: usize,
    passes: u32,
    spread: f32,
) {
    if width == 0 || height == 0 || passes == 0 {
        return;
    }
    let spread = spread.max(0.1);
    let o1 = 1.3846153846 * spread;
    let o2 = 3.2307692308 * spread;
    let w0 = 0.2270270270f32;
    let w1 = 0.3162162162f32;
    let w2 = 0.0702702703f32;

    let mut line_in = vec![0u16; width.max(height)];
    let mut line_out = vec![0u16; width.max(height)];

    for _ in 0..passes {
        // 水平滤波
        for y in 0..height {
            let row = y * width * channels;
            for c in 0..channels {
                for x in 0..width {
                    line_in[x] = data[row + x * channels + c];
                }
                blur_line_cairn_phi(&line_in[..width], &mut line_out[..width], o1, o2, w0, w1, w2);
                for x in 0..width {
                    data[row + x * channels + c] = line_out[x];
                }
            }
        }
        // 垂直滤波
        for x in 0..width {
            for c in 0..channels {
                for y in 0..height {
                    line_in[y] = data[(y * width + x) * channels + c];
                }
                blur_line_cairn_phi(&line_in[..height], &mut line_out[..height], o1, o2, w0, w1, w2);
                for y in 0..height {
                    data[(y * width + x) * channels + c] = line_out[y];
                }
            }
        }
    }
}

#[inline(always)]
fn sample_clamped(src: &[u16], x: f32) -> f32 {
    let last = (src.len() - 1) as f32;
    let clamped = x.clamp(0.0, last);
    let i0 = clamped.floor() as usize;
    let i1 = (i0 + 1).min(src.len() - 1);
    let f = clamped - i0 as f32;
    src[i0] as f32 * (1.0 - f) + src[i1] as f32 * f
}

#[inline]
fn blur_line_cairn_phi(
    src: &[u16],
    dst: &mut [u16],
    o1: f32,
    o2: f32,
    w0: f32,
    w1: f32,
    w2: f32,
) {
    let n = src.len();
    if n <= 1 {
        if n == 1 {
            dst[0] = src[0];
        }
        return;
    }
    for i in 0..n {
        let fi = i as f32;
        let c = src[i] as f32 * w0
            + (sample_clamped(src, fi + o1) + sample_clamped(src, fi - o1)) * w1
            + (sample_clamped(src, fi + o2) + sample_clamped(src, fi - o2)) * w2;
        dst[i] = c.round().clamp(0.0, 65535.0) as u16;
    }
}

/// 预乘 RGBA8 图像按 `factor` 倍降采样，每个低分辨率像素取对应方块的面积平均。
/// 返回 16 位（`值 × 257`）的交错 RGBA 和新尺寸。
pub fn downsample_rgba(src: &[u8], width: usize, height: usize, factor: usize) -> (Vec<u16>, usize, usize) {
    let f = factor.max(1);
    let lw = width.div_ceil(f).max(1);
    let lh = height.div_ceil(f).max(1);
    let mut out = vec![0u16; lw * lh * 4];
    for ly in 0..lh {
        let y0 = ly * f;
        let y1 = ((ly + 1) * f).min(height);
        for lx in 0..lw {
            let x0 = lx * f;
            let x1 = ((lx + 1) * f).min(width);
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                let row = y * width;
                for x in x0..x1 {
                    let i = (row + x) * 4;
                    for c in 0..4 {
                        sum[c] += src[i + c] as u32;
                    }
                }
            }
            let count = ((y1 - y0) * (x1 - x0)).max(1) as u32;
            let o = (ly * lw + lx) * 4;
            for c in 0..4 {
                out[o + c] = ((sum[c] * 257 + count / 2) / count).min(65535) as u16;
            }
        }
    }
    (out, lw, lh)
}

/// 调整饱和度（语义同 CSS `saturate()`，亮度系数取 Rec. 709）。输入输出都是 16 位预乘 RGBA。
pub(crate) fn saturate(data: &mut [u16], amount: f32) {
    if (amount - 1.0).abs() < 1e-3 {
        return;
    }
    for px in data.chunks_exact_mut(4) {
        let (r, g, b, a) = (px[0] as f32, px[1] as f32, px[2] as f32, px[3] as f32);
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let f = |c: f32| (luma + (c - luma) * amount).clamp(0.0, a).round() as u16;
        px[0] = f(r);
        px[1] = f(g);
        px[2] = f(b);
    }
}

/// 调整亮度（每个颜色通道乘以 `factor`，对应 CairnPhi 的背景亮度衰减系数，alpha 保持不变）。
pub(crate) fn adjust_brightness(data: &mut [u16], factor: f32) {
    if (factor - 1.0).abs() < 1e-3 {
        return;
    }
    let factor = factor.max(0.0);
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as f32;
        px[0] = (px[0] as f32 * factor).clamp(0.0, a).round() as u16;
        px[1] = (px[1] as f32 * factor).clamp(0.0, a).round() as u16;
        px[2] = (px[2] as f32 * factor).clamp(0.0, a).round() as u16;
    }
}

/// 确定性的 256×256 灰度噪点（逐像素整数哈希，均匀分布）。
pub(crate) fn noise_tile() -> Vec<u8> {
    (0..256u32 * 256)
        .map(|i| {
            // Chris Wellons 的 lowbias32。
            let mut x = i ^ 0x5bd1_e995;
            x ^= x >> 16;
            x = x.wrapping_mul(0x7feb_352d);
            x ^= x >> 15;
            x = x.wrapping_mul(0x846c_a68b);
            x ^= x >> 16;
            (x >> 24) as u8
        })
        .collect()
}

pub(crate) const NOISE_SIZE: usize = 256;

/// 模糊后的圆角矩形覆盖率蒙版。形状位于 `(pad, pad)` 处，尺寸 `shape_w × shape_h`。
pub(crate) struct ShadowMask {
    pub width: usize,
    pub height: usize,
    pub pad: usize,
    pub data: Vec<u8>,
}

impl ShadowMask {
    fn new(shape_w: usize, shape_h: usize, radius: f32, sigma: f32) -> ShadowMask {
        let pad = (3.0 * sigma).ceil() as usize + 1;
        let (w, h) = (shape_w + 2 * pad, shape_h + 2 * pad);
        let (l, t) = (pad as f32, pad as f32);
        let (r, b) = (l + shape_w as f32, t + shape_h as f32);
        let mut buf = vec![0u16; w * h];
        for y in pad.saturating_sub(1)..(pad + shape_h + 1).min(h) {
            for x in pad.saturating_sub(1)..(pad + shape_w + 1).min(w) {
                let c = rrect_coverage(x as f32 + 0.5, y as f32 + 0.5, l, t, r, b, radius);
                buf[y * w + x] = (c * 65535.0 + 0.5) as u16;
            }
        }
        gaussian_blur(&mut buf, w, h, 1, sigma);
        ShadowMask {
            width: w,
            height: h,
            pad,
            data: buf.iter().map(|&v| ((v as u32 + 128) >> 8).min(255) as u8).collect(),
        }
    }

    fn bytes(&self) -> usize {
        self.data.len()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ShadowKey {
    w: usize,
    h: usize,
    /// 半径和标准差按 1/4 物理像素量化。
    radius: u32,
    sigma: u32,
}

/// 阴影蒙版缓存。键是物理尺寸（宽、高、圆角、模糊），所以缩放因子也隐含在里面。
/// 总字节数超过上限时淘汰最久没用过的。
pub(crate) struct ShadowCache {
    map: HashMap<ShadowKey, (Rc<ShadowMask>, u64)>,
    bytes: usize,
    tick: u64,
    limit: usize,
}

/// 阴影蒙版缓存的总上限。
const SHADOW_CACHE_LIMIT: usize = 16 << 20;

impl Default for ShadowCache {
    fn default() -> Self {
        ShadowCache {
            map: HashMap::new(),
            bytes: 0,
            tick: 0,
            limit: SHADOW_CACHE_LIMIT,
        }
    }
}

impl ShadowCache {
    pub fn get(&mut self, shape_w: usize, shape_h: usize, radius: f32, sigma: f32) -> Rc<ShadowMask> {
        self.tick += 1;
        let key = ShadowKey {
            w: shape_w,
            h: shape_h,
            radius: (radius * 4.0).round().max(0.0) as u32,
            sigma: (sigma * 4.0).round().max(0.0) as u32,
        };
        if let Some((mask, used)) = self.map.get_mut(&key) {
            *used = self.tick;
            return mask.clone();
        }
        let mask = Rc::new(ShadowMask::new(
            shape_w,
            shape_h,
            key.radius as f32 / 4.0,
            key.sigma as f32 / 4.0,
        ));
        self.bytes += mask.bytes();
        self.map.insert(key, (mask.clone(), self.tick));
        while self.bytes > self.limit && self.map.len() > 1 {
            let Some((&oldest, _)) = self
                .map
                .iter()
                .filter(|(k, _)| **k != key)
                .min_by_key(|(_, (_, used))| *used)
            else {
                break;
            };
            if let Some((m, _)) = self.map.remove(&oldest) {
                self.bytes -= m.bytes();
            }
        }
        mask
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }

    #[cfg(test)]
    fn with_limit(limit: usize) -> Self {
        ShadowCache {
            limit,
            ..ShadowCache::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_is_exact_on_axis_aligned_edges() {
        // 右边在 10.3：像素 [10, 11) 覆盖 0.3，像素 [9, 10) 完全覆盖。
        assert!((rrect_coverage(10.5, 5.5, 0.0, 0.0, 10.3, 20.0, 0.0) - 0.3).abs() < 1e-4);
        assert_eq!(rrect_coverage(9.5, 5.5, 0.0, 0.0, 10.3, 20.0, 0.0), 1.0);
        // 圆角外面的角点没有覆盖。
        assert_eq!(rrect_coverage(0.5, 0.5, 0.0, 0.0, 40.0, 40.0, 10.0), 0.0);
        assert_eq!(rrect_coverage(20.5, 20.5, 0.0, 0.0, 40.0, 40.0, 10.0), 1.0);
    }

    #[test]
    fn box_blur_matches_gaussian_variance() {
        // 对单个亮点模糊，结果的方差应接近 sigma²，总量守恒。
        for sigma in [2.0f32, 5.0, 10.0] {
            let n = 201;
            let mut line = vec![0u16; n];
            line[n / 2] = 60000;
            let mut data = line.clone();
            // 一维：用 1×n 的图像只做横向（纵向高度 1，不改变）。
            gaussian_blur(&mut data, n, 1, 1, sigma);
            let total: f64 = data.iter().map(|&v| v as f64).sum();
            let mean: f64 = data.iter().enumerate().map(|(i, &v)| i as f64 * v as f64).sum::<f64>() / total;
            let var: f64 = data
                .iter()
                .enumerate()
                .map(|(i, &v)| (i as f64 - mean).powi(2) * v as f64)
                .sum::<f64>()
                / total;
            assert!((total - 60000.0).abs() / 60000.0 < 0.01, "sigma {sigma}: total {total}");
            assert!(
                (var.sqrt() - sigma as f64).abs() < 0.25,
                "sigma {sigma}: got {}",
                var.sqrt()
            );
            line.clear();
        }
    }

    #[test]
    fn downsample_and_saturate() {
        // 2×2 方块平均。
        let src = [
            0, 0, 0, 255, 255, 255, 255, 255, //
            255, 0, 0, 255, 0, 0, 255, 255,
        ];
        let (out, w, h) = downsample_rgba(&src, 2, 2, 2);
        assert_eq!((w, h), (1, 1));
        let px: Vec<u8> = out.iter().map(|&v| (v / 257) as u8).collect();
        assert_eq!(px, vec![127, 63, 127, 255]);

        // 灰色不受饱和度影响，彩色离灰度更远。
        let mut gray = vec![30000u16, 30000, 30000, 65535];
        saturate(&mut gray, 1.8);
        assert_eq!(gray, vec![30000, 30000, 30000, 65535]);
        let mut blue = vec![20000u16, 25000, 40000, 65535];
        saturate(&mut blue, 1.8);
        assert!(blue[2] > 40000 && blue[0] < 20000);
    }

    #[test]
    fn noise_is_deterministic_and_balanced() {
        let a = noise_tile();
        assert_eq!(a, noise_tile());
        assert_eq!(a.len(), NOISE_SIZE * NOISE_SIZE);
        let mean = a.iter().map(|&v| v as f64).sum::<f64>() / a.len() as f64;
        assert!((mean - 127.5).abs() < 2.0, "mean {mean}");
    }

    #[test]
    fn shadow_cache_reuses_and_evicts() {
        let mut cache = ShadowCache::with_limit(200_000);
        let a = cache.get(100, 50, 10.0, 8.0);
        let b = cache.get(100, 50, 10.0, 8.0);
        assert!(Rc::ptr_eq(&a, &b), "同样的参数命中缓存");
        // 形状中心完全不透明，远离形状处为 0，左右对称。
        let mid = a.pad + 25;
        let row = &a.data[mid * a.width..(mid + 1) * a.width];
        assert_eq!(row[a.pad + 50], 255);
        assert_eq!(row[0], 0);
        assert_eq!(row[a.pad + 2], row[a.width - a.pad - 3]);
        // 塞满后最久未用的被淘汰。
        for i in 0..20 {
            cache.get(120 + i, 60, 10.0, 8.0);
        }
        assert!(cache.bytes <= 200_000 || cache.map.len() == 1);
        let c = cache.get(100, 50, 10.0, 8.0);
        assert!(!Rc::ptr_eq(&a, &c), "被淘汰后重新生成");
    }

    #[test]
    fn cairn_phi_blur_conserves_energy_and_is_symmetric() {
        let n = 101;
        let mut data = vec![0u16; n * n * 4];
        let mid = n / 2;
        data[(mid * n + mid) * 4] = 60000;
        data[(mid * n + mid) * 4 + 1] = 40000;
        data[(mid * n + mid) * 4 + 2] = 20000;
        data[(mid * n + mid) * 4 + 3] = 65535;

        cairn_phi_gaussian_blur(&mut data, n, n, 4, 2, 1.0);

        // 能量守恒：总和在边界效应外保持一致
        let total_r: u64 = (0..n * n).map(|i| data[i * 4] as u64).sum();
        assert!((total_r as i64 - 60000).abs() < 100, "total_r: {total_r}");

        // 对称性：中心点周围上下左右对称
        for d in 1..10 {
            let left = data[(mid * n + (mid - d)) * 4];
            let right = data[(mid * n + (mid + d)) * 4];
            let top = data[((mid - d) * n + mid) * 4];
            let bottom = data[((mid + d) * n + mid) * 4];
            assert_eq!(left, right, "左右对称 d={d}");
            assert_eq!(top, bottom, "上下对称 d={d}");
            assert!((left as i32 - top as i32).abs() <= 1, "各向同性 d={d}");
        }

        // 亮度调节
        adjust_brightness(&mut data, 0.95);
        let adjusted_r: u64 = (0..n * n).map(|i| data[i * 4] as u64).sum();
        let expected = (60000.0 * 0.95) as i64;
        assert!((adjusted_r as i64 - expected).abs() < 100);
    }
}
