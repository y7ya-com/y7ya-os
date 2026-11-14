mod icons;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use media::pdf;
use media::{Kind, Loader};
use ui::*;

const TOOLBAR_H: f32 = 52.0;
const SIDEBAR_W: f32 = 170.0;
const THUMB_W: f32 = 120.0;
const THUMB_ROW_H: f32 = 168.0;
const THUMB_PX: u32 = 256;
/// Images bigger than this are shown scaled down.
const MAX_IMAGE_PX: u32 = 6144;
const ZOOM_STEPS: [f32; 13] = [0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 8.0];
/// Scrolling past the end of a page this far turns to the next one.
const PAGE_TURN: f32 = 140.0;

/// One thing in the sidebar: an image file, or a page of a PDF.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub path: PathBuf,
    /// 1-based page number for PDFs.
    pub pdf_page: Option<u32>,
}

/// Which pages to show for the given command-line paths: a folder's images,
/// an image's siblings, every page of a PDF. Returns the pages and which to
/// open first.
pub fn pages_for(paths: &[PathBuf]) -> (Vec<Page>, usize) {
    let is_image = |p: &Path| p.file_name().is_some_and(|n| media::kind_of(&n.to_string_lossy(), false) == Kind::Image);
    let images_in = |dir: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|it| it.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_image(p) && !hidden(p)).collect())
            .unwrap_or_default();
        v.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
        v
    };
    let mut pages = Vec::new();
    let mut first = 0;
    if let [single] = paths {
        if single.is_dir() {
            return (images_in(single).into_iter().map(|path| Page { path, pdf_page: None }).collect(), 0);
        }
        if is_image(single) {
            // Like browsing a folder: the image's neighbors come along.
            let siblings = single.parent().map(images_in).unwrap_or_default();
            if let Some(i) = siblings.iter().position(|p| p == single) {
                return (siblings.into_iter().map(|path| Page { path, pdf_page: None }).collect(), i);
            }
        }
    }
    for (i, p) in paths.iter().enumerate() {
        if i == 0 {
            first = pages.len();
        }
        if is_pdf(p) {
            let n = pdf::info(p).map_or(1, |i| i.pages.max(1));
            pages.extend((1..=n).map(|k| Page { path: p.clone(), pdf_page: Some(k) }));
        } else if p.is_dir() {
            pages.extend(images_in(p).into_iter().map(|path| Page { path, pdf_page: None }));
        } else {
            pages.push(Page { path: p.clone(), pdf_page: None });
        }
    }
    (pages, first)
}

fn hidden(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

fn is_pdf(p: &Path) -> bool {
    p.file_name().is_some_and(|n| media::kind_of(&n.to_string_lossy(), false) == Kind::Pdf)
}

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    // Good enough here: case-insensitive, numbers by length then value.
    let key = |s: &str| {
        let mut out = String::new();
        let mut digits = String::new();
        for c in s.chars().chain(std::iter::once('\0')) {
            if c.is_ascii_digit() {
                digits.push(c);
                continue;
            }
            if !digits.is_empty() {
                out.push_str(&format!("{:0>12}", digits));
                digits.clear();
            }
            out.extend(c.to_lowercase());
        }
        out
    };
    key(a).cmp(&key(b))
}

#[derive(Clone, Copy)]
struct Theme {
    window: Color,
    canvas: Color,
    sidebar: Color,
    divider: Color,
    text: Color,
    text_secondary: Color,
    icon: Color,
    icon_disabled: Color,
    icon_hover_bg: Color,
    accent: Color,
    window_border: Color,
}

impl Theme {
    fn get(dark: bool) -> Theme {
        if dark {
            Theme {
                window: rgb(0x2a2a2c),
                canvas: rgb(0x1e1e1e),
                sidebar: rgb(0x2a2a2c),
                divider: rgba(0xffffff, 0.10),
                text: rgba(0xffffff, 0.85),
                text_secondary: rgba(0xffffff, 0.55),
                icon: rgba(0xffffff, 0.66),
                icon_disabled: rgba(0xffffff, 0.22),
                icon_hover_bg: rgba(0xffffff, 0.08),
                accent: rgb(0x0a84ff),
                window_border: rgba(0xffffff, 0.14),
            }
        } else {
            Theme {
                window: rgb(0xf2f2f4),
                canvas: rgb(0xe6e6e8),
                sidebar: rgb(0xf2f2f4),
                divider: rgba(0x000000, 0.10),
                text: rgba(0x000000, 0.85),
                text_secondary: rgba(0x000000, 0.50),
                icon: rgba(0x000000, 0.62),
                icon_disabled: rgba(0x000000, 0.22),
                icon_hover_bg: rgba(0x000000, 0.06),
                accent: rgb(0x007aff),
                window_border: rgba(0x000000, 0.22),
            }
        }
    }
}

enum Slot {
    Pending,
    Ready(Image),
    Failed,
}

/// Key for a decoded page: its index and the pixel size it was rendered at
/// (0 for image files, which decode once at full size).
type PageKey = (usize, u32);

pub struct PreviewApp {
    pages: Vec<Page>,
    current: usize,
    pdf_info: HashMap<PathBuf, pdf::Info>,
    /// `None` fits the page to the window.
    zoom: Option<f32>,
    pan: Point,
    turns: u32,
    sidebar_open: bool,
    full: HashMap<PageKey, Slot>,
    full_loader: Loader<PageKey, Option<Image>>,
    thumbs: HashMap<usize, Option<Image>>,
    thumb_loader: Loader<usize, Option<Image>>,
    drag_from: Option<Point>,
    overscroll: f32,
}

impl PreviewApp {
    pub fn new(pages: Vec<Page>, first: usize, inline: bool, cx: &mut Cx) -> Self {
        let mut full_loader = Loader::new(2, cx.waker());
        let mut thumb_loader = Loader::new(2, cx.waker());
        full_loader.set_inline(inline);
        thumb_loader.set_inline(inline);
        let mut pdf_info = HashMap::new();
        for p in &pages {
            if p.pdf_page.is_some() && !pdf_info.contains_key(&p.path) {
                if let Some(i) = pdf::info(&p.path) {
                    pdf_info.insert(p.path.clone(), i);
                }
            }
        }
        cx.window_radius = 10.0;
        let sidebar_open = pages.len() > 1;
        let mut app = Self {
            current: first.min(pages.len().saturating_sub(1)),
            pages,
            pdf_info,
            zoom: None,
            pan: Point::default(),
            turns: 0,
            sidebar_open,
            full: HashMap::new(),
            full_loader,
            thumbs: HashMap::new(),
            thumb_loader,
            drag_from: None,
            overscroll: 0.0,
        };
        app.reveal_thumb(cx);
        app
    }

    /// For snapshots: turn the page and zoom (`None` fits).
    pub fn set_view(&mut self, turns: u32, zoom: Option<f32>, cx: &mut Cx) {
        self.turns = turns % 4;
        if let Some(z) = zoom {
            // The image has to be loaded to know its size.
            self.page_image(self.current, 1024);
            self.set_zoom(z, None, cx);
        }
    }

    pub fn window_title(&self) -> String {
        self.pages.get(self.current).and_then(|p| p.path.file_name()).map_or("Preview".into(), |n| n.to_string_lossy().into_owned())
    }

    // -- geometry -------------------------------------------------------------

    fn viewer_rect(&self, cx: &Cx) -> Bounds {
        let (w, h) = cx.window_size;
        let x = if self.show_sidebar() { SIDEBAR_W + 1.0 } else { 0.0 };
        Bounds::new(x, TOOLBAR_H + 1.0, (w - x).max(1.0), (h - TOOLBAR_H - 1.0).max(1.0))
    }

    fn show_sidebar(&self) -> bool {
        self.sidebar_open && self.pages.len() > 1
    }

    /// The current page's size in points, unrotated.
    fn natural_size(&self) -> Option<(f32, f32)> {
        let page = self.pages.get(self.current)?;
        if page.pdf_page.is_some() {
            return self.pdf_info.get(&page.path).map(|i| i.page_size);
        }
        // Image pixels map to points 1:1, as in macOS Preview.
        match self.full.get(&(self.current, 0)) {
            Some(Slot::Ready(img)) => Some((img.width() as f32, img.height() as f32)),
            _ => None,
        }
    }

    fn rotated(&self, (w, h): (f32, f32)) -> (f32, f32) {
        if self.turns % 2 == 1 { (h, w) } else { (w, h) }
    }

    fn fit_scale(&self, size: (f32, f32), view: Bounds, is_pdf: bool) -> f32 {
        let (w, h) = self.rotated(size);
        let margin = if is_pdf { 32.0 } else { 0.0 };
        let s = ((view.w - margin) / w).min((view.h - margin) / h);
        // Small images aren't blown up to fill the window.
        if is_pdf { s } else { s.min(1.0) }
    }

    fn current_is_pdf(&self) -> bool {
        self.pages.get(self.current).is_some_and(|p| p.pdf_page.is_some())
    }

    fn scale(&self, cx: &Cx) -> Option<f32> {
        let size = self.natural_size()?;
        Some(self.zoom.unwrap_or_else(|| self.fit_scale(size, self.viewer_rect(cx), self.current_is_pdf())))
    }

    /// Where the page lands on screen (rotated), with the pan clamped so
    /// it can't be dragged out of view.
    fn page_rect(&mut self, cx: &Cx) -> Option<Bounds> {
        let size = self.natural_size()?;
        let s = self.scale(cx)?;
        let view = self.viewer_rect(cx);
        let (w, h) = self.rotated(size);
        let (dw, dh) = (w * s, h * s);
        let max_x = ((dw - view.w) / 2.0 + 16.0).max(0.0);
        let max_y = ((dh - view.h) / 2.0 + 16.0).max(0.0);
        self.pan.x = if dw + 32.0 <= view.w { 0.0 } else { self.pan.x.clamp(-max_x, max_x) };
        self.pan.y = if dh + 32.0 <= view.h { 0.0 } else { self.pan.y.clamp(-max_y, max_y) };
        let c = view.center();
        Some(Bounds::new(c.x - dw / 2.0 + self.pan.x, c.y - dh / 2.0 + self.pan.y, dw, dh))
    }

    // -- navigation -----------------------------------------------------------

    fn go_to(&mut self, i: usize, cx: &mut Cx) {
        if i >= self.pages.len() || i == self.current {
            return;
        }
        let same_file = self.pages[i].path == self.pages[self.current].path;
        self.current = i;
        self.pan = Point::default();
        self.overscroll = 0.0;
        if !same_file {
            // A new image starts fitted and upright.
            self.zoom = None;
            self.turns = 0;
        }
        self.reveal_thumb(cx);
    }

    fn step(&mut self, delta: isize, cx: &mut Cx) {
        let i = (self.current as isize + delta).clamp(0, self.pages.len() as isize - 1) as usize;
        self.go_to(i, cx);
    }

    fn reveal_thumb(&mut self, cx: &mut Cx) {
        let view_h = cx.window_size.1 - TOOLBAR_H;
        let y = self.current as f32 * THUMB_ROW_H;
        let off = cx.scroll_y("thumbs");
        if y < off {
            cx.set_scroll_y("thumbs", y);
        } else if y + THUMB_ROW_H > off + view_h {
            cx.set_scroll_y("thumbs", y + THUMB_ROW_H - view_h);
        }
    }

    // -- zoom -----------------------------------------------------------------

    /// Zooms to `s`, keeping the point under `anchor` (or the center) still.
    fn set_zoom(&mut self, s: f32, anchor: Option<Point>, cx: &Cx) {
        let Some(old) = self.scale(cx) else { return };
        let s = s.clamp(ZOOM_STEPS[0], ZOOM_STEPS[ZOOM_STEPS.len() - 1]);
        let c = self.viewer_rect(cx).center();
        let m = anchor.unwrap_or(c);
        let k = s / old;
        self.pan = Point { x: m.x - c.x - (m.x - c.x - self.pan.x) * k, y: m.y - c.y - (m.y - c.y - self.pan.y) * k };
        self.zoom = Some(s);
    }

    fn zoom_step(&mut self, up: bool, cx: &Cx) {
        let Some(cur) = self.scale(cx) else { return };
        let next = if up {
            ZOOM_STEPS.iter().copied().find(|z| *z > cur * 1.01)
        } else {
            ZOOM_STEPS.iter().rev().copied().find(|z| *z < cur * 0.99)
        };
        if let Some(z) = next {
            self.set_zoom(z, None, cx);
        }
    }

    fn rotate(&mut self, clockwise: bool) {
        self.turns = (self.turns + if clockwise { 1 } else { 3 }) % 4;
        self.pan = Point::default();
    }

    // -- loading --------------------------------------------------------------

    /// The best decoded image for page `i` at `want_px`, requesting a sharper
    /// one when needed. Falls back to the thumbnail meanwhile.
    fn page_image(&mut self, i: usize, want_px: u32) -> Option<Image> {
        let page = self.pages.get(i)?.clone();
        let bucket = if page.pdf_page.is_some() { want_px.clamp(256, 4096).next_power_of_two().min(4096) } else { 0 };
        let key = (i, bucket);
        if !self.full.contains_key(&key) {
            self.full.insert(key, Slot::Pending);
            self.full_loader.request(key, move || match page.pdf_page {
                Some(n) => pdf::render_page(&page.path, n, bucket),
                None => Image::load(&page.path, MAX_IMAGE_PX),
            });
            self.take_results();
        }
        if let Some(Slot::Ready(img)) = self.full.get(&key) {
            return Some(img.clone());
        }
        // Meanwhile: the sharpest render of this page we already have.
        let best = self
            .full
            .iter()
            .filter_map(|((p, b), slot)| match slot {
                Slot::Ready(img) if *p == i => Some((*b, img)),
                _ => None,
            })
            .max_by_key(|(b, _)| *b);
        best.map(|(_, img)| img.clone()).or_else(|| self.thumb(i))
    }

    fn thumb(&mut self, i: usize) -> Option<Image> {
        if let Some(t) = self.thumbs.get(&i) {
            return t.clone();
        }
        let page = self.pages.get(i)?.clone();
        self.thumbs.insert(i, None);
        self.thumb_loader.request(i, move || match page.pdf_page {
            Some(n) => pdf::render_page(&page.path, n, THUMB_PX),
            None => Image::load(&page.path, THUMB_PX),
        });
        self.take_results();
        self.thumbs.get(&i).cloned().flatten()
    }

    fn take_results(&mut self) {
        for (key, img) in self.full_loader.take_results() {
            self.full.insert(key, img.map_or(Slot::Failed, Slot::Ready));
        }
        for (i, img) in self.thumb_loader.take_results() {
            self.thumbs.insert(i, img);
        }
        // Keep only the pages near the current one at full size.
        let cur = self.current;
        if self.full.len() > 12 {
            self.full.retain(|(i, _), _| i.abs_diff(cur) <= 1);
        }
    }

    /// Whether page `i` couldn't be decoded at all.
    fn failed(&self, i: usize) -> bool {
        let mut slots = self.full.iter().filter(|((p, _), _)| *p == i).map(|(_, s)| s);
        slots.clone().any(|s| matches!(s, Slot::Failed)) && !slots.any(|s| matches!(s, Slot::Ready(_)))
    }

    // -- rendering --------------------------------------------------------------

    fn tool_button(id: &'static str, t: &Theme, enabled: bool, icon: fn(&mut Scene, Bounds, Color), on_click: impl Fn(&mut Self, &mut Cx) + 'static) -> Div<Self> {
        let color = if enabled { t.icon } else { t.icon_disabled };
        let mut d = div().id(id).w(32.0).h(28.0).rounded(6.0).items_center().justify_center().child(canvas(move |s, b| icon(s, b, color)).size(18.0));
        if enabled {
            d = d.hover_bg(t.icon_hover_bg).on_click(on_click);
        }
        d
    }

    fn toolbar(&mut self, cx: &Cx, t: &Theme) -> Div<Self> {
        let has_page = self.natural_size().is_some();
        let pct = self.scale(cx).map(|s| format!("{}%", (s * 100.0).round()));
        let subtitle = match self.pages.get(self.current) {
            Some(Page { pdf_page: Some(n), path }) => {
                let total = self.pdf_info.get(path).map_or(1, |i| i.pages);
                format!("Page {n} of {total}")
            }
            Some(_) if self.pages.len() > 1 => format!("{} of {}", self.current + 1, self.pages.len()),
            _ => String::new(),
        };
        let subtitle = match (subtitle.is_empty(), pct) {
            (false, Some(p)) => format!("{subtitle} \u{2013} {p}"),
            (true, Some(p)) => p,
            (_, None) => subtitle,
        };
        let mut bar = div().h(TOOLBAR_H).shrink_0().flex_row().items_center().gap(6.0).pl(20.0).pr(12.0).bg(t.window).border_b(t.divider).window_drag();
        if let Some(b) = ui::chrome::window_buttons(true, cx, t.text) {
            bar = bar.child(b.mr(12.0));
        }
        if self.pages.len() > 1 {
            bar = bar.child(Self::tool_button("toggle-sidebar", t, true, icons::sidebar, |s, _| s.sidebar_open = !s.sidebar_open));
        }
        bar = bar.child(
            div()
                .flex_col()
                .flex_1()
                .min_w_0()
                .pl(6.0)
                .child(text(self.window_title()).weight(Weight::Bold).truncate())
                .child_if(!subtitle.is_empty(), || text(subtitle.clone()).size(11.0).color(t.text_secondary).truncate().into()),
        );
        let multi = self.pages.len() > 1;
        if multi {
            bar = bar
                .child(Self::tool_button("prev", t, self.current > 0, icons::chevron_left, |s, cx| s.step(-1, cx)))
                .child(Self::tool_button("next", t, self.current + 1 < self.pages.len(), icons::chevron_right, |s, cx| s.step(1, cx)))
                .child(div().w(1.0).h(18.0).bg(t.divider).mx(4.0));
        }
        bar = bar
            .child(Self::tool_button("zoom-out", t, has_page, icons::zoom_out, |s, cx| s.zoom_step(false, cx)))
            .child(Self::tool_button("zoom-in", t, has_page, icons::zoom_in, |s, cx| s.zoom_step(true, cx)))
            .child(Self::tool_button("fit", t, has_page && self.zoom.is_some(), icons::fit, |s, _| {
                s.zoom = None;
                s.pan = Point::default();
            }))
            .child(Self::tool_button("actual-size", t, has_page, |sc, b, c| actual_size_icon(sc, b, c), |s, cx| s.set_zoom(1.0, None, cx)))
            .child(div().w(1.0).h(18.0).bg(t.divider).mx(4.0))
            .child(Self::tool_button("rotate-left", t, has_page, icons::rotate_left, |s, _| s.rotate(false)))
            .child(Self::tool_button("rotate-right", t, has_page, icons::rotate_right, |s, _| s.rotate(true)));
        bar.children(ui::chrome::window_buttons(false, cx, t.text))
    }

    fn sidebar(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let view_h = cx.window_size.1 - TOOLBAR_H;
        let scroll = cx.scroll_y("thumbs");
        let n = self.pages.len();
        let first = ((scroll / THUMB_ROW_H).floor().max(0.0) as usize).min(n);
        let last = (((scroll + view_h) / THUMB_ROW_H).ceil() as usize + 1).min(n);
        let mut list = div().flex_col().pt(first as f32 * THUMB_ROW_H).pb((n - last) as f32 * THUMB_ROW_H);
        let pdf = self.current_is_pdf();
        for i in first..last {
            let selected = i == self.current;
            let thumb = self.thumb(i);
            let label = match self.pages[i].pdf_page {
                Some(k) => k.to_string(),
                None => self.pages[i].path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            };
            let accent = t.accent;
            let img: AnyElement<Self> = match thumb {
                Some(img) => canvas(move |s, b| {
                    let (r, _) = ui::fit_rect(b, (img.width() as f32, img.height() as f32), Fit::Scale);
                    if selected {
                        s.rect_bordered(r.inset(-4.0), Color::TRANSPARENT, 5.0, accent, 3.0);
                    }
                    s.shadow(Bounds::new(r.x, r.y + 1.0, r.w, r.h), rgba(0x000000, 0.25), 1.0, 3.0);
                    s.image(r, &img, if pdf { 0.0 } else { 2.0 });
                })
                .w(THUMB_W)
                .h(THUMB_ROW_H - 40.0)
                .into(),
                None => div().w(THUMB_W).h(THUMB_ROW_H - 40.0).into(),
            };
            let (pill, fg) = if selected { (Some(t.accent), Color::WHITE) } else { (None, t.text_secondary) };
            list = list.child(
                div()
                    .id(("thumb", i))
                    .h(THUMB_ROW_H)
                    .flex_col()
                    .items_center()
                    .pt(14.0)
                    .gap(8.0)
                    .on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.go_to(i, cx))
                    .child(img)
                    .child(div().px(7.0).h(18.0).max_w(THUMB_W + 20.0).rounded(9.0).items_center().bg_opt(pill).child(text(label).size(11.0).color(fg).truncate())),
            );
        }
        div()
            .w(SIDEBAR_W)
            .h_full()
            .shrink_0()
            .flex_col()
            .bg(t.sidebar)
            .child(div().id("thumbs").scroll_y().flex_1().flex_col().child(list))
    }

    fn viewer(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let mut area = div()
            .id("viewer")
            .focusable()
            .flex_1()
            .h_full()
            .bg(t.canvas)
            .clip()
            .cursor(if self.zoom.is_some() { CursorIcon::Grab } else { CursorIcon::Default })
            .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.drag_from = Some(cx.mouse))
            .on_double_click(|s: &mut Self, cx: &mut Cx| {
                // Double-click toggles between fit and actual size.
                if s.zoom.is_some() {
                    s.zoom = None;
                    s.pan = Point::default();
                } else {
                    let m = cx.mouse;
                    s.set_zoom(1.0, Some(m), cx);
                }
            });
        let Some(page) = self.pages.get(self.current).cloned() else {
            return area.items_center().justify_center().child(text("No Image").size(15.0).color(t.text_secondary));
        };
        if page.pdf_page.is_some() && pdf::backend().is_none() {
            return area.items_center().justify_center().child(message(
                t,
                "PDFs need a renderer",
                "Install poppler (pdftoppm) or MuPDF (mutool) to view PDF pages.",
            ));
        }
        let rect = self.page_rect(cx);
        let want = rect.map_or(1024, |r| (r.w.max(r.h) * cx.scale) as u32);
        // This also starts loading an image, whose size isn't known before.
        let img = self.page_image(self.current, want);
        let rect = rect.or_else(|| self.page_rect(cx));
        match (rect, img) {
            (Some(r), img) => {
                let turns = self.turns;
                let pdf = page.pdf_page.is_some();
                area = area.child(canvas(move |s, _| {
                    if pdf {
                        s.shadow(Bounds::new(r.x, r.y + 2.0, r.w, r.h), rgba(0x000000, 0.25), 0.0, 6.0);
                        s.rect(r, Color::WHITE, 0.0);
                    }
                    if let Some(img) = &img {
                        s.image_with(r, img, ImageStyle { turns, ..Default::default() });
                    }
                }));
            }
            (None, _) if self.failed(self.current) => {
                area = area.items_center().justify_center().child(message(t, "Can\u{2019}t open this file", "It may be damaged or in a format Preview doesn\u{2019}t read."));
            }
            (None, _) => {}
        }
        area
    }
}

fn message(t: &Theme, title: &str, detail: &str) -> Div<PreviewApp> {
    let c = t.text_secondary;
    div()
        .flex_col()
        .items_center()
        .gap(8.0)
        .max_w(360.0)
        .child(canvas(move |s, b| icons::broken_page(s, b, c)).size(48.0))
        .child(text(title.to_string()).size(15.0).weight(Weight::Semibold))
        .child(text(detail.to_string()).size(12.0).color(t.text_secondary))
}

/// "1:1" drawn with strokes: actual size.
fn actual_size_icon(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let h = b.h.min(b.w) * 0.28;
    for x in [c.x - h * 1.1, c.x + h * 1.1] {
        scene.segment(pt(x, c.y - h), pt(x, c.y + h), 1.6, color);
        scene.segment(pt(x - h * 0.45, c.y - h * 0.6), pt(x, c.y - h), 1.4, color);
    }
    for dy in [-0.35, 0.35] {
        scene.rect(Bounds::new(c.x - 0.9, c.y + dy * h * 1.4 - 0.9, 1.8, 1.8), color, 0.9);
    }
}

impl App for PreviewApp {
    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Theme::get(cx.dark).text }
    }

    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        let t = Theme::get(cx.dark);
        let (w, h) = cx.window_size;
        let mut body = div().flex_1().min_h(0.0).flex_row();
        if self.show_sidebar() {
            body = body.child(self.sidebar(cx, &t)).child(div().w(1.0).h_full().bg(t.divider));
        }
        body = body.child(self.viewer(cx, &t));
        let mut root = div().w(w).h(h).flex_col().bg(t.window).text_color(t.text).child(self.toolbar(cx, &t)).child(body);
        if cx.transparent && !cx.maximized && !ui::is_y7ya() {
            root = root.child(div().inset_0().rounded(cx.window_radius).border(t.window_border, 1.0));
        }
        root
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        if key.ctrl {
            if key.is_char('=') || key.is_char('+') {
                self.zoom_step(true, cx);
            } else if key.is_char('-') {
                self.zoom_step(false, cx);
            } else if key.is_char('0') {
                self.set_zoom(1.0, None, cx);
            } else if key.is_char('9') {
                self.zoom = None;
                self.pan = Point::default();
            } else if key.is_char('r') {
                self.rotate(true);
            } else if key.is_char('l') {
                self.rotate(false);
            } else if key.is_char('w') || key.is_char('q') {
                cx.close_window();
            } else if key.is_char('o') {
                if let Some(p) = self.pages.get(self.current) {
                    // Show it in the default app (Files can be that for folders).
                    let _ = std::process::Command::new("xdg-open").arg(p.path.parent().unwrap_or(Path::new("/"))).spawn();
                }
            } else {
                return false;
            }
            return true;
        }
        let pan_step = 60.0;
        let zoomed = self.zoom.is_some();
        match &key.key {
            Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::PageDown) | Key::Named(NamedKey::Space) if !zoomed || key.is_named(NamedKey::PageDown) => self.step(1, cx),
            Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::PageUp) if !zoomed || key.is_named(NamedKey::PageUp) => self.step(-1, cx),
            Key::Named(NamedKey::ArrowDown) if !zoomed => self.step(1, cx),
            Key::Named(NamedKey::ArrowUp) if !zoomed => self.step(-1, cx),
            Key::Named(NamedKey::ArrowRight) => self.pan.x -= pan_step,
            Key::Named(NamedKey::ArrowLeft) => self.pan.x += pan_step,
            Key::Named(NamedKey::ArrowDown) => self.pan.y -= pan_step,
            Key::Named(NamedKey::ArrowUp) => self.pan.y += pan_step,
            Key::Named(NamedKey::Home) => self.go_to(0, cx),
            Key::Named(NamedKey::End) => self.go_to(self.pages.len().saturating_sub(1), cx),
            Key::Named(NamedKey::Escape) if zoomed => {
                self.zoom = None;
                self.pan = Point::default();
            }
            _ => return false,
        }
        true
    }

    fn on_external(&mut self, _cx: &mut Cx) {
        self.take_results();
    }

    fn on_mouse_move(&mut self, cx: &mut Cx) {
        if let Some(from) = self.drag_from.filter(|_| cx.mouse_down) {
            self.pan.x += cx.mouse.x - from.x;
            self.pan.y += cx.mouse.y - from.y;
            self.drag_from = Some(cx.mouse);
            cx.request_redraw();
        }
    }

    fn on_mouse_up(&mut self, _cx: &mut Cx) {
        self.drag_from = None;
    }

    fn on_scroll(&mut self, delta: Point, cx: &mut Cx) -> bool {
        let view = self.viewer_rect(cx);
        if !view.contains(cx.mouse) {
            return false;
        }
        // Ctrl+scroll (and touchpad pinch, which arrives as Ctrl+scroll on
        // many setups) zooms around the pointer.
        if cx.modifiers.control_key() {
            if let Some(s) = self.scale(cx) {
                let factor = (1.0 + delta.y * 0.004).clamp(0.5, 2.0);
                let m = cx.mouse;
                self.set_zoom(s * factor, Some(m), cx);
            }
            cx.request_redraw();
            return true;
        }
        let before = self.pan;
        self.pan.x += delta.x;
        self.pan.y += delta.y;
        self.page_rect(cx);
        // At the edge of the page (or when it all fits), keep scrolling to turn it.
        if (self.pan.y - before.y).abs() < 0.5 && delta.y.abs() > delta.x.abs() {
            self.overscroll += delta.y;
            if self.overscroll.abs() > PAGE_TURN {
                let dir = if self.overscroll < 0.0 { 1 } else { -1 };
                self.overscroll = 0.0;
                self.step(dir, cx);
            }
        } else {
            self.overscroll = 0.0;
        }
        cx.request_redraw();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_and_pages() {
        let mut v = vec!["img10.png", "img2.png", "Img1.png"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["Img1.png", "img2.png", "img10.png"]);

        let dir = std::env::temp_dir().join(format!("preview-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["b.png", "a.jpg", "notes.txt", ".hidden.png"] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        let (pages, first) = pages_for(&[dir.join("b.png")]);
        let names: Vec<String> = pages.iter().map(|p| p.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["a.jpg", "b.png"]);
        assert_eq!(first, 1);
        assert_eq!(pages_for(&[dir.clone()]).0.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
