//! cairn-ui 最小示例：一个标签、一个按钮（点击后标签文字变化）、一个文本输入框。
//!
//! 写法说明：
//! - 应用状态就是普通结构体 `Hello`，没有 `remember` / 信号之类的东西。
//! - `view` 根据当前状态描述界面；回调改完状态后框架会重新调用 `view`，
//!   再和保留着的控件树做比对，只重新布局、重绘真正变化的控件。

use cairn_ui::prelude::*;

#[derive(Default)]
struct Hello {
    clicks: u32,
    name: String,
}

impl Hello {
    fn view(&self) -> impl View<Self> {
        let name = self.name.trim();
        let greeting = match (self.clicks, name.is_empty()) {
            (0, _) => "你好，cairn-ui！点一下按钮试试。".to_string(),
            (n, true) => format!("按钮被点了 {n} 次"),
            (n, false) => format!("{name}，按钮被点了 {n} 次"),
        };

        column()
            .padding(24.0)
            .gap(14.0)
            .child(label(greeting).font_size(18.0))
            .child(
                row()
                    .gap(8.0)
                    .child(button("点我").on_click(|app: &mut Self| app.clicks += 1))
                    .child(button("清空名字").on_click(|app: &mut Self| app.name.clear())),
            )
            .child(
                text_input(&self.name)
                    .placeholder("输入你的名字（支持中文输入法）")
                    .fill_width()
                    .on_change(|app: &mut Self, text| app.name = text),
            )
    }
}

fn main() -> cairn_ui::Result<()> {
    App::new(Hello::view)
        .title("cairn-ui · hello")
        .size(460.0, 220.0)
        .run(Hello::default())
}
