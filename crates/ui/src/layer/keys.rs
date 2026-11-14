//! xkb keysyms to the winit key values the rest of the framework speaks.

use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym, Modifiers};
use winit::keyboard::{Key, NamedKey};

use crate::input::KeyPress;

pub fn key_press(event: &KeyEvent, mods: Modifiers) -> Option<KeyPress> {
    let named = match event.keysym {
        Keysym::Escape => Some(NamedKey::Escape),
        Keysym::Return | Keysym::KP_Enter => Some(NamedKey::Enter),
        Keysym::BackSpace => Some(NamedKey::Backspace),
        Keysym::Delete | Keysym::KP_Delete => Some(NamedKey::Delete),
        Keysym::Tab | Keysym::ISO_Left_Tab => Some(NamedKey::Tab),
        Keysym::space => Some(NamedKey::Space),
        Keysym::Left | Keysym::KP_Left => Some(NamedKey::ArrowLeft),
        Keysym::Right | Keysym::KP_Right => Some(NamedKey::ArrowRight),
        Keysym::Up | Keysym::KP_Up => Some(NamedKey::ArrowUp),
        Keysym::Down | Keysym::KP_Down => Some(NamedKey::ArrowDown),
        Keysym::Home | Keysym::KP_Home => Some(NamedKey::Home),
        Keysym::End | Keysym::KP_End => Some(NamedKey::End),
        Keysym::Page_Up | Keysym::KP_Page_Up => Some(NamedKey::PageUp),
        Keysym::Page_Down | Keysym::KP_Page_Down => Some(NamedKey::PageDown),
        _ => None,
    };
    let text = event.utf8.clone().filter(|t| !t.is_empty());
    let key = match named {
        Some(n) => Key::Named(n),
        None => {
            // With Ctrl held xkb gives control characters; use the plain symbol.
            let ch = event.keysym.key_char().or_else(|| text.as_deref().and_then(|t| t.chars().next()))?;
            if ch.is_control() {
                return None;
            }
            Key::Character(ch.to_string().into())
        }
    };
    Some(KeyPress { key, text, ctrl: mods.ctrl, shift: mods.shift, alt: mods.alt })
}
