//! The day and week views: an all-day strip over a scrolling hour grid, with
//! events as colored blocks you can sweep out, move and resize.

use calendar_core::model::{date_of, local};
use calendar_core::{Event, Occurrence};
use chrono::{Datelike, NaiveDate, Weekday};
use ui::*;

use super::layout::{self, DAY_MIN, HOUR_H, View, minute_of, minutes_to_y};
use super::model::{hour_label, time_label};
use super::theme::Theme;
use super::{CalendarApp, DragKind, icons, today};

/// Width of the hour labels on the left.
pub const GUTTER: f32 = 56.0;
const ALL_DAY_ROW: f32 = 20.0;
/// Hour labels this close to the current time's label make way for it.
const NOW_LABEL_CLEARANCE: f32 = 18.0;

impl CalendarApp {
    pub(super) fn grid_view(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let days = self.grid_days();
        let n = days.len();
        let cols_w = (self.content_width(cx) - GUTTER).max(100.0);
        let col_w = cols_w / n as f32;
        let occs = self.store.occurrences(days[0], days[n - 1], &self.hidden);

        let mut body = div().flex_1().flex_col().min_h(0.0);
        if self.view == View::Week {
            body = body.child(self.day_headers(&days, col_w, t));
        }
        body.child(self.all_day_strip(&days, col_w, &occs, t))
            .child(div().h(1.0).shrink_0().bg(t.divider))
            .child(
                div()
                    .id("grid-scroll")
                    .scroll_y()
                    .flex_1()
                    .flex_col()
                    .child(
                        div()
                            .flex_row()
                            .pt(10.0)
                            .pb(10.0)
                            .child(self.gutter(&days, t))
                            .child(self.columns(&days, col_w, &occs, t)),
                    ),
            )
    }

    /// "Sun 8 ... Sat 14", today's number in red.
    fn day_headers(&self, days: &[NaiveDate], col_w: f32, t: &Theme) -> Div<Self> {
        let today = today();
        let mut row = div().h(30.0).shrink_0().flex_row().items_center().pl(GUTTER);
        for &d in days {
            let num: Div<Self> = if d == today {
                div()
                    .h(22.0)
                    .px(6.0)
                    .rounded(6.0)
                    .bg(t.red)
                    .items_center()
                    .child(text(d.day().to_string()).size(13.0).weight(Weight::Semibold).tabular().color(Color::WHITE))
            } else {
                div().child(text(d.day().to_string()).size(13.0).weight(Weight::Medium).tabular())
            };
            let label = text(d.format("%a").to_string()).size(13.0).color(if d == today { t.red } else { t.text_secondary });
            row = row.child(
                div()
                    .id(("day-header", d))
                    .w(col_w)
                    .h_full()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(5.0)
                    .on_double_click(move |s: &mut Self, cx: &mut Cx| {
                        s.go_to(d, cx);
                        s.set_view(View::Day, cx);
                    })
                    .child(label)
                    .child(num),
            );
        }
        row
    }

    fn all_day_strip(&self, days: &[NaiveDate], col_w: f32, occs: &[Occurrence], t: &Theme) -> Div<Self> {
        let all_day: Vec<&Occurrence> = occs.iter().filter(|o| o.all_day).collect();
        let first = days[0];
        let last = days[days.len() - 1];
        let spans: Vec<(usize, usize)> = all_day
            .iter()
            .map(|o| {
                let a = (date_of(o.start) - first).num_days().max(0) as usize;
                let b = (date_of((o.end - 1).max(o.start)).min(last) - first).num_days().max(0) as usize;
                (a, b.max(a))
            })
            .collect();
        let rows = layout::bar_rows(&spans);
        let n_rows = rows.iter().max().map_or(0, |r| r + 1);
        let h = (n_rows.max(1) as f32 * ALL_DAY_ROW + 6.0).max(26.0);

        let mut bars = div().w(col_w * days.len() as f32).h(h);
        for ((o, &(a, b)), &row) in all_day.iter().zip(&spans).zip(&rows) {
            let Some(ev) = self.store.event(o.event) else { continue };
            let selected = self.selected == Some(ev.id);
            let (fill, bar, ink) = t.event_colors(self.store.color_of(ev), selected);
            let (id, start, end) = (ev.id, o.start, o.end);
            // Ends that continue past the visible days are squared off.
            let left = date_of(o.start) >= first;
            let right = date_of((o.end - 1).max(o.start)) <= last;
            let x = a as f32 * col_w + if left { 3.0 } else { 0.0 };
            let w = (b - a + 1) as f32 * col_w - if left { 3.0 } else { 0.0 } - if right { 4.0 } else { 0.0 };
            bars = bars.child(
                div()
                    .id(("all-day", id, start))
                    .absolute(x, 3.0 + row as f32 * ALL_DAY_ROW)
                    .w(w)
                    .h(ALL_DAY_ROW - 2.0)
                    .rounded(if left && right { 4.0 } else { 2.0 })
                    .bg(fill)
                    .flex_row()
                    .items_center()
                    .clip()
                    .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                        s.menu = None;
                        if s.editor.as_ref().is_some_and(|e| e.event != id) {
                            s.close_editor(cx);
                        }
                        s.selected = Some(id);
                    })
                    .on_double_click(move |s: &mut Self, cx: &mut Cx| {
                        let anchor = Bounds::new(cx.mouse.x - 30.0, cx.mouse.y - 9.0, 60.0, 18.0);
                        s.open_editor(id, start, end, anchor, cx);
                    })
                    .child_if(left, || div().w(3.0).h_full().bg(bar).into())
                    .child(text(ev.title.clone()).size(11.5).weight(Weight::Medium).color(ink).truncate().pl(5.0)),
            );
        }
        div()
            .h(h)
            .shrink_0()
            .flex_row()
            .child(
                div()
                    .w(GUTTER)
                    .h_full()
                    .pt(5.0)
                    .pr(8.0)
                    .flex_row()
                    .justify_end()
                    .child(text("all-day").size(11.0).color(t.text_secondary)),
            )
            .child(bars)
    }

    /// Hour labels, with the current time in red.
    fn gutter(&self, days: &[NaiveDate], t: &Theme) -> Div<Self> {
        let now = today();
        let now_y = days.contains(&now).then(|| minutes_to_y(minute_of(y7_sync::now(), now) as f32));
        let mut g = div().w(GUTTER).h(24.0 * HOUR_H).shrink_0();
        for h in 0..24u32 {
            let y = h as f32 * HOUR_H;
            if now_y.is_some_and(|ny| (ny - y).abs() < NOW_LABEL_CLEARANCE) {
                continue;
            }
            g = g.child(
                div()
                    .absolute(0.0, y - 7.0)
                    .w(GUTTER - 8.0)
                    .flex_row()
                    .justify_end()
                    .child(text(hour_label(h)).size(11.0).tabular().color(t.text_secondary)),
            );
        }
        if let Some(y) = now_y {
            let label = local(y7_sync::now()).format("%-I:%M").to_string();
            g = g.child(
                div()
                    .absolute(0.0, y - 7.0)
                    .w(GUTTER - 8.0)
                    .flex_row()
                    .justify_end()
                    .child(text(label).size(11.0).weight(Weight::Semibold).tabular().color(t.red)),
            );
        }
        g
    }

    /// The day columns: lines, events, the drag preview and the time line.
    fn columns(&self, days: &[NaiveDate], col_w: f32, occs: &[Occurrence], t: &Theme) -> Div<Self> {
        let n = days.len();
        let weekend: Vec<bool> = days.iter().map(|d| n > 1 && matches!(d.weekday(), Weekday::Sat | Weekday::Sun)).collect();
        let (grid, shade, divider) = (t.grid, t.shade, t.grid);
        let bounds = self.grid_bounds.clone();
        let lines = canvas(move |s, b| {
            // Remember where the grid is, to turn the pointer into a time.
            bounds.set(b);
            for (i, &we) in weekend.iter().enumerate() {
                if we {
                    s.rect(Bounds::new(b.x + i as f32 * col_w, b.y, col_w, b.h), shade, 0.0);
                }
            }
            for h in 0..=24 {
                let y = (b.y + h as f32 * HOUR_H).round();
                s.rect(Bounds::new(b.x, y, b.w, 1.0), grid, 0.0);
            }
            for i in 0..n {
                let x = (b.x + i as f32 * col_w).round();
                s.rect(Bounds::new(x, b.y, 1.0, b.h), divider, 0.0);
            }
        })
        .absolute(0.0, 0.0)
        .w(col_w * n as f32)
        .h(24.0 * HOUR_H);

        let mut cols = div()
            .id("grid")
            .w(col_w * n as f32)
            .h(24.0 * HOUR_H)
            .shrink_0()
            .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.grid_down(cx))
            .on_double_click(|s: &mut Self, cx: &mut Cx| s.grid_double_click(cx))
            .child(lines);

        // What a drag in progress shows instead of the event's real place.
        let dragging = self.drag.filter(|d| d.moved);
        let dragged = dragging.and_then(|d| match d.kind {
            DragKind::Move { event, start, .. } | DragKind::Resize { event, start, .. } => Some((event, start)),
            DragKind::Create { .. } => None,
        });

        for (i, &day) in days.iter().enumerate() {
            let mut segs: Vec<(&Occurrence, (i32, i32))> = occs
                .iter()
                .filter(|o| !o.all_day && dragged != Some((o.event, o.start)))
                .filter_map(|o| layout::day_segment(o.start, o.end, day).map(|seg| (o, seg)))
                .collect();
            segs.sort_by_key(|(o, seg)| (seg.0, -seg.1, o.event));
            let placed = layout::columns(&segs.iter().map(|(_, s)| *s).collect::<Vec<_>>());
            for ((o, seg), (col, cols_n)) in segs.iter().zip(placed) {
                let Some(ev) = self.store.event(o.event) else { continue };
                // Leave room on the right to click the empty grid, like Calendar.
                let avail = col_w - 10.0;
                let sub = avail / cols_n as f32;
                let x = i as f32 * col_w + 2.0 + col as f32 * sub;
                cols = cols.child(self.block(ev, (o.start, o.end), day, *seg, x, sub - 2.0, false, t));
            }
            // The event being dragged, where it would land.
            if let Some(drag) = dragging {
                let (s, e) = drag.preview;
                let Some(seg) = layout::day_segment(s, e, day) else { continue };
                let x = i as f32 * col_w + 2.0;
                let w = col_w - 12.0;
                match drag.kind {
                    DragKind::Create { .. } => {
                        let cal = self.store.calendar(self.default_calendar()).map_or("blue".to_string(), |c| c.color.clone());
                        let preview = Event {
                            id: 0,
                            calendar: 0,
                            title: "New Event".into(),
                            start: s,
                            end: e,
                            all_day: false,
                            location: String::new(),
                            notes: String::new(),
                            rrule: String::new(),
                            created: 0,
                            modified: 0,
                        };
                        cols = cols.child(self.block_with_color(&preview, &cal, (s, e), day, seg, x, w, true, t));
                    }
                    DragKind::Move { event, .. } | DragKind::Resize { event, .. } => {
                        if let Some(ev) = self.store.event(event) {
                            cols = cols.child(self.block(ev, (s, e), day, seg, x, w, true, t));
                        }
                    }
                }
            }
        }

        // The current time.
        let now = today();
        if let Some(i) = days.iter().position(|d| *d == now) {
            let y = minutes_to_y(minute_of(y7_sync::now(), now) as f32);
            let x = i as f32 * col_w;
            cols = cols
                .child(div().absolute(0.0, y - 0.5).w(col_w * n as f32).h(1.0).bg(t.red.alpha(0.35)))
                .child(div().absolute(x, y - 1.0).w(col_w).h(2.0).bg(t.red))
                .child(div().absolute(x - 4.0, y - 4.0).size(8.0).rounded(4.0).bg(t.red));
        }
        cols
    }

    #[allow(clippy::too_many_arguments)]
    fn block(&self, ev: &Event, occ: (i64, i64), day: NaiveDate, seg: (i32, i32), x: f32, w: f32, lifted: bool, t: &Theme) -> Div<Self> {
        self.block_with_color(ev, self.store.color_of(ev), occ, day, seg, x, w, lifted, t)
    }

    /// One event on the hour grid. `lifted` draws it as being dragged.
    #[allow(clippy::too_many_arguments)]
    fn block_with_color(
        &self,
        ev: &Event,
        color: &str,
        occ: (i64, i64),
        day: NaiveDate,
        (s, e): (i32, i32),
        x: f32,
        w: f32,
        lifted: bool,
        t: &Theme,
    ) -> Div<Self> {
        let selected = lifted || self.selected == Some(ev.id);
        let (fill, bar, ink) = t.event_colors(color, selected);
        let y = minutes_to_y(s as f32) + 1.0;
        let h = (minutes_to_y((e - s) as f32) - 2.0).max(15.0);
        let id = ev.id;
        let time = if minute_of(occ.0, day) < 0 { format!("from {}", time_label(occ.0)) } else { time_label(occ.0) };
        let mut content = div().flex_col().flex_1().min_w_0().pl(5.0).pr(3.0).pt(2.0);
        if h < 34.0 {
            content = content.child(
                div()
                    .flex_row()
                    .gap(5.0)
                    .child(text(ev.title.clone()).size(11.0).weight(Weight::Semibold).color(ink).truncate())
                    .child(text(time).size(11.0).tabular().color(ink.alpha(0.8)).shrink_0()),
            );
        } else {
            // Titles wrap only where there's room to show the time below them.
            let title = text(ev.title.clone()).size(11.5).weight(Weight::Semibold).color(ink).line_height(1.25);
            content = content.child(if h >= 70.0 { title } else { title.truncate() });
            if !ev.location.is_empty() && h > 52.0 {
                content = content.child(text(ev.location.clone()).size(11.0).color(ink.alpha(0.85)).truncate());
            }
            let repeat = !ev.rrule.is_empty();
            content = content.child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(3.0)
                    .child(text(time).size(11.0).tabular().color(ink.alpha(0.8)))
                    .child_if(repeat, || {
                        let c = ink.alpha(0.8);
                        canvas(move |sc, b| icons::repeat(sc, b, c)).size(11.0).into()
                    }),
            );
        }
        let mut block = div()
            .id(("event", id, occ.0, day))
            .absolute(x, y)
            .w(w.max(8.0))
            .h(h)
            .rounded(4.0)
            .bg(fill)
            .flex_row()
            .clip()
            .child(div().w(3.0).h_full().bg(bar))
            .child(content);
        if lifted {
            block = block.shadow(t.shadow, 8.0, 3.0);
        }
        if id == 0 {
            return block;
        }
        block
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.event_down(id, occ, day, cx))
            .on_double_click(move |s: &mut Self, cx: &mut Cx| {
                s.drag = None;
                let anchor = s.block_anchor(day, minute_of(occ.0, day).max(0), minute_of(occ.1, day).min(DAY_MIN));
                s.open_editor(id, occ.0, occ.1, anchor, cx);
            })
    }
}
