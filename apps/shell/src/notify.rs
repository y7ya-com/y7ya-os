//! Notifications: the `org.freedesktop.Notifications` DBus server and the
//! state behind banners and Notification Center.
//!
//! The server runs on zbus's own thread and only touches the shared `Store`;
//! the UI thread reads it, handles clicks, and sends back the signals
//! (`ActionInvoked`, `NotificationClosed`) through `Server::flush`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use zbus::zvariant::OwnedValue;

/// How long a banner stays up unless the app asks otherwise.
pub const BANNER_TIME: Duration = Duration::from_secs(5);
const MAX_BANNERS: usize = 3;
const MAX_HISTORY: usize = 100;

pub const CLOSED_EXPIRED: u32 = 1;
pub const CLOSED_DISMISSED: u32 = 2;
pub const CLOSED_BY_APP: u32 = 3;

#[derive(Clone, Debug)]
pub struct Notification {
    pub id: u32,
    pub app: String,
    pub title: String,
    pub body: String,
    /// (key, label), without the "default" action.
    pub actions: Vec<(String, String)>,
    /// Clicking the banner invokes the "default" action.
    pub has_default: bool,
    pub critical: bool,
    /// Gone once its banner is (doesn't stay in Notification Center).
    pub transient: bool,
    pub time: DateTime<Local>,
}

/// A `Notify` call, already decoded.
#[derive(Clone, Debug, Default)]
pub struct Incoming {
    pub app: String,
    pub replaces_id: u32,
    pub title: String,
    pub body: String,
    /// Flat key/label pairs as on the wire.
    pub actions: Vec<String>,
    /// 0 low, 1 normal, 2 critical.
    pub urgency: u8,
    pub transient: bool,
    /// Milliseconds; -1 for the default, 0 for "until dismissed".
    pub timeout_ms: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Banner {
    pub id: u32,
    /// `None` stays until dismissed (critical notifications, timeout 0).
    pub expires: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Signal {
    Closed(u32, u32),
    Invoked(u32, String),
}

#[derive(Default)]
pub struct Store {
    next_id: u32,
    /// Oldest first.
    pub history: Vec<Notification>,
    /// Newest first (top of the stack).
    pub banners: Vec<Banner>,
    pub dnd: bool,
    /// Signals for the DBus side to send.
    pub outbox: Vec<Signal>,
}

impl Store {
    pub fn notify(&mut self, n: Incoming, now: Instant) -> u32 {
        let existing = self.history.iter().position(|x| n.replaces_id != 0 && x.id == n.replaces_id);
        let id = match existing {
            Some(i) => self.history[i].id,
            None => {
                self.next_id += 1;
                self.next_id
            }
        };
        let mut actions: Vec<(String, String)> =
            n.actions.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].clone(), c[1].clone())).collect();
        let has_default = actions.iter().any(|(k, _)| k == "default");
        actions.retain(|(k, _)| k != "default");
        let note = Notification {
            id,
            app: if n.app.is_empty() { "Notification".into() } else { n.app },
            title: n.title,
            body: strip_markup(&n.body),
            actions,
            has_default,
            critical: n.urgency >= 2,
            transient: n.transient,
            time: Local::now(),
        };
        let critical = note.critical;
        match existing {
            Some(i) => {
                // An update replaces the content; it keeps its place if it's still up.
                self.history.remove(i);
                self.history.push(note);
            }
            None => {
                self.history.push(note);
                if self.history.len() > MAX_HISTORY {
                    self.history.remove(0);
                }
            }
        }
        if self.dnd && !critical {
            self.banners.retain(|b| b.id != id);
            return id;
        }
        let expires = match n.timeout_ms {
            _ if critical => None,
            0 => None,
            t if t > 0 => Some(now + Duration::from_millis(t as u64).max(Duration::from_secs(2))),
            _ => Some(now + BANNER_TIME),
        };
        match self.banners.iter_mut().find(|b| b.id == id) {
            Some(b) => b.expires = expires,
            None => {
                self.banners.insert(0, Banner { id, expires });
                while self.banners.len() > MAX_BANNERS {
                    let gone = self.banners.pop().unwrap();
                    self.banner_gone(gone.id, CLOSED_EXPIRED);
                }
            }
        }
        id
    }

    pub fn get(&self, id: u32) -> Option<&Notification> {
        self.history.iter().find(|n| n.id == id)
    }

    /// Removes a notification everywhere. False if it wasn't there.
    pub fn close(&mut self, id: u32, reason: u32) -> bool {
        let before = self.history.len();
        self.history.retain(|n| n.id != id);
        self.banners.retain(|b| b.id != id);
        let found = self.history.len() != before;
        if found {
            self.outbox.push(Signal::Closed(id, reason));
        }
        found
    }

    /// A banner went away; the notification stays in Notification Center
    /// unless it's transient.
    fn banner_gone(&mut self, id: u32, reason: u32) {
        self.banners.retain(|b| b.id != id);
        if self.get(id).is_some_and(|n| n.transient) {
            self.close(id, reason);
        }
    }

    pub fn dismiss_banner(&mut self, id: u32) {
        self.banner_gone(id, CLOSED_DISMISSED);
    }

    /// The user clicked the notification (`action` None) or one of its buttons.
    pub fn activate(&mut self, id: u32, action: Option<&str>) {
        let Some(n) = self.get(id) else { return };
        let key = match action {
            Some(k) => Some(k.to_string()),
            None => n.has_default.then(|| "default".to_string()),
        };
        if let Some(key) = key {
            self.outbox.push(Signal::Invoked(id, key));
        }
        self.close(id, CLOSED_DISMISSED);
    }

    /// Takes down banners whose time is up. `hovered` ones get a little longer.
    pub fn expire(&mut self, now: Instant, hovered: Option<u32>) {
        let due: Vec<u32> = self.banners.iter().filter(|b| b.expires.is_some_and(|t| t <= now)).map(|b| b.id).collect();
        for id in due {
            if Some(id) == hovered {
                if let Some(b) = self.banners.iter_mut().find(|b| b.id == id) {
                    b.expires = Some(now + Duration::from_secs(2));
                }
                continue;
            }
            self.banner_gone(id, CLOSED_EXPIRED);
        }
    }

    pub fn next_expiry(&self) -> Option<Instant> {
        self.banners.iter().filter_map(|b| b.expires).min()
    }

    pub fn clear_app(&mut self, app: &str) {
        let ids: Vec<u32> = self.history.iter().filter(|n| n.app == app).map(|n| n.id).collect();
        for id in ids {
            self.close(id, CLOSED_DISMISSED);
        }
    }

    pub fn clear_all(&mut self) {
        let ids: Vec<u32> = self.history.iter().map(|n| n.id).collect();
        for id in ids {
            self.close(id, CLOSED_DISMISSED);
        }
    }

    /// Notification Center's groups: apps with the newest notification first,
    /// each newest first.
    pub fn groups(&self) -> Vec<(String, Vec<&Notification>)> {
        let mut groups: Vec<(String, Vec<&Notification>)> = Vec::new();
        for n in self.history.iter().rev() {
            match groups.iter_mut().find(|(app, _)| app == &n.app) {
                Some((_, list)) => list.push(n),
                None => groups.push((n.app.clone(), vec![n])),
            }
        }
        groups
    }
}

/// Bodies may carry a little HTML; we show plain text.
pub fn strip_markup(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&")
}

// -- DBus -------------------------------------------------------------------------

struct Interface {
    store: Arc<Mutex<Store>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Interface {
    fn get_capabilities(&self) -> Vec<String> {
        ["actions", "body", "body-markup", "persistence"].map(String::from).to_vec()
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let urgency = hints.get("urgency").and_then(|v| u8::try_from(v).ok()).unwrap_or(1);
        let transient = hints.get("transient").and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
        let n = Incoming { app: app_name, replaces_id, title: summary, body, actions, urgency, transient, timeout_ms: expire_timeout };
        let id = self.store.lock().unwrap().notify(n, Instant::now());
        (self.wake)();
        id
    }

    fn close_notification(&self, id: u32) {
        self.store.lock().unwrap().close(id, CLOSED_BY_APP);
        (self.wake)();
    }

    #[zbus(out_args("name", "vendor", "version", "spec_version"))]
    fn get_server_information(&self) -> (String, String, String, String) {
        ("y7-shell".into(), "y7ya-os".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }
}

const PATH: &str = "/org/freedesktop/Notifications";
const NAME: &str = "org.freedesktop.Notifications";

/// The running server; sends signals queued in the store.
pub struct Server {
    conn: zbus::blocking::Connection,
}

impl Server {
    /// Serves notifications on the session bus. Fails if another notification
    /// daemon owns the name (e.g. running nested inside Plasma).
    pub fn start(store: Arc<Mutex<Store>>, wake: Arc<dyn Fn() + Send + Sync>) -> zbus::Result<Server> {
        let conn = zbus::blocking::connection::Builder::session()?.serve_at(PATH, Interface { store, wake })?.build()?;
        let reply = conn.request_name_with_flags(NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())?;
        if reply != zbus::fdo::RequestNameReply::PrimaryOwner && reply != zbus::fdo::RequestNameReply::AlreadyOwner {
            return Err(zbus::Error::NameTaken);
        }
        Ok(Server { conn })
    }

    pub fn flush(&self, store: &mut Store) {
        for signal in std::mem::take(&mut store.outbox) {
            let r = match signal {
                Signal::Closed(id, reason) => self.conn.emit_signal(None::<&str>, PATH, NAME, "NotificationClosed", &(id, reason)),
                Signal::Invoked(id, key) => self.conn.emit_signal(None::<&str>, PATH, NAME, "ActionInvoked", &(id, key)),
            };
            if let Err(e) = r {
                eprintln!("y7-shell: couldn't send notification signal: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incoming(app: &str, title: &str) -> Incoming {
        Incoming { app: app.into(), title: title.into(), timeout_ms: -1, urgency: 1, ..Default::default() }
    }

    #[test]
    fn banners_and_history() {
        let mut s = Store::default();
        let t0 = Instant::now();
        let a = s.notify(incoming("Mail", "Hello"), t0);
        let b = s.notify(incoming("Chat", "Hi"), t0);
        assert!(b > a);
        assert_eq!(s.banners.iter().map(|b| b.id).collect::<Vec<_>>(), [b, a]);
        assert_eq!(s.next_expiry(), Some(t0 + BANNER_TIME));

        // Banners expire into Notification Center; hovering buys time.
        s.expire(t0 + BANNER_TIME, Some(a));
        assert_eq!(s.banners.len(), 1);
        assert_eq!(s.banners[0].id, a);
        s.expire(t0 + BANNER_TIME + Duration::from_secs(3), None);
        assert!(s.banners.is_empty());
        assert_eq!(s.history.len(), 2);
        assert!(s.outbox.is_empty());

        // Replacing keeps the id and updates the content.
        let a2 = s.notify(Incoming { replaces_id: a, body: "<b>New</b> &amp; improved".into(), ..incoming("Mail", "Hello") }, t0);
        assert_eq!(a2, a);
        assert_eq!(s.history.len(), 2);
        assert_eq!(s.get(a).unwrap().body, "New & improved");

        // Groups: newest app first.
        s.notify(incoming("Chat", "Again"), t0);
        let groups: Vec<(String, usize)> = s.groups().into_iter().map(|(app, l)| (app, l.len())).collect();
        assert_eq!(groups, [("Chat".to_string(), 2), ("Mail".to_string(), 1)]);

        s.clear_app("Chat");
        assert_eq!(s.history.len(), 1);
        assert_eq!(s.outbox.len(), 2);
        assert!(s.outbox.iter().all(|sig| matches!(sig, Signal::Closed(_, CLOSED_DISMISSED))));
        s.clear_all();
        assert!(s.history.is_empty() && s.banners.is_empty());
    }

    #[test]
    fn dnd_critical_actions() {
        let mut s = Store { dnd: true, ..Default::default() };
        let t0 = Instant::now();
        let quiet = s.notify(incoming("Mail", "Shh"), t0);
        assert!(s.banners.is_empty());
        assert!(s.get(quiet).is_some());
        // Critical ones break through Do Not Disturb and stay up.
        let loud = s.notify(Incoming { urgency: 2, ..incoming("Battery", "Low") }, t0);
        assert_eq!(s.banners, [Banner { id: loud, expires: None }]);
        s.dnd = false;

        let id = s.notify(
            Incoming { actions: vec!["default".into(), "".into(), "reply".into(), "Reply".into()], ..incoming("Chat", "Q") },
            t0,
        );
        let n = s.get(id).unwrap();
        assert!(n.has_default);
        assert_eq!(n.actions, [("reply".to_string(), "Reply".to_string())]);
        s.activate(id, Some("reply"));
        assert_eq!(s.outbox, [Signal::Invoked(id, "reply".into()), Signal::Closed(id, CLOSED_DISMISSED)]);
        assert!(s.get(id).is_none());

        // Transient notifications vanish with their banner.
        s.outbox.clear();
        let t = s.notify(Incoming { transient: true, ..incoming("Volume", "50%") }, t0);
        s.expire(t0 + BANNER_TIME, None);
        assert!(s.get(t).is_none());
        assert_eq!(s.outbox, [Signal::Closed(t, CLOSED_EXPIRED)]);

        // Apps can close their own; unknown ids are ignored.
        assert!(s.close(quiet, CLOSED_BY_APP));
        assert!(!s.close(9999, CLOSED_BY_APP));
    }

    #[test]
    fn banner_stack_is_capped() {
        let mut s = Store::default();
        let t0 = Instant::now();
        let ids: Vec<u32> = (0..5).map(|i| s.notify(incoming("App", &i.to_string()), t0)).collect();
        assert_eq!(s.banners.iter().map(|b| b.id).collect::<Vec<_>>(), [ids[4], ids[3], ids[2]]);
        assert_eq!(s.history.len(), 5);
    }
}
