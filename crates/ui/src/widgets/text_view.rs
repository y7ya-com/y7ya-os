//! A scrolling plain-text document editor for long texts (TextEdit and the
//! like). Unlike `TextInput`, only the lines in view get shaped, edits splice
//! the changed lines in place and undo records just what changed, so files of
//! several megabytes stay responsive. It also highlights find matches.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use cosmic_text::{
    Action, Attrs, AttrsList, Buffer, BufferLine, Cursor, Edit, Editor, Family, FontSystem, LineEnding, Motion,
    Scroll, Selection, Shaping, Wrap,
};
use winit::keyboard::{Key, NamedKey};

use crate::cx::{CARET_BLINK_MS, Cx};
use crate::geom::{Bounds, Color, Point};
use crate::input::KeyPress;
use crate::scene::Scene;
use crate::text::{FONT_FAMILY, TextStyle, draw_buffer};
use crate::widget::Widget;

const MAX_UNDO: usize = 1000;
const SCROLLBAR_HIT: f32 = 14.0;

#[derive(Clone, Copy, PartialEq)]
enum EditKind {
    Typing,
    Deleting,
    Other,
}

/// One undoable change: `removed` was replaced by `inserted` at `at`.
struct Step {
    /// Identifies the document state after this step (see [`TextView::state_id`]).
    id: u64,
    at: Cursor,
    removed: String,
    inserted: String,
    cursor_before: Cursor,
}

pub struct TextView {
    editor: Editor<'static>,
    style: TextStyle,
    family: &'static str,
    wrap: bool,
    /// Space between the edges and the text (x, y).
    pub inset: (f32, f32),
    pub selection_color: Color,
    pub inactive_selection_color: Color,
    pub caret_color: Color,
    pub match_color: Color,
    pub current_match_color: Color,
    pub scrollbar_color: Color,
    undo: Vec<Step>,
    redo: Vec<Step>,
    next_id: u64,
    last_edit: (EditKind, Instant),
    last_activity: Instant,
    /// The find query and whether it's case-sensitive.
    query: Option<(String, bool)>,
    /// Find matches in document order, each within one line.
    matches: Vec<(Cursor, Cursor)>,
    matches_stale: bool,
    /// Size of the text area last painted.
    size: (f32, f32),
    /// Scrollbar thumb (top, height) in local coordinates, when scrollable.
    thumb: Option<(f32, f32)>,
    /// Where the thumb was grabbed, while dragging it.
    thumb_grab: Option<f32>,
}

impl TextView {
    pub fn new(fonts: &mut FontSystem, style: TextStyle) -> Self {
        let mut buffer = Buffer::new(fonts, style.metrics());
        buffer.set_wrap(Wrap::WordOrGlyph);
        Self {
            editor: Editor::new(buffer),
            style,
            family: FONT_FAMILY,
            wrap: true,
            inset: (8.0, 6.0),
            selection_color: Color::BLACK.alpha(0.2),
            inactive_selection_color: Color::BLACK.alpha(0.1),
            caret_color: Color::BLACK,
            match_color: Color { r: 1.0, g: 0.9, b: 0.0, a: 0.35 },
            current_match_color: Color { r: 1.0, g: 0.85, b: 0.0, a: 0.9 },
            scrollbar_color: Color::BLACK.alpha(0.3),
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 1,
            last_edit: (EditKind::Other, Instant::now()),
            last_activity: Instant::now(),
            query: None,
            matches: Vec::new(),
            matches_stale: false,
            size: (0.0, 0.0),
            thumb: None,
            thumb_grab: None,
        }
    }

    fn attrs(&self) -> Attrs<'static> {
        Attrs::new().family(Family::Name(self.family)).weight(self.style.attrs().weight)
    }

    fn restyle(&mut self) {
        let attrs = AttrsList::new(&self.attrs());
        let metrics = self.style.metrics();
        self.editor.with_buffer_mut(|b| {
            b.set_metrics(metrics);
            for line in &mut b.lines {
                line.set_attrs_list(attrs.clone());
            }
        });
    }

    pub fn set_style(&mut self, style: TextStyle) {
        if self.style != style {
            let reshape = self.style.size != style.size || self.style.weight != style.weight || self.style.line_height != style.line_height;
            self.style = style;
            if reshape {
                self.restyle();
            }
        }
    }

    /// Switches between Inter and the system's monospaced font.
    pub fn set_monospace(&mut self, fonts: &FontSystem, on: bool) {
        let family = if on { mono_family(fonts) } else { FONT_FAMILY };
        if family != self.family {
            self.family = family;
            self.restyle();
        }
    }

    pub fn is_monospace(&self) -> bool {
        self.family != FONT_FAMILY
    }

    pub fn set_wrap(&mut self, wrap: bool) {
        if wrap != self.wrap {
            self.wrap = wrap;
            self.editor.with_buffer_mut(|b| {
                b.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
                let s = b.scroll();
                b.set_scroll(Scroll::new(s.line, s.vertical, 0.0));
            });
            self.editor.set_cursor(self.editor.cursor());
        }
    }

    pub fn wraps(&self) -> bool {
        self.wrap
    }

    // -- content ------------------------------------------------------------

    /// Replaces the whole document, putting the caret at the start and
    /// clearing undo history.
    pub fn set_text(&mut self, text: &str) {
        let attrs = self.attrs();
        self.editor.with_buffer_mut(|b| {
            b.set_text(text, &attrs, Shaping::Advanced, None);
            b.set_scroll(Scroll::default());
        });
        self.editor.set_selection(Selection::None);
        self.editor.set_cursor(Cursor::new(0, 0));
        self.undo.clear();
        self.redo.clear();
        self.matches_stale = true;
        self.last_activity = Instant::now();
    }

    /// The document with `\n` line endings.
    pub fn text(&self) -> String {
        self.editor.with_buffer(|b| {
            let mut out = String::with_capacity(b.lines.iter().map(|l| l.text().len() + 1).sum());
            for (i, line) in b.lines.iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(line.text());
            }
            out
        })
    }

    pub fn line_count(&self) -> usize {
        self.editor.with_buffer(|b| b.lines.len())
    }

    pub fn is_empty(&self) -> bool {
        self.editor.with_buffer(|b| b.lines.len() == 1 && b.lines[0].text().is_empty())
    }

    /// Changes whenever the text does and returns to an earlier value when
    /// undo gets back to that text, so apps can tell whether a document
    /// differs from what they saved.
    pub fn state_id(&self) -> u64 {
        self.undo.last().map_or(0, |s| s.id)
    }

    /// 1-based line and column (in characters) of the caret.
    pub fn line_col(&self) -> (usize, usize) {
        let c = self.editor.cursor();
        let col = self.editor.with_buffer(|b| b.lines.get(c.line).map_or(0, |l| l.text()[..c.index.min(l.text().len())].chars().count()));
        (c.line + 1, col + 1)
    }

    pub fn cursor(&self) -> Cursor {
        self.editor.cursor()
    }

    pub fn selection(&self) -> Option<(Cursor, Cursor)> {
        self.editor.selection_bounds().filter(|(a, b)| a != b)
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().and_then(|_| self.editor.copy_selection())
    }

    /// Selects `start..end` and scrolls it into view.
    pub fn select(&mut self, start: Cursor, end: Cursor) {
        self.editor.set_selection(Selection::Normal(start));
        self.editor.set_cursor(end);
        self.last_activity = Instant::now();
    }

    pub fn select_all(&mut self) {
        let end = self.end_cursor();
        self.select(Cursor::new(0, 0), end);
    }

    fn end_cursor(&self) -> Cursor {
        self.editor.with_buffer(|b| {
            let line = b.lines.len() - 1;
            Cursor::new(line, b.lines[line].text().len())
        })
    }

    /// Inserts `text` over the selection, as one undo step.
    pub fn insert(&mut self, text: &str) {
        let (start, end) = self.selection().unwrap_or((self.editor.cursor(), self.editor.cursor()));
        self.edit(start, end, &normalize(text), EditKind::Other);
    }

    // -- editing ------------------------------------------------------------

    /// Replaces `start..end` with `text` in the buffer, returning what was
    /// there. Only the touched lines are rebuilt.
    fn splice(&mut self, start: Cursor, end: Cursor, text: &str) -> String {
        let attrs = AttrsList::new(&self.attrs());
        self.editor.with_buffer_mut(|b| {
            let mut removed = String::new();
            for i in start.line..=end.line {
                let t = b.lines[i].text();
                let from = if i == start.line { start.index } else { 0 };
                let to = if i == end.line { end.index } else { t.len() };
                if i > start.line {
                    removed.push('\n');
                }
                removed.push_str(&t[from..to]);
            }
            let joined = format!("{}{}{}", &b.lines[start.line].text()[..start.index], text, &b.lines[end.line].text()[end.index..]);
            let ending = b.lines[end.line].ending();
            if start.line == end.line && !text.contains('\n') {
                b.lines[start.line].set_text(joined, ending, attrs);
            } else {
                let parts: Vec<&str> = joined.split('\n').collect();
                let n = parts.len();
                let lines = parts.into_iter().enumerate().map(|(i, p)| {
                    BufferLine::new(p, if i + 1 == n { ending } else { LineEnding::Lf }, attrs.clone(), Shaping::Advanced)
                });
                b.lines.splice(start.line..=end.line, lines);
            }
            removed
        })
    }

    /// Replaces `start..end` with `text`, recording undo and moving the caret
    /// after the new text. Runs of typing or deleting merge into one step.
    fn edit(&mut self, start: Cursor, end: Cursor, text: &str, kind: EditKind) {
        if start == end && text.is_empty() {
            return;
        }
        let before = self.editor.cursor();
        let removed = self.splice(start, end, text);
        self.editor.set_selection(Selection::None);
        self.editor.set_cursor(end_of(start, text));

        let now = Instant::now();
        let (last_kind, at) = self.last_edit;
        let recent = kind != EditKind::Other && kind == last_kind && now - at < Duration::from_secs(1);
        let id = self.next_id;
        self.next_id += 1;
        let merged = match self.undo.last_mut() {
            Some(last) if recent => {
                if kind == EditKind::Typing && removed.is_empty() && end_of(last.at, &last.inserted) == start {
                    last.inserted.push_str(text);
                    true
                } else if kind == EditKind::Deleting && text.is_empty() && last.inserted.is_empty() && end == last.at {
                    last.at = start;
                    last.removed.insert_str(0, &removed);
                    true
                } else if kind == EditKind::Deleting && text.is_empty() && last.inserted.is_empty() && start == last.at {
                    last.removed.push_str(&removed);
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        match self.undo.last_mut() {
            Some(last) if merged => last.id = id,
            _ => {
                self.undo.push(Step { id, at: start, removed, inserted: text.to_string(), cursor_before: before });
                if self.undo.len() > MAX_UNDO {
                    self.undo.remove(0);
                }
            }
        }
        self.redo.clear();
        self.last_edit = (kind, now);
        self.matches_stale = true;
        self.last_activity = now;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else { return false };
        self.splice(step.at, end_of(step.at, &step.inserted), &step.removed);
        let c = self.clamp(step.cursor_before);
        self.editor.set_selection(Selection::None);
        self.editor.set_cursor(c);
        self.redo.push(step);
        self.after_history();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(step) = self.redo.pop() else { return false };
        self.splice(step.at, end_of(step.at, &step.removed), &step.inserted);
        self.editor.set_selection(Selection::None);
        self.editor.set_cursor(end_of(step.at, &step.inserted));
        self.undo.push(step);
        self.after_history();
        true
    }

    fn after_history(&mut self) {
        self.last_edit = (EditKind::Other, Instant::now());
        self.matches_stale = true;
        self.last_activity = Instant::now();
    }

    fn clamp(&self, c: Cursor) -> Cursor {
        self.editor.with_buffer(|b| {
            let line = c.line.min(b.lines.len() - 1);
            let text = b.lines[line].text();
            let mut index = c.index.min(text.len());
            while !text.is_char_boundary(index) {
                index -= 1;
            }
            Cursor::new(line, index)
        })
    }

    /// Where `motion` would take the caret, without moving it.
    fn target(&mut self, fonts: &mut FontSystem, motion: Motion) -> Cursor {
        let (cursor, selection) = (self.editor.cursor(), self.editor.selection());
        self.editor.set_selection(Selection::None);
        self.editor.action(fonts, Action::Motion(motion));
        let target = self.editor.cursor();
        self.editor.set_cursor(cursor);
        self.editor.set_selection(selection);
        target
    }

    fn motion(&mut self, fonts: &mut FontSystem, motion: Motion, extend: bool) {
        if extend {
            if self.editor.selection() == Selection::None {
                let c = self.editor.cursor();
                self.editor.set_selection(Selection::Normal(c));
            }
        } else if let Some((start, end)) = self.editor.selection_bounds() {
            // Collapse the selection toward the direction of travel, like macOS.
            self.editor.set_selection(Selection::None);
            match motion {
                Motion::Left | Motion::Up | Motion::LeftWord | Motion::PreviousWord => {
                    self.editor.set_cursor(start);
                    return;
                }
                Motion::Right | Motion::Down | Motion::RightWord | Motion::NextWord => {
                    self.editor.set_cursor(end);
                    return;
                }
                _ => {}
            }
        }
        self.editor.action(fonts, Action::Motion(motion));
    }

    /// Deletes the selection, or from the caret to where `motion` goes.
    fn delete(&mut self, fonts: &mut FontSystem, motion: Motion) -> bool {
        let (start, end) = match self.selection() {
            Some(s) => s,
            None => {
                let c = self.editor.cursor();
                let t = self.target(fonts, motion);
                if (t.line, t.index) < (c.line, c.index) { (t, c) } else { (c, t) }
            }
        };
        if start == end {
            return false;
        }
        self.edit(start, end, "", EditKind::Deleting);
        true
    }

    fn type_text(&mut self, text: &str, kind: EditKind) {
        let (start, end) = self.selection().unwrap_or((self.editor.cursor(), self.editor.cursor()));
        let kind = if start != end { EditKind::Other } else { kind };
        self.edit(start, end, text, kind);
    }

    // -- find ---------------------------------------------------------------

    /// Sets (or clears, with an empty query) what to highlight and find.
    pub fn set_query(&mut self, query: &str, case_sensitive: bool) {
        let q = (!query.is_empty() && !query.contains('\n')).then(|| (query.to_string(), case_sensitive));
        if q != self.query {
            self.query = q;
            self.matches_stale = true;
        }
    }

    fn refresh_matches(&mut self) {
        if !self.matches_stale {
            return;
        }
        self.matches_stale = false;
        self.matches.clear();
        let Some((query, case)) = &self.query else { return };
        let needle = if *case { query.clone() } else { fold(query) };
        let matches = &mut self.matches;
        self.editor.with_buffer(|b| {
            for (i, line) in b.lines.iter().enumerate() {
                for (s, e) in find_in(line.text(), &needle, *case) {
                    matches.push((Cursor::new(i, s), Cursor::new(i, e)));
                }
            }
        });
    }

    pub fn match_count(&mut self) -> usize {
        self.refresh_matches();
        self.matches.len()
    }

    /// Index of the match that is currently selected, if any.
    pub fn current_match(&mut self) -> Option<usize> {
        self.refresh_matches();
        let (s, e) = self.selection()?;
        self.matches.binary_search_by(|m| pos(m.0).cmp(&pos(s))).ok().filter(|&i| pos(self.matches[i].1) == pos(e))
    }

    /// Selects the next (or previous) match after the selection, wrapping
    /// around. Returns false if there are none.
    pub fn find_next(&mut self, forward: bool) -> bool {
        self.refresh_matches();
        if self.matches.is_empty() {
            return false;
        }
        let c = self.editor.cursor();
        let (s, e) = self.selection().unwrap_or((c, c));
        let i = if forward {
            let i = self.matches.partition_point(|m| pos(m.0) < pos(e));
            if i == self.matches.len() { 0 } else { i }
        } else {
            let i = self.matches.partition_point(|m| pos(m.1) <= pos(s));
            if i == 0 { self.matches.len() - 1 } else { i - 1 }
        };
        let (a, b) = self.matches[i];
        self.select(a, b);
        true
    }

    /// Selects the first match at or after the start of the selection, for
    /// search as you type. Returns false if there are none.
    pub fn find_from_selection(&mut self) -> bool {
        self.refresh_matches();
        let c = self.selection().map_or(self.editor.cursor(), |(s, _)| s);
        let i = self.matches.partition_point(|m| pos(m.0) < pos(c));
        match self.matches.get(i).or(self.matches.first()).copied() {
            Some((a, b)) => {
                self.select(a, b);
                true
            }
            None => false,
        }
    }

    /// Replaces the selected match and moves on to the next one.
    pub fn replace_current(&mut self, with: &str) -> bool {
        if self.current_match().is_none() {
            return self.find_next(true);
        }
        let (s, e) = self.selection().unwrap();
        self.edit(s, e, &normalize(with), EditKind::Other);
        self.find_next(true);
        true
    }

    /// Replaces every match as one undo step; returns how many.
    pub fn replace_all(&mut self, with: &str) -> usize {
        self.refresh_matches();
        let (Some(&(first, _)), Some(&(_, last))) = (self.matches.first(), self.matches.last()) else { return 0 };
        let with = normalize(with);
        let mut out = String::new();
        self.editor.with_buffer(|b| {
            let mut ms = self.matches.iter().peekable();
            for i in first.line..=last.line {
                let t = b.lines[i].text();
                let mut from = if i == first.line { first.index } else { 0 };
                let to = if i == last.line { last.index } else { t.len() };
                while let Some((s, e)) = ms.next_if(|m| m.0.line == i) {
                    out.push_str(&t[from..s.index]);
                    out.push_str(&with);
                    from = e.index;
                }
                out.push_str(&t[from.min(to)..to]);
                if i < last.line {
                    out.push('\n');
                }
            }
        });
        let n = self.matches.len();
        self.edit(first, last, &out, EditKind::Other);
        n
    }

    // -- paint --------------------------------------------------------------

    /// Lays out the lines in view. Spliced-in lines have no layout yet and
    /// cosmic-text would otherwise stop drawing at the first of them.
    fn layout_visible(&mut self, fonts: &mut FontSystem) {
        let height = self.size.1;
        self.editor.with_buffer_mut(|b| {
            let mut s = b.scroll();
            if s.line >= b.lines.len() {
                s = Scroll::new(b.lines.len() - 1, 0.0, s.horizontal);
                b.set_scroll(s);
            }
            let line_h = b.metrics().line_height;
            let mut y = -s.vertical;
            let mut i = s.line;
            while i < b.lines.len() && y < height {
                y += b.line_layout(fonts, i).map_or(1, |l| l.len().max(1)) as f32 * line_h;
                i += 1;
            }
        });
    }
}

impl Widget for TextView {
    fn paint(&mut self, cx: &mut Cx, scene: &mut Scene, bounds: Bounds, focused: bool) -> Option<Bounds> {
        let size = ((bounds.w - 2.0 * self.inset.0).max(1.0), (bounds.h - self.inset.1).max(1.0));
        if size != self.size {
            self.size = size;
            self.editor.with_buffer_mut(|b| b.set_size(Some(size.0), Some(size.1)));
        }
        let fonts = &mut cx.text.fonts;
        self.editor.shape_as_needed(fonts, false);
        self.layout_visible(fonts);
        self.refresh_matches();

        let scroll = self.editor.with_buffer(|b| b.scroll());
        let origin = Point { x: bounds.x + self.inset.0 - scroll.horizontal, y: bounds.y + self.inset.1 };
        let selection = self.editor.selection_bounds();
        let sel_color = if focused { self.selection_color } else { self.inactive_selection_color };
        let current = self.current_match();
        // A selected match shows as the current match instead.
        let selection = selection.filter(|_| current.is_none());
        let cursor = self.editor.cursor();
        let mut caret = None;
        let mut lines_in_view = 0;
        scene.push_clip(bounds);
        self.editor.with_buffer(|b| {
            let mut last_line = usize::MAX;
            for run in b.layout_runs() {
                if run.line_i != last_line {
                    last_line = run.line_i;
                    lines_in_view += 1;
                }
                let first = self.matches.partition_point(|m| m.0.line < run.line_i);
                for (k, m) in self.matches[first..].iter().enumerate().take_while(|(_, m)| m.0.line == run.line_i) {
                    let color = if current == Some(first + k) { self.current_match_color } else { self.match_color };
                    for (x, w) in run.highlight(m.0, m.1) {
                        scene.rect(Bounds::new(origin.x + x - 1.0, origin.y + run.line_top, w + 2.0, run.line_height), color, 3.0);
                    }
                }
                // `highlight` treats lines outside the range as selected.
                if let Some((start, end)) = selection.filter(|(s, e)| (s.line..=e.line).contains(&run.line_i)) {
                    let mut any = false;
                    for (x, w) in run.highlight(start, end) {
                        any = true;
                        scene.rect(Bounds::new(origin.x + x, origin.y + run.line_top, w.max(1.0), run.line_height), sel_color, 0.0);
                    }
                    // Line ends inside a selection get a sliver, like macOS.
                    if !any && run.glyphs.is_empty() && run.line_i > start.line && run.line_i < end.line {
                        scene.rect(Bounds::new(origin.x, origin.y + run.line_top, 6.0, run.line_height), sel_color, 0.0);
                    }
                }
                if caret.is_none() && run.line_i == cursor.line {
                    if let Some(x) = run.cursor_position(&cursor) {
                        caret = Some(Bounds::new(origin.x + x, origin.y + run.line_top, 2.0, run.line_height));
                    }
                }
            }
            draw_buffer(&mut cx.text.fonts, &mut cx.text.atlas, scene, b, origin, self.style.color, cx.scale);
        });
        let blink_on = self.last_activity.elapsed().as_millis() / CARET_BLINK_MS % 2 == 0;
        if let (true, true, true, Some(c)) = (focused, blink_on, self.selection().is_none(), caret) {
            let inset = (c.h * 0.1).round();
            scene.rect(Bounds::new(c.x.round(), c.y + inset, 2.0, c.h - 2.0 * inset), self.caret_color, 1.0);
        }
        scene.pop_clip();

        // An overlay scroller, sized by lines (wrapped lines make it approximate).
        let total = self.line_count();
        self.thumb = None;
        if total > lines_in_view && lines_in_view > 0 {
            let track = bounds.h - 6.0;
            let h = (track * lines_in_view as f32 / total as f32).max(28.0).min(track);
            let frac = scroll.line as f32 / (total - lines_in_view).max(1) as f32;
            let top = 3.0 + (track - h) * frac.min(1.0);
            self.thumb = Some((top, h));
            let w = if self.thumb_grab.is_some() { 8.0 } else { 6.0 };
            scene.rect(Bounds::new(bounds.right() - w - 3.0, bounds.y + top, w, h), self.scrollbar_color, w / 2.0);
        }
        caret
    }

    fn key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        let fonts = &mut cx.text.fonts;
        let before = self.state_id();
        if key.ctrl && !key.alt {
            if key.is_char('a') {
                self.select_all();
            } else if key.is_char('c') || key.is_char('x') {
                if let (Some(text), Some(cb)) = (self.selected_text(), cx.clipboard.as_mut()) {
                    let _ = cb.set_text(text);
                    if key.is_char('x') {
                        self.delete(fonts, Motion::Left);
                    }
                }
            } else if key.is_char('v') {
                if let Some(text) = cx.clipboard.as_mut().and_then(|cb| cb.get_text().ok()) {
                    self.insert(&text);
                }
            } else if key.is_char('z') && !key.shift {
                self.undo();
            } else if key.is_char('y') || (key.is_char('z') && key.shift) {
                self.redo();
            } else if key.is_named(NamedKey::Backspace) {
                self.delete(fonts, Motion::PreviousWord);
            } else if key.is_named(NamedKey::Delete) {
                self.delete(fonts, Motion::NextWord);
            } else {
                let motion = match &key.key {
                    Key::Named(NamedKey::ArrowLeft) => Motion::PreviousWord,
                    Key::Named(NamedKey::ArrowRight) => Motion::NextWord,
                    Key::Named(NamedKey::Home | NamedKey::ArrowUp) => Motion::BufferStart,
                    Key::Named(NamedKey::End | NamedKey::ArrowDown) => Motion::BufferEnd,
                    _ => return false,
                };
                self.motion(fonts, motion, key.shift);
            }
        } else {
            match &key.key {
                Key::Named(named) => {
                    let motion = match named {
                        NamedKey::ArrowLeft => Some(Motion::Left),
                        NamedKey::ArrowRight => Some(Motion::Right),
                        NamedKey::ArrowUp => Some(Motion::Up),
                        NamedKey::ArrowDown => Some(Motion::Down),
                        NamedKey::Home => Some(Motion::Home),
                        NamedKey::End => Some(Motion::End),
                        NamedKey::PageUp => Some(Motion::PageUp),
                        NamedKey::PageDown => Some(Motion::PageDown),
                        _ => None,
                    };
                    match (motion, named) {
                        (Some(m), _) => self.motion(fonts, m, key.shift),
                        (_, NamedKey::Backspace) => {
                            self.delete(fonts, Motion::Previous);
                        }
                        (_, NamedKey::Delete) => {
                            self.delete(fonts, Motion::Next);
                        }
                        (_, NamedKey::Enter) => self.type_text("\n", EditKind::Other),
                        (_, NamedKey::Tab) => self.type_text("\t", EditKind::Typing),
                        (_, NamedKey::Space) => self.type_text(" ", EditKind::Typing),
                        _ => return false,
                    }
                }
                _ => match key.text.as_deref().filter(|t| t.chars().any(|c| !c.is_control())) {
                    Some(text) => {
                        let text: String = text.chars().filter(|c| !c.is_control()).collect();
                        self.type_text(&text, EditKind::Typing);
                    }
                    None => return false,
                },
            }
        }
        self.last_activity = Instant::now();
        self.state_id() != before
    }

    fn mouse_down(&mut self, local: Point, clicks: u32, shift: bool, cx: &mut Cx) {
        self.last_activity = Instant::now();
        if let Some((top, h)) = self.thumb.filter(|_| local.x >= self.size.0 + 2.0 * self.inset.0 - SCROLLBAR_HIT) {
            let grab = if local.y >= top && local.y < top + h { local.y - top } else { h / 2.0 };
            self.thumb_grab = Some(grab);
            self.mouse_drag(local, cx);
            return;
        }
        self.thumb_grab = None;
        let scroll = self.editor.with_buffer(|b| b.scroll());
        let (x, y) = ((local.x - self.inset.0 + scroll.horizontal) as i32, (local.y - self.inset.1) as i32);
        let fonts = &mut cx.text.fonts;
        if shift {
            if self.editor.selection() == Selection::None {
                let c = self.editor.cursor();
                self.editor.set_selection(Selection::Normal(c));
            }
            self.editor.action(fonts, Action::Drag { x, y });
        } else {
            let action = match clicks {
                1 => Action::Click { x, y },
                2 => Action::DoubleClick { x, y },
                _ => Action::TripleClick { x, y },
            };
            self.editor.action(fonts, action);
        }
    }

    fn mouse_drag(&mut self, local: Point, cx: &mut Cx) {
        if let (Some(grab), Some((_, h))) = (self.thumb_grab, self.thumb) {
            let track = self.size.1 + self.inset.1 - 6.0;
            let frac = ((local.y - 3.0 - grab) / (track - h).max(1.0)).clamp(0.0, 1.0);
            let total = self.line_count();
            let line = (frac * total as f32) as usize;
            self.editor.with_buffer_mut(|b| {
                let s = b.scroll();
                b.set_scroll(Scroll::new(line.min(total - 1), 0.0, s.horizontal));
            });
            return;
        }
        let scroll = self.editor.with_buffer(|b| b.scroll());
        let (x, y) = ((local.x - self.inset.0 + scroll.horizontal) as i32, (local.y - self.inset.1) as i32);
        self.editor.action(&mut cx.text.fonts, Action::Drag { x, y });
        self.last_activity = Instant::now();
    }

    fn scroll(&mut self, dy: f32, cx: &mut Cx) -> bool {
        self.editor.action(&mut cx.text.fonts, Action::Scroll { pixels: dy });
        true
    }

    fn last_activity(&self) -> Option<Instant> {
        Some(self.last_activity)
    }
}

/// Where the caret ends up after inserting `text` at `at`.
fn end_of(at: Cursor, text: &str) -> Cursor {
    match text.rfind('\n') {
        None => Cursor::new(at.line, at.index + text.len()),
        Some(i) => Cursor::new(at.line + text.matches('\n').count(), text.len() - i - 1),
    }
}

fn pos(c: Cursor) -> (usize, usize) {
    (c.line, c.index)
}

fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Lowercases `s` without changing byte offsets (characters whose lowercase
/// form has a different length are left alone).
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| {
            let mut l = c.to_lowercase();
            match (l.next(), l.next()) {
                (Some(lc), None) if lc.len_utf8() == c.len_utf8() => lc,
                _ => c,
            }
        })
        .collect()
}

/// Non-overlapping byte ranges of `needle` in `line`.
fn find_in(line: &str, needle: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    let hay = if case_sensitive { std::borrow::Cow::Borrowed(line) } else { std::borrow::Cow::Owned(fold(line)) };
    hay.match_indices(needle).map(|(i, m)| (i, i + m.len())).collect()
}

/// The first monospaced family installed, preferring the usual coding fonts.
fn mono_family(fonts: &FontSystem) -> &'static str {
    static FAMILY: OnceLock<&'static str> = OnceLock::new();
    FAMILY.get_or_init(|| {
        let installed: Vec<String> =
            fonts.db().faces().filter(|f| f.monospaced).flat_map(|f| f.families.iter().map(|(n, _)| n.clone())).collect();
        let preferred = ["SF Mono", "Menlo", "JetBrains Mono", "Hack", "Adwaita Mono", "DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono"];
        let name = preferred
            .iter()
            .find(|p| installed.iter().any(|n| n == *p))
            .map(|p| p.to_string())
            .or_else(|| installed.first().cloned())
            .unwrap_or_else(|| FONT_FAMILY.to_string());
        Box::leak(name.into_boxed_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(text: &str) -> (TextView, FontSystem) {
        let mut fonts = FontSystem::new();
        let mut v = TextView::new(&mut fonts, TextStyle::default());
        v.set_text(text);
        (v, fonts)
    }

    #[test]
    fn splice_and_undo() {
        let (mut v, _) = view("hello\nworld");
        v.edit(Cursor::new(0, 5), Cursor::new(1, 0), ", big\nwide ", EditKind::Other);
        assert_eq!(v.text(), "hello, big\nwide world");
        assert_eq!(v.cursor(), Cursor::new(1, 5));
        let changed = v.state_id();
        assert!(v.undo());
        assert_eq!(v.text(), "hello\nworld");
        assert_eq!(v.state_id(), 0);
        assert!(v.redo());
        assert_eq!(v.text(), "hello, big\nwide world");
        assert_eq!(v.state_id(), changed);
    }

    #[test]
    fn typing_runs_merge() {
        let (mut v, _) = view("");
        for c in ["a", "b", "c"] {
            v.type_text(c, EditKind::Typing);
        }
        assert_eq!(v.text(), "abc");
        assert_eq!(v.undo.len(), 1);
        v.undo();
        assert_eq!(v.text(), "");
    }

    #[test]
    fn find_and_replace() {
        let (mut v, _) = view("One fish\ntwo FISH\nred fish, blue fish");
        v.set_query("fish", false);
        assert_eq!(v.match_count(), 4);
        assert!(v.find_next(true));
        assert_eq!(v.selected_text().as_deref(), Some("fish"));
        assert_eq!(v.current_match(), Some(0));
        v.find_next(false);
        assert_eq!(v.current_match(), Some(3));

        v.set_query("fish", true);
        assert_eq!(v.match_count(), 3);
        v.select(Cursor::new(0, 0), Cursor::new(0, 0));
        v.find_next(true);
        assert!(v.replace_current("cat"));
        assert_eq!(v.text(), "One cat\ntwo FISH\nred fish, blue fish");
        assert_eq!(v.current_match(), Some(0));

        assert_eq!(v.replace_all("dog"), 2);
        assert_eq!(v.text(), "One cat\ntwo FISH\nred dog, blue dog");
        assert_eq!(v.match_count(), 0);
        v.undo();
        assert_eq!(v.text(), "One cat\ntwo FISH\nred fish, blue fish");
    }

    #[test]
    fn replace_all_across_lines_ignores_case() {
        let (mut v, _) = view("aXa\nxx\nbx");
        v.set_query("x", false);
        assert_eq!(v.replace_all("—"), 4);
        assert_eq!(v.text(), "a—a\n——\nb—");
    }

    #[test]
    fn big_paste_is_fast() {
        let line = "The quick brown fox jumps over the lazy dog. ".repeat(3);
        let doc: String = (0..60_000).map(|i| format!("{i} {line}\n")).collect();
        let (mut v, _) = view(&doc);
        let start = Instant::now();
        v.select(Cursor::new(10, 0), Cursor::new(10, 0));
        v.insert(&doc);
        v.select_all();
        v.insert("x");
        v.undo();
        v.undo();
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
        assert_eq!(v.line_count(), 60_001);
    }
}
