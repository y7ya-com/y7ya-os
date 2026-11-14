//! The Performance page: a list of devices with sparklines on the left, and
//! the selected device's live graph and details on the right.

use ui::widgets::graph::{Graph, Series, nice_max};
use ui::*;

use super::theme::Theme;
use super::{TaskManager, format};
use crate::sys::system::HISTORY;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PerfItem {
    Cpu,
    Memory,
    Disk(usize),
    Net(usize),
    Gpu,
}

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

fn gb(b: u64) -> String {
    format!("{:.1} GB", b as f64 / GIB)
}

impl TaskManager {
    fn perf_items(&self) -> Vec<PerfItem> {
        let sys = &self.sampler.system;
        let mut v = vec![PerfItem::Cpu, PerfItem::Memory];
        v.extend((0..sys.disks.len()).map(PerfItem::Disk));
        v.extend((0..sys.nets.len()).map(PerfItem::Net));
        if sys.gpu.is_some() {
            v.push(PerfItem::Gpu);
        }
        v
    }

    fn perf_color(&self, item: PerfItem, t: &Theme) -> Color {
        match item {
            PerfItem::Cpu => t.cpu,
            PerfItem::Memory => t.memory,
            PerfItem::Disk(_) => t.disk,
            PerfItem::Net(_) => t.network,
            PerfItem::Gpu => t.gpu,
        }
    }

    /// (title, one-line summary, sparkline series, sparkline max)
    fn perf_summary(&self, item: PerfItem) -> (String, String, Vec<f32>, f32) {
        let sys = &self.sampler.system;
        match item {
            PerfItem::Cpu => {
                (String::from("CPU"), format!("{:.0}%  {:.2} GHz", sys.cpu.util.last(), sys.cpu.speed_ghz), sys.cpu.util.values(), 100.0)
            }
            PerfItem::Memory => {
                let m = &sys.mem.info;
                let pct = sys.mem.used.last();
                (
                    "Memory".into(),
                    format!("{:.1}/{:.1} GB ({pct:.0}%)", m.used() as f64 / GIB, m.total as f64 / GIB),
                    sys.mem.used.values(),
                    100.0,
                )
            }
            PerfItem::Disk(i) => {
                let d = &sys.disks[i];
                (format!("Disk {i} ({})", d.name), format!("{}\n{:.0}%", d.kind, d.active.last()), d.active.values(), 100.0)
            }
            PerfItem::Net(i) => {
                let n = &sys.nets[i];
                let total: Vec<f32> = n.rx.values().iter().zip(n.tx.values()).map(|(a, b)| a + b).collect();
                let max = nice_max(total.iter().copied().fold(100e3, f32::max) * 8.0) / 8.0;
                let kind = if n.wireless { "Wi-Fi" } else { "Ethernet" };
                (kind.into(), format!("{}\nS: {} R: {}", n.name, format::bits(n.tx.last() as f64), format::bits(n.rx.last() as f64)), total, max)
            }
            PerfItem::Gpu => {
                let g = sys.gpu.as_ref().unwrap();
                ("GPU 0".into(), format!("{}\n{:.0}%", g.name, g.util.last()), g.util.values(), 100.0)
            }
        }
    }

    pub(super) fn performance_page(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let items = self.perf_items();
        if !items.contains(&self.perf_item) {
            self.perf_item = PerfItem::Cpu;
        }
        let mut list = div().id("perf-list").scroll_y().w(250.0).h_full().shrink_0().flex_col().gap(4.0).p(10.0);
        for item in items {
            list = list.child(self.perf_card(item, t));
        }
        let _ = cx;
        div()
            .id("table")
            .focusable()
            .flex_1()
            .min_h(0.0)
            .flex_row()
            .child(list)
            .child(div().w(1.0).h_full().shrink_0().bg(t.divider))
            .child(self.perf_detail(self.perf_item, t))
    }

    fn perf_card(&self, item: PerfItem, t: &Theme) -> Div<Self> {
        let selected = self.perf_item == item;
        let color = self.perf_color(item, t);
        let (title, summary, values, max) = self.perf_summary(item);
        let spark = Graph::new(HISTORY, max)
            .series(Series::new(values, color).fill(t.graph_fill(color)))
            .border(color.alpha(0.6))
            .background(t.content)
            .line_width(1.2)
            .element()
            .w(70.0)
            .h(46.0)
            .shrink_0();
        let mut lines = div().flex_col().flex_1().min_w_0().gap(1.0).child(text(title).size(14.0).weight(Weight::Medium).truncate());
        for l in summary.lines() {
            lines = lines.child(text(l.to_string()).size(11.0).color(t.text_secondary).truncate());
        }
        div()
            .id(("perf-item", format!("{item:?}")))
            .h(66.0)
            .px(8.0)
            .rounded(8.0)
            .flex_row()
            .items_center()
            .gap(10.0)
            .bg_opt(selected.then_some(t.sidebar_selection))
            .hover_bg(if selected { t.sidebar_selection } else { t.hover })
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                s.perf_item = item;
                cx.focus("table");
            })
            .child(spark)
            .child(lines)
    }

    /// A captioned live graph: labels above and below, like Task Manager's.
    fn big_graph(&self, caption: &str, top: &str, graph: Graph, t: &Theme, color: Color, flex: f32) -> Div<Self> {
        let mut g = graph.grid(12, 6, t.graph_grid(color)).border(color.alpha(0.7)).background(t.content).element().w_full().flex_1();
        g = g.min_h(80.0);
        let mut d = div()
            .flex_col()
            .gap(4.0)
            .min_h(0.0)
            .child(
                div()
                    .flex_row()
                    .justify_between()
                    .child(text(caption.to_string()).size(11.0).color(t.text_secondary))
                    .child(text(top.to_string()).size(11.0).color(t.text_secondary)),
            )
            .child(g)
            .child(
                div()
                    .flex_row()
                    .justify_between()
                    .child(text("60 seconds").size(11.0).color(t.text_secondary))
                    .child(text("0").size(11.0).color(t.text_secondary)),
            );
        d = d.flex_1();
        d.layout_mut().flex_grow = flex;
        d
    }

    fn stat(label: &str, value: String, t: &Theme) -> Div<Self> {
        div()
            .flex_col()
            .gap(1.0)
            .w(128.0)
            .child(text(label.to_string()).size(11.0).color(t.text_secondary))
            .child(text(value).size(19.0))
    }

    fn facts(rows: Vec<(&str, String)>, t: &Theme) -> Div<Self> {
        let mut d = div().flex_col().gap(4.0).w(270.0).shrink_0();
        for (k, v) in rows {
            d = d.child(
                div()
                    .flex_row()
                    .gap(10.0)
                    .child(text(k.to_string()).size(12.0).color(t.text_secondary).w(130.0).shrink_0())
                    .child(text(v).size(12.0).truncate()),
            );
        }
        d
    }

    fn perf_detail(&self, item: PerfItem, t: &Theme) -> Div<Self> {
        let sys = &self.sampler.system;
        let color = self.perf_color(item, t);
        let fill = t.graph_fill(color);
        let (title, subtitle, graphs, stats, facts): (String, String, Vec<Div<Self>>, Vec<Div<Self>>, Vec<(&str, String)>) = match item {
            PerfItem::Cpu => {
                let c = &sys.cpu;
                let threads: u64 = self.sampler.procs.iter().map(|p| p.threads as u64).sum();
                let g = Graph::new(HISTORY, 100.0).series(Series::new(c.util.values(), color).fill(fill));
                let mut facts = vec![
                    ("Base speed", c.info.base_ghz.map_or("—".into(), |g| format!("{g:.2} GHz"))),
                    ("Sockets", c.info.sockets.to_string()),
                    ("Cores", c.info.cores.to_string()),
                    ("Logical processors", c.info.logical.to_string()),
                    ("Virtualization", if c.info.virtualization { "Enabled".into() } else { "Not available".into() }),
                ];
                for (name, size) in &c.info.caches {
                    facts.push((name.as_str(), format::bytes(*size)));
                }
                (
                    "CPU".into(),
                    c.info.model.clone(),
                    vec![self.big_graph("% Utilization", "100%", g, t, color, 1.0)],
                    vec![
                        Self::stat("Utilization", format!("{:.0}%", c.util.last()), t),
                        Self::stat("Speed", format!("{:.2} GHz", c.speed_ghz), t),
                        Self::stat("Processes", format::count(self.sampler.procs.len() as u64), t),
                        Self::stat("Threads", format::count(threads), t),
                        Self::stat("Handles", format::count(sys.handles), t),
                        Self::stat("Up time", format::uptime(sys.uptime), t),
                    ],
                    facts,
                )
            }
            PerfItem::Memory => {
                let m = &sys.mem.info;
                let g = Graph::new(HISTORY, 100.0).series(Series::new(sys.mem.used.values(), color).fill(fill));
                let used = m.used();
                (
                    "Memory".into(),
                    format!("{} total", gb(m.total)),
                    vec![
                        self.big_graph("Memory usage", &gb(m.total), g, t, color, 1.0),
                        self.memory_composition(t, color),
                    ],
                    vec![
                        Self::stat("In use", gb(used), t),
                        Self::stat("Available", gb(m.available), t),
                        Self::stat("Committed", format!("{:.1}/{:.0} GB", m.committed as f64 / GIB, m.commit_limit as f64 / GIB), t),
                        Self::stat("Cached", gb(m.cache()), t),
                    ],
                    vec![
                        ("Total", gb(m.total)),
                        ("Free", gb(m.free)),
                        ("Shared", gb(m.shmem)),
                        ("Swap", format!("{} of {}", gb(m.swap_total.saturating_sub(m.swap_free)), gb(m.swap_total))),
                        ("Dirty", format::bytes(m.dirty)),
                    ],
                )
            }
            PerfItem::Disk(i) => {
                let d = &sys.disks[i];
                let active = Graph::new(HISTORY, 100.0).series(Series::new(d.active.values(), color).fill(fill));
                // Round the axis in the units it's labeled in (MB/s).
                const MB: f32 = 1024.0 * 1024.0;
                let top = nice_max(d.read.max().max(d.write.max()).max(MB) / MB) * MB;
                let rates = Graph::new(HISTORY, top)
                    .series(Series::new(d.read.values(), color).fill(fill))
                    .series(Series::new(d.write.values(), color).dashed());
                (
                    format!("Disk {i} ({})", d.name),
                    d.model.clone(),
                    vec![
                        self.big_graph("Active time", "100%", active, t, color, 1.2),
                        self.big_graph("Disk transfer rate (read solid, write dashed)", &format::rate(top as f64), rates, t, color, 1.0),
                    ],
                    vec![
                        Self::stat("Active time", format!("{:.0}%", d.active.last()), t),
                        Self::stat("Average response time", format!("{:.1} ms", d.response_ms), t),
                        Self::stat("Read speed", format::rate(d.read.last() as f64), t),
                        Self::stat("Write speed", format::rate(d.write.last() as f64), t),
                    ],
                    vec![("Capacity", gb(d.capacity)), ("Type", d.kind.into()), ("Model", d.model.clone()), ("Device", format!("/dev/{}", d.name))],
                )
            }
            PerfItem::Net(i) => {
                let n = &sys.nets[i];
                // Round the axis in bits per second, the unit it's labeled in.
                let top = nice_max(n.rx.max().max(n.tx.max()).max(100e3) * 8.0) / 8.0;
                let g = Graph::new(HISTORY, top)
                    .series(Series::new(n.rx.values(), color).fill(fill))
                    .series(Series::new(n.tx.values(), color).dashed());
                let kind = if n.wireless { "Wi-Fi" } else { "Ethernet" };
                (
                    kind.into(),
                    n.name.clone(),
                    vec![self.big_graph("Throughput (receive solid, send dashed)", &format::bits(top as f64), g, t, color, 1.0)],
                    vec![
                        Self::stat("Send", format::bits(n.tx.last() as f64), t),
                        Self::stat("Receive", format::bits(n.rx.last() as f64), t),
                    ],
                    vec![
                        ("Adapter name", n.name.clone()),
                        ("Connection type", kind.into()),
                        ("IPv4 address", n.ipv4.clone().unwrap_or_else(|| "—".into())),
                        ("Link speed", n.link_mbps.map_or("—".into(), |m| format!("{m} Mbps"))),
                    ],
                )
            }
            PerfItem::Gpu => {
                let g = sys.gpu.as_ref().unwrap();
                let graph = Graph::new(HISTORY, 100.0).series(Series::new(g.util.values(), color).fill(fill));
                let mut stats = vec![Self::stat("Utilization", format!("{:.0}%", g.util.last()), t)];
                if let Some((used, total)) = g.vram {
                    stats.push(Self::stat("Dedicated memory", format!("{:.1}/{:.1} GB", used as f64 / GIB, total as f64 / GIB), t));
                }
                (
                    "GPU 0".into(),
                    g.name.clone(),
                    vec![self.big_graph("% Utilization (3D)", "100%", graph, t, color, 1.0)],
                    stats,
                    vec![("Driver", g.driver.clone()), ("Memory", if g.vram.is_some() { "Dedicated".into() } else { "Shared with system".into() })],
                )
            }
        };

        // Big numbers in rows of three, Task Manager style.
        let mut stat_grid = div().flex_col().gap(12.0).flex_1().min_w_0();
        let mut stats = stats.into_iter().peekable();
        while stats.peek().is_some() {
            stat_grid = stat_grid.child(div().flex_row().gap(18.0).children(stats.by_ref().take(3)));
        }
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex_col()
            .gap(14.0)
            .px(24.0)
            .pt(16.0)
            .pb(18.0)
            .child(
                div()
                    .flex_row()
                    .items_end()
                    .justify_between()
                    .gap(16.0)
                    .child(text(title).size(24.0).weight(Weight::Bold).shrink_0())
                    .child(text(subtitle).size(14.0).color(t.text_secondary).truncate().mb(3.0)),
            )
            .children(graphs)
            .child(div().flex_row().gap(24.0).shrink_0().child(stat_grid).child(Self::facts(facts, t)))
    }

    /// Windows' "Memory composition" strip: in use, cached, free.
    fn memory_composition(&self, t: &Theme, color: Color) -> Div<Self> {
        let m = &self.sampler.system.mem.info;
        let total = m.total.max(1) as f32;
        let used = m.used() as f32 / total;
        let cached = (m.cache() as f32 / total).min(1.0 - used);
        let border = color.alpha(0.7);
        let bar = canvas(move |s, b| {
            s.rect(b, Color::TRANSPARENT, 0.0);
            let u = Bounds::new(b.x, b.y, (b.w * used).round(), b.h);
            let c = Bounds::new(u.right(), b.y, (b.w * cached).round(), b.h);
            s.rect(u, color.alpha(0.45), 0.0);
            s.rect(c, color.alpha(0.16), 0.0);
            s.rect(Bounds::new(u.right(), b.y, 1.0, b.h), border, 0.0);
            s.rect(Bounds::new(c.right(), b.y, 1.0, b.h), border.alpha(0.5), 0.0);
            s.rect_bordered(b, Color::TRANSPARENT, 0.0, border, 1.0);
        })
        .w_full()
        .h(30.0);
        let swatch = |c: Color, label: &str| {
            div()
                .flex_row()
                .items_center()
                .gap(5.0)
                .child(div().size(9.0).rounded(2.0).bg(c).border(border, 1.0))
                .child(text(label.to_string()).size(11.0).color(t.text_secondary))
        };
        div()
            .flex_col()
            .gap(4.0)
            .shrink_0()
            .child(text("Memory composition").size(11.0).color(t.text_secondary))
            .child(bar)
            .child(
                div()
                    .flex_row()
                    .gap(14.0)
                    .child(swatch(color.alpha(0.45), "In use"))
                    .child(swatch(color.alpha(0.16), "Cached"))
                    .child(swatch(Color::TRANSPARENT, "Free")),
            )
    }

    pub(super) fn performance_key(&mut self, key: &KeyPress) -> bool {
        let items = self.perf_items();
        let i = items.iter().position(|x| *x == self.perf_item).unwrap_or(0) as i32;
        let d = if key.is_named(NamedKey::ArrowDown) {
            1
        } else if key.is_named(NamedKey::ArrowUp) {
            -1
        } else {
            return false;
        };
        self.perf_item = items[(i + d).clamp(0, items.len() as i32 - 1) as usize];
        true
    }
}
