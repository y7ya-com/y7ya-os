//! The chat window (and its Spotlight-style quick panel): the conversation,
//! tool cards with confirm and undo, and the backend picker.

pub mod demo;
mod icons;
mod theme;
mod view;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;
use ui::widgets::{Menu, MenuKey, SPINNER_FRAME};
use ui::*;

use crate::backend::{self, Event, Kind, Status, Turn};
use crate::tools::{self, CallResult, Confirm, Ctx, Outcome};
use theme::Theme;

#[derive(Clone, Copy, PartialEq)]
pub enum Undo {
    None,
    Available,
    Working,
    Done,
    Failed,
}

pub enum CardState {
    Running,
    Done { outcome: Outcome, undo: Undo },
    Failed(String),
    /// Waiting for the user's go-ahead.
    Ask(Confirm),
    Confirming(Confirm),
    Cancelled(Confirm),
}

/// One tool call in the conversation.
pub struct Card {
    pub id: String,
    pub tool: String,
    pub args: Value,
    pub state: CardState,
}

pub enum Item {
    User(String),
    Reply(String),
    Tool(Card),
    Error(String),
    /// "New conversation with Codex".
    Divider(String),
    /// How to install or sign in to a backend.
    Setup(Kind),
}

enum Msg {
    /// From turn number `.0`.
    Backend(u64, Event),
    Status(Kind, Status),
    /// The user confirmed (or undid) a card and the call finished.
    Acted { card: String, undo: bool, result: CallResult },
}

/// Hands results from background threads to the UI thread.
#[derive(Default)]
struct Inbox {
    queue: Mutex<Vec<Msg>>,
    waker: Mutex<Option<Waker>>,
}

impl Inbox {
    fn push(&self, m: Msg) {
        self.queue.lock().unwrap().push(m);
        if let Some(w) = &*self.waker.lock().unwrap() {
            w.wake();
        }
    }

    fn take(&self) -> Vec<Msg> {
        std::mem::take(&mut *self.queue.lock().unwrap())
    }
}

pub const SUGGESTIONS: [(&str, &str); 8] = [
    ("Switch to dark mode", "set_appearance"),
    ("Set volume to 30%", "set_volume"),
    ("What Wi-Fi am I on?", "get_wifi"),
    ("Turn on Do Not Disturb", "set_do_not_disturb"),
    ("Make the text a bit bigger", "set_text_size"),
    ("Turn on Night Light", "set_night_light"),
    ("How much battery is left?", "get_power"),
    ("Connect my headphones", "connect_bluetooth_device"),
];

pub struct AssistantApp {
    /// The compact Spotlight-style panel (`--quick`).
    quick: bool,
    kind: Kind,
    status: [Status; 2],
    items: Vec<Item>,
    input: Rc<RefCell<TextInput>>,
    /// Session or thread id to resume.
    session: Option<String>,
    turn: Option<Turn>,
    /// Increments per turn so a stopped turn's late events are dropped.
    seq: u64,
    busy_since: Instant,
    inbox: Arc<Inbox>,
    /// The backend menu, while open.
    picker: Option<Menu<Kind>>,
    /// What the user did on cards since the last turn, told to the model next time.
    notes: Vec<String>,
    /// Remember the backend choice (off for snapshots).
    persist: bool,
    /// Where confirmed and undone calls run (tests swap in a harmless one).
    make_ctx: fn() -> Ctx,
}

impl AssistantApp {
    pub fn new(quick: bool, persist: bool, cx: &mut Cx) -> Self {
        let mut input = TextInput::new(&mut cx.text.fonts, TextStyle::default(), true);
        input.placeholder = if quick { "Ask Assistant to change a setting\u{2026}" } else { "Ask to change a setting\u{2026}" }.into();
        cx.window_radius = if quick { 12.0 } else { 10.0 };
        let inbox = Arc::new(Inbox::default());
        *inbox.waker.lock().unwrap() = Some(cx.waker());
        let mut app = Self {
            quick,
            kind: if persist { prefs::backend() } else { Kind::Claude },
            status: [Status::Checking; 2],
            items: Vec::new(),
            input: Rc::new(RefCell::new(input)),
            session: None,
            turn: None,
            seq: 0,
            busy_since: Instant::now(),
            inbox,
            picker: None,
            notes: Vec::new(),
            persist,
            make_ctx: Ctx::system,
        };
        if persist {
            app.check_backends();
        }
        cx.focus("input");
        app
    }

    fn theme(cx: &Cx) -> Theme {
        if cx.dark { Theme::dark() } else { Theme::light() }
    }

    pub fn status(&self, kind: Kind) -> Status {
        self.status[kind as usize]
    }

    pub fn set_status(&mut self, kind: Kind, s: Status) {
        self.status[kind as usize] = s;
    }

    fn check_backends(&mut self) {
        for kind in Kind::ALL {
            self.set_status(kind, Status::Checking);
            let inbox = self.inbox.clone();
            std::thread::spawn(move || inbox.push(Msg::Status(kind, backend::check(kind))));
        }
    }

    fn running(&self) -> bool {
        self.turn.is_some()
    }

    /// Whether anything on screen is spinning (and so needs frames).
    fn busy(&self) -> bool {
        self.running()
            || self.items.iter().any(|i| {
                matches!(i, Item::Tool(c) if matches!(c.state, CardState::Running | CardState::Confirming(_) | CardState::Done { undo: Undo::Working, .. }))
            })
    }

    // -- conversation ---------------------------------------------------------

    pub fn send(&mut self, text: &str, cx: &mut Cx) {
        let text = text.trim().to_string();
        if text.is_empty() || self.running() {
            return;
        }
        self.picker = None;
        self.input.borrow_mut().set_text(&mut cx.text.fonts, "");
        self.items.push(Item::User(text.clone()));
        cx.scroll_to_bottom("chat-scroll");
        if matches!(self.status(self.kind), Status::NotInstalled | Status::SignedOut) {
            self.items.push(Item::Setup(self.kind));
            return;
        }
        let mut prompt = String::new();
        for n in self.notes.drain(..) {
            prompt.push_str(&n);
            prompt.push('\n');
        }
        prompt.push_str(&text);

        self.seq += 1;
        let seq = self.seq;
        let inbox = self.inbox.clone();
        let sink: backend::Sink = Arc::new(move |e| inbox.push(Msg::Backend(seq, e)));
        self.turn = Some(backend::start(self.kind, self.session.clone(), prompt, sink));
        self.busy_since = Instant::now();
    }

    pub fn stop(&mut self) {
        if let Some(t) = self.turn.take() {
            t.stop();
        }
        self.seq += 1;
        self.finish_running("Stopped");
    }

    /// Cards still spinning when a turn ends didn't get a result.
    fn finish_running(&mut self, why: &str) {
        for item in &mut self.items {
            if let Item::Tool(c) = item {
                if matches!(c.state, CardState::Running) {
                    c.state = CardState::Failed(why.into());
                }
            }
        }
    }

    pub fn new_chat(&mut self, cx: &mut Cx) {
        self.stop();
        self.items.clear();
        self.session = None;
        self.notes.clear();
        self.picker = None;
        self.input.borrow_mut().set_text(&mut cx.text.fonts, "");
        cx.focus("input");
    }

    pub fn set_backend(&mut self, kind: Kind, cx: &mut Cx) {
        self.picker = None;
        if kind == self.kind {
            return;
        }
        self.stop();
        self.kind = kind;
        self.session = None;
        if self.persist {
            prefs::set_backend(kind);
        }
        if !self.items.is_empty() {
            self.items.push(Item::Divider(format!("New conversation with {}", kind.name())));
            cx.scroll_to_bottom("chat-scroll");
        }
    }

    /// Opens or closes the backend menu (placed under its button when drawn).
    pub fn toggle_picker(&mut self) {
        self.picker = match self.picker {
            Some(_) => None,
            None => Some(Menu::new(Point::default(), self.picker_items()).with_id("pick").right_aligned()),
        };
    }

    fn card_mut(&mut self, id: &str) -> Option<&mut Card> {
        self.items.iter_mut().rev().find_map(|i| match i {
            Item::Tool(c) if c.id == id => Some(c),
            _ => None,
        })
    }

    fn on_event(&mut self, e: Event) {
        match e {
            Event::Session(id) => self.session = Some(id),
            Event::Text(t) => match self.items.last_mut() {
                Some(Item::Reply(r)) => r.push_str(&t),
                _ => {
                    let t = t.trim_start();
                    if !t.is_empty() {
                        self.items.push(Item::Reply(t.to_string()));
                    }
                }
            },
            Event::ToolStart { id, name, args } => {
                if self.card_mut(&id).is_none() {
                    self.items.push(Item::Tool(Card { id, tool: name, args, state: CardState::Running }));
                }
            }
            Event::ToolEnd { id, result } => {
                if let Some(c) = self.card_mut(&id) {
                    c.state = card_state(result);
                }
            }
            Event::Error(e) => self.items.push(Item::Error(e)),
            Event::Done => {
                self.turn = None;
                self.finish_running("Didn\u{2019}t finish");
            }
        }
    }

    /// The user clicked the confirm (or cancel) button on a card.
    pub fn confirm(&mut self, card: &str, yes: bool) {
        let (inbox, make_ctx) = (self.inbox.clone(), self.make_ctx);
        let Some(c) = self.card_mut(card) else { return };
        let CardState::Ask(q) = &c.state else { return };
        let q = q.clone();
        if !yes {
            self.notes.push(format!("[The user cancelled \u{201c}{}\u{201d} on the card.]", q.title));
            if let Some(c) = self.card_mut(card) {
                c.state = CardState::Cancelled(q);
            }
            return;
        }
        let (tool, args, id) = (c.tool.clone(), c.args.clone(), c.id.clone());
        c.state = CardState::Confirming(q);
        self.busy_since = Instant::now();
        std::thread::spawn(move || {
            // The user's click is the confirmation.
            let result = tools::call(&make_ctx(), &tool, &args, true);
            inbox.push(Msg::Acted { card: id, undo: false, result });
        });
    }

    pub fn undo(&mut self, card: &str) {
        let (inbox, make_ctx) = (self.inbox.clone(), self.make_ctx);
        let Some(c) = self.card_mut(card) else { return };
        let CardState::Done { outcome, undo } = &mut c.state else { return };
        let (Undo::Available | Undo::Failed, Some(action)) = (*undo, outcome.undo.clone()) else { return };
        *undo = Undo::Working;
        let id = c.id.clone();
        self.busy_since = Instant::now();
        std::thread::spawn(move || {
            let result = tools::call(&make_ctx(), &action.tool, &action.args, true);
            inbox.push(Msg::Acted { card: id, undo: true, result });
        });
    }

    fn on_acted(&mut self, card: &str, undo: bool, result: CallResult) {
        let note;
        let Some(c) = self.card_mut(card) else { return };
        if undo {
            let CardState::Done { outcome, undo } = &mut c.state else { return };
            match result {
                CallResult::Done(o) => {
                    *undo = Undo::Done;
                    note = format!("[The user clicked Undo on \u{201c}{}\u{201d}: {}.]", outcome.summary, o.summary);
                }
                CallResult::Failed(e) => {
                    *undo = Undo::Failed;
                    note = format!("[The user clicked Undo on \u{201c}{}\u{201d}, but it failed: {e}]", outcome.summary);
                }
                CallResult::NeedsConfirmation(_) => return,
            }
        } else {
            note = match &result {
                CallResult::Done(o) => format!("[The user confirmed on the card: {}.]", o.summary),
                CallResult::Failed(e) => format!("[The user confirmed on the card, but it failed: {e}]"),
                CallResult::NeedsConfirmation(_) => return,
            };
            c.state = card_state(result);
        }
        self.notes.push(note);
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Backend(seq, e) if seq == self.seq => self.on_event(e),
            Msg::Backend(..) => {}
            Msg::Status(kind, s) => {
                self.set_status(kind, s);
                // Until the user picks one, use whichever backend works.
                let other = Kind::ALL.into_iter().find(|&k| k != self.kind).unwrap();
                let unusable = matches!(self.status(self.kind), Status::NotInstalled | Status::SignedOut);
                if self.persist && unusable && self.status(other) == Status::Ready && self.items.is_empty() && !prefs::chosen() {
                    self.kind = other;
                }
            }
            Msg::Acted { card, undo, result } => self.on_acted(&card, undo, result),
        }
    }
}

pub fn card_state(r: CallResult) -> CardState {
    match r {
        CallResult::Done(o) => {
            let undo = if o.undo.is_some() { Undo::Available } else { Undo::None };
            CardState::Done { outcome: o, undo }
        }
        CallResult::NeedsConfirmation(c) => CardState::Ask(c),
        CallResult::Failed(e) => CardState::Failed(e),
    }
}

impl App for AssistantApp {
    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Self::theme(cx).text }
    }

    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        self.render_root(cx)
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        if let Some(menu) = &mut self.picker {
            match menu.key(key, cx) {
                MenuKey::Pick(kind) => self.set_backend(kind, cx),
                MenuKey::Close => self.picker = None,
                MenuKey::Stay => {}
            }
            return true;
        }
        if key.is_named(NamedKey::Escape) {
            if self.running() {
                self.stop();
            } else if self.quick {
                cx.close_window();
            }
            return true;
        }
        if key.is_named(NamedKey::Enter) && !key.shift {
            let text = self.input.borrow().text();
            self.send(&text, cx);
            return true;
        }
        if key.ctrl && !key.alt {
            if key.is_char('n') {
                self.new_chat(cx);
                return true;
            }
            if key.is_char('q') || key.is_char('w') {
                cx.close_window();
                return true;
            }
        }
        if !cx.is_focused("input") {
            cx.focus("input");
        }
        false
    }

    fn next_wake(&self) -> Option<Instant> {
        self.busy().then(|| Instant::now() + SPINNER_FRAME)
    }

    fn on_external(&mut self, cx: &mut Cx) {
        let msgs = self.inbox.take();
        if msgs.is_empty() {
            return;
        }
        for m in msgs {
            self.handle(m);
        }
        cx.scroll_to_bottom("chat-scroll");
    }

    fn on_exit(&mut self) {
        if let Some(t) = self.turn.take() {
            t.stop();
        }
    }
}

/// The remembered backend choice, in `<data dir>/prefs.toml`.
mod prefs {
    use crate::backend::Kind;

    fn path() -> std::path::PathBuf {
        crate::data_dir().join("prefs.toml")
    }

    fn read() -> toml::Table {
        std::fs::read_to_string(path()).ok().and_then(|s| s.parse().ok()).unwrap_or_default()
    }

    /// Whether the user ever picked one.
    pub fn chosen() -> bool {
        read().contains_key("backend")
    }

    pub fn backend() -> Kind {
        read().get("backend").and_then(|v| v.as_str()).and_then(Kind::parse).unwrap_or(Kind::Claude)
    }

    pub fn set_backend(kind: Kind) {
        let mut t = read();
        t.insert("backend".into(), kind.key().into());
        let _ = std::fs::create_dir_all(crate::data_dir());
        let _ = std::fs::write(path(), toml::to_string(&t).unwrap_or_default());
    }
}

#[cfg(test)]
mod tests;
