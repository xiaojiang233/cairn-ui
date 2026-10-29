//! 应用入口：[`App`] 构建器、跨线程的 [`Proxy`]，以及接到平台层的适配。

use std::any::Any;
use std::marker::PhantomData;

use cairn_ui_platform::{self as platform, Context, Handler, PixelRect, Surface, WindowConfig};

use crate::element::{El, View};
use crate::host::PlatformHost;
use crate::theme::Theme;
use crate::ui::{Clock, FileDropFn, ThemeFn, Ui, Update, ViewBox};
use crate::{Error, FontSetup, Result, TextSystem};

/// “从状态生成界面”的函数，通常直接传 `Self::view` 这样的方法。
///
/// 这个 trait 只是为了让 `fn view(&self) -> impl View<Self>` 在 Rust 2024 里也能用
/// （返回类型会捕获 `&self` 的生命周期），一般不需要手动实现。
pub trait ViewFn<'a, S: 'static>: 'static {
    type Output: View<S>;
    fn call(&self, state: &'a S) -> Self::Output;
}

impl<'a, S: 'static, F, V> ViewFn<'a, S> for F
where
    F: Fn(&'a S) -> V + 'static,
    V: View<S>,
{
    type Output = V;
    fn call(&self, state: &'a S) -> V {
        self(state)
    }
}

pub(crate) fn erase_view<S: 'static, F>(view: F) -> ViewBox<S>
where
    F: for<'a> ViewFn<'a, S>,
{
    Box::new(move |s: &S| -> El<S> { view.call(s).into() })
}

/// 系统字体配置（由平台层提供）转成渲染层的字体加载参数。
pub(crate) fn system_fonts() -> FontSetup {
    let c = platform::font_config();
    FontSetup {
        dirs: c.dirs,
        sans_serif: c.families.sans_serif,
        serif: c.families.serif,
        monospace: c.families.monospace,
        locale: c.locale,
    }
}

/// 应用构建器。
///
/// ```no_run
/// use cairn_ui::prelude::*;
///
/// #[derive(Default)]
/// struct Counter { n: i32 }
///
/// impl Counter {
///     fn view(&self) -> impl View<Self> {
///         column()
///             .padding(16.0)
///             .gap(8.0)
///             .child(label(format!("n = {}", self.n)))
///             .child(button("+1").on_click(|app: &mut Self| app.n += 1))
///     }
/// }
///
/// fn main() -> cairn_ui::Result<()> {
///     App::new(Counter::view).title("计数器").run(Counter::default())
/// }
/// ```
pub struct App<S: 'static> {
    view: ViewBox<S>,
    window: WindowConfig,
    theme: Theme,
    file_drop: Option<FileDropFn<S>>,
    theme_source: Option<ThemeFn<S>>,
}

/// 拖放文件的进展，见 [`App::on_file_drop`]。拖入多个文件时，每个文件各有一个 `Hovered` / `Dropped`。
#[derive(Clone, Debug, PartialEq)]
pub enum FileDrop {
    /// 文件被拖到窗口上方，还没松开。
    Hovered(std::path::PathBuf),
    /// 拖到窗口上方的文件又被拖走了。
    Cancelled,
    /// 文件被放进窗口。
    Dropped(std::path::PathBuf),
}

impl<S: 'static> App<S> {
    pub fn new<F>(view: F) -> Self
    where
        F: for<'a> ViewFn<'a, S>,
    {
        App {
            view: erase_view(view),
            window: WindowConfig::default(),
            theme: Theme::default(),
            file_drop: None,
            theme_source: None,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.window.title = title.into();
        self
    }

    /// 窗口客户区大小（逻辑像素）。
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.window.width = width as f64;
        self.window.height = height as f64;
        self
    }

    pub fn min_size(mut self, width: f32, height: f32) -> Self {
        self.window.min_size = Some((width as f64, height as f64));
        self
    }

    pub fn max_size(mut self, width: f32, height: f32) -> Self {
        self.window.max_size = Some((width as f64, height as f64));
        self
    }

    pub fn maximizable(mut self, maximizable: bool) -> Self {
        self.window.maximizable = maximizable;
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.window.resizable = resizable;
        self
    }

    /// 是否使用系统标题栏和边框，默认使用。
    ///
    /// 关闭后窗口内容铺满整个窗口：标题栏自己画，用 [`crate::drag_area`] 做可拖动的区域、
    /// [`crate::window_controls`] 做最小化 / 最大化 / 关闭按钮；可调整大小时，窗口四周 6 像素宽的一圈
    /// 由框架处理成拖动改变大小。Windows 上保留系统阴影，Windows 11 上是系统圆角。
    pub fn decorations(mut self, decorations: bool) -> Self {
        self.window.decorations = decorations;
        self
    }

    /// 文件被拖到窗口上方、拖走、放下时调用，回调里修改状态后界面自动重建。
    /// 不设置时窗口不处理拖放。
    pub fn on_file_drop(mut self, f: impl Fn(&mut S, FileDrop) + 'static) -> Self {
        self.file_drop = Some(Box::new(f));
        self
    }

    /// 运行中换主题（例如深浅模式、主题色、壁纸由用户设置）：每次状态变化后调用 `f`，
    /// 返回 `Some` 时换上新主题并按新主题重建整个界面。通常在状态里放一个 `Option<Theme>`，
    /// 回调里 `take()` 出来：`.theme_from(|app: &mut Self| app.new_theme.take())`。
    /// 换主题时所有控件重新测量、整窗重画（背景场景也重新生成），不要每帧都换。
    pub fn theme_from(mut self, f: impl Fn(&mut S) -> Option<Theme> + 'static) -> Self {
        self.theme_source = Some(Box::new(f));
        self
    }

    /// 外观，默认 [`Theme::light`]。用户在系统里关闭了动画时，主题里的过渡时长在启动时被清零。
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// 打开窗口并运行，直到窗口关闭。必须在主线程调用。
    pub fn run(self, state: S) -> Result<()> {
        self.run_with(move |_| state)
    }

    /// 同 [`App::run`]，但状态在窗口创建后才构造，可以拿到 [`Proxy`] 存进状态里，
    /// 供后台线程（下载、读文件……）回头更新界面。
    pub fn run_with(self, init: impl FnOnce(Proxy<S>) -> S + 'static) -> Result<()> {
        // 扫描系统字体要几十到上百毫秒，与创建窗口并行做。
        let fonts = std::thread::spawn(|| TextSystem::new(&system_fonts()));
        let resize_border = !self.window.decorations && self.window.resizable;
        let handler = AppHandler {
            pending: Some(Pending {
                view: self.view,
                theme: self.theme,
                init: Box::new(init),
                fonts,
                resize_border,
                file_drop: self.file_drop,
                theme_source: self.theme_source,
            }),
            ui: None,
        };
        platform::run(self.window, handler).map_err(Error::from)
    }
}

/// 从其他线程更新应用状态。可以 `clone`，可以发送到别的线程。
pub struct Proxy<S> {
    inner: platform::Proxy,
    _state: PhantomData<fn(S)>,
}

impl<S> Clone for Proxy<S> {
    fn clone(&self) -> Self {
        Proxy {
            inner: self.inner.clone(),
            _state: PhantomData,
        }
    }
}

impl<S: 'static> Proxy<S> {
    /// 在主线程上用 `f` 修改应用状态，随后界面自动重建。窗口已关闭时返回 `false`。
    pub fn update(&self, f: impl FnOnce(&mut S) + Send + 'static) -> bool {
        let f: Update<S> = Box::new(f);
        self.inner.send(Box::new(f))
    }
}

struct Pending<S: 'static> {
    view: ViewBox<S>,
    theme: Theme,
    init: Box<dyn FnOnce(Proxy<S>) -> S>,
    fonts: std::thread::JoinHandle<TextSystem>,
    resize_border: bool,
    file_drop: Option<FileDropFn<S>>,
    theme_source: Option<ThemeFn<S>>,
}

struct AppHandler<S: 'static> {
    pending: Option<Pending<S>>,
    ui: Option<Ui<S>>,
}

impl<S: 'static> Handler for AppHandler<S> {
    fn init(&mut self, cx: &mut Context) {
        let Some(p) = self.pending.take() else { return };
        let text = p.fonts.join().unwrap_or_else(|_| TextSystem::new(&system_fonts()));
        let state = (p.init)(Proxy {
            inner: cx.proxy(),
            _state: PhantomData,
        });
        let mut theme = p.theme;
        let reduce_motion = platform::reduce_motion();
        if reduce_motion {
            theme.motion = theme.motion.reduced();
        }
        let mut ui = Ui::new(
            state,
            p.view,
            theme,
            text,
            cx.scale_factor() as f32,
            cx.physical_size(),
            Clock::Real,
        );
        ui.set_window_frame(p.resize_border, cx.is_maximized());
        ui.set_file_drop(p.file_drop);
        ui.set_theme_source(p.theme_source, reduce_motion);
        ui.after(&mut PlatformHost(cx));
        self.ui = Some(ui);
    }

    fn event(&mut self, cx: &mut Context, event: platform::Event) {
        if let Some(ui) = &mut self.ui {
            ui.handle(&mut PlatformHost(cx), event);
        }
    }

    fn redraw(&mut self, cx: &mut Context, surface: &mut Surface) {
        let Some(ui) = &mut self.ui else { return };
        let (w, h) = cx.physical_size();
        if (w, h) != (ui.canvas.width(), ui.canvas.height()) {
            let scale = ui.scale();
            ui.set_size(w, h, scale);
        }
        let regions = ui.render(&mut PlatformHost(cx));
        let damage: Vec<PixelRect> = regions
            .iter()
            .map(|r| PixelRect {
                x: r.x.max(0) as u32,
                y: r.y.max(0) as u32,
                width: r.width,
                height: r.height,
            })
            .collect();
        if let Err(e) = surface.present(ui.canvas.pixels(), ui.canvas.width(), ui.canvas.height(), &damage) {
            eprintln!("cairn-ui: {e}");
        }
    }

    fn timer(&mut self, cx: &mut Context) {
        if let Some(ui) = &mut self.ui {
            ui.fire_timers(&mut PlatformHost(cx), std::time::Instant::now());
        }
    }

    fn user_event(&mut self, cx: &mut Context, payload: Box<dyn Any + Send>) {
        if let Some(ui) = &mut self.ui {
            ui.user_event(&mut PlatformHost(cx), payload);
        }
    }
}
