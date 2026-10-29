//! 无边框窗口的标题栏部件：窗口按钮。

use cairn_ui::testing::Headless;
use cairn_ui::{El, Point, row, window_controls};

fn bar(_: &()) -> El<()> {
    row().child(window_controls()).into()
}

#[test]
fn window_buttons_minimize_maximize_and_close() {
    let theme = cairn_ui::Theme::nocturne();
    let (w, h) = (theme.caption.width, theme.caption.height);
    let mut ui = Headless::with_theme(
        (),
        bar,
        400.0,
        100.0,
        1.0,
        theme,
        cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()),
    );
    ui.render();
    ui.click(Point::new(w * 0.5, h * 0.5));
    assert_eq!(ui.window_state(), (false, true), "最小化");
    ui.click(Point::new(w * 1.5, h * 0.5));
    assert_eq!(ui.window_state(), (true, true), "最大化");
    assert!(!ui.exit_requested());
    ui.click(Point::new(w * 2.5, h * 0.5));
    assert!(ui.exit_requested(), "关闭");
}
