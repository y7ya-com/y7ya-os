//! The event popover: title, location, times, repeat, calendar and notes.

use std::cell::RefCell;
use std::rc::Rc;

use ui::widgets::popover;
use ui::*;

use calendar_core::model::local;

use super::layout::{DAY_MIN, View, minute_of};
use super::theme::Theme;
use super::{CalendarApp, icons};

const WIDTH: f32 = 300.0;
const LABEL_W: f32 = 52.0;

impl CalendarApp {
    pub(super) fn popover(&self, cx: &Cx, t: &Theme) -> Option<Div<Self>> {
        let ed = self.editor.as_ref()?;
        let ev = self.store.event(ed.event)?;
        let cal = self.store.calendar(ev.calendar);
        let sep = || div().h(1.0).bg(t.menu_border).mt(8.0).mb(8.0);
        let label = |s: &str| div().w(LABEL_W).flex_row().justify_end().pr(8.0).child(text(s.to_string()).size(12.0).color(t.text_secondary));

        let all_day = ev.all_day;
        let check = div()
            .id("ed-all-day")
            .size(14.0)
            .rounded(4.0)
            .items_center()
            .justify_center()
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.set_all_day(!all_day, cx));
        let check = if all_day {
            check.bg(t.accent).child(canvas(|s, b| icons::check(s, b, Color::WHITE, 1.8)).size(11.0))
        } else {
            check.border(t.text_tertiary, 1.0).bg(t.content)
        };

        let f = &self.fields;
        let times = |date_id: &'static str, date: &Rc<RefCell<TextInput>>, time_id: &'static str, time: &Rc<RefCell<TextInput>>| {
            let row = div().flex_row().gap(6.0).child(field(date_id, date, 88.0, t));
            if all_day { row } else { row.child(field(time_id, time, 76.0, t)) }
        };
        let repeat = ev.rule().map_or("None".to_string(), |r| r.describe());
        let ink = t.text_secondary;
        let popup = |id: &'static str, dot: Option<Color>, value: String, open: fn(&mut Self, &Cx)| {
            div()
                .id(id)
                .h(22.0)
                .pl(8.0)
                .pr(4.0)
                .rounded(5.0)
                .flex_row()
                .items_center()
                .gap(6.0)
                .bg(t.field)
                .hover_bg(t.icon_hover_bg)
                .on_mouse_down(move |s: &mut Self, cx: &mut Cx| open(s, cx))
                .children(dot.map(|c| div().size(10.0).rounded(5.0).bg(c)))
                .child(text(value).size(12.0))
                .child(canvas(move |s, b| icons::updown(s, b, ink)).w(12.0).h(16.0))
        };

        let body = div()
            .w(WIDTH)
            .flex_col()
            .p(14.0)
            .child(div().h(24.0).flex_row().items_center().child(input("ed-title", &f.title).flex_1()))
            .child(div().h(20.0).flex_row().items_center().child(input("ed-location", &f.location).flex_1()))
            .child(sep())
            .child(
                div()
                    .flex_col()
                    .gap(6.0)
                    .child(div().h(22.0).flex_row().items_center().child(label("all-day:")).child(check))
                    .child(div().flex_row().items_center().child(label("starts:")).child(times("ed-sdate", &f.start_date, "ed-stime", &f.start_time)))
                    .child(div().flex_row().items_center().child(label("ends:")).child(times("ed-edate", &f.end_date, "ed-etime", &f.end_time)))
                    .child(div().flex_row().items_center().child(label("repeat:")).child(popup("ed-repeat", None, repeat, |s, cx| s.repeat_menu(cx)))),
            )
            .child(sep())
            .child(div().flex_row().items_center().child(popup(
                "ed-calendar",
                cal.map(|c| t.cal(&c.color)),
                cal.map_or("Calendar".into(), |c| c.name.clone()),
                |s, cx| s.calendar_menu(cx),
            )))
            .child(sep())
            .child(
                div()
                    .id("ed-notes-area")
                    .min_h(40.0)
                    .flex_col()
                    .cursor(CursorIcon::Text)
                    .on_mouse_down(|s: &mut Self, cx: &mut Cx| {
                        s.fields.notes.borrow_mut().move_to_end();
                        cx.focus("ed-notes");
                    })
                    .child(input("ed-notes", &f.notes).w_full()),
            )
            .child(
                div().flex_row().justify_end().mt(6.0).child(
                    div()
                        .id("ed-delete")
                        .h(22.0)
                        .px(10.0)
                        .rounded(5.0)
                        .items_center()
                        .hover_bg(t.hover)
                        .on_click(|s: &mut Self, cx: &mut Cx| {
                            if let Some(id) = s.editor.as_ref().map(|e| e.event) {
                                s.delete_event(id, cx);
                            }
                        })
                        .child(text("Delete Event").size(12.0).color(t.red)),
                ),
            );

        // On the hour grid, point at the event's block wherever it's scrolled to.
        let mut anchor = ed.anchor;
        if matches!(self.view, View::Day | View::Week) && !ev.all_day && self.grid_bounds.get().w > 0.0 {
            let day = local(ed.start).date();
            if self.grid_days().contains(&day) {
                anchor = self.block_anchor(day, minute_of(ed.start, day).max(0), minute_of(ed.end, day).min(DAY_MIN));
            }
        }
        let panel = popover(cx, anchor, WIDTH, 330.0, t.menu_bg, t.menu_border, t.shadow, body);
        Some(div().inset_0().deferred().on_mouse_down(|s: &mut Self, cx: &mut Cx| s.close_editor(cx)).child(panel))
    }
}

/// A small text field (dates and times).
fn field(id: &'static str, state: &Rc<RefCell<TextInput>>, w: f32, t: &Theme) -> Div<CalendarApp> {
    div()
        .w(w)
        .h(22.0)
        .px(6.0)
        .rounded(5.0)
        .bg(t.field)
        .flex_row()
        .items_center()
        .cursor(CursorIcon::Text)
        .on_mouse_down(move |_: &mut CalendarApp, cx: &mut Cx| cx.focus(id))
        .child(input(id, state).flex_1())
}
