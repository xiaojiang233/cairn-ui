//! 运行时需要的宿主服务。真实窗口由平台层提供，测试时用记录型的假实现。

use std::time::Instant;

use cairn_ui_platform::{Context, CursorIcon, PixelRect};

pub(crate) trait Host {
    fn clipboard_get(&mut self) -> Option<String>;
    fn clipboard_set(&mut self, text: &str);
    fn set_cursor(&mut self, icon: CursorIcon);
    fn set_ime_allowed(&mut self, allowed: bool);
    fn set_ime_cursor_area(&mut self, area: PixelRect);
    fn request_redraw(&mut self);
    fn set_wakeup(&mut self, at: Option<Instant>);
    fn exit(&mut self);
}

/// 真实窗口。
pub(crate) struct PlatformHost<'a, 'b>(pub &'a mut Context<'b>);

impl Host for PlatformHost<'_, '_> {
    fn clipboard_get(&mut self) -> Option<String> {
        self.0.clipboard_get()
    }
    fn clipboard_set(&mut self, text: &str) {
        self.0.clipboard_set(text);
    }
    fn set_cursor(&mut self, icon: CursorIcon) {
        self.0.set_cursor(icon);
    }
    fn set_ime_allowed(&mut self, allowed: bool) {
        self.0.set_ime_allowed(allowed);
    }
    fn set_ime_cursor_area(&mut self, area: PixelRect) {
        self.0.set_ime_cursor_area(area);
    }
    fn request_redraw(&mut self) {
        self.0.request_redraw();
    }
    fn set_wakeup(&mut self, at: Option<Instant>) {
        self.0.set_wakeup(at);
    }
    fn exit(&mut self) {
        self.0.exit();
    }
}

/// 不连窗口的宿主：把请求记下来，供测试检查。
#[derive(Debug, Default)]
pub(crate) struct RecordingHost {
    pub clipboard: Option<String>,
    pub cursor: CursorIcon,
    pub ime_allowed: bool,
    pub ime_area: Option<PixelRect>,
    pub redraw_requested: bool,
    pub wakeup: Option<Instant>,
    pub exit_requested: bool,
}

impl Host for RecordingHost {
    fn clipboard_get(&mut self) -> Option<String> {
        self.clipboard.clone()
    }
    fn clipboard_set(&mut self, text: &str) {
        self.clipboard = Some(text.to_string());
    }
    fn set_cursor(&mut self, icon: CursorIcon) {
        self.cursor = icon;
    }
    fn set_ime_allowed(&mut self, allowed: bool) {
        if self.ime_allowed != allowed {
            self.ime_area = None;
        }
        self.ime_allowed = allowed;
    }
    fn set_ime_cursor_area(&mut self, area: PixelRect) {
        self.ime_area = Some(area);
    }
    fn request_redraw(&mut self) {
        self.redraw_requested = true;
    }
    fn set_wakeup(&mut self, at: Option<Instant>) {
        self.wakeup = at;
    }
    fn exit(&mut self) {
        self.exit_requested = true;
    }
}
