//! winit 类型到本 crate 事件类型的转换。winit 本身跨平台，所以这里没有 `cfg`。

use winit::event as we;
use winit::keyboard as wk;

use crate::event::*;

pub(crate) fn pointer_button(b: we::MouseButton) -> PointerButton {
    match b {
        we::MouseButton::Left => PointerButton::Left,
        we::MouseButton::Right => PointerButton::Right,
        we::MouseButton::Middle => PointerButton::Middle,
        we::MouseButton::Back => PointerButton::Back,
        we::MouseButton::Forward => PointerButton::Forward,
        we::MouseButton::Other(n) => PointerButton::Other(n),
    }
}

pub(crate) fn scroll_delta(d: we::MouseScrollDelta) -> ScrollDelta {
    match d {
        we::MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines { x, y },
        we::MouseScrollDelta::PixelDelta(p) => ScrollDelta::Pixels { x: p.x, y: p.y },
    }
}

pub(crate) fn modifiers(m: &we::Modifiers) -> Modifiers {
    let s = m.state();
    Modifiers {
        shift: s.shift_key(),
        ctrl: s.control_key(),
        alt: s.alt_key(),
        logo: s.super_key(),
    }
}

pub(crate) fn key_event(e: &we::KeyEvent) -> KeyEvent {
    KeyEvent {
        key: key(&e.logical_key),
        text: e.text.as_ref().map(|t| t.to_string()),
        pressed: e.state == we::ElementState::Pressed,
        repeat: e.repeat,
    }
}

fn key(k: &wk::Key) -> Key {
    use wk::NamedKey as W;
    match k {
        wk::Key::Character(c) => Key::Character(c.to_string()),
        wk::Key::Named(n) => Key::Named(match n {
            W::Enter => NamedKey::Enter,
            W::Tab => NamedKey::Tab,
            W::Space => NamedKey::Space,
            W::Backspace => NamedKey::Backspace,
            W::Delete => NamedKey::Delete,
            W::Escape => NamedKey::Escape,
            W::ArrowLeft => NamedKey::ArrowLeft,
            W::ArrowRight => NamedKey::ArrowRight,
            W::ArrowUp => NamedKey::ArrowUp,
            W::ArrowDown => NamedKey::ArrowDown,
            W::Home => NamedKey::Home,
            W::End => NamedKey::End,
            W::PageUp => NamedKey::PageUp,
            W::PageDown => NamedKey::PageDown,
            W::Insert => NamedKey::Insert,
            W::Shift => NamedKey::Shift,
            W::Control => NamedKey::Control,
            W::Alt => NamedKey::Alt,
            W::Super => NamedKey::Super,
            W::Process => NamedKey::Process,
            W::F1 => NamedKey::F(1),
            W::F2 => NamedKey::F(2),
            W::F3 => NamedKey::F(3),
            W::F4 => NamedKey::F(4),
            W::F5 => NamedKey::F(5),
            W::F6 => NamedKey::F(6),
            W::F7 => NamedKey::F(7),
            W::F8 => NamedKey::F(8),
            W::F9 => NamedKey::F(9),
            W::F10 => NamedKey::F(10),
            W::F11 => NamedKey::F(11),
            W::F12 => NamedKey::F(12),
            _ => NamedKey::Other,
        }),
        wk::Key::Unidentified(_) | wk::Key::Dead(_) => Key::Unidentified,
    }
}

pub(crate) fn ime(e: we::Ime) -> ImeEvent {
    match e {
        we::Ime::Enabled => ImeEvent::Enabled,
        we::Ime::Preedit(text, cursor) => ImeEvent::Preedit { text, cursor },
        we::Ime::Commit(text) => ImeEvent::Commit(text),
        we::Ime::Disabled => ImeEvent::Disabled,
    }
}

pub(crate) fn cursor_icon(c: CursorIcon) -> winit::window::CursorIcon {
    use winit::window::CursorIcon as W;
    match c {
        CursorIcon::Default => W::Default,
        CursorIcon::Pointer => W::Pointer,
        CursorIcon::Text => W::Text,
        CursorIcon::NotAllowed => W::NotAllowed,
        CursorIcon::ResizeHorizontal => W::EwResize,
        CursorIcon::ResizeVertical => W::NsResize,
        CursorIcon::ResizeNwSe => W::NwseResize,
        CursorIcon::ResizeNeSw => W::NeswResize,
    }
}

pub(crate) fn resize_direction(e: crate::ResizeEdge) -> winit::window::ResizeDirection {
    use crate::ResizeEdge as E;
    use winit::window::ResizeDirection as D;
    match e {
        E::North => D::North,
        E::South => D::South,
        E::East => D::East,
        E::West => D::West,
        E::NorthEast => D::NorthEast,
        E::NorthWest => D::NorthWest,
        E::SouthEast => D::SouthEast,
        E::SouthWest => D::SouthWest,
    }
}
