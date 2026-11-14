//! The Users page: CPU and memory per user, expandable to their processes.

use std::collections::BTreeMap;

use ui::widgets::table::{Cell, Column, Row, Table};
use ui::*;

use super::processes::Sel;
use super::theme::Theme;
use super::{TaskManager, format, icons, step};
use crate::sys::process::Process;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum USel {
    User(u32),
    Pid(i32),
}

struct UserTotals {
    uid: u32,
    name: String,
    pids: Vec<i32>,
    cpu: f32,
    mem: u64,
    disk: f64,
}

impl TaskManager {
    fn user_totals(&self) -> Vec<UserTotals> {
        let q = self.query();
        let mut by: BTreeMap<u32, UserTotals> = BTreeMap::new();
        for p in self.sampler.procs.iter().filter(|p| !p.kernel) {
            let u = by.entry(p.uid).or_insert_with(|| UserTotals {
                uid: p.uid,
                name: self.sampler.user_name(p.uid),
                pids: Vec::new(),
                cpu: 0.0,
                mem: 0,
                disk: 0.0,
            });
            u.pids.push(p.pid);
            u.cpu += p.cpu;
            u.mem += p.mem;
            u.disk += p.disk;
        }
        let mut v: Vec<UserTotals> = by.into_values().filter(|u| q.is_empty() || u.name.to_lowercase().contains(&q)).collect();
        // Busiest first: people before system accounts, usually.
        v.sort_by(|a, b| b.mem.cmp(&a.mem));
        v
    }

    pub(super) fn user_rows_count(&self) -> usize {
        self.user_totals().len()
    }

    fn user_rows(&self) -> (Vec<UserTotals>, Vec<(USel, usize)>) {
        let users = self.user_totals();
        let mut rows = Vec::new();
        for (ui, u) in users.iter().enumerate() {
            rows.push((USel::User(u.uid), ui));
            if self.expanded_users.contains(&u.uid) {
                let mut ps: Vec<&Process> = u.pids.iter().filter_map(|&p| self.sampler.process(p)).collect();
                ps.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem.cmp(&a.mem)));
                rows.extend(ps.into_iter().map(|p| (USel::Pid(p.pid), ui)));
            }
        }
        (users, rows)
    }

    pub(super) fn users_page(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let (users, rows) = self.user_rows();
        let sys = &self.sampler.system;
        let total_mem = sys.mem.info.total.max(1) as f32;
        let (cpu, mem) = (sys.cpu.util.last(), sys.mem.used.last());
        let disk = sys.disks.iter().map(|d| d.active.last()).fold(0.0, f32::max);
        let columns = [
            Column::new("User"),
            Column::new("Processes").width(90.0).right(),
            Column::new("CPU").width(80.0).right().summary(format!("{cpu:.0}%"), Some(t.heat(cpu / 100.0))),
            Column::new("Memory").width(110.0).right().summary(format!("{mem:.0}%"), Some(t.heat(mem / 100.0))),
            Column::new("Disk").width(96.0).right().summary(format!("{disk:.0}%"), Some(t.heat(disk / 100.0))),
        ];
        let style = self.table_style(t, cx);
        let table = Table::new(&self.user_table, &columns, rows.len(), style).render(cx, |i| {
            let (sel, ui) = rows[i];
            let selected = self.user_sel == Some(sel);
            let ink = if selected { style.selection_text } else { t.text };
            let secondary = if selected { ink.alpha(0.75) } else { t.text_secondary };
            let u = &users[ui];
            let (name, count, cpu, mem, disk) = match sel {
                USel::User(uid) => {
                    let open = self.expanded_users.contains(&uid);
                    let el = div()
                        .flex_row()
                        .items_center()
                        .gap(6.0)
                        .min_w_0()
                        .child(
                            div()
                                .id(("user-disclose", uid))
                                .size(16.0)
                                .shrink_0()
                                .on_mouse_down(move |s: &mut Self, _| {
                                    if !s.expanded_users.remove(&uid) {
                                        s.expanded_users.insert(uid);
                                    }
                                })
                                .child(canvas(move |s, b| icons::disclosure(s, b, secondary, open)).size(16.0)),
                        )
                        .child(canvas(move |s, b| icons::person(s, b, secondary)).size(18.0).shrink_0())
                        .child(text(u.name.clone()).weight(Weight::Medium).truncate());
                    (el, u.pids.len().to_string(), u.cpu, u.mem, u.disk)
                }
                USel::Pid(pid) => {
                    let Some(p) = self.sampler.process(pid) else { return Row::new(("gone", pid), Vec::new()) };
                    let el = div()
                        .flex_row()
                        .items_center()
                        .gap(6.0)
                        .min_w_0()
                        .pl(44.0)
                        .child(canvas(move |s, b| icons::program(s, b, secondary)).size(16.0).shrink_0())
                        .child(text(p.name.clone()).truncate());
                    (el, String::new(), p.cpu, p.mem, p.disk)
                }
            };
            Row::new(("user-row", format!("{sel:?}")), vec![
                Cell::new(name),
                Cell::new(text(count).color(secondary)),
                Cell::new(text(format::percent(cpu))).bg(Some(t.heat(cpu / 50.0))),
                Cell::new(text(format::bytes(mem))).bg(Some(t.heat(mem as f32 / total_mem * 4.0))),
                Cell::new(text(format::disk_rate(disk))).bg(Some(t.heat(disk as f32 / 50e6))),
            ])
            .selected(selected)
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                s.menu = None;
                s.user_sel = Some(sel);
                cx.focus("table");
            })
            .on_double_click(move |s: &mut Self, _| match sel {
                USel::User(uid) => {
                    if !s.expanded_users.remove(&uid) {
                        s.expanded_users.insert(uid);
                    }
                }
                USel::Pid(pid) => s.sheet = Some(super::Sheet::Properties(pid)),
            })
            .on_right_click(move |s: &mut Self, cx: &mut Cx| {
                if let USel::Pid(pid) = sel {
                    s.user_sel = Some(sel);
                    s.process_menu(Sel::Pid(pid), cx);
                }
            })
        });
        div().id("table").focusable().flex_1().flex_col().min_h(0.0).child(table)
    }

    pub(super) fn users_key(&mut self, key: &KeyPress) -> bool {
        let (_, rows) = self.user_rows();
        let keys: Vec<Option<USel>> = rows.iter().map(|(s, _)| Some(*s)).collect();
        let d = match &key.key {
            Key::Named(NamedKey::ArrowDown) => 1,
            Key::Named(NamedKey::ArrowUp) => -1,
            Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::Enter) => {
                if let Some(USel::User(uid)) = self.user_sel {
                    match &key.key {
                        Key::Named(NamedKey::ArrowRight) => {
                            self.expanded_users.insert(uid);
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            self.expanded_users.remove(&uid);
                        }
                        _ => {
                            if !self.expanded_users.remove(&uid) {
                                self.expanded_users.insert(uid);
                            }
                        }
                    }
                }
                return true;
            }
            Key::Named(NamedKey::Delete) => {
                if let Some(USel::Pid(pid)) = self.user_sel {
                    self.end_pids(&[pid]);
                }
                return true;
            }
            _ => return false,
        };
        if let Some((i, s)) = step(&keys, self.user_sel.as_ref(), d) {
            self.user_sel = Some(s);
            self.user_table.reveal(i);
        }
        true
    }
}
