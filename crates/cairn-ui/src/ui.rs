//! 运行时核心：持有应用状态与控件树，处理输入、重建界面、布局、按脏区重绘。
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
use crate::{Canvas, DeviceRect, Point, Rect, Size, TextSystem};

/// 重绘时脏区外扩的距离（逻辑像素），覆盖抗锯齿边缘和画在控件外面的焦点框。
const PAINT_MARGIN: f32 = 4.0;

pub(crate) type ViewBox<S> = Box<dyn Fn(&S) -> El<S>>;
pub(crate) type Update<S> = Box<dyn FnOnce(&mut S) + Send>;

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
    hovered: Option<WidgetId>,
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
}

impl<S: 'static> Ui<S> {
    pub fn new(state: S, view: ViewBox<S>, theme: Theme, text: TextSystem, scale: f32, phys: (u32, u32)) -> Self {
        let mut ui = Ui {
            state,
            view,
            tree: Tree::new(),
            text,
            theme,
            scale,
            phys,
            canvas: Canvas::new(),
            damage: Damage::default(),
            hovered: None,
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
            self.damage.add(n.bounds);
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
            out: EventOutput::default(),
        };
        node.widget.event(&mut cx, event);
        let out = cx.out;
        if out.change.layout {
            self.tree.layout.mark_dirty(node.layout);
            self.tree.needs_layout = true;
        }
        if out.change.paint && node.laid_out {
            self.damage.add(node.bounds);
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
        let new = self.pointer.and_then(|p| self.tree.hit_test(p));
        if new != self.hovered {
            let old = std::mem::replace(&mut self.hovered, new);
            // 是否需要重画由控件自己决定（有悬停效果的控件在 Enter / Leave 里请求重绘），
            // 否则鼠标划过大容器时会整块重画。
            if let Some(o) = old {
                self.send(host, o, &Event::PointerLeave);
            }
            if let Some(n) = new {
                self.send(host, n, &Event::PointerEnter);
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
            let now = Instant::now();
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
            let hit = self.tree.hit_test(pos);
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
            let target = self.captured.take().or_else(|| self.tree.hit_test(pos));
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

    pub fn fire_timers(&mut self, host: &mut dyn Host, now: Instant) {
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
        host.set_wakeup(self.timers.iter().map(|(t, _)| *t).min());
        if !self.damage.is_empty() {
            host.request_redraw();
        }
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
    pub fn render(&mut self) -> Vec<DeviceRect> {
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
        for region in &regions {
            self.paint_region(*region);
        }
        regions
    }

    fn paint_region(&mut self, region: DeviceRect) {
        let s = self.scale;
        let area = Rect::new(
            region.x as f32 / s,
            region.y as f32 / s,
            region.width as f32 / s,
            region.height as f32 / s,
        );
        let order = self.tree.visible_in_order();
        let Ui {
            canvas,
            text,
            tree,
            theme,
            hovered,
            focused,
            focus_visible,
            window_focused,
            ..
        } = self;
        canvas.paint_region(region, s, text, |p| {
            p.fill_rect(area.inflate(1.0, 1.0), theme.background);
            for id in order {
                let node = &mut tree.nodes[id];
                if !node.laid_out || !node.bounds.inflate(PAINT_MARGIN, PAINT_MARGIN).intersects(&area) {
                    continue;
                }
                let mut cx = PaintCx {
                    bounds: node.bounds,
                    content: node.content(),
                    hovered: *hovered == Some(id),
                    focused: *focused == Some(id),
                    focus_visible: *focus_visible,
                    window_focused: *window_focused,
                    theme,
                };
                node.widget.paint(&mut cx, p);
            }
        });
    }
}
