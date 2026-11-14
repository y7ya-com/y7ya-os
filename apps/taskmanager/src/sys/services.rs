//! systemd services, read from `systemctl list-units`. Only user units can be
//! started and stopped from here (system units need privileges).

use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    User,
    System,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Service {
    pub unit: String,
    pub load: String,
    /// active, inactive, failed, activating…
    pub active: String,
    /// running, exited, dead…
    pub sub: String,
    pub description: String,
    pub scope: Scope,
}

impl Service {
    pub fn running(&self) -> bool {
        self.active == "active" || self.active == "activating" || self.active == "reloading"
    }

    pub fn status(&self) -> String {
        match (self.active.as_str(), self.sub.as_str()) {
            ("active", "running") => "Running".into(),
            ("active", "exited") => "Finished".into(),
            ("active", _) => "Active".into(),
            ("failed", _) => "Failed".into(),
            ("activating", _) => "Starting".into(),
            ("deactivating", _) => "Stopping".into(),
            _ => "Stopped".into(),
        }
    }

    /// The unit name without `.service`, with systemd's `\x2d` escapes undone.
    pub fn name(&self) -> String {
        self.unit.trim_end_matches(".service").replace("\\x2d", "-")
    }
}

/// Parses `systemctl list-units --type=service --all --plain --no-legend` output.
pub fn parse_list_units(out: &str, scope: Scope) -> Vec<Service> {
    out.lines()
        .filter_map(|line| {
            // Failed or not-found units carry a "●" marker in front.
            let line = line.trim_start().trim_start_matches('●').trim_start();
            let mut f = line.split_whitespace();
            let unit = f.next()?.to_string();
            let (load, active, sub) = (f.next()?, f.next()?, f.next()?);
            let description = f.collect::<Vec<_>>().join(" ");
            unit.ends_with(".service").then(|| Service {
                unit,
                load: load.into(),
                active: active.into(),
                sub: sub.into(),
                description,
                scope,
            })
        })
        .collect()
}

fn systemctl(scope: Scope) -> Command {
    let mut c = Command::new("systemctl");
    if scope == Scope::User {
        c.arg("--user");
    }
    c
}

/// Both scopes. Slow-ish (spawns systemctl), so call it off the UI thread.
pub fn list() -> Vec<Service> {
    let mut all = Vec::new();
    for scope in [Scope::User, Scope::System] {
        let out = systemctl(scope)
            .args(["list-units", "--type=service", "--all", "--plain", "--no-legend", "--no-pager"])
            .env("SYSTEMD_COLORS", "0")
            .output();
        if let Ok(o) = out {
            all.extend(parse_list_units(&String::from_utf8_lossy(&o.stdout), scope));
        }
    }
    all
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Start,
    Stop,
    Restart,
}

/// Runs the action on a user unit and waits for systemctl to finish.
pub fn control(unit: &str, action: Action) -> Result<(), String> {
    let verb = match action {
        Action::Start => "start",
        Action::Stop => "stop",
        Action::Restart => "restart",
    };
    let out = systemctl(Scope::User).args([verb, "--no-block", unit]).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_list_units() {
        let out = "accounts-daemon.service                                loaded    active   running Accounts Service\n\
                   ● broken.service                                       loaded    failed   failed  Something Broke\n\
                   alsa-state.service                                     loaded    inactive dead    Manage Sound Card State (restore and store)\n\
                   app-com.github.wwmm.easyeffects\\x2dservice@autostart.service loaded active running Easy Effects\n\
                   foo.socket loaded active listening Not a service\n";
        let s = parse_list_units(out, Scope::System);
        assert_eq!(s.len(), 4);
        assert_eq!(s[0].unit, "accounts-daemon.service");
        assert_eq!(s[0].status(), "Running");
        assert_eq!(s[1].unit, "broken.service");
        assert_eq!(s[1].status(), "Failed");
        assert_eq!(s[2].description, "Manage Sound Card State (restore and store)");
        assert_eq!(s[2].status(), "Stopped");
        assert!(!s[2].running());
        assert_eq!(s[3].name(), "app-com.github.wwmm.easyeffects-service@autostart");
    }
}
