use std::fmt;

#[derive(Clone)]
pub struct Image {
    pub(crate) pixmap: tiny_skia::Pixmap,
    pub(crate) glass: Option<tiny_skia::Pixmap>,
}

impl Image {
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
            glass: None,
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
