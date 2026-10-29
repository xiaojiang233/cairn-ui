//! Windows 实现：剪贴板、光标闪烁、双击参数、动画开关、字体目录、默认字体、无边框窗口的阴影与圆角。

use std::path::PathBuf;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCaretBlinkTime, GetSystemMetrics, SM_CXDOUBLECLK, SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
};
use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowAttributes};

use crate::FontFamilies;

fn hwnd(window: &Window) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as HWND),
        _ => None,
    }
}

/// 打开剪贴板；别的程序正占着时稍等重试几次。离开作用域时自动关闭。
struct OpenedClipboard;

impl OpenedClipboard {
    fn open(hwnd: HWND) -> Option<Self> {
        for _ in 0..5 {
            if unsafe { OpenClipboard(hwnd) } != 0 {
                return Some(OpenedClipboard);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }
}

impl Drop for OpenedClipboard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

pub(crate) fn clipboard_get(window: &Window) -> Option<String> {
    let hwnd = hwnd(window)?;
    let format = CF_UNICODETEXT as u32;
    unsafe {
        if IsClipboardFormatAvailable(format) == 0 {
            return None;
        }
        let _open = OpenedClipboard::open(hwnd)?;
        let handle = GetClipboardData(format);
        if handle.is_null() {
            return None;
        }
        let ptr = GlobalLock(handle) as *const u16;
        if ptr.is_null() {
            return None;
        }
        // 以 GlobalSize 为上限找结尾的 0，防止数据不规范时越界读取。
        let max = GlobalSize(handle) / 2;
        let units = std::slice::from_raw_parts(ptr, max);
        let len = units.iter().position(|&u| u == 0).unwrap_or(max);
        let text = String::from_utf16_lossy(&units[..len]);
        GlobalUnlock(handle);
        Some(text)
    }
}

pub(crate) fn clipboard_set(window: &Window, text: &str) -> bool {
    let Some(hwnd) = hwnd(window) else { return false };
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let Some(_open) = OpenedClipboard::open(hwnd) else {
            return false;
        };
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if mem.is_null() {
            return false;
        }
        let dst = GlobalLock(mem) as *mut u16;
        if dst.is_null() {
            GlobalFree(mem);
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
        GlobalUnlock(mem);
        // 成功后内存归系统所有；失败时要自己释放。
        if SetClipboardData(CF_UNICODETEXT as u32, mem).is_null() {
            GlobalFree(mem);
            return false;
        }
        true
    }
}

pub(crate) fn caret_blink_interval() -> Option<Duration> {
    match unsafe { GetCaretBlinkTime() } {
        // INFINITE：用户在系统设置里关掉了光标闪烁。
        u32::MAX => None,
        // 0 表示调用失败，用 Windows 默认值。
        0 => Some(Duration::from_millis(530)),
        ms => Some(Duration::from_millis(ms as u64)),
    }
}

pub(crate) fn double_click_interval() -> Duration {
    Duration::from_millis(unsafe { GetDoubleClickTime() } as u64)
}

pub(crate) fn double_click_distance() -> f64 {
    let px = unsafe { GetSystemMetrics(SM_CXDOUBLECLK) };
    if px > 0 { px as f64 } else { 4.0 }
}

/// “设置 → 辅助功能 → 视觉效果 → 动画效果”关掉时返回 `true`。
pub(crate) fn reduce_motion() -> bool {
    let mut enabled: i32 = 1;
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut enabled as *mut i32).cast(), 0) };
    ok != 0 && enabled == 0
}

pub(crate) fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let windir = std::env::var_os("WINDIR")
        .or_else(|| std::env::var_os("SystemRoot"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    dirs.push(windir.join("Fonts"));
    // Windows 10 1809 起，用户可以“仅为我安装”字体，放在这里。
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
    }
    dirs
}

pub(crate) fn default_font_families(locale: &str) -> FontFamilies {
    let lang = locale.to_ascii_lowercase();
    // 西文用系统界面字体（Windows 11 的 Segoe UI Variable，Windows 10 退到 Segoe UI），
    // 汉字等由 cosmic-text 按文字和语言回退（简体中文是 Microsoft YaHei UI）。
    // 两种 Segoe 都找不到时才用后面的中日韩字体做默认字体。
    let cjk: &[&str] = if lang.starts_with("zh-tw") || lang.starts_with("zh-hk") || lang.starts_with("zh-hant") {
        &["Microsoft JhengHei UI", "Microsoft JhengHei"]
    } else if lang.starts_with("zh") {
        &["Microsoft YaHei UI", "Microsoft YaHei"]
    } else if lang.starts_with("ja") {
        &["Yu Gothic UI", "Meiryo UI"]
    } else if lang.starts_with("ko") {
        &["Malgun Gothic"]
    } else {
        &["Microsoft YaHei UI"]
    };
    let serif: &[&str] = if lang.starts_with("zh") {
        &["SimSun", "Times New Roman"]
    } else {
        &["Times New Roman", "Cambria"]
    };
    let list = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut sans_serif = list(&["Segoe UI Variable", "Segoe UI"]);
    sans_serif.extend(list(cjk));
    FontFamilies {
        sans_serif,
        serif: list(serif),
        monospace: list(&["Cascadia Mono", "Consolas", "Courier New"]),
    }
}

/// 无边框窗口：保留系统投影，Windows 11 上用系统圆角（更早的系统忽略圆角设置）。
pub(crate) fn undecorated(attrs: WindowAttributes) -> WindowAttributes {
    attrs
        .with_undecorated_shadow(true)
        .with_corner_preference(CornerPreference::Round)
}
