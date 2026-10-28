//! cairn-ui 的布局层：taffy 的薄封装。
//!
//! 对上只暴露本 crate 自己的 [`Style`]（flexbox 的一个常用子集）和 [`Layout`]，
//! 上层不直接依赖 taffy 的类型——taffy 版本间 API 变化很大（例如 0.15 把对齐方式改成了结构体），
//! 隔离在这里，升级时只改这一个文件。
//!
//! 单位是逻辑像素。关闭了 taffy 自带的取整：取整放到渲染时按物理像素做，
//! 这样在 125%、150% 等缩放下边缘也能对齐像素网格。

use taffy::prelude::TaffyAuto;

/// 长度。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Length {
    #[default]
    Auto,
    /// 逻辑像素。
    Px(f32),
    /// 相对父容器内容区的百分比，取值 0–100。
    Percent(f32),
}

/// 四边的距离（内边距、外边距）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Insets {
    pub const ZERO: Insets = Insets {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    /// 按 CSS 的顺序：上、右、下、左。
    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Insets {
            top,
            right,
            bottom,
            left,
        }
    }

    pub const fn all(v: f32) -> Self {
        Insets::new(v, v, v, v)
    }

    /// 左右取 `horizontal`，上下取 `vertical`。
    pub const fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Insets::new(vertical, horizontal, vertical, horizontal)
    }

    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

impl From<f32> for Insets {
    fn from(v: f32) -> Self {
        Insets::all(v)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    #[default]
    Row,
    Column,
}

/// 交叉轴对齐。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

/// 主轴分布。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// 节点的布局样式（flexbox 子集）。
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// 隐藏：不占空间，也不参与布局（`display: none`）。
    pub hidden: bool,
    /// 子节点的排列方向。
    pub direction: Direction,
    pub wrap: bool,
    /// 子节点之间的间距。
    pub gap: f32,
    pub padding: Insets,
    pub margin: Insets,
    pub width: Length,
    pub height: Length,
    pub min_width: Length,
    pub min_height: Length,
    pub max_width: Length,
    pub max_height: Length,
    /// 子节点在交叉轴上的对齐方式。`None` 等同 CSS 默认值（拉伸）。
    pub align_items: Option<Align>,
    /// 覆盖父节点的 `align_items`。
    pub align_self: Option<Align>,
    pub justify: Justify,
    pub grow: f32,
    pub shrink: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            hidden: false,
            direction: Direction::Row,
            wrap: false,
            gap: 0.0,
            padding: Insets::ZERO,
            margin: Insets::ZERO,
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            min_height: Length::Auto,
            max_width: Length::Auto,
            max_height: Length::Auto,
            align_items: None,
            align_self: None,
            justify: Justify::Start,
            grow: 0.0,
            shrink: 1.0,
        }
    }
}

fn dimension(l: Length) -> taffy::Dimension {
    match l {
        Length::Auto => taffy::Dimension::auto(),
        Length::Px(v) => taffy::Dimension::length(v),
        Length::Percent(p) => taffy::Dimension::percent(p / 100.0),
    }
}

fn lpa(l: Length) -> taffy::LengthPercentageAuto {
    match l {
        Length::Auto => taffy::LengthPercentageAuto::AUTO,
        Length::Px(v) => taffy::LengthPercentageAuto::length(v),
        Length::Percent(p) => taffy::LengthPercentageAuto::percent(p / 100.0),
    }
}

fn align(a: Align) -> taffy::AlignItems {
    match a {
        Align::Start => taffy::AlignItems::FLEX_START,
        Align::Center => taffy::AlignItems::CENTER,
        Align::End => taffy::AlignItems::FLEX_END,
        Align::Stretch => taffy::AlignItems::STRETCH,
    }
}

fn to_taffy(s: &Style) -> taffy::Style {
    let lp = taffy::LengthPercentage::length;
    let lpa_px = taffy::LengthPercentageAuto::length;
    taffy::Style {
        display: if s.hidden {
            taffy::Display::None
        } else {
            taffy::Display::Flex
        },
        flex_direction: match s.direction {
            Direction::Row => taffy::FlexDirection::Row,
            Direction::Column => taffy::FlexDirection::Column,
        },
        flex_wrap: if s.wrap {
            taffy::FlexWrap::Wrap
        } else {
            taffy::FlexWrap::NoWrap
        },
        gap: taffy::Size {
            width: lp(s.gap),
            height: lp(s.gap),
        },
        padding: taffy::Rect {
            left: lp(s.padding.left),
            right: lp(s.padding.right),
            top: lp(s.padding.top),
            bottom: lp(s.padding.bottom),
        },
        margin: taffy::Rect {
            left: lpa_px(s.margin.left),
            right: lpa_px(s.margin.right),
            top: lpa_px(s.margin.top),
            bottom: lpa_px(s.margin.bottom),
        },
        size: taffy::Size {
            width: dimension(s.width),
            height: dimension(s.height),
        },
        min_size: taffy::Size {
            width: lpa(s.min_width),
            height: lpa(s.min_height),
        },
        max_size: taffy::Size {
            width: lpa(s.max_width),
            height: lpa(s.max_height),
        },
        align_items: s.align_items.map(align).unwrap_or(taffy::AlignItems::NORMAL),
        align_self: s.align_self.map(align),
        justify_content: match s.justify {
            Justify::Start => taffy::JustifyContent::FLEX_START,
            Justify::Center => taffy::JustifyContent::CENTER,
            Justify::End => taffy::JustifyContent::FLEX_END,
            Justify::SpaceBetween => taffy::JustifyContent::SPACE_BETWEEN,
            Justify::SpaceAround => taffy::JustifyContent::SPACE_AROUND,
            Justify::SpaceEvenly => taffy::JustifyContent::SPACE_EVENLY,
        },
        flex_grow: s.grow,
        flex_shrink: s.shrink,
        flex_basis: taffy::Dimension::AUTO,
        ..taffy::Style::default()
    }
}

/// 布局节点句柄。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(taffy::NodeId);

/// 测量叶子节点时的约束。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasureInput {
    /// 已确定的尺寸（含内边距）。两个都确定时通常直接返回即可。
    pub known_width: Option<f32>,
    pub known_height: Option<f32>,
    /// 内容区可用空间（已扣除内边距）。
    pub available_width: AvailableSpace,
    pub available_height: AvailableSpace,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AvailableSpace {
    Definite(f32),
    /// 求最小内容尺寸（例如文字能换行就尽量换行）。
    MinContent,
    /// 求最大内容尺寸（例如文字完全不换行）。
    MaxContent,
}

impl AvailableSpace {
    /// 确定值，或者 `None`。
    pub fn definite(self) -> Option<f32> {
        match self {
            AvailableSpace::Definite(v) => Some(v),
            _ => None,
        }
    }
}

fn from_taffy_space(a: taffy::AvailableSpace) -> AvailableSpace {
    match a {
        taffy::AvailableSpace::Definite(v) => AvailableSpace::Definite(v),
        taffy::AvailableSpace::MinContent => AvailableSpace::MinContent,
        taffy::AvailableSpace::MaxContent => AvailableSpace::MaxContent,
    }
}

/// 布局结果：相对父节点（边框盒左上角）的位置和尺寸，以及计算后的内边距。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub padding: Insets,
}

/// 布局树。`C` 是挂在节点上的上下文（上层用它找到对应的控件来测量）。
pub struct LayoutTree<C> {
    taffy: taffy::TaffyTree<C>,
}

impl<C> Default for LayoutTree<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> LayoutTree<C> {
    pub fn new() -> Self {
        let mut taffy = taffy::TaffyTree::new();
        taffy.disable_rounding();
        LayoutTree { taffy }
    }

    pub fn new_node(&mut self, style: &Style, context: C) -> NodeId {
        NodeId(
            self.taffy
                .new_leaf_with_context(to_taffy(style), context)
                .expect("taffy 新建节点不会失败"),
        )
    }

    pub fn set_style(&mut self, node: NodeId, style: &Style) {
        let _ = self.taffy.set_style(node.0, to_taffy(style));
    }

    pub fn set_children(&mut self, node: NodeId, children: &[NodeId]) {
        let ids: Vec<taffy::NodeId> = children.iter().map(|c| c.0).collect();
        let _ = self.taffy.set_children(node.0, &ids);
    }

    /// 删除节点（不递归，子节点需要调用方分别删除）。
    pub fn remove(&mut self, node: NodeId) {
        let _ = self.taffy.remove(node.0);
    }

    /// 节点的内容尺寸可能变了（例如文字改了），需要重新测量。会连带标记祖先节点。
    pub fn mark_dirty(&mut self, node: NodeId) {
        let _ = self.taffy.mark_dirty(node.0);
    }

    pub fn is_dirty(&self, node: NodeId) -> bool {
        self.taffy.dirty(node.0).unwrap_or(true)
    }

    /// 以 `root` 为根计算布局。`measure` 只对没有子节点的节点调用，返回内容尺寸（不含内边距）。
    pub fn compute(
        &mut self,
        root: NodeId,
        width: f32,
        height: f32,
        mut measure: impl FnMut(&mut C, MeasureInput) -> (f32, f32),
    ) {
        let available = taffy::Size {
            width: taffy::AvailableSpace::Definite(width),
            height: taffy::AvailableSpace::Definite(height),
        };
        let _ = self
            .taffy
            .compute_layout_with_measure(root.0, available, |inputs, _id, context, style| {
                taffy::compute_leaf_layout(
                    inputs,
                    style,
                    |_, _| 0.0,
                    |known, avail| match context {
                        Some(c) => {
                            let (w, h) = measure(
                                c,
                                MeasureInput {
                                    known_width: known.width,
                                    known_height: known.height,
                                    available_width: from_taffy_space(avail.width),
                                    available_height: from_taffy_space(avail.height),
                                },
                            );
                            taffy::Size { width: w, height: h }
                        }
                        None => taffy::Size::ZERO,
                    },
                )
            });
    }

    pub fn layout(&self, node: NodeId) -> Layout {
        match self.taffy.layout(node.0) {
            Ok(l) => Layout {
                x: l.location.x,
                y: l.location.y,
                width: l.size.width,
                height: l.size.height,
                padding: Insets::new(l.padding.top, l.padding.right, l.padding.bottom, l.padding.left),
            },
            Err(_) => Layout::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_with_padding_and_gap() {
        let mut tree = LayoutTree::<u32>::new();
        let a = tree.new_node(&Style::default(), 1);
        let b = tree.new_node(&Style::default(), 2);
        let root = tree.new_node(
            &Style {
                direction: Direction::Column,
                padding: Insets::all(10.0),
                gap: 5.0,
                width: Length::Px(200.0),
                height: Length::Px(100.0),
                align_items: Some(Align::Start),
                ..Style::default()
            },
            0,
        );
        tree.set_children(root, &[a, b]);
        tree.compute(root, 200.0, 100.0, |c, _| (10.0 * *c as f32, 20.0));
        assert_eq!(
            tree.layout(a),
            Layout {
                x: 10.0,
                y: 10.0,
                width: 10.0,
                height: 20.0,
                padding: Insets::ZERO
            }
        );
        assert_eq!(
            tree.layout(b),
            Layout {
                x: 10.0,
                y: 35.0,
                width: 20.0,
                height: 20.0,
                padding: Insets::ZERO
            }
        );
        assert!(!tree.is_dirty(a));
        tree.mark_dirty(a);
        assert!(tree.is_dirty(a) && tree.is_dirty(root));
    }

    #[test]
    fn percent_width_and_grow() {
        let mut tree = LayoutTree::<u32>::new();
        let full = tree.new_node(
            &Style {
                width: Length::Percent(100.0),
                ..Style::default()
            },
            0,
        );
        let grow = tree.new_node(
            &Style {
                grow: 1.0,
                ..Style::default()
            },
            0,
        );
        let col = tree.new_node(
            &Style {
                direction: Direction::Column,
                padding: Insets::all(4.0),
                ..Style::default()
            },
            0,
        );
        let row = tree.new_node(
            &Style {
                width: Length::Px(100.0),
                ..Style::default()
            },
            0,
        );
        tree.set_children(col, &[full]);
        tree.set_children(row, &[grow]);
        let root = tree.new_node(
            &Style {
                direction: Direction::Column,
                width: Length::Px(300.0),
                height: Length::Px(300.0),
                ..Style::default()
            },
            0,
        );
        tree.set_children(root, &[col, row]);
        tree.compute(root, 300.0, 300.0, |_, _| (0.0, 10.0));
        assert_eq!(tree.layout(full).width, 292.0);
        assert_eq!(tree.layout(grow).width, 100.0);
    }
}
