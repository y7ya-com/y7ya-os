//! Spotlight: a capsule search field with grouped results below, centered a
//! little above the middle of the screen like macOS 26.

use ui::*;

use super::theme::Theme;
use super::{Shell, icons, launch, shell_quote};
use crate::calc::Answer;
use crate::search::{self, Hit};

const WIDTH: f32 = 680.0;
const FIELD_H: f32 = 54.0;
const ROW_H: f32 = 36.0;
const HEADER_H: f32 = 26.0;

/// A colored rounded tile with the app's initial, standing in for its icon.
pub fn letter_tile<S: 'static>(name: &str, size: f32) -> Div<S> {
    // Not the accent blue, so tiles stay visible on a selected row.
    const COLORS: [u32; 8] = [0x5e5ce6, 0x30b158, 0xff9500, 0xaf52de, 0xff2d55, 0x30b0c7, 0x8e8e93, 0xff3b30];
    let hash = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
    div()
        .size(size)
        .shrink_0()
        .rounded(size * 0.26)
        .bg(rgb(COLORS[hash as usize % COLORS.len()]))
        .items_center()
        .justify_center()
        .child(text(letter).size(size * 0.5).weight(Weight::Bold).color(Color::WHITE))
}

fn tilde(path: &std::path::Path) -> String {
    let s = path.to_string_lossy().into_owned();
    match dirs::home_dir().map(|h| h.to_string_lossy().into_owned()) {
        Some(home) if s.starts_with(&home) => format!("~{}", &s[home.len()..]),
        _ => s,
    }
}

impl Shell {
    fn hits(&self) -> Vec<&Hit> {
        self.results.iter().flat_map(|s| s.hits.iter()).collect()
    }

    pub(super) fn update_results(&mut self) {
        let q = self.query.borrow().text();
        let files = self.files.lock().unwrap();
        let results = search::search(&q, &self.apps, files.as_ref());
        drop(files);
        if results == self.results {
            return;
        }
        // New typing starts at the top; a refresh (the file index caught up)
        // keeps the selected result if it's still there.
        let selected = self.hits().get(self.selected).map(|h| (*h).clone());
        self.results = results;
        self.selected = match selected {
            Some(hit) if q == self.last_query => self.hits().iter().position(|h| **h == hit).unwrap_or(0),
            _ => 0,
        };
        self.last_query = q;
    }

    /// Types `q` into Spotlight (for snapshots).
    pub fn set_query(&mut self, q: &str, cx: &mut Cx) {
        let mut input = self.query.borrow_mut();
        input.set_text(&mut cx.text.fonts, q);
        input.move_to_end();
        drop(input);
        self.update_results();
    }

    pub(super) fn spotlight_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        let n = self.hits().len();
        if key.is_named(NamedKey::ArrowDown) && n > 0 {
            self.selected = (self.selected + 1).min(n - 1);
            true
        } else if key.is_named(NamedKey::ArrowUp) {
            self.selected = self.selected.saturating_sub(1);
            true
        } else if key.is_named(NamedKey::Enter) {
            self.activate_hit(self.selected, cx);
            true
        } else {
            false
        }
    }

    fn activate_hit(&mut self, i: usize, cx: &mut Cx) {
        let Some(hit) = self.hits().get(i).map(|h| (*h).clone()) else { return };
        match hit {
            Hit::App(app) => launch(&app.command()),
            Hit::File { path, .. } => launch(&format!("xdg-open {}", shell_quote(&path.to_string_lossy()))),
            Hit::Answer(a) => {
                // Like Spotlight's ⌘C on a result: the value goes to the clipboard.
                let value = a.value().replace(',', "");
                let value = value.split(' ').next().unwrap_or("").to_string();
                if let Some(cb) = cx.clipboard.as_mut() {
                    let _ = cb.set_text(value);
                }
            }
        }
        self.close(cx);
    }

    pub(super) fn spotlight(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let (w, h) = cx.window_size;
        let x = ((w - WIDTH) / 2.0).max(8.0);
        let y = (h * 0.19).max(24.0);
        {
            let mut q = self.query.borrow_mut();
            q.set_styles(TextStyle { size: 21.0, line_height: 1.25, weight: Weight::Regular, color: t.label }, None);
            q.placeholder_color = t.tertiary;
            q.caret_color = t.accent;
            q.selection_color = t.accent.alpha(0.3);
        }
        let secondary = t.secondary;
        let field = div()
            .w(WIDTH.min(w - 16.0))
            .h(FIELD_H)
            .rounded(FIELD_H / 2.0)
            .bg(t.card)
            .border(t.border, 0.5)
            .shadow(t.shadow, 34.0, 12.0)
            .flex_row()
            .items_center()
            .px(20.0)
            .gap(12.0)
            .child(canvas(move |s, b| icons::magnifier(s, b, secondary, 21.0)).size(24.0).shrink_0())
            .child(input("spotlight-input", &self.query).flex_1());

        let mut column = div().absolute(x, y).flex_col().gap(10.0).occlude().child(field);
        if !self.results.is_empty() {
            column = column.child(self.results_card(h - y - FIELD_H - 40.0, t));
        }
        column
    }

    fn results_card(&self, max_h: f32, t: &Theme) -> Div<Self> {
        let mut card = div()
            .w(WIDTH)
            .flex_col()
            .p(8.0)
            .rounded(22.0)
            .bg(t.card)
            .border(t.border, 0.5)
            .shadow(t.shadow, 34.0, 12.0);
        let mut used = 16.0;
        let mut index = 0;
        'sections: for section in &self.results {
            if used + HEADER_H + ROW_H > max_h {
                break;
            }
            used += HEADER_H;
            card = card.child(
                text(section.title).size(11.0).weight(Weight::Semibold).color(t.secondary).pl(12.0).pt(7.0).pb(5.0),
            );
            for hit in &section.hits {
                if used + ROW_H > max_h {
                    break 'sections;
                }
                used += ROW_H;
                card = card.child(self.result_row(hit, index, t));
                index += 1;
            }
        }
        card
    }

    fn result_row(&self, hit: &Hit, i: usize, t: &Theme) -> Div<Self> {
        let selected = i == self.selected;
        let (fg, dim) = if selected { (Color::WHITE, Color::WHITE.alpha(0.75)) } else { (t.label, t.secondary) };
        let (icon, label, detail): (Div<Self>, String, String) = match hit {
            Hit::App(app) => (letter_tile(&app.name, 24.0), app.name.clone(), "Application".into()),
            Hit::File { path, is_dir } => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let parent = path.parent().map(tilde).unwrap_or_default();
                let (ink, paper) = (if t.dark { rgb(0x8e8e93) } else { rgb(0xa0a0a6) }, if t.dark { rgb(0x3a3a3c) } else { Color::WHITE });
                let folder_blue = rgb(0x5aa9f7);
                let icon = if *is_dir {
                    canvas(move |s, b| icons::folder(s, b, folder_blue))
                } else {
                    canvas(move |s, b| icons::document(s, b, ink, paper))
                };
                (div().size(24.0).shrink_0().child(icon.size(24.0)), name, parent)
            }
            Hit::Answer(a) => {
                let (glyph, detail): (fn(&mut Scene, Bounds, Color), String) = match a {
                    Answer::Math { expr, .. } => (icons::equals, expr.clone()),
                    Answer::Conversion { from, .. } => (icons::arrows, from.clone()),
                };
                let tile = div()
                    .size(24.0)
                    .shrink_0()
                    .rounded(6.0)
                    .bg(rgb(0xff9f0a))
                    .items_center()
                    .justify_center()
                    .child(canvas(move |s, b| glyph(s, b, Color::WHITE)).size(18.0));
                (tile, a.value().to_string(), detail)
            }
        };
        let big = matches!(hit, Hit::Answer(_));
        div()
            .id(("spotlight-row", i))
            .h(ROW_H)
            .px(10.0)
            .rounded(10.0)
            .flex_row()
            .items_center()
            .gap(10.0)
            .bg_opt(selected.then_some(t.accent))
            .hover_bg(if selected { t.accent } else { t.hover })
            .on_click(move |s: &mut Self, cx: &mut Cx| s.activate_hit(i, cx))
            .child(icon)
            .child(
                text(label)
                    .size(if big { 15.0 } else { 13.5 })
                    .weight(if big { Weight::Semibold } else { Weight::Regular })
                    .color(fg)
                    .truncate(),
            )
            .child(div().flex_1())
            .child(text(detail).size(11.5).color(dim).truncate().max_w(300.0))
    }
}
