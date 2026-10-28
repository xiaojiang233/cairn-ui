//! Nocturne Glass 的无窗口测试：各缩放下的渲染结果（像素检查与截图）、悬停动画的帧数与重画范围、
//! 减弱动效。
//!
//! 设置环境变量 `CAIRN_SNAPSHOT_DIR` 时会把截图写成 PNG，便于人工检查。
//! `cargo test --release -p cairn-ui --test glass -- --nocapture` 会打印悬停动画一帧的耗时。

use std::time::{Duration, Instant};

use cairn_ui::prelude::*;
use cairn_ui::testing::Headless;
use cairn_ui::{DeviceRect, Point, Rect, Role, TextSystem, Theme};

/// 与 `examples/glass_card` 相同的界面。
struct Card;

impl Card {
    fn view(&self) -> impl View<Self> {
        column()
            .fill_width()
            .fill_height()
            .align(Align::Center)
            .justify(Justify::Center)
            .child(
                panel()
                    .width(320.0)
                    .child(
                        column()
                            .gap(6.0)
                            .child(label("Nocturne Glass").title())
                            .child(label("玻璃无色，色彩属于夜景").secondary()),
                    )
                    .child(
                        row()
                            .gap(10.0)
                            .child(button("Button").accent())
                            .child(icon_button(Icon::Dot)),
                    ),
            )
    }
}

fn fonts() -> TextSystem {
    TextSystem::new(&cairn_ui::testing::system_font_setup())
}

fn card(scale: f32, theme: Theme) -> Headless<Card> {
    Headless::with_theme(Card, Card::view, 640.0, 420.0, scale, theme, fonts())
}

fn snapshot(ui: &Headless<Card>, name: &str) {
    if let Some(dir) = std::env::var_os("CAIRN_SNAPSHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.png")), ui.png()).unwrap();
    }
}

/// 物理像素 (x, y) 的 RGB（画布不透明）。
fn px(ui: &Headless<Card>, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * ui.width() + x) * 4) as usize;
    let p = ui.pixels();
    [p[i], p[i + 1], p[i + 2]]
}

fn luma(c: [u8; 3]) -> f32 {
    0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32
}

/// 红减蓝：金色（暖）为正，夜景的蓝灰为负。
fn warmth(c: [u8; 3]) -> i32 {
    c[0] as i32 - c[2] as i32
}

/// 面板的边框盒（面板和 `column` 的角色都是 `Group`，用宽度区分）。
fn panel_bounds(ui: &Headless<Card>) -> Rect {
    ui.find_role(Role::Group)
        .into_iter()
        .map(|n| n.bounds)
        .find(|b| b.width == 320.0)
        .expect("面板")
}

fn area(regions: &[DeviceRect]) -> u64 {
    regions.iter().map(|r| r.width as u64 * r.height as u64).sum()
}

/// 面板（含两种状态的阴影和上浮）可能画到的范围：物理像素，外扩 `margin` 个逻辑像素。
fn panel_visual_device(bounds: Rect, scale: f32, margin: f32) -> DeviceRect {
    let theme = Theme::nocturne();
    let (a, b) = (theme.panel.shadow.extent(), theme.panel.shadow_hover.extent());
    let m = |i: usize| a[i].max(b[i]);
    let r = bounds.outset(m(0) + theme.panel.lift, m(1), m(2), m(3));
    DeviceRect::cover(&r.inflate(margin, margin), scale)
}

fn contains(outer: DeviceRect, inner: DeviceRect) -> bool {
    inner.x >= outer.x && inner.y >= outer.y && inner.right() <= outer.right() && inner.bottom() <= outer.bottom()
}

/// 面板悬停并上浮后，上边缘中点的探测位置（物理像素）：静止时这里是面板上方的夜景和阴影，
/// 悬停后正好落在最亮的边框行上。
fn lift_probe(panel: Rect, scale: f32) -> (u32, u32) {
    let lift = (Theme::nocturne().panel.lift * scale).round() as u32;
    (
        ((panel.x + panel.width / 2.0) * scale) as u32,
        (panel.y * scale).round() as u32 - lift,
    )
}

#[test]
fn card_renders_at_all_scales() {
    let mut text = Some(fonts());
    for (scale, tag) in [(1.0, "100"), (1.25, "125"), (1.5, "150")] {
        let theme = Theme::nocturne();
        let mut ui = Headless::with_theme(Card, Card::view, 640.0, 420.0, scale, theme, text.take().unwrap());
        let first = ui.render();
        let (w, h) = (ui.width(), ui.height());
        assert_eq!((w, h), ((640.0 * scale) as u32, (420.0 * scale) as u32));
        assert_eq!(first, vec![DeviceRect::new(0, 0, w, h)], "第一帧整窗绘制");
        snapshot(&ui, &format!("glass_card_{tag}"));
        let s = |v: f32| (v * scale) as u32;

        // 压暗的角（左上、右下）接近边缘色 #060A13：取角上 6×6 个像素的平均，抹掉噪点的随机部分
        // （2.5% 的噪点平均把暗部抬高约 3 个等级，右下角还有右上光斑衰减的尾巴）。
        for (x, y) in [(0, 0), (w - 6, h - 6)] {
            let mut sum = [0u32; 3];
            for (dx, dy) in (0..6).flat_map(|dy| (0..6).map(move |dx| (dx, dy))) {
                let c = px(&ui, x + dx, y + dy);
                (0..3).for_each(|i| sum[i] += c[i] as u32);
            }
            let avg = sum.map(|v| v as f32 / 36.0);
            for (got, want) in avg.iter().zip([6.0f32, 10.0, 19.0]) {
                assert!(
                    (want..=want + 10.0).contains(got),
                    "scale {scale}: 角落 ({x}, {y}) 平均 {avg:?}，边缘色是 (6, 10, 19)"
                );
            }
        }
        // 右上角的月光蓝光斑：比左上角亮，也偏蓝。
        let (tl, tr) = (px(&ui, s(40.0), s(40.0)), px(&ui, w - s(40.0), s(40.0)));
        assert!(luma(tr) > luma(tl) + 12.0, "scale {scale}: 左上 {tl:?} 右上 {tr:?}");
        assert!(tr[2] > tr[0], "光斑偏蓝: {tr:?}");
        // 左下角另一个光斑：比同样压暗的右下角亮。
        let (bl, br) = (px(&ui, s(40.0), h - s(40.0)), px(&ui, w - s(40.0), h - s(40.0)));
        assert!(luma(bl) > luma(br) + 6.0, "scale {scale}: 左下 {bl:?} 右下 {br:?}");

        // 面板里面（左边距处）比旁边的背景亮得多：模糊的背景 + 白色填充 + 内发光。
        let p = panel_bounds(&ui);
        let mid = p.y + p.height / 2.0;
        let (inside, outside) = (px(&ui, s(p.x + 6.0), s(mid)), px(&ui, s(p.x - 30.0), s(mid)));
        assert!(
            luma(inside) > luma(outside) + 25.0,
            "scale {scale}: 面板内 {inside:?} 面板外 {outside:?}"
        );

        // 金色按钮：上边框一行是暖色（金色 40% 叠在玻璃上，红 > 蓝），文字最亮的像素也是暖色。
        let b = ui.find("Button").expect("按钮").bounds;
        let edge = px(&ui, s(b.x + b.width / 2.0), (b.y * scale).round() as u32);
        assert!(warmth(edge) > 0, "scale {scale}: 按钮上边框 {edge:?}");
        let mut brightest = [0u8; 3];
        for y in s(b.y)..s(b.bottom()) {
            for x in s(b.x + 8.0)..s(b.right() - 8.0) {
                let c = px(&ui, x, y);
                if luma(c) > luma(brightest) {
                    brightest = c;
                }
            }
        }
        assert!(warmth(brightest) > 30, "scale {scale}: 按钮文字 {brightest:?}");

        // 图标按钮里的圆点是月光蓝：蓝明显大于红。
        let ib = ui.find("dot").expect("图标按钮").bounds;
        let dot = px(&ui, s(ib.x + ib.width / 2.0), s(ib.y + ib.height / 2.0));
        assert!(warmth(dot) < -30, "scale {scale}: 圆点 {dot:?}");

        // 没有变化就不画：静止的界面没有动画帧，也不需要唤醒事件循环。
        assert!(ui.render().is_empty());
        assert!(!ui.animating());
        assert!(ui.next_timer().is_none());
        text = Some(ui.into_text_system());
    }
}

#[test]
fn hover_animation_runs_500ms_repaints_only_the_panel_and_respects_reduced_motion() {
    let scale = 1.5;
    let mut ui = card(scale, Theme::nocturne());
    ui.render();
    let p = panel_bounds(&ui);
    let (x, probe_y) = lift_probe(p, scale);
    let before = luma(px(&ui, x, probe_y));

    let t0 = ui.now();
    // 指针放在面板内边距里（不在任何子控件上）。
    ui.pointer_move(Point::new(p.x + 4.0, p.y + 4.0));
    assert!(ui.redraw_requested(), "悬停触发一次重画");
    let first = ui.render();
    assert!(ui.animating(), "动画开始后要求下一帧");
    // 一帧一帧推进并计时（release 构建下可以看到每帧的耗时，见文件头）。
    let (mut frames, mut times) = (vec![first.clone()], Vec::new());
    while let Some(at) = ui.next_timer() {
        ui.fire_timers(at);
        let start = Instant::now();
        frames.push(ui.render());
        times.push(start.elapsed());
    }
    let elapsed = ui.now() - t0;
    let total: Duration = times.iter().sum();
    eprintln!(
        "悬停动画 @1.5x（{}）：{} 帧，每帧重画 {} 像素，平均每帧 {:?}，最慢 {:?}",
        if cfg!(debug_assertions) { "debug" } else { "release" },
        frames.len(),
        area(&first),
        total / times.len() as u32,
        times.iter().max().unwrap(),
    );

    // 500 ms，按 60 Hz 约 30 帧；之后不再有帧、不再唤醒。
    assert!(
        elapsed >= Duration::from_millis(490) && elapsed <= Duration::from_millis(540),
        "动画持续 {elapsed:?}"
    );
    assert!((28..=34).contains(&frames.len()), "{} 帧", frames.len());
    assert!(!ui.animating() && ui.next_timer().is_none() && !ui.redraw_requested());
    assert!(ui.render().is_empty(), "动画结束后没有要画的东西");

    // 每一帧只重画面板的可见范围（含阴影和上浮）：不是整窗，也不会超出。
    let limit = panel_visual_device(p, scale, 4.0);
    let window = u64::from(ui.width()) * u64::from(ui.height());
    for (i, regions) in frames.iter().enumerate() {
        assert!(!regions.is_empty(), "第 {i} 帧");
        for r in regions {
            assert!(contains(limit, *r), "第 {i} 帧: {r:?} 超出 {limit:?}");
        }
        assert!(
            area(regions) < window / 2,
            "第 {i} 帧重画了 {} / {window}",
            area(regions)
        );
    }
    // 面板上浮了 2 个逻辑像素，上缘的边框行落在探测位置。
    let after = luma(px(&ui, x, probe_y));
    assert!(after > before + 40.0, "面板上缘 {before} -> {after}");
    snapshot(&ui, "glass_card_hover_150");

    // 系统关闭了动画（主题的时长全为 0）：悬停的第一帧就是终态，没有后续帧。
    let mut theme = Theme::nocturne();
    theme.motion = theme.motion.reduced();
    let mut still = card(scale, theme);
    still.render();
    let before = luma(px(&still, x, probe_y));
    still.pointer_move(Point::new(p.x + 4.0, p.y + 4.0));
    assert!(!still.render().is_empty());
    assert!(!still.animating() && still.run_animations(10).is_empty() && still.next_timer().is_none());
    assert!(luma(px(&still, x, probe_y)) > before + 40.0, "第一帧就已经上浮");
}
