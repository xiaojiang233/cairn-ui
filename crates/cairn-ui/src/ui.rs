//! 运行时核心：持有应用状态与控件树，处理输入、重建界面、布局、按脏区重绘、驱动动画帧。
//!
//! 不直接依赖窗口：平台服务通过 [`Host`] 注入，所以同一套逻辑也能在测试里无窗口运行。

use std::any::Any;
use std::time::{Duration, Instant};

use cairn_ui_platform::{self as platform, CursorIcon, Key, KeyEvent, Modifiers, NamedKey, PixelRect, PointerButton};

use crate::damage::{self, Damage};
use crate::element::El;
use crate::host::Host;
use crate::theme::Theme;
use crate::tree::{Tree, WidgetId};
use crate::widget::{Event, EventCx, EventOutput, LayoutCx, PaintCx};
use crate::{Background, Canvas, DeviceRect, Painter, Point, Rect, Size, TextSystem};

/// 重绘时脏区外扩的距离（逻辑像素），覆盖抗锯齿边缘和画在控件外面的焦点框。
const PAINT_MARGIN: f32 = 4.0;

pub(crate) type ViewBox<S> = Box<dyn Fn(&S) -> El<S>>;
pub(crate) type Update<S> = Box<dyn FnOnce(&mut S) + Send>;

/// 时间来源。真实窗口用系统时钟；测试用手动推进的虚拟时钟，动画结果可以精确复现。
#[derive(Clone, Copy, Debug)]
pub(crate) enum Clock {
    Real,
    Manual(Instant),
}

impl Clock {
    pub fn now(&self) -> Instant {
        match self {
            Clock::Real => Instant::now(),
            Clock::Manual(t) => *t,
        }
    }
}

pub(crate) struct Ui<S: 'static> {
    pub state: S,
    view: ViewBox<S>,
    pub tree: Tree<S>,
    pub text: TextSystem,
    theme: Theme,
    scale: f32,
    phys: (u32, u32),
    pub canvas: Canvas,
    damage: Damage,
    /// 指针下最深层的控件。
    hovered: Option<WidgetId>,
    /// 从 `hovered` 到根的路径：这些控件都处于“悬停”状态。
    hover_path: Vec<WidgetId>,
    captured: Option<WidgetId>,
    pub focused: Option<WidgetId>,
    focus_visible: bool,
    window_focused: bool,
    pointer: Option<Point>,
    modifiers: Modifiers,
    timers: Vec<(Instant, WidgetId)>,
    last_click: Option<(Instant, Point, u32)>,
    rebuild: bool,
    blink: Option<Duration>,
    double_click_time: Duration,
    double_click_distance: f32,
    pub clock: Clock,
    /// 上一帧请求了下一帧的控件（动画进行中）。
    animating: Vec<WidgetId>,
    /// 下一帧动画的时间点。
    next_frame: Option<Instant>,
}

impl<S: 'static> Ui<S> {
    pub fn new(
        state: S,
        view: ViewBox<S>,
        theme: Theme,
        text: TextSystem,
        scale: f32,
        phys: (u32, u32),
        clock: Clock,
    ) -> Self {
        let mut canvas = Canvas::new();
        canvas.set_background(match &theme.scene {
            Some(scene) => Background::Scene(scene.clone()),
            None => Background::Solid(theme.background),
        });
        canvas.set_backdrop_style(theme.backdrop);
        canvas.set_noise(theme.noise);
        let mut ui = Ui {
            state,
            view,
            tree: Tree::new(),
            text,
            theme,
            scale,
            phys,
            canvas,
            damage: Damage::default(),
            hovered: None,
            hover_path: Vec::new(),
            captured: None,
            focused: None,
            focus_visible: false,
            window_focused: true,
            pointer: None,
            modifiers: Modifiers::default(),
            timers: Vec::new(),
            last_click: None,
            rebuild: true,
            blink: platform::caret_blink_interval(),
            double_click_time: platform::double_click_interval(),
            double_click_distance: platform::double_click_distance() as f32,
            clock,
            animating: Vec::new(),
            next_frame: None,
        };
        ui.tree.set_viewport(ui.logical_size());
        ui.rebuild_view();
        ui.layout();
        ui.damage.add_full();
        ui
    }

    fn logical_size(&self) -> Size {
        Size::new(self.phys.0 as f32 / self.scale, self.phys.1 as f32 / self.scale)
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// 还有动画在进行（下一帧已经排上）。
    pub fn animating(&self) -> bool {
        self.next_frame.is_some()
    }

    pub fn next_frame(&self) -> Option<Instant> {
        self.next_frame
    }

    // ---------------------------------------------------------------- 构建与布局

    fn rebuild_view(&mut self) {
        self.rebuild = false;
        let el = (self.view)(&self.state);
        let mut cx = LayoutCx {
            text: &mut self.text,
            theme: &self.theme,
        };
        self.tree.set_content(&mut cx, el, &mut self.damage);
        let removed = std::mem::take(&mut self.tree.removed);
        if removed.is_empty() {
            return;
        }
        let gone = |id: &Option<WidgetId>| id.is_some_and(|id| removed.contains(&id));
        if gone(&self.hovered) {
            self.hovered = None;
        }
        if gone(&self.captured) {
            self.captured = None;
        }
        if gone(&self.focused) {
            self.focused = None;
        }
        self.hover_path.retain(|id| !removed.contains(id));
        self.animating.retain(|id| !removed.contains(id));
        self.timers.retain(|(_, id)| !removed.contains(id));
    }

    fn layout(&mut self) -> bool {
        let mut cx = LayoutCx {
            text: &mut self.text,
            theme: &self.theme,
        };
        self.tree.compute_layout(&mut cx, &mut self.damage)
    }

    fn damage_node(&mut self, id: WidgetId) {
        if let Some(n) = self.tree.nodes.get(id)
            && n.laid_out
        {
            self.damage.add(n.visual(&self.theme));
        }
    }

    // ---------------------------------------------------------------- 事件分发

    /// 把事件交给一个控件，并处理它在回调里提出的请求。返回是否已处理。
    fn send(&mut self, host: &mut dyn Host, id: WidgetId, event: &Event) -> bool {
        let Some(node) = self.tree.nodes.get_mut(id) else {
            return false;
        };
        let mut cx = EventCx {
            bounds: node.bounds,
            content: node.content(),
            state: &mut self.state,
            text: &mut self.text,
            theme: &self.theme,
            host: &mut *host,
            focused: self.focused == Some(id),
            window_focused: self.window_focused,
            modifiers: self.modifiers,
            scale: self.scale,
            blink: self.blink,
            now: self.clock.now(),
            out: EventOutput::default(),
        };
        node.widget.event(&mut cx, event);
        let out = cx.out;
        if out.change.layout {
            self.tree.layout.mark_dirty(node.layout);
            self.tree.needs_layout = true;
        }
        if out.change.paint && node.laid_out {
            self.damage.add(node.visual(&self.theme));
        }
        for r in out.damage {
            self.damage.add(r);
        }
        if out.app_touched {
            self.rebuild = true;
        }
        if let Some(at) = out.timer {
            self.timers.retain(|(_, w)| *w != id);
            self.timers.push((at, id));
        }
        if out.request_focus {
            let visible = self.focus_visible;
            self.set_focus(host, Some(id), visible);
        }
        out.handled
    }

    /// 从 `target` 开始分发，未处理就交给父控件（冒泡）。返回处理了事件的控件。
    fn dispatch(&mut self, host: &mut dyn Host, target: WidgetId, event: &Event, bubble: bool) -> Option<WidgetId> {
        let path = if bubble {
            self.tree.ancestors(target)
        } else {
            vec![target]
        };
        path.into_iter().find(|&id| self.send(host, id, event))
    }

    fn set_focus(&mut self, host: &mut dyn Host, new: Option<WidgetId>, visible: bool) {
        if self.focused == new {
            if self.focus_visible != visible {
                self.focus_visible = visible;
                if let Some(id) = new {
                    self.damage_node(id);
                }
            }
            return;
        }
        let old = std::mem::replace(&mut self.focused, new);
        self.focus_visible = visible;
        if let Some(o) = old {
            self.send(host, o, &Event::FocusLost);
            self.damage_node(o);
        }
        if let Some(n) = new {
            self.send(host, n, &Event::FocusGained);
            self.damage_node(n);
        }
    }

    fn focus_next(&mut self, host: &mut dyn Host, forward: bool) {
        let order: Vec<WidgetId> = self
            .tree
            .visible_in_order()
            .into_iter()
            .filter(|id| self.tree.nodes[*id].widget.focusable())
            .collect();
        if order.is_empty() {
            return;
        }
        let n = order.len();
        let next = match (self.focused.and_then(|f| order.iter().position(|x| *x == f)), forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        self.set_focus(host, Some(order[next]), true);
    }

    fn update_hover(&mut self, host: &mut dyn Host) {
        let new = self.pointer.and_then(|p| self.tree.hit_test(p, &self.theme));
        if new != self.hovered {
            self.hovered = new;
            let new_path = new.map(|h| self.tree.ancestors(h)).unwrap_or_default();
            let old_path = std::mem::replace(&mut self.hover_path, new_path.clone());
            // 是否需要重画由控件自己决定（有悬停效果的控件在 Enter / Leave 里请求重绘），
            // 否则鼠标划过大容器时会整块重画。离开从里往外发，进入从外往里发。
            for id in old_path.iter().filter(|id| !new_path.contains(id)) {
                self.send(host, *id, &Event::PointerLeave);
            }
            for id in new_path.iter().rev().filter(|id| !old_path.contains(id)) {
                self.send(host, *id, &Event::PointerEnter);
            }
        }
        let icon = self
            .captured
            .or(self.hovered)
            .and_then(|id| {
                self.tree
                    .ancestors(id)
                    .into_iter()
                    .map(|a| self.tree.nodes[a].widget.cursor())
                    .find(|c| *c != CursorIcon::Default)
            })
            .unwrap_or_default();
        host.set_cursor(icon);
    }

    fn pointer_button(&mut self, host: &mut dyn Host, button: PointerButton, pressed: bool) {
        let Some(pos) = self.pointer else { return };
        self.layout();
        if pressed {
            let now = self.clock.now();
            let count = match self.last_click {
                Some((t, p, c))
                    if now.duration_since(t) <= self.double_click_time
                        && (p.x - pos.x).abs() * self.scale <= self.double_click_distance
                        && (p.y - pos.y).abs() * self.scale <= self.double_click_distance =>
                {
                    c + 1
                }
                _ => 1,
            };
            self.last_click = Some((now, pos, count));
            let hit = self.tree.hit_test(pos, &self.theme);
            // 点到可获得焦点的控件（或其子元素）就把焦点给它，点到别处则清除焦点。
            let focus = hit.and_then(|h| {
                self.tree
                    .ancestors(h)
                    .into_iter()
                    .find(|id| self.tree.nodes[*id].widget.focusable())
            });
            self.set_focus(host, focus, false);
            if let Some(h) = hit {
                // 处理了按下事件的控件在松开前独占指针事件（拖选文字、按钮按住后移出等）。
                self.captured = self.dispatch(host, h, &Event::PointerDown { pos, button, count }, true);
            }
        } else {
            let target = self.captured.take().or_else(|| self.tree.hit_test(pos, &self.theme));
            if let Some(t) = target {
                self.dispatch(host, t, &Event::PointerUp { pos, button }, true);
            }
            self.update_hover(host);
        }
    }

    fn key(&mut self, host: &mut dyn Host, key: KeyEvent) {
        let tab = key.pressed && key.key == Key::Named(NamedKey::Tab);
        let handled = match self.focused {
            Some(f) => self.dispatch(host, f, &Event::Key(key), true).is_some(),
            None => false,
        };
        if tab && !handled {
            let forward = !self.modifiers.shift;
            self.focus_next(host, forward);
        }
    }

    pub fn handle(&mut self, host: &mut dyn Host, event: platform::Event) {
        use platform::Event as E;
        match event {
            E::Resized { width, height } => self.set_size(width, height, self.scale),
            E::ScaleFactorChanged { scale } => {
                let (w, h) = self.phys;
                self.set_size(w, h, scale as f32);
            }
            E::CloseRequested => host.exit(),
            E::WindowFocused(f) => {
                self.window_focused = f;
                if !f {
                    self.captured = None;
                }
                if let Some(id) = self.focused {
                    self.send(host, id, &Event::WindowFocusChanged(f));
                    self.damage_node(id);
                }
            }
            E::PointerMoved { x, y } => {
                let pos = Point::new(x as f32 / self.scale, y as f32 / self.scale);
                self.pointer = Some(pos);
                self.layout();
                self.update_hover(host);
                if let Some(t) = self.captured.or(self.hovered) {
                    self.dispatch(host, t, &Event::PointerMove { pos }, true);
                }
            }
            E::PointerLeft => {
                self.pointer = None;
                self.update_hover(host);
            }
            E::PointerButton { button, pressed } => self.pointer_button(host, button, pressed),
            E::Wheel(delta) => {
                if let Some(h) = self.hovered {
                    self.dispatch(host, h, &Event::Wheel { delta }, true);
                }
            }
            E::Key(k) => self.key(host, k),
            E::ModifiersChanged(m) => self.modifiers = m,
            E::Ime(ime) => {
                if let Some(f) = self.focused {
                    self.dispatch(host, f, &Event::Ime(ime), false);
                }
            }
        }
        self.after(host);
    }

    pub fn set_size(&mut self, width: u32, height: u32, scale: f32) {
        if scale != self.scale {
            // 字形按物理尺寸缓存，旧缩放下的字形不会再用到。
            self.text.clear_glyph_cache();
        }
        self.phys = (width, height);
        self.scale = scale;
        self.tree.set_viewport(self.logical_size());
        self.damage.add_full();
    }

    /// 定时器或动画帧的时间到了。
    pub fn fire_timers(&mut self, host: &mut dyn Host, now: Instant) {
        if self.next_frame.is_some_and(|f| f <= now) {
            self.next_frame = None;
            for id in std::mem::take(&mut self.animating) {
                self.damage_node(id);
            }
        }
        let due: Vec<WidgetId> = self
            .timers
            .iter()
            .filter(|(t, _)| *t <= now)
            .map(|(_, id)| *id)
            .collect();
        self.timers.retain(|(t, _)| *t > now);
        for id in due {
            self.send(host, id, &Event::Timer);
        }
        self.after(host);
    }

    pub fn user_event(&mut self, host: &mut dyn Host, payload: Box<dyn Any + Send>) {
        if let Ok(update) = payload.downcast::<Update<S>>() {
            update(&mut self.state);
            self.rebuild = true;
        }
        self.after(host);
    }

    /// 每批事件处理完之后：按需重建界面、重新布局，同步输入法与定时器，有脏区就请求重绘。
    pub fn after(&mut self, host: &mut dyn Host) {
        // 重建和布局本身也可能触发事件（悬停变化），最多来回几轮。
        for _ in 0..4 {
            if self.rebuild {
                self.rebuild_view();
            }
            if self.layout() && self.pointer.is_some() {
                self.update_hover(host);
            }
            if !self.rebuild {
                break;
            }
        }
        self.sync_ime(host);
        self.schedule(host);
        if !self.damage.is_empty() {
            host.request_redraw();
        }
    }

    /// 告诉宿主下一次需要醒来的时间：最近的定时器或下一帧动画。
    fn schedule(&mut self, host: &mut dyn Host) {
        let timer = self.timers.iter().map(|(t, _)| *t).min();
        let wake = match (timer, self.next_frame) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        host.set_wakeup(wake);
    }

    fn sync_ime(&mut self, host: &mut dyn Host) {
        let target = self
            .focused
            .filter(|f| self.tree.nodes.get(*f).is_some_and(|n| n.widget.wants_ime()));
        host.set_ime_allowed(target.is_some());
        let Some(id) = target else { return };
        let node = &self.tree.nodes[id];
        if let Some(area) = node.widget.ime_area(node.bounds, node.content()) {
            let s = self.scale;
            let l = (area.x * s).floor().max(0.0);
            let t = (area.y * s).floor().max(0.0);
            let r = (area.right() * s).ceil().max(l + 1.0);
            let b = (area.bottom() * s).ceil().max(t + 1.0);
            host.set_ime_cursor_area(PixelRect {
                x: l as u32,
                y: t as u32,
                width: (r - l) as u32,
                height: (b - t) as u32,
            });
        }
    }

    // ---------------------------------------------------------------- 绘制

    /// 重画所有脏区，返回本帧更新过的区域（物理像素）。没有脏区时返回空列表。
    /// 画的过程中有控件请求了下一帧，就按显示器刷新间隔排上。
    pub fn render(&mut self, host: &mut dyn Host) -> Vec<DeviceRect> {
        let (w, h) = self.phys;
        if w == 0 || h == 0 {
            return Vec::new();
        }
        if self.canvas.resize(w, h) {
            self.damage.add_full();
        }
        if self.rebuild {
            self.rebuild_view();
        }
        self.layout();
        let set = self.damage.take();
        let bounds = DeviceRect::new(0, 0, w, h);
        let regions = if set.full {
            vec![bounds]
        } else {
            damage::to_device(&set.rects, self.scale, PAINT_MARGIN, bounds)
        };
        let now = self.clock.now();
        for region in &regions {
            self.paint_region(*region, now);
        }
        if !self.animating.is_empty() && self.next_frame.is_none() {
            self.next_frame = Some(now + host.frame_interval());
        }
        self.schedule(host);
        regions
    }

    fn paint_region(&mut self, region: DeviceRect, now: Instant) {
        let s = self.scale;
        let area = Rect::new(
            region.x as f32 / s,
            region.y as f32 / s,
            region.width as f32 / s,
            region.height as f32 / s,
        );
        let Ui {
            canvas,
            text,
            tree,
            theme,
            hover_path,
            focused,
            focus_visible,
            window_focused,
            animating,
            ..
        } = self;
        let mut st = PaintState {
            tree,
            theme,
            hover_path,
            focused: *focused,
            focus_visible: *focus_visible,
            window_focused: *window_focused,
            now,
            area,
            animating,
        };
        canvas.paint_region(region, s, text, |p| {
            let root = st.tree.root;
            paint_node(&mut st, root, p);
        });
    }
}

struct PaintState<'a, S: 'static> {
    tree: &'a mut Tree<S>,
    theme: &'a Theme,
    hover_path: &'a [WidgetId],
    focused: Option<WidgetId>,
    focus_visible: bool,
    window_focused: bool,
    now: Instant,
    /// 本次重画的区域（逻辑像素）。
    area: Rect,
    animating: &'a mut Vec<WidgetId>,
}

/// 按树的结构递归绘制：变换、裁剪作用于整棵子树。
fn paint_node<S: 'static>(st: &mut PaintState<S>, id: WidgetId, p: &mut dyn Painter) {
    let theme = st.theme;
    let node = &mut st.tree.nodes[id];
    if node.style.hidden {
        return;
    }
    let visible = node.laid_out
        && node
            .visual(theme)
            .inflate(PAINT_MARGIN, PAINT_MARGIN)
            .intersects(&st.area);
    let mut cx = PaintCx {
        bounds: node.bounds,
        content: node.content(),
        hovered: st.hover_path.contains(&id),
        focused: st.focused == Some(id),
        focus_visible: st.focus_visible,
        window_focused: st.window_focused,
        theme,
        now: st.now,
        frame_requested: false,
    };
    node.widget.animate(&mut cx);
    let transform = node.widget.transform(&cx);
    let transformed = !transform.is_identity();
    if transformed {
        p.push_transform(transform);
    }
    if visible {
        node.widget.paint(&mut cx, p);
    }
    let clip = node.widget.clip_children(theme);
    let bounds = node.bounds;
    // 裁剪子元素的控件本身不在重画区域内时，子元素也不可能在。
    if clip.is_none() || visible {
        if let Some(radius) = clip {
            p.push_rounded_clip(bounds, radius);
        }
        for i in 0..st.tree.nodes[id].children.len() {
            let child = st.tree.nodes[id].children[i];
            paint_node(st, child, p);
        }
        if clip.is_some() {
            p.pop_clip();
        }
    }
    if visible {
        st.tree.nodes[id].widget.paint_over(&mut cx, p);
    }
    if transformed {
        p.pop_transform();
    }
    if cx.frame_requested && !st.animating.contains(&id) {
        st.animating.push(id);
    }
}
