//! 内置控件。

mod button;
mod checkbox;
mod collapse;
mod common;
mod flex;
mod icon;
mod icon_button;
mod label;
mod list_item;
mod misc;
mod panel;
mod progress;
mod reveal;
mod scroll;
mod slider;
mod tab;
mod text_input;
mod window;

pub use button::{Button, button};
pub use checkbox::{Checkbox, checkbox};
pub use collapse::{Collapse, collapse};
pub use flex::{Flex, column, row};
pub use icon::{Icon, paint_icon};
pub use icon_button::{IconButton, icon_button};
pub use label::{Label, TextRole, label};
pub use list_item::{ListItem, list_item};
pub use misc::{Divider, IconView, Tag, divider, icon, tag};
pub use panel::{Panel, panel};
pub use progress::{ProgressBar, Spinner, progress_bar, spinner};
pub use reveal::{ClickArea, Reveal, click_area, reveal};
pub use scroll::{Scroll, scroll};
pub use slider::{Slider, slider};
pub use tab::{Tab, tab};
pub use text_input::{TextInput, text_input};
pub use window::{DragArea, WindowControls, drag_area, window_controls};
