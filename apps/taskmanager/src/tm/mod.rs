//! The Task Manager window: Windows 11's layout (navigation rail, pages,
//! command bar) in macOS's visual language.

mod details;
mod format;
mod icons;
mod performance;
mod processes;
mod services;
mod startup;
mod theme;
mod users;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ui::widgets::table::{TableState, TableStyle};
use ui::*;

use crate::sys::Sampler;
use crate::sys::autostart::{Autostart, StartupEntry};
use crate::sys::kill::{GRACE, Reaper};
use crate::sys::services::{self as svc, Scope, Service};
use crate::sys::toplevel::Toplevels;
use performance::PerfItem;
use processes::{PCol, Sel};
use theme::Theme;

const SIDEBAR_W: f32 = 220.0;
const RAIL_W: f32 = 56.0;
const TOOLBAR_H: f32 = 52.0;
const ROW_H: f32 = 26.0;
/// Sampling period while the window has focus, and while it doesn't.
const ACTIVE_PERIOD: Duration = Duration::from_secs(1);
const IDLE_PERIOD: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Page {
    Processes,
    Performance,
    Startup,
    Users,
    Details,
    Services,
}

impl Page {
    const ALL: [Page; 6] = [Page::Processes, Page::Performance, Page::Startup, Page::Users, Page::Details, Page::Services];

    fn title(self) -> &'static str {
        match self {
            Page::Processes => "Processes",
            Page::Performance => "Performance",
            Page::Startup => "Startup apps",
            Page::Users => "Users",
            Page::Details => "Details",
            Page::Services => "Services",
        }
    }

    fn icon(self) -> fn(&mut Scene, Bounds, Color) {
        match self {
            Page::Processes => icons::processes,
            Page::Performance => icons::performance,
            Page::Startup => icons::startup,
            Page::Users => icons::users,
            Page::Details => icons::details,
            Page::Services => icons::services,
        }
    }

    pub fn parse(s: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.title().to_lowercase().starts_with(&s.to_lowercase()))
    }
}

#[derive(Clone)]
enum MenuAction {
    EndTask(Sel),
    EndTree(i32),
    OpenLocation(i32),
    Properties(i32),
    SetStartup(String, bool),
    Service(String, svc::Action),
}

struct MenuItem {
    label: String,
    /// `None` draws a separator.
    action: Option<MenuAction>,
    enabled: bool,
}

struct Menu {
    pos: Point,
    items: Vec<MenuItem>,
}

enum Sheet {
    Properties(i32),
    RunTask,
    Message(String, String),
}

pub struct TaskManager {
    sampler: Sampler,
    reaper: Reaper,
    page: Page,
    sidebar_open: bool,
    search: Rc<RefCell<TextInput>>,
    query: String,
    run_input: Rc<RefCell<TextInput>>,
    menu: Option<Menu>,
    sheet: Option<Sheet>,
    next_sample: Instant,
    focused: bool,

    expanded: HashSet<String>,
    proc_sel: Option<Sel>,
    proc_sort: (PCol, bool),
    proc_table: TableState,

    perf_item: PerfItem,

    autostart: Autostart,
    startup: Vec<StartupEntry>,
    startup_sel: Option<String>,
    startup_table: TableState,

    expanded_users: HashSet<u32>,
    user_sel: Option<users::USel>,
    user_table: TableState,

    detail_sel: Option<i32>,
    detail_sort: (details::DCol, bool),
    detail_table: TableState,

    services: Vec<Service>,
    services_inbox: Arc<Mutex<Option<Vec<Service>>>>,
    services_loading: bool,
    service_scope: Option<Scope>,
    service_sel: Option<String>,
    service_sort: (services::SCol, bool),
    service_table: TableState,
}

impl TaskManager {
    pub fn new(page: Page, cx: &mut Cx) -> Self {
        let waker = cx.waker();
        let toplevels = Toplevels::watch(move || waker.wake());
        let mut sampler = Sampler::new(toplevels);
        sampler.sample(Instant::now());

        let fonts = &mut cx.text.fonts;
        let mut search = TextInput::new(fonts, TextStyle::default(), true);
        search.placeholder = "Search".into();
        let mut run_input = TextInput::new(fonts, TextStyle::default(), true);
        run_input.placeholder = "Command, e.g. konsole".into();
        cx.window_radius = 10.0;

        let autostart = Autostart::standard();
        let mut app = Self {
            sampler,
            reaper: Reaper::new(GRACE),
            page,
            sidebar_open: true,
            search: Rc::new(RefCell::new(search)),
            query: String::new(),
            run_input: Rc::new(RefCell::new(run_input)),
            menu: None,
            sheet: None,
            // A quick second sample so CPU percentages appear right away.
            next_sample: Instant::now() + Duration::from_millis(400),
            focused: true,
            expanded: HashSet::new(),
            proc_sel: None,
            proc_sort: (PCol::Name, false),
            proc_table: TableState::new("proc-table"),
            perf_item: PerfItem::Cpu,
            startup: autostart.load(),
            autostart,
            startup_sel: None,
            startup_table: TableState::new("startup-table"),
            expanded_users: HashSet::new(),
            user_sel: None,
            user_table: TableState::new("user-table"),
            detail_sel: None,
            detail_sort: (details::DCol::Cpu, true),
            detail_table: TableState::new("detail-table"),
            services: Vec::new(),
            services_inbox: Arc::new(Mutex::new(None)),
            services_loading: false,
            service_scope: None,
            service_sel: None,
            service_sort: (services::SCol::Name, false),
            service_table: TableState::new("service-table"),
        };
        app.set_page(page, cx);
        cx.focus("table");
        app
    }

    fn theme(cx: &Cx) -> Theme {
        if cx.dark { Theme::dark() } else { Theme::light() }
    }

    fn sample(&mut self, now: Instant) {
        self.sampler.sample(now);
        self.reaper.tick(now);
    }

    fn set_page(&mut self, page: Page, cx: &mut Cx) {
        self.menu = None;
        self.page = page;
        match page {
            Page::Startup => self.startup = self.autostart.load(),
            Page::Services => self.refresh_services(cx),
            _ => {}
        }
        cx.focus("table");
    }

    fn refresh_services(&mut self, cx: &Cx) {
        if self.services_loading {
            return;
        }
        self.services_loading = true;
        let inbox = self.services_inbox.clone();
        let waker = cx.waker();
        std::thread::spawn(move || {
            *inbox.lock().unwrap() = Some(svc::list());
            waker.wake();
        });
    }

    fn query(&self) -> String {
        self.query.trim().to_lowercase()
    }

    fn clear_search(&mut self, cx: &mut Cx) {
        self.query.clear();
        self.search.borrow_mut().set_text(&mut cx.text.fonts, "");
    }

    fn show_error(&mut self, title: &str, body: String) {
        self.sheet = Some(Sheet::Message(title.into(), body));
    }

    /// Ends processes (SIGTERM, then SIGKILL after a grace period).
    fn end_pids(&mut self, pids: &[i32]) {
        let targets: Vec<_> = pids.iter().filter_map(|&p| self.sampler.target(p)).collect();
        if let Err(e) = self.reaper.terminate(&targets, Instant::now()) {
            self.show_error("Couldn't end the task", e);
        }
        // Resample soon so ended processes drop out of the list.
        self.next_sample = self.next_sample.min(Instant::now() + Duration::from_millis(300));
    }

    fn open_location(&mut self, pid: i32) {
        let dir = self.sampler.process(pid).and_then(|p| p.exe.as_ref()).and_then(|e| e.parent().map(|d| d.to_path_buf()));
        if let Some(dir) = dir {
            if let Err(e) = spawn_detached(std::process::Command::new("xdg-open").arg(dir)) {
                self.show_error("Couldn't open the file location", e.to_string());
            }
        }
    }

    fn run_task(&mut self, cx: &mut Cx) {
        let cmd = self.run_input.borrow().text().trim().to_string();
        self.sheet = None;
        cx.focus("table");
        if !cmd.is_empty() {
            if let Err(e) = spawn_detached(std::process::Command::new("sh").args(["-c", &cmd])) {
                self.show_error("Couldn't run the task", e.to_string());
            }
        }
    }

    fn run_menu(&mut self, action: MenuAction, cx: &mut Cx) {
        self.menu = None;
        match action {
            MenuAction::EndTask(sel) => self.end_selection(&sel),
            MenuAction::EndTree(pid) => {
                let tree = self.sampler.tree(pid);
                self.end_pids(&tree);
            }
            MenuAction::OpenLocation(pid) => self.open_location(pid),
            MenuAction::Properties(pid) => self.sheet = Some(Sheet::Properties(pid)),
            MenuAction::SetStartup(file, on) => self.set_startup(&file, on),
            MenuAction::Service(unit, action) => self.control_service(&unit, action, cx),
        }
    }

    // -- shared controls ----------------------------------------------------

    fn sync_input_styles(&self, t: &Theme) {
        for input in [&self.search, &self.run_input] {
            let mut i = input.borrow_mut();
            i.set_styles(TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: t.text }, None);
            i.placeholder_color = t.text_secondary;
            i.caret_color = t.accent;
            i.selection_color = t.text_selection;
        }
    }

    fn table_style(&self, t: &Theme, cx: &Cx) -> TableStyle {
        let active = cx.window_focused && cx.is_focused("table");
        TableStyle {
            row_h: ROW_H,
            text: t.text,
            text_secondary: t.text_secondary,
            divider: t.divider,
            hover: t.hover,
            selection: if active { t.accent } else { t.sidebar_selection.alpha(1.6) },
            selection_text: if active { Color::WHITE } else { t.text },
            zebra: None,
        }
    }

    /// A push button: accent-filled for the default action, gray otherwise.
    fn button(
        id: &'static str,
        label: &str,
        primary: bool,
        enabled: bool,
        t: &Theme,
        on_click: impl Fn(&mut Self, &mut Cx) + 'static,
    ) -> Div<Self> {
        let (bg, fg) = if primary { (t.accent, Color::WHITE) } else { (t.field, t.text) };
        let b = div()
            .id(id)
            .h(26.0)
            .px(12.0)
            .rounded(6.0)
            .items_center()
            .justify_center()
            .bg(if enabled { bg } else { bg.alpha(0.5) })
            .child(text(label.to_string()).color(if enabled { fg } else { fg.alpha(0.4) }).weight(Weight::Medium));
        if enabled { b.hover_bg(if primary { bg.alpha(0.85) } else { t.field.alpha(1.8) }).on_click(on_click) } else { b }
    }

    fn icon_button(
        id: &'static str,
        t: &Theme,
        icon: fn(&mut Scene, Bounds, Color),
        on_click: impl Fn(&mut Self, &mut Cx) + 'static,
    ) -> Div<Self> {
        let color = t.icon;
        div()
            .id(id)
            .w(32.0)
            .h(28.0)
            .rounded(6.0)
            .items_center()
            .justify_center()
            .hover_bg(t.hover.alpha(1.6))
            .on_click(on_click)
            .child(canvas(move |s, b| icon(s, b, color)).size(20.0))
    }

    /// A small colored tile with the app's initial, standing in for its icon.
    fn app_glyph(name: &str, size: f32) -> Div<Self> {
        const COLORS: [u32; 8] = [0x007aff, 0x34c759, 0xff9500, 0xaf52de, 0xff2d55, 0x5ac8fa, 0x5856d6, 0xa2845e];
        let c = COLORS[(eid(name) % COLORS.len() as u64) as usize];
        let initial = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        div()
            .size(size)
            .shrink_0()
            .rounded(size * 0.26)
            .bg(rgb(c))
            .items_center()
            .justify_center()
            .child(text(initial).size(size * 0.6).weight(Weight::Bold).color(Color::WHITE))
    }

    /// macOS segmented control.
    fn segmented<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: &[(&str, T)],
        current: T,
        t: &Theme,
        on_pick: fn(&mut Self, T, &mut Cx),
    ) -> Div<Self> {
        let mut row = div().flex_row().h(24.0).p(2.0).rounded(7.0).bg(t.field).gap(2.0);
        for (i, (label, value)) in options.iter().enumerate() {
            let value = *value;
            let on = value == current;
            let seg = div()
                .id((id, i))
                .h_full()
                .px(12.0)
                .rounded(5.0)
                .items_center()
                .justify_center()
                .on_click(move |s: &mut Self, cx: &mut Cx| on_pick(s, value, cx))
                .child(text(label.to_string()).size(12.0).weight(if on { Weight::Medium } else { Weight::Regular }));
            row = row.child(if on {
                seg.bg(if t.dark { rgba(0xffffff, 0.16) } else { Color::WHITE }).shadow(rgba(0x000000, 0.12), 2.0, 0.5)
            } else {
                seg.hover_bg(t.hover)
            });
        }
        row
    }

    /// macOS switch. `on_accent` when it sits on an accent-colored (selected) row.
    fn switch(
        id: impl std::hash::Hash,
        on: bool,
        on_accent: bool,
        t: &Theme,
        on_toggle: impl Fn(&mut Self, &mut Cx) + 'static,
    ) -> Div<Self> {
        let track = match (on, on_accent) {
            (true, true) => rgba(0xffffff, 0.45),
            (true, false) => t.accent,
            (false, true) => rgba(0x000000, 0.22),
            _ if t.dark => rgba(0xffffff, 0.18),
            _ => rgba(0x000000, 0.14),
        };
        div()
            .id(id)
            .w(30.0)
            .h(18.0)
            .shrink_0()
            .rounded(9.0)
            .bg(track)
            .p(2.0)
            .flex_row()
            .justify_end_if(on)
            .on_mouse_down(on_toggle)
            .child(div().size(14.0).rounded(7.0).bg(Color::WHITE).shadow(rgba(0x000000, 0.25), 1.5, 0.5))
    }

    fn window_buttons(&self, left: bool, cx: &Cx, t: &Theme) -> Option<Div<Self>> {
        ui::chrome::window_buttons(left, cx, t.text)
    }

    // -- layout -------------------------------------------------------------

    fn sidebar(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let open = self.sidebar_open;
        let mut header = div().h(TOOLBAR_H).shrink_0().flex_row().items_center().window_drag();
        header = if open {
            let mut h = header.justify_between().pl(20.0).pr(10.0);
            h = match self.window_buttons(true, cx, t) {
                Some(b) => h.child(b),
                None => h.child(text("Task Manager").weight(Weight::Bold)),
            };
            h.child(Self::icon_button("toggle-sidebar", t, icons::sidebar, |s, _| s.sidebar_open = false))
        } else {
            header.justify_center().child(Self::icon_button("toggle-sidebar", t, icons::sidebar, |s, _| s.sidebar_open = true))
        };

        let mut items = div().flex_col().gap(2.0).px(if open { 10.0 } else { 8.0 }).pt(4.0);
        for page in Page::ALL {
            let selected = self.page == page;
            let icon = page.icon();
            let color = if selected { t.accent } else { t.icon };
            let mut row = div()
                .id(("nav", page))
                .h(if open { 30.0 } else { 36.0 })
                .rounded(6.0)
                .flex_row()
                .items_center()
                .gap(9.0)
                .bg_opt(selected.then_some(t.sidebar_selection))
                .hover_bg(if selected { t.sidebar_selection } else { t.hover })
                .on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.set_page(page, cx))
                .child(canvas(move |s, b| icon(s, b, color)).size(18.0).shrink_0());
            row = if open {
                row.px(8.0).child(text(page.title()).weight(if selected { Weight::Medium } else { Weight::Regular }))
            } else {
                row.justify_center()
            };
            items = items.child(row);
        }

        // A glance at the machine, like the Windows status bar.
        let sys = &self.sampler.system;
        let footer = div()
            .flex_col()
            .gap(3.0)
            .px(20.0)
            .pb(14.0)
            .child(text(format!("CPU {:.0}%", sys.cpu.util.last())).size(11.0).color(t.text_secondary))
            .child(text(format!("Memory {:.0}%", sys.mem.used.last())).size(11.0).color(t.text_secondary));

        div()
            .w(if open { SIDEBAR_W } else { RAIL_W })
            .h_full()
            .shrink_0()
            .flex_col()
            .bg(t.sidebar)
            .child(header)
            .child(items.flex_1())
            .child_if(open, || footer.into())
    }

    fn toolbar(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let (title, subtitle) = self.page_title();
        let mut bar = div()
            .h(TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(8.0)
            .pl(if self.sidebar_open { 20.0 } else { 14.0 })
            .pr(12.0)
            .border_b(t.divider)
            .window_drag();
        if !self.sidebar_open {
            if let Some(b) = self.window_buttons(true, cx, t) {
                bar = bar.child(b.mr(10.0));
            }
        }
        bar = bar.child(
            div()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(text(title).size(15.0).weight(Weight::Bold).truncate())
                .child(text(subtitle).size(11.0).color(t.text_secondary).truncate()),
        );
        bar = bar.children(self.page_actions(t));
        bar.child(self.search_field(t)).children(self.window_buttons(false, cx, t))
    }

    fn page_title(&self) -> (String, String) {
        let sys = &self.sampler.system;
        let n = self.sampler.procs.iter().filter(|p| !p.kernel).count();
        let sub = match self.page {
            Page::Processes => format!("{n} processes · {:.0}% CPU · {:.0}% memory", sys.cpu.util.last(), sys.mem.used.last()),
            Page::Performance => format!("Up time {}", format::uptime(sys.uptime)),
            Page::Startup => {
                let on = self.startup.iter().filter(|e| e.enabled).count();
                format!("{} items · {on} enabled", self.startup.len())
            }
            Page::Users => format!("{} users", self.user_rows_count()),
            Page::Details => format!("{} processes and kernel threads", self.sampler.procs.len()),
            Page::Services => {
                if self.services_loading && self.services.is_empty() {
                    "Loading…".into()
                } else {
                    let running = self.services.iter().filter(|s| s.running()).count();
                    format!("{} services · {running} running", self.services.len())
                }
            }
        };
        (self.page.title().into(), sub)
    }

    fn page_actions(&self, t: &Theme) -> Vec<Div<Self>> {
        match self.page {
            Page::Processes => vec![
                Self::button("run-task", "Run new task", false, true, t, |s: &mut Self, cx: &mut Cx| {
                    s.run_input.borrow_mut().set_text(&mut cx.text.fonts, "");
                    s.sheet = Some(Sheet::RunTask);
                    cx.focus("run-input");
                }),
                Self::button("end-task", "End task", false, self.proc_sel.is_some(), t, |s: &mut Self, _| {
                    if let Some(sel) = s.proc_sel.clone() {
                        s.end_selection(&sel);
                    }
                }),
            ],
            Page::Details => vec![Self::button("end-task", "End task", false, self.detail_sel.is_some(), t, |s: &mut Self, _| {
                if let Some(pid) = s.detail_sel {
                    s.end_pids(&[pid]);
                }
            })],
            Page::Startup => {
                let sel = self.startup_sel.as_ref().and_then(|f| self.startup.iter().find(|e| &e.file == f));
                let (label, enable) = match sel {
                    Some(e) if e.enabled => ("Disable", false),
                    _ => ("Enable", true),
                };
                vec![Self::button("toggle-startup", label, false, sel.is_some(), t, move |s: &mut Self, _| {
                    if let Some(f) = s.startup_sel.clone() {
                        s.set_startup(&f, enable);
                    }
                })]
            }
            Page::Services => {
                let user_sel = self.selected_service().filter(|s| s.scope == Scope::User).map(|s| (s.unit.clone(), s.running()));
                let running = user_sel.as_ref().is_some_and(|(_, r)| *r);
                let can = user_sel.is_some();
                let act = |a: svc::Action| {
                    move |s: &mut Self, cx: &mut Cx| {
                        if let Some(unit) = s.selected_service().map(|x| x.unit.clone()) {
                            s.control_service(&unit, a, cx);
                        }
                    }
                };
                vec![
                    Self::button("svc-start", "Start", false, can && !running, t, act(svc::Action::Start)),
                    Self::button("svc-stop", "Stop", false, can && running, t, act(svc::Action::Stop)),
                    Self::button("svc-restart", "Restart", false, can && running, t, act(svc::Action::Restart)),
                ]
            }
            _ => Vec::new(),
        }
    }

    fn search_field(&self, t: &Theme) -> Div<Self> {
        let icon = t.icon;
        div()
            .id("search-box")
            .w(200.0)
            .h(28.0)
            .ml(6.0)
            .rounded(7.0)
            .bg(t.field)
            .flex_row()
            .items_center()
            .gap(6.0)
            .px(8.0)
            .cursor(CursorIcon::Text)
            .on_mouse_down(|_: &mut Self, cx: &mut Cx| cx.focus("search"))
            .child(canvas(move |s, b| icons::magnifier(s, b, icon)).size(14.0).shrink_0())
            .child(input("search", &self.search).flex_1())
            .child_if(!self.query.is_empty(), || {
                let c = t.text_secondary;
                div()
                    .id("search-clear")
                    .size(16.0)
                    .rounded(8.0)
                    .bg(c.alpha(0.5))
                    .shrink_0()
                    .on_click(|s: &mut Self, cx: &mut Cx| {
                        s.clear_search(cx);
                        cx.focus("table");
                    })
                    .child(canvas(|s, b| icons::xmark(s, b, Color::WHITE, 6.0, 1.4)).size(16.0))
                    .into()
            })
    }

    fn menu_overlay(&self, menu: &Menu, cx: &Cx, t: &Theme) -> Div<Self> {
        let (w, h) = cx.window_size;
        let width = 210.0;
        let est_h: f32 = 10.0 + menu.items.iter().map(|i| if i.action.is_some() { 24.0 } else { 9.0 }).sum::<f32>();
        let x = menu.pos.x.min(w - width - 6.0).max(6.0);
        let y = if menu.pos.y + est_h > h - 6.0 { (menu.pos.y - est_h).max(6.0) } else { menu.pos.y };
        let mut panel = div()
            .absolute(x, y)
            .w(width)
            .flex_col()
            .p(5.0)
            .rounded(8.0)
            .bg(t.menu_bg)
            .border(t.menu_border, 0.5)
            .shadow(t.shadow, 14.0, 6.0)
            .occlude();
        for (i, item) in menu.items.iter().enumerate() {
            match &item.action {
                None => panel = panel.child(div().h(1.0).mt(4.0).mb(4.0).px(6.0).child(div().flex_1().h(1.0).bg(t.menu_border))),
                Some(action) => {
                    let mut row = div().id(("menu-item", i)).h(24.0).px(10.0).rounded(5.0).items_center();
                    let color = if !item.enabled {
                        t.text_tertiary
                    } else if cx.hovered(("menu-item", i)) {
                        Color::WHITE
                    } else {
                        t.text
                    };
                    if item.enabled {
                        let action = action.clone();
                        row = row.hover_bg(t.accent).on_click(move |s: &mut Self, cx: &mut Cx| s.run_menu(action.clone(), cx));
                    }
                    panel = panel.child(row.child(text(item.label.clone()).color(color).truncate()));
                }
            }
        }
        div()
            .inset_0()
            .deferred()
            .on_mouse_down(|s: &mut Self, _| s.menu = None)
            .on_right_click(|s: &mut Self, _| s.menu = None)
            .child(panel)
    }

    fn sheet_overlay(&self, sheet: &Sheet, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let card = match sheet {
            Sheet::Properties(pid) => self.properties_card(*pid, t),
            Sheet::RunTask => self.run_task_card(t),
            Sheet::Message(title, body) => div()
                .w(360.0)
                .flex_col()
                .items_center()
                .gap(10.0)
                .child(text(title.clone()).size(15.0).weight(Weight::Bold))
                .child(text(body.clone()).color(t.text_secondary).line_height(1.4))
                .child(Self::button("sheet-ok", "OK", true, true, t, |s: &mut Self, _| s.sheet = None).w(96.0).mt(6.0)),
        };
        div()
            .inset_0()
            .deferred()
            .bg(rgba(0x000000, if cx.dark { 0.45 } else { 0.22 }))
            .items_center()
            .justify_center()
            .on_mouse_down(|s: &mut Self, _| s.sheet = None)
            .child(
                card.px(24.0)
                    .pt(20.0)
                    .pb(18.0)
                    .rounded(12.0)
                    .bg(t.content)
                    .border(t.menu_border, 0.5)
                    .shadow(t.shadow, 30.0, 10.0)
                    .occlude(),
            )
    }

    fn run_task_card(&self, t: &Theme) -> Div<Self> {
        div()
            .w(400.0)
            .flex_col()
            .gap(12.0)
            .child(text("Run new task").size(15.0).weight(Weight::Bold))
            .child(text("Type the name of a program or command, and it will start in the background.").color(t.text_secondary).line_height(1.4))
            .child(
                div()
                    .id("run-box")
                    .h(28.0)
                    .px(8.0)
                    .rounded(7.0)
                    .bg(t.field)
                    .border(t.accent.alpha(0.6), 1.5)
                    .items_center()
                    .cursor(CursorIcon::Text)
                    .on_mouse_down(|_: &mut Self, cx: &mut Cx| cx.focus("run-input"))
                    .child(input("run-input", &self.run_input).flex_1()),
            )
            .child(
                div()
                    .flex_row()
                    .justify_end()
                    .gap(8.0)
                    .mt(4.0)
                    .child(Self::button("run-cancel", "Cancel", false, true, t, |s: &mut Self, cx: &mut Cx| {
                        s.sheet = None;
                        cx.focus("table");
                    }))
                    .child(Self::button("run-ok", "Run", true, true, t, |s: &mut Self, cx: &mut Cx| s.run_task(cx))),
            )
    }
}

/// Moves a selection `delta` rows through `keys`, skipping rows that can't be
/// selected (`None`). Returns the new row index and key.
fn step<K: Clone + PartialEq>(keys: &[Option<K>], cur: Option<&K>, delta: i32) -> Option<(usize, K)> {
    let selectable: Vec<usize> = (0..keys.len()).filter(|&i| keys[i].is_some()).collect();
    if selectable.is_empty() {
        return None;
    }
    let pos = cur.and_then(|c| selectable.iter().position(|&i| keys[i].as_ref() == Some(c)));
    let next = match pos {
        Some(p) => (p as i32 + delta).clamp(0, selectable.len() as i32 - 1) as usize,
        None if delta < 0 => selectable.len() - 1,
        None => 0,
    };
    let i = selectable[next];
    Some((i, keys[i].clone()?))
}

trait JustifyIf {
    fn justify_end_if(self, cond: bool) -> Self;
}

impl<S> JustifyIf for Div<S> {
    fn justify_end_if(self, cond: bool) -> Self {
        if cond { self.justify_end() } else { self }
    }
}

fn spawn_detached(cmd: &mut std::process::Command) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // Own process group, so it outlives us and ignores our terminal's signals.
    let mut child = cmd.process_group(0).spawn()?;
    // Reap it in the background so it doesn't linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

impl App for TaskManager {
    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Self::theme(cx).text }
    }

    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        let t = Self::theme(cx);
        self.sync_input_styles(&t);
        // Coming back into focus: catch up on the slower background cadence now.
        if cx.window_focused && !self.focused {
            self.next_sample = self.next_sample.min(Instant::now() + Duration::from_millis(100));
        }
        self.focused = cx.window_focused;

        let (w, h) = cx.window_size;
        let page = match self.page {
            Page::Processes => self.processes_page(cx, &t),
            Page::Performance => self.performance_page(cx, &t),
            Page::Startup => self.startup_page(cx, &t),
            Page::Users => self.users_page(cx, &t),
            Page::Details => self.details_page(cx, &t),
            Page::Services => self.services_page(cx, &t),
        };
        let main = div().flex_1().h_full().min_w_0().flex_col().bg(t.content).child(self.toolbar(cx, &t)).child(page);
        let mut root = div()
            .w(w)
            .h(h)
            .flex_row()
            .bg(t.content)
            .text_color(t.text)
            .child(self.sidebar(cx, &t))
            .child(div().w(1.0).h_full().shrink_0().bg(t.divider))
            .child(main);
        if let Some(menu) = &self.menu {
            root = root.child(self.menu_overlay(menu, cx, &t));
        }
        if let Some(sheet) = &self.sheet {
            root = root.child(self.sheet_overlay(sheet, cx, &t));
        }
        if cx.transparent && !cx.maximized && !ui::is_y7ya() {
            root = root.child(div().inset_0().rounded(cx.window_radius).border(t.window_border, 1.0));
        }
        root
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        if let Some(sheet) = &self.sheet {
            let run = matches!(sheet, Sheet::RunTask);
            if key.is_named(NamedKey::Escape) {
                self.sheet = None;
                cx.focus("table");
                return true;
            }
            if key.is_named(NamedKey::Enter) {
                if run {
                    self.run_task(cx);
                } else {
                    self.sheet = None;
                }
                return true;
            }
            // Typing goes to the command field; nothing else reacts.
            return !run;
        }
        if key.is_named(NamedKey::Escape) {
            if self.menu.take().is_none() {
                if cx.is_focused("search") || !self.query.is_empty() {
                    self.clear_search(cx);
                }
                cx.focus("table");
            }
            return true;
        }
        if self.menu.is_some() {
            return true;
        }
        if key.ctrl && !key.alt {
            if key.is_char('f') {
                self.search.borrow_mut().select_all();
                cx.focus("search");
                return true;
            }
            if key.is_char('q') || key.is_char('w') {
                cx.close_window();
                return true;
            }
            if let Some(d) = ['1', '2', '3', '4', '5', '6'].iter().position(|c| key.is_char(*c)) {
                self.set_page(Page::ALL[d], cx);
                return true;
            }
            if key.is_char('r') && self.page == Page::Services {
                self.refresh_services(cx);
                return true;
            }
        }
        if cx.is_focused("search") {
            if key.is_named(NamedKey::Enter) || key.is_named(NamedKey::ArrowDown) {
                cx.focus("table");
                return true;
            }
            return false;
        }
        match self.page {
            Page::Processes => self.processes_key(key, cx),
            Page::Performance => self.performance_key(key),
            Page::Startup => self.startup_key(key),
            Page::Users => self.users_key(key),
            Page::Details => self.details_key(key),
            Page::Services => self.services_key(key, cx),
        }
    }

    fn on_input(&mut self, id: ElementId, cx: &mut Cx) {
        if id == eid("search") {
            self.query = self.search.borrow().text();
            for table in [&self.proc_table, &self.detail_table, &self.user_table, &self.startup_table, &self.service_table] {
                table.scroll_to_top(cx);
            }
        }
    }

    fn next_wake(&self) -> Option<Instant> {
        Some(self.reaper.next_deadline().map_or(self.next_sample, |d| d.min(self.next_sample)))
    }

    fn on_wake(&mut self, cx: &mut Cx) {
        let now = Instant::now();
        if now >= self.next_sample {
            self.sample(now);
            self.next_sample = now + if cx.window_focused { ACTIVE_PERIOD } else { IDLE_PERIOD };
        } else {
            self.reaper.tick(now);
        }
    }

    fn on_external(&mut self, _cx: &mut Cx) {
        if let Some(list) = self.services_inbox.lock().unwrap().take() {
            self.services = list;
            self.services_loading = false;
        }
        // The window list changed: regroup with the latest sample.
        self.sampler.regroup();
    }
}

impl TaskManager {
    /// Takes a sample immediately (for previews).
    pub fn on_wake_now(&mut self, _cx: &mut Cx) {
        self.sample(Instant::now());
    }

    /// Puts the window in an interesting state for `--snapshot` renders.
    pub fn preview_setup(&mut self, perf: Option<&str>, menu: bool, cx: &mut Cx) {
        self.perf_item = match perf {
            Some("memory") => PerfItem::Memory,
            Some("disk") => PerfItem::Disk(0),
            Some("network") => PerfItem::Net(0),
            Some("gpu") => PerfItem::Gpu,
            _ => PerfItem::Cpu,
        };
        if let Some(g) = self.sampler.grouping.apps.iter().max_by_key(|g| g.pids.len()) {
            self.expanded.insert(g.key.clone());
            self.proc_sel = Some(Sel::App(g.key.clone()));
        }
        let me = unsafe { libc::getuid() };
        self.expanded_users.insert(me);
        self.user_sel = Some(users::USel::User(me));
        self.detail_sel = self.sampler.procs.iter().max_by(|a, b| a.cpu.total_cmp(&b.cpu)).map(|p| p.pid);
        self.startup_sel = self.startup.first().map(|e| e.file.clone());
        if self.page == Page::Services {
            // Load synchronously so the render has data.
            self.services = svc::list();
            self.services_loading = false;
            self.service_sel = self.services.iter().find(|s| s.scope == Scope::User && s.running()).map(|s| s.unit.clone());
        }
        if menu {
            cx.mouse = Point { x: 420.0, y: 210.0 };
            if let Some(sel) = self.proc_sel.clone() {
                self.process_menu(sel, cx);
            }
        }
    }
}

/// Fills graph histories with plausible values, for `--snapshot` renders.
pub fn demo_history(app: &mut TaskManager) {
    let wave = |i: usize, base: f32, amp: f32, phase: f32| {
        let x = i as f32 / 7.0 + phase;
        (base + amp * (x.sin() * 0.6 + (x * 2.7).sin() * 0.25 + (x * 6.1).cos() * 0.15)).max(0.0)
    };
    let sys = &mut app.sampler.system;
    for i in 0..crate::sys::system::HISTORY {
        sys.cpu.util.push(wave(i, 18.0, 14.0, 0.0));
        sys.mem.used.push(wave(i, 46.0, 2.0, 1.0));
        for d in &mut sys.disks {
            d.active.push(wave(i, 3.0, 4.0, 2.0));
            d.read.push(wave(i, 2.0e6, 2.5e6, 0.5));
            d.write.push(wave(i, 1.0e6, 1.5e6, 3.0));
        }
        for n in &mut sys.nets {
            n.rx.push(wave(i, 180e3, 200e3, 1.3));
            n.tx.push(wave(i, 40e3, 50e3, 2.2));
        }
        if let Some(g) = &mut sys.gpu {
            g.util.push(wave(i, 9.0, 8.0, 0.7));
        }
    }
}
