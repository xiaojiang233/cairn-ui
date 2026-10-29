//! 玻璃面板：带背景模糊、内发光、细边和多层阴影的容器。

use crate::anim::Transition;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{drive, max_extent};
use crate::{Affine, Align, Brush, Direction, El, Insets, Justify, Painter, Rect, Shadow, Style, Styled};

/// 面板。用 [`panel`] 创建，默认纵向排列、子元素靠左（与 [`crate::column`] 相同）。
///
/// 外观全部来自主题的 [`Theme::panel`]（圆角取 `radius.panel`，内边距取 `spacing.panel_padding`，
/// 子元素间距取 `spacing.gap`，都可以用 [`Styled::padding`]、[`Panel::gap`] 覆盖）。
/// 在 Nocturne 主题下按下面的顺序绘制，前五层都被裁剪在圆角矩形内（阴影只在外面）：
///
/// 1. 外阴影，只画在面板外面；
/// 2. 窗口背景模糊后的图像（[`Brush::Backdrop`]）；
/// 3. 白色半透明填充；
/// 4. 内发光：顶部的白色向底部渐隐；
/// 5. 顶部 1 物理像素内高光与底部 1 物理像素内暗边；
/// 6. 1 物理像素边框；
/// 7. 子元素。
///
/// 指针在面板或它的任何子元素上时面板“悬停”：填充、边框、阴影过渡到悬停配色，并上浮 `panel.lift`
/// （用 [`Panel::hover_effect`] 关闭）。玻璃里不要再套玻璃：面板里不要再放面板。
pub struct Panel<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    horizontal: bool,
    align: Option<Align>,
    hover_effect: bool,
    bar: bool,
    popover: bool,
    hover: Transition,
}

pub fn panel<S: 'static>() -> Panel<S> {
    Panel {
        style: Style {
            // 占位值，布局前按主题补全（见 `resolve_style`）。
            padding: Insets::AUTO,
            gap: f32::NAN,
            ..Style::default()
        },
        key: None,
        children: Vec::new(),
        horizontal: false,
        align: None,
        hover_effect: true,
        bar: false,
        popover: false,
        hover: Transition::default(),
    }
}

impl<S: 'static> Panel<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// 子元素间距，默认取主题的 `spacing.gap`。
    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = gap;
        self
    }

    /// 子元素在交叉轴上的对齐方式。默认纵向时靠左、横向时居中。
    pub fn align(mut self, align: Align) -> Self {
        self.align = Some(align);
        self
    }

    /// 子元素在主轴上的分布方式。
    pub fn justify(mut self, justify: Justify) -> Self {
        self.style.justify = justify;
        self
    }

    /// 子元素横向排列。
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// 浮动工具栏外观：取主题的 [`Theme::bar`] 与 `radius.bar`（Nocturne 下没有边框、圆角更大），
    /// 例如贴着窗口底部的程序坞。
    pub fn bar(mut self) -> Self {
        self.bar = true;
        self
    }

    /// 弹出框外观：取主题的 [`Theme::popover`]（更不透明、阴影更重），圆角同普通面板。
    /// 悬停效果默认关闭。
    pub fn popover(mut self) -> Self {
        self.popover = true;
        self.hover_effect = false;
        self
    }

    /// 悬停时是否上浮、加亮填充与边框、加深阴影，默认开启。
    pub fn hover_effect(mut self, enabled: bool) -> Self {
        self.hover_effect = enabled;
        self
    }
}

impl<S: 'static> Panel<S> {
    fn look<'t>(&self, theme: &'t Theme) -> &'t crate::PanelStyle {
        if self.bar {
            &theme.bar
        } else if self.popover {
            &theme.popover
        } else {
            &theme.panel
        }
    }
}

impl<S: 'static> Styled for Panel<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Panel<S>> for El<S> {
    fn from(mut p: Panel<S>) -> Self {
        let mut style = std::mem::take(&mut p.style);
        let (direction, default_align) = if p.horizontal {
            (Direction::Row, Align::Center)
        } else {
            (Direction::Column, Align::Start)
        };
        style.direction = direction;
        style.align_items = Some(p.align.unwrap_or(default_align));
        let key = p.key.take();
        let children = std::mem::take(&mut p.children);
        let mut el = El::new(p);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for Panel<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        // 子元素、样式、key 已经由元素树处理，这里只剩自己的属性。
        let change = if new.hover_effect != self.hover_effect || new.bar != self.bar || new.popover != self.popover {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.hover_effect = new.hover_effect;
        self.bar = new.bar;
        self.popover = new.popover;
        change
    }

    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        if style.padding.is_auto() {
            style.padding = Insets::all(theme.spacing.panel_padding);
        }
        if style.gap.is_nan() {
            style.gap = theme.spacing.gap;
        }
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let look = self.look(theme);
        // 悬停时阴影会换成 `shadow_hover`，面板整体上浮 `lift`（向上多出一截）。
        let (e, lift) = if self.hover_effect {
            (max_extent(&look.shadow, &look.shadow_hover), look.lift)
        } else {
            (look.shadow.extent(), 0.0)
        };
        bounds.outset(e[0] + lift, e[1], e[2], e[3])
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let hovered = self.hover_effect && cx.is_hovered();
        let motion = &cx.theme().motion;
        drive(&mut self.hover, cx, hovered, motion.hover, motion.easing);
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        let up = self.look(cx.theme()).lift * self.hover.value(cx.now());
        if up.abs() > 1e-4 {
            Affine::translate(0.0, -up)
        } else {
            Affine::IDENTITY
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = self.look(theme);
        let h = self.hover.value(cx.now());
        let r = if self.bar { theme.radius.bar } else { theme.radius.panel };
        let b = cx.bounds();
        let hair = p.hairline();

        // 1. 外阴影（只在面板外面）
        p.draw_shadow(b, r, &look.shadow.lerp(&look.shadow_hover, h));
        // 2. 模糊后的背景
        if look.backdrop {
            p.fill(b, r, &Brush::Backdrop);
        }
        // 3. 填充
        let fill = look.fill.mix(look.fill_hover, h);
        if fill.a > 0 {
            p.fill_rounded_rect(b, r, fill);
        }
        // 4. 内发光：渐隐的终点用同一颜色、透明度 0，中间不会发灰
        if look.glow.a > 0 {
            p.fill(b, r, &Brush::vertical(b, look.glow, look.glow.with_alpha(0)));
        }
        // 5. 顶部内高光与底部内暗边，各 1 物理像素
        if look.top_highlight.a > 0 {
            p.draw_inner_shadow(b, r, &Shadow::new(0.0, hair, 0.0, look.top_highlight));
        }
        if look.bottom_edge.a > 0 {
            p.draw_inner_shadow(b, r, &Shadow::new(0.0, -hair, 0.0, look.bottom_edge));
        }
        // 6. 边框
        let border = look.border.mix(look.border_hover, h);
        if border.a > 0 {
            p.stroke_rect(b, r, 0.0, border);
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        // 悬停程度是在绘制时按当前状态过渡的，这里只负责让面板重画一次，把过渡启动起来。
        if self.hover_effect && matches!(event, Event::PointerEnter | Event::PointerLeave) {
            cx.request_paint();
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Group,
            ..Semantics::default()
        }
    }
}
