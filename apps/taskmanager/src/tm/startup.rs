//! The Startup apps page: XDG autostart entries with a switch each.

use ui::widgets::table::{Cell, Column, Row, Table};
use ui::*;

use super::theme::Theme;
use super::{Menu, MenuAction, MenuItem, TaskManager, icons, step};
use crate::sys::autostart::StartupEntry;

impl TaskManager {
    fn startup_rows(&self) -> Vec<&StartupEntry> {
        let q = self.query();
        self.startup
            .iter()
            .filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q) || e.exec.to_lowercase().contains(&q))
            .collect()
    }

    pub(super) fn set_startup(&mut self, file: &str, enabled: bool) {
        let Some(e) = self.startup.iter().find(|e| e.file == file).cloned() else { return };
        if let Err(err) = self.autostart.set_enabled(&e, enabled) {
            self.show_error("Couldn't change the startup app", err.to_string());
        }
        self.startup = self.autostart.load();
    }

    pub(super) fn startup_page(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let rows = self.startup_rows();
        let columns = [
            Column::new("Name"),
            Column::new("Command"),
            Column::new("Source").width(90.0),
            Column::new("Status").width(130.0),
        ];
        let style = self.table_style(t, cx);
        let table = Table::new(&self.startup_table, &columns, rows.len(), style).render(cx, |i| {
            let e = rows[i];
            let selected = self.startup_sel.as_deref() == Some(e.file.as_str());
            let secondary = if selected { style.selection_text.alpha(0.75) } else { t.text_secondary };
            let file = e.file.clone();
            let enabled = e.enabled;
            let name = div()
                .flex_row()
                .items_center()
                .gap(8.0)
                .min_w_0()
                .child(Self::app_glyph(&e.name, 18.0))
                .child(text(e.name.clone()).truncate());
            let status = div()
                .flex_row()
                .items_center()
                .gap(8.0)
                .child(Self::switch(("startup-switch", e.file.clone()), enabled, selected && style.selection_text == Color::WHITE, t, {
                    let file = file.clone();
                    move |s: &mut Self, _| {
                        s.startup_sel = Some(file.clone());
                        s.set_startup(&file, !enabled);
                    }
                }))
                .child(text(if enabled { "Enabled" } else { "Disabled" }).color(secondary));
            let source = match (e.user, e.system) {
                (true, true) => "User override",
                (true, false) => "User",
                _ => "System",
            };
            let (f1, f2) = (file.clone(), file.clone());
            Row::new(("startup", e.file.clone()), vec![
                Cell::new(name),
                Cell::new(text(e.exec.clone()).color(secondary).truncate()),
                Cell::new(text(source).color(secondary).truncate()),
                Cell::new(status),
            ])
            .selected(selected)
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                s.menu = None;
                s.startup_sel = Some(f1.clone());
                cx.focus("table");
            })
            .on_right_click(move |s: &mut Self, cx: &mut Cx| {
                s.startup_sel = Some(f2.clone());
                let label = if enabled { "Disable" } else { "Enable" };
                s.menu = Some(Menu {
                    pos: cx.mouse,
                    items: vec![MenuItem { label: label.into(), action: Some(MenuAction::SetStartup(f2.clone(), !enabled)), enabled: true }],
                });
            })
        });
        let body = div().id("table").focusable().flex_1().flex_col().min_h(0.0).child(table);
        if self.startup.is_empty() {
            let c = t.text_tertiary;
            return body.child(
                div()
                    .inset_0()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(8.0)
                    .child(canvas(move |s, b| icons::startup(s, b, c)).size(36.0))
                    .child(text("No startup apps").size(14.0).color(c)),
            );
        }
        body
    }

    pub(super) fn startup_key(&mut self, key: &KeyPress) -> bool {
        let files: Vec<Option<String>> = self.startup_rows().iter().map(|e| Some(e.file.clone())).collect();
        let d = if key.is_named(NamedKey::ArrowDown) {
            1
        } else if key.is_named(NamedKey::ArrowUp) {
            -1
        } else if key.is_named(NamedKey::Space) || key.is_named(NamedKey::Enter) {
            if let Some(f) = self.startup_sel.clone() {
                let on = self.startup.iter().find(|e| e.file == f).is_some_and(|e| e.enabled);
                self.set_startup(&f, !on);
            }
            return true;
        } else {
            return false;
        };
        if let Some((i, f)) = step(&files, self.startup_sel.as_ref(), d) {
            self.startup_sel = Some(f);
            self.startup_table.reveal(i);
        }
        true
    }
}
