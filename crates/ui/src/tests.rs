//! Layout, paint and id tests that run without a GPU: they lay out and paint
//! into a `Scene` and look at the hitboxes and primitives that come out.

use crate::cx::Cx;
use crate::element::{AnyElement, Div, Frame, Styled, div, eid, render_tree, text};
use crate::geom::{Bounds, Color, Point};
use crate::scene::Scene;
use crate::text::TextStyle;

struct S;

fn render(root: Div<S>, cx: &mut Cx) -> (Frame<S>, Scene) {
    let mut scene = Scene::default();
    let root = AnyElement::Div(root);
    let frame = render_tree(&root, cx, &mut scene, TextStyle::default());
    (frame, scene)
}

fn cx(w: f32, h: f32) -> Cx {
    let mut cx = Cx::new();
    cx.window_size = (w, h);
    cx.scale = 2.0;
    cx
}

fn hitbox(frame: &Frame<S>, id: &str) -> Bounds {
    frame.hitboxes.iter().find(|h| h.id == Some(eid(id))).expect("hitbox").bounds
}

/// Leftmost and topmost glyph edge painted, in logical points.
fn glyph_origin(scene: &Scene) -> Point {
    let mut x = f32::MAX;
    let mut y = f32::MAX;
    for i in scene.instances().filter(|i| i.params[3] == 1.0) {
        x = x.min(i.rect[0]);
        y = y.min(i.rect[1]);
    }
    Point { x, y }
}

#[test]
fn text_respects_its_padding() {
    let mut c = cx(400.0, 200.0);
    let (_, plain) = render(div().w(400.0).h(200.0).child(text("Header")), &mut c);
    let (_, padded) = render(div().w(400.0).h(200.0).child(text("Header").pl(20.0).pt(10.0)), &mut c);
    let (a, b) = (glyph_origin(&plain), glyph_origin(&padded));
    assert!((b.x - a.x - 20.0).abs() < 0.6, "x moved by {}", b.x - a.x);
    assert!((b.y - a.y - 10.0).abs() < 0.6, "y moved by {}", b.y - a.y);
}

#[test]
fn absolute_children_size_against_their_parent() {
    let mut c = cx(800.0, 600.0);
    let root = div().w(800.0).h(600.0).child(
        div()
            .absolute(100.0, 50.0)
            .w(200.0)
            .h(120.0)
            .child(div().id("fill").inset_0().on_click(|_: &mut S, _| {}))
            .child(div().id("full").absolute(0.0, 0.0).w_full().h_full().on_click(|_: &mut S, _| {})),
    );
    let (frame, _) = render(root, &mut c);
    assert_eq!(hitbox(&frame, "fill"), Bounds::new(100.0, 50.0, 200.0, 120.0));
    assert_eq!(hitbox(&frame, "full"), Bounds::new(100.0, 50.0, 200.0, 120.0));

    // Same inside a flex child that gets its size from flex_1 rather than w/h.
    let root = div().w(800.0).h(600.0).flex_row().child(div().w(300.0)).child(
        div().flex_1().h_full().child(div().id("overlay").inset_0().on_click(|_: &mut S, _| {})),
    );
    let (frame, _) = render(root, &mut c);
    assert_eq!(hitbox(&frame, "overlay"), Bounds::new(300.0, 0.0, 500.0, 600.0));
}

#[test]
fn precomputed_ids_are_not_rehashed() {
    let id = eid(("row", 7));
    assert_eq!(eid(id), id);
    let mut c = cx(200.0, 100.0);
    c.mouse = Point { x: 10.0, y: 10.0 };
    let root = div().w(200.0).h(100.0).child(div().id(id).size(50.0).hover_bg(Color::BLACK));
    let (frame, _) = render(root, &mut c);
    c.hovered = frame.hits(c.mouse).iter().filter_map(|h| h.id).collect();
    assert!(c.is_hovered(id));
    assert!(c.hovered(("row", 7)));
    c.focus(id);
    assert!(c.is_focused(("row", 7)));
}

fn key(k: winit::keyboard::Key, ctrl: bool, shift: bool) -> crate::KeyPress {
    crate::KeyPress { key: k, text: None, ctrl, shift, alt: false }
}

#[test]
fn text_editing_selects_like_a_mac() {
    use winit::keyboard::{Key, NamedKey};
    let mut c = cx(400.0, 200.0);
    let fonts = &mut c.text.fonts;
    let mut t = crate::TextInput::new(fonts, TextStyle::default(), false);
    t.set_text(fonts, "hello brave world\nsecond line here");
    t.measure(fonts, Some(400.0));

    // Double-click a word (over "brave"), triple-click a line.
    t.mouse_down(fonts, Point { x: 50.0, y: 5.0 }, 2, false);
    assert_eq!(t.selected_text().as_deref(), Some("brave"));
    t.mouse_down(fonts, Point { x: 50.0, y: 5.0 }, 3, false);
    assert_eq!(t.selected_text().as_deref().map(str::trim_end), Some("hello brave world"));

    // Ctrl+A, then Shift+arrows from a plain caret.
    t.handle_key(fonts, &key(Key::Character("a".into()), true, false), &mut None);
    assert_eq!(t.selected_text().as_deref(), Some("hello brave world\nsecond line here"));
    t.handle_key(fonts, &key(Key::Named(NamedKey::Home), true, false), &mut None);
    for _ in 0..5 {
        t.handle_key(fonts, &key(Key::Named(NamedKey::ArrowRight), false, true), &mut None);
    }
    assert_eq!(t.selected_text().as_deref(), Some("hello"));
    t.handle_key(fonts, &key(Key::Named(NamedKey::ArrowRight), true, true), &mut None);
    assert_eq!(t.selected_text().as_deref().map(str::trim_end), Some("hello brave"));

    // One-line fields jump to their ends with Up and Down.
    let mut f = crate::TextInput::new(fonts, TextStyle::default(), true);
    f.set_text(fonts, "search me");
    f.handle_key(fonts, &key(Key::Named(NamedKey::ArrowDown), false, true), &mut None);
    assert_eq!(f.selected_text().as_deref(), Some("search me"));
}

struct Rows;

impl crate::App for Rows {
    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        let rows = (0..60).map(|i| div().h(24.0).px(12.0).child(text(format!("Row {i}"))));
        let list = div().id("rows").scroll_y().flex_1().flex_col().children(rows);
        let (w, h) = cx.window_size;
        div().w(w).h(h).bg(if cx.dark { crate::rgb(0x1e1e1e) } else { Color::WHITE }).flex_col().child(list)
    }
    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { color: if cx.dark { Color::WHITE } else { Color::BLACK }, ..TextStyle::default() }
    }
}

#[test]
fn scrolling_shows_an_overlay_scrollbar() {
    use std::time::Instant;
    let mut c = cx(300.0, 400.0);
    let mut app = Rows;
    let paint = |app: &mut Rows, c: &mut Cx| {
        let mut scene = Scene::default();
        let root = AnyElement::Div(crate::App::render(app, c));
        render_tree(&root, c, &mut scene, TextStyle::default());
        scene
    };
    let knob = |scene: &Scene| scene.instances().any(|i| i.params[3] == 0.0 && i.rect[0] > 280.0 && i.rect[2] <= 10.0);
    let idle = paint(&mut app, &mut c);
    assert!(!knob(&idle), "no scrollbar before scrolling");
    c.scroll_region(eid("rows"), -200.0, crate::kinetic::Phase::Wheel, Instant::now());
    let scrolled = paint(&mut app, &mut c);
    assert!(knob(&scrolled), "scrollbar while scrolling");
    assert_eq!(c.scroll_y("rows"), 200.0);

    // For eyeballing: UI_TEST_PNG=/tmp/x.png cargo test -p ui scrolling_shows
    if let Some(path) = std::env::var_os("UI_TEST_PNG") {
        crate::snapshot((300.0, 400.0), 2.0, false, std::path::Path::new(&path), |_| Rows, |_, c| {
            c.note_region(eid("rows"), Bounds::new(0.0, 0.0, 300.0, 400.0), 1440.0, 1040.0);
            c.scroll_region(eid("rows"), -300.0, crate::kinetic::Phase::Touch, Instant::now());
            c.scroll_region(eid("rows"), 400.0, crate::kinetic::Phase::Touch, Instant::now());
        });
    }
}

#[test]
fn one_line_fields_scroll_to_keep_the_caret_in_view() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut c = cx(300.0, 100.0);
    let mut field = crate::TextInput::new(&mut c.text.fonts, TextStyle::default(), true);
    field.set_text(&mut c.text.fonts, "a long search query that is much wider than the field");
    field.move_to_end();
    let field = Rc::new(RefCell::new(field));
    c.focus("field");
    let root = || div::<S>().w(300.0).h(100.0).child(div().absolute(20.0, 20.0).w(120.0).h(20.0).child(crate::input("field", &field).w_full()));
    render(root(), &mut c);
    let caret = c.ime_area.expect("caret");
    assert!(caret.x >= 20.0 && caret.right() <= 142.0, "caret at {caret:?}");
    // Back at the start, the view scrolls back too.
    field.borrow_mut().handle_key(&mut c.text.fonts, &key(winit::keyboard::Key::Named(winit::keyboard::NamedKey::Home), false, false), &mut None);
    render(root(), &mut c);
    let caret = c.ime_area.expect("caret");
    assert!((caret.x - 20.0).abs() < 0.5, "caret at {caret:?}");
}

#[test]
fn menu_rows_highlight_under_the_pointer() {
    use crate::widgets::{Menu, MenuItem};
    let mut c = cx(400.0, 300.0);
    let menu = Menu::new(Point { x: 10.0, y: 10.0 }, vec![MenuItem::new("Open", 1), MenuItem::new("Rename", 2)]);
    let accent = crate::native::accent(&c).premultiplied();
    let lit = |scene: &Scene| scene.instances().any(|i| i.params[3] == 0.0 && i.color == accent);
    let root = |m: &Menu<i32>, c: &mut Cx| div::<S>().w(400.0).h(300.0).child(m.render(c, |_, _, _| {}, |_, _| {}));
    let (frame, scene) = render(root(&menu, &mut c), &mut c);
    assert!(!lit(&scene));
    // Point at the second row (pad 5 + one 24pt row + half a row).
    c.mouse = Point { x: 60.0, y: 10.0 + 5.0 + 24.0 + 12.0 };
    c.hovered = frame.hits(c.mouse).iter().filter_map(|h| h.id).collect();
    assert_eq!(menu.highlighted(&c), Some(1));
    let (_, scene) = render(root(&menu, &mut c), &mut c);
    assert!(lit(&scene), "the hovered row is filled with the accent");
}

#[test]
fn caret_blinks_only_in_an_active_window() {
    use std::time::{Duration, Instant};
    let mut c = cx(100.0, 100.0);
    let now = Instant::now();
    assert!(c.caret_on(now), "solid right after typing");
    // Long idle: stays on (no more blinking wake-ups).
    assert!(c.caret_on(now - crate::cx::CARET_BLINK_FOR - Duration::from_secs(1)));
    c.window_focused = false;
    assert!(!c.caret_on(now), "hidden while the window is inactive");
}

