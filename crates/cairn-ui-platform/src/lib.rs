//! cairn-ui 的平台层。
//!
//! 这一层把操作系统相关的东西都收在这里：窗口与事件循环（winit）、软件上屏（softbuffer）、
//! IME、DPI、剪贴板、字体目录、光标闪烁等系统参数。上层 crate 只使用这里定义的类型，
//! 不直接接触 winit / softbuffer，也不出现任何 Windows 专有调用。
//!
//! 使用方式：实现 [`Handler`]，然后调用 [`run`]。
//!
//! ## 调试用环境变量
//!
//! `CAIRN_UI_SCALE_FACTOR=1.5`：忽略系统缩放，强制使用给定的缩放因子。只用于在单显示器、
//! 不改系统设置的情况下检查 125% / 150% 等缩放下的显示效果；它**不能**代替真实的系统缩放测试
//! （例如不会触发跨显示器时的 DPI 变化流程）。

mod convert;
mod event;
mod present;
mod sys;

pub use event::*;
pub use present::Surface;

use std::any::Any;
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

/// 物理像素坐标下的整数矩形。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// 窗口创建参数。尺寸是**逻辑像素**。
#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub min_size: Option<(f64, f64)>,
    pub max_size: Option<(f64, f64)>,
    pub resizable: bool,
    pub maximizable: bool,
    /// 系统标题栏与边框。关闭后客户区铺满整个窗口，标题栏、窗口按钮、拖动和缩放边由上层自己画、自己处理
    /// （[`Context::drag_window`]、[`Context::drag_resize_window`] 等）。Windows 上仍保留系统阴影与 Windows 11 圆角。
    pub decorations: bool,
    /// 窗口图标。
    pub icon: Option<IconData>,
}

/// 窗口图标数据（RGBA 像素，尺寸）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconData {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl IconData {
    pub fn new(rgba: Vec<u8>, width: u32, height: u32) -> Self {
        Self { rgba, width, height }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        WindowConfig {
            title: "cairn-ui".into(),
            width: 640.0,
            height: 480.0,
            min_size: None,
            max_size: None,
            resizable: true,
            maximizable: true,
            decorations: true,
            icon: None,
        }
    }
}

/// 上层实现的事件处理器。所有回调都在主线程上调用。
pub trait Handler: 'static {
    /// 窗口创建完成后调用一次，此时可以查询缩放因子和窗口尺寸。
    fn init(&mut self, cx: &mut Context);
    /// 输入与窗口事件。
    fn event(&mut self, cx: &mut Context, event: Event);
    /// 需要出图时调用：系统要求重绘，或者上层调用过 [`Context::request_redraw`]。
    fn redraw(&mut self, cx: &mut Context, surface: &mut Surface);
    /// [`Context::set_wakeup`] 设定的时间到了。
    fn timer(&mut self, _cx: &mut Context) {}
    /// 其他线程通过 [`Proxy::send`] 发来的数据。
    fn user_event(&mut self, _cx: &mut Context, _payload: Box<dyn Any + Send>) {}
}

/// 平台层错误。
#[derive(Debug)]
pub struct Error(ErrorKind);

#[derive(Debug)]
enum ErrorKind {
    EventLoop(winit::error::EventLoopError),
    Window(winit::error::OsError),
    Surface(softbuffer::SoftBufferError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorKind::EventLoop(e) => write!(f, "事件循环错误: {e}"),
            ErrorKind::Window(e) => write!(f, "创建窗口失败: {e}"),
            ErrorKind::Surface(e) => write!(f, "上屏失败: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<softbuffer::SoftBufferError> for Error {
    fn from(e: softbuffer::SoftBufferError) -> Self {
        Error(ErrorKind::Surface(e))
    }
}

type UserPayload = Box<dyn Any + Send>;

/// 跨线程唤醒事件循环并投递数据。可以 `clone` 后发到别的线程。
#[derive(Clone)]
pub struct Proxy(EventLoopProxy<UserPayload>);

impl Proxy {
    /// 投递数据，主线程上会以 [`Handler::user_event`] 收到。事件循环已退出时返回 `false`。
    pub fn send(&self, payload: Box<dyn Any + Send>) -> bool {
        self.0.send_event(payload).is_ok()
    }
}

impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Proxy")
    }
}

/// 回调期间对窗口的操作入口。
pub struct Context<'a> {
    window: &'a Window,
    event_loop: &'a ActiveEventLoop,
    shared: &'a mut Shared,
}

impl Context<'_> {
    /// 当前缩放因子（物理像素 / 逻辑像素）。
    pub fn scale_factor(&self) -> f64 {
        self.shared.scale_override.unwrap_or_else(|| self.window.scale_factor())
    }

    /// 客户区物理尺寸。最小化时可能为 0。
    pub fn physical_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width, s.height)
    }

    /// 请求一次 [`Handler::redraw`]。多次请求会被合并。
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// 窗口所在显示器的刷新间隔，用来给动画定帧率。查不到时按 60 Hz。
    pub fn frame_interval(&self) -> Duration {
        let mhz = self
            .window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .filter(|&m| m >= 20_000)
            .unwrap_or(60_000);
        Duration::from_nanos(1_000_000_000_000 / mhz as u64)
    }

    pub fn set_cursor(&mut self, icon: CursorIcon) {
        if self.shared.cursor != icon {
            self.shared.cursor = icon;
            self.window.set_cursor(convert::cursor_icon(icon));
        }
    }

    /// 打开 / 关闭输入法。文本框获得焦点时打开，失去焦点时关闭。
    pub fn set_ime_allowed(&mut self, allowed: bool) {
        if self.shared.ime_allowed != allowed {
            self.shared.ime_allowed = allowed;
            self.shared.ime_area = None;
            self.window.set_ime_allowed(allowed);
        }
    }

    /// 告诉输入法当前编辑位置（物理像素），候选框会贴着这个区域显示且尽量不遮挡它。
    pub fn set_ime_cursor_area(&mut self, area: PixelRect) {
        if self.shared.ime_area != Some(area) {
            self.shared.ime_area = Some(area);
            self.window.set_ime_cursor_area(
                PhysicalPosition::new(area.x as i32, area.y as i32),
                PhysicalSize::new(area.width.max(1), area.height.max(1)),
            );
        }
    }

    /// 在给定时间唤醒一次（[`Handler::timer`]）。`None` 取消。只保留一个时间点。
    pub fn set_wakeup(&mut self, at: Option<Instant>) {
        self.shared.wakeup = at;
    }

    pub fn exit(&self) {
        self.event_loop.exit();
    }

    /// 开始用鼠标拖动窗口（系统的移动循环，支持拖到屏幕边缘贴靠）。在左键按下的事件里调用。
    pub fn drag_window(&self) {
        let _ = self.window.drag_window();
    }

    /// 开始从某条边或某个角拖动改变窗口大小。在左键按下的事件里调用。
    pub fn drag_resize_window(&self, edge: ResizeEdge) {
        let _ = self.window.drag_resize_window(convert::resize_direction(edge));
    }

    pub fn minimize(&self) {
        self.window.set_minimized(true);
    }

    pub fn is_maximized(&self) -> bool {
        self.window.is_maximized()
    }

    pub fn is_maximizable(&self) -> bool {
        self.shared.maximizable
    }

    /// 最大化与还原之间切换。
    pub fn toggle_maximize(&self) {
        if self.shared.maximizable {
            self.window.set_maximized(!self.window.is_maximized());
        }
    }

    /// 动态设置窗口图标。
    pub fn set_window_icon(&self, icon: Option<IconData>) {
        let winit_icon = icon.and_then(|ic| winit::window::Icon::from_rgba(ic.rgba, ic.width, ic.height).ok());
        self.window.set_window_icon(winit_icon);
    }

    pub fn proxy(&self) -> Proxy {
        Proxy(self.shared.proxy.clone())
    }

    pub fn clipboard_get(&self) -> Option<String> {
        sys::clipboard_get(self.window)
    }

    pub fn clipboard_set(&self, text: &str) -> bool {
        sys::clipboard_set(self.window, text)
    }
}

/// 字体查找配置：去哪些目录找字体、各通用字族优先用哪些字体。
#[derive(Clone, Debug)]
pub struct FontConfig {
    pub dirs: Vec<PathBuf>,
    pub families: FontFamilies,
    /// BCP 47 语言标签，如 `zh-CN`。字体回退时用来区分简繁日韩字形。
    pub locale: String,
}

/// 通用字族到具体字体名的映射，按优先级排列，取第一个系统里存在的。
#[derive(Clone, Debug, Default)]
pub struct FontFamilies {
    pub sans_serif: Vec<String>,
    pub serif: Vec<String>,
    pub monospace: Vec<String>,
}

/// 当前系统的用户界面语言，如 `zh-CN`。
pub fn locale() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en-US".to_string())
}

/// 当前系统的字体目录与默认字体。
pub fn font_config() -> FontConfig {
    let locale = locale();
    FontConfig {
        dirs: sys::font_dirs(),
        families: sys::default_font_families(&locale),
        locale,
    }
}

/// 文本光标闪烁间隔。`None` 表示用户关闭了闪烁。
pub fn caret_blink_interval() -> Option<Duration> {
    sys::caret_blink_interval()
}

/// 双击判定的最大时间间隔。
pub fn double_click_interval() -> Duration {
    sys::double_click_interval()
}

/// 双击判定的最大移动距离（物理像素）。
pub fn double_click_distance() -> f64 {
    sys::double_click_distance()
}

/// 用户在系统里关闭了界面动画（Windows：设置 → 辅助功能 → 视觉效果 → 动画效果）。
/// 上层应据此把过渡动画的时长设为 0。只在调用时读取一次，不跟踪之后的变化。
pub fn reduce_motion() -> bool {
    sys::reduce_motion()
}

/// 创建窗口并运行事件循环，直到 [`Context::exit`] 被调用。必须在主线程调用。
pub fn run<H: Handler>(config: WindowConfig, handler: H) -> Result<(), Error> {
    let event_loop = EventLoop::<UserPayload>::with_user_event()
        .build()
        .map_err(|e| Error(ErrorKind::EventLoop(e)))?;
    let scale_override = std::env::var("CAIRN_UI_SCALE_FACTOR")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s >= 0.5 && *s <= 4.0);
    let maximizable = config.maximizable;
    let mut runner = Runner {
        config,
        handler,
        window: None,
        error: None,
        shared: Shared {
            wakeup: None,
            scale_override,
            proxy: event_loop.create_proxy(),
            cursor: CursorIcon::Default,
            ime_allowed: false,
            ime_area: None,
            maximizable,
        },
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop
        .run_app(&mut runner)
        .map_err(|e| Error(ErrorKind::EventLoop(e)))?;
    match runner.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

struct Shared {
    wakeup: Option<Instant>,
    scale_override: Option<f64>,
    proxy: EventLoopProxy<UserPayload>,
    cursor: CursorIcon,
    ime_allowed: bool,
    ime_area: Option<PixelRect>,
    maximizable: bool,
}

struct WindowState {
    // 字段顺序即析构顺序：先放掉 surface，再放 softbuffer 上下文，最后是窗口。
    surface: Surface,
    _context: softbuffer::Context<Rc<Window>>,
    window: Rc<Window>,
}

struct Runner<H> {
    config: WindowConfig,
    handler: H,
    window: Option<WindowState>,
    shared: Shared,
    error: Option<Error>,
}

impl<H: Handler> Runner<H> {
    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<WindowState, Error> {
        let c = &self.config;
        let mut attrs = Window::default_attributes()
            .with_title(c.title.clone())
            .with_resizable(c.resizable)
            .with_decorations(c.decorations)
            // 先隐藏，画好第一帧再显示，避免创建时闪一下白底。
            .with_visible(false);
        match self.shared.scale_override {
            Some(s) => {
                attrs = attrs.with_inner_size(PhysicalSize::new(c.width * s, c.height * s));
                if let Some((w, h)) = c.min_size {
                    attrs = attrs.with_min_inner_size(PhysicalSize::new(w * s, h * s));
                }
                if let Some((w, h)) = c.max_size {
                    attrs = attrs.with_max_inner_size(PhysicalSize::new(w * s, h * s));
                }
            }
            None => {
                attrs = attrs.with_inner_size(LogicalSize::new(c.width, c.height));
                if let Some((w, h)) = c.min_size {
                    attrs = attrs.with_min_inner_size(LogicalSize::new(w, h));
                }
                if let Some((w, h)) = c.max_size {
                    attrs = attrs.with_max_inner_size(LogicalSize::new(w, h));
                }
            }
        }
        if !c.maximizable {
            attrs = attrs.with_enabled_buttons(winit::window::WindowButtons::CLOSE | winit::window::WindowButtons::MINIMIZE);
        }
        if let Some(icon) = &c.icon {
            if let Ok(winit_icon) = winit::window::Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height) {
                attrs = attrs.with_window_icon(Some(winit_icon));
            }
        }
        if !c.decorations {
            attrs = sys::undecorated(attrs);
        }
        let window = Rc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| Error(ErrorKind::Window(e)))?,
        );
        let context = softbuffer::Context::new(window.clone())?;
        let surface = Surface::new(softbuffer::Surface::new(&context, window.clone())?);
        Ok(WindowState {
            surface,
            _context: context,
            window,
        })
    }

    fn with_cx(&mut self, event_loop: &ActiveEventLoop, f: impl FnOnce(&mut H, &mut Context, &mut Surface)) {
        let Some(state) = self.window.as_mut() else { return };
        let mut cx = Context {
            window: &state.window,
            event_loop,
            shared: &mut self.shared,
        };
        f(&mut self.handler, &mut cx, &mut state.surface);
    }

    fn forward(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        self.with_cx(event_loop, |h, cx, _| h.event(cx, event));
    }
}

impl<H: Handler> ApplicationHandler<UserPayload> for Runner<H> {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: StartCause) {
        // 不只看 ResumeTimeReached：处理别的事件耗时较长时，定时点可能已经过了。
        if let Some(at) = self.shared.wakeup
            && Instant::now() >= at
        {
            self.shared.wakeup = None;
            self.with_cx(event_loop, |h, cx, _| h.timer(cx));
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match self.create_window(event_loop) {
            Ok(state) => self.window = Some(state),
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        }
        self.with_cx(event_loop, |h, cx, surface| {
            h.init(cx);
            h.redraw(cx, surface);
        });
        if let Some(state) = &self.window {
            state.window.set_visible(true);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, payload: UserPayload) {
        self.with_cx(event_loop, |h, cx, _| h.user_event(cx, payload));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let ev = match event {
            WindowEvent::RedrawRequested => {
                self.with_cx(event_loop, |h, cx, surface| h.redraw(cx, surface));
                return;
            }
            WindowEvent::Resized(size) => Event::Resized {
                width: size.width,
                height: size.height,
            },
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if self.shared.scale_override.is_some() {
                    return;
                }
                Event::ScaleFactorChanged { scale: scale_factor }
            }
            WindowEvent::CloseRequested => Event::CloseRequested,
            WindowEvent::Focused(f) => Event::WindowFocused(f),
            WindowEvent::CursorMoved { position, .. } => Event::PointerMoved {
                x: position.x,
                y: position.y,
            },
            WindowEvent::CursorLeft { .. } => Event::PointerLeft,
            WindowEvent::MouseInput { state, button, .. } => Event::PointerButton {
                button: convert::pointer_button(button),
                pressed: state == ElementState::Pressed,
            },
            WindowEvent::MouseWheel { delta, .. } => Event::Wheel(convert::scroll_delta(delta)),
            // 合成事件是窗口重新获得焦点时 winit 补发的“仍按着的键”，不应当作输入。
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => Event::Key(convert::key_event(&event)),
            WindowEvent::ModifiersChanged(m) => Event::ModifiersChanged(convert::modifiers(&m)),
            WindowEvent::Ime(ime) => Event::Ime(convert::ime(ime)),
            WindowEvent::HoveredFile(path) => Event::FileHovered(path),
            WindowEvent::HoveredFileCancelled => Event::FileHoverCancelled,
            WindowEvent::DroppedFile(path) => Event::FileDropped(path),
            _ => return,
        };
        self.forward(event_loop, ev);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(match self.shared.wakeup {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}
