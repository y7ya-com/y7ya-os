//! y7ya-os's portal backend. Browsers, Electron, GTK and Qt apps ask the
//! desktop portal whether to be light or dark (`org.freedesktop.appearance`);
//! inside y7ya-os it asks us, and we answer with the desktop's own appearance
//! and accent, and tell them the moment either changes.
//!
//! D-Bus activated (`org.freedesktop.impl.portal.desktop.y7ya`); idle until
//! the settings file or `kdeglobals` changes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use settings::Settings;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const NAME: &str = "org.freedesktop.impl.portal.desktop.y7ya";
const PATH: &str = "/org/freedesktop/portal/desktop";
const APPEARANCE: &str = "org.freedesktop.appearance";

#[derive(Clone, Copy, PartialEq)]
struct Look {
    dark: bool,
    accent: u32,
}

impl Look {
    fn current() -> Self {
        let s = Settings::load_or_default();
        Look { dark: s.is_dark(), accent: s.appearance.accent_rgb() }
    }

    /// The `org.freedesktop.appearance` keys.
    fn values(self) -> Vec<(&'static str, OwnedValue)> {
        // 1 prefers dark, 2 prefers light.
        let scheme = if self.dark { 1u32 } else { 2 };
        let c = |shift: u32| ((self.accent >> shift) & 0xff) as f64 / 255.0;
        let accent = Value::from((c(16), c(8), c(0)));
        let owned = |v: Value| OwnedValue::try_from(v).expect("plain values");
        vec![("color-scheme", owned(scheme.into())), ("accent-color", owned(accent)), ("contrast", owned(0u32.into()))]
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotFound(String),
}

struct Portal {
    look: Arc<Mutex<Look>>,
}

/// Whether `namespace` is asked for: an empty list means everything, and a
/// trailing `.*` matches a prefix.
fn wanted(patterns: &[String], namespace: &str) -> bool {
    patterns.is_empty()
        || patterns.iter().any(|p| match p.strip_suffix('*') {
            Some(prefix) => namespace.starts_with(prefix),
            None => p == namespace,
        })
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl Portal {
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut all = HashMap::new();
        if wanted(&namespaces, APPEARANCE) {
            let look = *self.look.lock().unwrap();
            all.insert(APPEARANCE.to_string(), look.values().into_iter().map(|(k, v)| (k.to_string(), v)).collect());
        }
        all
    }

    fn read(&self, namespace: &str, key: &str) -> Result<OwnedValue, Error> {
        let look = *self.look.lock().unwrap();
        let found = (namespace == APPEARANCE).then(|| look.values().into_iter().find(|(k, _)| *k == key)).flatten();
        found.map(|(_, v)| v).ok_or_else(|| Error::NotFound(format!("{namespace}.{key} isn't set")))
    }

    #[zbus(signal)]
    async fn setting_changed(emitter: &SignalEmitter<'_>, namespace: &str, key: &str, value: Value<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }
}

fn main() -> zbus::Result<()> {
    let look = Arc::new(Mutex::new(Look::current()));
    let conn = zbus::blocking::connection::Builder::session()?
        .serve_at(PATH, Portal { look: look.clone() })?
        .name(NAME)?
        .build()?;
    let iface = conn.object_server().interface::<_, Portal>(PATH)?;

    // Both files can change the answer: our settings (Light/Dark/Auto, the
    // accent) and, in Auto, the system color scheme.
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watchers = Vec::new();
    let files = [Some(settings::path()), dirs::config_dir().map(|d| d.join("kdeglobals"))];
    for path in files.into_iter().flatten() {
        let tx = tx.clone();
        match settings::watch_file(&path, move || {
            let _ = tx.send(());
        }) {
            Ok(w) => watchers.push(w),
            Err(e) => eprintln!("y7-portal: not watching {}: {e}", path.display()),
        }
    }
    for () in rx {
        let now = Look::current();
        let before = std::mem::replace(&mut *look.lock().unwrap(), now);
        if now == before {
            continue;
        }
        for ((key, new), (_, old)) in now.values().into_iter().zip(before.values()) {
            if new != old {
                let value = Value::try_from(new).expect("plain values");
                zbus::block_on(Portal::setting_changed(iface.signal_emitter(), APPEARANCE, key, value))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_namespaces() {
        assert!(wanted(&[], APPEARANCE));
        assert!(wanted(&["org.freedesktop.*".into()], APPEARANCE));
        assert!(wanted(&[APPEARANCE.into()], APPEARANCE));
        assert!(!wanted(&["org.gnome.*".into()], APPEARANCE));
    }

    #[test]
    fn answers_in_portal_terms() {
        let dark = Look { dark: true, accent: 0xff0000 }.values();
        assert_eq!(dark[0].1, OwnedValue::from(1u32));
        assert_eq!(Look { dark: false, accent: 0 }.values()[0].1, OwnedValue::from(2u32));
        let rgb: (f64, f64, f64) = dark[1].1.clone().try_into().unwrap();
        assert_eq!(rgb, (1.0, 0.0, 0.0));
    }
}
