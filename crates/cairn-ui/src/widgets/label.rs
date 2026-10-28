//! 文本标签。

use cairn_ui_layout::{AvailableSpace, MeasureInput};

use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::{Color, FontFamily, Painter, Rect, Size, Style, TextLayout, TextStyle};

/// 一段文字。宽度不够时自动换行。用 [`label`] 创建。
pub struct Label {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    text: String,
    font_size: Option<f32>,
    weight: u16,
    color: Option<Color>,
    layout: Option<TextLayout>,
    /// 不换行时的宽度。
    natural_width: f32,
}

pub fn label(text: impl Into<String>) -> Label {
    Label {
        style: Style::default(),
        key: None,
        text: text.into(),
        font_size: None,
        weight: 400,
        color: None,
        layout: None,
        natural_width: 0.0,
    }
}

element!(Label);

impl Label {
    /// 字号（逻辑像素），默认取主题字号。
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = Some(size);
        self
    }

    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    pub fn bold(self) -> Self {
        self.weight(700)
    }

    /// 文字颜色，默认取主题文字色。
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    fn text_style(&self, theme: &Theme) -> TextStyle {
        TextStyle {
            font_size: self.font_size.unwrap_or(theme.font_size),
            line_height: theme.line_height,
            weight: self.weight,
            family: FontFamily::SansSerif,
        }
    }

    /// 让排版结果与当前文字、样式一致。
    fn sync(&mut self, cx: &mut LayoutCx) -> &mut TextLayout {
        let style = self.text_style(cx.theme());
        let changed = match &mut self.layout {
            Some(l) => {
                let a = l.set_style(cx.text(), &style);
                let b = l.set_text(cx.text(), &self.text);
                a || b
            }
            None => {
                self.layout = Some(TextLayout::new(cx.text(), &self.text, &style));
                true
            }
        };
        let layout = self.layout.as_mut().expect("刚刚创建");
        if changed {
            layout.set_wrap_width(cx.text(), None);
            self.natural_width = layout.size().width;
        }
        layout
    }

    /// 宽度不够放下整行时才换行；留一点余量，避免浮点误差导致恰好放得下的文字被折行。
    fn wrap_for(&self, width: f32) -> Option<f32> {
        (width + 0.5 < self.natural_width).then_some(width)
    }
}

impl<S: 'static> Widget<S> for Label {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.text != self.text || new.font_size != self.font_size || new.weight != self.weight {
            change = Change::LAYOUT;
        }
        if new.color != self.color {
            change |= Change::PAINT;
        }
        self.text = new.text;
        self.font_size = new.font_size;
        self.weight = new.weight;
        self.color = new.color;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        self.sync(cx);
        let wrap = match input.available_width {
            AvailableSpace::Definite(w) => self.wrap_for(w),
            AvailableSpace::MinContent => Some(0.0),
            AvailableSpace::MaxContent => None,
        };
        let layout = self.layout.as_mut().expect("sync 之后一定存在");
        layout.set_wrap_width(cx.text(), wrap);
        layout.size()
    }

    fn arrange(&mut self, cx: &mut LayoutCx, _bounds: Rect, content: Rect) {
        let wrap = self.wrap_for(content.width);
        if let Some(layout) = &mut self.layout {
            layout.set_wrap_width(cx.text(), wrap);
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        if let Some(layout) = &self.layout {
            p.draw_text(layout, cx.content().origin(), self.color.unwrap_or(cx.theme().text));
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Label,
            label: Some(self.text.clone()),
            ..Semantics::default()
        }
    }
}
