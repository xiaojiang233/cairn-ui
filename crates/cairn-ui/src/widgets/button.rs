//! 按钮。

use cairn_ui_layout::MeasureInput;

use crate::element::element;
use crate::theme::BUTTON_PADDING;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::{CursorIcon, FontFamily, Key, NamedKey, Painter, Point, PointerButton, Size, Style, TextLayout, TextStyle};

type ClickHandler<S> = Box<dyn Fn(&mut S)>;

/// 文字按钮。用 [`button`] 创建，[`Button::on_click`] 设置点击回调。
///
/// 鼠标在按钮上按下、并在按钮上松开才算一次点击；有键盘焦点时 Enter / 空格也会触发。
pub struct Button<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    label: String,
    on_click: Option<ClickHandler<S>>,
    enabled: bool,
    pressed: bool,
    text: Option<TextLayout>,
}

pub fn button<S: 'static>(label: impl Into<String>) -> Button<S> {
    Button {
        style: Style {
            padding: BUTTON_PADDING,
            ..Style::default()
        },
        key: None,
        label: label.into(),
        on_click: None,
        enabled: true,
        pressed: false,
        text: None,
    }
}

element!(Button<S>);

impl<S: 'static> Button<S> {
    /// 点击时调用，参数是应用状态。
    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// 禁用后不响应点击、不能获得焦点，并显示为灰色。
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn click(&self, cx: &mut EventCx<S>) {
        if let Some(f) = &self.on_click {
            f(cx.app());
        }
    }
}

impl<S: 'static> Widget<S> for Button<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.label != self.label {
            change = Change::LAYOUT;
        }
        if new.enabled != self.enabled {
            change |= Change::PAINT;
            self.pressed = false;
        }
        self.label = new.label;
        self.enabled = new.enabled;
        self.on_click = new.on_click;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let style = TextStyle {
            font_size: cx.theme().font_size,
            line_height: cx.theme().line_height,
            weight: 400,
            family: FontFamily::SansSerif,
        };
        match &mut self.text {
            Some(l) => {
                l.set_style(cx.text(), &style);
                l.set_text(cx.text(), &self.label);
            }
            None => self.text = Some(TextLayout::new(cx.text(), &self.label, &style)),
        }
        self.text.as_ref().map(|l| l.size()).unwrap_or_default()
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let t = cx.theme();
        let (bg, fg) = if !self.enabled {
            (t.disabled_background, t.disabled_text)
        } else if self.pressed && cx.is_hovered() {
            (t.accent_pressed, t.on_accent)
        } else if cx.is_hovered() {
            (t.accent_hover, t.on_accent)
        } else {
            (t.accent, t.on_accent)
        };
        let b = cx.bounds();
        if cx.focus_visible() {
            // 键盘焦点框：外扩 3 像素、2 像素宽的描边，和按钮之间留 1 像素空隙。
            p.stroke_rect(b.inflate(3.0, 3.0), t.radius + 3.0, 2.0, t.accent);
        }
        p.fill_rounded_rect(b, t.radius, bg);
        if let Some(text) = &self.text {
            let c = cx.content();
            let s = text.size();
            p.draw_text(
                text,
                Point::new(c.x + (c.width - s.width) / 2.0, c.y + (c.height - s.height) / 2.0),
                fg,
            );
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        match event {
            Event::PointerEnter | Event::PointerLeave => cx.request_paint(),
            Event::PointerDown {
                button: PointerButton::Left,
                ..
            } if self.enabled => {
                self.pressed = true;
                cx.request_paint();
                cx.set_handled();
            }
            Event::PointerUp {
                pos,
                button: PointerButton::Left,
            } => {
                if self.pressed {
                    self.pressed = false;
                    cx.request_paint();
                    if cx.bounds().contains(*pos) {
                        self.click(cx);
                    }
                }
                cx.set_handled();
            }
            Event::Key(k)
                if k.pressed
                    && !k.repeat
                    && self.enabled
                    && matches!(k.key, Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space)) =>
            {
                self.click(cx);
                cx.set_handled();
            }
            Event::FocusLost if self.pressed => {
                self.pressed = false;
                cx.request_paint();
            }
            _ => {}
        }
    }

    fn focusable(&self) -> bool {
        self.enabled
    }

    fn cursor(&self) -> CursorIcon {
        if self.enabled {
            CursorIcon::Pointer
        } else {
            CursorIcon::Default
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Button,
            label: Some(self.label.clone()),
            value: None,
            disabled: !self.enabled,
        }
    }
}
