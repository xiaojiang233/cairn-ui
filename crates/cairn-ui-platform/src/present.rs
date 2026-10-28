//! 软件上屏：把上层画好的 RGBA 画布拷进 softbuffer 的缓冲区并提交。

use std::num::NonZeroU32;
use std::rc::Rc;

use winit::window::Window;

use crate::{Error, PixelRect};

/// 窗口的上屏表面。
pub struct Surface {
    inner: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    size: (u32, u32),
    rects: Vec<softbuffer::Rect>,
}

impl Surface {
    pub(crate) fn new(inner: softbuffer::Surface<Rc<Window>, Rc<Window>>) -> Self {
        Surface {
            inner,
            size: (0, 0),
            rects: Vec::new(),
        }
    }

    /// 提交一帧。
    ///
    /// - `rgba`：`width * height` 个像素、行优先、紧密排列的 RGBA8（预乘或不预乘都行，
    ///   alpha 通道被忽略——窗口是不透明的）。尺寸必须与当前客户区一致。
    /// - `damage`：本帧改动过的区域。缓冲区内容仍有效时只拷贝并提交这些区域；
    ///   缓冲区是新的（刚创建或尺寸变了），或者 `damage` 为空（例如系统要求重绘），则整帧提交。
    pub fn present(&mut self, rgba: &[u8], width: u32, height: u32, damage: &[PixelRect]) -> Result<(), Error> {
        let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return Ok(()); // 最小化时尺寸为 0，什么也不做。
        };
        assert_eq!(rgba.len(), width as usize * height as usize * 4, "画布尺寸与窗口不一致");
        if self.size != (width, height) {
            self.inner.resize(w, h)?;
            self.size = (width, height);
        }
        let mut buffer = self.inner.buffer_mut()?;

        self.rects.clear();
        // age == 1：缓冲区里还是上一帧的内容，只需更新改动区域。
        if buffer.age() == 1 && !damage.is_empty() {
            for r in damage {
                let x0 = r.x.min(width);
                let y0 = r.y.min(height);
                let x1 = r.x.saturating_add(r.width).min(width);
                let y1 = r.y.saturating_add(r.height).min(height);
                if let (Some(rw), Some(rh)) = (NonZeroU32::new(x1 - x0), NonZeroU32::new(y1 - y0)) {
                    self.rects.push(softbuffer::Rect {
                        x: x0,
                        y: y0,
                        width: rw,
                        height: rh,
                    });
                }
            }
        }
        if self.rects.is_empty() {
            self.rects.push(softbuffer::Rect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            });
        }

        let stride = width as usize;
        for r in &self.rects {
            let (x0, x1) = (r.x as usize, (r.x + r.width.get()) as usize);
            for y in r.y as usize..(r.y + r.height.get()) as usize {
                let row = y * stride;
                let src = &rgba[(row + x0) * 4..(row + x1) * 4];
                let dst = &mut buffer[row + x0..row + x1];
                // softbuffer 的像素格式是 0x00RRGGBB。
                for (d, s) in dst.iter_mut().zip(src.chunks_exact(4)) {
                    *d = (s[0] as u32) << 16 | (s[1] as u32) << 8 | s[2] as u32;
                }
            }
        }
        buffer.present_with_damage(&self.rects)?;
        Ok(())
    }
}
