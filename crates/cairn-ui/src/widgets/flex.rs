//! 弹性布局容器：[`column`]（纵向）与 [`row`]（横向）。

use crate::widget::{Change, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::{Align, Color, Direction, El, Justify, Painter, Style, Styled};

/// 纵向或横向排列子元素的容器。用 [`column`] / [`row`] 创建。
pub struct Flex<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    background: Option<Color>,
    radius: f32,
}

/// 纵向排列。子元素默认靠左（不拉伸），需要占满宽度时对子元素用 `.fill_width()`。
pub fn column<S: 'static>() -> Flex<S> {
    Flex::new(Direction::Column, Align::Start)
}

/// 横向排列。子元素默认在纵向居中。
pub fn row<S: 'static>() -> Flex<S> {
    Flex::new(Direction::Row, Align::Center)
}

impl<S: 'static> Flex<S> {
    fn new(direction: Direction, align: Align) -> Self {
        Flex {
            style: Style {
                direction,
                align_items: Some(align),
                ..Style::default()
            },
            key: None,
            children: Vec::new(),
            background: None,
            radius: 0.0,
        }
    }

    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// 子元素间距。
    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = gap;
        self
    }

    /// 子元素在交叉轴上的对齐方式。
    pub fn align(mut self, align: Align) -> Self {
        self.style.align_items = Some(align);
        self
    }

    /// 子元素在主轴上的分布方式。
    pub fn justify(mut self, justify: Justify) -> Self {
        self.style.justify = justify;
        self
    }

    /// 放不下时换行。
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.style.wrap = wrap;
        self
    }

    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

impl<S: 'static> Styled for Flex<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Flex<S>> for El<S> {
    fn from(mut f: Flex<S>) -> Self {
        let style = std::mem::take(&mut f.style);
        let key = f.key.take();
        let children = std::mem::take(&mut f.children);
        let mut el = El::new(f);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for Flex<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.background != self.background || new.radius != self.radius {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.background = new.background;
        self.radius = new.radius;
        change
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        if let Some(bg) = self.background {
            p.fill_rounded_rect(cx.bounds(), self.radius, bg);
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Group,
            ..Semantics::default()
        }
    }
}
