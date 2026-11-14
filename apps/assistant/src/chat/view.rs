//! Rendering: the window's toolbar, conversation and composer, the quick
//! panel, cards, the backend picker and the setup card.

use ui::native::{self, TOOLBAR_H};
use ui::widgets::{MenuItem, spinner, spinner_phase};
use ui::*;

use super::icons;
use super::theme::Theme;
use super::*;
use crate::tools::{Category, Effect};

const COLUMN_W: f32 = 680.0;
const CARD_W: f32 = 520.0;
const QUICK_HEADER_H: f32 = 56.0;
/// The quick panel's side margins.
const QUICK_PAD: f32 = 16.0;
const WHITE: Color = Color::WHITE;

#[derive(Clone, Copy)]
enum ButtonStyle {
    Primary,
    Destructive,
    Plain,
}

/// A push button: filled ones darken while held, plain ones get a hover fill.
fn button<S: 'static>(
    id: impl std::hash::Hash + Clone,
    label: &str,
    style: ButtonStyle,
    cx: &Cx,
    t: &Theme,
    on_click: impl Fn(&mut S, &mut Cx) + 'static,
) -> Div<S> {
    let pressed = held(cx, id.clone());
    let (bg, hover, fg, weight) = match style {
        ButtonStyle::Primary => (t.accent, if pressed { darken(t.accent) } else { t.accent }, WHITE, Weight::Medium),
        ButtonStyle::Destructive => (t.destructive, if pressed { darken(t.destructive) } else { t.destructive }, WHITE, Weight::Medium),
        ButtonStyle::Plain => (t.field, if pressed { t.pressed } else { t.hover }, t.text, Weight::Regular),
    };
    div()
        .id(id)
        .h(24.0)
        .px(12.0)
        .rounded(6.0)
        .items_center()
        .justify_center()
        .shrink_0()
        .bg(bg)
        .hover_bg(hover)
        .on_click(on_click)
        .child(text(label.to_string()).weight(weight).color(fg))
}

/// A filled control held down.
fn darken(c: Color) -> Color {
    Color { r: c.r * 0.85, g: c.g * 0.85, b: c.b * 0.85, a: c.a }
}

/// Whether the pointer is holding this element down.
fn held(cx: &Cx, id: impl std::hash::Hash) -> bool {
    cx.hovered(id) && cx.mouse_down
}

/// A setting's colored tile with its white glyph.
fn tile<S>(c: Category, size: f32) -> Div<S> {
    let bg = icons::tile_color(c);
    div()
        .size(size)
        .shrink_0()
        .rounded(size * 0.24)
        .bg(bg)
        .child(canvas(move |s, b| icons::glyph(c, s, b.inset(b.w * 0.17), WHITE, bg)).size(size))
}

fn category(tool: &str) -> Category {
    tools::find(tool).map_or(Category::About, |t| t.category)
}

fn is_read(tool: &str) -> bool {
    tools::find(tool).is_none_or(|t| t.effect == Effect::Read)
}

fn doing(tool: &str) -> String {
    tools::find(tool).map_or_else(|| format!("Running {tool}"), |t| t.doing.to_string())
}

fn category_label(c: Category) -> &'static str {
    match c {
        Category::Appearance => "Appearance",
        Category::Wallpaper => "Wallpaper",
        Category::Display => "Displays",
        Category::NightLight => "Night Light",
        Category::Sound => "Sound",
        Category::Wifi => "Wi-Fi",
        Category::Bluetooth => "Bluetooth",
        Category::Power => "Battery",
        Category::Notifications => "Notifications",
        Category::Keyboard => "Keyboard",
        Category::Apps => "Default Apps",
        Category::DateTime => "Date & Time",
        Category::About => "About",
    }
}

impl AssistantApp {
    pub(super) fn render_root(&mut self, cx: &mut Cx) -> Div<Self> {
        let t = Self::theme(cx);
        {
            let size = if self.quick { 18.0 } else { 13.0 };
            let mut i = self.input.borrow_mut();
            i.set_styles(TextStyle { size, line_height: 1.3, weight: Weight::Regular, color: t.text }, None);
            i.placeholder_color = t.text_tertiary;
            i.caret_color = t.accent;
            i.selection_color = t.text_selection;
            i.inactive_selection_color = t.text_selection_inactive;
        }
        let (w, h) = cx.window_size;
        let mut root = div().w(w).h(h).flex_col().bg(if self.quick { t.panel } else { t.window }).text_color(t.text);
        root = if self.quick { self.quick_layout(root, cx, &t) } else { self.window_layout(root, cx, &t) };
        if self.picker.is_some() {
            root = root.child(self.picker_menu(cx));
        }
        if cx.transparent && !cx.maximized && !ui::is_y7ya() {
            root = root.child(div().inset_0().rounded(cx.window_radius).border(t.window_border, 1.0));
        }
        root
    }

    // -- window -----------------------------------------------------------------

    fn window_layout(&self, root: Div<Self>, cx: &Cx, t: &Theme) -> Div<Self> {
        let mut toolbar = div()
            .h(TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(8.0)
            .pl(native::TRAFFIC_LIGHTS_X)
            .pr(12.0)
            .border_b(t.separator)
            .window_drag();
        if let Some(b) = ui::chrome::window_buttons(true, cx, t.text) {
            toolbar = toolbar.child(b.mr(12.0));
        }
        let sparkle = native::toolbar_ink(cx, t.accent);
        toolbar = toolbar
            .child(canvas(move |s, b| icons::sparkle(s, b, sparkle)).size(16.0).shrink_0())
            .child(text("Assistant").weight(Weight::Bold).color(native::toolbar_ink(cx, t.text)).truncate())
            .child(div().flex_1())
            .child(self.picker_button(cx, t))
            .child(self.icon_button("new-chat", cx, t, icons::compose, |s, cx| s.new_chat(cx)))
            .children(ui::chrome::window_buttons(false, cx, t.text).map(|b| b.ml(6.0)));

        let body: AnyElement<Self> = if self.items.is_empty() {
            self.empty_state(cx, t).into()
        } else {
            div()
                .id("chat-scroll")
                .scroll_y()
                .flex_1()
                .flex_col()
                .items_center()
                .child(div().w_full().max_w(COLUMN_W).flex_col().gap(12.0).px(24.0).pt(20.0).pb(16.0).children(self.transcript(cx, t)))
                .into()
        };

        let composer = div()
            .shrink_0()
            .flex_row()
            .justify_center()
            .px(24.0)
            .pt(8.0)
            .pb(16.0)
            .child(
                div()
                    .id("composer")
                    .w_full()
                    .max_w(COLUMN_W - 48.0)
                    .h(36.0)
                    .rounded(18.0)
                    .bg(t.field)
                    .border(t.field_border, 1.0)
                    .focus_ring(cx.is_focused("input") && cx.window_focused, cx)
                    .flex_row()
                    .items_center()
                    .pl(16.0)
                    .pr(4.0)
                    .gap(8.0)
                    .cursor(CursorIcon::Text)
                    .on_mouse_down(|_: &mut Self, cx: &mut Cx| cx.focus("input"))
                    .child(input("input", &self.input).flex_1().min_w_0())
                    .child(self.send_button(28.0, cx, t)),
            );
        root.child(toolbar).child(body).child(composer)
    }

    fn icon_button(
        &self,
        id: &'static str,
        cx: &Cx,
        t: &Theme,
        icon: fn(&mut Scene, Bounds, Color),
        on_click: impl Fn(&mut Self, &mut Cx) + 'static,
    ) -> Div<Self> {
        let color = native::toolbar_ink(cx, t.icon);
        div()
            .id(id)
            .w(32.0)
            .h(28.0)
            .rounded(6.0)
            .items_center()
            .justify_center()
            .shrink_0()
            .hover_bg(if held(cx, id) { t.pressed } else { t.hover })
            .on_click(on_click)
            .child(canvas(move |s, b| icon(s, b, color)).size(20.0))
    }

    fn status_color(&self, kind: Kind, t: &Theme) -> Color {
        match self.status(kind) {
            Status::Ready => t.success,
            Status::Checking => t.text_tertiary,
            Status::NotInstalled | Status::SignedOut => t.warning,
        }
    }

    fn picker_button(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let chevron = native::toolbar_ink(cx, t.icon);
        let open = self.picker.is_some();
        div()
            .id("backend")
            .h(28.0)
            .pl(10.0)
            .pr(8.0)
            .rounded(6.0)
            .flex_row()
            .items_center()
            .gap(6.0)
            .shrink_0()
            .bg_opt(open.then_some(t.pressed))
            .hover_bg(if open || held(cx, "backend") { t.pressed } else { t.hover })
            .on_click(|s: &mut Self, _: &mut Cx| s.toggle_picker())
            .child(div().size(8.0).rounded(4.0).bg(self.status_color(self.kind, t)))
            .child(text(self.kind.name()).color(native::toolbar_ink(cx, t.text)))
            .child(canvas(move |s, b| icons::chevron_down(s, b, chevron)).size(12.0))
    }

    /// The backend menu's rows: each backend, checked if current, with its status.
    pub(super) fn picker_items(&self) -> Vec<MenuItem<Kind>> {
        Kind::ALL.into_iter().map(|k| MenuItem::new(k.product(), k).checked(k == self.kind).shortcut(self.status_label(k))).collect()
    }

    fn status_label(&self, kind: Kind) -> &'static str {
        match self.status(kind) {
            Status::Ready => "Ready",
            Status::Checking => "Checking\u{2026}",
            Status::NotInstalled => "Not Installed",
            Status::SignedOut => "Not Signed In",
        }
    }

    /// The backend menu, opening below its button with the right edges lined up.
    fn picker_menu(&mut self, cx: &mut Cx) -> Div<Self> {
        // Statuses may have changed since it opened.
        let items = self.picker_items();
        let (w, _) = cx.window_size;
        // The button's right edge: left of the send button in the quick panel,
        // of the new chat button in the window.
        let (right, top) = if self.quick { (w - QUICK_PAD - 26.0 - 12.0, 44.0) } else { (w - 12.0 - 32.0 - 8.0, TOOLBAR_H - 8.0) };
        let Some(menu) = &mut self.picker else { return div() };
        menu.items = items;
        menu.pos = pt(right, top);
        menu.render(cx, |s: &mut Self, kind, cx: &mut Cx| s.set_backend(kind, cx), |s: &mut Self, _| s.picker = None)
    }

    fn send_button(&self, size: f32, cx: &Cx, t: &Theme) -> Div<Self> {
        let running = self.running();
        let empty = self.input.borrow().is_empty();
        let (bg, glyph): (Color, fn(&mut Scene, Bounds, Color)) = if running {
            (t.text, icons::stop)
        } else if empty {
            (t.text_tertiary, icons::arrow_up)
        } else {
            (t.accent, icons::arrow_up)
        };
        let fg = if running { t.window } else { WHITE };
        let live = running || !empty;
        div()
            .id("send")
            .size(size)
            .rounded(size / 2.0)
            .shrink_0()
            .bg(bg)
            .hover_bg(if live && held(cx, "send") { darken(bg) } else { bg })
            .items_center()
            .justify_center()
            .on_click(|s: &mut Self, cx: &mut Cx| {
                if s.running() {
                    s.stop();
                } else {
                    let text = s.input.borrow().text();
                    s.send(&text, cx);
                }
            })
            .child(canvas(move |s, b| glyph(s, b, fg)).size(size * 0.62))
    }

    fn empty_state(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let accent = t.accent;
        let mut col = div()
            .flex_1()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(8.0)
            .px(32.0)
            .pb(24.0)
            .child(
                div()
                    .size(56.0)
                    .rounded(16.0)
                    .bg(accent.alpha(0.12))
                    .items_center()
                    .justify_center()
                    .mb(8.0)
                    .child(canvas(move |s, b| icons::sparkle(s, b, accent)).size(30.0)),
            )
            .child(text("What would you like to change?").size(22.0).weight(Weight::Bold))
            .child(text("Ask in your own words, and Assistant changes the setting for you.").color(t.text_secondary));
        match self.status(self.kind) {
            Status::NotInstalled | Status::SignedOut => col = col.child(self.setup_card(self.kind, cx, t).mt(16.0)),
            _ => {
                // Equal widths, so the suggestions read as one column.
                let mut chips = div().w(280.0).flex_col().gap(8.0).mt(16.0);
                for (i, (label, tool)) in SUGGESTIONS.iter().take(3).enumerate() {
                    chips = chips.child(
                        div()
                            .id(("suggest", i))
                            .h(36.0)
                            .w_full()
                            .pl(6.0)
                            .pr(16.0)
                            .rounded(18.0)
                            .flex_row()
                            .items_center()
                            .gap(10.0)
                            .bg(t.card)
                            .border(t.card_border, 1.0)
                            .hover_bg(if held(cx, ("suggest", i)) { t.pressed } else { t.hover })
                            .on_click(move |s: &mut Self, cx: &mut Cx| s.send(label, cx))
                            .child(tile(category(tool), 24.0).rounded(12.0))
                            .child(text(*label).truncate()),
                    );
                }
                col = col.child(chips);
            }
        }
        col
    }

    // -- quick panel ----------------------------------------------------------------

    fn quick_layout(&self, root: Div<Self>, cx: &Cx, t: &Theme) -> Div<Self> {
        let accent = t.accent;
        let open = self.picker.is_some();
        let badge = div()
            .id("backend")
            .h(22.0)
            .px(8.0)
            .rounded(11.0)
            .flex_row()
            .items_center()
            .gap(6.0)
            .shrink_0()
            .bg(if open { t.pressed } else { t.field })
            .hover_bg(if open || held(cx, "backend") { t.pressed } else { t.hover })
            .on_click(|s: &mut Self, _: &mut Cx| s.toggle_picker())
            .child(div().size(6.0).rounded(3.0).bg(self.status_color(self.kind, t)))
            .child(text(self.kind.name()).size(11.0).weight(Weight::Medium).color(t.text_secondary));
        let header = div()
            .h(QUICK_HEADER_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(12.0)
            .px(QUICK_PAD)
            .window_drag()
            .child(canvas(move |s, b| icons::sparkle(s, b, accent)).size(22.0).shrink_0())
            .child(
                div()
                    .id("quick-field")
                    .flex_1()
                    .min_w_0()
                    .h(32.0)
                    .items_center()
                    .clip()
                    .cursor(CursorIcon::Text)
                    .on_mouse_down(|_: &mut Self, cx: &mut Cx| cx.focus("input"))
                    .child(input("input", &self.input).flex_1().min_w_0()),
            )
            .child(badge)
            .child(self.send_button(26.0, cx, t));

        let body: AnyElement<Self> = if !self.items.is_empty() {
            div()
                .id("chat-scroll")
                .scroll_y()
                .flex_1()
                .flex_col()
                .gap(12.0)
                .px(QUICK_PAD)
                .pt(16.0)
                .pb(16.0)
                .children(self.transcript(cx, t))
                .into()
        } else if matches!(self.status(self.kind), Status::NotInstalled | Status::SignedOut) {
            div().flex_1().p(QUICK_PAD).items_start().justify_center().child(self.setup_card(self.kind, cx, t)).into()
        } else {
            let mut list = div()
                .flex_1()
                .flex_col()
                .p(8.0)
                .child(div().px(8.0).pt(4.0).pb(4.0).child(text("Suggestions").size(11.0).weight(Weight::Semibold).color(t.text_secondary)));
            for (i, (label, tool)) in SUGGESTIONS.iter().enumerate() {
                let lit = cx.hovered(("suggest", i));
                list = list.child(
                    div()
                        .id(("suggest", i))
                        .h(36.0)
                        .shrink_0()
                        .px(8.0)
                        .rounded(8.0)
                        .flex_row()
                        .items_center()
                        .gap(10.0)
                        .hover_bg(if held(cx, ("suggest", i)) { darken(t.accent) } else { t.accent })
                        .on_click(move |s: &mut Self, cx: &mut Cx| s.send(label, cx))
                        .child(tile(category(tool), 24.0))
                        .child(text(*label).color(if lit { WHITE } else { t.text }).truncate()),
                );
            }
            list.into()
        };
        root.child(header).child(div().h(1.0).shrink_0().bg(t.separator)).child(body)
    }

    // -- conversation ---------------------------------------------------------------

    fn transcript(&self, cx: &Cx, t: &Theme) -> Vec<AnyElement<Self>> {
        let phase = spinner_phase(self.busy_since);
        let mut out: Vec<AnyElement<Self>> = Vec::new();
        for item in &self.items {
            out.push(match item {
                Item::User(s) => div()
                    .w_full()
                    .flex_row()
                    .justify_end()
                    .pl(64.0)
                    .child(div().max_w(460.0).px(12.0).py(8.0).rounded(17.0).bg(t.accent).child(text(s.clone()).line_height(1.4).color(WHITE)))
                    .into(),
                Item::Reply(s) => text(s.clone()).line_height(1.45).w_full().into(),
                Item::Tool(c) => self.card(c, phase, cx, t).into(),
                Item::Error(e) => div()
                    .w_full()
                    .max_w(CARD_W)
                    .flex_row()
                    .items_start()
                    .gap(8.0)
                    .px(12.0)
                    .py(10.0)
                    .rounded(10.0)
                    .bg(t.destructive.alpha(0.09))
                    .child(status_icon(t.destructive, icons::exclamation))
                    .child(text(e.clone()).size(12.0).line_height(1.45).flex_1())
                    .into(),
                Item::Divider(s) => div()
                    .w_full()
                    .flex_row()
                    .items_center()
                    .gap(12.0)
                    .py(4.0)
                    .child(div().h(1.0).flex_1().bg(t.separator))
                    .child(text(s.clone()).size(11.0).color(t.text_tertiary))
                    .child(div().h(1.0).flex_1().bg(t.separator))
                    .into(),
                Item::Setup(kind) => self.setup_card(*kind, cx, t).into(),
            });
        }
        // Waiting on the model: nothing streaming and no card spinning.
        let waiting = self.running()
            && !matches!(self.items.last(), Some(Item::Reply(_)))
            && !matches!(self.items.last(), Some(Item::Tool(c)) if matches!(c.state, CardState::Running));
        if waiting {
            out.push(
                div()
                    .flex_row()
                    .items_center()
                    .gap(8.0)
                    .h(20.0)
                    .child(spinner(14.0, phase, t.text_secondary))
                    .child(text("Thinking\u{2026}").size(12.0).color(t.text_secondary))
                    .into(),
            );
        }
        out
    }

    fn card(&self, c: &Card, phase: f32, cx: &Cx, t: &Theme) -> Div<Self> {
        let cat = category(&c.tool);
        // Reads are quiet one-liners; changes get a full card.
        if is_read(&c.tool) {
            let (label, trailing): (String, Option<AnyElement<Self>>) = match &c.state {
                CardState::Running => (format!("{}\u{2026}", doing(&c.tool)), Some(spinner(12.0, phase, t.text_secondary).into())),
                CardState::Done { outcome, .. } => (outcome.summary.clone(), None),
                CardState::Failed(e) => (format!("Couldn\u{2019}t finish: {e}"), None),
                _ => (doing(&c.tool), None),
            };
            return div()
                .w_full()
                .flex_row()
                .items_center()
                .gap(8.0)
                .child(tile(cat, 18.0))
                .child(text(label).size(12.0).color(t.text_secondary).truncate())
                .children(trailing);
        }

        let id = c.id.clone();
        let base = div()
            .id(("card", c.id.clone()))
            .w_full()
            .max_w(CARD_W)
            .flex_row()
            .items_center()
            .gap(12.0)
            .px(12.0)
            .py(10.0)
            .rounded(10.0)
            .bg(t.card)
            .border(t.card_border, 1.0)
            .child(tile(cat, 28.0));
        let lines = |title: String, sub: String, sub_color: Color| {
            div()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(2.0)
                .child(text(title).weight(Weight::Medium).line_height(1.35).w_full())
                .child(text(sub).size(11.0).line_height(1.35).color(sub_color).w_full())
        };
        match &c.state {
            CardState::Running => base
                .child(lines(format!("{}\u{2026}", doing(&c.tool)), category_label(cat).into(), t.text_secondary))
                .child(spinner(16.0, phase, t.text_secondary)),
            CardState::Done { outcome, undo } => {
                let (sub, sub_color) = match undo {
                    Undo::Done => ("Undone".to_string(), t.text_secondary),
                    Undo::Failed => ("Couldn\u{2019}t undo".to_string(), t.destructive),
                    _ => (category_label(cat).to_string(), t.text_secondary),
                };
                let title_done = *undo == Undo::Done;
                let mut card = base.child(lines(outcome.summary.clone(), sub, sub_color));
                card = if title_done {
                    card.child(status_icon(t.text_tertiary, icons::check_bold))
                } else {
                    card.child(status_icon(t.success, icons::check_bold))
                };
                match undo {
                    Undo::Available | Undo::Failed => card.child(button(("undo", id.clone()), "Undo", ButtonStyle::Plain, cx, t, move |s: &mut Self, _| s.undo(&id))),
                    Undo::Working => card.child(spinner(16.0, phase, t.text_secondary)),
                    _ => card,
                }
            }
            CardState::Failed(e) => base
                .items_start()
                .child(lines(format!("Couldn\u{2019}t finish: {}", doing(&c.tool).to_lowercase()), e.clone(), t.text_secondary))
                .child(status_icon(t.destructive, icons::exclamation)),
            CardState::Ask(q) | CardState::Confirming(q) => {
                let confirming = matches!(c.state, CardState::Confirming(_));
                let (yes, no) = (id.clone(), id.clone());
                let style = if q.destructive { ButtonStyle::Destructive } else { ButtonStyle::Primary };
                let actions: AnyElement<Self> = if confirming {
                    div()
                        .flex_row()
                        .items_center()
                        .gap(8.0)
                        .h(24.0)
                        .child(spinner(14.0, phase, t.text_secondary))
                        .child(text("Working\u{2026}").size(12.0).color(t.text_secondary))
                        .into()
                } else {
                    div()
                        .flex_row()
                        .gap(8.0)
                        .child(button(("cancel", id.clone()), "Cancel", ButtonStyle::Plain, cx, t, move |s: &mut Self, _| s.confirm(&no, false)))
                        .child(button(("confirm", id.clone()), &q.button, style, cx, t, move |s: &mut Self, _| s.confirm(&yes, true)))
                        .into()
                };
                base.items_start().child(
                    div()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(2.0)
                        .child(text(q.title.clone()).weight(Weight::Semibold).line_height(1.35).w_full())
                        .child(text(q.detail.clone()).size(12.0).line_height(1.4).color(t.text_secondary).w_full())
                        .child(div().flex_row().justify_end().mt(8.0).child(actions)),
                )
            }
            CardState::Cancelled(q) => base.child(lines(q.title.clone(), "Cancelled \u{2014} nothing changed".into(), t.text_secondary)),
        }
    }

    fn setup_card(&self, kind: Kind, cx: &Cx, t: &Theme) -> Div<Self> {
        let installed = self.status(kind) != Status::NotInstalled;
        let title = if installed { format!("Sign in to {}", kind.product()) } else { format!("{} isn\u{2019}t installed", kind.product()) };
        let mut card = div()
            .w_full()
            .max_w(CARD_W)
            .flex_col()
            .gap(8.0)
            .p(16.0)
            .rounded(12.0)
            .bg(t.card)
            .border(t.card_border, 1.0)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(10.0)
                    .child(status_icon(t.warning, icons::exclamation))
                    .child(text(title).weight(Weight::Semibold).truncate()),
            )
            .child(
                text(format!(
                    "Assistant talks to {} through the {} app on this computer, signed in with your own account. Nothing is stored by Assistant.",
                    kind.name(),
                    kind.product()
                ))
                .size(12.0)
                .line_height(1.45)
                .color(t.text_secondary)
                .w_full(),
            );
        let mut steps: Vec<(&str, &str)> = Vec::new();
        if !installed {
            steps.push(("Install it in a terminal:", kind.install_command()));
        }
        steps.push(("Sign in:", kind.login_command()));
        for (label, cmd) in steps {
            card = card.child(text(label).size(12.0).weight(Weight::Medium).mt(4.0)).child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(8.0)
                    .h(32.0)
                    .pl(10.0)
                    .pr(4.0)
                    .rounded(7.0)
                    .bg(t.field)
                    .child(text(cmd).size(12.0).truncate().flex_1())
                    .child(button(("copy", cmd), "Copy", ButtonStyle::Plain, cx, t, move |_: &mut Self, cx: &mut Cx| {
                        if let Some(c) = cx.clipboard.as_mut() {
                            let _ = c.set_text(cmd);
                        }
                    })),
            );
        }
        let other = Kind::ALL.into_iter().find(|&k| k != kind).unwrap();
        let mut buttons = div().flex_row().justify_end().gap(8.0).mt(8.0);
        if self.status(other) == Status::Ready {
            buttons = buttons.child(button(("use", other.key()), &format!("Use {} Instead", other.name()), ButtonStyle::Plain, cx, t, move |s: &mut Self, cx: &mut Cx| s.set_backend(other, cx)));
        }
        buttons = buttons.child(button("check-again", "Check Again", ButtonStyle::Primary, cx, t, |s: &mut Self, _| s.check_backends()));
        card.child(buttons)
    }
}

fn status_icon<S>(color: Color, glyph: fn(&mut Scene, Bounds, Color)) -> Div<S> {
    div()
        .size(18.0)
        .rounded(9.0)
        .shrink_0()
        .bg(color)
        .child(canvas(move |s, b| glyph(s, b.inset(3.0), WHITE)).size(18.0))
}
