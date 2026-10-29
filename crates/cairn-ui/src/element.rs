//! 界面描述：[`El`]（元素）、[`View`]、以及所有元素共用的布局修饰方法 [`Styled`]。

use std::hash::{Hash, Hasher};

use crate::widget::{AnyWidget, Widget};
use crate::{Align, Insets, Length, Style};

/// 一个界面元素：控件 + 布局样式 + 子元素。`view` 函数返回的就是一棵元素树。
///
/// 元素只是描述，构建很便宜；框架拿它和保留着的控件树比对，只把变化应用上去。
pub struct El<S: 'static> {
    pub(crate) widget: Box<dyn AnyWidget<S>>,
    pub(crate) style: Style,
    pub(crate) children: Vec<El<S>>,
    pub(crate) key: Option<u64>,
}

impl<S: 'static> El<S> {
    /// 用自定义控件创建元素。
    pub fn new(widget: impl Widget<S>) -> Self {
        El {
            widget: Box::new(widget),
            style: Style::default(),
            children: Vec::new(),
            key: None,
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }
}

/// 能被 `view` 返回的东西：任何能转成 [`El`] 的类型。
///
/// `fn view(&self) -> impl View<Self>` 这样写就不需要在末尾 `.into()`。
pub trait View<S: 'static>: Into<El<S>> {}

impl<S: 'static, T: Into<El<S>>> View<S> for T {}

/// 所有元素共用的布局修饰方法。单位是逻辑像素。
pub trait Styled: Sized {
    #[doc(hidden)]
    fn style_mut(&mut self) -> &mut Style;
    #[doc(hidden)]
    fn key_mut(&mut self) -> &mut Option<u64>;

    fn width(mut self, w: f32) -> Self {
        self.style_mut().width = Length::Px(w);
        self
    }

    fn height(mut self, h: f32) -> Self {
        self.style_mut().height = Length::Px(h);
        self
    }

    fn size(self, w: f32, h: f32) -> Self {
        self.width(w).height(h)
    }

    fn min_width(mut self, w: f32) -> Self {
        self.style_mut().min_width = Length::Px(w);
        self
    }

    fn max_width(mut self, w: f32) -> Self {
        self.style_mut().max_width = Length::Px(w);
        self
    }

    fn min_height(mut self, h: f32) -> Self {
        self.style_mut().min_height = Length::Px(h);
        self
    }

    fn max_height(mut self, h: f32) -> Self {
        self.style_mut().max_height = Length::Px(h);
        self
    }

    /// 宽度占满父容器的内容区。
    fn fill_width(mut self) -> Self {
        self.style_mut().width = Length::Percent(100.0);
        self
    }

    /// 高度占满父容器的内容区。
    fn fill_height(mut self) -> Self {
        self.style_mut().height = Length::Percent(100.0);
        self
    }

    /// 在父容器主轴方向上分配剩余空间的权重。
    fn grow(mut self, weight: f32) -> Self {
        self.style_mut().grow = weight;
        self
    }

    /// 空间不够时在主轴方向上收缩的权重，默认 1；0 表示不收缩（例如固定高度的标题栏）。
    fn shrink(mut self, weight: f32) -> Self {
        self.style_mut().shrink = weight;
        self
    }

    /// 绝对定位：不参与父容器的排列，用 [`Styled::top`]、[`Styled::right`]、[`Styled::bottom`]、
    /// [`Styled::left`] 相对父容器定位。后面的兄弟元素画在上面、先接收点击，所以浮层通常放在最后。
    fn absolute(mut self) -> Self {
        self.style_mut().absolute = true;
        self
    }

    /// 绝对定位时与父容器上边的距离。
    fn top(mut self, v: f32) -> Self {
        self.style_mut().inset[0] = Length::Px(v);
        self
    }

    fn right(mut self, v: f32) -> Self {
        self.style_mut().inset[1] = Length::Px(v);
        self
    }

    fn bottom(mut self, v: f32) -> Self {
        self.style_mut().inset[2] = Length::Px(v);
        self
    }

    fn left(mut self, v: f32) -> Self {
        self.style_mut().inset[3] = Length::Px(v);
        self
    }

    /// 内边距。传一个数字表示四边相同，或传 [`Insets`]。
    fn padding(mut self, p: impl Into<Insets>) -> Self {
        self.style_mut().padding = p.into();
        self
    }

    fn margin(mut self, m: impl Into<Insets>) -> Self {
        self.style_mut().margin = m.into();
        self
    }

    /// 覆盖父容器的交叉轴对齐方式。
    fn align_self(mut self, a: Align) -> Self {
        self.style_mut().align_self = Some(a);
        self
    }

    /// 隐藏（不占空间）。
    fn hidden(mut self, hidden: bool) -> Self {
        self.style_mut().hidden = hidden;
        self
    }

    /// 在同一父元素的子元素中唯一标识本元素。列表增删、重排时用它保持控件状态
    /// （例如输入框里的光标）跟着数据走，而不是跟着位置走。
    fn key(mut self, key: impl Hash) -> Self {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        *self.key_mut() = Some(h.finish());
        self
    }
}

impl<S: 'static> Styled for El<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

/// 为内置控件的构建器实现 [`Styled`] 和到 [`El`] 的转换。
/// 构建器结构体需要有 `style: Style` 与 `key: Option<u64>` 两个字段。
macro_rules! element {
    (@impl [$($gen:tt)*] $ty:ty) => {
        impl<$($gen)*> $crate::Styled for $ty {
            fn style_mut(&mut self) -> &mut $crate::Style {
                &mut self.style
            }
            fn key_mut(&mut self) -> &mut Option<u64> {
                &mut self.key
            }
        }

        impl<S: 'static> From<$ty> for $crate::El<S>
        where
            $ty: $crate::Widget<S>,
        {
            fn from(mut w: $ty) -> Self {
                let style = std::mem::take(&mut w.style);
                let key = w.key.take();
                let mut el = $crate::El::new(w);
                el.style = style;
                el.key = key;
                el
            }
        }
    };
    // 与应用状态无关的控件（如 Label），可以放进任何 `El<S>`。
    ($ty:ident) => {
        element!(@impl [] $ty);
    };
    // 带回调、因而与应用状态类型 `S` 绑定的控件（如 `Button<S>`）。
    ($ty:ident<S>) => {
        element!(@impl [S: 'static] $ty<S>);
    };
}

pub(crate) use element;
