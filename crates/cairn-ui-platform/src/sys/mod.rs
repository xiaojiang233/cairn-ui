//! 操作系统相关的实现。上层 crate 不直接调用这里，只通过 crate 根部的函数访问。
//!
//! 新增平台时：仿照 `windows.rs` 实现同名函数，并在下面的 `cfg` 里接上。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

#[cfg(not(windows))]
mod fallback;
#[cfg(not(windows))]
pub(crate) use fallback::*;
