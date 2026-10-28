//! 主题：颜色、字号、圆角、间距、阴影、动效时长与曲线等全部外观参数（design tokens）。
//!
//! 控件代码里不写外观字面量，一律从这里取。内置两套：
//! - [`Theme::light`]：浅色、实色控件（默认）。
//! - [`Theme::nocturne`]：夜色玻璃。深墨蓝夜景背景，无色玻璃面板，唯一强调色是香槟金。
//!
//! 可以在内置主题上改字段得到自己的主题：
//!
//! ```
//! use cairn_ui::{Color, Theme};
//! let mut theme = Theme::nocturne();
//! theme.radius.panel = 12.0;
//! theme.panel.fill = Color::rgbaf(255, 255, 255, 0.06);
//! ```

use std::rc::Rc;
use std::time::Duration;

use crate::anim::Easing;
use crate::{BackdropStyle, Color, Insets, NightScene, Scene, Shadow};

/// 一种文字角色的样式。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeStyle {
    /// 字号（逻辑像素）。
    pub size: f32,
    /// 字重，400 常规、600 半粗、700 粗体。
    pub weight: u16,
    pub color: Color,
}

/// 文字层级。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typography {
    pub title: TypeStyle,
    /// 正文，也是标签、按钮、输入框的默认字号。
    pub body: TypeStyle,
    /// 次要说明，例如副标题。
    pub secondary: TypeStyle,
    /// 辅助小字。
    pub caption: TypeStyle,
    /// 行高相对字号的倍数。
    pub line_height: f32,
}

/// 圆角半径。图标按钮、滑块是全圆角，不在这里。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    pub panel: f32,
    /// 按钮、输入框。
    pub control: f32,
    /// 标签、徽章。
    pub tag: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    /// 面板内边距。
    pub panel_padding: f32,
    /// 面板里子元素的默认间距。
    pub gap: f32,
}

/// 面板。`nocturne` 下是玻璃（背景模糊 + 半透明白 + 内发光 + 细边 + 多层阴影），`light` 下是白色卡片。
///
/// 所有“边”都是 1 个物理像素：顶部内高光、底部内暗边、边框。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelStyle {
    /// 先铺一层模糊后的背景（[`crate::Brush::Backdrop`]）。
    pub backdrop: bool,
    pub fill: Color,
    pub fill_hover: Color,
    /// 内发光：从顶部的这个颜色渐变到底部透明。透明表示没有。
    pub glow: Color,
    pub top_highlight: Color,
    pub bottom_edge: Color,
    pub border: Color,
    pub border_hover: Color,
    pub shadow: Shadow,
    pub shadow_hover: Shadow,
    /// 悬停时上浮的距离（逻辑像素）。
    pub lift: f32,
}

/// 按钮的一种外观。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonStyle {
    pub fill: Color,
    pub fill_hover: Color,
    pub fill_pressed: Color,
    /// 1 物理像素边框，透明表示没有。
    pub border: Color,
    pub border_hover: Color,
    pub text: Color,
    pub weight: u16,
    pub top_highlight: Color,
    /// 外发光（静止 / 悬停）。
    pub glow: Shadow,
    pub glow_hover: Shadow,
    /// 悬停时上浮的距离。
    pub lift: f32,
    /// 按下时缩放到的比例（1 表示不缩放）。
    pub press_scale: f32,
    pub padding: Insets,
    /// 文字前面的图标（[`crate::Button::icon`]）的边长，以及它和文字的间距。
    pub icon_size: f32,
    pub icon_gap: f32,
    pub disabled_fill: Color,
    pub disabled_text: Color,
    pub disabled_border: Color,
}

/// 圆形图标按钮。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconButtonStyle {
    /// 直径。
    pub size: f32,
    pub fill: Color,
    pub fill_hover: Color,
    pub border: Color,
    pub border_hover: Color,
    pub top_highlight: Color,
    pub icon: Color,
    /// 图标边长。
    pub icon_size: f32,
    pub press_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputStyle {
    pub fill: Color,
    pub fill_hover: Color,
    pub fill_focus: Color,
    pub border: Color,
    pub border_hover: Color,
    pub border_focus: Color,
    /// 边框宽度（逻辑像素）；`<= 0` 表示 1 个物理像素。
    pub border_width: f32,
    pub border_focus_width: f32,
    /// 获得焦点时的外光晕，透明表示没有。
    pub focus_glow: Shadow,
    pub text: Color,
    pub placeholder: Color,
    pub caret: Color,
    pub selection: Color,
    /// 输入法预编辑文本的下划线。
    pub preedit: Color,
    pub padding: Insets,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderStyle {
    pub track_height: f32,
    pub track: Color,
    pub track_border: Color,
    /// 已填充部分。
    pub fill: Color,
    /// 滑块直径。
    pub knob_size: f32,
    pub knob: Color,
    pub knob_border: Color,
    pub knob_shadow: Shadow,
    /// 拖动时滑块外圈的细环：颜色、线宽，以及它和滑块之间的空隙。
    pub knob_ring: Color,
    pub knob_ring_width: f32,
    pub knob_ring_gap: f32,
}

/// 键盘焦点框：在控件外 `offset` 处画 `width` 宽的圆角框。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusRing {
    pub color: Color,
    pub width: f32,
    pub offset: f32,
}

/// 动效时长与曲线。系统关闭了动画时（[`crate::App`] 启动时读取）所有时长都变成 0。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// 悬停：上浮、阴影、边框变亮。
    pub hover: Duration,
    /// 按下与回弹。
    pub press: Duration,
    /// 焦点光晕。
    pub focus: Duration,
    pub easing: Easing,
    pub press_easing: Easing,
}

impl Motion {
    /// 所有时长为 0。
    pub fn reduced(self) -> Motion {
        Motion {
            hover: Duration::ZERO,
            press: Duration::ZERO,
            focus: Duration::ZERO,
            ..self
        }
    }
}

/// 全局外观。通过 [`crate::App::theme`] 设置。
#[derive(Clone, Debug)]
pub struct Theme {
    /// 窗口底色。设置了 [`Theme::scene`] 时，它也是“背景模糊”在场景生成前的退化颜色。
    pub background: Color,
    /// 窗口背景场景。`None` 时窗口用纯色 `background`。
    pub scene: Option<Rc<dyn Scene>>,
    /// 玻璃面板背后的模糊参数。
    pub backdrop: BackdropStyle,
    /// 全局噪点的不透明度（0 关闭），整帧叠加，顺带消除深色渐变的色带。
    pub noise: f32,
    pub typography: Typography,
    /// 唯一的强调色：焦点、选中。
    pub accent: Color,
    /// 点缀色：滑块填充、图标、图表线。
    pub highlight: Color,
    pub radius: Radii,
    pub spacing: Spacing,
    pub focus_ring: FocusRing,
    pub panel: PanelStyle,
    /// 普通按钮。
    pub button: ButtonStyle,
    /// 强调按钮（[`crate::Button::accent`]）。一页最多一两个。
    pub accent_button: ButtonStyle,
    pub icon_button: IconButtonStyle,
    pub input: InputStyle,
    pub slider: SliderStyle,
    /// 没有专门“禁用”配色的控件（图标按钮、滑块）禁用时的整体不透明度。
    pub disabled_opacity: f32,
    pub motion: Motion,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::light()
    }
}

impl Theme {
    /// 浅色主题。
    pub fn light() -> Self {
        let text = Color::hex(0x1E2024);
        let muted = Color::hex(0x8A9099);
        let accent = Color::hex(0x2B66D9);
        let border = Color::hex(0xC5CAD2);
        let border_hover = Color::hex(0x9EA5B0);
        let button = ButtonStyle {
            fill: accent,
            fill_hover: Color::hex(0x3B76E6),
            fill_pressed: Color::hex(0x2152B5),
            border: Color::TRANSPARENT,
            border_hover: Color::TRANSPARENT,
            text: Color::WHITE,
            weight: 400,
            top_highlight: Color::TRANSPARENT,
            glow: Shadow::NONE,
            glow_hover: Shadow::NONE,
            lift: 0.0,
            press_scale: 1.0,
            padding: Insets::symmetric(16.0, 7.0),
            icon_size: 16.0,
            icon_gap: 7.0,
            disabled_fill: Color::hex(0xDADDE2),
            disabled_text: Color::hex(0x9AA0A8),
            disabled_border: Color::TRANSPARENT,
        };
        Theme {
            background: Color::hex(0xF4F5F7),
            scene: None,
            backdrop: BackdropStyle::default(),
            noise: 0.0,
            typography: Typography {
                title: TypeStyle {
                    size: 20.0,
                    weight: 600,
                    color: text,
                },
                body: TypeStyle {
                    size: 14.0,
                    weight: 400,
                    color: text,
                },
                secondary: TypeStyle {
                    size: 14.0,
                    weight: 400,
                    color: Color::hex(0x5D636B),
                },
                caption: TypeStyle {
                    size: 12.0,
                    weight: 400,
                    color: muted,
                },
                line_height: 1.45,
            },
            accent,
            highlight: accent,
            radius: Radii {
                panel: 10.0,
                control: 6.0,
                tag: 4.0,
            },
            spacing: Spacing {
                panel_padding: 16.0,
                gap: 10.0,
            },
            focus_ring: FocusRing {
                color: accent,
                width: 2.0,
                offset: 3.0,
            },
            panel: PanelStyle {
                backdrop: false,
                fill: Color::WHITE,
                fill_hover: Color::WHITE,
                glow: Color::TRANSPARENT,
                top_highlight: Color::TRANSPARENT,
                bottom_edge: Color::TRANSPARENT,
                border: Color::hex(0xE1E4E8),
                border_hover: Color::hex(0xD0D5DB),
                shadow: Shadow::new(0.0, 1.0, 3.0, Color::rgbaf(16, 24, 40, 0.08)),
                shadow_hover: Shadow::new(0.0, 4.0, 12.0, Color::rgbaf(16, 24, 40, 0.10)),
                lift: 0.0,
            },
            button,
            accent_button: button,
            icon_button: IconButtonStyle {
                size: 32.0,
                fill: Color::rgbaf(30, 32, 36, 0.05),
                fill_hover: Color::rgbaf(30, 32, 36, 0.09),
                border: Color::hex(0xD0D5DB),
                border_hover,
                top_highlight: Color::TRANSPARENT,
                icon: accent,
                icon_size: 16.0,
                press_scale: 1.0,
            },
            input: InputStyle {
                fill: Color::WHITE,
                fill_hover: Color::WHITE,
                fill_focus: Color::WHITE,
                border,
                border_hover,
                border_focus: accent,
                border_width: 1.0,
                border_focus_width: 2.0,
                focus_glow: Shadow::NONE,
                text,
                placeholder: muted,
                caret: text,
                selection: accent.with_alpha(72),
                preedit: text,
                padding: Insets::symmetric(10.0, 7.0),
            },
            slider: SliderStyle {
                track_height: 4.0,
                track: Color::hex(0xDADDE2),
                track_border: Color::TRANSPARENT,
                fill: accent,
                knob_size: 16.0,
                knob: Color::WHITE,
                knob_border: border,
                knob_shadow: Shadow::new(0.0, 1.0, 3.0, Color::rgbaf(0, 0, 0, 0.18)),
                knob_ring: accent,
                knob_ring_width: 2.0,
                knob_ring_gap: 2.0,
            },
            disabled_opacity: 0.5,
            // 状态切换是瞬时的，与 0.1.0 一致，也不会有动画帧。
            motion: Motion {
                hover: Duration::ZERO,
                press: Duration::ZERO,
                focus: Duration::ZERO,
                easing: Easing::EaseOut,
                press_easing: Easing::EaseOut,
            },
        }
    }

    /// 夜色玻璃（Nocturne Glass）。
    ///
    /// 深墨蓝夜景（[`NightScene`]）上放无色玻璃：玻璃本身不着色，白色填充不超过 15%，
    /// 背后的场景模糊 40 逻辑像素、饱和度 ×1.8；面板有外部深度阴影、顶部高光、底部暗边三层。
    /// 唯一的强调色是香槟金 `#E4B863`，点缀色是月光蓝 `#7C9CC4`。过渡 500 ms，按压 150 ms。
    pub fn nocturne() -> Self {
        let white = |a: f32| Color::WHITE.with_opacity(a);
        let gold = Color::hex(0xE4B863);
        let gold_light = Color::hex(0xF3DCA8);
        let moon = Color::hex(0x7C9CC4);
        let ink = |a: f32| Color::rgbaf(3, 7, 18, a);
        let glass_button = ButtonStyle {
            fill: white(0.12),
            fill_hover: white(0.15),
            fill_pressed: white(0.15),
            border: white(0.40),
            border_hover: white(0.60),
            text: Color::WHITE,
            weight: 500,
            top_highlight: white(0.20),
            glow: Shadow::NONE,
            glow_hover: Shadow::NONE,
            lift: 2.0,
            press_scale: 0.97,
            padding: Insets::symmetric(18.0, 8.0),
            icon_size: 16.0,
            icon_gap: 8.0,
            disabled_fill: white(0.06),
            disabled_text: white(0.35),
            disabled_border: white(0.15),
        };
        Theme {
            background: Color::hex(0x0B1322),
            scene: Some(Rc::new(NightScene::default())),
            backdrop: BackdropStyle {
                blur: 40.0,
                saturation: 1.8,
                downsample: 4,
            },
            noise: 0.025,
            typography: Typography {
                title: TypeStyle {
                    size: 22.0,
                    weight: 600,
                    color: Color::WHITE,
                },
                body: TypeStyle {
                    size: 14.0,
                    weight: 400,
                    color: white(0.80),
                },
                secondary: TypeStyle {
                    size: 14.0,
                    weight: 400,
                    color: white(0.60),
                },
                caption: TypeStyle {
                    size: 12.0,
                    weight: 400,
                    color: white(0.45),
                },
                line_height: 1.45,
            },
            accent: gold,
            highlight: moon,
            radius: Radii {
                panel: 14.0,
                control: 10.0,
                tag: 8.0,
            },
            spacing: Spacing {
                panel_padding: 16.0,
                gap: 10.0,
            },
            focus_ring: FocusRing {
                color: gold.with_opacity(0.55),
                width: 1.5,
                offset: 3.0,
            },
            panel: PanelStyle {
                backdrop: true,
                fill: white(0.08),
                fill_hover: white(0.11),
                glow: white(0.12),
                top_highlight: white(0.22),
                bottom_edge: Color::rgbaf(2, 6, 16, 0.35),
                border: white(0.15),
                border_hover: white(0.30),
                shadow: Shadow::new(0.0, 16.0, 40.0, ink(0.5)),
                shadow_hover: Shadow::new(0.0, 22.0, 40.0, ink(0.6)),
                lift: 2.0,
            },
            button: glass_button,
            accent_button: ButtonStyle {
                fill: gold.with_opacity(0.15),
                fill_hover: gold.with_opacity(0.22),
                fill_pressed: gold.with_opacity(0.22),
                border: gold.with_opacity(0.40),
                border_hover: gold.with_opacity(0.55),
                text: gold_light,
                top_highlight: gold_light.with_opacity(0.35),
                glow: Shadow::new(0.0, 0.0, 18.0, gold.with_opacity(0.15)),
                glow_hover: Shadow::new(0.0, 0.0, 24.0, gold.with_opacity(0.22)),
                ..glass_button
            },
            icon_button: IconButtonStyle {
                size: 32.0,
                fill: white(0.10),
                fill_hover: white(0.14),
                border: white(0.15),
                border_hover: white(0.30),
                top_highlight: white(0.20),
                icon: moon,
                icon_size: 16.0,
                press_scale: 0.97,
            },
            input: InputStyle {
                fill: white(0.08),
                fill_hover: white(0.10),
                fill_focus: white(0.12),
                border: white(0.15),
                border_hover: white(0.30),
                border_focus: white(0.50),
                border_width: 0.0,
                border_focus_width: 0.0,
                focus_glow: Shadow::new(0.0, 0.0, 0.0, gold.with_opacity(0.18)).spread(3.0),
                text: Color::WHITE,
                placeholder: white(0.40),
                caret: Color::WHITE,
                selection: moon.with_opacity(0.35),
                preedit: moon,
                padding: Insets::symmetric(12.0, 8.0),
            },
            slider: SliderStyle {
                track_height: 6.0,
                track: white(0.10),
                track_border: white(0.15),
                fill: moon,
                knob_size: 16.0,
                knob: Color::rgb(236, 240, 246),
                knob_border: white(0.60),
                knob_shadow: Shadow::new(0.0, 2.0, 6.0, ink(0.45)),
                knob_ring: gold.with_opacity(0.85),
                knob_ring_width: 1.5,
                knob_ring_gap: 2.0,
            },
            disabled_opacity: 0.4,
            motion: Motion {
                hover: Duration::from_millis(500),
                press: Duration::from_millis(150),
                focus: Duration::from_millis(500),
                easing: Easing::SMOOTH,
                press_easing: Easing::EaseOut,
            },
        }
    }

    /// 换背景场景（例如 [`crate::ImageScene`] 图片背景）。
    pub fn with_scene(mut self, scene: impl Scene) -> Self {
        self.scene = Some(Rc::new(scene));
        self
    }
}
