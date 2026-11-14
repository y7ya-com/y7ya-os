//! The clock's flyout: notifications floating over a month calendar card.

use chrono::{Datelike, Local, Months, NaiveDate};
use ui::*;

use super::notifications::{CARD_W, LIST_PAD};
use super::theme::Theme;
use super::{MARGIN, Shell, icons};

const CELL_W: f32 = 44.0;
const CELL_H: f32 = 36.0;
const CELL_GAP: f32 = 2.0;
const CARD_H: f32 = 16.0 + 22.0 + 12.0 + 28.0 + 24.0 + 6.0 * CELL_H + 5.0 * CELL_GAP + 14.0;

/// The six weeks shown for `first`'s month, Sunday first: each day and
/// whether it's in the month.
pub fn month_grid(first: NaiveDate) -> Vec<[(NaiveDate, bool); 7]> {
    let start = first - chrono::Duration::days(first.weekday().num_days_from_sunday() as i64);
    (0..6)
        .map(|week| {
            std::array::from_fn(|d| {
                let day = start + chrono::Duration::days(week * 7 + d as i64);
                (day, day.month() == first.month())
            })
        })
        .collect()
}

impl Shell {
    pub(super) fn calendar_flyout(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let (_, h) = cx.window_size;
        let room = h - CARD_H - 2.0 * MARGIN - 8.0;
        // The list pads itself for the cards' shadows; line the cards up
        // with the calendar below.
        let list = self.notification_list(room, cx, t);
        div().flex_col().items_end().gap(0.0).children(list.map(|l| l.mr(-LIST_PAD).mb(8.0 - LIST_PAD))).child(self.month_card(cx, t))
    }

    fn month_card(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let today = Local::now().date_naive();
        let first = today.with_day(1).unwrap();
        let shown = if self.month_offset >= 0 {
            first.checked_add_months(Months::new(self.month_offset as u32))
        } else {
            first.checked_sub_months(Months::new(self.month_offset.unsigned_abs()))
        }
        .unwrap_or(first);

        let ink = t.label;
        let nav = |id: &'static str, right: bool| {
            div()
                .id(id)
                .size(28.0)
                .rounded(7.0)
                .items_center()
                .justify_center()
                .hover_bg(t.hover.alpha(1.6))
                .on_click(move |s: &mut Self, cx: &mut Cx| {
                    s.month_offset += if right { 1 } else { -1 };
                    cx.request_redraw();
                })
                .child(canvas(move |s, b| icons::chevron(s, b, ink, right)).size(16.0))
        };
        let month_row = div()
            .h(28.0)
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .id("cal-month")
                    .flex_row()
                    .items_center()
                    .px(4.0)
                    .on_click(|s: &mut Self, cx: &mut Cx| {
                        s.month_offset = 0;
                        cx.request_redraw();
                    })
                    .child(text(shown.format("%B %Y").to_string()).size(14.0).weight(Weight::Semibold)),
            )
            .child(div().flex_row().gap(4.0).child(nav("cal-prev", false)).child(nav("cal-next", true)));

        let mut weekdays = div().flex_row().gap(CELL_GAP).h(24.0).items_center();
        for d in ["S", "M", "T", "W", "T", "F", "S"] {
            weekdays = weekdays.child(div().w(CELL_W).justify_center().child(text(d).size(11.0).weight(Weight::Medium).color(t.secondary)));
        }
        let mut grid = div().flex_col().gap(CELL_GAP);
        for week in month_grid(shown) {
            let mut row = div().flex_row().gap(CELL_GAP);
            for (day, in_month) in week {
                let is_today = day == today;
                let color = if is_today {
                    Color::WHITE
                } else if in_month {
                    t.label
                } else {
                    t.tertiary
                };
                let mut number = div()
                    .size(30.0)
                    .rounded(15.0)
                    .items_center()
                    .justify_center()
                    .child(text(day.day().to_string()).size(12.5).weight(if is_today { Weight::Semibold } else { Weight::Regular }).color(color));
                if is_today {
                    number = number.bg(t.accent);
                }
                row = row.child(div().w(CELL_W).h(CELL_H).items_center().justify_center().child(number));
            }
            grid = grid.child(row);
        }

        let _ = cx;
        div()
            .w(CARD_W)
            .h(CARD_H)
            .flex_col()
            .px(16.0)
            .pt(16.0)
            .rounded(14.0)
            .bg(t.panel)
            .border(t.border, 0.5)
            .shadow(t.shadow, 30.0, 10.0)
            .occlude()
            .child(text(Local::now().format("%A, %B %-d").to_string()).size(15.0).weight(Weight::Bold).h(22.0))
            .child(div().h(1.0 / cx.scale.max(1.0)).bg(t.separator).mt(12.0).mb(0.0))
            .child(month_row.mt(6.0))
            .child(weekdays)
            .child(grid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_weeks_from_sunday() {
        // November 2026 starts on a Sunday.
        let grid = month_grid(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
        assert_eq!(grid.len(), 6);
        assert_eq!(grid[0][0], (NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(), true));
        assert_eq!(grid[4][1], (NaiveDate::from_ymd_opt(2026, 11, 30).unwrap(), true));
        assert!(!grid[4][2].1);
        // May 2026 starts on a Friday: the grid opens with April's last days.
        let grid = month_grid(NaiveDate::from_ymd_opt(2026, 5, 1).unwrap());
        assert_eq!(grid[0][0], (NaiveDate::from_ymd_opt(2026, 4, 26).unwrap(), false));
        assert_eq!(grid[0][5], (NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(), true));
        assert_eq!(grid[5][6], (NaiveDate::from_ymd_opt(2026, 6, 6).unwrap(), false));
    }
}
