//! Notification banners (their own small surface at the top right) and
//! Notification Center (in the panel, opened from the clock).

use chrono::Local;
use ui::*;

use super::spotlight::letter_tile;
use super::theme::Theme;
use super::{Panel, Shell, icons};
use crate::notify::{CLOSED_DISMISSED, Notification};

const CARD_W: f32 = 356.0;
/// Room around banners for their shadow.
const MARGIN: f32 = 16.0;
const GAP: f32 = 8.0;
const ICON: f32 = 32.0;
const TEXT_W: f32 = CARD_W - 2.0 * 14.0 - ICON - 10.0;
const BODY: TextStyle = TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Color::BLACK };

fn when(n: &Notification) -> String {
    let mins = (Local::now() - n.time).num_minutes();
    match mins {
        ..1 => "now".into(),
        1..60 => format!("{mins}m ago"),
        60..1440 => format!("{}h ago", mins / 60),
        _ => n.time.format("%b %-d").to_string(),
    }
}

/// A label on a small pill, so it reads over any window or wallpaper.
fn chip<S: 'static>(label: impl Into<AnyElement<S>>, t: &Theme) -> Div<S> {
    div().h(22.0).px(10.0).rounded(11.0).items_center().bg(t.card).border(t.border, 0.5).child(label)
}

/// Long bodies are cut so a banner stays a banner.
fn short_body(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 180 { flat.chars().take(178).collect::<String>() + "\u{2026}" } else { flat }
}

impl Shell {
    fn card_height(cx: &mut Cx, n: &Notification) -> f32 {
        let body = short_body(&n.body);
        let body_h = if body.is_empty() { 0.0 } else { cx.text.measure(&body, &BODY, Some(TEXT_W), false).1 };
        let text_h = 16.0 + 17.0 + body_h;
        let actions = if n.actions.is_empty() { 0.0 } else { 34.0 };
        2.0 * 12.0 + text_h.max(ICON) + actions
    }

    /// The banner surface's size for the current stack, or `None` if empty.
    pub(super) fn banner_size(&self, cx: &mut Cx) -> Option<(u32, u32)> {
        let store = self.store.lock().unwrap();
        if store.banners.is_empty() {
            return None;
        }
        let h: f32 = store.banners.iter().filter_map(|b| store.get(b.id)).map(|n| Self::card_height(cx, n) + GAP).sum();
        Some(((CARD_W + 2.0 * MARGIN) as u32, (h - GAP + 2.0 * MARGIN).ceil() as u32 + 10))
    }

    pub(super) fn hovered_banner(&self, cx: &Cx) -> Option<u32> {
        let store = self.store.lock().unwrap();
        store.banners.iter().map(|b| b.id).find(|id| cx.hovered(("banner", *id)))
    }

    pub(super) fn banners(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let store = self.store.lock().unwrap();
        let shown: Vec<Notification> = store.banners.iter().filter_map(|b| store.get(b.id).cloned()).collect();
        drop(store);
        let mut col = div().flex_col().gap(GAP).p(MARGIN);
        for n in &shown {
            col = col.child(self.card(n, true, cx, t));
        }
        col
    }

    /// A notification as a banner or in Notification Center.
    fn card(&self, n: &Notification, banner: bool, cx: &Cx, t: &Theme) -> Div<Self> {
        let id = n.id;
        let key = if banner { ("banner", id) } else { ("nc-card", id) };
        let hovered = cx.hovered(key);
        let body = short_body(&n.body);
        let mut text_col = div()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(6.0)
                    .h(16.0)
                    .child(text(n.app.clone()).size(11.0).weight(Weight::Semibold).color(t.secondary).truncate())
                    .child(div().flex_1())
                    .child(text(when(n)).size(11.0).color(t.secondary)),
            )
            .child(text(n.title.clone()).weight(Weight::Semibold).truncate());
        if !body.is_empty() {
            text_col = text_col.child(text(body).w(TEXT_W).color(t.label));
        }
        if !n.actions.is_empty() {
            let mut row = div().flex_row().gap(6.0).mt(8.0);
            for (k, label) in n.actions.iter().take(3) {
                let k = k.clone();
                row = row.child(
                    div()
                        .id(("action", id, k.clone()))
                        .h(26.0)
                        .px(12.0)
                        .rounded(7.0)
                        .items_center()
                        .bg(t.fill)
                        .hover_bg(t.hover.alpha(2.0))
                        .on_click(move |s: &mut Self, _: &mut Cx| s.notification_clicked(id, Some(&k)))
                        .child(text(label.clone()).size(12.0).weight(Weight::Medium)),
                );
            }
            text_col = text_col.child(row);
        }
        let close_bg = t.card;
        let close = div()
            .id((key.0, "close", id))
            .absolute(-7.0, -7.0)
            .size(20.0)
            .rounded(10.0)
            .bg(close_bg)
            .border(t.border, 0.5)
            .shadow(t.shadow.alpha(0.5), 4.0, 1.0)
            .items_center()
            .justify_center()
            .on_click(move |s: &mut Self, _: &mut Cx| {
                let mut store = s.store.lock().unwrap();
                if banner {
                    store.dismiss_banner(id);
                } else {
                    store.close(id, CLOSED_DISMISSED);
                }
                drop(store);
                s.flush_signals();
            })
            .child({
                let ink = t.secondary;
                canvas(move |s, b| icons::xmark(s, b, ink, 6.0, 1.4)).size(12.0)
            });
        div()
            .id(key)
            .w(CARD_W)
            .flex_row()
            .items_start()
            .gap(10.0)
            .p(12.0)
            .px(14.0)
            .rounded(18.0)
            .bg(t.card)
            .border(t.border, 0.5)
            .shadow(t.shadow.alpha(if banner { 1.0 } else { 0.6 }), 18.0, 5.0)
            .cursor(CursorIcon::Default)
            .on_click(move |s: &mut Self, _: &mut Cx| s.notification_clicked(id, None))
            .child(letter_tile(&n.app, ICON))
            .child(text_col)
            .child_if(hovered || cx.hovered((key.0, "close", id)), || close.into())
    }

    fn notification_clicked(&mut self, id: u32, action: Option<&str>) {
        self.store.lock().unwrap().activate(id, action);
        self.flush_signals();
    }

    pub(super) fn notification_center(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let (w, h) = cx.window_size;
        let store = self.store.lock().unwrap();
        let groups: Vec<(String, Vec<Notification>)> =
            store.groups().into_iter().map(|(app, list)| (app, list.into_iter().cloned().collect())).collect();
        drop(store);

        let now = Local::now();
        let widget = div()
            .w(CARD_W)
            .flex_row()
            .items_center()
            .gap(14.0)
            .p(14.0)
            .rounded(18.0)
            .bg(t.card)
            .border(t.border, 0.5)
            .shadow(t.shadow.alpha(0.6), 18.0, 5.0)
            .child(
                div()
                    .flex_col()
                    .items_center()
                    .w(64.0)
                    .child(text(now.format("%A").to_string().to_uppercase()).size(10.0).weight(Weight::Bold).color(t.destructive))
                    .child(text(now.format("%-d").to_string()).size(34.0).weight(Weight::Regular).line_height(1.1)),
            )
            .child(
                div()
                    .flex_col()
                    .gap(2.0)
                    .child(text(now.format("%B %Y").to_string()).weight(Weight::Semibold))
                    .child(text("No events today").size(12.0).color(t.secondary)),
            );

        let mut col = div().flex_col().gap(GAP).w(CARD_W + 2.0 * MARGIN).px(MARGIN).pb(MARGIN).pt(6.0).child(widget);
        if groups.is_empty() {
            col = col.child(
                div()
                    .w(CARD_W)
                    .h(64.0)
                    .rounded(18.0)
                    .bg(t.card)
                    .border(t.border, 0.5)
                    .items_center()
                    .justify_center()
                    .child(text("No Notifications").color(t.secondary)),
            );
        } else {
            col = col.child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .mt(6.0)
                    .px(4.0)
                    .child(chip(text("Notifications").size(13.0).weight(Weight::Bold), t))
                    .child(self.pill("nc-clear-all", "Clear All", t, |s| {
                        s.store.lock().unwrap().clear_all();
                    })),
            );
        }
        for (app, list) in &groups {
            let a = app.clone();
            col = col.child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .mt(4.0)
                    .px(4.0)
                    .child(chip(text(app.clone()).size(12.0).weight(Weight::Semibold), t))
                    .child(self.pill(("nc-clear", app.clone()), "Clear", t, move |s| s.store.lock().unwrap().clear_app(&a))),
            );
            for n in list.iter().take(4) {
                col = col.child(self.card(n, false, cx, t));
            }
            if list.len() > 4 {
                col = col.child(div().flex_row().child(chip(text(format!("{} more", list.len() - 4)).size(11.0).color(t.secondary), t)));
            }
        }
        div()
            .id("nc-scroll")
            .absolute(w - CARD_W - 2.0 * MARGIN + 6.0, 0.0)
            .h(h)
            .scroll_y()
            .child(col.occlude())
    }

    fn pill(&self, id: impl std::hash::Hash, label: &'static str, t: &Theme, f: impl Fn(&mut Self) + 'static) -> Div<Self> {
        div()
            .id(id)
            .h(22.0)
            .px(10.0)
            .rounded(11.0)
            .items_center()
            .bg(t.card)
            .hover_bg(t.menu)
            .border(t.border, 0.5)
            .on_click(move |s: &mut Self, cx: &mut Cx| {
                f(s);
                s.flush_signals();
                if s.store.lock().unwrap().history.is_empty() && s.panel == Panel::NotificationCenter {
                    cx.request_redraw();
                }
            })
            .child(text(label).size(11.5).weight(Weight::Medium))
    }
}
