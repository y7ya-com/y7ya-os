//! The Details page: every process (and kernel thread) in one flat table.

use std::cmp::Ordering;

use ui::widgets::table::{Cell, Column, Row, Table};
use ui::*;

use super::processes::{Sel, matches};
use super::theme::Theme;
use super::{Sheet, TaskManager, format, step};
use crate::sys::process::Process;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DCol {
    Name,
    Pid,
    Status,
    User,
    Cpu,
    Memory,
    Threads,
    Command,
}

const COLS: [DCol; 8] = [DCol::Name, DCol::Pid, DCol::Status, DCol::User, DCol::Cpu, DCol::Memory, DCol::Threads, DCol::Command];

fn state_name(p: &Process) -> &'static str {
    match p.state {
        'R' => "Running",
        'S' => "Sleeping",
        'I' => "Idle",
        'D' => "Waiting",
        'T' | 't' => "Suspended",
        'Z' => "Zombie",
        _ => "Unknown",
    }
}

impl TaskManager {
    fn detail_rows(&self) -> Vec<&Process> {
        let q = self.query();
        let mut v: Vec<&Process> = self.sampler.procs.iter().filter(|p| matches(p, &q)).collect();
        let (col, desc) = self.detail_sort;
        v.sort_by(|a, b| {
            let o = match col {
                DCol::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                DCol::Pid => a.pid.cmp(&b.pid),
                DCol::Status => state_name(a).cmp(state_name(b)),
                DCol::User => a.uid.cmp(&b.uid),
                DCol::Cpu => a.cpu.total_cmp(&b.cpu),
                DCol::Memory => a.mem.cmp(&b.mem),
                DCol::Threads => a.threads.cmp(&b.threads),
                DCol::Command => a.cmdline.cmp(&b.cmdline),
            };
            let o = if desc { o.reverse() } else { o };
            if o == Ordering::Equal { a.pid.cmp(&b.pid) } else { o }
        });
        v
    }

    pub(super) fn details_page(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let rows = self.detail_rows();
        let columns = [
            Column::new("Name").width(190.0),
            Column::new("PID").width(70.0).right(),
            Column::new("Status").width(86.0),
            Column::new("User").width(96.0),
            Column::new("CPU").width(64.0).right(),
            Column::new("Memory").width(92.0).right(),
            Column::new("Threads").width(70.0).right(),
            Column::new("Command line"),
        ];
        let style = self.table_style(t, cx);
        let sort = COLS.iter().position(|c| *c == self.detail_sort.0).map(|c| (c, self.detail_sort.1));
        let table = Table::new(&self.detail_table, &columns, rows.len(), style)
            .sort(sort)
            .on_sort(|s: &mut Self, c, _| {
                let col = COLS[c];
                s.detail_sort = if s.detail_sort.0 == col {
                    (col, !s.detail_sort.1)
                } else {
                    (col, matches!(col, DCol::Cpu | DCol::Memory | DCol::Threads))
                };
            })
            .render(cx, |i, _| {
                let p = rows[i];
                let pid = p.pid;
                let selected = self.detail_sel == Some(pid);
                let secondary = super::row_secondary(selected, &style, t);
                let status = if self.reaper.is_pending(pid) { "Ending…" } else { state_name(p) };
                let cmd = if p.cmdline.is_empty() { format!("[{}]", p.name) } else { p.cmdline.clone() };
                Row::new(("detail", pid), vec![
                    Cell::new(text(p.name.clone()).truncate()),
                    Cell::new(text(pid.to_string()).tabular()),
                    Cell::new(text(status).color(secondary)),
                    Cell::new(text(self.sampler.user_name(p.uid)).truncate()),
                    Cell::new(text(format!("{:.0}", p.cpu)).tabular()),
                    Cell::new(text(format::bytes(p.mem)).tabular()),
                    Cell::new(text(p.threads.to_string()).tabular()),
                    Cell::new(text(cmd).color(secondary).truncate()),
                ])
                .selected(selected)
                .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                    s.menu = None;
                    s.detail_sel = Some(pid);
                    cx.focus("table");
                })
                .on_double_click(move |s: &mut Self, _| s.sheet = Some(Sheet::Properties(pid)))
                .on_right_click(move |s: &mut Self, cx: &mut Cx| {
                    s.detail_sel = Some(pid);
                    s.process_menu(Sel::Pid(pid), cx);
                })
            });
        div()
            .id("table")
            .focusable()
            .flex_1()
            .flex_col()
            .min_h(0.0)
            .child(table)
            .child_if(rows.is_empty(), || Self::empty_state("No Results", t).into())
    }

    pub(super) fn details_key(&mut self, key: &KeyPress) -> bool {
        let pids: Vec<Option<i32>> = self.detail_rows().iter().map(|p| Some(p.pid)).collect();
        let page = self.detail_table.page_rows(super::ROW_H) as i32;
        let d = match &key.key {
            Key::Named(NamedKey::ArrowDown) => 1,
            Key::Named(NamedKey::ArrowUp) => -1,
            Key::Named(NamedKey::PageDown) => page,
            Key::Named(NamedKey::PageUp) => -page,
            Key::Named(NamedKey::Home) => -(pids.len() as i32),
            Key::Named(NamedKey::End) => pids.len() as i32,
            Key::Named(NamedKey::Delete) => {
                if let Some(pid) = self.detail_sel {
                    self.end_pids(&[pid]);
                }
                return true;
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(pid) = self.detail_sel {
                    self.sheet = Some(Sheet::Properties(pid));
                }
                return true;
            }
            _ => return false,
        };
        if let Some((i, pid)) = step(&pids, self.detail_sel.as_ref(), d) {
            self.detail_sel = Some(pid);
            self.detail_table.reveal(i);
        }
        true
    }
}
