//! cairn-ui 的 Nocturne Glass 示例：夜景背景上一块无色玻璃面板，里面是标题、副标题、
//! 一个香槟金强调按钮和一个圆形图标按钮。
//!
//! 夜景背景、背景模糊、阴影、玻璃分层、悬停上浮等动效都由库提供（`Theme::nocturne()` 和 `panel()`），
//! 这里只描述界面，没有任何自己的绘制代码。

use cairn_ui::prelude::*;

struct GlassCard;

impl GlassCard {
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

fn main() -> cairn_ui::Result<()> {
    App::new(GlassCard::view)
        .title("cairn-ui · glass card")
        .size(640.0, 420.0)
        .theme(Theme::nocturne())
        .run(GlassCard)
}
