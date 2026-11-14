//! Spotlight's ranking: apps, files, the calculator and conversions, grouped
//! into sections the way macOS shows them.

use std::path::PathBuf;

use crate::apps::DesktopEntry;
use crate::calc::{self, Answer};
use crate::files::FileIndex;

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    App(DesktopEntry),
    File { path: PathBuf, is_dir: bool },
    Answer(Answer),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: &'static str,
    pub hits: Vec<Hit>,
}

const MAX_APPS: usize = 6;
const MAX_FILES: usize = 5;

/// How well `app` matches the lowercase query `q`, if at all.
pub fn app_score(app: &DesktopEntry, q: &str) -> Option<i32> {
    let name = app.name.to_lowercase();
    if name == q {
        return Some(1000);
    }
    if name.starts_with(q) {
        return Some(900 - (name.len() - q.len()).min(50) as i32);
    }
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| w.starts_with(q)) {
        return Some(800);
    }
    // Initials: "vsc" for Visual Studio Code.
    let initials: String = words.iter().filter_map(|w| w.chars().next()).collect();
    if q.len() >= 2 && initials.starts_with(q) {
        return Some(700);
    }
    if name.contains(q) {
        return Some(600);
    }
    let program = app.command().split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("").to_lowercase();
    if program.starts_with(q) {
        return Some(550);
    }
    let generic = app.generic_name.to_lowercase();
    let extra_word = generic
        .split(|c: char| !c.is_alphanumeric())
        .chain(app.keywords.iter().map(String::as_str))
        .any(|w| w.to_lowercase().starts_with(q));
    if extra_word {
        return Some(500);
    }
    // Letters in order with gaps ("ffx" → Firefox), short gaps first.
    let mut gaps = 0;
    let mut chars = name.chars();
    for qc in q.chars() {
        let mut found = false;
        for c in chars.by_ref() {
            if c == qc {
                found = true;
                break;
            }
            gaps += 1;
        }
        if !found {
            return None;
        }
    }
    (q.len() >= 3).then(|| 300 - gaps.min(100))
}

pub fn search(query: &str, apps: &[DesktopEntry], files: Option<&FileIndex>) -> Vec<Section> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut sections = Vec::new();
    let answer = calc::answer(query);

    let mut ranked: Vec<(i32, &DesktopEntry)> = apps.iter().filter_map(|a| Some((app_score(a, &q)?, a))).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.len().cmp(&b.1.name.len())).then(a.1.name.cmp(&b.1.name)));
    ranked.truncate(MAX_APPS);
    // An app whose name starts with the query is the Top Hit, unless it's math.
    if let Some(&(_, app)) = ranked.first().filter(|(s, _)| *s >= 850 && answer.is_none()) {
        sections.push(Section { title: "Top Hit", hits: vec![Hit::App(app.clone())] });
        ranked.remove(0);
    }
    if let Some(a) = answer {
        let title = if matches!(a, Answer::Conversion { .. }) { "Conversion" } else { "Calculator" };
        sections.push(Section { title, hits: vec![Hit::Answer(a)] });
    }
    if !ranked.is_empty() {
        sections.push(Section { title: "Applications", hits: ranked.into_iter().map(|(_, a)| Hit::App(a.clone())).collect() });
    }
    // One or two letters match half of every disk; files wait for a third.
    if let Some(index) = files.filter(|_| q.chars().count() >= 3) {
        let hits: Vec<Hit> =
            index.search(&q, MAX_FILES).into_iter().map(|h| Hit::File { path: h.path, is_dir: h.is_dir }).collect();
        if !hits.is_empty() {
            sections.push(Section { title: "Documents", hits });
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, exec: &str, generic: &str, keywords: &[&str]) -> DesktopEntry {
        DesktopEntry {
            id: exec.into(),
            name: name.into(),
            exec: exec.into(),
            generic_name: generic.into(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            ..Default::default()
        }
    }

    fn apps() -> Vec<DesktopEntry> {
        vec![
            app("Firefox", "firefox %u", "Web Browser", &["internet", "www"]),
            app("Files", "dolphin", "File Manager", &[]),
            app("Konsole", "konsole", "Terminal", &["shell", "cmd"]),
            app("Visual Studio Code", "code", "Text Editor", &["vscode"]),
            app("Notes", "notes", "Notes", &[]),
            app("System Settings", "systemsettings", "", &["preferences"]),
            app("KCalc", "kcalc", "Scientific Calculator", &["calculator"]),
        ]
    }

    fn names(sections: &[Section]) -> Vec<(String, Vec<String>)> {
        sections
            .iter()
            .map(|s| {
                let hits = s
                    .hits
                    .iter()
                    .map(|h| match h {
                        Hit::App(a) => a.name.clone(),
                        Hit::File { path, .. } => path.file_name().unwrap().to_string_lossy().into(),
                        Hit::Answer(a) => a.value().to_string(),
                    })
                    .collect();
                (s.title.to_string(), hits)
            })
            .collect()
    }

    #[test]
    fn app_ranking() {
        let apps = apps();
        let r = names(&search("fi", &apps, None));
        assert_eq!(r[0], ("Top Hit".into(), vec!["Files".into()]));
        assert_eq!(r[1], ("Applications".into(), vec!["Firefox".into()]));

        // Word starts, initials, program names, keywords and fuzzy matches.
        let top = |q: &str| search(q, &apps, None).first().and_then(|s| match &s.hits[0] {
            Hit::App(a) => Some(a.name.clone()),
            _ => None,
        });
        assert_eq!(top("settings").as_deref(), Some("System Settings"));
        assert_eq!(top("vsc").as_deref(), Some("Visual Studio Code"));
        assert_eq!(top("dolph").as_deref(), Some("Files"));
        assert_eq!(top("terminal").as_deref(), Some("Konsole"));
        assert_eq!(top("calculator").as_deref(), Some("KCalc"));
        assert_eq!(top("ffx").as_deref(), Some("Firefox"));
        assert!(search("zzzz", &apps, None).is_empty());
        assert!(search("  ", &apps, None).is_empty());
    }

    #[test]
    fn calculator_comes_first() {
        let r = names(&search("12*12", &apps(), None));
        assert_eq!(r, vec![("Calculator".to_string(), vec!["144".to_string()])]);
        let r = names(&search("3 ft in cm", &apps(), None));
        assert_eq!(r[0], ("Conversion".to_string(), vec!["91.44 cm".to_string()]));
    }

    #[test]
    fn files_section() {
        let root = std::env::temp_dir().join(format!("y7-search-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("notes for firefox.txt"), "x").unwrap();
        let index = FileIndex::build(&root);
        let r = names(&search("notes", &apps(), Some(&index)));
        assert_eq!(r[0], ("Top Hit".into(), vec!["Notes".into()]));
        assert_eq!(r.last().unwrap(), &("Documents".to_string(), vec!["notes for firefox.txt".to_string()]));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
