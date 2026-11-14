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

