//! 滚动容器的无窗口测试：走完整运行时（比对、布局、事件、命中测试、脏区重画）。
//!
//! 滚动移动的是子元素真实的窗口坐标，所以大部分检查都是直接读控件的边框盒、按坐标点击、看像素。

use std::time::Duration;

use cairn_ui::prelude::*;
use cairn_ui::testing::Headless;
use cairn_ui::{DeviceRect, Point, PointerButton, Rect, Role, ScrollDelta, TextSystem, Theme};

/// 滚轮向下 / 向上一格（winit：向上滚 `y` 为正）。
const DOWN: ScrollDelta = ScrollDelta::Lines { x: 0.0, y: -1.0 };
const UP: ScrollDelta = ScrollDelta::Lines { x: 0.0, y: 1.0 };

fn fonts() -> TextSystem {
    TextSystem::new(&cairn_ui::testing::system_font_setup())
}

/// 第 `n` 个滚动容器（按树的先后顺序）的视口边框盒。
fn viewport<S: 'static>(ui: &Headless<S>, n: usize) -> Rect {
    ui.find_role(Role::ScrollArea)[n].bounds
}

fn bounds<S: 'static>(ui: &Headless<S>, label: &str) -> Rect {
    ui.find(label).unwrap_or_else(|| panic!("找不到 {label}")).bounds
}

/// 物理像素 (x, y) 的 RGB（缩放 1，画布不透明）。
fn rgb<S: 'static>(ui: &Headless<S>, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * ui.width() + x) * 4) as usize;
    let p = ui.pixels();
    [p[i], p[i + 1], p[i + 2]]
}

/// 一个 300 × 200 的视口，里面 `rows` 个 30 高的按钮；`pad` 是随内容滚动的内边距。窗口 340 × 240，视口在 (20, 20)。
struct List {
    rows: usize,
    pad: f32,
    clicked: Option<usize>,
}

impl List {
    fn view(&self) -> impl View<Self> {
        column()
            .padding(20.0)
            .child(
                scroll()
                    .width(300.0)
                    .height(200.0)
                    .padding(self.pad)
                    .children((0..self.rows).map(|i| {
                        button(format!("Row {i}"))
                            .fill_width()
                            .height(30.0)
                            .on_click(move |app: &mut Self| app.clicked = Some(i))
                    })),
            )
    }

    fn start(theme: Theme, pad: f32) -> Headless<List> {
        let state = List {
            rows: 40,
            pad,
            clicked: None,
        };
        let mut ui = Headless::with_theme(state, List::view, 340.0, 240.0, 1.0, theme, fonts());
        ui.render();
        ui
    }
}

/// 外层视口 200 高，里面先是一个 100 高的内层滚动容器（10 行，共 300），再是 20 行。
struct Nested;

impl Nested {
    fn view(&self) -> impl View<Self> {
        column().padding(20.0).child(
            scroll()
                .width(300.0)
                .height(200.0)
                .child(
                    scroll()
                        .fill_width()
                        .height(100.0)
                        .children((0..10).map(|i| button(format!("Inner {i}")).fill_width().height(30.0))),
                )
                .children((0..20).map(|i| button(format!("Tail {i}")).fill_width().height(30.0))),
        )
    }
}

/// 视口上方、下方各有别的控件，视口里是一百行：行的真实位置一直延伸到视口下面的页脚上。
struct Page;

impl Page {
    fn view(&self) -> impl View<Self> {
        column()
            .padding(20.0)
            .gap(10.0)
            .child(label("Header"))
            .child(
                scroll()
                    .fill_width()
                    .height(200.0)
                    .children((0..100).map(|i| button(format!("Row {i}")).fill_width().height(30.0))),
            )
            .child(label("Footer"))
    }
}

#[test]
fn wheel_moves_real_bounds_so_hits_clips_and_hover_follow() {
    let mut ui = List::start(Theme::light(), 0.0);
    let view = viewport(&ui, 0);
    assert_eq!(view, Rect::new(20.0, 20.0, 300.0, 200.0));
    let step = Theme::light().scroll.wheel_step;
    assert_eq!(bounds(&ui, "Row 0"), Rect::new(20.0, 20.0, 300.0, 30.0));

    // 指针放在视口里，滚一格：所有按钮的真实位置上移 `wheel_step`。浅色主题的滚动是瞬时的。
    ui.pointer_move(Point::new(60.0, 120.0));
    ui.wheel(DOWN);
    assert_eq!(bounds(&ui, "Row 0").y, view.y - step);
    assert_eq!(bounds(&ui, "Row 3").y, view.y + 90.0 - step);

    // 点击按坐标命中滚动之后的按钮：视口顶端 +10 处现在是第 2 行（内容坐标 66，行高 30）。
    ui.click(Point::new(60.0, view.y + 10.0));
    assert_eq!(ui.state().clicked, Some(2));
    ui.update(|s| s.clicked = None);

    // 被视口裁掉的部分点不到：第 8 行的边框盒跨过视口下沿，下沿以外的点不属于它。
    let row8 = bounds(&ui, "Row 8");
    assert!(row8.y < view.bottom() && row8.bottom() > view.bottom());
    ui.click(Point::new(60.0, view.bottom() + 5.0));
    assert_eq!(ui.state().clicked, None);

    // 悬停跟着内容走：指针不动，滚动后它下面换成了另一行——新的一行亮起、原来那行恢复。
    // 在每行靠左的位置取色（避开居中的文字）；浅色主题的悬停是瞬时的。
    let (theme, color) = (Theme::light(), |c: Color| [c.r, c.g, c.b]);
    ui.pointer_move(Point::new(60.0, 120.0));
    ui.render();
    assert_eq!(rgb(&ui, 28, 120), color(theme.button.fill_hover), "指针下的第 5 行亮着");
    ui.wheel(DOWN);
    ui.render();
    assert_eq!(
        rgb(&ui, 28, 120),
        color(theme.button.fill_hover),
        "滚动后指针下换成新的一行，它亮起"
    );
    let old = bounds(&ui, "Row 5").center();
    assert!(old.y < 120.0 - 15.0, "原来那行已经被带到指针上面");
    assert_eq!(rgb(&ui, 28, old.y as u32), color(theme.button.fill), "原来那行恢复");

    // 触控板的像素增量直接生效（物理像素 ÷ 缩放），不走动画。
    let before = bounds(&ui, "Row 0").y;
    ui.wheel(ScrollDelta::Pixels { x: 0.0, y: -30.0 });
    assert_eq!(bounds(&ui, "Row 0").y, before - 30.0);
}

#[test]
fn wheel_animates_clamps_at_both_ends_and_chains_to_the_outer_container() {
    let mut theme = Theme::light();
    theme.motion.scroll = Duration::from_millis(200);
    let step = theme.scroll.wheel_step;
    // 内容列有 8 像素内边距：滚到底时最后一行下面还要留着它。
    let mut ui = List::start(theme.clone(), 8.0);
    let view = viewport(&ui, 0);
    let top = view.y + 8.0;
    ui.pointer_move(Point::new(60.0, 120.0));

    // 滚一格后位置不会立刻跳变，而是随时间过渡；中途再来一格从当前位置继续，最后停在两格处。
    ui.wheel(DOWN);
    assert_eq!(bounds(&ui, "Row 0").y, top);
    ui.advance(Duration::from_millis(100));
    let mid = bounds(&ui, "Row 0").y;
    assert!(mid < top && mid > top - step, "过渡中: {mid}");
    ui.wheel(DOWN);
    ui.advance(Duration::from_millis(400));
    assert_eq!(bounds(&ui, "Row 0").y, top - 2.0 * step);

    // 转很多格：停在底部，最后一行的下沿加内边距正好是视口下沿；反方向转过头则停在 0。
    for _ in 0..40 {
        ui.wheel(DOWN);
    }
    ui.advance(Duration::from_secs(2));
    assert_eq!(bounds(&ui, "Row 39").bottom() + 8.0, view.bottom());
    for _ in 0..60 {
        ui.wheel(UP);
    }
    ui.advance(Duration::from_secs(2));
    assert_eq!(bounds(&ui, "Row 0").y, top);
    assert!(ui.next_timer().is_none(), "到位之后不再有定时器");

    // 内容变短到放得下：滚动量被限制回 0；之后内容再变长，也不会冒出缩短之前的滚动量。
    for _ in 0..40 {
        ui.wheel(DOWN);
    }
    ui.advance(Duration::from_secs(2));
    ui.update(|s| s.rows = 5);
    assert_eq!(bounds(&ui, "Row 0").y, top);
    ui.update(|s| s.rows = 40);
    assert_eq!(bounds(&ui, "Row 0").y, top);

    // 嵌套：内层滚到头之后，多出来的滚轮交给外层。第 5 格来时内层的过渡还没走完，仍由内层消费，
    // 外层不会趁内层没停稳就动起来。
    let mut ui = Headless::with_theme(Nested, Nested::view, 340.0, 240.0, 1.0, theme, fonts());
    ui.render();
    let (outer, inner) = (viewport(&ui, 0), viewport(&ui, 1));
    assert_eq!(inner.height, 100.0);
    ui.pointer_move(Point::new(60.0, inner.y + 40.0));
    for _ in 0..5 {
        ui.wheel(DOWN);
    }
    ui.advance(Duration::from_secs(1));
    assert_eq!(bounds(&ui, "Inner 9").bottom(), inner.bottom(), "内层滚到了头");
    assert_eq!(bounds(&ui, "Tail 0").y, outer.y + 100.0, "外层还没动");
    ui.wheel(DOWN);
    ui.advance(Duration::from_secs(1));
    assert_eq!(bounds(&ui, "Tail 0").y, outer.y + 100.0 - step, "内层停稳之后外层接手");
}

#[test]
fn scrolling_repaints_only_inside_the_viewport() {
    // 100% 与 125%：分数缩放下脏区换算成物理像素时向外取整，也不能越出视口。
    for scale in [1.0f32, 1.25] {
        let mut ui = Headless::with_theme(Page, Page::view, 340.0, 340.0, scale, Theme::light(), fonts());
        let size = (340.0 * scale).round() as u32;
        assert_eq!(ui.render(), vec![DeviceRect::new(0, 0, size, size)]);
        let view = viewport(&ui, 0);
        let footer = bounds(&ui, "Footer");
        assert!(footer.y >= view.bottom(), "页脚在视口下面，行的真实位置会盖过它");

        // 先让指针进入视口并画完，再滚：之后每次重画都不能越出视口（外扩 4 像素的抗锯齿余量），
        // 哪怕有几十个屏幕外的行都动了。
        ui.pointer_move(Point::new(60.0, view.y + 100.0));
        ui.render();
        let limit = DeviceRect::cover(&view.inflate(4.0, 4.0), scale);
        for delta in [DOWN, DOWN, ScrollDelta::Pixels { x: 0.0, y: -13.0 }, UP] {
            ui.wheel(delta);
            let regions = ui.render();
            assert!(!regions.is_empty(), "内容动了，视口要重画");
            for r in &regions {
                let inside =
                    r.x >= limit.x && r.y >= limit.y && r.right() <= limit.right() && r.bottom() <= limit.bottom();
                assert!(inside, "缩放 {scale}: {r:?} 越出了视口 {limit:?}");
            }
        }
    }
}

#[test]
fn dragging_the_thumb_and_clicking_the_track_scroll() {
    let mut ui = List::start(Theme::light(), 0.0);
    let view = viewport(&ui, 0);
    let look = Theme::light().scroll;
    // 内容 1200、视口 200：可滚动 1000。滑块长度正比于视口占内容的比例，在轨道里活动的距离是 `free`。
    let (track_y, track) = (view.y + look.bar_inset, view.height - 2.0 * look.bar_inset);
    let thumb = (track * view.height / 1200.0).max(look.min_thumb);
    let free = track - thumb;
    let x = view.right() - look.bar_inset - look.bar_width / 2.0;
    let row0 = |ui: &Headless<List>| bounds(ui, "Row 0").y;

    // 按住滑块往下拖半程：保持抓取位置，滚动量是一半，拖动过程中内容立刻跟手。
    ui.pointer_move(Point::new(x, track_y + thumb / 2.0));
    ui.pointer_button(PointerButton::Left, true);
    ui.pointer_move(Point::new(x, track_y + thumb / 2.0 + free / 2.0));
    assert!((row0(&ui) - (view.y - 500.0)).abs() <= 1.0, "{}", row0(&ui));
    // 拖过头停在末端（最后一行贴着视口下沿），拖回去停在 0；松开后滚动条上的点击不会落到下面的按钮上。
    ui.pointer_move(Point::new(x, view.bottom() + 300.0));
    assert_eq!(bounds(&ui, "Row 39").bottom(), view.bottom());
    ui.pointer_move(Point::new(x, view.y - 300.0));
    assert_eq!(row0(&ui), view.y);
    ui.pointer_button(PointerButton::Left, false);
    assert_eq!(ui.state().clicked, None);

    // 点击滑块下面的轨道翻一页（视口高度的 90%），点击上面的翻回来。
    ui.click(Point::new(x, view.bottom() - 4.0));
    assert_eq!(row0(&ui), view.y - 180.0);
    ui.click(Point::new(x, track_y + 1.0));
    assert_eq!(row0(&ui), view.y);
    assert_eq!(ui.state().clicked, None, "滚动条盖在按钮上面，点击归滚动条");

    // 拖动到一半窗口失去了焦点（运行时放弃指针独占，松开事件再也收不到）：回来之后在内容上
    // 移动指针不能再把内容拖着走。
    ui.pointer_move(Point::new(x, track_y + thumb / 2.0));
    ui.pointer_button(PointerButton::Left, true);
    ui.pointer_move(Point::new(x, track_y + thumb / 2.0 + 40.0));
    let dragged = row0(&ui);
    assert!(dragged < view.y);
    ui.set_window_focused(false);
    ui.set_window_focused(true);
    ui.pointer_move(Point::new(100.0, view.y + 60.0));
    ui.pointer_move(Point::new(100.0, view.y + 120.0));
    assert_eq!(row0(&ui), dragged);
}
