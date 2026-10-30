use std::num::NonZeroU32;
use std::rc::Rc;

use winit::window::Window;

use crate::{Error, ErrorKind, PixelRect};

pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
    pub damage: &'a [PixelRect],
    pub glass: &'a [u8],
    pub scene: Option<SceneImage<'a>>,
    pub background: [u8; 4],
    pub blur: Blur,
    pub noise: f32,
}

pub struct SceneImage<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blur {
    pub downsample: u32,
    pub passes: u32,
    pub spread: f32,
    pub saturation: f32,
    pub brightness: f32,
}

pub struct Surface(Backend);

enum Backend {
    #[cfg(windows)]
    Gl(Box<crate::gl::Gl>),
    Soft(Soft),
}

impl Surface {
    #[cfg(windows)]
    pub(crate) fn gl(gl: crate::gl::Gl) -> Self {
        Surface(Backend::Gl(Box::new(gl)))
    }

    pub(crate) fn soft(context: softbuffer::Context<Rc<Window>>, window: Rc<Window>) -> Result<Self, Error> {
        let inner = softbuffer::Surface::new(&context, window)?;
        Ok(Surface(Backend::Soft(Soft {
            inner,
            _context: context,
            size: (0, 0),
            rects: Vec::new(),
        })))
    }

    pub fn composites(&self) -> bool {
        match &self.0 {
            #[cfg(windows)]
            Backend::Gl(_) => true,
            Backend::Soft(_) => false,
        }
    }

    pub fn present(&mut self, frame: &Frame) -> Result<(), Error> {
        match &mut self.0 {
            #[cfg(windows)]
            Backend::Gl(gl) => gl.present(frame).map_err(|e| Error(ErrorKind::Gl(e))),
            Backend::Soft(s) => s.present(frame.rgba, frame.width, frame.height, frame.damage),
        }
    }
}

struct Soft {
    inner: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    _context: softbuffer::Context<Rc<Window>>,
    size: (u32, u32),
    rects: Vec<softbuffer::Rect>,
}

impl Soft {
    fn present(&mut self, rgba: &[u8], width: u32, height: u32, damage: &[PixelRect]) -> Result<(), Error> {
        let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return Ok(());
        };
        assert_eq!(rgba.len(), width as usize * height as usize * 4, "画布尺寸与窗口不一致");
        if self.size != (width, height) {
            self.inner.resize(w, h)?;
            self.size = (width, height);
        }
        let mut buffer = self.inner.buffer_mut()?;

        self.rects.clear();
        if buffer.age() == 1 && !damage.is_empty() {
            for r in damage {
                let r = r.clamp(width, height);
                if let (Some(rw), Some(rh)) = (NonZeroU32::new(r.width), NonZeroU32::new(r.height)) {
                    self.rects.push(softbuffer::Rect {
                        x: r.x,
                        y: r.y,
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
                for (d, s) in dst.iter_mut().zip(src.chunks_exact(4)) {
                    *d = (s[0] as u32) << 16 | (s[1] as u32) << 8 | s[2] as u32;
                }
            }
        }
        buffer.present_with_damage(&self.rects)?;
        Ok(())
    }
}
