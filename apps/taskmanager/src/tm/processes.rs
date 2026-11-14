//! The Processes page: apps (with their child processes) and background
//! processes, with heat-mapped resource columns.

use std::cmp::Ordering;

use ui::widgets::table::{Cell, Column, Row, Table};
use ui::*;

use super::{MenuAction, MenuItem, Menu, Sheet, TaskManager, format, icons, step};
use super::theme::Theme;
use crate::sys::process::Process;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Sel {
    App(String),
    Pid(i32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PCol {
    Name,
    Status,
    Cpu,
    Memory,
    Disk,
    Network,
}

const COLS: [PCol; 6] = [PCol::Name, PCol::Status, PCol::Cpu, PCol::Memory, PCol::Disk, PCol::Network];

enum PRow {
    Section(&'static str, usize),
    /// Index into the grouping's apps.
    App(usize, bool),
    Child(i32),
    Background(i32),
}

/// What a row shows and sorts by.
struct Vals {
    name: String,
    status: &'static str,
    cpu: f32,
    mem: u64,
    disk: f64,
}

fn compare(a: &Vals, b: &Vals, col: PCol, desc: bool) -> Ordering {
    let o = match col {
        PCol::Name | PCol::Network => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        PCol::Status => a.status.cmp(b.status),
        PCol::Cpu => a.cpu.total_cmp(&b.cpu),
        PCol::Memory => a.mem.cmp(&b.mem),
        PCol::Disk => a.disk.total_cmp(&b.disk),
    };
    let o = if desc { o.reverse() } else { o };
    o.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
}

pub fn matches(p: &Process, q: &str) -> bool {
    q.is_empty() || p.name.to_lowercase().contains(q) || p.pid.to_string() == q || p.cmdline.to_lowercase().contains(q)
}

impl TaskManager {
    fn proc_vals(&self, p: &Process) -> Vals {
        let status = if self.reaper.is_pending(p.pid) { "Ending…" } else { p.status() };
        Vals { name: p.name.clone(), status, cpu: p.cpu, mem: p.mem, disk: p.disk }
    }

    fn group_vals(&self, gi: usize) -> Vals {
        let g = &self.sampler.grouping.apps[gi];
        let ps: Vec<&Process> = g.pids.iter().filter_map(|&pid| self.sampler.process(pid)).collect();
        let ending = g.pids.iter().any(|&pid| self.reaper.is_pending(pid));
        Vals {
            name: g.name.clone(),
            status: if ending { "Ending…" } else { "" },
            cpu: ps.iter().map(|p| p.cpu).sum(),
            mem: ps.iter().map(|p| p.mem).sum(),
            disk: ps.iter().map(|p| p.disk).sum(),
        }
    }

    fn proc_rows(&self) -> Vec<PRow> {
        let q = self.query();
        let (col, desc) = self.proc_sort;
        let grouping = &self.sampler.grouping;

        let mut apps: Vec<(usize, Vals, bool)> = Vec::new();
        for (gi, g) in grouping.apps.iter().enumerate() {
            let by_name = q.is_empty() || g.name.to_lowercase().contains(&q);
            let by_member = !q.is_empty() && g.pids.iter().filter_map(|&p| self.sampler.process(p)).any(|p| matches(p, &q));
            if by_name || by_member {
                apps.push((gi, self.group_vals(gi), by_member && !by_name));
            }
        }
        apps.sort_by(|a, b| compare(&a.1, &b.1, col, desc));

        let mut bg: Vec<(i32, Vals)> = grouping
            .background
            .iter()
            .filter_map(|&pid| self.sampler.process(pid))
            .filter(|p| matches(p, &q))
            .map(|p| (p.pid, self.proc_vals(p)))
            .collect();
        bg.sort_by(|a, b| compare(&a.1, &b.1, col, desc));

        let mut rows = Vec::new();
        if !apps.is_empty() {
            rows.push(PRow::Section("Apps", apps.len()));
        }
        for (gi, _, reveal) in apps {
            let g = &grouping.apps[gi];
            let open = reveal || self.expanded.contains(&g.key);
            rows.push(PRow::App(gi, open));
            if open {
                let mut kids: Vec<(i32, Vals)> =
                    g.pids.iter().filter_map(|&pid| self.sampler.process(pid)).map(|p| (p.pid, self.proc_vals(p))).collect();
                kids.sort_by(|a, b| compare(&a.1, &b.1, col, desc));
                rows.extend(kids.into_iter().map(|(pid, _)| PRow::Child(pid)));
            }
        }
        if !bg.is_empty() {
            rows.push(PRow::Section("Background processes", bg.len()));
        }
        rows.extend(bg.into_iter().map(|(pid, _)| PRow::Background(pid)));
        rows
    }

    fn row_sel(&self, r: &PRow) -> Option<Sel> {
        match r {
            PRow::Section(..) => None,
            PRow::App(gi, _) => Some(Sel::App(self.sampler.grouping.apps[*gi].key.clone())),
            PRow::Child(pid) | PRow::Background(pid) => Some(Sel::Pid(*pid)),
        }
    }

    /// The process a selection stands for (an app's first process).
    fn sel_pid(&self, sel: &Sel) -> Option<i32> {
        match sel {
            Sel::Pid(p) => Some(*p),
            Sel::App(key) => self.sampler.grouping.apps.iter().find(|g| &g.key == key).and_then(|g| g.pids.first().copied()),
        }
    }

    pub(super) fn end_selection(&mut self, sel: &Sel) {
        let pids = match sel {
            Sel::Pid(p) => vec![*p],
            // Ending an app ends all of its processes.
            Sel::App(key) => self.sampler.grouping.apps.iter().find(|g| &g.key == key).map(|g| g.pids.clone()).unwrap_or_default(),
        };
        self.end_pids(&pids);
    }

    pub(super) fn process_menu(&mut self, sel: Sel, cx: &mut Cx) {
        self.proc_sel = Some(sel.clone());
        cx.focus("table");
        let Some(pid) = self.sel_pid(&sel) else { return };
        let has_exe = self.sampler.process(pid).is_some_and(|p| p.exe.is_some());
        let item = |label: &str, a: MenuAction, enabled: bool| MenuItem { label: label.into(), action: Some(a), enabled };
        let sep = || MenuItem { label: String::new(), action: None, enabled: true };
        self.menu = Some(Menu {
            pos: cx.mouse,
            items: vec![
                item("End task", MenuAction::EndTask(sel.clone()), true),
                item("End process tree", MenuAction::EndTree(pid), true),
                sep(),
                item("Open file location", MenuAction::OpenLocation(pid), has_exe),
                item("Properties", MenuAction::Properties(pid), true),
            ],
        });
    }

    fn toggle_expanded(&mut self, key: &str) {
        if !self.expanded.remove(key) {
            self.expanded.insert(key.to_string());
        }
    }

    pub(super) fn processes_page(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let rows = self.proc_rows();
        let sys = &self.sampler.system;
        let cpu = sys.cpu.util.last();
        let mem = sys.mem.used.last();
        let disk = sys.disks.iter().map(|d| d.active.last()).fold(0.0, f32::max);
        let net: f32 = sys.nets.iter().map(|n| n.rx.last() + n.tx.last()).sum();
        let columns = [
            Column::new("Name"),
            Column::new("Status").width(96.0),
            Column::new("CPU").width(78.0).right().summary(format!("{cpu:.0}%"), Some(t.heat(cpu / 100.0))),
            Column::new("Memory").width(100.0).right().summary(format!("{mem:.0}%"), Some(t.heat(mem / 100.0))),
            Column::new("Disk").width(92.0).right().summary(format!("{disk:.0}%"), Some(t.heat(disk / 100.0))),
            Column::new("Network").width(92.0).right().summary(format::bits(net as f64), Some(t.heat(0.0))),
        ];
        let sort_col = COLS.iter().position(|c| *c == self.proc_sort.0);
        let style = self.table_style(t, cx);
        let total_mem = sys.mem.info.total.max(1) as f32;
        let table = Table::new(&self.proc_table, &columns, rows.len(), style)
            .sort(sort_col.map(|c| (c, self.proc_sort.1)))
            .on_sort(|s: &mut Self, c, _| {
                let col = COLS[c];
                s.proc_sort = if s.proc_sort.0 == col {
                    (col, !s.proc_sort.1)
                } else {
                    (col, !matches!(col, PCol::Name | PCol::Status))
                };
            })
            .render(cx, |i, _| self.proc_row(&rows[i], total_mem, t, &style));
        div().id("table").focusable().flex_1().flex_col().min_h(0.0).child(table)
    }

    fn proc_row(&self, r: &PRow, total_mem: f32, t: &Theme, style: &ui::widgets::table::TableStyle) -> Row<Self> {
        let sel = self.row_sel(r);
        let selected = sel.is_some() && sel == self.proc_sel;
        let ink = if selected { style.selection_text } else { t.text };
        let secondary = if selected { ink.alpha(0.75) } else { t.text_secondary };

        let (name_el, vals, key): (Div<Self>, Vals, String) = match r {
            PRow::Section(title, n) => {
                let label = div()
                    .flex_row()
                    .items_center()
                    .gap(6.0)
                    .pt(6.0)
                    .child(text(title.to_string()).weight(Weight::Semibold))
                    .child(text(format!("({n})")).color(t.text_secondary));
                return Row::section(("section", *title), label);
            }
            PRow::App(gi, open) => {
                let g = &self.sampler.grouping.apps[*gi];
                let key = g.key.clone();
                let open = *open;
                let chevron = div()
                    .id(("disclose", key.clone()))
                    .size(16.0)
                    .shrink_0()
                    .rounded(4.0)
                    .items_center()
                    .justify_center()
                    .on_mouse_down({
                        let key = key.clone();
                        move |s: &mut Self, _| s.toggle_expanded(&key)
                    })
                    .child(canvas(move |s, b| icons::disclosure(s, b, secondary, open)).size(16.0));
                let el = div()
                    .flex_row()
                    .items_center()
                    .gap(6.0)
                    .min_w_0()
                    .child(chevron)
                    .child(Self::app_glyph(&g.name, 16.0))
                    .child(text(g.name.clone()).truncate())
                    .child(text(format!("({})", g.pids.len())).color(secondary).shrink_0());
                (el, self.group_vals(*gi), key)
            }
            PRow::Child(pid) | PRow::Background(pid) => {
                let Some(p) = self.sampler.process(*pid) else { return Row::new(("gone", *pid), Vec::new()) };
                let indent = if matches!(r, PRow::Child(_)) { 44.0 } else { 22.0 };
                let el = div()
                    .flex_row()
                    .items_center()
                    .gap(6.0)
                    .min_w_0()
                    .pl(indent)
                    .child(canvas(move |s, b| icons::program(s, b, secondary)).size(16.0).shrink_0())
                    .child(text(p.name.clone()).truncate());
                (el, self.proc_vals(p), p.pid.to_string())
            }
        };

        let num = |s: String| text(s).color(ink);
        let cells = vec![
            Cell::new(name_el),
            Cell::new(text(vals.status).color(secondary).truncate()),
            Cell::new(num(format::percent(vals.cpu))).bg(Some(t.heat(vals.cpu / 50.0))),
            Cell::new(num(format::bytes(vals.mem))).bg(Some(t.heat(vals.mem as f32 / total_mem * 8.0))),
            Cell::new(num(format::disk_rate(vals.disk))).bg(Some(t.heat(vals.disk as f32 / 50e6))),
            // Per-process traffic needs privileged accounting (eBPF/netlink), so it isn't shown.
            Cell::new(text("—").color(if selected { secondary } else { t.text_tertiary })).bg(Some(t.heat(0.0))),
        ];
        let sel = sel.unwrap();
        let (s1, s2, s3) = (sel.clone(), sel.clone(), sel);
        let is_app = matches!(r, PRow::App(..));
        Row::new(("proc", key), cells)
            .selected(selected)
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                s.menu = None;
                s.proc_sel = Some(s1.clone());
                cx.focus("table");
            })
            .on_double_click(move |s: &mut Self, _| match &s2 {
                Sel::App(key) if is_app => {
                    let key = key.clone();
                    s.toggle_expanded(&key);
                }
                Sel::Pid(pid) => s.sheet = Some(Sheet::Properties(*pid)),
                _ => {}
            })
            .on_right_click(move |s: &mut Self, cx: &mut Cx| s.process_menu(s3.clone(), cx))
    }

    pub(super) fn processes_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        let rows = self.proc_rows();
        let keys: Vec<Option<Sel>> = rows.iter().map(|r| self.row_sel(r)).collect();
        let page = self.proc_table.page_rows(super::ROW_H) as i32;
        let delta = match &key.key {
            Key::Named(NamedKey::ArrowDown) => Some(1),
            Key::Named(NamedKey::ArrowUp) => Some(-1),
            Key::Named(NamedKey::PageDown) => Some(page),
            Key::Named(NamedKey::PageUp) => Some(-page),
            Key::Named(NamedKey::Home) => Some(-(rows.len() as i32)),
            Key::Named(NamedKey::End) => Some(rows.len() as i32),
            _ => None,
        };
        if let Some(d) = delta {
            if let Some((i, sel)) = step(&keys, self.proc_sel.as_ref(), d) {
                self.proc_sel = Some(sel);
                self.proc_table.reveal(i);
            }
            return true;
        }
        let Some(sel) = self.proc_sel.clone() else { return false };
        if key.is_named(NamedKey::Delete) {
            self.end_selection(&sel);
            return true;
        }
        match (&sel, &key.key) {
            (Sel::App(k), Key::Named(NamedKey::ArrowRight)) => {
                self.expanded.insert(k.clone());
                true
            }
            (Sel::App(k), Key::Named(NamedKey::ArrowLeft)) => {
                self.expanded.remove(k);
                true
            }
            (Sel::App(k), Key::Named(NamedKey::Enter)) => {
                let k = k.clone();
                self.toggle_expanded(&k);
                true
            }
            // From a child, Left goes back to its app.
            (Sel::Pid(pid), Key::Named(NamedKey::ArrowLeft)) => {
                if let Some(g) = self.sampler.grouping.apps.iter().find(|g| g.pids.contains(pid)) {
                    self.proc_sel = Some(Sel::App(g.key.clone()));
                }
                true
            }
            (Sel::Pid(pid), Key::Named(NamedKey::Enter)) => {
                self.sheet = Some(Sheet::Properties(*pid));
                let _ = cx;
                true
            }
            _ => false,
        }
    }

    pub(super) fn properties_card(&self, pid: i32, t: &Theme) -> Div<Self> {
        let Some(p) = self.sampler.process(pid) else {
            return div().w(360.0).child(text("This process has ended.").color(t.text_secondary));
        };
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
        let running_for = (self.sampler.system.uptime - p.start_ticks as f64 / hz).max(0.0);
        let parent = self.sampler.process(p.ppid).map_or_else(|| p.ppid.to_string(), |pp| format!("{} ({})", pp.name, pp.pid));
        let app = self.sampler.grouping.apps.iter().find(|g| g.pids.contains(&pid)).map(|g| g.name.clone());
        let mut rows: Vec<(&str, String)> = vec![
            ("Process ID", pid.to_string()),
            ("Parent", parent),
            ("User", self.sampler.user_name(p.uid)),
            ("Status", if p.status().is_empty() { "Running".into() } else { p.status().into() }),
            ("Running for", format::uptime(running_for)),
            ("CPU time", format::uptime(p.cpu_ticks as f64 / hz)),
            ("Threads", p.threads.to_string()),
            ("Memory", format::bytes(p.mem)),
        ];
        if let Some(app) = app {
            rows.push(("App", app));
        }
        rows.push(("Location", p.exe.as_ref().map_or("—".into(), |e| e.display().to_string())));
        rows.push(("Command line", if p.cmdline.is_empty() { "—".into() } else { p.cmdline.clone() }));

        let mut grid = div().flex_col().gap(7.0).w_full();
        for (k, v) in rows {
            grid = grid.child(
                div()
                    .flex_row()
                    .gap(12.0)
                    .child(text(k).color(t.text_secondary).w(100.0).shrink_0())
                    .child(text(v).flex_1().min_w_0().line_height(1.35)),
            );
        }
        div()
            .w(460.0)
            .flex_col()
            .gap(14.0)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(10.0)
                    .child(Self::app_glyph(&p.name, 32.0))
                    .child(
                        div()
                            .flex_col()
                            .min_w_0()
                            .child(text(p.name.clone()).size(15.0).weight(Weight::Bold).truncate())
                            .child(text("Process properties").size(11.0).color(t.text_secondary)),
                    ),
            )
            .child(div().h(1.0).w_full().bg(t.divider))
            .child(grid)
            .child(
                div()
                    .flex_row()
                    .justify_end()
                    .gap(8.0)
                    .mt(4.0)
                    .child(Self::button("props-end", "End task", false, true, t, move |s: &mut Self, _| {
                        s.sheet = None;
                        s.end_pids(&[pid]);
                    }))
                    .child(Self::button("props-ok", "Done", true, true, t, |s: &mut Self, _| s.sheet = None)),
            )
    }
}
