//! The month view (a grid of days listing their events) and the year view
//! (twelve small months).

use std::collections::HashMap;

use calendar_core::dates::month_grid;
use calendar_core::model::{date_key, date_of, local};
use calendar_core::Occurrence;
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use ui::*;

use super::layout::{self, View, at};
use super::model::time_label;
use super::theme::Theme;
use super::{CalendarApp, TOOLBAR_H, today};

const LINE_H: f32 = 18.0;
const WEEKDAYS_H: f32 = 26.0;

impl CalendarApp {
    pub(super) fn month_view(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let m = self.date.with_day(1).unwrap();
        let days = month_grid(m.year(), m.month(), self.first, false);
        let weeks = days.len() / 7;
        let w = self.content_width(cx);
        let col_w = w / 7.0;
        let grid_h = (cx.window_size.1 - TOOLBAR_H - 50.0 - WEEKDAYS_H).max(200.0);
        let cell_h = grid_h / weeks as f32;
        let occs = self.store.occurrences(days[0], days[days.len() - 1], &self.hidden);

        let mut header = div().h(WEEKDAYS_H).shrink_0().flex_row().items_center();
        for d in &days[..7] {
            header = header.child(
                div()
                    .w(col_w)
                    .flex_row()
                    .justify_end()
                    .pr(10.0)
                    .child(text(d.format("%a").to_string()).size(12.0).color(t.text_secondary)),
            );
        }

        let shaded: Vec<bool> = days.iter().map(|d| d.month() != m.month() || is_weekend(*d)).collect();
        let (line, shade) = (t.grid, t.shade);
        let lines = canvas(move |s, b| {
            for (i, &sh) in shaded.iter().enumerate() {
                if sh {
                    let (r, c) = (i / 7, i % 7);
                    s.rect(Bounds::new(b.x + c as f32 * col_w, b.y + r as f32 * cell_h, col_w, cell_h), shade, 0.0);
                }
            }
            for r in 0..weeks {
                s.rect(Bounds::new(b.x, (b.y + r as f32 * cell_h).round(), b.w, 1.0), line, 0.0);
            }
            for c in 1..7 {
                s.rect(Bounds::new((b.x + c as f32 * col_w).round(), b.y, 1.0, b.h), line, 0.0);
            }
        })
        .absolute(0.0, 0.0)
        .w(w)
        .h(grid_h);

        let mut grid = div().w(w).h(grid_h).shrink_0().flex_col().child(lines);
        for week in days.chunks(7) {
            let mut row = div().flex_row().h(cell_h);
            for &d in week {
                let day_occs: Vec<&Occurrence> = occs.iter().filter(|o| touches(o, d)).collect();
                row = row.child(self.month_cell(d, m, col_w, cell_h, &day_occs, t));
            }
            grid = grid.child(row);
        }
        div().flex_1().flex_col().child(header).child(grid)
    }

    fn month_cell(&self, d: NaiveDate, month: NaiveDate, w: f32, h: f32, occs: &[&Occurrence], t: &Theme) -> Div<Self> {
        let is_today = d == today();
        let in_month = d.month() == month.month();
        let label = if d.day() == 1 { d.format("%b %-d").to_string() } else { d.day().to_string() };
        let num_color = if is_today { Color::WHITE } else if in_month { t.text } else { t.text_tertiary };
        let mut num = div().h(22.0).px(if is_today { 7.0 } else { 2.0 }).rounded(11.0).items_center();
        if is_today {
            num = num.bg(t.red);
        }
        let num = num.child(text(label).size(12.5).weight(if is_today { Weight::Semibold } else { Weight::Regular }).color(num_color));

        let fits = (((h - 30.0) / LINE_H).floor() as usize).max(1);
        let shown = if occs.len() > fits { fits - 1 } else { occs.len() };
        let mut list = div().flex_col().gap(1.0).px(4.0);
        for o in &occs[..shown] {
            list = list.child(self.month_line(o, d, t));
        }
        if occs.len() > shown {
            list = list.child(
                div()
                    .id(("more", d))
                    .h(LINE_H)
                    .pl(6.0)
                    .items_center()
                    .on_double_click(move |s: &mut Self, cx: &mut Cx| {
                        s.go_to(d, cx);
                        s.set_view(View::Day, cx);
                    })
                    .child(text(format!("{} more", occs.len() - shown)).size(11.0).weight(Weight::Medium).color(t.text_secondary)),
            );
        }
        div()
            .id(("month-day", d))
            .w(w)
            .h(h)
            .flex_col()
            .clip()
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                s.menu = None;
                if s.editor.is_some() {
                    s.close_editor(cx);
                    return;
                }
                s.selected = None;
                if d.month() == s.date.month() {
                    s.date = d;
                } else {
                    s.go_to(d, cx);
                }
            })
            .on_double_click(move |s: &mut Self, cx: &mut Cx| {
                // A new all-day event on that day, like Calendar.
                let anchor = Bounds::new(cx.mouse.x - 40.0, cx.mouse.y - 10.0, 80.0, 20.0);
                s.create_event(date_key(d), date_key(d + Duration::days(1)), true, anchor, cx);
            })
            .child(div().h(26.0).shrink_0().flex_row().items_center().justify_end().pr(6.0).child(num))
            .child(list)
    }

    /// One event in a month cell: a filled bar for all-day events, a dot,
    /// title and time for the rest.
    fn month_line(&self, o: &Occurrence, day: NaiveDate, t: &Theme) -> Div<Self> {
        let ev = self.store.event(o.event).expect("occurrence of a known event");
        let selected = self.selected == Some(ev.id);
        let color = self.store.color_of(ev);
        let (id, start, end) = (ev.id, o.start, o.end);
        let line = div()
            .id(("month-event", id, start, day))
            .h(LINE_H)
            .rounded(4.0)
            .flex_row()
            .items_center()
            .gap(5.0)
            .px(5.0)
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                s.menu = None;
                if s.editor.as_ref().is_some_and(|e| e.event != id) {
                    s.close_editor(cx);
                }
                s.selected = Some(id);
            })
            .on_double_click(move |s: &mut Self, cx: &mut Cx| {
                let anchor = Bounds::new(cx.mouse.x - 40.0, cx.mouse.y - 9.0, 80.0, 18.0);
                s.open_editor(id, start, end, anchor, cx);
            });
        if o.all_day {
            let (fill, _, ink) = t.event_colors(color, selected);
            return line.bg(fill).child(text(ev.title.clone()).size(11.0).weight(Weight::Medium).color(ink).truncate().flex_1());
        }
        let c = t.cal(color);
        let (title_c, time_c) = if selected { (Color::WHITE, Color::WHITE.alpha(0.85)) } else { (t.text, t.text_secondary) };
        let time = if local(o.start).date() == day { time_label(o.start) } else { String::new() };
        let line = if selected { line.bg(c) } else { line };
        line.child(div().size(7.0).rounded(3.5).shrink_0().bg(if selected { Color::WHITE } else { c }))
            .child(text(ev.title.clone()).size(11.0).color(title_c).truncate().flex_1())
            .child(text(time).size(10.5).color(time_c).shrink_0())
    }

    pub(super) fn year_view(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let year = self.date.year();
        let first = NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
        let last = NaiveDate::from_ymd_opt(year, 12, 31).unwrap();
        // How much is on each day.
        let mut busy: HashMap<NaiveDate, usize> = HashMap::new();
        for o in self.store.occurrences(first, last, &self.hidden) {
            let (a, b) = if o.all_day {
                (date_of(o.start), date_of((o.end - 1).max(o.start)))
            } else {
                (local(o.start).date(), local((o.end - 1).max(o.start)).date())
            };
            let mut d = a.max(first);
            while d <= b.min(last) {
                *busy.entry(d).or_default() += 1;
                d += Duration::days(1);
            }
        }
        let pad = 24.0;
        let gap = 28.0;
        let w = self.content_width(cx) - 2.0 * pad;
        let mini_w = (w - 3.0 * gap) / 4.0;
        let avail_h = cx.window_size.1 - TOOLBAR_H - 50.0 - 16.0;
        let row_h = (avail_h / 3.0).max(150.0);
        let mut grid = div().flex_col().px(pad).pt(4.0);
        for r in 0..3 {
            let mut row = div().flex_row().gap(gap).h(row_h);
            for c in 0..4 {
                let month = (r * 4 + c + 1) as u32;
                row = row.child(self.year_month(year, month, mini_w, row_h, &busy, t));
            }
            grid = grid.child(row);
        }
        div().flex_1().flex_col().child(grid)
    }

    fn year_month(&self, year: i32, month: u32, w: f32, h: f32, busy: &HashMap<NaiveDate, usize>, t: &Theme) -> Div<Self> {
        let today = today();
        let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
        let current = today.year() == year && today.month() == month;
        let cell_w = w / 7.0;
        let cell_h = ((h - 52.0) / 6.0).clamp(16.0, 26.0);
        let mut weekdays = div().flex_row().h(18.0);
        let days = month_grid(year, month, self.first, true);
        for d in &days[..7] {
            let initial = d.format("%a").to_string().chars().next().unwrap_or(' ').to_string();
            weekdays = weekdays.child(div().w(cell_w).items_center().justify_center().child(text(initial).size(10.0).color(t.text_secondary)));
        }
        let mut grid = div().flex_col();
        for week in days.chunks(7) {
            let mut row = div().flex_row().h(cell_h);
            for &d in week {
                if d.month() != month {
                    row = row.child(div().w(cell_w));
                    continue;
                }
                let is_today = d == today;
                let mut dot = div().size((cell_h - 2.0).min(22.0)).rounded(11.0).items_center().justify_center();
                let color = if is_today {
                    dot = dot.bg(t.red);
                    Color::WHITE
                } else {
                    // Busier days are shaded deeper, like Calendar's year heat map.
                    if let Some(&n) = busy.get(&d) {
                        let a = [0.0, 0.10, 0.18, 0.27, 0.36][n.min(4)] * if t.dark { 1.6 } else { 1.0 };
                        dot = dot.bg(t.accent.alpha(a));
                    }
                    if is_weekend(d) { t.text_secondary } else { t.text }
                };
                row = row.child(
                    div()
                        .id(("year-day", d))
                        .w(cell_w)
                        .h(cell_h)
                        .items_center()
                        .justify_center()
                        .on_mouse_down(move |s: &mut Self, _: &mut Cx| s.date = d)
                        .on_double_click(move |s: &mut Self, cx: &mut Cx| {
                            s.go_to(d, cx);
                            s.set_view(View::Day, cx);
                        })
                        .child(dot.child(text(d.day().to_string()).size(11.0).weight(if is_today { Weight::Semibold } else { Weight::Regular }).color(color))),
                );
            }
            grid = grid.child(row);
        }
        div()
            .w(w)
            .flex_col()
            .child(
                div()
                    .id(("year-month", month))
                    .h(30.0)
                    .flex_row()
                    .items_center()
                    .pl(4.0)
                    .cursor(CursorIcon::Pointer)
                    .on_click(move |s: &mut Self, cx: &mut Cx| {
                        s.go_to(first, cx);
                        s.set_view(View::Month, cx);
                    })
                    .child(text(first.format("%B").to_string()).size(15.0).weight(Weight::Semibold).color(if current { t.red } else { t.text })),
            )
            .child(weekdays)
            .child(grid)
    }
}

fn is_weekend(d: NaiveDate) -> bool {
    matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// Whether occurrence `o` is on local day `d`.
fn touches(o: &Occurrence, d: NaiveDate) -> bool {
    if o.all_day {
        date_of(o.start) <= d && d <= date_of((o.end - 1).max(o.start))
    } else {
        layout::day_segment(o.start, o.end, d).is_some() && (o.end > at(d, 0) || o.start == o.end)
    }
}
