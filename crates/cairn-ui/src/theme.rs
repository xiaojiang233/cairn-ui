//! 主题：颜色、字号、圆角等全局外观参数。

use crate::{Color, Insets};

/// 全局外观。通过 [`crate::App::theme`] 设置。
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// 窗口背景。
    pub background: Color,
    pub text: Color,
    /// 次要文字，例如输入框占位提示。
    pub text_muted: Color,
    /// 强调色：按钮底色、焦点框、选区。
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    /// 强调色上的文字颜色。
    pub on_accent: Color,
    pub disabled_background: Color,
    pub disabled_text: Color,
    pub input_background: Color,
    pub border: Color,
    pub border_hover: Color,
    pub selection: Color,
    /// 默认字号（逻辑像素）。
    pub font_size: f32,
    /// 默认行高倍数。
    pub line_height: f32,
    /// 按钮、输入框的圆角半径。
    pub radius: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::light()
    }
}

impl Theme {
    pub fn light() -> Self {
        let accent = Color::hex(0x2B66D9);
        Theme {
            background: Color::hex(0xF4F5F7),
            text: Color::hex(0x1E2024),
            text_muted: Color::hex(0x8A9099),
            accent,
            accent_hover: Color::hex(0x3B76E6),
            accent_pressed: Color::hex(0x2152B5),
            on_accent: Color::WHITE,
            disabled_background: Color::hex(0xDADDE2),
            disabled_text: Color::hex(0x9AA0A8),
            input_background: Color::WHITE,
            border: Color::hex(0xC5CAD2),
            border_hover: Color::hex(0x9EA5B0),
            selection: accent.with_alpha(72),
            font_size: 14.0,
            line_height: 1.45,
            radius: 6.0,
        }
    }
}

/// 控件的默认内边距（逻辑像素），不随主题变化，可以用 [`crate::Styled::padding`] 覆盖。
pub(crate) const BUTTON_PADDING: Insets = Insets::symmetric(16.0, 7.0);
pub(crate) const INPUT_PADDING: Insets = Insets::symmetric(10.0, 7.0);
