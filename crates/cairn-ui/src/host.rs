//! 运行时需要的宿主服务。真实窗口由平台层提供，测试时用记录型的假实现。

use std::time::{Duration, Instant};

use cairn_ui_platform::{Context, CursorIcon, PixelRect, ResizeEdge};

pub(crate) trait Host {
    fn clipboard_get(&mut self) -> Option<String>;
    fn clipboard_set(&mut self, text: &str);
    fn set_cursor(&mut self, icon: CursorIcon);
    fn set_ime_allowed(&mut self, allowed: bool);
    fn set_ime_cursor_area(&mut self, area: PixelRect);
    fn request_redraw(&mut self);
    fn set_wakeup(&mut self, at: Option<Instant>);
    /// 动画帧间隔（显示器刷新间隔）。
    fn frame_interval(&self) -> Duration;
    fn exit(&mut self);
    /// 无边框窗口自己处理的窗口操作。
    fn drag_window(&mut self);
    fn drag_resize_window(&mut self, edge: ResizeEdge);
    fn minimize_window(&mut self);
    fn toggle_maximize_window(&mut self);
    fn is_window_maximized(&self) -> bool;
    fn is_window_maximizable(&self) -> bool {
        true
    }
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
    fn frame_interval(&self) -> Duration {
        self.0.frame_interval()
    }
    fn exit(&mut self) {
        self.0.exit();
    }
    fn drag_window(&mut self) {
        self.0.drag_window();
    }
    fn drag_resize_window(&mut self, edge: ResizeEdge) {
        self.0.drag_resize_window(edge);
    }
    fn minimize_window(&mut self) {
        self.0.minimize();
    }
    fn toggle_maximize_window(&mut self) {
        self.0.toggle_maximize();
    }
    fn is_window_maximized(&self) -> bool {
        self.0.is_maximized()
    }
    fn is_window_maximizable(&self) -> bool {
        self.0.is_maximizable()
    }
}

/// 测试里假定的显示器刷新间隔（60 Hz）。
pub(crate) const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

/// 不连窗口的宿主：把请求记下来，供测试检查。
#[derive(Debug)]
pub(crate) struct RecordingHost {
    pub clipboard: Option<String>,
    pub cursor: CursorIcon,
    pub ime_allowed: bool,
    pub ime_area: Option<PixelRect>,
    pub redraw_requested: bool,
    pub wakeup: Option<Instant>,
    pub exit_requested: bool,
    pub drag_requested: bool,
    pub resize_requested: Option<ResizeEdge>,
    pub minimized: bool,
    pub maximized: bool,
    pub maximizable: bool,
}

impl Default for RecordingHost {
    fn default() -> Self {
        RecordingHost {
            clipboard: None,
            cursor: CursorIcon::Default,
            ime_allowed: false,
            ime_area: None,
            redraw_requested: false,
            wakeup: None,
            exit_requested: false,
            drag_requested: false,
            resize_requested: None,
            minimized: false,
            maximized: false,
            maximizable: true,
        }
    }
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
    fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }
    fn exit(&mut self) {
        self.exit_requested = true;
    }
    fn drag_window(&mut self) {
        self.drag_requested = true;
    }
    fn drag_resize_window(&mut self, edge: ResizeEdge) {
        self.resize_requested = Some(edge);
    }
    fn minimize_window(&mut self) {
        self.minimized = true;
    }
    fn toggle_maximize_window(&mut self) {
        self.maximized = !self.maximized;
    }
    fn is_window_maximized(&self) -> bool {
        self.maximized
    }
    fn is_window_maximizable(&self) -> bool {
        self.maximizable
    }
}
