//! The arithmetic behind the views: which days a view shows, where a time
//! sits on the hour grid, how overlapping events share a column, and what a
//! drag turns an event into. Pure functions, so they're easy to test.

use calendar_core::dates::{add_months, month_start, week_days, week_start};
use calendar_core::model::{day_start, instant, local};
use chrono::{Datelike, Duration, NaiveDate, Timelike, Weekday};

/// Height of one hour in the day and week views.
pub const HOUR_H: f32 = 48.0;
/// Drags snap to this many minutes.
pub const SNAP: i32 = 15;
pub const DAY_MIN: i32 = 24 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Day,
    Week,
    Month,
    Year,
}

impl View {
    pub const ALL: [View; 4] = [View::Day, View::Week, View::Month, View::Year];

    pub fn label(self) -> &'static str {
        match self {
            View::Day => "Day",
            View::Week => "Week",
            View::Month => "Month",
            View::Year => "Year",
        }
    }

    pub fn index(self) -> usize {
        View::ALL.iter().position(|v| *v == self).unwrap()
    }
}

/// The days `view` shows around `date` (inclusive).
pub fn range(view: View, date: NaiveDate, first: Weekday) -> (NaiveDate, NaiveDate) {
    match view {
        View::Day => (date, date),
        View::Week => {
            let days = week_days(date, first);
            (days[0], days[6])
        }
        View::Month => {
            let start = month_start(date);
            let grid = calendar_core::dates::month_grid(start.year(), start.month(), first, false);
            (grid[0], *grid.last().unwrap())
        }
        View::Year => {
            let start = NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap();
            (week_start(start, first), NaiveDate::from_ymd_opt(date.year(), 12, 31).unwrap() + Duration::days(14))
        }
    }
}

/// The date the back (`-1`) and forward (`1`) buttons go to.
pub fn step(view: View, date: NaiveDate, dir: i32) -> NaiveDate {
    match view {
        View::Day => date + Duration::days(dir as i64),
        View::Week => date + Duration::weeks(dir as i64),
        View::Month => add_months(date, dir),
        View::Year => add_months(date, 12 * dir),
    }
}

/// Minutes from the top of the grid for a y offset into it.
pub fn y_to_minutes(y: f32) -> i32 {
    ((y / HOUR_H * 60.0).round() as i32).clamp(0, DAY_MIN)
}

pub fn minutes_to_y(min: f32) -> f32 {
    min / 60.0 * HOUR_H
}

/// Rounds to the nearest `step` minutes.
pub fn snap(min: i32, step: i32) -> i32 {
    ((min as f32 / step as f32).round() as i32 * step).clamp(0, DAY_MIN)
}

/// Rounds down to `step` minutes.
pub fn snap_down(min: i32, step: i32) -> i32 {
    (min.div_euclid(step) * step).clamp(0, DAY_MIN)
}

/// The instant `min` minutes into local day `day` (past 24:00 runs into the next day).
pub fn at(day: NaiveDate, min: i32) -> i64 {
    instant(day.and_hms_opt(0, 0, 0).unwrap() + Duration::minutes(min as i64))
}

/// Minutes into local day `day` of instant `ts` (negative before, > 1440 after).
pub fn minute_of(ts: i64, day: NaiveDate) -> i32 {
    let t = local(ts);
    (t.date() - day).num_days() as i32 * DAY_MIN + t.hour() as i32 * 60 + t.minute() as i32
}

/// The part of `[start, end)` on day `day`, in minutes, if any.
pub fn day_segment(start: i64, end: i64, day: NaiveDate) -> Option<(i32, i32)> {
    let (day0, day1) = (day_start(day), day_start(day + Duration::days(1)));
    if end <= day0 || start >= day1 || (start == end && start < day0) {
        return None;
    }
    let s = minute_of(start.max(day0), day).max(0);
    let e = if end >= day1 { DAY_MIN } else { minute_of(end, day) };
    Some((s, e.max(s)))
}

/// Side-by-side placement of overlapping events: for each `(start, end)`
/// (minutes), its column and how many columns its cluster of overlapping
/// events uses. Events are packed into the first free column in start order.
pub fn columns(items: &[(i32, i32)]) -> Vec<(usize, usize)> {
    // Very short events still take room on screen, so they overlap like 20 min.
    let span = |(s, e): (i32, i32)| (s, e.max(s + 20));
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| (items[i].0, -(items[i].1)));
    let mut out = vec![(0, 1); items.len()];
    let mut cluster: Vec<usize> = Vec::new();
    let mut ends: Vec<i32> = Vec::new(); // end of the last event in each column
    let mut cluster_end = i32::MIN;
    let finish = |cluster: &mut Vec<usize>, ends: &mut Vec<i32>, out: &mut Vec<(usize, usize)>| {
        for &i in cluster.iter() {
            out[i].1 = ends.len();
        }
        cluster.clear();
        ends.clear();
    };
    for i in order {
        let (s, e) = span(items[i]);
        if s >= cluster_end {
            finish(&mut cluster, &mut ends, &mut out);
            cluster_end = i32::MIN;
        }
        let col = match ends.iter().position(|&end| end <= s) {
            Some(c) => {
                ends[c] = e;
                c
            }
            None => {
                ends.push(e);
                ends.len() - 1
            }
        };
        out[i].0 = col;
        cluster.push(i);
        cluster_end = cluster_end.max(e);
    }
    finish(&mut cluster, &mut ends, &mut out);
    out
}

/// Rows for bars spanning columns `first..=last` (all-day events across a
/// week), so bars never overlap: the first row free over the whole span.
pub fn bar_rows(spans: &[(usize, usize)]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| (spans[i].0, std::cmp::Reverse(spans[i].1)));
    let mut rows: Vec<Vec<(usize, usize)>> = Vec::new();
    let mut out = vec![0; spans.len()];
    for i in order {
        let (a, b) = spans[i];
        let row = rows.iter().position(|r| r.iter().all(|&(c, d)| b < c || a > d)).unwrap_or_else(|| {
            rows.push(Vec::new());
            rows.len() - 1
        });
        rows[row].push((a, b));
        out[i] = row;
    }
    out
}

/// Where an event lands when dragged: it was grabbed `grab` minutes after
/// its start, and the pointer is now `min` minutes into `day`. Keeps the
/// duration; snaps the start.
pub fn moved(start: i64, end: i64, grab: i32, day: NaiveDate, min: i32) -> (i64, i64) {
    let new_start = at(day, snap(min - grab, SNAP).clamp(0, DAY_MIN - SNAP));
    (new_start, new_start + (end - start))
}

/// The new end when an event's bottom edge is dragged to `min` minutes into
/// `day`: snapped, and at least one snap step long.
pub fn resized(start: i64, day: NaiveDate, min: i32) -> i64 {
    at(day, snap(min, SNAP)).max(start + SNAP as i64 * 60)
}

/// The event a drag across empty grid creates, from `a` to `b` minutes into
/// `day` (either order): snapped outward, at least one snap step.
pub fn created(day: NaiveDate, a: i32, b: i32) -> (i64, i64) {
    let (lo, hi) = (a.min(b), a.max(b));
    let s = snap_down(lo, SNAP);
    let e = (snap_down(hi, SNAP) + SNAP).max(s + SNAP).min(DAY_MIN);
    (at(day, s), at(day, e))
}

/// Initial scroll of the hour grid: 7:30 AM, unless that would hide the
/// current time (shown with `visible` hours of grid on screen).
pub fn initial_scroll(now_min: i32, visible: f32) -> f32 {
    let top = 7.5_f32;
    let now = now_min as f32 / 60.0;
    let top = if now > top + visible - 1.0 { now - visible + 2.0 } else { top };
    minutes_to_y(top.clamp(0.0, (24.0 - visible).max(0.0)) * 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn ranges_and_steps() {
        let sat = d(2026, 11, 14);
        assert_eq!(range(View::Day, sat, Weekday::Sun), (sat, sat));
        assert_eq!(range(View::Week, sat, Weekday::Sun), (d(2026, 11, 8), sat));
        assert_eq!(range(View::Week, sat, Weekday::Mon), (d(2026, 11, 9), d(2026, 11, 15)));
        assert_eq!(range(View::Month, sat, Weekday::Sun), (d(2026, 11, 1), d(2026, 12, 5)));
        assert_eq!(step(View::Week, sat, -1), d(2026, 11, 7));
        assert_eq!(step(View::Month, d(2026, 1, 31), 1), d(2026, 2, 28));
        assert_eq!(step(View::Year, sat, 1), d(2027, 11, 14));
    }

    #[test]
    fn grid_positions_snap() {
        assert_eq!(y_to_minutes(HOUR_H * 9.5), 570);
        assert_eq!(minutes_to_y(570.0), HOUR_H * 9.5);
        assert_eq!(y_to_minutes(-20.0), 0);
        assert_eq!(snap(574, 15), 570);
        assert_eq!(snap(578, 15), 585);
        assert_eq!(snap(1439, 15), 1440);
        assert_eq!(snap_down(584, 15), 570);
    }

    #[test]
    fn segments_clip_to_the_day() {
        let day = d(2026, 11, 14);
        assert_eq!(day_segment(at(day, 540), at(day, 600), day), Some((540, 600)));
        // 11 PM to 1 AM shows on both days.
        let (s, e) = (at(day, 23 * 60), at(day, 25 * 60));
        assert_eq!(day_segment(s, e, day), Some((1380, 1440)));
        assert_eq!(day_segment(s, e, day + Duration::days(1)), Some((0, 60)));
        assert_eq!(day_segment(s, e, day - Duration::days(1)), None);
        // Ending exactly at midnight doesn't spill into the next day.
        assert_eq!(day_segment(at(day, 1380), at(day, 1440), day + Duration::days(1)), None);
    }

    #[test]
    fn overlapping_events_share_columns() {
        // 9-10, 9:30-11 overlap; 10:30-11 fits under the first; 13-14 is alone.
        let cols = columns(&[(540, 600), (570, 660), (630, 660), (780, 840)]);
        assert_eq!(cols, [(0, 2), (1, 2), (0, 2), (0, 1)]);
        // Three at once.
        assert_eq!(columns(&[(0, 60), (0, 60), (30, 90)]), [(0, 3), (1, 3), (2, 3)]);
        // Back to back don't overlap.
        assert_eq!(columns(&[(540, 600), (600, 660)]), [(0, 1), (0, 1)]);
    }

    #[test]
    fn all_day_bars_stack() {
        // A 3-day bar, a 1-day bar inside it, and one after.
        assert_eq!(bar_rows(&[(1, 3), (2, 2), (4, 5)]), [0, 1, 0]);
    }

    #[test]
    fn drags() {
        let day = d(2026, 11, 14);
        let (s, e) = (at(day, 540), at(day, 600));
        // Grabbed 20 min in, dropped with the pointer at 11:07 the next day.
        let next = day + Duration::days(1);
        assert_eq!(moved(s, e, 20, next, 667), (at(next, 645), at(next, 705)));
        // Can't be dragged above midnight.
        assert_eq!(moved(s, e, 30, day, 10).0, at(day, 0));
        assert_eq!(resized(s, day, 652), at(day, 645));
        assert_eq!(resized(s, day, 500), s + 15 * 60, "never shorter than one step");
        assert_eq!(created(day, 610, 548), (at(day, 540), at(day, 615)));
        assert_eq!(created(day, 600, 600), (at(day, 600), at(day, 615)));
    }

    #[test]
    fn initial_scroll_keeps_now_visible() {
        assert_eq!(initial_scroll(10 * 60, 12.0), minutes_to_y(450.0));
        assert_eq!(initial_scroll(21 * 60, 12.0), minutes_to_y(11.0 * 60.0));
        assert_eq!(initial_scroll(23 * 60 + 50, 12.0), minutes_to_y(12.0 * 60.0));
    }
}
