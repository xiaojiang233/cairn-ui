//! 展开 / 收起容器：高度随动画变化，下面的兄弟元素跟着移动，展开后高度重新交给内容。

use std::time::Duration;

use cairn_ui::testing::Headless;
use cairn_ui::{Align, El, Styled, collapse, column, label};

#[derive(Default)]
struct State {
    open: bool,
    /// 收起动画播完时置位。
    collapsed: u32,
    /// 展开的内容再多出来的行数(模拟内容异步加载)。
    extra: usize,
}

fn view(s: &State) -> El<State> {
    let mut list = collapse(s.open).on_collapsed(|s: &mut State| s.collapsed += 1);
    for i in 0..3 + s.extra {
        list = list.child(label(format!("条目{i}")).height(20.0));
    }
    column().align(Align::Stretch).child(label("上面")).child(list).child(label("下面")).into()
}

fn ui() -> Headless<State> {
    Headless::with_theme(
        State::default(),
        view,
        300.0,
        400.0,
        1.0,
        cairn_ui::Theme::nocturne(),
        cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()),
    )
}

fn below(ui: &Headless<State>) -> f32 {
    ui.find("下面").expect("下面的标签").bounds.y
}

#[test]
fn height_grows_with_the_animation_and_pushes_siblings() {
    let mut ui = ui();
    ui.render();
    let closed = below(&ui);
    // 收起时子元素不占高度。
    assert!(ui.find("条目0").is_none() || ui.find("条目0").unwrap().bounds.height <= 20.0);

    ui.update(|s| s.open = true);
    ui.render();
    let mut seen = Vec::new();
    for _ in 0..40 {
        if !ui.animating() {
            break;
        }
        ui.run_animations(1);
        seen.push(below(&ui));
    }
    let open = below(&ui);
    // 完全展开:三行,每行 20,加上行间距。
    assert!(open - closed >= 60.0 - 0.5, "展开后下面的标签只下移了 {}", open - closed);
    // 过程中的高度单调不减,并且确实经过了中间值(不是一下子跳到位)。
    assert!(seen.windows(2).all(|w| w[1] >= w[0] - 0.01), "{seen:?}");
    assert!(seen.iter().any(|y| *y > closed + 1.0 && *y < open - 1.0), "没有中间状态:{seen:?}");
    assert!(!ui.animating(), "动画结束后不再请求帧");
}

#[test]
fn collapsing_shrinks_back_and_reports_when_done() {
    let mut ui = ui();
    ui.update(|s| s.open = true);
    ui.render();
    ui.run_animations(200);
    let open = below(&ui);
    assert_eq!(ui.state().collapsed, 0);

    ui.update(|s| s.open = false);
    ui.render();
    let mut seen = Vec::new();
    for _ in 0..40 {
        if !ui.animating() {
            break;
        }
        ui.run_animations(1);
        seen.push(below(&ui));
    }
    ui.advance(Duration::from_millis(50));
    ui.render();
    let closed = below(&ui);
    assert!(closed < open - 50.0, "收起后下面的标签没有回来:{open} → {closed}");
    assert!(seen.iter().any(|y| *y < open - 1.0 && *y > closed + 1.0), "没有中间状态:{seen:?}");
    assert_eq!(ui.state().collapsed, 1, "收起完成的回调只触发一次");
}

#[test]
fn after_opening_the_content_decides_the_height_again() {
    let mut ui = ui();
    ui.update(|s| s.open = true);
    ui.render();
    ui.run_animations(200);
    let before = below(&ui);
    // 展开完成后内容变多,面板应该跟着撑高(高度不是被动画锁死的)。
    ui.update(|s| s.extra = 2);
    ui.render();
    let after = below(&ui);
    assert!(after - before >= 40.0 - 0.5, "多了两行,下面的标签只下移了 {}", after - before);
}
