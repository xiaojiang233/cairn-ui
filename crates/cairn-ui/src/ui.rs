//! 运行时核心：持有应用状态与控件树，处理输入、重建界面、布局、按脏区重绘、驱动动画帧。
//!
//! 不直接依赖窗口：平台服务通过 [`Host`] 注入，所以同一套逻辑也能在测试里无窗口运行。

use std::any::Any;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use cairn_ui_platform::{
    self as platform, CursorIcon, Key, KeyEvent, Modifiers, NamedKey, PixelRect, PointerButton, ResizeEdge,
};

use crate::app::FileDrop;
use crate::damage::{self, Damage};
use crate::element::El;
use crate::host::Host;
use crate::theme::Theme;
use crate::tree::{Tree, WidgetId};
use crate::widget::{Event, EventCx, EventOutput, LayoutCx, PaintCx};
use crate::{Affine, Background, Canvas, DeviceRect, Image, Painter, Point, Rect, Size, TextSystem};

/// 重绘时脏区外扩的距离（逻辑像素），覆盖抗锯齿边缘和画在控件外面的焦点框。
const PAINT_MARGIN: f32 = 4.0;
/// 无边框窗口四周用来拖动改变大小的一圈的宽度，以及四个角的热区边长（逻辑像素）。
const RESIZE_BAND: f32 = 6.0;
const RESIZE_CORNER: f32 = 14.0;

pub(crate) type ViewBox<S> = Box<dyn Fn(&S) -> El<S>>;
pub(crate) type Update<S> = Box<dyn FnOnce(&mut S) + Send>;
pub(crate) type FileDropFn<S> = Box<dyn Fn(&mut S, FileDrop)>;
pub(crate) type ThemeFn<S> = Box<dyn Fn(&mut S) -> Option<Theme>>;

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
    /// 窗口没有系统边框且可以调整大小：四周一圈由这里处理成“拖动改变大小”。
    resize_border: bool,
    maximized: bool,
    file_drop: Option<FileDropFn<S>>,
    /// 应用通过它在运行中换主题（[`crate::App::theme_from`]）。
    theme_source: Option<ThemeFn<S>>,
    reduce_motion: bool,
    /// 上一帧在动画中、且带变换或半透明的控件，以及它当时的变换：它们影响整棵子树，下一帧要重画整棵子树。
    subtree_anim: Vec<(WidgetId, Affine)>,
    /// 合成层缓存（[`crate::Widget::cache_layer`]）。
    layer_cache: HashMap<WidgetId, CachedLayer>,
    /// 内容版本：状态、布局、重绘请求、主题有任何变化就加一，缓存的版本对不上就重画。
    epoch: u64,
}

/// 一张合成层缓存：子树在不做本控件变换、不透明时的样子。
pub(crate) struct CachedLayer {
    image: Image,
    /// 位图覆盖的范围（逻辑像素，窗口坐标）。
    rect: Rect,
    epoch: u64,
    scale: f32,
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
        apply_canvas_theme(&mut canvas, &theme);
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
            resize_border: false,
            maximized: false,
            file_drop: None,
            theme_source: None,
            reduce_motion: false,
            subtree_anim: Vec::new(),
            layer_cache: HashMap::new(),
            epoch: 0,
        };
        ui.tree.set_viewport(ui.logical_size());
        ui.rebuild_view();
        ui.layout();
        ui.damage.add_full();
        ui
    }

    /// 窗口没有系统边框时由运行时处理四周的缩放边（`resize_border`），以及窗口当前是否最大化。
    pub fn set_window_frame(&mut self, resize_border: bool, maximized: bool) {
        self.resize_border = resize_border;
        self.maximized = maximized;
    }

    pub fn set_file_drop(&mut self, f: Option<FileDropFn<S>>) {
        self.file_drop = f;
    }

    pub fn set_theme_source(&mut self, f: Option<ThemeFn<S>>, reduce_motion: bool) {
        self.theme_source = f;
        self.reduce_motion = reduce_motion;
    }

    /// 应用放进状态里的新主题：换背景和模糊参数，所有控件重新按新主题补全样式、重新测量，整窗重画。
    /// 控件本身保留（焦点、滚动位置、正在拖动的滑块都不受影响）。
    fn poll_theme(&mut self) {
        let Some(f) = &self.theme_source else { return };
        let Some(mut theme) = f(&mut self.state) else { return };
        if self.reduce_motion {
            theme.motion = theme.motion.reduced();
        }
        apply_canvas_theme(&mut self.canvas, &theme);
        self.theme = theme;
        let Tree { nodes, layout, .. } = &mut self.tree;
        for node in nodes.values() {
            layout.mark_dirty(node.layout);
        }
        self.tree.needs_layout = true;
        self.rebuild = true;
        self.damage.add_full();
        self.epoch += 1;
    }

    /// 指针在无边框窗口四周的缩放边上时，返回对应的边或角。最大化时没有缩放边。
    fn resize_edge(&self, p: Point) -> Option<ResizeEdge> {
        if !self.resize_border || self.maximized {
            return None;
        }
        let s = self.logical_size();
        let near = |v: f32, max: f32, band: f32| (v < band, v > max - band);
        let (left, right) = near(p.x, s.width, RESIZE_BAND);
        let (top, bottom) = near(p.y, s.height, RESIZE_BAND);
        let (left_c, right_c) = near(p.x, s.width, RESIZE_CORNER);
        let (top_c, bottom_c) = near(p.y, s.height, RESIZE_CORNER);
        Some(if (top && left_c) || (left && top_c) {
            ResizeEdge::NorthWest
        } else if (top && right_c) || (right && top_c) {
            ResizeEdge::NorthEast
        } else if (bottom && left_c) || (left && bottom_c) {
            ResizeEdge::SouthWest
        } else if (bottom && right_c) || (right && bottom_c) {
            ResizeEdge::SouthEast
        } else if top {
            ResizeEdge::North
        } else if bottom {
            ResizeEdge::South
        } else if left {
            ResizeEdge::West
        } else if right {
            ResizeEdge::East
        } else {
            return None;
        })
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
        let changes = self.tree.changes;
        self.tree.set_content(&mut cx, el, &mut self.damage);
        // 重建后画面没有任何变化（后台推来的状态没影响界面）就不让合成层缓存作废，
        // 否则动画进行中每来一次无关的状态更新，整个面板都要重画一遍。
        if self.tree.changes != changes {
            self.epoch += 1;
        }
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
        self.subtree_anim.retain(|(id, _)| !removed.contains(id));
        self.layer_cache.retain(|id, _| !removed.contains(id));
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
            let visual = n.visual(&self.theme);
            self.tree.damage_rect(id, visual, &self.theme, &mut self.damage);
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
            captured: self.captured == Some(id),
            window_focused: self.window_focused,
            modifiers: self.modifiers,
            scale: self.scale,
            blink: self.blink,
            now: self.clock.now(),
            out: EventOutput::default(),
        };
        node.widget.event(&mut cx, event);
        let out = cx.out;
        if !out.change.is_none() || !out.damage.is_empty() || out.arrange {
            self.epoch += 1;
        }
        if out.change.layout {
            self.tree.layout.mark_dirty(node.layout);
            self.tree.needs_layout = true;
        }
        if out.arrange {
            self.tree.needs_arrange = true;
        }
        // 脏区先去掉被祖先（滚动容器）裁剪掉的部分：滚出视口的控件请求重绘不该触发视口外的重画。
        let visual = (out.change.paint && node.laid_out).then(|| node.visual(&self.theme));
        for r in visual.into_iter().chain(out.damage) {
            self.tree.damage_rect(id, r, &self.theme, &mut self.damage);
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
        let icon = match self.pointer.and_then(|p| self.resize_edge(p)) {
            Some(ResizeEdge::North | ResizeEdge::South) => CursorIcon::ResizeVertical,
            Some(ResizeEdge::East | ResizeEdge::West) => CursorIcon::ResizeHorizontal,
            Some(ResizeEdge::NorthWest | ResizeEdge::SouthEast) => CursorIcon::ResizeNwSe,
            Some(ResizeEdge::NorthEast | ResizeEdge::SouthWest) => CursorIcon::ResizeNeSw,
            None => icon,
        };
        host.set_cursor(icon);
    }

    fn pointer_button(&mut self, host: &mut dyn Host, button: PointerButton, pressed: bool) {
        let Some(pos) = self.pointer else { return };
        self.layout();
        // 缩放边优先于下面的控件：交给系统的缩放循环，控件收不到这次按下。
        if pressed
            && button == PointerButton::Left
            && let Some(edge) = self.resize_edge(pos)
        {
            host.drag_resize_window(edge);
            return;
        }
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
            E::Resized { width, height } => {
                self.maximized = host.is_window_maximized();
                self.set_size(width, height, self.scale);
            }
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
            E::FileHovered(path) => self.file_drop_event(FileDrop::Hovered(path)),
            E::FileHoverCancelled => self.file_drop_event(FileDrop::Cancelled),
            E::FileDropped(path) => self.file_drop_event(FileDrop::Dropped(path)),
        }
        self.after(host);
    }

    fn file_drop_event(&mut self, drop: FileDrop) {
        if let Some(f) = &self.file_drop {
            f(&mut self.state, drop);
            self.rebuild = true;
        }
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
        self.epoch += 1;
    }

    /// 定时器或动画帧的时间到了。
    pub fn fire_timers(&mut self, host: &mut dyn Host, now: Instant) {
        if self.next_frame.is_some_and(|f| f <= now + std::time::Duration::from_millis(1)) {
            self.next_frame = None;
            let subtree = std::mem::take(&mut self.subtree_anim);
            for id in std::mem::take(&mut self.animating) {
                match subtree.iter().find(|(s, _)| *s == id) {
                    // 变换、透明度作用于整棵子树。控件自己的可见范围按约定已经包含它的变换（悬停上浮之类），
                    // 子孙的没有：重画子孙在上一帧变换前后覆盖的范围。
                    Some((_, t)) => {
                        self.damage_node(id);
                        if let Some(u) = self.tree.descendants_visual(id, &self.theme) {
                            self.damage.add(u.union(&t.apply_rect(u)));
                        }
                    }
                    None => self.damage_node(id),
                }
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
        self.poll_theme();
        let stats = frame_stats();
        let (mut rebuilt, mut laid_out) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        // 重建和布局本身也可能触发事件（悬停变化），最多来回几轮。
        for _ in 0..4 {
            if self.rebuild {
                let t = std::time::Instant::now();
                self.rebuild_view();
                rebuilt += t.elapsed();
            }
            let t = std::time::Instant::now();
            let changed = self.layout();
            laid_out += t.elapsed();
            if changed && self.pointer.is_some() {
                self.update_hover(host);
            }
            if !self.rebuild {
                break;
            }
        }
        if stats && (rebuilt.as_micros() > 500 || laid_out.as_micros() > 500) {
            eprintln!(
                "cairn-ui 事件后: 重建 {:.1} ms, 布局 {:.1} ms",
                rebuilt.as_secs_f64() * 1000.0,
                laid_out.as_secs_f64() * 1000.0
            );
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
        let now = self.clock.now();
        // 先更新合成层缓存：丢掉缓存时记下的脏区要在这一帧就画掉。
        let cache_started = std::time::Instant::now();
        self.refresh_layer_caches(now);
        let cache_time = cache_started.elapsed();
        let set = self.damage.take();
        let bounds = DeviceRect::new(0, 0, w, h);
        let regions = if set.full {
            vec![bounds]
        } else {
            damage::to_device(&set.rects, self.scale, PAINT_MARGIN, bounds)
        };
        let started = std::time::Instant::now();
        for region in &regions {
            self.paint_region(*region, now);
        }
        if frame_stats() {
            let px: u64 = regions.iter().map(|r| u64::from(r.width) * u64::from(r.height)).sum();
            eprintln!(
                "cairn-ui 帧: 缓存 {:.1} ms, 绘制 {:.1} ms, {} 块, {:.2} 百万像素, 第一块 {:?}",
                cache_time.as_secs_f64() * 1000.0,
                started.elapsed().as_secs_f64() * 1000.0,
                regions.len(),
                px as f64 / 1e6,
                regions.first()
            );
        }
        if !self.animating.is_empty() && self.next_frame.is_none() {
            self.next_frame = Some(now + host.frame_interval());
        }
        self.schedule(host);
        regions
    }

    /// 更新合成层缓存：不再需要缓存的控件丢掉缓存并重画那块区域；需要缓存而缓存缺失或过期的，
    /// 把子树画进新的位图。
    fn refresh_layer_caches(&mut self, now: Instant) {
        let wanted: Vec<WidgetId> = self
            .tree
            .nodes
            .iter()
            .filter(|(_, n)| n.laid_out && !n.style.hidden && n.widget.cache_layer())
            .map(|(id, _)| id)
            .collect();
        let dropped: Vec<WidgetId> = self
            .layer_cache
            .keys()
            .filter(|id| !wanted.contains(id))
            .copied()
            .collect();
        for id in dropped {
            if let Some(c) = self.layer_cache.remove(&id) {
                self.damage.add(c.rect);
            }
        }
        let s = self.scale;
        for id in wanted {
            if self
                .layer_cache
                .get(&id)
                .is_some_and(|c| c.epoch == self.epoch && c.scale == s)
            {
                continue;
            }
            let node = &self.tree.nodes[id];
            let own = node.visual(&self.theme);
            let logical = match self.tree.descendants_visual(id, &self.theme) {
                Some(d) => own.union(&d),
                None => own,
            };
            // 先裁到窗口里：位图只画得到窗口内的部分，记录的范围必须与位图一致，否则贴图时会被拉伸。
            let window = DeviceRect::new(0, 0, self.phys.0, self.phys.1);
            let Some(device) = DeviceRect::cover(&logical.inflate(PAINT_MARGIN, PAINT_MARGIN), s).intersection(&window)
            else {
                continue;
            };
            let area = Rect::new(
                device.x as f32 / s,
                device.y as f32 / s,
                device.width as f32 / s,
                device.height as f32 / s,
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
                maximized,
                animating,
                timers,
                subtree_anim,
                layer_cache,
                ..
            } = self;
            let mut st = PaintState {
                tree,
                theme,
                hover_path,
                focused: *focused,
                focus_visible: *focus_visible,
                window_focused: *window_focused,
                window_maximized: *maximized,
                now,
                area,
                animating,
                timers,
                subtree_anim,
                layer_cache: &*layer_cache,
                cache_root: Some(id),
            };
            let before = st.animating.len();
            let image = canvas.render_layer(device, s, text, |p| paint_node(&mut st, id, p));
            // 子树里还有控件在播自己的动画（悬停、选中的过渡……）：缓存只是这一刻的样子，下一帧要重画。
            let inner_moving = self.animating[before..].iter().any(|a| *a != id);
            if let Some(image) = image {
                self.layer_cache.insert(
                    id,
                    CachedLayer {
                        image,
                        rect: area,
                        epoch: if inner_moving { u64::MAX } else { self.epoch },
                        scale: s,
                    },
                );
            }
        }
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
            maximized,
            animating,
            timers,
            subtree_anim,
            layer_cache,
            ..
        } = self;
        let mut st = PaintState {
            tree,
            theme,
            hover_path,
            focused: *focused,
            focus_visible: *focus_visible,
            window_focused: *window_focused,
            window_maximized: *maximized,
            now,
            area,
            animating,
            timers,
            subtree_anim,
            layer_cache: &*layer_cache,
            cache_root: None,
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
    window_maximized: bool,
    now: Instant,
    /// 本次重画的区域（逻辑像素）。
    area: Rect,
    animating: &'a mut Vec<WidgetId>,
    timers: &'a mut Vec<(Instant, WidgetId)>,
    subtree_anim: &'a mut Vec<(WidgetId, Affine)>,
    layer_cache: &'a HashMap<WidgetId, CachedLayer>,
    /// 正在为它画合成层缓存的控件：它自己的变换和透明度不画进缓存，也不能反过来用缓存。
    cache_root: Option<WidgetId>,
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
        window_maximized: st.window_maximized,
        theme,
        now: st.now,
        frame_requested: false,
        timer: None,
    };
    node.widget.animate(&mut cx);
    let caching = st.cache_root == Some(id);
    let (transform, opacity) = if caching {
        (Affine::IDENTITY, 1.0)
    } else {
        (node.widget.transform(&cx), node.widget.opacity(&cx).clamp(0.0, 1.0))
    };
    let transformed = !transform.is_identity();
    if let Some(at) = cx.timer {
        st.timers.retain(|(_, w)| *w != id);
        st.timers.push((at, id));
    }
    if cx.frame_requested && (transformed || opacity < 1.0) && !st.subtree_anim.iter().any(|(w, _)| *w == id) {
        st.subtree_anim.push((id, transform));
    }
    // 完全透明：整棵子树都不用画。
    if opacity <= 0.0 {
        if cx.frame_requested && !st.animating.contains(&id) {
            st.animating.push(id);
        }
        return;
    }
    // 有合成层缓存：贴上缓存的位图，不再画子树。
    if !caching && node.widget.cache_layer() && let Some(cache) = st.layer_cache.get(&id) {
        if transformed {
            p.push_transform(transform);
        }
        p.draw_image(&cache.image, cache.rect, opacity);
        if transformed {
            p.pop_transform();
        }
        if cx.frame_requested && !st.animating.contains(&id) {
            st.animating.push(id);
        }
        return;
    }
    let layered = opacity < 1.0;
    if transformed {
        p.push_transform(transform);
    }
    if layered {
        // 图层覆盖本次重画的整个区域：子元素（阴影之类）可能画到本控件外面。
        p.push_layer(st.area, opacity);
    }
    if visible {
        node.widget.paint(&mut cx, p);
    }
    let clip = node.widget.clip_children(theme);
    let bounds = node.bounds;
    // 裁剪子元素的控件本身不在重画区域内时，子元素也不可能在。
    if clip.is_none() || visible {
        match clip {
            None => paint_children(st, id, p),
            Some(radius) => {
                // 子元素只可能画进裁剪区，所以重画区域也收窄到裁剪区：滚出视口的子元素在可见性判断里
                // 直接被跳过。收窄后为空（只有本控件的阴影之类落在区域里）则子元素一个也不用画。
                if let Some(inside) = st.area.intersection(&bounds) {
                    let outer = std::mem::replace(&mut st.area, inside);
                    p.push_rounded_clip(bounds, radius);
                    paint_children(st, id, p);
                    p.pop_clip();
                    st.area = outer;
                }
            }
        }
    }
    if visible {
        st.tree.nodes[id].widget.paint_over(&mut cx, p);
    }
    if layered {
        p.pop_layer();
    }
    if transformed {
        p.pop_transform();
    }
    if cx.frame_requested && !st.animating.contains(&id) {
        st.animating.push(id);
    }
}

fn paint_children<S: 'static>(st: &mut PaintState<S>, id: WidgetId, p: &mut dyn Painter) {
    for i in 0..st.tree.nodes[id].children.len() {
        let child = st.tree.nodes[id].children[i];
        paint_node(st, child, p);
    }
}

fn apply_canvas_theme(canvas: &mut Canvas, theme: &Theme) {
    canvas.set_background(match &theme.scene {
        Some(scene) => Background::Scene(scene.clone()),
        None => Background::Solid(theme.background),
    });
    canvas.set_backdrop_style(theme.backdrop);
    canvas.set_noise(theme.noise);
}

/// 设置了环境变量 `CAIRN_UI_FRAME_STATS` 时，每帧把绘制耗时和重画面积打到标准错误。
fn frame_stats() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CAIRN_UI_FRAME_STATS").is_some())
}
