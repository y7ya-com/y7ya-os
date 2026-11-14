//! Default apps through `xdg-mime`, with installed apps found from their
//! `.desktop` files.

use std::collections::HashSet;

use serde_json::{Value, json};

use super::*;

/// A kind of default app and the MIME types it covers (the first is the one we read back).
struct Kind {
    key: &'static str,
    label: &'static str,
    mimes: &'static [&'static str],
}

const KINDS: &[Kind] = &[
    Kind { key: "web_browser", label: "web browser", mimes: &["x-scheme-handler/https", "x-scheme-handler/http", "text/html"] },
    Kind { key: "email", label: "email app", mimes: &["x-scheme-handler/mailto"] },
    Kind { key: "file_manager", label: "file manager", mimes: &["inode/directory"] },
    Kind { key: "text_editor", label: "text editor", mimes: &["text/plain"] },
    Kind { key: "image_viewer", label: "image viewer", mimes: &["image/png", "image/jpeg", "image/gif", "image/webp"] },
    Kind { key: "pdf_viewer", label: "PDF viewer", mimes: &["application/pdf"] },
    Kind { key: "music_player", label: "music player", mimes: &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav"] },
    Kind { key: "video_player", label: "video player", mimes: &["video/mp4", "video/x-matroska", "video/webm"] },
];

fn kind_schema() -> Value {
    json!({ "type": "string", "enum": KINDS.iter().map(|k| k.key).collect::<Vec<_>>() })
}

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "get_default_apps",
        category: Category::Apps,
        doing: "Checking default apps",
        description: "Read the default web browser, email app, file manager, text editor, image viewer, PDF viewer, music and video player.",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_default_apps,
    },
    Tool {
        name: "list_apps_for",
        category: Category::Apps,
        doing: "Looking for apps",
        description: "List installed apps that can be the default for a kind (e.g. every installed web browser).",
        effect: Effect::Read,
        schema: || object(&[("kind", kind_schema())], &["kind"]),
        confirm: never,
        run: list_apps_for,
    },
    Tool {
        name: "set_default_app",
        category: Category::Apps,
        doing: "Changing default app",
        description: "Make an installed app the default for a kind. `app` is its name (e.g. \"Firefox\") or desktop id (e.g. \"firefox.desktop\").",
        effect: Effect::Change,
        schema: || object(&[("kind", kind_schema()), ("app", json!({ "type": "string" }))], &["kind", "app"]),
        confirm: never,
        run: set_default_app,
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopApp {
    pub id: String,
    pub name: String,
    pub mimes: Vec<String>,
}

/// Name, MIME types and visibility from a `.desktop` file's main group.
pub fn parse_desktop_file(id: &str, s: &str) -> Option<DesktopApp> {
    let mut in_main = false;
    let mut name = None;
    let mut mimes = Vec::new();
    for line in s.lines().map(str::trim) {
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "Name" => name = Some(v.trim().to_string()),
            "MimeType" => mimes = v.split(';').filter(|m| !m.is_empty()).map(str::to_string).collect(),
            "NoDisplay" | "Hidden" if v.trim() == "true" => return None,
            "Type" if v.trim() != "Application" => return None,
            _ => {}
        }
    }
    Some(DesktopApp { id: id.to_string(), name: name?, mimes })
}

fn installed(ctx: &Ctx) -> Vec<DesktopApp> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in &ctx.app_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let id = e.file_name().to_string_lossy().into_owned();
            if !id.ends_with(".desktop") || !seen.insert(id.clone()) {
                continue;
            }
            if let Some(app) = std::fs::read_to_string(e.path()).ok().and_then(|s| parse_desktop_file(&id, &s)) {
                apps.push(app);
            }
        }
    }
    apps
}

fn kind(key: &str) -> Result<&'static Kind, String> {
    KINDS.iter().find(|k| k.key == key).ok_or_else(|| {
        let keys: Vec<&str> = KINDS.iter().map(|k| k.key).collect();
        format!("\u{201c}kind\u{201d} must be one of: {}.", keys.join(", "))
    })
}

fn current(ctx: &Ctx, k: &Kind) -> Result<Option<String>, String> {
    let id = ctx.cmd("xdg-mime", &["query", "default", k.mimes[0]])?;
    Ok(Some(id.trim().to_string()).filter(|s| !s.is_empty()))
}

fn display_name(apps: &[DesktopApp], id: &str) -> String {
    apps.iter().find(|a| a.id == id).map_or_else(|| id.trim_end_matches(".desktop").to_string(), |a| a.name.clone())
}

fn get_default_apps(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    let apps = installed(ctx);
    let mut out = serde_json::Map::new();
    for k in KINDS {
        let v = current(ctx, k)?.map(|id| json!({ "id": id, "name": display_name(&apps, &id) }));
        out.insert(k.key.into(), v.unwrap_or(Value::Null));
    }
    Ok(Outcome::new("Checked default apps", Value::Object(out)))
}

fn candidates(apps: &[DesktopApp], k: &Kind) -> Vec<DesktopApp> {
    apps.iter().filter(|a| a.mimes.iter().any(|m| m == k.mimes[0])).cloned().collect()
}

fn list_apps_for(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let k = kind(&str_arg(args, "kind")?)?;
    let apps = candidates(&installed(ctx), k);
    let default = current(ctx, k)?;
    let list: Vec<Value> = apps.iter().map(|a| json!({ "id": a.id, "name": a.name, "default": Some(&a.id) == default.as_ref() })).collect();
    Ok(Outcome::new(format!("Found {} {}s", list.len(), k.label), json!({ "kind": k.key, "apps": list })))
}

fn set_default_app(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let k = kind(&str_arg(args, "kind")?)?;
    let want = str_arg(args, "app")?;
    let all = installed(ctx);
    let apps = candidates(&all, k);
    let q = want.to_lowercase();
    let id = format!("{}.desktop", q.trim_end_matches(".desktop"));
    // An exact desktop id is trusted even if the app doesn't list the type (undo relies on it).
    let app = all
        .iter()
        .find(|a| a.id.to_lowercase() == id)
        .or_else(|| apps.iter().find(|a| a.name.to_lowercase() == q))
        .or_else(|| apps.iter().find(|a| a.name.to_lowercase().contains(&q)))
        .ok_or_else(|| {
            let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
            format!("No installed {} matches \u{201c}{want}\u{201d}. Installed: {}.", k.label, if names.is_empty() { "none".into() } else { names.join(", ") })
        })?
        .clone();
    let before = current(ctx, k)?;
    let mut cmd = vec!["default", app.id.as_str()];
    cmd.extend(k.mimes);
    ctx.cmd("xdg-mime", &cmd)?;
    let after = current(ctx, k)?;
    if after.as_deref() != Some(app.id.as_str()) {
        return Err(format!("The default {} is still {}.", k.label, after.map_or("unset".into(), |a| display_name(&all, &a))));
    }
    let mut o = Outcome::new(format!("Made {} the default {}", app.name, k.label), json!({ "kind": k.key, "id": app.id, "name": app.name }));
    if let Some(b) = before.filter(|b| *b != app.id) {
        o = o.undo("set_default_app", json!({ "kind": k.key, "app": b }));
    }
    Ok(o)
}
