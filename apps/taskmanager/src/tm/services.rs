//! The Services page: systemd user and system services. User services can be
//! started, stopped and restarted; system ones are read-only here.

use ui::widgets::table::{Cell, Column, Row, Table};
use ui::*;

use super::theme::Theme;
use super::{Menu, MenuAction, MenuItem, TaskManager, step};
use crate::sys::services::{self as svc, Scope, Service};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SCol {
    Name,
    Description,
    Status,
    Kind,
}

const COLS: [SCol; 4] = [SCol::Name, SCol::Description, SCol::Status, SCol::Kind];

impl TaskManager {
    fn service_rows(&self) -> Vec<&Service> {
        let q = self.query();
        let mut v: Vec<&Service> = self
            .services
            .iter()
            .filter(|s| self.service_scope.is_none_or(|sc| s.scope == sc))
            .filter(|s| q.is_empty() || s.unit.to_lowercase().contains(&q) || s.description.to_lowercase().contains(&q))
            .collect();
        let (col, desc) = self.service_sort;
        v.sort_by(|a, b| {
            let o = match col {
                SCol::Name => a.name().to_lowercase().cmp(&b.name().to_lowercase()),
                SCol::Description => a.description.to_lowercase().cmp(&b.description.to_lowercase()),
                SCol::Status => b.running().cmp(&a.running()).then(a.status().cmp(&b.status())),
                SCol::Kind => (a.scope == Scope::System).cmp(&(b.scope == Scope::System)),
            };
            let o = if desc { o.reverse() } else { o };
            o.then_with(|| a.unit.cmp(&b.unit))
        });
        v
    }

    pub(super) fn selected_service(&self) -> Option<&Service> {
        let unit = self.service_sel.as_ref()?;
        self.services.iter().find(|s| &s.unit == unit)
    }

    pub(super) fn control_service(&mut self, unit: &str, action: svc::Action, cx: &mut Cx) {
        if let Err(e) = svc::control(unit, action) {
            self.show_error("Couldn't change the service", e);
        }
        self.refresh_services(cx);
    }

    pub(super) fn services_page(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let rows = self.service_rows();
        let columns =
            [Column::new("Name"), Column::new("Description"), Column::new("Status").width(96.0), Column::new("Type").width(80.0)];
        let style = self.table_style(t, cx);
        let sort = COLS.iter().position(|c| *c == self.service_sort.0).map(|c| (c, self.service_sort.1));
        let table = Table::new(&self.service_table, &columns, rows.len(), style)
            .sort(sort)
            .on_sort(|s: &mut Self, c, _| {
                let col = COLS[c];
                s.service_sort = if s.service_sort.0 == col { (col, !s.service_sort.1) } else { (col, false) };
            })
            .render(cx, |i| {
                let svc = rows[i];
                let unit = svc.unit.clone();
                let selected = self.service_sel.as_deref() == Some(unit.as_str());
                let ink = if selected { style.selection_text } else { t.text };
                let secondary = if selected { ink.alpha(0.75) } else { t.text_secondary };
                let dot = match svc.status().as_str() {
                    "Running" => t.success,
                    "Failed" => t.destructive,
                    _ => t.text_tertiary,
                };
                let status = div()
                    .flex_row()
                    .items_center()
                    .gap(6.0)
                    .child(div().size(7.0).rounded(3.5).bg(if selected { ink } else { dot }))
                    .child(text(svc.status()).color(if svc.running() { ink } else { secondary }));
                let kind = if svc.scope == Scope::User { "User" } else { "System" };
                let (u1, u2) = (unit.clone(), unit.clone());
                let (user, running) = (svc.scope == Scope::User, svc.running());
                Row::new(("service", unit.clone(), kind), vec![
                    Cell::new(text(svc.name()).truncate()),
                    Cell::new(text(svc.description.clone()).color(secondary).truncate()),
                    Cell::new(status),
                    Cell::new(text(kind).color(secondary)),
                ])
                .selected(selected)
                .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                    s.menu = None;
                    s.service_sel = Some(u1.clone());
                    cx.focus("table");
                })
                .on_right_click(move |s: &mut Self, cx: &mut Cx| {
                    s.service_sel = Some(u2.clone());
                    let item = |label: &str, a: svc::Action, enabled: bool| MenuItem {
                        label: label.into(),
                        action: Some(MenuAction::Service(u2.clone(), a)),
                        enabled: user && enabled,
                    };
                    s.menu = Some(Menu {
                        pos: cx.mouse,
                        items: vec![
                            item("Start", svc::Action::Start, !running),
                            item("Stop", svc::Action::Stop, running),
                            item("Restart", svc::Action::Restart, running),
                        ],
                    });
                })
            });

        let options = [("All", None), ("User", Some(Scope::User)), ("System", Some(Scope::System))];
        let filter_bar = div()
            .h(40.0)
            .shrink_0()
            .flex_row()
            .items_center()
            .justify_between()
            .px(14.0)
            .border_b(t.divider)
            .child(self.segmented("svc-scope", &options, self.service_scope, t, |s, v, _| s.service_scope = v))
            .child(text("System services can only be viewed. Start and stop your own (user) services.").size(11.0).color(t.text_secondary).truncate().ml(16.0));
        div().id("table").focusable().flex_1().flex_col().min_h(0.0).child(filter_bar).child(table)
    }

    pub(super) fn services_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        let units: Vec<Option<String>> = self.service_rows().iter().map(|s| Some(s.unit.clone())).collect();
        let d = if key.is_named(NamedKey::ArrowDown) {
            1
        } else if key.is_named(NamedKey::ArrowUp) {
            -1
        } else if key.is_named(NamedKey::F5) {
            self.refresh_services(cx);
            return true;
        } else {
            return false;
        };
        if let Some((i, u)) = step(&units, self.service_sel.as_ref(), d) {
            self.service_sel = Some(u);
            self.service_table.reveal(i);
        }
        true
    }
}
