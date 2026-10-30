use std::any::Any;
use std::marker::PhantomData;

use cairn_ui_platform::{
    self as platform, Blur, Context, Frame, Handler, PixelRect, SceneImage, Surface, WindowConfig,
};

use crate::element::{El, View};
use crate::host::PlatformHost;
use crate::theme::Theme;
use crate::ui::{Clock, FileDropFn, ThemeFn, Ui, Update, ViewBox};
use crate::{Background, Error, FontSetup, Result, TextSystem};

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

pub struct App<S: 'static> {
    view: ViewBox<S>,
    window: WindowConfig,
    theme: Theme,
    file_drop: Option<FileDropFn<S>>,
    theme_source: Option<ThemeFn<S>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FileDrop {
    Hovered(std::path::PathBuf),
    Cancelled,
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

    pub fn decorations(mut self, decorations: bool) -> Self {
        self.window.decorations = decorations;
        self
    }

    pub fn icon(mut self, rgba: Vec<u8>, width: u32, height: u32) -> Self {
        self.window.icon = Some(platform::IconData::new(rgba, width, height));
        self
    }

    pub fn on_file_drop(mut self, f: impl Fn(&mut S, FileDrop) + 'static) -> Self {
        self.file_drop = Some(Box::new(f));
        self
    }

    pub fn theme_from(mut self, f: impl Fn(&mut S) -> Option<Theme> + 'static) -> Self {
        self.theme_source = Some(Box::new(f));
        self
    }

    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn run(self, state: S) -> Result<()> {
        self.run_with(move |_| state)
    }

    pub fn run_with(self, init: impl FnOnce(Proxy<S>) -> S + 'static) -> Result<()> {
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
        ui.set_compositing(surface.composites());
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
        let canvas = &ui.canvas;
        let style = canvas.backdrop_style();
        let (passes, spread) = style.kernel(ui.scale());
        let frame = Frame {
            width: canvas.width(),
            height: canvas.height(),
            rgba: canvas.pixels(),
            damage: &damage,
            glass: canvas.glass(),
            scene: canvas
                .scene_image()
                .map(|(rgba, width, height, generation)| SceneImage {
                    rgba,
                    width,
                    height,
                    generation,
                }),
            background: match canvas.background() {
                Background::Solid(c) => [c.r, c.g, c.b, c.a],
                Background::Scene(_) => [0, 0, 0, 255],
            },
            blur: Blur {
                downsample: style.downsample,
                passes,
                spread,
                saturation: style.saturation,
                brightness: style.brightness(),
            },
            noise: canvas.noise(),
        };
        if let Err(e) = surface.present(&frame) {
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
