use std::time::{Duration, Instant};

use cosmic_text::{Action, AttrsList, Buffer, Cursor, Edit, Editor, FontSystem, Motion, Selection, Shaping, Wrap};
use winit::keyboard::{Key, NamedKey};

use super::geom::{Bounds, Color, Point};
use super::scene::Scene;
use super::text::{GlyphAtlas, TextStyle, buffer_size, draw_buffer};

#[derive(Clone, Debug)]
pub struct KeyPress {
    pub key: Key,
    pub text: Option<String>,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyPress {
    pub fn is_char(&self, c: char) -> bool {
        matches!(&self.key, Key::Character(s) if s.chars().next().is_some_and(|k| k.eq_ignore_ascii_case(&c)))
    }

    pub fn is_named(&self, k: NamedKey) -> bool {
        self.key == Key::Named(k)
    }
}

#[derive(Default)]
pub struct KeyOutcome {
    pub handled: bool,
    pub changed: bool,
}

struct Snapshot {
    text: String,
    cursor: Cursor,
}

#[derive(Clone, Copy, PartialEq)]
enum EditKind {
    Typing,
    Deleting,
    Other,
}

/// Editable text backed by a cosmic-text `Editor`. Owned by the app (behind
/// `Rc<RefCell<..>>`) and rendered with the `input()` element.
pub struct TextInput {
    editor: Editor<'static>,
    pub single_line: bool,
    pub placeholder: String,
    pub style: TextStyle,
    /// Optional distinct style for the first line (the note title).
    pub first_line_style: Option<TextStyle>,
    pub placeholder_color: Color,
    pub selection_color: Color,
    pub caret_color: Color,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: (EditKind, Instant),
    pub(crate) last_activity: Instant,
    /// Set when the caret moved and should be scrolled into view.
    pub(crate) reveal_caret: bool,
    width: f32,
}

impl TextInput {
    pub fn new(fonts: &mut FontSystem, style: TextStyle, single_line: bool) -> Self {
        let mut buffer = Buffer::new(fonts, style.metrics());
        buffer.set_wrap(if single_line { Wrap::None } else { Wrap::WordOrGlyph });
        buffer.set_text("", &style.attrs(), Shaping::Advanced, None);
        Self {
            editor: Editor::new(buffer),
            single_line,
            placeholder: String::new(),
            style,
            first_line_style: None,
            placeholder_color: Color::BLACK.alpha(0.3),
            selection_color: Color::BLACK.alpha(0.2),
            caret_color: Color::BLACK,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: (EditKind::Other, Instant::now()),
            last_activity: Instant::now(),
            reveal_caret: false,
            width: 0.0,
        }
    }

    pub fn text(&self) -> String {
        self.editor.with_buffer(|b| {
            let mut out = String::new();
            for (i, line) in b.lines.iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(line.text());
            }
            out
        })
    }

    pub fn is_empty(&self) -> bool {
        self.editor.with_buffer(|b| b.lines.iter().all(|l| l.text().is_empty()))
    }

    /// Replace the content, put the caret at the end and clear undo history.
    pub fn set_text(&mut self, fonts: &mut FontSystem, text: &str) {
        let attrs = self.style.attrs();
        self.editor.with_buffer_mut(|b| {
            b.set_metrics(self.style.metrics());
            b.set_text(text, &attrs, Shaping::Advanced, None)
        });
        self.apply_styles();
        self.editor.set_selection(Selection::None);
        self.editor.set_cursor(Cursor::new(0, 0));
        self.editor.shape_as_needed(fonts, false);
        self.undo.clear();
        self.redo.clear();
    }

    /// Replaces the text (e.g. after an edit from another device), keeping
    /// the caret at the same character offset where possible.
    pub fn replace_text(&mut self, fonts: &mut FontSystem, text: &str) {
        let offset = self.editor.with_buffer(|b| {
            let c = self.editor.cursor();
            b.lines[..c.line.min(b.lines.len())].iter().map(|l| l.text().len() + 1).sum::<usize>() + c.index
        });
        let undo = std::mem::take(&mut self.undo);
        self.set_text(fonts, text);
        self.undo = undo;
        let mut offset = offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = text[..offset].matches('\n').count();
        let index = offset - text[..offset].rfind('\n').map_or(0, |i| i + 1);
        self.editor.set_cursor(Cursor::new(line, index));
    }

    pub fn move_to_end(&mut self) {
        let end = self.end_cursor();
        self.editor.set_selection(Selection::None);
        self.editor.set_cursor(end);
        self.last_activity = Instant::now();
    }

    pub fn select_all(&mut self) {
        self.editor.set_selection(Selection::Normal(Cursor::new(0, 0)));
        let end = self.end_cursor();
        self.editor.set_cursor(end);
        self.last_activity = Instant::now();
    }

    fn end_cursor(&self) -> Cursor {
        self.editor.with_buffer(|b| {
            let line = b.lines.len().saturating_sub(1);
            Cursor::new(line, b.lines.get(line).map_or(0, |l| l.text().len()))
        })
    }

    /// Update text styles, re-styling existing text if they changed.
    pub fn set_styles(&mut self, style: TextStyle, first_line: Option<TextStyle>) {
        if self.style != style || self.first_line_style != first_line {
            self.style = style;
            self.first_line_style = first_line;
            self.editor.with_buffer_mut(|b| b.set_metrics(style.metrics()));
            self.apply_styles();
        }
    }

    fn apply_styles(&mut self) {
        let body = AttrsList::new(&self.style.attrs().metrics(self.style.metrics()));
        let title = self.first_line_style.map(|s| AttrsList::new(&s.attrs().metrics(s.metrics())));
        self.editor.with_buffer_mut(|b| {
            for (i, line) in b.lines.iter_mut().enumerate() {
                let attrs = match (&title, i) {
                    (Some(t), 0) => t.clone(),
                    _ => body.clone(),
                };
                line.set_attrs_list(attrs);
            }
        });
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text(), cursor: self.editor.cursor() }
    }

    fn restore(&mut self, fonts: &mut FontSystem, snap: Snapshot) {
        let attrs = self.style.attrs();
        self.editor.with_buffer_mut(|b| b.set_text(&snap.text, &attrs, Shaping::Advanced, None));
        self.apply_styles();
        self.editor.set_selection(Selection::None);
        let max_line = self.editor.with_buffer(|b| b.lines.len().saturating_sub(1));
        let line = snap.cursor.line.min(max_line);
        let len = self.editor.with_buffer(|b| b.lines[line].text().len());
        self.editor.set_cursor(Cursor::new(line, snap.cursor.index.min(len)));
        self.editor.shape_as_needed(fonts, false);
    }

    /// Record an undo point, coalescing runs of typing or deleting.
    fn checkpoint(&mut self, kind: EditKind) {
        let now = Instant::now();
        let (last_kind, at) = self.last_edit;
        let coalesce = kind != EditKind::Other && kind == last_kind && now - at < Duration::from_secs(1);
        if !coalesce {
            let snap = self.snapshot();
            self.undo.push(snap);
            if self.undo.len() > 500 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = (kind, now);
    }

    fn undo(&mut self, fonts: &mut FontSystem) -> bool {
        let Some(snap) = self.undo.pop() else { return false };
        let current = self.snapshot();
        self.redo.push(current);
        self.restore(fonts, snap);
        self.last_edit = (EditKind::Other, Instant::now());
        true
    }

    fn redo(&mut self, fonts: &mut FontSystem) -> bool {
        let Some(snap) = self.redo.pop() else { return false };
        let current = self.snapshot();
        self.undo.push(current);
        self.restore(fonts, snap);
        self.last_edit = (EditKind::Other, Instant::now());
        true
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

    fn insert(&mut self, fonts: &mut FontSystem, text: &str) {
        let text = if self.single_line { text.replace(['\n', '\r'], " ") } else { text.replace("\r\n", "\n") };
        self.editor.delete_selection();
        for c in text.chars() {
            match c {
                '\n' => self.editor.action(fonts, Action::Enter),
                c if c == '\t' || !c.is_control() => self.editor.action(fonts, Action::Insert(c)),
                _ => {}
            }
        }
    }

    pub fn handle_key(
        &mut self,
        fonts: &mut FontSystem,
        key: &KeyPress,
        clipboard: &mut Option<arboard::Clipboard>,
    ) -> KeyOutcome {
        let mut out = KeyOutcome { handled: true, changed: false };
        let before = self.editor.cursor();

        if key.ctrl && !key.alt {
            if key.is_char('a') {
                self.select_all();
            } else if key.is_char('c') || key.is_char('x') {
                if let (Some(text), Some(cb)) = (self.editor.copy_selection(), clipboard.as_mut()) {
                    let _ = cb.set_text(text);
                    if key.is_char('x') {
                        self.checkpoint(EditKind::Other);
                        out.changed = self.editor.delete_selection();
                    }
                }
            } else if key.is_char('v') {
                if let Some(text) = clipboard.as_mut().and_then(|cb| cb.get_text().ok()) {
                    self.checkpoint(EditKind::Other);
                    self.insert(fonts, &text);
                    out.changed = true;
                }
            } else if key.is_char('z') && !key.shift {
                out.changed = self.undo(fonts);
            } else if key.is_char('y') || (key.is_char('z') && key.shift) {
                out.changed = self.redo(fonts);
            } else if key.is_named(NamedKey::Backspace) {
                self.checkpoint(EditKind::Deleting);
                if self.editor.selection() == Selection::None {
                    self.motion(fonts, Motion::PreviousWord, true);
                }
                out.changed = self.editor.delete_selection();
            } else if key.is_named(NamedKey::Delete) {
                self.checkpoint(EditKind::Deleting);
                if self.editor.selection() == Selection::None {
                    self.motion(fonts, Motion::NextWord, true);
                }
                out.changed = self.editor.delete_selection();
            } else if let Key::Named(named) = &key.key {
                let motion = match named {
                    NamedKey::ArrowLeft => Some(Motion::PreviousWord),
                    NamedKey::ArrowRight => Some(Motion::NextWord),
                    NamedKey::Home | NamedKey::ArrowUp => Some(Motion::BufferStart),
                    NamedKey::End | NamedKey::ArrowDown => Some(Motion::BufferEnd),
                    _ => None,
                };
                match motion {
                    Some(m) => self.motion(fonts, m, key.shift),
                    None => out.handled = false,
                }
            } else {
                out.handled = false;
            }
        } else {
            match &key.key {
                Key::Named(named) => {
                    let motion = match named {
                        NamedKey::ArrowLeft => Some(Motion::Left),
                        NamedKey::ArrowRight => Some(Motion::Right),
                        NamedKey::ArrowUp if !self.single_line => Some(Motion::Up),
                        NamedKey::ArrowDown if !self.single_line => Some(Motion::Down),
                        NamedKey::Home => Some(Motion::Home),
                        NamedKey::End => Some(Motion::End),
                        NamedKey::PageUp if !self.single_line => Some(Motion::PageUp),
                        NamedKey::PageDown if !self.single_line => Some(Motion::PageDown),
                        _ => None,
                    };
                    if let Some(m) = motion {
                        self.motion(fonts, m, key.shift);
                    } else {
                        match named {
                            NamedKey::Backspace => {
                                self.checkpoint(EditKind::Deleting);
                                self.editor.action(fonts, Action::Backspace);
                                out.changed = true;
                            }
                            NamedKey::Delete => {
                                self.checkpoint(EditKind::Deleting);
                                self.editor.action(fonts, Action::Delete);
                                out.changed = true;
                            }
                            NamedKey::Enter if !self.single_line => {
                                self.checkpoint(EditKind::Other);
                                self.editor.delete_selection();
                                self.editor.action(fonts, Action::Enter);
                                out.changed = true;
                            }
                            NamedKey::Tab if !self.single_line => {
                                self.checkpoint(EditKind::Typing);
                                self.insert(fonts, "\t");
                                out.changed = true;
                            }
                            NamedKey::Space => {
                                self.checkpoint(EditKind::Typing);
                                self.insert(fonts, " ");
                                out.changed = true;
                            }
                            _ => out.handled = false,
                        }
                    }
                }
                _ => match key.text.as_deref().filter(|t| t.chars().any(|c| !c.is_control())) {
                    Some(text) => {
                        self.checkpoint(EditKind::Typing);
                        self.insert(fonts, text);
                        out.changed = true;
                    }
                    None => out.handled = false,
                },
            }
        }

        if out.changed {
            self.apply_styles();
        }
        if out.handled {
            self.last_activity = Instant::now();
            if out.changed || self.editor.cursor() != before {
                self.reveal_caret = true;
            }
        }
        out
    }

    pub fn insert_text(&mut self, fonts: &mut FontSystem, text: &str) {
        self.checkpoint(EditKind::Typing);
        self.insert(fonts, text);
        self.apply_styles();
        self.last_activity = Instant::now();
        self.reveal_caret = true;
    }

    /// `local` is relative to the input's top-left corner.
    pub fn mouse_down(&mut self, fonts: &mut FontSystem, local: Point, clicks: u32, extend: bool) {
        let (x, y) = (local.x as i32, local.y as i32);
        if extend {
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
        self.last_activity = Instant::now();
    }

    pub fn mouse_drag(&mut self, fonts: &mut FontSystem, local: Point) {
        self.editor.action(fonts, Action::Drag { x: local.x as i32, y: local.y as i32 });
        self.last_activity = Instant::now();
        self.reveal_caret = true;
    }

    pub(crate) fn measure(&mut self, fonts: &mut FontSystem, width: Option<f32>) -> (f32, f32) {
        let w = if self.single_line { None } else { width };
        self.editor.with_buffer_mut(|b| b.set_size(w, None));
        self.editor.shape_as_needed(fonts, false);
        let line_h = self.style.metrics().line_height;
        let (mw, h) = self.editor.with_buffer(|b| buffer_size(b, line_h));
        (width.unwrap_or(mw), h)
    }

    /// Paints the text and, when focused, the selection and caret.
    /// Returns the caret rectangle in window coordinates.
    pub(crate) fn paint(
        &mut self,
        fonts: &mut FontSystem,
        atlas: &mut GlyphAtlas,
        scene: &mut Scene,
        bounds: Bounds,
        focused: bool,
        caret_on: bool,
        scale: f32,
    ) -> Option<Bounds> {
        if (self.width - bounds.w).abs() > 0.5 {
            self.width = bounds.w;
            if !self.single_line {
                self.editor.with_buffer_mut(|b| b.set_size(Some(bounds.w), None));
            }
        }
        self.editor.shape_as_needed(fonts, false);
        let origin = Point { x: bounds.x, y: bounds.y };

        if self.is_empty() && !self.placeholder.is_empty() {
            let mut buffer = Buffer::new(fonts, self.style.metrics());
            buffer.set_text(&self.placeholder, &self.style.attrs(), Shaping::Advanced, None);
            buffer.shape_until_scroll(fonts, false);
            draw_buffer(fonts, atlas, scene, &buffer, origin, self.placeholder_color, scale);
        }

        let selection = if focused { self.editor.selection_bounds() } else { None };
        let cursor = self.editor.cursor();
        let mut caret = None;
        self.editor.with_buffer(|b| {
            for run in b.layout_runs() {
                if let Some((start, end)) = selection.filter(|(s, e)| (s.line..=e.line).contains(&run.line_i)) {
                    let mut any = false;
                    for (x, w) in run.highlight(start, end) {
                        any = true;
                        let r = Bounds::new(origin.x + x, origin.y + run.line_top, w.max(1.0), run.line_height);
                        scene.rect(r, self.selection_color, 0.0);
                    }
                    // Empty lines inside a selection still get a sliver, like macOS.
                    if !any && run.glyphs.is_empty() && run.line_i > start.line && run.line_i < end.line {
                        let r = Bounds::new(origin.x, origin.y + run.line_top, 6.0, run.line_height);
                        scene.rect(r, self.selection_color, 0.0);
                    }
                }
                if caret.is_none() && run.line_i == cursor.line {
                    if let Some(x) = run.cursor_position(&cursor) {
                        caret = Some(Bounds::new(origin.x + x, origin.y + run.line_top, 2.0, run.line_height));
                    }
                }
            }
            draw_buffer(fonts, atlas, scene, b, origin, self.style.color, scale);
        });
        let caret = caret.unwrap_or_else(|| {
            let lh = self.first_line_style.unwrap_or(self.style).metrics().line_height;
            Bounds::new(origin.x, origin.y, 2.0, lh)
        });
        if focused && caret_on {
            let inset = (caret.h * 0.1).round();
            let r = Bounds::new(caret.x.round(), caret.y + inset, 2.0, caret.h - 2.0 * inset);
            scene.rect(r, self.caret_color, 1.0);
        }
        Some(caret)
    }
}
