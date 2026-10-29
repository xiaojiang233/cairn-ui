//! 保留式控件树：节点存储、与新元素树的比对（reconcile）、布局与包围盒更新、命中测试。

use std::collections::HashMap;

use cairn_ui_layout::{Insets, LayoutTree, NodeId, Style};
use slotmap::SlotMap;

use crate::damage::Damage;
use crate::element::El;
use crate::theme::Theme;
use crate::widget::{AnyWidget, Change, LayoutCx, PaintCx, Widget};
use crate::{Painter, Point, Rect, Size};

slotmap::new_key_type! {
    /// 控件的稳定标识。控件存在期间不变；删除后不会被复用到别的控件上（带代数）。
    pub struct WidgetId;
}

impl WidgetId {
    /// 转成 `u64`，例如用作无障碍树的节点 id。
    pub fn to_raw(self) -> u64 {
        slotmap::Key::data(&self).as_ffi()
    }
}

pub(crate) struct Node<S: 'static> {
    pub widget: Box<dyn AnyWidget<S>>,
    pub layout: NodeId,
    pub parent: Option<WidgetId>,
    pub children: Vec<WidgetId>,
    pub key: Option<u64>,
    pub style: Style,
    /// 边框盒，窗口坐标（逻辑像素）。第一次布局前为空。
    pub bounds: Rect,
    pub padding: Insets,
    pub laid_out: bool,
    /// 布局引擎里这个节点用的是动画给出的高度（见 [`Widget::animated_height`]），不是 `style` 里的。
    pub animated: bool,
}

impl<S: 'static> Node<S> {
    /// 可能画到的范围（含阴影、外发光等），用于脏区。
    pub fn visual(&self, theme: &Theme) -> Rect {
        self.widget.visual_bounds(self.bounds, theme)
    }

    pub fn content(&self) -> Rect {
        let p = &self.padding;
        Rect::new(
            self.bounds.x + p.left,
            self.bounds.y + p.top,
            (self.bounds.width - p.horizontal()).max(0.0),
            (self.bounds.height - p.vertical()).max(0.0),
        )
    }
}

/// 树的根节点：铺满窗口，里面放 `view` 返回的元素。
struct Root;

impl<S: 'static> Widget<S> for Root {
    fn update(&mut self, _cx: &mut LayoutCx, _new: Self) -> Change {
        Change::NONE
    }
    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}
}

pub(crate) struct Tree<S: 'static> {
    pub nodes: SlotMap<WidgetId, Node<S>>,
    pub layout: LayoutTree<WidgetId>,
    pub root: WidgetId,
    /// 有节点的样式或内容尺寸变了，需要重新布局。
    pub needs_layout: bool,
    /// 只有子元素的位置要重新算（滚动量变了），尺寸都没变：不跑布局引擎，只重新定位各节点。
    pub needs_arrange: bool,
    /// 本轮比对中被删除的节点，运行时据此清理焦点、悬停等引用。
    pub removed: Vec<WidgetId>,
    /// 比对时节点新增、删除或内容 / 样式有变化的累计次数。运行时据此判断重建界面之后画面有没有变，
    /// 没变就不必让合成层缓存作废。
    pub changes: u64,
}

impl<S: 'static> Tree<S> {
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();
        let mut layout = LayoutTree::new();
        let style = Style::default();
        let root = nodes.insert_with_key(|id| Node {
            widget: Box::new(Root) as Box<dyn AnyWidget<S>>,
            layout: layout.new_node(&style, id),
            parent: None,
            children: Vec::new(),
            key: None,
            style,
            bounds: Rect::ZERO,
            padding: Insets::ZERO,
            laid_out: false,
            animated: false,
        });
        Tree {
            nodes,
            layout,
            root,
            needs_layout: true,
            needs_arrange: false,
            removed: Vec::new(),
            changes: 0,
        }
    }

    /// 根节点尺寸跟随窗口（逻辑像素）。
    pub fn set_viewport(&mut self, size: Size) {
        let style = Style {
            width: cairn_ui_layout::Length::Px(size.width),
            height: cairn_ui_layout::Length::Px(size.height),
            direction: cairn_ui_layout::Direction::Column,
            align_items: Some(cairn_ui_layout::Align::Stretch),
            ..Style::default()
        };
        let root = &mut self.nodes[self.root];
        if root.style != style {
            self.layout.set_style(root.layout, &style);
            root.style = style;
            self.needs_layout = true;
        }
    }

    /// 用新的元素树替换根节点下的内容。
    pub fn set_content(&mut self, cx: &mut LayoutCx, el: El<S>, damage: &mut Damage) {
        let root = self.root;
        self.reconcile_children(cx, root, vec![el], damage);
    }

    fn build(&mut self, cx: &mut LayoutCx, el: El<S>, parent: WidgetId) -> WidgetId {
        let El {
            widget,
            mut style,
            children,
            key,
        } = el;
        resolve_style(&*widget, &mut style, cx.theme);
        let layout = &mut self.layout;
        let id = self.nodes.insert_with_key(|id| Node {
            widget,
            layout: layout.new_node(&style, id),
            parent: Some(parent),
            children: Vec::new(),
            key,
            style,
            bounds: Rect::ZERO,
            padding: Insets::ZERO,
            laid_out: false,
            animated: false,
        });
        let child_ids: Vec<WidgetId> = children.into_iter().map(|c| self.build(cx, c, id)).collect();
        let layout_children: Vec<NodeId> = child_ids.iter().map(|c| self.nodes[*c].layout).collect();
        self.layout.set_children(self.nodes[id].layout, &layout_children);
        self.nodes[id].children = child_ids;
        self.needs_layout = true;
        self.changes += 1;
        id
    }

    /// 同类型就就地更新，返回 `Err` 表示类型不同、需要重建。
    // Err 里原样退回元素（不是错误信息），装箱只会多一次分配，所以不理会 result_large_err。
    #[allow(clippy::result_large_err)]
    fn update(&mut self, cx: &mut LayoutCx, id: WidgetId, el: El<S>, damage: &mut Damage) -> Result<(), El<S>> {
        let El {
            widget,
            mut style,
            children,
            key,
        } = el;
        resolve_style(&*widget, &mut style, cx.theme);
        let node = &mut self.nodes[id];
        let change = match node.widget.update_any(cx, widget) {
            Ok(change) => change,
            Err(widget) => {
                return Err(El {
                    widget,
                    style,
                    children,
                    key,
                });
            }
        };
        if change.layout {
            self.layout.mark_dirty(node.layout);
            self.needs_layout = true;
        }
        if change.layout || change.paint {
            self.changes += 1;
        }
        let visual = (change.paint && node.laid_out).then(|| node.visual(cx.theme));
        if node.style != style {
            self.layout.set_style(node.layout, &style);
            node.style = style;
            self.needs_layout = true;
            self.changes += 1;
        }
        if let Some(visual) = visual {
            add_clipped(damage, visual, self.clip_of(id, cx.theme));
        }
        self.reconcile_children(cx, id, children, damage);
        Ok(())
    }

    /// 子元素比对：带 key 的按 key 匹配，不带 key 的按出现顺序匹配；匹配上且类型相同就复用。
    fn reconcile_children(&mut self, cx: &mut LayoutCx, parent: WidgetId, new: Vec<El<S>>, damage: &mut Damage) {
        let old = std::mem::take(&mut self.nodes[parent].children);
        let mut keyed: HashMap<u64, WidgetId> = HashMap::new();
        let mut unkeyed = Vec::new();
        for &id in &old {
            match self.nodes[id].key {
                Some(k) => {
                    keyed.insert(k, id);
                }
                None => unkeyed.push(id),
            }
        }
        let mut unkeyed = unkeyed.into_iter();

        let mut result = Vec::with_capacity(new.len());
        for el in new {
            let candidate = match el.key {
                Some(k) => keyed.remove(&k),
                None => unkeyed.next(),
            };
            let id = match candidate {
                Some(old_id) => match self.update(cx, old_id, el, damage) {
                    Ok(()) => old_id,
                    Err(el) => {
                        self.remove_subtree(cx.theme, old_id, damage);
                        self.build(cx, el, parent)
                    }
                },
                None => self.build(cx, el, parent),
            };
            result.push(id);
        }
        for id in keyed.into_values().chain(unkeyed) {
            self.remove_subtree(cx.theme, id, damage);
        }
        if result != old {
            let layout_children: Vec<NodeId> = result.iter().map(|c| self.nodes[*c].layout).collect();
            self.layout.set_children(self.nodes[parent].layout, &layout_children);
            self.needs_layout = true;
            self.changes += 1;
        }
        self.nodes[parent].children = result;
    }

    fn remove_subtree(&mut self, theme: &Theme, id: WidgetId, damage: &mut Damage) {
        let clip = self.clip_of(id, theme);
        self.remove_subtree_in(theme, id, clip, damage);
    }

    /// `clip` 是 `id` 被祖先裁剪到的范围；子树里每个裁剪子元素的节点再叠加自己那一层。
    fn remove_subtree_in(&mut self, theme: &Theme, id: WidgetId, clip: Option<Rect>, damage: &mut Damage) {
        let Some(node) = self.nodes.remove(id) else { return };
        if node.laid_out {
            add_clipped(damage, node.visual(theme), clip);
        }
        self.layout.remove(node.layout);
        self.removed.push(id);
        self.changes += 1;
        let child_clip = if node.laid_out && node.widget.clip_children(theme).is_some() {
            narrow(clip, node.bounds)
        } else {
            clip
        };
        for c in node.children {
            self.remove_subtree_in(theme, c, child_clip, damage);
        }
        self.needs_layout = true;
    }

    /// 需要时重新计算布局并更新各节点的包围盒，位置或尺寸变化的节点新旧区域都计入脏区。
    /// 返回是否真的做了事（布局或重新定位）。
    ///
    /// 只有 `needs_arrange`（滚动量变了）时不跑布局引擎，沿用上次的布局结果直接重新定位。
    pub fn compute_layout(&mut self, cx: &mut LayoutCx, damage: &mut Damage) -> bool {
        if !self.needs_layout && !self.needs_arrange {
            return false;
        }
        if std::mem::take(&mut self.needs_layout) {
            self.apply_animated_heights();
            let root = &self.nodes[self.root];
            let (w, h) = (root.bounds.width, root.bounds.height);
            let (w, h) = match (&root.style.width, &root.style.height) {
                (cairn_ui_layout::Length::Px(w), cairn_ui_layout::Length::Px(h)) => (*w, *h),
                _ => (w, h),
            };
            let root_layout = root.layout;
            let nodes = &mut self.nodes;
            self.layout.compute(root_layout, w, h, |id, input| {
                let size = nodes[*id].widget.measure(cx, &input);
                (size.width, size.height)
            });
        }
        self.needs_arrange = false;
        let root = self.root;
        self.update_bounds(cx, root, Point::ZERO, None, damage);
        true
    }

    /// 高度在动画中的节点（[`Widget::animated_height`]）：布局前把动画给的高度写进布局引擎，
    /// 动画结束后把样式里原来的高度还回去。
    fn apply_animated_heights(&mut self) {
        for (_, node) in self.nodes.iter_mut() {
            match node.widget.animated_height() {
                Some(h) => {
                    let mut style = node.style.clone();
                    style.height = cairn_ui_layout::Length::Px(h);
                    style.grow = 0.0;
                    style.shrink = 0.0;
                    self.layout.set_style(node.layout, &style);
                    node.animated = true;
                }
                None if node.animated => {
                    self.layout.set_style(node.layout, &node.style);
                    node.animated = false;
                }
                None => {}
            }
        }
    }

    /// 定位 `id` 及其子树。`origin` 是父节点给子节点的原点（父节点的边框盒左上角减去 [`Widget::child_offset`]），
    /// `clip` 是所有祖先的裁剪范围之交（没有裁剪子元素的祖先时为 `None`）。
    ///
    /// 记入脏区的矩形都先与 `clip` 求交：滚出视口的节点位置变了也画不出来，不能把它们的新旧位置都记进去——
    /// 几百个屏幕外的行会把脏区（最多 8 块，超过就合并成外接矩形）撑成一大片。包围盒和 `laid_out` 照常更新。
    fn update_bounds(
        &mut self,
        cx: &mut LayoutCx,
        id: WidgetId,
        origin: Point,
        clip: Option<Rect>,
        damage: &mut Damage,
    ) {
        let l = self.layout.layout(self.nodes[id].layout);
        let node = &mut self.nodes[id];
        let bounds = Rect::new(origin.x + l.x, origin.y + l.y, l.width, l.height);
        if !node.laid_out || bounds != node.bounds {
            if node.laid_out {
                add_clipped(damage, node.visual(cx.theme), clip);
            }
            node.bounds = bounds;
            node.laid_out = true;
            add_clipped(damage, node.visual(cx.theme), clip);
        }
        node.padding = l.padding;
        let content = node.content();
        node.widget.arrange(cx, bounds, content);
        if self.nodes[id].children.is_empty() {
            return;
        }
        let content_size = self.content_size(id, l.padding);
        let node = &mut self.nodes[id];
        node.widget.arrange_content(cx, content_size);
        let offset = node.widget.child_offset();
        let origin = Point::new(bounds.x - offset.x, bounds.y - offset.y);
        // 本节点裁剪子元素：子元素（及其子孙）只可能画在本节点的边框盒里。
        let clip = if node.widget.clip_children(cx.theme).is_some() {
            narrow(clip, bounds)
        } else {
            clip
        };
        for i in 0..self.nodes[id].children.len() {
            let child = self.nodes[id].children[i];
            self.update_bounds(cx, child, origin, clip, damage);
        }
    }

    /// 可见子元素占据的范围，从 `id` 的边框盒左上角算起：每个子元素的“位置 + 尺寸 + 右 / 下外边距”的最大值，
    /// 加上 `id` 自己的右 / 下内边距（与 CSS 的 `scrollWidth` / `scrollHeight` 一致）。隐藏的子元素不算。
    fn content_size(&self, id: WidgetId, padding: Insets) -> Size {
        let (mut w, mut h) = (0.0f32, 0.0f32);
        for &c in &self.nodes[id].children {
            let child = &self.nodes[c];
            if child.style.hidden {
                continue;
            }
            let l = self.layout.layout(child.layout);
            w = w.max(l.x + l.width + child.style.margin.right);
            h = h.max(l.y + l.height + child.style.margin.bottom);
        }
        Size::new(w + padding.right, h + padding.bottom)
    }

    /// 节点 `id` 的 `rect` 范围需要重画：去掉被祖先裁剪掉的部分后记入脏区。
    pub fn damage_rect(&self, id: WidgetId, rect: Rect, theme: &Theme, damage: &mut Damage) {
        add_clipped(damage, rect, self.clip_of(id, theme));
    }

    /// 节点被祖先裁剪到的范围（窗口坐标）：所有 `clip_children()` 为 `Some` 的祖先的边框盒之交；
    /// 没有这样的祖先时为 `None`。用于把脏区限制在画得出来的部分。
    fn clip_of(&self, id: WidgetId, theme: &Theme) -> Option<Rect> {
        let mut clip = None;
        let mut cur = self.nodes.get(id).and_then(|n| n.parent);
        while let Some(p) = cur {
            let Some(n) = self.nodes.get(p) else { break };
            if n.laid_out && n.widget.clip_children(theme).is_some() {
                clip = narrow(clip, n.bounds);
            }
            cur = n.parent;
        }
        clip
    }

    /// 节点的所有子孙（不含它自己）可能画到的范围之并（被裁剪子元素的祖先裁掉的部分不算），
    /// 用于带变换或半透明的控件在动画中重画整棵子树。
    pub fn descendants_visual(&self, id: WidgetId, theme: &Theme) -> Option<Rect> {
        let node = self.nodes.get(id)?;
        let clip = self.clip_of(id, theme);
        let clip = if node.laid_out && node.widget.clip_children(theme).is_some() {
            narrow(clip, node.bounds)
        } else {
            clip
        };
        let mut out: Option<Rect> = None;
        let mut stack: Vec<(WidgetId, Option<Rect>)> = node.children.iter().map(|c| (*c, clip)).collect();
        while let Some((n, clip)) = stack.pop() {
            let Some(node) = self.nodes.get(n) else { continue };
            if node.style.hidden || !node.laid_out {
                continue;
            }
            let v = node.visual(theme);
            let v = match clip {
                Some(c) => v.intersection(&c),
                None => Some(v),
            };
            if let Some(v) = v {
                out = Some(out.map_or(v, |o| o.union(&v)));
            }
            let child_clip = if node.widget.clip_children(theme).is_some() {
                narrow(clip, node.bounds)
            } else {
                clip
            };
            stack.extend(node.children.iter().map(|c| (*c, child_clip)));
        }
        out
    }

    /// 包含 `p` 的最深层节点（后画的在上面，优先命中）。
    pub fn hit_test(&self, p: Point, theme: &Theme) -> Option<WidgetId> {
        self.hit(self.root, p, theme)
    }

    fn hit(&self, id: WidgetId, p: Point, theme: &Theme) -> Option<WidgetId> {
        let node = &self.nodes[id];
        if node.style.hidden {
            return None;
        }
        let inside = id == self.root || node.bounds.contains(p);
        // 画在子元素上面的东西（滚动条）先于子元素接收指针。
        if inside && node.widget.intercepts_pointer(p, node.bounds, theme) {
            return Some(id);
        }
        // 子节点可能超出父节点，所以不裁剪子元素的控件不先判断父节点是否包含 p。
        if inside || node.widget.clip_children(theme).is_none() {
            for &c in node.children.iter().rev() {
                if let Some(hit) = self.hit(c, p, theme) {
                    return Some(hit);
                }
            }
        }
        inside.then_some(id)
    }

    /// 从 `id` 到根节点的路径（含两端）。
    pub fn ancestors(&self, id: WidgetId) -> Vec<WidgetId> {
        let mut path = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            path.push(c);
            cur = self.nodes.get(c).and_then(|n| n.parent);
        }
        path
    }

    /// 深度优先（先父后子）的节点顺序，即 Tab 顺序。隐藏的子树被跳过。
    pub fn visible_in_order(&self) -> Vec<WidgetId> {
        let mut out = Vec::new();
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            if node.style.hidden {
                continue;
            }
            out.push(id);
            stack.extend(node.children.iter().rev());
        }
        out
    }
}

/// 把脏区矩形限制在 `clip`（祖先的裁剪范围之交）内再记录：被裁掉的部分画不出来，不用重画；
/// 完全被裁掉就不记录。`clip` 为 `None` 表示没有被裁剪。
fn add_clipped(damage: &mut Damage, rect: Rect, clip: Option<Rect>) {
    match clip {
        None => damage.add(rect),
        Some(clip) => {
            if let Some(r) = rect.intersection(&clip) {
                damage.add(r);
            }
        }
    }
}

/// 在已有的裁剪范围上再叠加一层 `bounds`。交集为空时得到零尺寸矩形——
/// 它与任何矩形求交都是空的，所以“完全被裁掉”在子树里一直保持。
fn narrow(clip: Option<Rect>, bounds: Rect) -> Option<Rect> {
    Some(match clip {
        None => bounds,
        Some(c) => c.intersection(&bounds).unwrap_or(Rect::ZERO),
    })
}

/// 让控件按主题补全样式里的占位值，剩下的占位值按 0 处理（不能把 NaN 交给布局引擎）。
fn resolve_style<S: 'static>(widget: &dyn AnyWidget<S>, style: &mut Style, theme: &Theme) {
    widget.resolve_style(style, theme);
    for v in [
        &mut style.padding.top,
        &mut style.padding.right,
        &mut style.padding.bottom,
        &mut style.padding.left,
        &mut style.margin.top,
        &mut style.margin.right,
        &mut style.margin.bottom,
        &mut style.margin.left,
        &mut style.gap,
    ] {
        if v.is_nan() {
            *v = 0.0;
        }
    }
}
