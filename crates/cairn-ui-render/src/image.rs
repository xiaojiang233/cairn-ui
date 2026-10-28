//! 位图。本 crate 不做图片解码（不引入解码库），调用方传入已解码的 RGBA 像素。

use std::fmt;

/// 一张位图，内部是预乘 alpha 的 RGBA8。
#[derive(Clone)]
pub struct Image {
    pub(crate) pixmap: tiny_skia::Pixmap,
}

impl Image {
    /// 从非预乘的 RGBA8 像素创建（行优先、紧密排列，长度必须是 `width * height * 4`）。
    pub fn from_rgba8(width: u32, height: u32, mut rgba: Vec<u8>) -> Option<Image> {
        if rgba.len() != width as usize * height as usize * 4 {
            return None;
        }
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a != 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * a + 127) / 255) as u8;
                }
            }
        }
        let size = tiny_skia::IntSize::from_wh(width, height)?;
        Some(Image {
            pixmap: tiny_skia::Pixmap::from_vec(rgba, size)?,
        })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Image({}x{})", self.width(), self.height())
    }
}
