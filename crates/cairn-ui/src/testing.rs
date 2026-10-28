//! 不开窗口驱动界面，用于自动化测试与截图检查。
//!
//! 和真实窗口走的是同一套运行时（控件树、比对、布局、事件分发、脏区重绘），
//! 只是平台服务换成了记录型的假实现：输入法开关、候选框位置、剪贴板等都可以直接读出来检查。
//! 字体用的是系统字体（与真实运行一致），所以结果依赖当前系统安装的字体。

use std::time::{Duration, Instant};

use cairn_ui_platform::{self as platform, ImeEvent, Key, KeyEvent, Modifiers, PixelRect, PointerButton, ScrollDelta};

use crate::app::{ViewFn, erase_view, system_fonts};
use crate::host::{FRAME_INTERVAL, RecordingHost};
use crate::theme::Theme;
use crate::tree::WidgetId;
use crate::ui::{Clock, Ui};
use crate::widget::{Role, Semantics};
use crate::{DeviceRect, Point, Rect, TextSystem};

/// 当前系统的字体加载参数（与真实窗口运行时相同）。
pub fn system_font_setup() -> crate::FontSetup {
    system_fonts()
}

/// 控件树里一个节点的可观察信息。
#[derive(Clone, Debug, PartialEq)]
pub struct NodeInfo {
    pub id: WidgetId,
    pub semantics: Semantics,
    /// 边框盒，窗口坐标（逻辑像素）。
    pub bounds: Rect,
    pub focused: bool,
}

/// 无窗口运行的界面。
///
/// 时间是虚拟的：从创建时刻开始，只有 [`Headless::advance`]、[`Headless::fire_timers`]
/// 会推进它，所以动画的中间状态可以精确复现。动画帧间隔按 60 Hz 计。
pub struct Headless<S: 'static> {
    ui: Ui<S>,
    host: RecordingHost,
}

impl<S: 'static> Headless<S> {
    /// `width`、`height` 是逻辑像素，`scale` 是缩放因子（1.25 即 125%）。
    pub fn new<F>(state: S, view: F, width: f32, height: f32, scale: f32) -> Self
    where
        F: for<'a> ViewFn<'a, S>,
    {
        Self::with_text_system(state, view, width, height, scale, TextSystem::new(&system_fonts()))
    }

    /// 复用已经加载好的字体库（加载系统字体比较慢，多个测试可以共用一个）。
    pub fn with_text_system<F>(state: S, view: F, width: f32, height: f32, scale: f32, text: TextSystem) -> Self
    where
        F: for<'a> ViewFn<'a, S>,
    {
        Self::with_theme(state, view, width, height, scale, Theme::default(), text)
    }

    /// 指定主题。
    pub fn with_theme<F>(state: S, view: F, width: f32, height: f32, scale: f32, theme: Theme, text: TextSystem) -> Self
    where
        F: for<'a> ViewFn<'a, S>,
    {
        let phys = ((width * scale).round() as u32, (height * scale).round() as u32);
        let clock = Clock::Manual(Instant::now());
        let mut ui = Ui::new(state, erase_view(view), theme, text, scale, phys, clock);
        let mut host = RecordingHost::default();
        ui.after(&mut host);
        Headless { ui, host }
    }

    /// 取回字体库，给下一个测试用。
    pub fn into_text_system(self) -> TextSystem {
        self.ui.text
    }

    pub fn state(&self) -> &S {
        &self.ui.state
    }

    /// 像后台线程通过 [`crate::Proxy::update`] 那样修改状态。
    pub fn update(&mut self, f: impl FnOnce(&mut S) + Send + 'static) {
        let f: crate::ui::Update<S> = Box::new(f);
        self.ui.user_event(&mut self.host, Box::new(f));
    }

    /// 重画脏区，返回本帧更新的区域（物理像素）。
    pub fn render(&mut self) -> Vec<DeviceRect> {
        self.host.redraw_requested = false;
        self.ui.render(&mut self.host)
    }

    /// 虚拟时钟的当前时间。
    pub fn now(&self) -> Instant {
        self.ui.clock.now()
    }

    /// 把虚拟时钟往前拨 `d`，触发期间到期的定时器和动画帧（不自动重画）。
    pub fn advance(&mut self, d: Duration) {
        let now = self.now() + d;
        self.fire_timers(now);
    }

    /// 还有动画在进行（已经排上了下一帧）。
    pub fn animating(&self) -> bool {
        self.ui.animating()
    }

    /// 一帧一帧推进并重画，直到动画全部结束（最多 `max_frames` 帧）。返回每帧重画的区域。
    pub fn run_animations(&mut self, max_frames: usize) -> Vec<Vec<DeviceRect>> {
        let mut frames = Vec::new();
        while frames.len() < max_frames {
            let Some(at) = self.ui.next_frame() else { break };
            self.fire_timers(at.max(self.now()));
            frames.push(self.render());
        }
        frames
    }

    /// 画布像素：RGBA8，`width() * height()` 个。
    pub fn pixels(&self) -> &[u8] {
        self.ui.canvas.pixels()
    }

    pub fn width(&self) -> u32 {
        self.ui.canvas.width()
    }

    pub fn height(&self) -> u32 {
        self.ui.canvas.height()
    }

    /// 当前画布编码成 PNG。
    pub fn png(&self) -> Vec<u8> {
        encode_png(self.pixels(), self.width(), self.height())
    }

    /// 自上次 [`Headless::render`] 以来是否请求过重绘。
    pub fn redraw_requested(&self) -> bool {
        self.host.redraw_requested
    }

    /// 所有可见节点，按 Tab 顺序（深度优先，先父后子）。
    pub fn nodes(&self) -> Vec<NodeInfo> {
        let tree = &self.ui.tree;
        tree.visible_in_order()
            .into_iter()
            .map(|id| {
                let n = &tree.nodes[id];
                NodeInfo {
                    id,
                    semantics: n.widget.semantics(),
                    bounds: n.bounds,
                    focused: self.ui.focused == Some(id),
                }
            })
            .collect()
    }

    /// 第一个名称或值等于 `text` 的节点。
    pub fn find(&self, text: &str) -> Option<NodeInfo> {
        self.nodes()
            .into_iter()
            .find(|n| n.semantics.label.as_deref() == Some(text) || n.semantics.value.as_deref() == Some(text))
    }

    pub fn find_role(&self, role: Role) -> Vec<NodeInfo> {
        self.nodes().into_iter().filter(|n| n.semantics.role == role).collect()
    }

    fn send(&mut self, e: platform::Event) {
        self.ui.handle(&mut self.host, e);
    }

    pub fn pointer_move(&mut self, p: Point) {
        let s = self.ui.scale() as f64;
        self.send(platform::Event::PointerMoved {
            x: p.x as f64 * s,
            y: p.y as f64 * s,
        });
    }

    pub fn pointer_leave(&mut self) {
        self.send(platform::Event::PointerLeft);
    }

    pub fn pointer_button(&mut self, button: PointerButton, pressed: bool) {
        self.send(platform::Event::PointerButton { button, pressed });
    }

    /// 移动到 `p` 并单击左键。
    pub fn click(&mut self, p: Point) {
        self.pointer_move(p);
        self.pointer_button(PointerButton::Left, true);
        self.pointer_button(PointerButton::Left, false);
    }

    /// 滚轮（作用在指针当前悬停的控件上，未处理就冒泡）。`Lines` 的 `y` 为正是向上滚，与 winit 一致；
    /// `Pixels` 是物理像素。
    pub fn wheel(&mut self, delta: ScrollDelta) {
        self.send(platform::Event::Wheel(delta));
    }

    pub fn set_modifiers(&mut self, m: Modifiers) {
        self.send(platform::Event::ModifiersChanged(m));
    }

    /// 按下再松开一个键。
    pub fn key(&mut self, key: Key, text: Option<&str>) {
        let ev = |pressed| KeyEvent {
            key: key.clone(),
            text: if pressed { text.map(str::to_string) } else { None },
            pressed,
            repeat: false,
        };
        self.send(platform::Event::Key(ev(true)));
        self.send(platform::Event::Key(ev(false)));
    }

    /// 逐个字符模拟键盘输入（不经过输入法）。
    pub fn type_text(&mut self, s: &str) {
        for c in s.chars() {
            let t = c.to_string();
            self.key(Key::Character(t.clone()), Some(&t));
        }
    }

    pub fn ime(&mut self, ev: ImeEvent) {
        self.send(platform::Event::Ime(ev));
    }

    pub fn ime_allowed(&self) -> bool {
        self.host.ime_allowed
    }

    /// 最近一次设置的输入法候选框位置（物理像素）。
    pub fn ime_area(&self) -> Option<PixelRect> {
        self.host.ime_area
    }

    pub fn clipboard(&self) -> Option<&str> {
        self.host.clipboard.as_deref()
    }

    pub fn set_clipboard(&mut self, text: &str) {
        self.host.clipboard = Some(text.to_string());
    }

    /// 最近的定时器时间点（例如光标闪烁）。
    pub fn next_timer(&self) -> Option<Instant> {
        self.host.wakeup
    }

    /// 把虚拟时钟拨到 `now`（不会往回拨），触发到期的定时器和动画帧。
    pub fn fire_timers(&mut self, now: Instant) {
        let now = now.max(self.now());
        self.ui.clock = Clock::Manual(now);
        self.ui.fire_timers(&mut self.host, now);
    }

    /// 动画帧间隔（60 Hz）。
    pub fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    pub fn set_window_focused(&mut self, focused: bool) {
        self.send(platform::Event::WindowFocused(focused));
    }

    /// 改变窗口尺寸（逻辑像素）与缩放因子，相当于拖到另一块显示器上。
    pub fn resize(&mut self, width: f32, height: f32, scale: f32) {
        let (w, h) = ((width * scale).round() as u32, (height * scale).round() as u32);
        self.send(platform::Event::ScaleFactorChanged { scale: scale as f64 });
        self.send(platform::Event::Resized { width: w, height: h });
    }
}

/// 把 RGBA8（预乘 alpha）像素编码成 PNG。不压缩，只用于调试截图。
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);
    for row in rgba.chunks_exact(w * 4).take(h) {
        raw.push(0); // 不做行过滤
        for px in row.chunks_exact(4) {
            let a = px[3] as u32;
            if a == 255 || a == 0 {
                raw.extend_from_slice(px);
            } else {
                // 预乘 → 非预乘
                raw.extend(px[..3].iter().map(|&c| ((c as u32 * 255 + a / 2) / a).min(255) as u8));
                raw.push(px[3]);
            }
        }
    }
    // zlib：存储块（不压缩），每块最多 65535 字节。
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    if chunks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(c) = chunks.next() {
        z.push(chunks.peek().is_none() as u8);
        let len = c.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 位、RGBA
    for (kind, data) in [(b"IHDR", &ihdr[..]), (b"IDAT", &z[..]), (b"IEND", &[][..])] {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    }
    png
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
