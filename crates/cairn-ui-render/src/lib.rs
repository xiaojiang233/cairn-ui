//! cairn-ui 的渲染层。
//!
//! - [`Painter`]：控件用的绘图接口，与后端无关。填充（纯色 / 渐变 / 背景模糊）、细线描边、
//!   外阴影与内阴影、文字、图片、圆角裁剪、平移缩放变换。
//! - [`SkiaPainter`] / [`Canvas`]：基于 tiny-skia 的 CPU 实现，支持只重画脏区；
//!   窗口背景（纯色或 [`Scene`]）、背景模糊、阴影蒙版、全局噪点都在这里缓存。
//! - [`TextSystem`] / [`TextLayout`]：字体加载、排版、字形光栅化缓存（cosmic-text + swash）。
//!
//! 坐标约定：对外接口一律用逻辑像素；只有 [`DeviceRect`] 和 [`Canvas`] 用物理像素。

mod effects;
mod geom;
mod image;
mod painter;
mod path;
mod scene;
mod skia;
mod text;

pub use geom::{Affine, Color, DeviceRect, Point, Rect, Size};
pub use image::Image;
pub use painter::{Brush, FillRule, GradientStop, LineCap, LineJoin, Painter, Shadow, StrokeStyle};
pub use path::{Path, PathParseError, Seg};
pub use scene::{ImageScene, NightScene, Scene, Spot};
pub use skia::{BackdropStyle, Background, Canvas, SkiaPainter};
pub use text::{FontFamily, FontSetup, TextLayout, TextStyle, TextSystem};
