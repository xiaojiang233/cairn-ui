//! 文字：字体库、排版（cosmic-text），以及光标 / 选区几何查询。
//!
//! 排版在**逻辑像素**下进行（cosmic-text 默认不做度量取整，字形位置是小数），
//! 只在光栅化时按缩放因子换算到物理像素。所以缩放因子变化时不需要重新排版。

use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cosmic_text::{
    Attrs, Buffer, CacheKeyFlags, Cursor, Family, FontSystem, LineIter, Metrics, Shaping, SwashCache, Weight, Wrap, fontdb,
};

use crate::geom::{Point, Rect, Size};

/// 字体加载配置。通常由平台层提供（字体目录、默认字体随系统和语言而定）。
#[derive(Clone, Debug, Default)]
pub struct FontSetup {
    /// 递归扫描这些目录里的 ttf / otf / ttc / otc。
    pub dirs: Vec<PathBuf>,
    /// 各通用字族的候选字体名，按优先级排列，取第一个找得到的。
    pub sans_serif: Vec<String>,
    pub serif: Vec<String>,
    pub monospace: Vec<String>,
    /// BCP 47 语言标签，如 `zh-CN`，影响汉字回退时选简体 / 繁体 / 日文字形。
    pub locale: String,
}

/// 字体库与字形缓存。整个应用共用一个。
pub struct TextSystem {
    pub(crate) fonts: FontSystem,
    pub(crate) glyphs: SwashCache,
    load_time: Duration,
    sans_serif: Option<String>,
}

impl TextSystem {
    pub fn new(setup: &FontSetup) -> Self {
        let start = Instant::now();
        let mut db = fontdb::Database::new();
        for dir in &setup.dirs {
            db.load_fonts_dir(dir);
        }
        let pick = |db: &fontdb::Database, names: &[String]| {
            names
                .iter()
                .find(|name| {
                    db.faces()
                        .any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)))
                })
                .cloned()
        };
        let sans_serif = pick(&db, &setup.sans_serif);
        if let Some(name) = &sans_serif {
            db.set_sans_serif_family(name.clone());
        }
        if let Some(name) = pick(&db, &setup.serif) {
            db.set_serif_family(name);
        }
        if let Some(name) = pick(&db, &setup.monospace) {
            db.set_monospace_family(name);
        }
        let locale = if setup.locale.is_empty() {
            "en-US".to_string()
        } else {
            setup.locale.clone()
        };
        let fonts = FontSystem::new_with_locale_and_db(locale, db);
        TextSystem {
            fonts,
            glyphs: SwashCache::new(),
            load_time: start.elapsed(),
            sans_serif,
        }
    }

    /// 已加载的字体面数量。
    pub fn face_count(&self) -> usize {
        self.fonts.db().len()
    }

    /// 加载字体花的时间（扫描目录 + 解析字体表头）。
    pub fn load_time(&self) -> Duration {
        self.load_time
    }

    /// 实际选中的无衬线默认字体。`None` 表示候选字体一个都没找到。
    pub fn sans_serif_family(&self) -> Option<&str> {
        self.sans_serif.as_deref()
    }

    /// 清空光栅化字形缓存。缩放因子变化后旧尺寸的字形不再需要，可以调用这个释放内存。
    pub fn clear_glyph_cache(&mut self) {
        self.glyphs.image_cache.clear();
        self.glyphs.outline_command_cache.clear();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FontFamily {
    SansSerif,
    Serif,
    Monospace,
    Named(String),
}

/// 文字样式。尺寸是逻辑像素。
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font_size: f32,
    /// 行高相对字号的倍数。
    pub line_height: f32,
    /// 字重，400 为常规，700 为粗体。
    pub weight: u16,
    pub family: FontFamily,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            font_size: 14.0,
            line_height: 1.4,
            weight: 400,
            family: FontFamily::SansSerif,
        }
    }
}

impl TextStyle {
    fn metrics(&self) -> Metrics {
        Metrics::new(self.font_size, (self.font_size * self.line_height).round().max(1.0))
    }

    fn attrs(&self) -> Attrs<'_> {
        let family = match &self.family {
            FontFamily::SansSerif => Family::SansSerif,
            FontFamily::Serif => Family::Serif,
            FontFamily::Monospace => Family::Monospace,
            FontFamily::Named(name) => Family::Name(name),
        };
        // 关键：Windows 上微软雅黑（Microsoft YaHei）等中文字体只有 400 (Regular) 和 700 (Bold) 两个物理字重。
        // 若向系统字体库请求 500 或 600，fontdb 的精确匹配规则会优先拾取系统中恰好带有 500/600 字重的日文字体
        // （如 YuGothicUI-Semibold、BIZ-UDMincho-Medium），造成同一个词汇里“一字日文游黑/明朝、一字中文等线”的交叉混排，
        // 出现严重的字形形变畸变与基线交错不齐（文字上蹿下跳）。
        // 遵循 W3C CSS 字体匹配规范，将字重归一化到存在的真实字重（< 600 对应 400 常规体，>= 600 对应 700 粗体）。
        let weight = if self.weight >= 600 { 700 } else { 400 };
        Attrs::new()
            .family(family)
            .weight(Weight(weight))
            // 灰度抗锯齿下禁用字节码 Hinting，避免微软 TrueType Hinting 在小字号下强制像素网格贴合导致汉字笔画丢失、粘连或严重畸变（如“截”、“启”）。
            .cache_key_flags(CacheKeyFlags::DISABLE_HINTING)
    }
}

/// 一段排好版的文字。可以反复修改内容，内部会复用缓冲区。
pub struct TextLayout {
    pub(crate) buffer: Buffer,
    text: String,
    style: TextStyle,
    wrap_width: Option<f32>,
    /// 每个逻辑行在 `text` 里的起始字节位置（与 cosmic-text 的分行规则一致）。
    line_starts: Vec<usize>,
    size: Size,
}

impl TextLayout {
    pub fn new(ts: &mut TextSystem, text: &str, style: &TextStyle) -> Self {
        let mut buffer = Buffer::new(&mut ts.fonts, style.metrics());
        buffer.set_wrap(Wrap::None);
        buffer.set_size(None, None);
        let mut layout = TextLayout {
            buffer,
            text: String::new(),
            style: style.clone(),
            wrap_width: None,
            line_starts: vec![0],
            size: Size::ZERO,
        };
        layout.reshape(ts, text);
        layout
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn style(&self) -> &TextStyle {
        &self.style
    }

    /// 修改文字内容。内容没变时什么也不做。返回是否真的变了。
    pub fn set_text(&mut self, ts: &mut TextSystem, text: &str) -> bool {
        if text == self.text {
            return false;
        }
        self.reshape(ts, text);
        true
    }

    /// 修改样式。样式没变时什么也不做。返回是否真的变了。
    pub fn set_style(&mut self, ts: &mut TextSystem, style: &TextStyle) -> bool {
        if *style == self.style {
            return false;
        }
        self.style = style.clone();
        self.buffer.set_metrics(self.style.metrics());
        let text = std::mem::take(&mut self.text);
        self.reshape(ts, &text);
        true
    }

    /// 设置自动换行宽度，`None` 表示不换行。
    pub fn set_wrap_width(&mut self, ts: &mut TextSystem, width: Option<f32>) {
        let width = width.map(|w| w.max(0.0));
        if width == self.wrap_width {
            return;
        }
        self.wrap_width = width;
        self.buffer
            .set_wrap(if width.is_some() { Wrap::WordOrGlyph } else { Wrap::None });
        self.buffer.set_size(width, None);
        self.relayout(ts);
    }

    fn reshape(&mut self, ts: &mut TextSystem, text: &str) {
        self.text.clear();
        self.text.push_str(text);
        self.buffer.set_text(text, &self.style.attrs(), Shaping::Advanced, None);
        self.line_starts.clear();
        let mut last_had_ending = true;
        for (range, ending) in LineIter::new(text) {
            self.line_starts.push(range.start);
            last_had_ending = ending != cosmic_text::LineEnding::None;
        }
        if last_had_ending {
            // 与 cosmic-text 一致：末尾有换行符（或文本为空）时补一个空行。
            self.line_starts.push(text.len());
        }
        self.relayout(ts);
    }

    fn relayout(&mut self, ts: &mut TextSystem) {
        self.buffer.shape_until_scroll(&mut ts.fonts, false);
        let mut w: f32 = 0.0;
        let mut h: f32 = 0.0;
        for run in self.buffer.layout_runs() {
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
        }
        self.size = Size::new(w, h.max(self.line_height()));
    }

    /// 排版后的尺寸（逻辑像素）。
    pub fn size(&self) -> Size {
        self.size
    }

    pub fn line_height(&self) -> f32 {
        self.buffer.metrics().line_height
    }

    /// 第一行基线到文字框顶部的距离。
    pub fn first_baseline(&self) -> f32 {
        self.buffer
            .layout_runs()
            .next()
            .map(|r| r.line_y)
            .unwrap_or(self.line_height() * 0.8)
    }

    fn index_to_cursor(&self, index: usize) -> Cursor {
        let index = index.min(self.text.len());
        let line = self.line_starts.partition_point(|&s| s <= index).saturating_sub(1);
        Cursor::new(line, index - self.line_starts[line])
    }

    fn cursor_to_index(&self, c: Cursor) -> usize {
        let start = self.line_starts.get(c.line).copied().unwrap_or(self.text.len());
        (start + c.index).min(self.text.len())
    }

    /// 点击位置（相对文字框左上角）对应的字节位置，总是落在字素簇边界上。
    pub fn hit(&self, p: Point) -> usize {
        match self.buffer.hit(p.x, p.y) {
            Some(c) => self.cursor_to_index(c),
            None if p.y < 0.0 => 0,
            None => self.text.len(),
        }
    }

    /// 字节位置 `index` 处光标的左上角（相对文字框）；光标高度为 [`Self::line_height`]。
    pub fn cursor_position(&self, index: usize) -> Point {
        let cursor = self.index_to_cursor(index);
        let mut fallback = None;
        for run in self.buffer.layout_runs().filter(|r| r.line_i == cursor.line) {
            if let Some(x) = run.cursor_position(&cursor) {
                return Point::new(x, run.line_top);
            }
            fallback = Some(Point::new(run.line_w, run.line_top));
        }
        fallback.unwrap_or(Point::ZERO)
    }

    /// 字节区间对应的高亮矩形（相对文字框），可能有多个（换行或双向文字）。
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let mut rects = Vec::new();
        if range.start >= range.end {
            return rects;
        }
        let (start, end) = (self.index_to_cursor(range.start), self.index_to_cursor(range.end));
        for run in self.buffer.layout_runs() {
            if run.line_i < start.line || run.line_i > end.line {
                continue;
            }
            for (x, w) in run.highlight(start, end) {
                rects.push(Rect::new(x, run.line_top, w, run.line_height));
            }
        }
        rects
    }
}

impl std::fmt::Debug for TextLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextLayout")
            .field("text", &self.text)
            .field("size", &self.size)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attrs_have_disable_hinting_flag_and_quantized_weight() {
        let regular = TextStyle { weight: 500, ..TextStyle::default() };
        assert_eq!(regular.attrs().weight, Weight(400));
        assert!(regular.attrs().cache_key_flags.contains(CacheKeyFlags::DISABLE_HINTING));

        let bold = TextStyle { weight: 600, ..TextStyle::default() };
        assert_eq!(bold.attrs().weight, Weight(700));
        assert!(bold.attrs().cache_key_flags.contains(CacheKeyFlags::DISABLE_HINTING));
    }
}
