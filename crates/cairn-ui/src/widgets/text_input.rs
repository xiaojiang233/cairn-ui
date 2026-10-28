//! 单行文本输入框：光标移动、删除、选区、剪贴板、中文输入法预编辑。

use cairn_ui_layout::MeasureInput;

use crate::anim::Transition;
use crate::element::element;
use crate::text_edit::EditState;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::drive;
use crate::{
    CursorIcon, FontFamily, ImeEvent, Insets, Key, KeyEvent, NamedKey, Painter, Point, PointerButton, Rect, Shadow,
    Size, Style, TextLayout, TextStyle,
};

/// 没有指定宽度时的内容区宽度（逻辑像素）。
const DEFAULT_WIDTH: f32 = 200.0;

/// 输入法正在组字、尚未提交的文本。显示在光标处，不属于输入框的值。
struct Preedit {
    text: String,
    /// 预编辑文本内的字节区间（输入法给出）。两端相等是光标位置，不等是当前正在转换的子句。
    cursor: Option<(usize, usize)>,
}

type TextHandler<S> = Box<dyn Fn(&mut S, String)>;

/// 单行文本输入框。用 [`text_input`] 创建。
///
/// 值由应用持有：传入当前值，用 [`TextInput::on_change`] 接收用户的修改并写回状态。
/// 设置了 `on_change` 时输入框是“受控”的——界面重建时若应用给的值与框内内容不同，
/// 以应用为准（例如应用清空了它）。没设置 `on_change` 时只在应用给的值本身变化时才覆盖。
///
/// 外观来自主题的 [`Theme::input`]：悬停、获得焦点时填充与边框渐变，焦点光晕渐显。
pub struct TextInput<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    value: String,
    placeholder: String,
    on_change: Option<TextHandler<S>>,
    on_submit: Option<TextHandler<S>>,
    edit: EditState,
    preedit: Option<Preedit>,
    /// 显示用的排版：内容加上插在光标处的预编辑文本。
    display: Option<TextLayout>,
    hint: Option<TextLayout>,
    scroll_x: f32,
    content_width: f32,
    caret_on: bool,
    dragging: bool,
    hover: Transition,
    focus: Transition,
}

pub fn text_input<S: 'static>(value: impl Into<String>) -> TextInput<S> {
    let value = value.into();
    TextInput {
        style: Style {
            padding: Insets::AUTO,
            ..Style::default()
        },
        key: None,
        edit: EditState::new(value.clone()),
        value,
        placeholder: String::new(),
        on_change: None,
        on_submit: None,
        preedit: None,
        display: None,
        hint: None,
        scroll_x: 0.0,
        content_width: DEFAULT_WIDTH,
        caret_on: true,
        dragging: false,
        hover: Transition::default(),
        focus: Transition::default(),
    }
}

element!(TextInput<S>);

/// 去掉控制字符；换行变成空格（粘贴多行文本到单行框时）。
fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            '\n' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

impl<S: 'static> TextInput<S> {
    /// 内容为空时显示的提示文字。
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }

    /// 内容被用户修改时调用，参数是应用状态和新内容。
    pub fn on_change(mut self, f: impl Fn(&mut S, String) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// 按回车时调用。
    pub fn on_submit(mut self, f: impl Fn(&mut S, String) + 'static) -> Self {
        self.on_submit = Some(Box::new(f));
        self
    }

    fn text_style(theme: &Theme) -> TextStyle {
        let body = theme.typography.body;
        TextStyle {
            font_size: body.size,
            line_height: theme.typography.line_height,
            weight: body.weight,
            family: FontFamily::SansSerif,
        }
    }

    fn display_text(&self) -> String {
        let text = self.edit.text();
        match &self.preedit {
            None => text.to_string(),
            Some(pe) => {
                let c = self.edit.cursor();
                format!("{}{}{}", &text[..c], pe.text, &text[c..])
            }
        }
    }

    /// 光标在显示文本中的位置；输入法正在转换某个子句时不显示光标，返回 `None`。
    fn caret_index(&self) -> Option<usize> {
        let c = self.edit.cursor();
        match &self.preedit {
            None => Some(c),
            Some(pe) => match pe.cursor {
                Some((a, b)) if a == b => Some(c + b.min(pe.text.len())),
                Some(_) => None,
                None => Some(c + pe.text.len()),
            },
        }
    }

    fn sync_layouts(&mut self, cx: &mut LayoutCx) {
        let style = Self::text_style(cx.theme());
        let text = self.display_text();
        match &mut self.display {
            Some(d) => {
                d.set_style(cx.text(), &style);
                d.set_text(cx.text(), &text);
            }
            None => self.display = Some(TextLayout::new(cx.text(), &text, &style)),
        }
        match &mut self.hint {
            Some(h) => {
                h.set_style(cx.text(), &style);
                h.set_text(cx.text(), &self.placeholder);
            }
            None => self.hint = Some(TextLayout::new(cx.text(), &self.placeholder, &style)),
        }
    }

    fn line_height(&self) -> f32 {
        self.display.as_ref().map(|d| d.line_height()).unwrap_or(20.0)
    }

    /// 文字框左上角（窗口坐标）：内容区左边减去横向滚动量，纵向居中。
    fn text_origin(&self, content: Rect) -> Point {
        Point::new(
            content.x - self.scroll_x,
            content.y + (content.height - self.line_height()) / 2.0,
        )
    }

    /// 光标矩形（窗口坐标）。宽度为 1 个物理像素（150% 及以上为 2 个）。
    fn caret_rect(&self, content: Rect, scale: f32) -> Option<Rect> {
        let display = self.display.as_ref()?;
        let pos = display.cursor_position(self.caret_index()?);
        let origin = self.text_origin(content);
        let w = scale.round().max(1.0) / scale;
        Some(Rect::new(origin.x + pos.x, origin.y + pos.y, w, display.line_height()))
    }

    /// 横向滚动，保证光标可见。
    fn scroll_to_caret(&mut self) {
        let Some(display) = &self.display else { return };
        const CARET_ROOM: f32 = 2.0;
        let w = self.content_width;
        let index = self.caret_index().unwrap_or(self.edit.cursor());
        let x = display.cursor_position(index).x;
        let max_scroll = (display.size().width + CARET_ROOM - w).max(0.0);
        let mut s = self.scroll_x;
        if x - s > w - CARET_ROOM {
            s = x - w + CARET_ROOM;
        }
        if x - s < 0.0 {
            s = x;
        }
        self.scroll_x = s.clamp(0.0, max_scroll);
    }

    fn hit_index(&self, content: Rect, pos: Point) -> usize {
        match &self.display {
            Some(d) => d.hit(Point::new(pos.x - content.x + self.scroll_x, d.line_height() / 2.0)),
            None => 0,
        }
    }

    fn reset_blink(&mut self, cx: &mut EventCx<S>) {
        self.caret_on = true;
        if let Some(interval) = cx.caret_blink_interval() {
            cx.request_timer(cx.now() + interval);
        }
    }

    /// 光标或选区变了。
    fn moved(&mut self, cx: &mut EventCx<S>) {
        self.scroll_to_caret();
        self.reset_blink(cx);
        cx.request_paint();
    }

    /// 内容（或预编辑文本）变了。
    fn relayout(&mut self, cx: &mut EventCx<S>) {
        self.sync_layouts(&mut cx.layout_cx());
        self.moved(cx);
    }

    /// 用户修改了内容：重新排版并通知应用。
    fn edited(&mut self, cx: &mut EventCx<S>) {
        self.relayout(cx);
        if let Some(f) = &self.on_change {
            f(cx.app(), self.edit.text().to_string());
        }
    }

    fn key(&mut self, cx: &mut EventCx<S>, k: &KeyEvent) {
        let m = cx.modifiers();
        let (word, shift) = (m.word_jump(), m.shift);
        let mut handled = true;
        let mut edited = false;
        let mut moved = false;
        match &k.key {
            // 输入法正在处理这个键，等它的 Preedit / Commit。
            Key::Named(NamedKey::Process) => return,
            Key::Named(NamedKey::ArrowLeft) => {
                self.edit.move_left(word, shift);
                moved = true;
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.edit.move_right(word, shift);
                moved = true;
            }
            Key::Named(NamedKey::Home) => {
                self.edit.home(shift);
                moved = true;
            }
            Key::Named(NamedKey::End) => {
                self.edit.end(shift);
                moved = true;
            }
            Key::Named(NamedKey::Backspace) => edited = self.edit.backspace(word),
            Key::Named(NamedKey::Delete) => edited = self.edit.delete_forward(word),
            Key::Named(NamedKey::Enter) => {
                if let Some(f) = &self.on_submit {
                    f(cx.app(), self.edit.text().to_string());
                }
            }
            Key::Character(c) if m.shortcut() && !m.alt => match c.to_lowercase().as_str() {
                "a" => {
                    self.edit.select_all();
                    moved = true;
                }
                "c" => {
                    if self.edit.has_selection() {
                        cx.clipboard_set(self.edit.selected_text());
                    }
                }
                "x" => {
                    if self.edit.has_selection() {
                        cx.clipboard_set(self.edit.selected_text());
                        edited = self.edit.delete_selection();
                    }
                }
                "v" => {
                    let text = cx.clipboard_get().map(|t| sanitize(&t)).unwrap_or_default();
                    if !text.is_empty() {
                        self.edit.insert(&text);
                        edited = true;
                    }
                }
                _ => handled = false,
            },
            _ => {
                // Ctrl+字母 不是文字输入；AltGr（Windows 上表现为 Ctrl+Alt）打出的字符是。
                let text = k
                    .text
                    .as_deref()
                    .filter(|_| !m.ctrl || m.alt)
                    .map(sanitize)
                    .unwrap_or_default();
                if text.is_empty() {
                    handled = false;
                } else {
                    self.edit.insert(&text);
                    edited = true;
                }
            }
        }
        if handled {
            cx.set_handled();
        }
        if edited {
            self.edited(cx);
        } else if moved {
            self.moved(cx);
        }
    }

    fn ime(&mut self, cx: &mut EventCx<S>, ev: &ImeEvent) {
        cx.set_handled();
        match ev {
            ImeEvent::Enabled => {}
            ImeEvent::Preedit { text, cursor } if !text.is_empty() => {
                // 开始组字时有选区：先删掉选区（与常见编辑器一致，候选字替换选中内容）。
                let deleted = self.preedit.is_none() && self.edit.delete_selection();
                self.preedit = Some(Preedit {
                    text: text.clone(),
                    cursor: *cursor,
                });
                if deleted { self.edited(cx) } else { self.relayout(cx) }
            }
            ImeEvent::Preedit { .. } | ImeEvent::Disabled => {
                if self.preedit.take().is_some() {
                    self.relayout(cx);
                }
            }
            ImeEvent::Commit(text) => {
                self.preedit = None;
                let text = sanitize(text);
                if text.is_empty() {
                    self.relayout(cx);
                } else {
                    self.edit.insert(&text);
                    self.edited(cx);
                }
            }
        }
    }
}

impl<S: 'static> Widget<S> for TextInput<S> {
    fn update(&mut self, cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        let controlled = new.on_change.is_some();
        let external = new.value != self.value;
        if (controlled && new.value != self.edit.text()) || (!controlled && external) {
            self.edit.replace_text(new.value.clone());
            self.preedit = None;
            change = Change::PAINT;
        }
        if new.placeholder != self.placeholder {
            change = Change::PAINT;
        }
        self.value = new.value;
        self.placeholder = new.placeholder;
        self.on_change = new.on_change;
        self.on_submit = new.on_submit;
        if !change.is_none() {
            self.sync_layouts(cx);
            self.scroll_to_caret();
        }
        change
    }

    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        if style.padding.is_auto() {
            style.padding = theme.input.padding;
        }
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        self.sync_layouts(cx);
        Size::new(DEFAULT_WIDTH, self.line_height())
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let e = theme.input.focus_glow.extent();
        bounds.outset(e[0], e[1], e[2], e[3])
    }

    fn arrange(&mut self, cx: &mut LayoutCx, _bounds: Rect, content: Rect) {
        self.content_width = content.width;
        self.sync_layouts(cx);
        self.scroll_to_caret();
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let motion = &cx.theme().motion;
        let (focused, hovered) = (cx.is_focused(), cx.is_hovered());
        drive(&mut self.hover, cx, hovered, motion.hover, motion.easing);
        drive(&mut self.focus, cx, focused, motion.focus, motion.easing);
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = &theme.input;
        let (b, c) = (cx.bounds(), cx.content());
        let focused = cx.is_focused();
        let now = cx.now();
        let (hover, focus) = (self.hover.value(now), self.focus.value(now));
        let radius = theme.radius.control;

        // 焦点光晕画在外面，随焦点程度渐显；再依次是填充和边框。
        let glow = Shadow {
            color: look.focus_glow.color.fade(focus),
            ..look.focus_glow
        };
        p.draw_shadow(b, radius, &glow);
        p.fill_rounded_rect(
            b,
            radius,
            look.fill.mix(look.fill_hover, hover).mix(look.fill_focus, focus),
        );
        let border = look.border.mix(look.border_hover, hover).mix(look.border_focus, focus);
        let width = if focused {
            look.border_focus_width
        } else {
            look.border_width
        };
        p.stroke_rect(b, radius, width, border);

        let (Some(display), Some(hint)) = (&self.display, &self.hint) else {
            return;
        };
        let origin = self.text_origin(c);
        let px = p.scale().round().max(1.0) / p.scale();
        p.push_clip(c);
        if display.text().is_empty() {
            p.draw_text(hint, Point::new(c.x, origin.y), look.placeholder);
        } else {
            if focused && self.preedit.is_none() && self.edit.has_selection() {
                for r in display.selection_rects(self.edit.selection()) {
                    p.fill_rect(r.translate(origin), look.selection);
                }
            }
            p.draw_text(display, origin, look.text);
            if let Some(pe) = &self.preedit {
                // 预编辑文本画下划线；输入法正在转换的子句用粗下划线。
                let start = self.edit.cursor();
                let y = origin.y + display.first_baseline() + 2.0 * px.max(1.0);
                for r in display.selection_rects(start..start + pe.text.len()) {
                    p.fill_rect(Rect::new(origin.x + r.x, y, r.width, px), look.preedit);
                }
                if let Some((a, z)) = pe.cursor.filter(|(a, z)| a < z) {
                    for r in display.selection_rects(start + a..start + z.min(pe.text.len())) {
                        p.fill_rect(Rect::new(origin.x + r.x, y, r.width, 2.0 * px), look.preedit);
                    }
                }
            }
        }
        if focused
            && cx.window_focused()
            && self.caret_on
            && let Some(caret) = self.caret_rect(c, p.scale())
        {
            p.fill_rect(caret, look.caret);
        }
        p.pop_clip();
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        match event {
            Event::PointerEnter | Event::PointerLeave => cx.request_paint(),
            Event::FocusGained => {
                self.reset_blink(cx);
                cx.request_paint();
            }
            Event::FocusLost => {
                self.dragging = false;
                if self.preedit.take().is_some() {
                    self.relayout(cx);
                }
                cx.request_paint();
            }
            Event::WindowFocusChanged(focused) => {
                if *focused {
                    self.reset_blink(cx);
                }
                cx.request_paint();
            }
            Event::Timer => {
                // 失去焦点后不再续订，闪烁自然停止。
                if cx.is_focused()
                    && cx.window_focused()
                    && let Some(interval) = cx.caret_blink_interval()
                {
                    self.caret_on = !self.caret_on;
                    if let Some(r) = self.caret_rect(cx.content(), cx.scale()) {
                        cx.request_paint_rect(r.inflate(1.0, 1.0));
                    }
                    cx.request_timer(cx.now() + interval);
                }
            }
            Event::PointerDown {
                pos,
                button: PointerButton::Left,
                count,
            } => {
                cx.set_handled();
                // 组字过程中点击交给输入法处理（多数输入法会自行提交或取消）。
                if self.preedit.is_some() {
                    return;
                }
                let index = self.hit_index(cx.content(), *pos);
                match count {
                    1 => {
                        self.edit.set_cursor(index, cx.modifiers().shift);
                        self.dragging = true;
                    }
                    2 => self.edit.select_word_at(index),
                    _ => self.edit.select_all(),
                }
                self.moved(cx);
            }
            Event::PointerMove { pos } if self.dragging => {
                let index = self.hit_index(cx.content(), *pos);
                self.edit.set_cursor(index, true);
                self.moved(cx);
            }
            Event::PointerUp {
                button: PointerButton::Left,
                ..
            } => {
                self.dragging = false;
                cx.set_handled();
            }
            Event::Key(k) if k.pressed => self.key(cx, k),
            Event::Ime(ime) => self.ime(cx, ime),
            _ => {}
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn wants_ime(&self) -> bool {
        true
    }

    fn ime_area(&self, bounds: Rect, content: Rect) -> Option<Rect> {
        let display = self.display.as_ref()?;
        // 组字时取预编辑文本的起点，候选框不会随着输入的拼音左右跳；平时取光标位置。
        let index = if self.preedit.is_some() {
            self.edit.cursor()
        } else {
            self.caret_index()?
        };
        let x = (self.text_origin(content).x + display.cursor_position(index).x).clamp(content.x, content.right());
        // 纵向用整个输入框，候选框会出现在输入框下方而不是盖住它。
        Some(Rect::new(x, bounds.y, 1.0, bounds.height))
    }

    fn cursor(&self) -> CursorIcon {
        CursorIcon::Text
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::TextInput,
            label: (!self.placeholder.is_empty()).then(|| self.placeholder.clone()),
            value: Some(self.edit.text().to_string()),
            disabled: false,
        }
    }
}
