pub mod fs;
mod icons;
pub mod places;
mod theme;
pub mod trash;
mod view;
mod watch;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use fs::{Entry, Op, Progress, SortKey};
use media::{Kind, Loader};
use places::{Mount, Places};
use theme::Theme;
use trash::Trash;
use ui::native::{SIDEBAR_W, TOOLBAR_H};
use ui::widgets::menu::{Menu, MenuItem, MenuKey};
use ui::*;

/// Transfers that finish sooner than this never show a progress panel.
const PROGRESS_DELAY: Duration = Duration::from_millis(400);
const TYPE_SELECT_RESET: Duration = Duration::from_millis(1000);
const THUMB_PX: u32 = 256;
const THUMB_CACHE: usize = 400;

#[derive(Clone, Debug, PartialEq)]
pub enum Location {
    Dir(PathBuf),
    Recents,
    Trash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Icons,
    List,
    Columns,
}

impl ViewMode {
    fn name(self) -> &'static str {
        match self {
            ViewMode::Icons => "icons",
            ViewMode::List => "list",
            ViewMode::Columns => "columns",
        }
    }

    pub fn parse(s: &str) -> Option<ViewMode> {
        [ViewMode::Icons, ViewMode::List, ViewMode::Columns].into_iter().find(|v| v.name() == s)
    }
}

#[derive(Clone, Debug)]
enum Action {
    Open,
    QuickLook,
    Rename,
    Duplicate,
    Trash,
    Copy,
    Cut,
    Paste,
    NewFolder,
    PutBack,
    DeleteNow,
    EmptyTrash,
    ShowEnclosing,
    View(ViewMode),
    Sort(SortKey),
    ToggleHidden,
}

enum Dialog {
    EmptyTrash,
    DeleteNow(Vec<PathBuf>),
    Error(String),
}

/// What Quick Look shows for the file under it.
enum Preview {
    Image(Image),
    Pages(Vec<Image>, u32),
    Text(String),
    Generic,
}

struct QuickLook {
    path: PathBuf,
    preview: Option<Preview>,
}

enum Thumb {
    Pending,
    Ready(Image),
    Failed,
}

struct Job {
    op: Op,
    count: usize,
    dest: PathBuf,
    progress: Arc<Progress>,
    started: Instant,
}

/// A mouse press on an item, which may turn into a drag.
struct Press {
    path: PathBuf,
    start: Point,
    /// Plain click on an already-selected item: select only it on release,
    /// unless the press becomes a drag of the whole selection.
    collapse: bool,
}

struct Drag {
    paths: Vec<PathBuf>,
}

/// Rubber-band selection in icon view, in content coordinates.
struct Band {
    start: Point,
    end: Point,
    base: HashSet<PathBuf>,
}

#[derive(Clone, Debug)]
enum DropTarget {
    Folder(PathBuf),
    Trash,
}

/// Command-line overrides for `--snapshot` renders.
#[derive(Default)]
pub struct Options {
    pub start: Option<PathBuf>,
    pub view: Option<ViewMode>,
    /// Don't read or write settings; run loads inline.
    pub snapshot: bool,
}

pub struct FilesApp {
    places: Places,
    disk_name: String,
    home_trash: Trash,
    mounts: Vec<Mount>,
    mounts_changed: Arc<AtomicBool>,
    loc: Location,
    back: Vec<Location>,
    forward: Vec<Location>,
    /// What the content area shows: the folder listing or search results.
    entries: Vec<Entry>,
    index: HashMap<PathBuf, usize>,
    selection: HashSet<PathBuf>,
    anchor: Option<PathBuf>,
    lead: Option<PathBuf>,
    view: ViewMode,
    sort: SortKey,
    ascending: bool,
    show_hidden: bool,
    /// Column view's first column; columns run from here to the current folder.
    col_root: PathBuf,
    /// Extra horizontal scroll of column view, from the right end.
    col_scroll: f32,
    listings: HashMap<PathBuf, Vec<Entry>>,
    search: Rc<RefCell<TextInput>>,
    query: String,
    search_gen: u64,
    searcher: Loader<u64, Vec<Entry>>,
    searching_busy: bool,
    rename: Rc<RefCell<TextInput>>,
    renaming: Option<PathBuf>,
    clipboard: Option<(Vec<PathBuf>, bool)>,
    menu: Option<Menu<Action>>,
    /// The open menu came from the toolbar's action button (drawn pressed meanwhile).
    menu_from_toolbar: bool,
    dialog: Option<Dialog>,
    quicklook: Option<QuickLook>,
    previews: Loader<PathBuf, Preview>,
    jobs: Vec<Job>,
    emptying: Option<Arc<AtomicBool>>,
    thumbs: HashMap<PathBuf, (Option<SystemTime>, Thumb, u64)>,
    thumb_loader: Loader<PathBuf, Option<Image>>,
    frame: u64,
    watcher: Option<watch::Watcher>,
    press: Option<Press>,
    drag: Option<Drag>,
    band: Option<Band>,
    /// Drop targets by the key given to their `.id()` (look up with `cx.hovered`).
    drop_targets: Vec<(ElementId, DropTarget)>,
    /// The content area's bounds as last laid out (for virtualized views).
    content_bounds: Rc<Cell<Bounds>>,
    icon_cols: usize,
    /// Scroll the lead item into view on the next render.
    reveal: bool,
    labels: HashMap<(String, u32), Vec<String>>,
    type_select: (String, Instant),
    waker: Waker,
    data_dir: Option<PathBuf>,
}

impl FilesApp {
    pub fn new(places: Places, opts: Options, cx: &mut Cx) -> Self {
        let fonts = &mut cx.text.fonts;
        let mut search = TextInput::new(fonts, TextStyle::default(), true);
        search.placeholder = "Search".into();
        let rename = TextInput::new(fonts, TextStyle::default(), true);
        cx.window_radius = 10.0;
        let waker = cx.waker();

        let mut searcher = Loader::new(1, waker.clone());
        let mut previews = Loader::new(1, waker.clone());
        let mut thumb_loader = Loader::new(3, waker.clone());
        if opts.snapshot {
            searcher.set_inline(true);
            previews.set_inline(true);
            thumb_loader.set_inline(true);
        }
        let data_dir = (!opts.snapshot).then(|| {
            std::env::var_os("FILES_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| places.data_home.join("files"))
        });
        let start = opts.start.clone().unwrap_or_else(|| places.home.clone());
        let mut app = Self {
            home_trash: Trash::home(&places.data_home),
            disk_name: places::disk_name(),
            mounts: places::mounts(),
            mounts_changed: if opts.snapshot { Arc::default() } else { watch::watch_mounts(waker.clone()) },
            loc: Location::Dir(start.clone()),
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            index: HashMap::new(),
            selection: HashSet::new(),
            anchor: None,
            lead: None,
            view: ViewMode::Icons,
            sort: SortKey::Name,
            ascending: true,
            show_hidden: false,
            col_root: start,
            col_scroll: 0.0,
            listings: HashMap::new(),
            search: Rc::new(RefCell::new(search)),
            query: String::new(),
            search_gen: 0,
            searcher,
            searching_busy: false,
            rename: Rc::new(RefCell::new(rename)),
            renaming: None,
            clipboard: None,
            menu: None,
            menu_from_toolbar: false,
            dialog: None,
            quicklook: None,
            previews,
            jobs: Vec::new(),
            emptying: None,
            thumbs: HashMap::new(),
            thumb_loader,
            frame: 0,
            watcher: if opts.snapshot { None } else { watch::Watcher::new(waker.clone()) },
            press: None,
            drag: None,
            band: None,
            drop_targets: Vec::new(),
            content_bounds: Rc::new(Cell::new(Bounds::default())),
            icon_cols: 1,
            reveal: false,
            labels: HashMap::new(),
            type_select: (String::new(), Instant::now()),
            waker,
            data_dir,
            places,
        };
        app.load_settings();
        if let Some(v) = opts.view {
            app.view = v;
        }
        app.col_root = app.column_root_for(&app.current_dir().unwrap_or(Path::new("/")).to_path_buf());
        app.reload();
        cx.focus("content");
        app
    }

    fn theme(cx: &Cx) -> Theme {
        if cx.dark { Theme::dark() } else { Theme::light() }
    }

    // -- settings -------------------------------------------------------------

    fn load_settings(&mut self) {
        let Some(dir) = &self.data_dir else { return };
        let Ok(text) = std::fs::read_to_string(dir.join("settings")) else { return };
        for line in text.lines() {
            match line.split_once('=') {
                Some(("view", v)) => self.view = ViewMode::parse(v).unwrap_or(self.view),
                Some(("sort", v)) => {
                    if let Some(k) = SortKey::ALL.into_iter().find(|k| k.label() == v) {
                        self.sort = k;
                    }
                }
                Some(("ascending", v)) => self.ascending = v == "true",
                Some(("hidden", v)) => self.show_hidden = v == "true",
                _ => {}
            }
        }
    }

    fn save_settings(&self) {
        let Some(dir) = &self.data_dir else { return };
        let _ = std::fs::create_dir_all(dir);
        let text = format!(
            "view={}\nsort={}\nascending={}\nhidden={}\n",
            self.view.name(),
            self.sort.label(),
            self.ascending,
            self.show_hidden
        );
        let _ = std::fs::write(dir.join("settings"), text);
    }

    // -- locations ------------------------------------------------------------

    fn current_dir(&self) -> Option<&Path> {
        match &self.loc {
            Location::Dir(p) => Some(p),
            _ => None,
        }
    }

    /// Where new folders and pastes go: the open folder, unless searching.
    fn target_dir(&self) -> Option<PathBuf> {
        if self.searching() { None } else { self.current_dir().map(Path::to_path_buf) }
    }

    fn searching(&self) -> bool {
        !self.query.trim().is_empty()
    }

    fn in_trash(&self) -> bool {
        self.loc == Location::Trash
    }

    fn all_trashes(&self) -> Vec<Trash> {
        let points: Vec<PathBuf> = self.mounts.iter().map(|m| m.path.clone()).collect();
        trash::all(&self.home_trash, &points)
    }

    /// The sidebar place a folder belongs under, which column view starts from.
    fn column_root_for(&self, dir: &Path) -> PathBuf {
        let p = &self.places;
        let mut roots: Vec<&Path> = vec![&p.desktop, &p.documents, &p.downloads, &p.pictures, &p.music, &p.videos, &p.home];
        roots.extend(self.mounts.iter().map(|m| m.path.as_path()));
        roots
            .into_iter()
            .filter(|r| dir.starts_with(r))
            .max_by_key(|r| r.components().count())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    pub fn go(&mut self, loc: Location, cx: &mut Cx) {
        if loc == self.loc && !self.searching() {
            return;
        }
        let prev = std::mem::replace(&mut self.loc, loc);
        self.back.push(prev);
        self.forward.clear();
        self.after_navigate(cx);
    }

    fn after_navigate(&mut self, cx: &mut Cx) {
        self.menu = None;
        self.finish_rename(true, cx);
        if self.searching() {
            self.query.clear();
            self.search.borrow_mut().set_text(&mut cx.text.fonts, "");
        }
        if let Some(dir) = self.current_dir().map(Path::to_path_buf) {
            if !dir.starts_with(&self.col_root) {
                self.col_root = self.column_root_for(&dir);
            }
        }
        self.col_scroll = 0.0;
        self.selection.clear();
        self.anchor = None;
        self.lead = None;
        self.reload();
        for id in ["icons-scroll", "list-scroll"] {
            cx.scroll_to_top(id);
        }
        cx.focus("content");
    }

    fn go_back(&mut self, cx: &mut Cx) {
        if let Some(loc) = self.back.pop() {
            let came_from = std::mem::replace(&mut self.loc, loc);
            self.forward.push(came_from.clone());
            self.after_navigate(cx);
            self.select_if_present(&came_from);
        }
    }

    fn go_forward(&mut self, cx: &mut Cx) {
        if let Some(loc) = self.forward.pop() {
            let came_from = std::mem::replace(&mut self.loc, loc);
            self.back.push(came_from);
            self.after_navigate(cx);
        }
    }

    /// Ctrl+Up: the enclosing folder, with the one we left selected.
    fn go_up(&mut self, cx: &mut Cx) {
        let Some(dir) = self.current_dir().map(Path::to_path_buf) else { return };
        if let Some(parent) = dir.parent() {
            self.go(Location::Dir(parent.to_path_buf()), cx);
            self.select_if_present(&Location::Dir(dir));
        }
    }

    fn select_if_present(&mut self, loc: &Location) {
        if let Location::Dir(p) = loc {
            if self.index.contains_key(p) {
                self.select_only(p.clone());
                self.reveal = true;
            }
        }
    }

    /// Column view: make `dir` the active column without touching history.
    fn enter_column(&mut self, dir: PathBuf, select: Option<PathBuf>) {
        self.loc = Location::Dir(dir);
        self.selection.clear();
        self.anchor = None;
        self.lead = None;
        self.reload();
        if let Some(p) = select {
            self.select_only(p);
        }
        self.col_scroll = 0.0;
        self.reveal = true;
    }

    // -- listing --------------------------------------------------------------

    fn list(&self, dir: &Path) -> Vec<Entry> {
        let mut v = fs::list_dir(dir, self.show_hidden).unwrap_or_default();
        fs::sort_entries(&mut v, self.sort, self.ascending);
        v
    }

    /// A folder's sorted entries, cached for column view.
    fn listing(&mut self, dir: &Path) -> &[Entry] {
        if !self.listings.contains_key(dir) {
            let v = self.list(dir);
            self.listings.insert(dir.to_path_buf(), v);
        }
        &self.listings[dir]
    }

    /// Re-reads the current location, keeping the selection where it still exists.
    fn reload(&mut self) {
        self.listings.clear();
        if self.searching() {
            self.start_search();
            return;
        }
        let mut entries = match &self.loc {
            Location::Dir(p) => fs::list_dir(p, self.show_hidden).unwrap_or_default(),
            Location::Recents => places::recents(&self.places.data_home, 200).iter().filter_map(|(p, _)| Entry::from_path(p)).collect(),
            Location::Trash => self.all_trashes().iter().flat_map(|t| t.items()).filter_map(|i| Entry::from_path(&i.path)).collect(),
        };
        fs::sort_entries(&mut entries, self.sort, self.ascending);
        self.set_entries(entries);
        self.update_watches();
    }

    fn set_entries(&mut self, entries: Vec<Entry>) {
        self.index = entries.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect();
        self.entries = entries;
        let index = &self.index;
        self.selection.retain(|p| index.contains_key(p));
        for p in [&mut self.anchor, &mut self.lead] {
            if p.as_ref().is_some_and(|p| !index.contains_key(p)) {
                *p = None;
            }
        }
        if self.renaming.as_ref().is_some_and(|p| !index.contains_key(p)) {
            self.renaming = None;
        }
    }

    fn resort(&mut self) {
        let mut entries = std::mem::take(&mut self.entries);
        fs::sort_entries(&mut entries, self.sort, self.ascending);
        self.set_entries(entries);
        self.listings.clear();
        self.save_settings();
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.sort == key {
            self.ascending = !self.ascending;
        } else {
            self.sort = key;
            self.ascending = key.default_ascending();
        }
        self.resort();
    }

    fn set_view(&mut self, view: ViewMode) {
        self.view = view;
        self.col_scroll = 0.0;
        self.reveal = true;
        if let Some(dir) = self.current_dir().map(Path::to_path_buf) {
            if !dir.starts_with(&self.col_root) {
                self.col_root = self.column_root_for(&dir);
            }
        }
        self.update_watches();
        self.save_settings();
    }

    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.reload();
        self.save_settings();
    }

    /// The folders shown as columns, from `col_root` to the current folder.
    fn column_dirs(&self) -> Vec<PathBuf> {
        let Some(dir) = self.current_dir() else { return Vec::new() };
        let mut chain: Vec<PathBuf> = dir.ancestors().take_while(|a| a.starts_with(&self.col_root)).map(Path::to_path_buf).collect();
        chain.reverse();
        chain
    }

    fn update_watches(&mut self) {
        let Some(w) = &self.watcher else { return };
        let mut dirs: Vec<PathBuf> = match &self.loc {
            Location::Dir(p) => vec![p.clone()],
            Location::Trash => self.all_trashes().iter().map(Trash::files).collect(),
            Location::Recents => vec![self.places.data_home.clone()],
        };
        if self.view == ViewMode::Columns {
            dirs.extend(self.column_dirs());
            if let Some(next) = self.single_selected().filter(|e| e.is_dir) {
                dirs.push(next.path.clone());
            }
        }
        let refs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
        w.watch(&refs);
    }

    // -- search ---------------------------------------------------------------

    fn start_search(&mut self) {
        self.search_gen += 1;
        let q = self.query.trim().to_lowercase();
        if q.is_empty() {
            return;
        }
        self.searching_busy = true;
        let show_hidden = self.show_hidden;
        let generation = self.search_gen;
        match &self.loc {
            Location::Dir(root) => {
                let root = root.clone();
                self.searcher.request(generation, move || search_tree(&root, &q, show_hidden));
            }
            _ => {
                // Recents and Trash: filter what's listed.
                let mut entries = match &self.loc {
                    Location::Recents => places::recents(&self.places.data_home, 200).iter().filter_map(|(p, _)| Entry::from_path(p)).collect(),
                    _ => self.all_trashes().iter().flat_map(|t| t.items()).filter_map(|i| Entry::from_path(&i.path)).collect::<Vec<_>>(),
                };
                entries.retain(|e| e.name.to_lowercase().contains(&q));
                self.searcher.request(generation, move || entries);
            }
        }
        self.take_search_results();
    }

    fn take_search_results(&mut self) {
        for (generation, mut results) in self.searcher.take_results() {
            if generation == self.search_gen && self.searching() {
                fs::sort_entries(&mut results, self.sort, self.ascending);
                self.set_entries(results);
                self.searching_busy = false;
            }
        }
    }

    // -- selection ------------------------------------------------------------

    fn selected_entries(&self) -> Vec<&Entry> {
        let mut v: Vec<&Entry> = self.selection.iter().filter_map(|p| self.index.get(p).map(|&i| &self.entries[i])).collect();
        v.sort_by_key(|e| self.index[&e.path]);
        v
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_entries().into_iter().map(|e| e.path.clone()).collect()
    }

    fn single_selected(&self) -> Option<&Entry> {
        if self.selection.len() == 1 { self.selected_entries().into_iter().next() } else { None }
    }

    fn select_only(&mut self, path: PathBuf) {
        self.selection.clear();
        self.selection.insert(path.clone());
        self.anchor = Some(path.clone());
        self.lead = Some(path);
        self.after_select();
    }

    fn toggle_selected(&mut self, path: PathBuf) {
        if !self.selection.remove(&path) {
            self.selection.insert(path.clone());
        }
        self.anchor = Some(path.clone());
        self.lead = Some(path);
        self.after_select();
    }

    /// Shift-click: everything between the anchor and `path`.
    fn extend_to(&mut self, path: PathBuf) {
        let Some(&to) = self.index.get(&path) else { return };
        let from = self.anchor.as_ref().and_then(|a| self.index.get(a)).copied().unwrap_or(to);
        let (lo, hi) = (from.min(to), from.max(to));
        self.selection = self.entries[lo..=hi].iter().map(|e| e.path.clone()).collect();
        self.lead = Some(path);
        self.after_select();
    }

    fn select_all(&mut self) {
        self.selection = self.entries.iter().map(|e| e.path.clone()).collect();
        self.after_select();
    }

    fn clear_selection(&mut self) {
        self.selection.clear();
        self.lead = None;
        self.after_select();
    }

    fn after_select(&mut self) {
        if self.renaming.as_ref().is_some_and(|r| !self.selection.contains(r)) {
            self.renaming = None;
        }
        if self.view == ViewMode::Columns {
            self.update_watches();
        }
        self.sync_quicklook();
    }

    /// Arrow keys: move the lead by `delta` items (rows in icon view).
    fn move_lead(&mut self, delta: isize, extend: bool) {
        if self.entries.is_empty() {
            return;
        }
        let cur = self.lead.as_ref().and_then(|p| self.index.get(p)).copied();
        let next = match cur {
            Some(i) => (i as isize + delta).clamp(0, self.entries.len() as isize - 1) as usize,
            None if delta < 0 => self.entries.len() - 1,
            None => 0,
        };
        let path = self.entries[next].path.clone();
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(path.clone());
            }
            self.extend_to(path);
        } else {
            self.select_only(path);
        }
        self.reveal = true;
    }

    fn move_to_end(&mut self, end: bool, extend: bool) {
        if let Some(e) = if end { self.entries.last() } else { self.entries.first() } {
            let path = e.path.clone();
            if extend { self.extend_to(path) } else { self.select_only(path) }
            self.reveal = true;
        }
    }

    /// Typing a name jumps to the first item that starts with it.
    fn type_select(&mut self, text: &str) {
        let (buf, at) = &mut self.type_select;
        if at.elapsed() > TYPE_SELECT_RESET {
            buf.clear();
        }
        buf.push_str(&text.to_lowercase());
        *at = Instant::now();
        let prefix = buf.clone();
        let hit = self.entries.iter().find(|e| e.name.to_lowercase().starts_with(&prefix)).or_else(|| {
            // Otherwise the closest name after it, like Finder.
            self.entries.iter().filter(|e| fs::natural_cmp(&e.name, &prefix).is_ge()).min_by(|a, b| fs::natural_cmp(&a.name, &b.name))
        });
        if let Some(e) = hit {
            let p = e.path.clone();
            self.select_only(p);
            self.reveal = true;
        }
    }

    // -- opening ---------------------------------------------------------------

    fn open_entry(&mut self, path: PathBuf, cx: &mut Cx) {
        let is_dir = self.index.get(&path).map_or_else(|| path.is_dir(), |&i| self.entries[i].is_dir);
        if is_dir {
            if self.view == ViewMode::Columns && !self.searching() && path.starts_with(&self.col_root) {
                self.enter_column(path, None);
                self.move_lead(0, false);
                return;
            }
            self.go(Location::Dir(path), cx);
        } else {
            open_with_system(&path);
        }
    }

    fn open_selection(&mut self, cx: &mut Cx) {
        let paths = self.selected_paths();
        let dirs: Vec<&PathBuf> = paths.iter().filter(|p| p.is_dir()).collect();
        if paths.len() == 1 || dirs.len() == 1 && paths.len() == dirs.len() {
            if let Some(p) = paths.first() {
                self.open_entry(p.clone(), cx);
            }
            return;
        }
        for p in paths.iter().filter(|p| !p.is_dir()) {
            open_with_system(p);
        }
    }

    fn show_enclosing(&mut self, cx: &mut Cx) {
        let Some(e) = self.single_selected() else { return };
        let path = e.path.clone();
        if let Some(parent) = path.parent() {
            self.go(Location::Dir(parent.to_path_buf()), cx);
            self.select_only(path);
            self.reveal = true;
        }
    }

    // -- rename ----------------------------------------------------------------

    fn start_rename(&mut self, cx: &mut Cx) {
        if self.in_trash() {
            return;
        }
        let Some(e) = self.single_selected() else { return };
        let (path, name, is_dir) = (e.path.clone(), e.name.clone(), e.is_dir);
        self.menu = None;
        let mut input = self.rename.borrow_mut();
        input.set_text(&mut cx.text.fonts, &name);
        // Select the name without its extension, like Finder.
        let stem = fs::split_ext(&name, is_dir).0.chars().count();
        let right = KeyPress { key: Key::Named(NamedKey::ArrowRight), text: None, ctrl: false, shift: true, alt: false };
        for _ in 0..stem {
            input.handle_key(&mut cx.text.fonts, &right, &mut None);
        }
        drop(input);
        self.renaming = Some(path);
        cx.focus("rename");
    }

    fn finish_rename(&mut self, commit: bool, cx: &mut Cx) {
        let Some(path) = self.renaming.take() else { return };
        cx.focus("content");
        if !commit {
            return;
        }
        let name = self.rename.borrow().text();
        let old = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.trim() == old {
            return;
        }
        match fs::rename(&path, &name) {
            Ok(new) => {
                self.reload();
                self.select_only(new);
                self.reveal = true;
            }
            Err(e) => self.dialog = Some(Dialog::Error(e)),
        }
    }

    // -- file operations ------------------------------------------------------

    fn new_folder(&mut self, cx: &mut Cx) {
        let Some(dir) = self.target_dir() else { return };
        match fs::new_folder(&dir) {
            Ok(p) => {
                self.reload();
                self.select_only(p);
                self.reveal = true;
                self.start_rename(cx);
            }
            Err(e) => self.dialog = Some(Dialog::Error(format!("Couldn\u{2019}t create a folder: {e}"))),
        }
    }

    fn copy(&mut self, cut: bool, cx: &mut Cx) {
        let paths = self.selected_paths();
        if paths.is_empty() || self.in_trash() {
            return;
        }
        // Other apps get the paths as text.
        if let Some(cb) = cx.clipboard.as_mut() {
            let text: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            let _ = cb.set_text(text.join("\n"));
        }
        self.clipboard = Some((paths, cut));
    }

    fn paste(&mut self) {
        let (Some(dest), Some((paths, cut))) = (self.target_dir(), self.clipboard.clone()) else { return };
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| std::fs::symlink_metadata(p).is_ok()).collect();
        if paths.is_empty() {
            return;
        }
        if cut {
            self.clipboard = None;
        }
        self.start_job(if cut { Op::Move } else { Op::Copy }, paths, dest);
    }

    fn duplicate(&mut self) {
        let Some(dir) = self.target_dir() else { return };
        let paths = self.selected_paths();
        if !paths.is_empty() && !self.in_trash() {
            self.start_job(Op::Duplicate, paths, dir);
        }
    }

    fn start_job(&mut self, op: Op, sources: Vec<PathBuf>, dest: PathBuf) {
        let progress = Arc::new(Progress::default());
        let p = progress.clone();
        let waker = self.waker.clone();
        let count = sources.len();
        let d = dest.clone();
        std::thread::spawn(move || {
            // Wake the UI for progress at most ten times a second.
            let last = Mutex::new(Instant::now());
            let tick = || {
                let mut last = last.lock().unwrap();
                if last.elapsed() >= Duration::from_millis(100) || p.finished.load(Ordering::Relaxed) {
                    *last = Instant::now();
                    waker.wake();
                }
            };
            fs::transfer(op, &sources, &d, &p, &tick);
        });
        self.jobs.push(Job { op, count, dest, progress, started: Instant::now() });
    }

    fn poll_jobs(&mut self) {
        let mut done = Vec::new();
        self.jobs.retain(|j| {
            let finished = j.progress.finished.load(Ordering::Relaxed);
            if finished {
                done.push((j.dest.clone(), j.progress.clone()));
            }
            !finished
        });
        for (dest, progress) in done {
            self.reload();
            if self.current_dir() == Some(dest.as_path()) {
                let created = progress.created.lock().unwrap().clone();
                if !created.is_empty() {
                    self.selection = created.iter().filter(|p| self.index.contains_key(*p)).cloned().collect();
                    self.lead = created.last().cloned();
                    self.anchor = created.first().cloned();
                    self.reveal = true;
                }
            }
            if let Some(e) = progress.error.lock().unwrap().take() {
                self.dialog = Some(Dialog::Error(e));
            }
        }
    }

    fn move_to_trash(&mut self, paths: Vec<PathBuf>) {
        let mut errors = Vec::new();
        for p in &paths {
            if let Err(e) = trash::trash(p, &self.home_trash) {
                errors.push(format!("\u{201c}{}\u{201d}: {e}", p.file_name().unwrap_or_default().to_string_lossy()));
            }
        }
        // Like Finder, select what followed the trashed items.
        let next = paths.iter().filter_map(|p| self.index.get(p)).max().map(|&i| i + 1);
        let follower = next.and_then(|i| self.entries[i..].iter().find(|e| !paths.contains(&e.path))).map(|e| e.path.clone());
        self.reload();
        if let Some(p) = follower {
            self.select_only(p);
        }
        if !errors.is_empty() {
            self.dialog = Some(Dialog::Error(format!("Some items couldn\u{2019}t be moved to the Trash.\n{}", errors.join("\n"))));
        }
    }

    fn trash_selection(&mut self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        if self.in_trash() {
            self.dialog = Some(Dialog::DeleteNow(paths));
        } else {
            self.move_to_trash(paths);
        }
    }

    fn trash_holding(&self, path: &Path) -> Option<Trash> {
        self.all_trashes().into_iter().find(|t| t.holds(path))
    }

    fn put_back(&mut self) {
        let mut errors = Vec::new();
        for p in self.selected_paths() {
            let result = self.trash_holding(&p).ok_or_else(|| std::io::Error::other("not in the Trash")).and_then(|t| t.restore(&p));
            if let Err(e) = result {
                errors.push(format!("\u{201c}{}\u{201d} couldn\u{2019}t be put back: {e}", p.file_name().unwrap_or_default().to_string_lossy()));
            }
        }
        self.reload();
        if !errors.is_empty() {
            self.dialog = Some(Dialog::Error(errors.join("\n")));
        }
    }

    fn delete_now(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            if let Some(t) = self.trash_holding(&p) {
                let _ = t.erase(&p);
            }
        }
        self.reload();
    }

    fn trash_is_empty(&self) -> bool {
        self.all_trashes().iter().all(Trash::is_empty)
    }

    fn empty_trash(&mut self) {
        let done = Arc::new(AtomicBool::new(false));
        let d = done.clone();
        let trashes = self.all_trashes();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            for t in trashes {
                let _ = t.empty();
            }
            d.store(true, Ordering::Relaxed);
            waker.wake();
        });
        self.emptying = Some(done);
    }

    /// Drops dragged items on a folder: a move on the same disk, a copy
    /// across disks or with Ctrl held (Option on a Mac).
    fn drop_on(&mut self, target: DropTarget, paths: Vec<PathBuf>, copy: bool) {
        match target {
            DropTarget::Trash => {
                if !self.in_trash() {
                    self.move_to_trash(paths);
                }
            }
            DropTarget::Folder(dir) => {
                let paths: Vec<PathBuf> = paths.into_iter().filter(|p| *p != dir && p.parent() != Some(dir.as_path())).collect();
                if paths.is_empty() || paths.iter().any(|p| dir.starts_with(p)) {
                    return;
                }
                let op = if copy || !fs::same_device(&paths[0], &dir) { Op::Copy } else { Op::Move };
                self.start_job(op, paths, dir);
            }
        }
    }

    // -- thumbnails ------------------------------------------------------------

    /// The thumbnail for an image or PDF, loading it in the background.
    fn thumbnail(&mut self, path: &Path, modified: Option<SystemTime>, kind: Kind) -> Option<Image> {
        if !kind.has_thumbnail() || (kind == Kind::Pdf && media::pdf::backend().is_none()) {
            return None;
        }
        let frame = self.frame;
        if let Some((m, thumb, used)) = self.thumbs.get_mut(path) {
            if *m == modified {
                *used = frame;
                return match thumb {
                    Thumb::Ready(img) => Some(img.clone()),
                    _ => None,
                };
            }
        }
        self.thumbs.insert(path.to_path_buf(), (modified, Thumb::Pending, frame));
        let p = path.to_path_buf();
        self.thumb_loader.request(p.clone(), move || {
            if kind == Kind::Pdf { media::pdf::render_page(&p, 1, THUMB_PX) } else { Image::load(&p, THUMB_PX) }
        });
        self.take_thumbs();
        match &self.thumbs[path].1 {
            Thumb::Ready(img) => Some(img.clone()),
            _ => None,
        }
    }

    fn take_thumbs(&mut self) {
        for (path, img) in self.thumb_loader.take_results() {
            if let Some(t) = self.thumbs.get_mut(&path) {
                t.1 = match img {
                    Some(i) => Thumb::Ready(i),
                    None => Thumb::Failed,
                };
            }
        }
    }

    /// After a render: forget queued loads that scrolled out of view and
    /// trim the cache.
    fn trim_thumbs(&mut self) {
        let frame = self.frame;
        let thumbs = &self.thumbs;
        let dropped = self.thumb_loader.retain_pending(|p| thumbs.get(p).is_some_and(|t| t.2 == frame));
        for p in dropped {
            self.thumbs.remove(&p);
        }
        if self.thumbs.len() > THUMB_CACHE {
            let mut by_age: Vec<(u64, PathBuf)> =
                self.thumbs.iter().filter(|(_, t)| !matches!(t.1, Thumb::Pending)).map(|(p, t)| (t.2, p.clone())).collect();
            by_age.sort();
            for (_, p) in by_age.into_iter().take(self.thumbs.len() - THUMB_CACHE) {
                self.thumbs.remove(&p);
            }
        }
        self.frame += 1;
    }

    // -- Quick Look -----------------------------------------------------------

    fn toggle_quicklook(&mut self) {
        if self.quicklook.take().is_some() {
            return;
        }
        if let Some(p) = self.lead.clone().filter(|p| self.selection.contains(p)).or_else(|| self.selected_paths().first().cloned()) {
            self.quicklook = Some(QuickLook { path: PathBuf::new(), preview: None });
            self.show_in_quicklook(p);
        }
    }

    fn sync_quicklook(&mut self) {
        if self.quicklook.is_none() {
            return;
        }
        match self.lead.clone().filter(|p| self.selection.contains(p)) {
            Some(p) => self.show_in_quicklook(p),
            None => self.quicklook = None,
        }
    }

    fn show_in_quicklook(&mut self, path: PathBuf) {
        let Some(ql) = self.quicklook.as_mut() else { return };
        if ql.path == path {
            return;
        }
        ql.path = path.clone();
        ql.preview = None;
        let kind = self.index.get(&path).map_or(Kind::Other, |&i| self.entries[i].kind);
        self.previews.request(path.clone(), move || load_preview(&path, kind));
        self.take_previews();
    }

    fn take_previews(&mut self) {
        for (path, preview) in self.previews.take_results() {
            if let Some(ql) = self.quicklook.as_mut().filter(|q| q.path == path) {
                ql.preview = Some(preview);
            }
        }
    }

    // -- menus ----------------------------------------------------------------

    fn selection_items(&self) -> Vec<MenuItem<Action>> {
        let sel = self.selected_entries();
        let n = sel.len();
        let name = if n == 1 { format!("\u{201c}{}\u{201d}", sel[0].name) } else { format!("{n} Items") };
        if self.in_trash() {
            return vec![
                MenuItem::new("Put Back", Action::PutBack),
                MenuItem::new("Delete Immediately\u{2026}", Action::DeleteNow),
                MenuItem::separator(),
                MenuItem::new(format!("Quick Look {name}"), Action::QuickLook).shortcut("Space"),
                MenuItem::separator(),
                MenuItem::new("Empty Trash", Action::EmptyTrash).enabled(!self.trash_is_empty()),
            ];
        }
        let mut items = vec![
            MenuItem::new("Open", Action::Open).shortcut("Ctrl+O"),
            MenuItem::separator(),
            MenuItem::new("Move to Trash", Action::Trash).shortcut("Delete"),
            MenuItem::separator(),
            MenuItem::new("Rename", Action::Rename).shortcut("Enter").enabled(n == 1),
            MenuItem::new("Duplicate", Action::Duplicate).shortcut("Ctrl+D").enabled(self.target_dir().is_some()),
            MenuItem::new(format!("Quick Look {name}"), Action::QuickLook).shortcut("Space"),
            MenuItem::separator(),
            MenuItem::new(format!("Copy {name}"), Action::Copy).shortcut("Ctrl+C"),
            MenuItem::new("Cut", Action::Cut).shortcut("Ctrl+X"),
        ];
        if self.searching() || self.loc == Location::Recents {
            items.push(MenuItem::separator());
            items.push(MenuItem::new("Show in Enclosing Folder", Action::ShowEnclosing).enabled(n == 1));
        }
        items
    }

    fn background_items(&self) -> Vec<MenuItem<Action>> {
        let mut items = Vec::new();
        if self.in_trash() {
            items.push(MenuItem::new("Empty Trash", Action::EmptyTrash).enabled(!self.trash_is_empty()));
            items.push(MenuItem::separator());
        } else {
            let can = self.target_dir().is_some();
            items.push(MenuItem::new("New Folder", Action::NewFolder).shortcut("Ctrl+Shift+N").enabled(can));
            items.push(MenuItem::separator());
            let label = match &self.clipboard {
                Some((p, _)) if p.len() == 1 => "Paste Item".to_string(),
                Some((p, _)) => format!("Paste {} Items", p.len()),
                None => "Paste".to_string(),
            };
            items.push(MenuItem::new(label, Action::Paste).shortcut("Ctrl+V").enabled(can && self.clipboard.is_some()));
            items.push(MenuItem::separator());
        }
        for (v, label, key) in [(ViewMode::Icons, "as Icons", "Ctrl+1"), (ViewMode::List, "as List", "Ctrl+2"), (ViewMode::Columns, "as Columns", "Ctrl+3")] {
            items.push(MenuItem::new(label, Action::View(v)).checked(self.view == v).shortcut(key));
        }
        items.push(MenuItem::separator());
        for k in SortKey::ALL {
            items.push(MenuItem::new(format!("Sort by {}", k.label()), Action::Sort(k)).checked(self.sort == k));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::new("Show Hidden Files", Action::ToggleHidden).checked(self.show_hidden).shortcut("Ctrl+Shift+."));
        items
    }

    fn item_menu(&mut self, path: PathBuf, cx: &mut Cx) {
        if !self.selection.contains(&path) {
            self.select_only(path);
        }
        cx.focus("content");
        self.menu = Some(Menu::new(cx.mouse, self.selection_items()));
        self.menu_from_toolbar = false;
    }

    fn background_menu(&mut self, cx: &mut Cx) {
        self.clear_selection();
        cx.focus("content");
        self.menu = Some(Menu::new(cx.mouse, self.background_items()));
        self.menu_from_toolbar = false;
    }

    /// The toolbar's action menu: the selection's commands, then the folder's.
    fn action_menu(&mut self, pos: Point) {
        let mut items = Vec::new();
        if !self.selection.is_empty() {
            items.extend(self.selection_items());
            items.push(MenuItem::separator());
        }
        items.extend(self.background_items());
        self.menu = Some(Menu::new(pos, items));
        self.menu_from_toolbar = true;
    }

    fn run(&mut self, action: Action, cx: &mut Cx) {
        self.menu = None;
        match action {
            Action::Open => self.open_selection(cx),
            Action::QuickLook => self.toggle_quicklook(),
            Action::Rename => self.start_rename(cx),
            Action::Duplicate => self.duplicate(),
            Action::Trash => self.trash_selection(),
            Action::Copy => self.copy(false, cx),
            Action::Cut => self.copy(true, cx),
            Action::Paste => self.paste(),
            Action::NewFolder => self.new_folder(cx),
            Action::PutBack => self.put_back(),
            Action::DeleteNow => self.dialog = Some(Dialog::DeleteNow(self.selected_paths())),
            Action::EmptyTrash => self.dialog = Some(Dialog::EmptyTrash),
            Action::ShowEnclosing => self.show_enclosing(cx),
            Action::View(v) => self.set_view(v),
            Action::Sort(k) => {
                if self.sort != k {
                    self.set_sort(k);
                }
            }
            Action::ToggleHidden => self.toggle_hidden(),
        }
    }

    fn confirm_dialog(&mut self, cx: &mut Cx) {
        match self.dialog.take() {
            Some(Dialog::EmptyTrash) => self.empty_trash(),
            Some(Dialog::DeleteNow(paths)) => self.delete_now(paths),
            _ => {}
        }
        cx.focus("content");
    }

    // -- mouse ----------------------------------------------------------------

    fn item_mouse_down(&mut self, path: PathBuf, cx: &mut Cx) {
        self.menu = None;
        if self.renaming.as_ref().is_some_and(|r| *r != path) {
            self.finish_rename(true, cx);
        }
        let m = cx.modifiers;
        let mut collapse = false;
        if m.shift_key() {
            self.extend_to(path.clone());
        } else if m.control_key() {
            self.toggle_selected(path.clone());
        } else if self.selection.contains(&path) {
            collapse = self.selection.len() > 1;
            self.lead = Some(path.clone());
            self.anchor = Some(path.clone());
            self.sync_quicklook();
        } else {
            self.select_only(path.clone());
        }
        if self.selection.contains(&path) {
            self.press = Some(Press { path, start: cx.mouse, collapse });
        }
        cx.focus("content");
    }

    fn background_mouse_down(&mut self, origin: Point, cx: &mut Cx) {
        self.menu = None;
        self.finish_rename(true, cx);
        let m = cx.modifiers;
        let keep = m.shift_key() || m.control_key();
        if !keep {
            self.clear_selection();
        }
        if self.view == ViewMode::Icons {
            let p = Point { x: cx.mouse.x - origin.x, y: cx.mouse.y - origin.y };
            self.band = Some(Band { start: p, end: p, base: if keep { self.selection.clone() } else { HashSet::new() } });
        }
        cx.focus("content");
    }
}

impl App for FilesApp {
    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Self::theme(cx).text }
    }

    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        self.render_window(cx)
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        self.handle_key(key, cx)
    }

    fn on_input(&mut self, id: ElementId, cx: &mut Cx) {
        if id == eid("search") {
            let q = self.search.borrow().text();
            let was = self.searching();
            self.query = q;
            if self.searching() {
                self.start_search();
            } else if was {
                self.searching_busy = false;
                self.reload();
            }
            cx.scroll_to_top("icons-scroll");
            cx.scroll_to_top("list-scroll");
        }
    }

    fn next_wake(&self) -> Option<Instant> {
        // Show the progress panel once a transfer has run for a moment.
        self.jobs.iter().map(|j| j.started + PROGRESS_DELAY).filter(|t| *t > Instant::now()).min()
    }

    fn on_external(&mut self, cx: &mut Cx) {
        self.take_thumbs();
        self.take_previews();
        self.take_search_results();
        self.poll_jobs();
        if self.emptying.as_ref().is_some_and(|d| d.load(Ordering::Relaxed)) {
            self.emptying = None;
            self.reload();
        }
        if self.mounts_changed.swap(false, Ordering::Relaxed) {
            self.mounts = places::mounts();
            // The folder we're in may have been unmounted.
            if self.current_dir().is_some_and(|d| !d.exists()) {
                let home = self.places.home.clone();
                self.go(Location::Dir(home), cx);
            }
        }
        let changed = self.watcher.as_ref().map(|w| w.take_changed()).unwrap_or_default();
        if !changed.is_empty() {
            if self.current_dir().is_some_and(|d| !d.exists()) {
                // Our folder went away: go to the nearest one that's left.
                let dir = self.current_dir().unwrap().to_path_buf();
                let parent = dir.ancestors().find(|a| a.is_dir()).unwrap_or(Path::new("/")).to_path_buf();
                self.loc = Location::Dir(parent);
            }
            self.reload();
        }
    }

    fn on_mouse_move(&mut self, cx: &mut Cx) {
        if !cx.mouse_down {
            return;
        }
        if let Some(press) = &self.press {
            let d = (cx.mouse.x - press.start.x).abs().max((cx.mouse.y - press.start.y).abs());
            if self.drag.is_none() && d > 4.0 && self.renaming.is_none() {
                self.drag = Some(Drag { paths: self.selected_paths() });
            }
            if self.drag.is_some() {
                cx.request_redraw();
            }
        }
        if self.band.is_some() {
            let vp = self.content_bounds.get();
            let scroll = cx.scroll_y("icons-scroll");
            let end = Point { x: cx.mouse.x - vp.x, y: cx.mouse.y - vp.y + scroll };
            self.update_band(end);
            cx.request_redraw();
        }
    }

    fn on_mouse_up(&mut self, cx: &mut Cx) {
        let press = self.press.take();
        if let Some(drag) = self.drag.take() {
            let target = self.drop_targets.iter().find(|(id, _)| cx.hovered(*id)).map(|(_, t)| t.clone());
            if let Some(t) = target {
                self.drop_on(t, drag.paths, cx.modifiers.control_key());
            }
            cx.request_redraw();
        } else if let Some(p) = press.filter(|p| p.collapse) {
            self.select_only(p.path);
        }
        if self.band.take().is_some() {
            cx.request_redraw();
        }
    }

    fn on_scroll(&mut self, delta: Point, cx: &mut Cx) -> bool {
        // Sideways scrolling in column view (or Shift+wheel).
        if self.view != ViewMode::Columns || self.searching() || self.current_dir().is_none() {
            return false;
        }
        let vp = self.content_bounds.get();
        if !vp.contains(cx.mouse) {
            return false;
        }
        let dx = if cx.modifiers.shift_key() { delta.y } else { delta.x };
        if dx.abs() <= delta.y.abs() * 0.5 && !cx.modifiers.shift_key() {
            return false;
        }
        self.col_scroll = (self.col_scroll + dx).max(0.0);
        cx.request_redraw();
        true
    }
}

impl FilesApp {
    fn handle_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        if self.dialog.is_some() {
            if key.is_named(NamedKey::Escape) {
                self.dialog = None;
            } else if key.is_named(NamedKey::Enter) {
                self.confirm_dialog(cx);
            }
            return true;
        }
        // An open menu takes the keyboard: arrows, type-to-select, Return, Escape.
        if let Some(menu) = &mut self.menu {
            match menu.key(key, cx) {
                MenuKey::Pick(a) => self.run(a, cx),
                MenuKey::Close => self.menu = None,
                MenuKey::Stay => {}
            }
            return true;
        }
        if key.is_named(NamedKey::Escape) {
            if self.quicklook.take().is_some() {
            } else if self.renaming.is_some() {
                self.finish_rename(false, cx);
            } else if self.drag.take().is_some() {
            } else if cx.is_focused("search") || self.searching() {
                self.search.borrow_mut().set_text(&mut cx.text.fonts, "");
                self.on_input(eid("search"), cx);
                cx.focus("content");
            }
            return true;
        }
        if self.renaming.is_some() {
            if key.is_named(NamedKey::Enter) {
                self.finish_rename(true, cx);
            } else {
                // Typed straight into the field: it may not be on screen yet
                // when typing starts right after New Folder.
                self.rename.borrow_mut().handle_key(&mut cx.text.fonts, key, &mut cx.clipboard);
            }
            return true;
        }
        if cx.is_focused("search") {
            if key.is_named(NamedKey::Enter) || key.is_named(NamedKey::ArrowDown) {
                cx.focus("content");
                if self.selection.is_empty() {
                    self.move_lead(0, false);
                }
                return true;
            }
            if !(key.ctrl && (key.is_char('w') || key.is_char('q'))) {
                return false;
            }
        }

        let shift = key.shift;
        if key.ctrl && !key.alt {
            // Ctrl+Shift+. toggles hidden files ('>' with Shift on most layouts).
            if shift && (key.is_char('.') || key.is_char('>')) {
                self.toggle_hidden();
                return true;
            }
            if shift {
                let place = if key.is_char('h') {
                    Some(self.places.home.clone())
                } else if key.is_char('d') {
                    Some(self.places.desktop.clone())
                } else if key.is_char('o') {
                    Some(self.places.documents.clone())
                } else if key.is_char('l') {
                    Some(self.places.downloads.clone())
                } else {
                    None
                };
                if let Some(p) = place {
                    self.go(Location::Dir(p), cx);
                    return true;
                }
                if key.is_char('n') {
                    self.new_folder(cx);
                    return true;
                }
                if key.is_char('f') {
                    self.go(Location::Recents, cx);
                    return true;
                }
                if key.is_named(NamedKey::Backspace) || key.is_named(NamedKey::Delete) {
                    if !self.trash_is_empty() {
                        self.dialog = Some(Dialog::EmptyTrash);
                    }
                    return true;
                }
            }
            let handled = if key.is_char('o') || key.is_named(NamedKey::ArrowDown) {
                self.open_selection(cx);
                true
            } else if key.is_named(NamedKey::ArrowUp) {
                self.go_up(cx);
                true
            } else if key.is_char('[') {
                self.go_back(cx);
                true
            } else if key.is_char(']') {
                self.go_forward(cx);
                true
            } else if key.is_char('a') {
                self.select_all();
                true
            } else if key.is_char('c') {
                self.copy(false, cx);
                true
            } else if key.is_char('x') {
                self.copy(true, cx);
                true
            } else if key.is_char('v') {
                self.paste();
                true
            } else if key.is_char('d') {
                self.duplicate();
                true
            } else if key.is_char('y') {
                self.toggle_quicklook();
                true
            } else if key.is_char('f') {
                self.search.borrow_mut().select_all();
                cx.focus("search");
                true
            } else if key.is_char('1') {
                self.set_view(ViewMode::Icons);
                true
            } else if key.is_char('2') {
                self.set_view(ViewMode::List);
                true
            } else if key.is_char('3') {
                self.set_view(ViewMode::Columns);
                true
            } else if key.is_char('w') || key.is_char('q') {
                cx.close_window();
                true
            } else if key.is_named(NamedKey::Backspace) {
                self.trash_selection();
                true
            } else {
                false
            };
            return handled;
        }
        if key.alt && !key.ctrl {
            if key.is_named(NamedKey::ArrowLeft) {
                self.go_back(cx);
                return true;
            }
            if key.is_named(NamedKey::ArrowRight) {
                self.go_forward(cx);
                return true;
            }
            if key.is_named(NamedKey::ArrowUp) {
                self.go_up(cx);
                return true;
            }
        }
        let Key::Named(named) = &key.key else {
            // Plain typing selects by name.
            if let Some(t) = key.text.as_deref().filter(|t| t.chars().all(|c| !c.is_control()) && !t.is_empty()) {
                if !key.ctrl && !key.alt && t != " " {
                    self.type_select(t);
                    return true;
                }
            }
            return false;
        };
        let columns = self.view == ViewMode::Columns && !self.searching() && self.current_dir().is_some();
        let cols = if self.view == ViewMode::Icons { self.icon_cols.max(1) as isize } else { 1 };
        match named {
            NamedKey::Space => self.toggle_quicklook(),
            NamedKey::Enter => self.start_rename(cx),
            NamedKey::Delete => self.trash_selection(),
            NamedKey::Backspace if self.in_trash() => self.trash_selection(),
            NamedKey::Backspace => self.go_back(cx),
            NamedKey::ArrowDown => self.move_lead(cols, shift),
            NamedKey::ArrowUp => self.move_lead(-cols, shift),
            NamedKey::ArrowRight if columns => {
                if let Some(e) = self.single_selected().filter(|e| e.is_dir) {
                    let dir = e.path.clone();
                    let first = self.listing(&dir).first().map(|e| e.path.clone());
                    if let Some(first) = first {
                        self.enter_column(dir, Some(first));
                    }
                }
            }
            NamedKey::ArrowLeft if columns => {
                let dir = self.current_dir().unwrap().to_path_buf();
                if dir != self.col_root {
                    if let Some(parent) = dir.parent() {
                        self.enter_column(parent.to_path_buf(), Some(dir));
                    }
                }
            }
            NamedKey::ArrowRight if self.view == ViewMode::Icons => self.move_lead(1, shift),
            NamedKey::ArrowLeft if self.view == ViewMode::Icons => self.move_lead(-1, shift),
            NamedKey::Home => self.move_to_end(false, shift),
            NamedKey::End => self.move_to_end(true, shift),
            NamedKey::PageDown => self.move_lead(cols * 10, shift),
            NamedKey::PageUp => self.move_lead(-cols * 10, shift),
            NamedKey::Tab => {
                cx.focus(if cx.is_focused("search") { "content" } else { "search" });
            }
            _ => return false,
        }
        true
    }
}

/// Opens a file with the user's default app.
fn open_with_system(path: &Path) {
    if let Ok(mut child) = std::process::Command::new("xdg-open").arg(path).spawn() {
        // Reap it so it doesn't linger as a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Searches `root` breadth-first for names containing `q`, with limits so a
/// search from `/` stays quick.
fn search_tree(root: &Path, q: &str, show_hidden: bool) -> Vec<Entry> {
    const MAX_RESULTS: usize = 500;
    const MAX_VISITED: usize = 60_000;
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    let mut visited = 0;
    while let Some(dir) = queue.pop_front() {
        let Ok(it) = std::fs::read_dir(&dir) else { continue };
        for item in it.flatten() {
            visited += 1;
            let name = item.file_name().to_string_lossy().into_owned();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let ft = item.file_type().ok();
            if ft.is_some_and(|t| t.is_dir()) {
                queue.push_back(item.path());
            }
            if name.to_lowercase().contains(q) {
                if let Some(e) = Entry::from_path(&item.path()) {
                    out.push(e);
                }
            }
            if out.len() >= MAX_RESULTS || visited >= MAX_VISITED {
                return out;
            }
        }
    }
    out
}

fn load_preview(path: &Path, kind: Kind) -> Preview {
    match kind {
        Kind::Image => Image::load(path, 2048).map_or(Preview::Generic, Preview::Image),
        Kind::Pdf => {
            let Some(info) = media::pdf::info(path) else { return Preview::Generic };
            let pages: Vec<Image> = (1..=info.pages.min(4)).filter_map(|p| media::pdf::render_page(path, p, 1400)).collect();
            if pages.is_empty() { Preview::Generic } else { Preview::Pages(pages, info.pages) }
        }
        _ if kind.is_textual() || kind == Kind::Other => {
            match media::text_excerpt(path, 48 * 1024) {
                Some(t) if !t.is_empty() => {
                    let lines: Vec<&str> = t.lines().take(600).collect();
                    Preview::Text(lines.join("\n"))
                }
                _ => Preview::Generic,
            }
        }
        _ => Preview::Generic,
    }
}

impl FilesApp {
    /// For snapshots: selects entries by name.
    pub fn select_names(&mut self, names: &[&str]) {
        self.selection = self.entries.iter().filter(|e| names.contains(&e.name.as_str())).map(|e| e.path.clone()).collect();
        self.lead = self.entries.iter().find(|e| names.last() == Some(&e.name.as_str())).map(|e| e.path.clone());
        self.anchor = self.lead.clone();
        self.after_select();
    }

    /// For snapshots: opens Quick Look on the selection.
    pub fn open_quicklook(&mut self) {
        self.toggle_quicklook();
    }

    pub fn set_column_root(&mut self, root: PathBuf) {
        self.col_root = root;
    }

    pub fn show_location(&mut self, loc: Location, cx: &mut Cx) {
        self.go(loc, cx);
    }

    pub fn open_context_menu(&mut self, at: Point, cx: &mut Cx) {
        cx.mouse = at;
        match self.selected_paths().first().cloned() {
            Some(p) => self.item_menu(p, cx),
            None => self.background_menu(cx),
        }
    }

    pub fn confirm_empty_trash(&mut self) {
        self.dialog = Some(Dialog::EmptyTrash);
    }

    pub fn set_query(&mut self, q: &str, cx: &mut Cx) {
        self.search.borrow_mut().set_text(&mut cx.text.fonts, q);
        self.on_input(eid("search"), cx);
        cx.focus("search");
    }

    /// Band selection: everything whose icon cell meets the band.
    fn update_band(&mut self, end: Point) {
        let Some(band) = self.band.as_mut() else { return };
        band.end = end;
        let r = Bounds::new(band.start.x.min(end.x), band.start.y.min(end.y), (band.start.x - end.x).abs(), (band.start.y - end.y).abs());
        let mut sel = band.base.clone();
        for (i, e) in self.entries.iter().enumerate() {
            let cell = view::icon_item_rect(i, self.icon_cols, self.content_bounds.get().w);
            let hit = cell.x < r.right() && cell.right() > r.x && cell.y < r.bottom() && cell.bottom() > r.y;
            if hit {
                sel.insert(e.path.clone());
            }
        }
        self.selection = sel;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: Key, ctrl: bool, shift: bool) -> KeyPress {
        let text = match &k {
            Key::Character(s) => Some(s.to_string()),
            _ => None,
        };
        KeyPress { key: k, text, ctrl, shift, alt: false }
    }

    fn names(app: &FilesApp) -> Vec<String> {
        app.entries.iter().map(|e| e.name.clone()).collect()
    }

    fn wait_for_jobs(app: &mut FilesApp) {
        let start = Instant::now();
        while !app.jobs.is_empty() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
            app.poll_jobs();
        }
        assert!(app.jobs.is_empty(), "transfer didn't finish");
    }

    /// Drives the whole app against a throwaway home: navigation, selection,
    /// new folder + rename, copy/paste, duplicate, trash and put back.
    #[test]
    fn file_operations_through_the_ui() {
        let home = std::env::temp_dir().join(format!("files-app-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in ["Documents", "Pictures"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join("Documents/b.txt"), b"bee").unwrap();
        std::fs::write(home.join("Documents/a.txt"), b"a").unwrap();
        std::fs::write(home.join("Documents/.secret"), b"").unwrap();
        let home = std::fs::canonicalize(&home).unwrap();
        let docs = home.join("Documents");
        let places = Places::rooted(&home);
        let png = std::env::temp_dir().join(format!("files-app-test-{}.png", std::process::id()));
        let opts = Options { start: Some(docs.clone()), view: Some(ViewMode::List), snapshot: true };
        ui::snapshot((900.0, 600.0), 1.0, false, &png, |cx| FilesApp::new(places, opts, cx), |app, cx| {
            assert_eq!(names(app), ["a.txt", "b.txt"]);
            app.toggle_hidden();
            assert_eq!(names(app), [".secret", "a.txt", "b.txt"]);
            app.toggle_hidden();

            // Keyboard navigation and shift-extension.
            app.handle_key(&key(Key::Named(NamedKey::ArrowDown), false, false), cx);
            assert_eq!(app.selected_paths(), [docs.join("a.txt")]);
            app.handle_key(&key(Key::Named(NamedKey::ArrowDown), false, true), cx);
            assert_eq!(app.selection.len(), 2);
            app.type_select("b");
            assert_eq!(app.selected_paths(), [docs.join("b.txt")]);

            // New folder starts renaming it.
            app.handle_key(&key(Key::Character("n".into()), true, true), cx);
            assert_eq!(app.renaming.as_deref(), Some(docs.join("untitled folder").as_path()));
            app.rename.borrow_mut().handle_key(&mut cx.text.fonts, &key(Key::Character("Z".into()), false, false), &mut None);
            app.handle_key(&key(Key::Named(NamedKey::Enter), false, false), cx);
            assert!(docs.join("Z").is_dir());
            assert_eq!(app.selected_paths(), [docs.join("Z")]);

            // Enter on a file selects only the stem, so typing keeps the extension.
            app.select_names(&["a.txt"]);
            app.handle_key(&key(Key::Named(NamedKey::Enter), false, false), cx);
            app.rename.borrow_mut().handle_key(&mut cx.text.fonts, &key(Key::Character("c".into()), false, false), &mut None);
            app.handle_key(&key(Key::Named(NamedKey::Enter), false, false), cx);
            assert!(docs.join("c.txt").exists() && !docs.join("a.txt").exists());

            // Copy into the folder, then duplicate in place.
            app.select_names(&["b.txt"]);
            app.copy(false, cx);
            app.go(Location::Dir(docs.join("Z")), cx);
            app.paste();
            wait_for_jobs(app);
            assert_eq!(names(app), ["b.txt"]);
            assert_eq!(app.selected_paths(), [docs.join("Z/b.txt")]);
            app.duplicate();
            wait_for_jobs(app);
            assert_eq!(names(app), ["b copy.txt", "b.txt"]);
            app.go_back(cx);
            assert_eq!(app.current_dir(), Some(docs.as_path()));
            assert_eq!(app.selected_paths(), [docs.join("Z")]);

            // Move to Trash, see it there, put it back.
            app.select_names(&["c.txt"]);
            app.trash_selection();
            assert!(!docs.join("c.txt").exists());
            assert!(home.join(".local/share/Trash/info/c.txt.trashinfo").exists());
            app.go(Location::Trash, cx);
            assert_eq!(names(app), ["c.txt"]);
            app.select_all();
            app.put_back();
            assert!(docs.join("c.txt").exists());
            assert!(app.entries.is_empty());

            // Empty Trash asks first.
            app.go(Location::Dir(docs.clone()), cx);
            app.select_names(&["b.txt"]);
            app.handle_key(&key(Key::Named(NamedKey::Delete), false, false), cx);
            app.handle_key(&key(Key::Named(NamedKey::Backspace), true, true), cx);
            assert!(matches!(app.dialog, Some(Dialog::EmptyTrash)));
            app.handle_key(&key(Key::Named(NamedKey::Enter), false, false), cx);
            let start = Instant::now();
            while app.emptying.as_ref().is_some_and(|d| !d.load(Ordering::Relaxed)) && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(app.trash_is_empty());

            // Sorting by size, biggest first.
            app.set_sort(SortKey::Size);
            assert_eq!(names(app), ["c.txt", "Z"]);

            // Quick Look follows the selection.
            app.select_names(&["c.txt"]);
            app.toggle_quicklook();
            assert!(matches!(app.quicklook.as_ref().and_then(|q| q.preview.as_ref()), Some(Preview::Text(t)) if t == "a"));
            app.handle_key(&key(Key::Named(NamedKey::ArrowDown), false, false), cx);
            assert_eq!(app.quicklook.as_ref().map(|q| q.path.clone()), Some(docs.join("Z")));
            app.handle_key(&key(Key::Named(NamedKey::Space), false, false), cx);
            assert!(app.quicklook.is_none());

            // Column view: right enters the folder, left comes back.
            app.set_view(ViewMode::Columns);
            app.select_names(&["Z"]);
            app.handle_key(&key(Key::Named(NamedKey::ArrowRight), false, false), cx);
            assert_eq!(app.current_dir(), Some(docs.join("Z").as_path()));
            // Sorted by size, biggest first; equal sizes fall back to name.
            assert_eq!(app.selected_paths(), [docs.join("Z/b.txt")]);
            app.handle_key(&key(Key::Named(NamedKey::ArrowLeft), false, false), cx);
            assert_eq!(app.selected_paths(), [docs.join("Z")]);

            // The context menu from the keyboard: type to jump, arrows, Return.
            app.background_menu(cx);
            app.handle_key(&key(Key::Character("a".into()), false, false), cx);
            app.handle_key(&key(Key::Named(NamedKey::ArrowDown), false, false), cx);
            app.handle_key(&key(Key::Named(NamedKey::Enter), false, false), cx);
            assert!(app.menu.is_none());
            assert_eq!(app.view, ViewMode::List);
            // Escape closes it without doing anything; keys don't reach the list meanwhile.
            app.background_menu(cx);
            app.handle_key(&key(Key::Named(NamedKey::Delete), false, false), cx);
            app.handle_key(&key(Key::Named(NamedKey::Escape), false, false), cx);
            assert!(app.menu.is_none());
            assert_eq!(app.view, ViewMode::List);
        });
        assert!(png.exists());
        let _ = std::fs::remove_file(png);
        std::fs::remove_dir_all(&home).unwrap();
    }
}
