//! 纵向滚动容器：把超出的内容裁剪在视口里，用滚轮、触控板和滚动条滚动。
//!
//! 实现上滚动改变的是子元素**真实的窗口坐标**（[`Widget::child_offset`]），不是只在绘制时加一个平移：
//! 命中测试、事件坐标、输入法候选框、焦点框和脏区因此始终一致，滚进视口的控件和没有滚动时没有任何区别。
//!
//! 元素结构：视口（[`Scroll`] 控件本身）下面恰好有一个子元素——内部的内容列，使用者的子元素都在列里。
//! 竖直方向的细节集中在 `range`、`thumb`、`strip` 几个函数里，以后加横向滚动时主要改这几处。

use crate::anim::Transition;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::drive;
use crate::{
    Align, Direction, El, Insets, Length, Painter, Point, PointerButton, Rect, ScrollDelta, Size, Style, Styled, column,
};

/// 内容比视口多出不到这个距离（逻辑像素）算放得下，免得浮点误差让滚动条在临界处闪现。
const OVERFLOW_EPS: f32 = 0.5;
/// 点击滚动条轨道翻一页：距离是视口高度的这一比例，留出一部分让人接上上下文。
const PAGE_FRACTION: f32 = 0.9;

/// 纵向滚动容器。用 [`scroll`] 创建。
///
/// 超出视口的内容被裁剪；内容放不下时在右侧画滚动条（一条细长的圆头滑块，没有轨道底色）。
/// 操作方式：
///
/// - 滚轮：每格滚动 [`crate::ScrollStyle::wheel_step`]，按 [`crate::Motion::scroll`] 平滑过渡，
///   连续转动时从当前位置继续，不会卡顿或回头；触控板的像素增量直接生效、没有动画。
/// - 滚动条：拖动滑块（保持抓取位置）；点击轨道翻一页（视口高度的 90%）。
/// - 滚到头（或内容放得下）之后，滚轮事件不被消费，交给外层的滚动容器。
///
/// 滑块平时很淡，指针在滚动区域内变亮，在滚动条上或拖动时最亮、并变宽，外观全部来自 [`crate::Theme::scroll`]。
/// 滚动条画在内容上面、不占布局空间；右侧约 14 像素宽的热区里的点击归滚动条，不会落到下面的内容上。
/// 不希望滑块盖住内容边缘时，给内容留出右内边距。
///
/// # 尺寸与内边距
///
/// 视口的尺寸就是它自己的布局尺寸，用 [`Styled::width`]、[`Styled::height`]、[`Styled::grow`] 等照常设置；
/// 没有设置 `min_height` / `min_width` 时按 0 处理，所以放在纵向容器里 `.grow(1.0)` 会占满剩余空间并滚动，
/// 而不是被内容撑大。
///
/// [`Styled::padding`] 设置的内边距属于**内容**而不是视口：它随内容一起滚动，最后一行下面的留白也在。
/// 焦点框这类画在控件外面的东西会被视口裁掉，需要完整显示时让内边距不小于焦点框的外扩距离。
///
/// # 暂不支持
///
/// 横向滚动、键盘滚动（方向键、PageUp / PageDown）、焦点移到视口外的控件时自动滚到可见位置，
/// 以及用代码设置 / 读取滚动位置。
pub struct Scroll<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    gap: f32,
    align: Align,
    radius: f32,

    // ---- 下面是控件自己的状态，`update` 时保留 ----
    /// 当前滚动量：内容向上移出视口的距离，0 是顶部。
    offset: f32,
    /// 滚轮和点击轨道的平滑滚动。`target()` 是最终停下的位置；`offset` 在每个定时器节拍里取它此刻的值。
    glide: Transition,
    /// 下一个定时器节拍已经排上。连续转滚轮时不能每次事件都重排，否则节拍一直被往后推，一帧也走不到。
    ticking: bool,
    /// 内容高度（含视口自己的下内边距，视口的内边距总是 0）与视口的边框盒，布局回调里更新。
    content_h: f32,
    bounds: Rect,
    /// 缩放因子，由事件得知。滚动量取整到整数个物理像素，滚动时文字和边缘才能刚好落在像素网格上。
    scale: f32,
    /// 正在拖动滑块：按下时指针到滑块顶端的距离。
    grab: Option<f32>,
    /// 指针在右侧的滚动条热区里。
    bar_hover: bool,
    /// 滑块外观的过渡：指针在滚动区域内；指针在滚动条上或正在拖动。
    hover: Transition,
    active: Transition,
}

/// 创建一个纵向滚动容器。默认子元素靠左、间距 0、没有内边距、不做圆角裁剪（与 [`column()`] 一致）。
///
/// ```no_run
/// use cairn_ui::prelude::*;
///
/// struct Log {
///     lines: Vec<String>,
/// }
///
/// impl Log {
///     fn view(&self) -> impl View<Self> {
///         scroll()
///             .height(240.0)
///             .padding(12.0)
///             .gap(6.0)
///             .children(self.lines.iter().map(label))
///     }
/// }
/// ```
pub fn scroll<S: 'static>() -> Scroll<S> {
    Scroll {
        style: Style::default(),
        key: None,
        children: Vec::new(),
        gap: 0.0,
        align: Align::Start,
        radius: 0.0,
        offset: 0.0,
        glide: Transition::default(),
        ticking: false,
        content_h: 0.0,
        bounds: Rect::ZERO,
        scale: 1.0,
        grab: None,
        bar_hover: false,
        hover: Transition::default(),
        active: Transition::default(),
    }
}

impl<S: 'static> Scroll<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// 子元素之间的间距，默认 0。
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// 子元素在横向的对齐方式，默认靠左（不拉伸）。`Align::Stretch` 让每个子元素占满视口宽度。
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// 视口裁剪内容的圆角半径，默认 0（直角）。放在圆角面板里时设成和面板一致，内容才不会盖住圆角。
    pub fn clip_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

impl<S: 'static> Styled for Scroll<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Scroll<S>> for El<S> {
    fn from(mut s: Scroll<S>) -> Self {
        let mut style = std::mem::take(&mut s.style);
        let key = s.key.take();
        let children = std::mem::take(&mut s.children);

        // 内容列：使用者的子元素、间距、对齐和内边距都在这里，随内容一起滚动。
        // `shrink = 0`：列的高度永远是内容的高度，放不下的部分溢出视口，而不是被压扁。
        let mut content: El<S> = column().gap(s.gap).align(s.align).children(children).into();
        content.style.padding = std::mem::replace(&mut style.padding, Insets::ZERO);
        content.style.shrink = 0.0;

        // 视口：纵向排列并拉伸，内容列才能得到视口的宽度。最小尺寸没设时按 0，否则弹性布局的
        // 自动最小尺寸会让它在 `grow` 时被内容撑到完整高度，而不是滚动。
        style.direction = Direction::Column;
        style.align_items = Some(Align::Stretch);
        if style.min_height == Length::Auto {
            style.min_height = Length::Px(0.0);
        }
        if style.min_width == Length::Auto {
            style.min_width = Length::Px(0.0);
        }

        let mut el = El::new(s);
        el.style = style;
        el.key = key;
        el.children = vec![content];
        el
    }
}

/// 滑块在竖直方向上的几何（窗口坐标）。
struct Thumb {
    /// 轨道的起点与长度：视口高度减去上下各一个 `bar_inset`。
    track_y: f32,
    track_len: f32,
    /// 滑块的顶端与长度。
    y: f32,
    len: f32,
}

impl<S: 'static> Scroll<S> {
    /// 可滚动的距离：内容超出视口的部分。超出不到半个像素算放得下，返回 0。
    fn range(&self) -> f32 {
        let over = self.content_h - self.bounds.height;
        if over > OVERFLOW_EPS { over } else { 0.0 }
    }

    fn overflowing(&self) -> bool {
        self.range() > 0.0
    }

    /// 用来定位子元素和画滑块的滚动量：限制在范围内，取整到整数个物理像素。
    fn shown(&self) -> f32 {
        ((self.offset * self.scale).round() / self.scale).clamp(0.0, self.range())
    }

    /// 右侧滚动条的热区：宽度是滑块变宽后的宽度加两侧的留白，比看得见的滑块好点。
    fn strip(bounds: Rect, theme: &Theme) -> Rect {
        let look = &theme.scroll;
        let w = look.bar_width_hover + 2.0 * look.bar_inset;
        Rect::new(bounds.right() - w, bounds.y, w, bounds.height)
    }

    /// 内容放不下，并且 `pos` 在滚动条热区里。
    fn on_bar(&self, pos: Point, bounds: Rect, theme: &Theme) -> bool {
        self.overflowing() && Self::strip(bounds, theme).contains(pos)
    }

    /// 滑块长度正比于视口占内容的比例，但不短于 `min_thumb`；位置正比于滚动量占可滚动距离的比例。
    fn thumb(&self, bounds: Rect, theme: &Theme) -> Thumb {
        let look = &theme.scroll;
        let track_y = bounds.y + look.bar_inset;
        let track_len = (bounds.height - 2.0 * look.bar_inset).max(0.0);
        let ratio = if self.content_h > 0.0 {
            bounds.height / self.content_h
        } else {
            1.0
        };
        let len = (track_len * ratio).max(look.min_thumb).min(track_len);
        let range = self.range();
        let t = if range > 0.0 { self.shown() / range } else { 0.0 };
        Thumb {
            track_y,
            track_len,
            y: track_y + (track_len - len) * t,
            len,
        }
    }

    /// 立即跳到 `to`（限制在范围内），同时取消正在进行的平滑滚动。返回滚动量是否变了。
    fn jump(&mut self, to: f32) -> bool {
        let to = to.clamp(0.0, self.range());
        let changed = to != self.offset;
        self.offset = to;
        self.glide.set(to);
        changed
    }

    /// 相对滚动 `dy`（正为向下）。`smooth` 时按 [`crate::Motion::scroll`] 平滑过渡（滚轮、点击轨道），
    /// 否则立即到位（触控板）。返回是否消费了这次滚动：已经到头（或内容放得下）返回 `false`，
    /// 调用者应当让外层的滚动容器去处理。
    fn scroll_by(&mut self, cx: &mut EventCx<S>, dy: f32, smooth: bool) -> bool {
        let range = self.range();
        if range <= 0.0 || dy == 0.0 || !dy.is_finite() {
            return false;
        }
        let now = cx.now();
        // 连续滚动从上一次的目标继续算（还没走完的距离不丢），立即生效的则从眼前的位置算。
        let base = if smooth { self.glide.target() } else { self.offset };
        let to = (base + dy).clamp(0.0, range);
        // 平滑滚动还在朝这个方向走时，滚轮多转的几格也算消费掉：否则外层会趁内层还没停稳就动起来。
        let heading = self.glide.is_animating(now) && (self.glide.target() - self.offset) * dy > 0.0;
        if to == base && !heading {
            return false;
        }
        let (duration, easing) = (cx.theme().motion.scroll, cx.theme().motion.easing);
        if smooth && !duration.is_zero() {
            self.glide.animate_to(to, now, duration, easing);
            // 先把当前时刻的位置落实，节拍之间的位置才不会落后于动画。
            self.offset = self.glide.value(now).clamp(0.0, range);
            if !self.ticking {
                self.ticking = true;
                cx.request_timer(now + cx.frame_interval());
            }
        } else {
            self.jump(to);
        }
        cx.request_arrange();
        cx.request_paint();
        true
    }

    /// 定时器节拍：推进平滑滚动，还没到位就排下一个节拍。
    fn tick(&mut self, cx: &mut EventCx<S>) {
        self.ticking = false;
        let now = cx.now();
        let at = self.glide.value(now).clamp(0.0, self.range());
        if at != self.offset {
            self.offset = at;
            cx.request_arrange();
            cx.request_paint();
        }
        if self.glide.is_animating(now) {
            self.ticking = true;
            cx.request_timer(now + cx.frame_interval());
        }
    }

    fn wheel(&mut self, cx: &mut EventCx<S>, delta: ScrollDelta) {
        let consumed = match delta {
            // winit：向上滚一格 y 为 +1，而向上滚是让内容往下走、滚动量变小。
            ScrollDelta::Lines { y, .. } => {
                let step = cx.theme().scroll.wheel_step;
                self.scroll_by(cx, -y * step, true)
            }
            // 触控板给的是物理像素，本来就是平滑的，直接跟手。
            ScrollDelta::Pixels { y, .. } => {
                let dy = -(y as f32) / cx.scale();
                self.scroll_by(cx, dy, false)
            }
        };
        if consumed {
            cx.set_handled();
        }
    }

    /// 左键在滚动条热区里按下：按在滑块上开始拖动，按在轨道上翻一页。
    fn press(&mut self, cx: &mut EventCx<S>, pos: Point) {
        let (bounds, theme) = (cx.bounds(), cx.theme());
        if !self.on_bar(pos, bounds, theme) {
            return;
        }
        let thumb = self.thumb(bounds, theme);
        if pos.y >= thumb.y && pos.y < thumb.y + thumb.len {
            // 保持抓取位置；平滑滚动进行到一半就地停下，免得和拖动打架。
            self.glide.set(self.offset);
            self.grab = Some(pos.y - thumb.y);
        } else {
            let dir = if pos.y < thumb.y { -1.0 } else { 1.0 };
            self.scroll_by(cx, dir * PAGE_FRACTION * bounds.height, true);
        }
        self.bar_hover = true;
        // 处理了按下，运行时才会让本控件独占指针直到松开。
        cx.set_handled();
        cx.request_paint();
    }

    fn pointer_move(&mut self, cx: &mut EventCx<S>, pos: Point) {
        let (bounds, theme) = (cx.bounds(), cx.theme());
        if let Some(grab) = self.grab {
            if cx.is_captured() {
                let thumb = self.thumb(bounds, theme);
                let free = thumb.track_len - thumb.len;
                let to = if free > 0.0 {
                    (pos.y - grab - thumb.track_y) / free * self.range()
                } else {
                    0.0
                };
                if self.jump(to) {
                    cx.request_arrange();
                    cx.request_paint();
                }
                return;
            }
            // 指针独占已经没了（例如拖动中窗口失去了焦点），松开事件不会再来：拖动到此为止。
            self.grab = None;
            cx.request_paint();
        }
        let on = self.on_bar(pos, bounds, theme);
        if on != self.bar_hover {
            self.bar_hover = on;
            cx.request_paint();
        }
    }

    /// 左键松开。拖动结束；在滚动条上松开（翻页点击的另一半）也归我们。
    fn release(&mut self, cx: &mut EventCx<S>, pos: Point) {
        let (bounds, theme) = (cx.bounds(), cx.theme());
        let was_dragging = self.grab.take().is_some();
        let on = self.on_bar(pos, bounds, theme);
        if was_dragging || on != self.bar_hover {
            cx.request_paint();
        }
        self.bar_hover = on;
        // 内容里的子元素冒泡上来的松开不归我们管，留给外层。
        if was_dragging || on {
            cx.set_handled();
        }
    }
}

impl<S: 'static> Widget<S> for Scroll<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        // 子元素、样式、key 已经由元素树处理；间距和对齐在构建内容列时就用掉了，是内容列的样式。
        let change = if new.radius != self.radius {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.radius = new.radius;
        change
    }

    fn arrange(&mut self, _cx: &mut LayoutCx, bounds: Rect, _content: Rect) {
        self.bounds = bounds;
    }

    fn arrange_content(&mut self, _cx: &mut LayoutCx, content_size: Size) {
        self.content_h = content_size.height;
        // 窗口变大或内容变短之后，滚动量不能停在末端之外。
        let range = self.range();
        if self.offset > range {
            self.offset = range;
        }
        if self.glide.target() > range {
            // 平滑滚动的目标越界了：就地停下（滚动量已经被限制在新的末端以内）。
            self.glide.set(self.offset);
        }
    }

    fn child_offset(&self) -> Point {
        Point::new(0.0, self.shown())
    }

    fn clip_children(&self, _theme: &Theme) -> Option<f32> {
        Some(self.radius)
    }

    fn intercepts_pointer(&self, pos: Point, bounds: Rect, theme: &Theme) -> bool {
        self.on_bar(pos, bounds, theme)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let motion = &cx.theme().motion;
        let hovered = cx.is_hovered();
        let active = self.bar_hover || self.grab.is_some();
        drive(&mut self.hover, cx, hovered, motion.hover, motion.easing);
        drive(&mut self.active, cx, active, motion.hover, motion.easing);
    }

    /// 视口本身是透明的，什么也不画。
    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    /// 滑块画在子元素上面。
    fn paint_over(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        if !self.overflowing() {
            return;
        }
        let theme = cx.theme();
        let look = &theme.scroll;
        let now = cx.now();
        let (hover, active) = (self.hover.value(now), self.active.value(now));
        let bounds = cx.bounds();
        let thumb = self.thumb(bounds, theme);
        let width = look.bar_width + (look.bar_width_hover - look.bar_width) * active;
        let rect = Rect::new(bounds.right() - look.bar_inset - width, thumb.y, width, thumb.len);
        let color = look.thumb.mix(look.thumb_hover, hover).mix(look.thumb_active, active);
        p.fill_rounded_rect(rect, width / 2.0, color);
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if cx.scale() != self.scale {
            self.scale = cx.scale();
            // 滚动量按物理像素取整，缩放变了要按新的网格重新定位子元素。
            cx.request_arrange();
        }
        match event {
            Event::Wheel { delta } => self.wheel(cx, *delta),
            // 悬停只影响滑块的亮度：内容放得下（没有滑块）时不用重画。
            Event::PointerEnter => {
                if self.overflowing() {
                    cx.request_paint();
                }
            }
            Event::PointerLeave => {
                self.bar_hover = false;
                if self.overflowing() {
                    cx.request_paint();
                }
            }
            Event::PointerDown {
                pos,
                button: PointerButton::Left,
                ..
            } => self.press(cx, *pos),
            Event::PointerMove { pos } => self.pointer_move(cx, *pos),
            Event::PointerUp {
                pos,
                button: PointerButton::Left,
            } => self.release(cx, *pos),
            Event::Timer => self.tick(cx),
            _ => {}
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::ScrollArea,
            ..Semantics::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_snaps_to_physical_pixels_and_stays_in_range() {
        let mut s = scroll::<()>();
        s.bounds = Rect::new(0.0, 0.0, 100.0, 200.0);
        s.content_h = 1000.0;
        // 125% 缩放：显示用的滚动量总是整数个物理像素，各行才会整体平移、不会忽高忽矮。
        s.scale = 1.25;
        for offset in [0.0f32, 0.3, 1.0, 2.7, 123.4, 799.9] {
            s.offset = offset;
            let device = s.shown() * 1.25;
            assert!((device - device.round()).abs() < 1e-3, "{offset} -> {}", s.shown());
        }
        // 限制在 [0, 可滚动距离]；内容放得下（多出不到半个像素也算）时没有滚动量。
        s.offset = 5000.0;
        assert_eq!(s.shown(), 800.0);
        s.content_h = 200.3;
        s.offset = 10.0;
        assert_eq!((s.range(), s.shown()), (0.0, 0.0));
    }
}
