//! 与具体窗口库无关的输入事件类型。上层只看到这些类型，看不到 winit。

/// 平台层投递给上层的事件。坐标一律是**物理像素**，由上层按缩放因子换算。
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// 客户区大小变化（物理像素）。
    Resized { width: u32, height: u32 },
    /// 缩放因子变化：用户改了系统缩放，或窗口被拖到另一块 DPI 不同的显示器上。
    /// 随后通常会紧跟一个 [`Event::Resized`]。
    ScaleFactorChanged { scale: f64 },
    /// 用户点了关闭按钮。上层决定是否调用 [`crate::Context::exit`]。
    CloseRequested,
    /// 窗口获得 / 失去键盘焦点。
    WindowFocused(bool),
    /// 指针移动，位置相对客户区左上角（物理像素）。
    PointerMoved { x: f64, y: f64 },
    /// 指针离开窗口。
    PointerLeft,
    /// 鼠标按键按下 / 抬起。位置以最近一次 [`Event::PointerMoved`] 为准。
    PointerButton { button: PointerButton, pressed: bool },
    /// 滚轮。
    Wheel(ScrollDelta),
    /// 键盘按键。IME 组字过程中被输入法吃掉的按键不会以字符形式出现。
    Key(KeyEvent),
    /// 修饰键状态变化。
    ModifiersChanged(Modifiers),
    /// 输入法事件。只有调用过 [`crate::Context::set_ime_allowed`]`(true)` 才会收到。
    Ime(ImeEvent),
    /// 有文件被拖到窗口上方（还没松开）。拖入多个文件时每个文件一个事件。
    FileHovered(std::path::PathBuf),
    /// 拖到窗口上方的文件又被拖走了。
    FileHoverCancelled,
    /// 文件被拖放进窗口。拖入多个文件时每个文件一个事件。
    FileDropped(std::path::PathBuf),
}

/// 改变窗口大小时拖动的边或角。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResizeEdge {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other(u16),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollDelta {
    /// 以“行”为单位（普通滚轮）。
    Lines { x: f32, y: f32 },
    /// 以物理像素为单位（触控板等）。
    Pixels { x: f64, y: f64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    /// 这次按键产生的文本（已考虑 Shift、大小写、死键等）。控制键通常为 `None`。
    pub text: Option<String>,
    pub pressed: bool,
    pub repeat: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Named(NamedKey),
    /// 字符键，内容是当前布局下的字符（不受 Ctrl 影响）。
    Character(String),
    Unidentified,
}

/// 上层需要识别的功能键子集。没列出的统一归到 [`NamedKey::Other`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Tab,
    Space,
    Backspace,
    Delete,
    Escape,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Shift,
    Control,
    Alt,
    Super,
    /// Windows 上被输入法处理掉的按键（VK_PROCESSKEY）。文本框应忽略它。
    Process,
    F(u8),
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Windows 键 / Command 键。
    pub logo: bool,
}

impl Modifiers {
    /// 平台惯用的快捷键修饰键：Windows / Linux 上是 Ctrl，macOS 上是 Command。
    pub fn shortcut(&self) -> bool {
        if cfg!(target_vendor = "apple") {
            self.logo
        } else {
            self.ctrl
        }
    }

    /// 按单词移动光标用的修饰键：Windows / Linux 上是 Ctrl，macOS 上是 Option。
    pub fn word_jump(&self) -> bool {
        if cfg!(target_vendor = "apple") {
            self.alt
        } else {
            self.ctrl
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// 一次组字开始（Windows 上每次组字都会先发这个，不代表焦点变化）。
    Enabled,
    /// 预编辑文本更新。`text` 为空表示清除预编辑。
    /// `cursor` 是预编辑文本内的字节区间（输入法当前选中的子句或光标），`None` 表示隐藏光标。
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// 提交最终文本。
    Commit(String),
    /// 一次组字结束。应清除残留的预编辑文本。
    Disabled,
}

/// 鼠标指针形状。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorIcon {
    #[default]
    Default,
    /// 手形，用于可点击元素。
    Pointer,
    /// I 形，用于文本。
    Text,
    NotAllowed,
    /// 左右双向箭头，用于改变宽度。
    ResizeHorizontal,
    /// 上下双向箭头，用于改变高度。
    ResizeVertical,
    /// 左上—右下双向箭头。
    ResizeNwSe,
    /// 右上—左下双向箭头。
    ResizeNeSw,
}
