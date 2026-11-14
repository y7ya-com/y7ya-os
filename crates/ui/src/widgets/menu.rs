//! A macOS context menu: 8pt radius, hairline border, soft shadow, 24pt rows
//! with an accent highlight, separators, checkmarks, shortcuts, and keyboard
//! navigation (Up/Down, Home/End, Return/Space, Escape, type-to-select).
//!
//! The app owns a `Menu<A>` (A = its action type), renders it last with
//! [`Menu::render`] and forwards keys to [`Menu::key`] while it's open.

use std::rc::Rc;

use crate::cx::Cx;
use crate::element::{Div, Styled, canvas, div, text};
use crate::geom::{Color, Point, pt, rgb, rgba};
use crate::input::KeyPress;
use crate::native;
use crate::text::{TextStyle, Weight};
use winit::keyboard::{Key, NamedKey};

const ROW_H: f32 = 24.0;
const SEPARATOR_H: f32 = 11.0;
const PAD: f32 = 5.0;
const CHECK_W: f32 = 20.0;
const EDGE: f32 = 6.0;

pub struct MenuItem<A> {
    pub label: String,
    pub shortcut: Option<String>,
    pub checked: bool,
    pub enabled: bool,
    /// Red text, for actions like "Delete" (used sparingly, as macOS does).
    pub destructive: bool,
    /// `None` draws a separator.
    pub action: Option<A>,
}

impl<A> MenuItem<A> {
    pub fn new(label: impl Into<String>, action: A) -> Self {
        Self { label: label.into(), shortcut: None, checked: false, enabled: true, destructive: false, action: Some(action) }
    }
    pub fn separator() -> Self {
        Self { label: String::new(), shortcut: None, checked: false, enabled: false, destructive: false, action: None }
    }
    pub fn shortcut(mut self, s: impl Into<String>) -> Self {
        self.shortcut = Some(s.into());
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
    fn selectable(&self) -> bool {
        self.action.is_some() && self.enabled
    }
}

pub struct Menu<A> {
    /// Top-left corner in window coordinates (where the click was).
    pub pos: Point,
    pub items: Vec<MenuItem<A>>,
    /// Row picked with the keyboard, and where the pointer was then (moving
    /// the pointer hands the highlight back to it).
    highlight: Option<(usize, Point)>,
    id: &'static str,
}

/// What a key did to an open menu.
pub enum MenuKey<A> {
    /// Moved the highlight (or ignored the key); the menu stays open.
    Stay,
    /// Return or Space on a row: run this, the menu is closed.
    Pick(A),
    /// Escape: close without doing anything.
    Close,
}

impl<A: Clone + 'static> Menu<A> {
    pub fn new(pos: Point, items: Vec<MenuItem<A>>) -> Self {
        Self { pos, items, highlight: None, id: "menu" }
    }

    /// Gives this menu's rows their own ids (when two menus can be open).
    pub fn with_id(mut self, id: &'static str) -> Self {
        self.id = id;
        self
    }

    /// The row under the pointer, if it's a row that can be picked.
    fn hovered(&self, cx: &Cx) -> Option<usize> {
        (0..self.items.len()).find(|&i| self.items[i].selectable() && cx.hovered((self.id, i)))
    }

    /// The highlighted row: the keyboard's until the pointer moves.
    pub fn highlighted(&self, cx: &Cx) -> Option<usize> {
        match self.highlight {
            Some((i, at)) if at == cx.mouse => Some(i),
            _ => self.hovered(cx).or(self.highlight.map(|(i, _)| i)),
        }
    }

    pub fn key(&mut self, key: &KeyPress, cx: &Cx) -> MenuKey<A> {
        let n = self.items.len();
        let pick = |from: Option<usize>, step: isize| -> Option<usize> {
            let mut i = from.map_or(if step > 0 { -1 } else { n as isize }, |i| i as isize);
            for _ in 0..n {
                i += step;
                if i < 0 || i >= n as isize {
                    return from;
                }
                if self.items[i as usize].selectable() {
                    return Some(i as usize);
                }
            }
            from
        };
        let current = self.highlighted(cx);
        let next = match &key.key {
            Key::Named(NamedKey::Escape) => return MenuKey::Close,
            Key::Named(NamedKey::Enter | NamedKey::Space) => {
                return match current.and_then(|i| self.items[i].action.clone().filter(|_| self.items[i].enabled)) {
                    Some(a) => MenuKey::Pick(a),
                    None => MenuKey::Stay,
                };
            }
            Key::Named(NamedKey::ArrowDown) => pick(current, 1),
            Key::Named(NamedKey::ArrowUp) => pick(current, -1),
            Key::Named(NamedKey::Home | NamedKey::PageUp) => pick(None, 1),
            Key::Named(NamedKey::End | NamedKey::PageDown) => pick(None, -1),
            // Typing a letter jumps to the next row starting with it.
            _ => match key.text.as_deref().and_then(|t| t.chars().next()).filter(|c| c.is_alphanumeric()) {
                Some(c) => {
                    let c = c.to_lowercase().next().unwrap_or(c);
                    let start = current.map_or(0, |i| i + 1);
                    (0..n)
                        .map(|k| (start + k) % n)
                        .find(|&i| self.items[i].selectable() && self.items[i].label.to_lowercase().starts_with(c))
                        .or(current)
                }
                None => current,
            },
        };
        self.highlight = next.map(|i| (i, cx.mouse));
        MenuKey::Stay
    }

    /// The menu panel over a full-window catcher that closes it (via
    /// `on_close`) when clicking elsewhere. `on_pick` runs a row's action.
    pub fn render<S: 'static>(
        &self,
        cx: &mut Cx,
        on_pick: impl Fn(&mut S, A, &mut Cx) + 'static,
        on_close: impl Fn(&mut S, &mut Cx) + 'static,
    ) -> Div<S> {
        let dark = cx.dark;
        let (bg, border, ink, shortcut_ink, disabled, shadow) = if dark {
            (rgb(0x2c2c2e), rgba(0xffffff, 0.16), rgba(0xffffff, 0.88), rgba(0xffffff, 0.45), rgba(0xffffff, 0.28), rgba(0x000000, 0.5))
        } else {
            (rgb(0xf6f6f6), rgba(0x000000, 0.14), rgba(0x000000, 0.85), rgba(0x000000, 0.42), rgba(0x000000, 0.26), rgba(0x000000, 0.2))
        };
        let destructive = if dark { rgb(0xff453a) } else { rgb(0xff3b30) };
        let accent = native::accent(cx);

        // Size the panel to its longest label and shortcut.
        let style = TextStyle { size: 13.0, ..TextStyle::default() };
        let mut label_w: f32 = 0.0;
        let mut short_w: f32 = 0.0;
        for item in self.items.iter().filter(|i| i.action.is_some()) {
            label_w = label_w.max(cx.text.measure(&item.label, &style, None, false).0);
            if let Some(s) = &item.shortcut {
                short_w = short_w.max(cx.text.measure(s, &style, None, false).0);
            }
        }
        let gap = if short_w > 0.0 { 28.0 } else { 0.0 };
        let (win_w, win_h) = cx.window_size;
        let width = (CHECK_W + label_w + gap + short_w + 16.0 + 2.0 * PAD).clamp(160.0, (win_w - 2.0 * EDGE).max(160.0));
        let height: f32 =
            2.0 * PAD + self.items.iter().map(|i| if i.action.is_some() { ROW_H } else { SEPARATOR_H }).sum::<f32>();
        // Open down and to the right of the click; flip where there's no room.
        let x = if self.pos.x + width > win_w - EDGE { (self.pos.x - width).max(EDGE) } else { self.pos.x };
        let y = if self.pos.y + height > win_h - EDGE { (self.pos.y - height).max(EDGE) } else { self.pos.y };

        let highlighted = self.highlighted(cx);
        let on_pick = Rc::new(on_pick);
        let mut panel = div()
            .absolute(x, y)
            .w(width)
            .flex_col()
            .p(PAD)
            .rounded(8.0)
            .bg(bg)
            .border(border, 0.5)
            .shadow(shadow, 18.0, 6.0)
            .occlude();
        for (i, item) in self.items.iter().enumerate() {
            let Some(action) = item.action.clone() else {
                panel = panel.child(div().h(SEPARATOR_H).px(5.0).items_center().child(div().flex_1().h(1.0).bg(border)));
                continue;
            };
            let lit = highlighted == Some(i) && item.enabled;
            let (fg, sub) = match (lit, item.enabled, item.destructive) {
                (true, ..) => (Color::WHITE, Color::WHITE.alpha(0.8)),
                (false, false, _) => (disabled, disabled),
                (false, true, true) => (destructive, shortcut_ink),
                (false, true, false) => (ink, shortcut_ink),
            };
            let check = if item.checked {
                canvas(move |s, b| {
                    let c = b.center();
                    s.segment(pt(c.x - 4.0, c.y + 0.5), pt(c.x - 1.2, c.y + 3.3), 1.6, fg);
                    s.segment(pt(c.x - 1.2, c.y + 3.3), pt(c.x + 4.2, c.y - 3.6), 1.6, fg);
                })
                .w(CHECK_W)
                .h(ROW_H)
                .shrink_0()
            } else {
                canvas(|_, _| {}).w(CHECK_W).h(ROW_H).shrink_0()
            };
            let mut row = div()
                .id((self.id, i))
                .h(ROW_H)
                .pr(10.0)
                .rounded(5.0)
                .flex_row()
                .items_center()
                .bg_opt(lit.then_some(accent))
                .child(check)
                .child(text(item.label.clone()).color(fg).weight(Weight::Regular).truncate().flex_1());
            if let Some(s) = &item.shortcut {
                row = row.child(text(s.clone()).color(sub).shrink_0().ml(gap.min(28.0)));
            }
            if item.enabled {
                let on_pick = on_pick.clone();
                row = row.hover_bg(Color::TRANSPARENT).on_click(move |s: &mut S, cx: &mut Cx| on_pick(s, action.clone(), cx));
            }
            panel = panel.child(row);
        }
        let on_close = Rc::new(on_close);
        let close_right = on_close.clone();
        div()
            .inset_0()
            .deferred()
            .on_mouse_down(move |s: &mut S, cx: &mut Cx| on_close(s, cx))
            .on_right_click(move |s: &mut S, cx: &mut Cx| close_right(s, cx))
            .child(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(k: Key, text: Option<&str>) -> KeyPress {
        KeyPress { key: k, text: text.map(Into::into), ctrl: false, shift: false, alt: false }
    }

    #[test]
    fn keyboard_skips_separators_and_disabled_rows() {
        let cx = Cx::new();
        let mut m = Menu::new(
            Point::default(),
            vec![
                MenuItem::new("Open", 1),
                MenuItem::separator(),
                MenuItem::new("Paste", 2).enabled(false),
                MenuItem::new("Rename", 3),
                MenuItem::new("Delete", 4).destructive(),
            ],
        );
        let down = press(Key::Named(NamedKey::ArrowDown), None);
        assert!(matches!(m.key(&down, &cx), MenuKey::Stay));
        assert_eq!(m.highlighted(&cx), Some(0));
        m.key(&down, &cx);
        assert_eq!(m.highlighted(&cx), Some(3));
        m.key(&down, &cx);
        m.key(&down, &cx);
        assert_eq!(m.highlighted(&cx), Some(4), "stops at the last row");
        m.key(&press(Key::Character("r".into()), Some("r")), &cx);
        assert_eq!(m.highlighted(&cx), Some(3), "type-to-select");
        assert!(matches!(m.key(&press(Key::Named(NamedKey::Enter), None), &cx), MenuKey::Pick(3)));
        assert!(matches!(m.key(&press(Key::Named(NamedKey::Escape), None), &cx), MenuKey::Close));
    }
}
