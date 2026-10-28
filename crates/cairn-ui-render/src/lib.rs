//! cairn-ui 的渲染层。
//!
//! - [`Painter`]：控件用的绘图接口，与后端无关。
//! - [`SkiaPainter`] / [`Canvas`]：基于 tiny-skia 的 CPU 实现，支持只重画脏区。
//! - [`TextSystem`] / [`TextLayout`]：字体加载、排版、字形光栅化缓存（cosmic-text + swash）。
//!
//! 坐标约定：对外接口一律用逻辑像素；只有 [`DeviceRect`] 和 [`Canvas`] 用物理像素。

mod geom;
mod painter;
mod skia;
mod text;

pub use geom::{Color, DeviceRect, Point, Rect, Size};
pub use painter::Painter;
pub use skia::{Canvas, SkiaPainter};
pub use text::{FontFamily, FontSetup, TextLayout, TextStyle, TextSystem};
