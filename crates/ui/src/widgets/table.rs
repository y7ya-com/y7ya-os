//! A sortable table with a fixed header and a virtualized body: rows have one
//! fixed height, and only those in view are built each frame, so tables with
//! thousands of rows stay cheap to lay out.

use std::cell::Cell as StdCell;
use std::hash::Hash;
use std::rc::Rc;

use crate::cx::Cx;
use crate::element::{AnyElement, Div, ElementId, Handler, Styled, canvas, div, eid, text};
use crate::geom::{Bounds, Color, pt};
use crate::scene::Scene;
use crate::text::Weight;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

#[derive(Clone)]
pub struct Column {
    pub title: String,
    /// A total shown above the title, like Task Manager's "12%" over "CPU".
    pub summary: Option<String>,
    /// Background behind the header cell (a heat-map tint for the summary).
    pub header_bg: Option<Color>,
    /// Fixed width, or `None` to share the remaining space.
    pub width: Option<f32>,
    pub align: Align,
}

impl Column {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), summary: None, header_bg: None, width: None, align: Align::Left }
    }
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }
    pub fn right(mut self) -> Self {
        self.align = Align::Right;
        self
    }
    pub fn summary(mut self, s: impl Into<String>, bg: Option<Color>) -> Self {
        self.summary = Some(s.into());
        self.header_bg = bg;
        self
    }
}

pub struct Cell<S> {
    pub content: AnyElement<S>,
    /// Cell background (heat-map shading); hidden while the row is selected.
    pub bg: Option<Color>,
}

impl<S> Cell<S> {
    pub fn new(content: impl Into<AnyElement<S>>) -> Self {
        Self { content: content.into(), bg: None }
    }
    pub fn bg(mut self, c: Option<Color>) -> Self {
        self.bg = c;
        self
    }
}

pub struct Row<S> {
    pub key: u64,
    pub cells: Vec<Cell<S>>,
    pub selected: bool,
    /// A full-width group heading (its first cell spans the row).
    pub section: bool,
    on_mouse_down: Option<Handler<S>>,
    on_right_click: Option<Handler<S>>,
    on_double_click: Option<Handler<S>>,
}

impl<S> Row<S> {
    pub fn new(key: impl Hash, cells: Vec<Cell<S>>) -> Self {
        Self { key: eid(key), cells, selected: false, section: false, on_mouse_down: None, on_right_click: None, on_double_click: None }
    }
    /// A group heading spanning all columns.
    pub fn section(key: impl Hash, content: impl Into<AnyElement<S>>) -> Self {
        Self { section: true, ..Self::new(key, vec![Cell::new(content)]) }
    }
    pub fn selected(mut self, v: bool) -> Self {
        self.selected = v;
        self
    }
    pub fn on_mouse_down(mut self, f: impl Fn(&mut S, &mut Cx) + 'static) -> Self {
        self.on_mouse_down = Some(Rc::new(f));
        self
    }
    pub fn on_right_click(mut self, f: impl Fn(&mut S, &mut Cx) + 'static) -> Self {
        self.on_right_click = Some(Rc::new(f));
        self
    }
    pub fn on_double_click(mut self, f: impl Fn(&mut S, &mut Cx) + 'static) -> Self {
        self.on_double_click = Some(Rc::new(f));
        self
    }
}

#[derive(Clone, Copy)]
pub struct TableStyle {
    pub row_h: f32,
    pub text: Color,
    pub text_secondary: Color,
    pub divider: Color,
    pub hover: Color,
    pub selection: Color,
    pub selection_text: Color,
    /// Alternating row tint, macOS list style; `None` for plain rows.
    pub zebra: Option<Color>,
}

/// What a table remembers between frames: its scroll position (kept by `Cx`
/// under `id`), the measured viewport height and a pending scroll-into-view.
pub struct TableState {
    id: ElementId,
    viewport_h: Rc<StdCell<f32>>,
    reveal: StdCell<Option<usize>>,
}

impl TableState {
    pub fn new(id: impl Hash) -> Self {
        Self { id: eid(id), viewport_h: Rc::new(StdCell::new(0.0)), reveal: StdCell::new(None) }
    }

    /// Scroll row `index` into view on the next render.
    pub fn reveal(&self, index: usize) {
        self.reveal.set(Some(index));
    }

    pub fn scroll_to_top(&self, cx: &mut Cx) {
        cx.set_scroll_offset(self.id, 0.0);
    }

    /// Rows that fit in the viewport (for Page Up/Down).
    pub fn page_rows(&self, row_h: f32) -> usize {
        ((self.viewport_h.get() / row_h).floor() as usize).max(1)
    }
}

pub struct Table<'a, S> {
    state: &'a TableState,
    columns: &'a [Column],
    len: usize,
    sort: Option<(usize, bool)>,
    on_sort: Option<Rc<dyn Fn(&mut S, usize, &mut Cx)>>,
    style: TableStyle,
    header: bool,
}

const H_PAD: f32 = 8.0;
const ROW_INSET: f32 = 6.0;

impl<'a, S: 'static> Table<'a, S> {
    /// A table of `len` rows; rows are produced on demand by `render`'s callback.
    pub fn new(state: &'a TableState, columns: &'a [Column], len: usize, style: TableStyle) -> Self {
        Self { state, columns, len, sort: None, on_sort: None, style, header: true }
    }

    /// The sorted column and whether it's descending (draws the chevron).
    pub fn sort(mut self, sort: Option<(usize, bool)>) -> Self {
        self.sort = sort;
        self
    }

    /// A plain list: no header row.
    pub fn headerless(mut self) -> Self {
        self.header = false;
        self
    }

    /// Clicking a header calls `f` with the column index.
    pub fn on_sort(mut self, f: impl Fn(&mut S, usize, &mut Cx) + 'static) -> Self {
        self.on_sort = Some(Rc::new(f));
        self
    }

    /// Builds the table, calling `row` for each row in view.
    pub fn render(self, cx: &mut Cx, mut row: impl FnMut(usize, &mut Cx) -> Row<S>) -> Div<S> {
        let st = self.style;
        let id = self.state.id;
        let two_line = self.columns.iter().any(|c| c.summary.is_some());
        let header_h = if two_line { 44.0 } else { 28.0 };

        let mut header = div().h(header_h).shrink_0().flex_row().px(ROW_INSET).border_b(st.divider);
        for (i, col) in self.columns.iter().enumerate() {
            header = header.child(self.header_cell(i, col, cx));
        }

        // Pick the rows in view from the scroll offset of the previous frame.
        let row_h = st.row_h;
        let viewport = match self.state.viewport_h.get() {
            v if v > 0.0 => v,
            _ => cx.window_size.1,
        };
        let mut offset = cx.scroll_offset(id).min((self.len as f32 * row_h - viewport).max(0.0));
        if let Some(i) = self.state.reveal.take() {
            let top = i as f32 * row_h;
            if top < offset {
                offset = top;
            } else if top + row_h > offset + viewport {
                offset = top + row_h - viewport;
            }
            cx.set_scroll_offset(id, offset);
        }
        let first = ((offset / row_h).floor() as usize).min(self.len);
        let last = (first + (viewport / row_h).ceil() as usize + 2).min(self.len);

        let mut body = div().id(id).scroll_y().flex_1().flex_col().child(div().h(first as f32 * row_h).shrink_0());
        for i in first..last {
            let r = row(i, cx);
            body = body.child(self.row(i, r, cx));
        }
        body = body.child(div().h((self.len - last) as f32 * row_h + 4.0).shrink_0());

        let measured = self.state.viewport_h.clone();
        div()
            .flex_1()
            .flex_col()
            .min_w_0()
            .child_if(self.header, || header.into())
            .child(
                div()
                    .flex_1()
                    .flex_col()
                    .child(body)
                    .child(canvas(move |_, b| measured.set(b.h)).inset_0()),
            )
    }

    fn sized(&self, col: &Column, d: Div<S>) -> Div<S> {
        match col.width {
            Some(w) => d.w(w).shrink_0(),
            None => d.flex_1().min_w_0(),
        }
    }

    fn header_cell(&self, i: usize, col: &Column, cx: &Cx) -> Div<S> {
        let st = self.style;
        let sorted = self.sort.filter(|(c, _)| *c == i).map(|(_, desc)| desc);
        let right = col.align == Align::Right;
        let mut label = div().flex_row().items_center().gap(3.0);
        let title = text(col.title.clone()).size(11.0).weight(Weight::Medium).color(st.text_secondary).truncate();
        let chevron = sorted.map(|desc| {
            let c = st.text_secondary;
            canvas(move |s, b| sort_chevron(s, b, desc, c)).w(9.0).h(9.0).shrink_0()
        });
        if right {
            label = label.children(chevron).child(title);
        } else {
            label = label.child(title).children(chevron);
        }
        let mut cell = div()
            .h_full()
            .flex_col()
            .justify_center()
            .px(H_PAD)
            .gap(1.0)
            .bg_opt(col.header_bg);
        cell = if right { cell.items_end() } else { cell.items_start() };
        if let Some(s) = &col.summary {
            cell = cell.child(text(s.clone()).size(15.0).color(st.text).truncate());
        }
        cell = cell.child(label);
        if i > 0 {
            // Hairline column separator, macOS table style.
            cell = cell.child(div().absolute(0.0, 9.0).w(1.0).h(10.0).bg(st.divider));
        }
        if let Some(f) = self.on_sort.clone() {
            let hover = cx.hovered((self.state.id, "header", i));
            cell = cell
                .id((self.state.id, "header", i))
                .on_click(move |s: &mut S, cx: &mut Cx| f(s, i, cx))
                .bg_opt(if hover && col.header_bg.is_none() { Some(st.hover) } else { None });
        }
        self.sized(col, cell)
    }

    fn row(&self, i: usize, r: Row<S>, _cx: &Cx) -> Div<S> {
        let st = self.style;
        let mut d = div()
            .id((self.state.id, r.key))
            .h(st.row_h)
            .shrink_0()
            .flex_row()
            .items_center()
            .mx(ROW_INSET)
            .rounded(5.0);
        d.on_mouse_down = r.on_mouse_down;
        d.on_right_click = r.on_right_click;
        d.on_double_click = r.on_double_click;
        if r.selected {
            d = d.bg(st.selection).text_color(st.selection_text);
        } else if !r.section {
            d = d.hover_bg(st.hover);
            if let Some(z) = st.zebra.filter(|_| i % 2 == 1) {
                d = d.bg(z).hover_bg(st.hover);
            }
        }
        if r.section {
            let mut cells = r.cells.into_iter();
            if let Some(c) = cells.next() {
                d = d.child(div().flex_1().h_full().items_center().px(H_PAD).child(c.content));
            }
            return d;
        }
        for (col, cell) in self.columns.iter().zip(r.cells) {
            let mut c = div().h_full().flex_row().items_center().px(H_PAD);
            if col.align == Align::Right {
                c = c.justify_end();
            }
            if !r.selected {
                c = c.bg_opt(cell.bg);
            }
            d = d.child(self.sized(col, c).child(cell.content));
        }
        d
    }
}

fn sort_chevron(scene: &mut Scene, b: Bounds, descending: bool, color: Color) {
    let c = b.center();
    let (dx, dy) = (3.2, 1.8);
    let s = if descending { 1.0 } else { -1.0 };
    scene.segment(pt(c.x - dx, c.y - s * dy), pt(c.x, c.y + s * dy), 1.3, color);
    scene.segment(pt(c.x, c.y + s * dy), pt(c.x + dx, c.y - s * dy), 1.3, color);
}
