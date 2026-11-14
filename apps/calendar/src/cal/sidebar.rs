//! The sidebar: calendars with their colored checkboxes, the mini month and
//! the shared "Sync with Phone" row.

use calendar_core::dates::{add_months, month_grid};
use chrono::{Datelike, NaiveDate};
use ui::*;

use super::theme::Theme;
use super::{CalendarApp, SIDEBAR_W, TOOLBAR_H, icons, today};

impl CalendarApp {
    pub(super) fn sync_colors(&self, t: &Theme) -> sync_ui::Colors {
        sync_ui::Colors {
            accent: t.accent,
            on_accent: Color::WHITE,
            content: t.content,
            field: t.field,
            text_secondary: t.text_secondary,
            hover: t.hover,
            border: t.menu_border,
            shadow: t.shadow,
        }
    }

    pub(super) fn sidebar(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let mut header = div()
            .h(TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .justify_between()
            .pl(native::TRAFFIC_LIGHTS_X)
            .pr(10.0)
            .window_drag();
        if let Some(buttons) = ui::chrome::window_buttons(true, cx, t.text) {
            header = header.child(buttons);
        }
        let header = header.child(Self::icon_button("hide-sidebar", cx, t, icons::sidebar, |s, _| s.sidebar_open = false));

        let mut rows: Vec<AnyElement<Self>> = vec![native::sidebar_header("On My Computer", cx).pt(4.0).into()];
        let outlined = self.menu_calendar();
        for c in &self.store.calendars {
            rows.push(self.calendar_row(c.id, &c.name, &c.color, outlined == Some(c.id), cx, t).into());
        }

        div()
            .w(SIDEBAR_W)
            .h_full()
            .shrink_0()
            .flex_col()
            .bg(t.sidebar)
            .child(header)
            .child(
                div()
                    .id("sidebar-scroll")
                    .scroll_y()
                    .flex_1()
                    .flex_col()
                    .on_right_click(|s: &mut Self, cx: &mut Cx| s.sidebar_menu(None, cx))
                    .children(rows),
            )
            .child(self.mini_month(cx, t))
            .child(div().h(1.0).mx(16.0).mt(6.0).mb(6.0).shrink_0().bg(t.divider))
            .child(self.pairing.sidebar_row(&self.sync_colors(t)))
            .child(div().h(10.0).shrink_0())
    }

    /// A calendar: its colored checkbox (in the icon slot) and name. `outlined`
    /// while its context menu is open, like a macOS sidebar.
    fn calendar_row(&self, id: u64, name: &str, color: &str, outlined: bool, cx: &Cx, t: &Theme) -> Div<Self> {
        let c = t.cal(color);
        let shown = !self.hidden.contains(&id);
        let check = div()
            .id(("cal-check", id))
            .size(14.0)
            .rounded(4.0)
            .shrink_0()
            .items_center()
            .justify_center()
            .on_mouse_down(move |s: &mut Self, _: &mut Cx| s.toggle_calendar(id));
        let check = if shown {
            check.bg(c).child(canvas(|s, b| icons::check(s, b, Color::WHITE, 1.8)).size(11.0))
        } else {
            check.border(c, 1.5)
        };
        let check = div().size(native::SIDEBAR_ICON).shrink_0().items_center().justify_center().child(check);
        let name_el: AnyElement<Self> = if self.renaming == Some(id) {
            div()
                .flex_1()
                .min_w_0()
                .h(22.0)
                .px(4.0)
                .items_center()
                .rounded(4.0)
                .bg(t.content)
                .border(t.divider, 0.5)
                .focus_ring(cx.is_focused("rename"), cx)
                .child(input("rename", &self.fields.rename).flex_1())
                .into()
        } else {
            text(name.to_string()).truncate().flex_1().into()
        };
        let row = div()
            .id(("calendar", id))
            .h(native::SIDEBAR_ROW_H)
            .shrink_0()
            .mx(native::SIDEBAR_INSET)
            .px(native::SIDEBAR_ROW_PAD)
            .rounded(native::SIDEBAR_RADIUS)
            .flex_row()
            .items_center()
            .gap(8.0)
            .on_double_click(move |s: &mut Self, cx: &mut Cx| s.start_rename(id, cx))
            .on_right_click(move |s: &mut Self, cx: &mut Cx| s.sidebar_menu(Some(id), cx))
            .child(check)
            .child(name_el);
        if outlined { row.border(native::accent(cx), 2.0) } else { row }
    }

    /// The small month at the bottom of the sidebar.
    fn mini_month(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let m = self.mini_month;
        let today = today();
        let arrow = |id: &'static str, icon: fn(&mut Scene, Bounds, Color), dir: i32| {
            let c = t.icon;
            div()
                .id(id)
                .size(20.0)
                .rounded(5.0)
                .items_center()
                .justify_center()
                .hover_bg(Self::hover_fill(id, cx, t))
                .on_click(move |s: &mut Self, _: &mut Cx| s.mini_month = add_months(s.mini_month, dir))
                .child(canvas(move |s, b| icon(s, b, c)).size(11.0))
        };
        let header = div()
            .h(26.0)
            .flex_row()
            .items_center()
            .px(4.0)
            .child(text(m.format("%B %Y").to_string()).size(12.0).weight(Weight::Semibold).flex_1())
            .child(arrow("mini-prev", icons::chevron_left, -1))
            .child(arrow("mini-next", icons::chevron_right, 1));

        let days = month_grid(m.year(), m.month(), self.first, true);
        let cell_w = (SIDEBAR_W - 24.0) / 7.0;
        let mut weekdays = div().flex_row().h(18.0);
        for d in &days[..7] {
            let initial = d.format("%a").to_string().chars().next().unwrap_or(' ').to_string();
            weekdays = weekdays.child(div().w(cell_w).items_center().justify_center().child(text(initial).size(10.0).weight(Weight::Semibold).color(t.text_secondary)));
        }
        let (first, last) = super::layout::range(self.view, self.date, self.first);
        let mut grid = div().flex_col();
        for week in days.chunks(7) {
            let mut row = div().flex_row().h(22.0);
            for &d in week {
                row = row.child(self.mini_day(d, m, today, (first, last), cell_w, cx, t));
            }
            grid = grid.child(row);
        }
        div().flex_col().px(12.0).pt(8.0).shrink_0().child(header).child(weekdays).child(grid)
    }

    #[allow(clippy::too_many_arguments)]
    fn mini_day(&self, d: NaiveDate, month: NaiveDate, today: NaiveDate, shown: (NaiveDate, NaiveDate), w: f32, _cx: &Cx, t: &Theme) -> Div<Self> {
        let in_month = d.month() == month.month();
        let is_today = d == today;
        // The days the main view shows get a soft band, the selected day a pill.
        let in_view = matches!(self.view, super::View::Day | super::View::Week) && d >= shown.0 && d <= shown.1;
        let color = if is_today {
            Color::WHITE
        } else if in_month {
            t.text
        } else {
            t.text_tertiary
        };
        let mut dot = div().size(20.0).rounded(10.0).items_center().justify_center();
        if is_today {
            dot = dot.bg(t.red);
        } else if d == self.date {
            dot = dot.bg(t.field.alpha(2.0));
        }
        let weight = if is_today { Weight::Semibold } else { Weight::Regular };
        let mut cell = div()
            .id(("mini-day", d))
            .w(w)
            .h(22.0)
            .items_center()
            .justify_center()
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.go_to(d, cx))
            .child(dot.child(text(d.day().to_string()).size(11.0).weight(weight).tabular().color(color)));
        if in_view {
            cell = cell.bg(t.field);
        }
        cell
    }
}
