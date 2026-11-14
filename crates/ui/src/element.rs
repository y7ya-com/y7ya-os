//! A tiny GPUI-flavored element tree: `div()` builders with flexbox styles
//! (via taffy), text, editable inputs and custom-painted canvases. The tree is
//! rebuilt every frame by the app, laid out, painted, and kept around until
//! the next frame so input events can be dispatched to its handlers.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use taffy::prelude::*;
use winit::window::CursorIcon;

use super::cx::Cx;
use super::geom::{Bounds, Color, Point};
use super::input::TextInput;
use super::scene::Scene;
use super::text::{TextStyle, Weight};
use super::widget::{Widget, WidgetEl};

pub type ElementId = u64;

/// Hashes `v` into an id. A bare `u64` is taken to be an id already (say one
/// from an earlier `eid` call) and kept as is, so `.id(eid(x))`, `.id(x)`
/// and `cx.hovered(x)` all agree.
pub fn eid(v: impl Hash) -> ElementId {
    let mut h = IdHasher::default();
    v.hash(&mut h);
    h.finish()
}

/// `DefaultHasher`, except that a lone `write_u64` passes straight through.
#[derive(Default)]
struct IdHasher {
    inner: std::collections::hash_map::DefaultHasher,
    writes: u32,
    raw: u64,
}

impl Hasher for IdHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.writes += 2;
        self.inner.write(bytes);
    }
    fn write_u64(&mut self, v: u64) {
        self.writes += 1;
        self.raw = v;
        self.inner.write_u64(v);
    }
    fn finish(&self) -> u64 {
        if self.writes == 1 { self.raw } else { self.inner.finish() }
    }
}

pub type Handler<S> = Rc<dyn Fn(&mut S, &mut Cx)>;

#[derive(Clone, Copy, Default)]
pub struct TextOverrides {
    pub size: Option<f32>,
    pub weight: Option<Weight>,
    pub color: Option<Color>,
    pub line_height: Option<f32>,
}

impl TextOverrides {
    fn apply(&self, mut s: TextStyle) -> TextStyle {
        if let Some(v) = self.size {
            s.size = v;
        }
        if let Some(v) = self.weight {
            s.weight = v;
        }
        if let Some(v) = self.color {
            s.color = v;
        }
        if let Some(v) = self.line_height {
            s.line_height = v;
        }
        s
    }
}

pub enum AnyElement<S> {
    Div(Div<S>),
    Text(Text),
    Input(InputEl),
    Canvas(Canvas),
    Widget(WidgetEl),
}

// ---------------------------------------------------------------------------
// div

pub struct Div<S> {
    pub(crate) id: Option<ElementId>,
    pub(crate) layout: Style,
    pub(crate) bg: Option<Color>,
    pub(crate) hover_bg: Option<Color>,
    pub(crate) border: Option<(Color, f32)>,
    pub(crate) border_bottom: Option<Color>,
    pub(crate) radius: f32,
    pub(crate) shadow: Option<(Color, f32, f32)>,
    pub(crate) ring: Option<(Color, f32)>,
    pub(crate) text: TextOverrides,
    pub(crate) cursor: Option<CursorIcon>,
    pub(crate) children: Vec<AnyElement<S>>,
    pub(crate) on_mouse_down: Option<Handler<S>>,
    pub(crate) on_click: Option<Handler<S>>,
    pub(crate) on_right_click: Option<Handler<S>>,
    pub(crate) on_middle_click: Option<Handler<S>>,
    pub(crate) on_double_click: Option<Handler<S>>,
    pub(crate) window_drag: bool,
    pub(crate) focusable: bool,
    pub(crate) scroll: bool,
    pub(crate) clip: bool,
    pub(crate) deferred: bool,
    pub(crate) occlude: bool,
    /// Scales the alpha of everything the div paints (see `opacity.rs`).
    pub(crate) opacity: f32,
}

pub fn div<S>() -> Div<S> {
    Div {
        id: None,
        // Relative, so absolutely positioned children (`inset_0`, `absolute`)
        // are placed and sized against this div rather than the window.
        layout: Style { display: Display::Flex, position: Position::Relative, ..Default::default() },
        bg: None,
        hover_bg: None,
        border: None,
        border_bottom: None,
        radius: 0.0,
        shadow: None,
        ring: None,
        text: TextOverrides::default(),
        cursor: None,
        children: Vec::new(),
        on_mouse_down: None,
        on_click: None,
        on_right_click: None,
        on_middle_click: None,
        on_double_click: None,
        window_drag: false,
        focusable: false,
        scroll: false,
        clip: false,
        deferred: false,
        occlude: false,
        opacity: 1.0,
    }
}

fn len(v: f32) -> LengthPercentage {
    LengthPercentage::length(v)
}

/// Layout helpers shared by every element type.
pub trait Styled: Sized {
    fn layout_mut(&mut self) -> &mut Style;

    fn flex_row(mut self) -> Self {
        self.layout_mut().flex_direction = FlexDirection::Row;
        self
    }
    fn flex_col(mut self) -> Self {
        self.layout_mut().flex_direction = FlexDirection::Column;
        self
    }
    fn items_center(mut self) -> Self {
        self.layout_mut().align_items = AlignItems::CENTER;
        self
    }
    fn items_start(mut self) -> Self {
        self.layout_mut().align_items = AlignItems::FLEX_START;
        self
    }
    fn items_end(mut self) -> Self {
        self.layout_mut().align_items = AlignItems::FLEX_END;
        self
    }
    fn justify_center(mut self) -> Self {
        self.layout_mut().justify_content = JustifyContent::CENTER;
        self
    }
    fn justify_between(mut self) -> Self {
        self.layout_mut().justify_content = JustifyContent::SPACE_BETWEEN;
        self
    }
    fn justify_end(mut self) -> Self {
        self.layout_mut().justify_content = JustifyContent::FLEX_END;
        self
    }
    fn flex_1(mut self) -> Self {
        let l = self.layout_mut();
        l.flex_grow = 1.0;
        l.flex_shrink = 1.0;
        l.flex_basis = Dimension::length(0.0);
        l.min_size.width = LengthPercentageAuto::length(0.0);
        l.min_size.height = LengthPercentageAuto::length(0.0);
        self
    }
    fn grow(mut self) -> Self {
        self.layout_mut().flex_grow = 1.0;
        self
    }
    fn shrink_0(mut self) -> Self {
        self.layout_mut().flex_shrink = 0.0;
        self
    }
    fn min_w_0(mut self) -> Self {
        self.layout_mut().min_size.width = LengthPercentageAuto::length(0.0);
        self
    }
    fn w(mut self, v: f32) -> Self {
        self.layout_mut().size.width = Dimension::length(v);
        self
    }
    fn h(mut self, v: f32) -> Self {
        self.layout_mut().size.height = Dimension::length(v);
        self
    }
    fn size(self, v: f32) -> Self {
        self.w(v).h(v)
    }
    fn w_full(mut self) -> Self {
        self.layout_mut().size.width = Dimension::percent(1.0);
        self
    }
    fn h_full(mut self) -> Self {
        self.layout_mut().size.height = Dimension::percent(1.0);
        self
    }
    fn min_w(mut self, v: f32) -> Self {
        self.layout_mut().min_size.width = LengthPercentageAuto::length(v);
        self
    }
    fn min_h(mut self, v: f32) -> Self {
        self.layout_mut().min_size.height = LengthPercentageAuto::length(v);
        self
    }
    fn max_w(mut self, v: f32) -> Self {
        self.layout_mut().max_size.width = LengthPercentageAuto::length(v);
        self
    }
    fn p(mut self, v: f32) -> Self {
        self.layout_mut().padding = taffy::Rect { left: len(v), right: len(v), top: len(v), bottom: len(v) };
        self
    }
    fn px(mut self, v: f32) -> Self {
        let l = self.layout_mut();
        l.padding.left = len(v);
        l.padding.right = len(v);
        self
    }
    fn py(mut self, v: f32) -> Self {
        let l = self.layout_mut();
        l.padding.top = len(v);
        l.padding.bottom = len(v);
        self
    }
    fn pt(mut self, v: f32) -> Self {
        self.layout_mut().padding.top = len(v);
        self
    }
    fn pb(mut self, v: f32) -> Self {
        self.layout_mut().padding.bottom = len(v);
        self
    }
    fn pl(mut self, v: f32) -> Self {
        self.layout_mut().padding.left = len(v);
        self
    }
    fn pr(mut self, v: f32) -> Self {
        self.layout_mut().padding.right = len(v);
        self
    }
    fn mt(mut self, v: f32) -> Self {
        self.layout_mut().margin.top = LengthPercentageAuto::length(v);
        self
    }
    fn mb(mut self, v: f32) -> Self {
        self.layout_mut().margin.bottom = LengthPercentageAuto::length(v);
        self
    }
    fn ml(mut self, v: f32) -> Self {
        self.layout_mut().margin.left = LengthPercentageAuto::length(v);
        self
    }
    fn mr(mut self, v: f32) -> Self {
        self.layout_mut().margin.right = LengthPercentageAuto::length(v);
        self
    }
    fn mx(self, v: f32) -> Self {
        self.ml(v).mr(v)
    }
    fn mx_auto(mut self) -> Self {
        let l = self.layout_mut();
        l.margin.left = LengthPercentageAuto::auto();
        l.margin.right = LengthPercentageAuto::auto();
        self
    }
    fn gap(mut self, v: f32) -> Self {
        self.layout_mut().gap = taffy::Size { width: len(v), height: len(v) };
        self
    }
    fn absolute(mut self, x: f32, y: f32) -> Self {
        let l = self.layout_mut();
        l.position = Position::Absolute;
        l.inset.left = LengthPercentageAuto::length(x);
        l.inset.top = LengthPercentageAuto::length(y);
        self
    }
    fn inset_0(mut self) -> Self {
        let l = self.layout_mut();
        l.position = Position::Absolute;
        let z = LengthPercentageAuto::length(0.0);
        l.inset = taffy::Rect { left: z, right: z, top: z, bottom: z };
        self
    }
}

impl<S> Styled for Div<S> {
    fn layout_mut(&mut self) -> &mut Style {
        &mut self.layout
    }
}

impl<S> Div<S> {
    pub fn id(mut self, id: impl Hash) -> Self {
        self.id = Some(eid(id));
        self
    }
    pub fn bg(mut self, c: Color) -> Self {
        self.bg = Some(c);
        self
    }
    pub fn bg_opt(mut self, c: Option<Color>) -> Self {
        if c.is_some() {
            self.bg = c;
        }
        self
    }
    pub fn hover_bg(mut self, c: Color) -> Self {
        self.hover_bg = Some(c);
        self
    }
    pub fn border(mut self, c: Color, w: f32) -> Self {
        self.border = Some((c, w));
        self
    }
    /// A hairline divider along the bottom edge.
    pub fn border_b(mut self, c: Color) -> Self {
        self.border_bottom = Some(c);
        self
    }
    pub fn rounded(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    pub fn shadow(mut self, c: Color, blur: f32, dy: f32) -> Self {
        self.shadow = Some((c, blur, dy));
        self
    }
    /// A ring `w` wide just outside the edge, following the corner radius:
    /// the keyboard focus ring (see `native::focus_ring`).
    pub fn ring(mut self, c: Color, w: f32) -> Self {
        self.ring = Some((c, w));
        self
    }
    /// `ring` with the system focus ring, when `on`.
    pub fn focus_ring(self, on: bool, cx: &Cx) -> Self {
        if on { self.ring(crate::native::focus_ring(cx), crate::native::FOCUS_RING_W) } else { self }
    }
    pub fn text_size(mut self, v: f32) -> Self {
        self.text.size = Some(v);
        self
    }
    pub fn text_color(mut self, c: Color) -> Self {
        self.text.color = Some(c);
        self
    }
    pub fn font_weight(mut self, w: Weight) -> Self {
        self.text.weight = Some(w);
        self
    }
    pub fn cursor(mut self, c: CursorIcon) -> Self {
        self.cursor = Some(c);
        self
    }
    pub fn child(mut self, c: impl Into<AnyElement<S>>) -> Self {
        self.children.push(c.into());
        self
    }
    pub fn child_if(self, cond: bool, f: impl FnOnce() -> AnyElement<S>) -> Self {
        if cond { self.child(f()) } else { self }
    }
    pub fn children<E: Into<AnyElement<S>>>(mut self, cs: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(cs.into_iter().map(Into::into));
        self
    }
    pub fn on_mouse_down(mut self, f: impl Fn(&mut S, &mut Cx) + 'static) -> Self {
        self.on_mouse_down = Some(Rc::new(f));
        self
    }
    pub fn on_click(mut self, f: impl Fn(&mut S, &mut Cx) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
    pub fn on_right_click(mut self, f: impl Fn(&mut S, &mut Cx) + 'static) -> Self {
        self.on_right_click = Some(Rc::new(f));
        self
    }
    pub fn on_middle_click(mut self, f: impl Fn(&mut S, &mut Cx) + 'static) -> Self {
        self.on_middle_click = Some(Rc::new(f));
        self
    }
    pub fn on_double_click(mut self, f: impl Fn(&mut S, &mut Cx) + 'static) -> Self {
        self.on_double_click = Some(Rc::new(f));
        self
    }
    /// Pressing here moves the window; double-clicking toggles maximize.
    pub fn window_drag(mut self) -> Self {
        self.window_drag = true;
        self
    }
    /// Clicking this element (it needs an `id`) gives it keyboard focus.
    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }
    /// Vertical scrolling; needs an `id` to remember the offset.
    pub fn scroll_y(mut self) -> Self {
        self.scroll = true;
        self.clip = true;
        self.layout.overflow.y = taffy::Overflow::Scroll;
        self.layout.overflow.x = taffy::Overflow::Hidden;
        self.layout.scrollbar_width = 0.0;
        self
    }
    pub fn clip(mut self) -> Self {
        self.clip = true;
        self.layout.overflow = taffy::Point { x: taffy::Overflow::Hidden, y: taffy::Overflow::Hidden };
        self
    }
    /// Paint above everything else (menus, popovers).
    pub fn deferred(mut self) -> Self {
        self.deferred = true;
        self
    }
    /// Swallow hover and clicks so nothing underneath reacts.
    pub fn occlude(mut self) -> Self {
        self.occlude = true;
        self
    }
}

impl<S> From<Div<S>> for AnyElement<S> {
    fn from(d: Div<S>) -> Self {
        AnyElement::Div(d)
    }
}

// ---------------------------------------------------------------------------
// text

pub struct Text {
    pub(crate) content: String,
    pub(crate) overrides: TextOverrides,
    pub(crate) truncate: bool,
    pub(crate) tabular: bool,
    pub(crate) layout: Style,
}

pub fn text(s: impl Into<String>) -> Text {
    Text { content: s.into(), overrides: TextOverrides::default(), truncate: false, tabular: false, layout: Style::default() }
}

impl Styled for Text {
    fn layout_mut(&mut self) -> &mut Style {
        &mut self.layout
    }
}

impl Text {
    pub fn size(mut self, v: f32) -> Self {
        self.overrides.size = Some(v);
        self
    }
    pub fn weight(mut self, w: Weight) -> Self {
        self.overrides.weight = Some(w);
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.overrides.color = Some(c);
        self
    }
    pub fn line_height(mut self, v: f32) -> Self {
        self.overrides.line_height = Some(v);
        self
    }
    /// Fixed-width figures, for numbers that line up in columns or change
    /// in place (times, sizes, percentages, a calculator display).
    pub fn tabular(mut self) -> Self {
        self.tabular = true;
        self
    }
    /// Single line, cut off with an ellipsis.
    pub fn truncate(mut self) -> Self {
        self.truncate = true;
        self.layout.min_size.width = LengthPercentageAuto::length(0.0);
        self.layout.flex_shrink = 1.0;
        self
    }
}

impl<S> From<Text> for AnyElement<S> {
    fn from(t: Text) -> Self {
        AnyElement::Text(t)
    }
}

// ---------------------------------------------------------------------------
// input

pub struct InputEl {
    pub(crate) id: ElementId,
    pub(crate) state: Rc<RefCell<TextInput>>,
    pub(crate) layout: Style,
}

pub fn input(id: impl Hash, state: &Rc<RefCell<TextInput>>) -> InputEl {
    InputEl { id: eid(id), state: state.clone(), layout: Style::default() }
}

impl Styled for InputEl {
    fn layout_mut(&mut self) -> &mut Style {
        &mut self.layout
    }
}

impl<S> From<InputEl> for AnyElement<S> {
    fn from(i: InputEl) -> Self {
        AnyElement::Input(i)
    }
}

// ---------------------------------------------------------------------------
// canvas

pub struct Canvas {
    pub(crate) paint: Box<dyn Fn(&mut Scene, Bounds)>,
    pub(crate) layout: Style,
}

pub fn canvas(paint: impl Fn(&mut Scene, Bounds) + 'static) -> Canvas {
    Canvas { paint: Box::new(paint), layout: Style::default() }
}

impl Styled for Canvas {
    fn layout_mut(&mut self) -> &mut Style {
        &mut self.layout
    }
}

impl<S> From<Canvas> for AnyElement<S> {
    fn from(c: Canvas) -> Self {
        AnyElement::Canvas(c)
    }
}

// ---------------------------------------------------------------------------
// layout + paint

enum Measure {
    Text { content: String, style: TextStyle, truncate: bool, tabular: bool },
    Input(Rc<RefCell<TextInput>>),
}

pub struct Hitbox<S> {
    pub id: Option<ElementId>,
    pub bounds: Bounds,
    pub layer: usize,
    pub on_mouse_down: Option<Handler<S>>,
    pub on_click: Option<Handler<S>>,
    pub on_right_click: Option<Handler<S>>,
    pub on_middle_click: Option<Handler<S>>,
    pub on_double_click: Option<Handler<S>>,
    pub window_drag: bool,
    pub focusable: bool,
    pub cursor: Option<CursorIcon>,
    pub input: Option<(Rc<RefCell<TextInput>>, Bounds)>,
    pub widget: Option<(Rc<RefCell<dyn Widget>>, Bounds)>,
}

pub struct ScrollRegion {
    pub id: ElementId,
    pub bounds: Bounds,
    pub layer: usize,
}

/// What a painted frame leaves behind for event dispatch.
pub struct Frame<S> {
    pub hitboxes: Vec<Hitbox<S>>,
    pub scrolls: Vec<ScrollRegion>,
}

impl<S> Default for Frame<S> {
    fn default() -> Self {
        Self { hitboxes: Vec::new(), scrolls: Vec::new() }
    }
}

impl<S> Frame<S> {
    /// Hitboxes under `p` on the topmost layer that has any, innermost first.
    pub fn hits(&self, p: Point) -> Vec<&Hitbox<S>> {
        let top = self.hitboxes.iter().filter(|h| h.bounds.contains(p)).map(|h| h.layer).max();
        let Some(top) = top else { return Vec::new() };
        self.hitboxes.iter().rev().filter(|h| h.layer == top && h.bounds.contains(p)).collect()
    }
}

fn build<S>(el: &AnyElement<S>, tree: &mut TaffyTree<Measure>, inherited: TextStyle) -> NodeId {
    match el {
        AnyElement::Div(d) => {
            let style = d.text.apply(inherited);
            let children: Vec<NodeId> = d.children.iter().map(|c| build(c, tree, style)).collect();
            if d.scroll {
                // Rows in a scrolling list keep their height instead of
                // squeezing to fit the viewport.
                for &c in &children {
                    let mut s = tree.style(c).unwrap().clone();
                    s.flex_shrink = 0.0;
                    tree.set_style(c, s).unwrap();
                }
            }
            tree.new_with_children(d.layout.clone(), &children).unwrap()
        }
        AnyElement::Text(t) => {
            let style = t.overrides.apply(inherited);
            let m = Measure::Text { content: t.content.clone(), style, truncate: t.truncate, tabular: t.tabular };
            tree.new_leaf_with_context(t.layout.clone(), m).unwrap()
        }
        AnyElement::Input(i) => tree.new_leaf_with_context(i.layout.clone(), Measure::Input(i.state.clone())).unwrap(),
        AnyElement::Canvas(c) => tree.new_leaf(c.layout.clone()).unwrap(),
        AnyElement::Widget(w) => tree.new_leaf(w.layout.clone()).unwrap(),
    }
}

struct PaintCx<'a, S> {
    cx: &'a mut Cx,
    tree: &'a TaffyTree<Measure>,
    scene: &'a mut Scene,
    frame: &'a mut Frame<S>,
    /// Scroll containers enclosing the element being painted (id, viewport).
    scroll_stack: Vec<(ElementId, Bounds)>,
    /// Caret reveal requests: (scroll container, viewport, caret rect).
    reveals: Vec<(ElementId, Bounds, Bounds)>,
}

/// Lays out and paints `root` into `scene`, returning the hitboxes.
pub fn render_tree<S>(root: &AnyElement<S>, cx: &mut Cx, scene: &mut Scene, base_text: TextStyle) -> Frame<S> {
    let mut tree: TaffyTree<Measure> = TaffyTree::new();
    let root_node = build(root, &mut tree, base_text);
    let (w, h) = cx.window_size;
    {
        let fonts = &mut cx.text;
        tree.compute_layout_with_measure(
            root_node,
            taffy::Size { width: AvailableSpace::Definite(w), height: AvailableSpace::Definite(h) },
            |inputs, _node, ctx, style| {
                // Taffy hands leaves to us entirely; let it apply size/padding
                // and only ask us for the content size.
                taffy::compute_leaf_layout(inputs, style, |_, _| 0.0, |known, avail| {
                    let Some(ctx) = ctx else { return taffy::Size::ZERO };
                    let width = known.width.or(match avail.width {
                        AvailableSpace::Definite(w) => Some(w),
                        _ => None,
                    });
                    let (w, h) = match ctx {
                        Measure::Text { content, style, truncate, tabular } => {
                            if *truncate {
                                let (mw, mh) = fonts.measure_with(content, style, None, true, *tabular);
                                let w = match avail.width {
                                    AvailableSpace::MinContent => 0.0,
                                    _ => width.map_or(mw, |w| mw.min(w)),
                                };
                                (w, mh)
                            } else {
                                fonts.measure_with(content, style, width, false, *tabular)
                            }
                        }
                        Measure::Input(state) => state.borrow_mut().measure(&mut fonts.fonts, width),
                    };
                    taffy::Size { width: known.width.unwrap_or(w), height: known.height.unwrap_or(h) }
                })
            },
        )
        .unwrap();
    }

    let mut frame = Frame::default();
    let mut deferred: Vec<(&Div<S>, NodeId, Point, TextStyle)> = Vec::new();
    let mut pcx = PaintCx { cx, tree: &tree, scene, frame: &mut frame, scroll_stack: Vec::new(), reveals: Vec::new() };
    pcx.scene.set_layer(0);
    paint(root, root_node, Point::default(), base_text, &mut pcx, &mut deferred, false);

    // Overlays: each gets its own layer above the main content, unclipped.
    for (layer, (d, node, origin, style)) in std::mem::take(&mut deferred).into_iter().enumerate() {
        pcx.scene.set_layer(layer + 1);
        let clips = pcx.scene.take_clips();
        let scrolls = std::mem::take(&mut pcx.scroll_stack);
        paint_div(d, node, origin, style, &mut pcx, &mut deferred, true);
        pcx.scene.restore_clips(clips);
        pcx.scroll_stack = scrolls;
    }
    pcx.scene.set_layer(0);

    for (id, viewport, caret) in std::mem::take(&mut pcx.reveals) {
        let off = pcx.cx.scroll_offset(id);
        let margin = 24.0;
        let new = if caret.bottom() + margin > viewport.bottom() {
            off + caret.bottom() + margin - viewport.bottom()
        } else if caret.y - margin < viewport.y {
            off - (viewport.y - (caret.y - margin))
        } else {
            off
        };
        if (new - off).abs() > 0.5 {
            pcx.cx.set_scroll_offset(id, new);
            pcx.cx.request_redraw();
        }
    }
    frame
}

fn abs_bounds(tree: &TaffyTree<Measure>, node: NodeId, origin: Point) -> Bounds {
    let l = tree.layout(node).unwrap();
    Bounds::new(origin.x + l.location.x, origin.y + l.location.y, l.size.width, l.size.height)
}

/// Like `abs_bounds`, minus padding and border: where a leaf's content goes.
fn content_bounds(tree: &TaffyTree<Measure>, node: NodeId, origin: Point) -> Bounds {
    let b = abs_bounds(tree, node, origin);
    let l = tree.layout(node).unwrap();
    let (left, top) = (l.padding.left + l.border.left, l.padding.top + l.border.top);
    let (right, bottom) = (l.padding.right + l.border.right, l.padding.bottom + l.border.bottom);
    Bounds::new(b.x + left, b.y + top, (b.w - left - right).max(0.0), (b.h - top - bottom).max(0.0))
}

fn paint<'e, S>(
    el: &'e AnyElement<S>,
    node: NodeId,
    origin: Point,
    style: TextStyle,
    pcx: &mut PaintCx<'_, S>,
    deferred: &mut Vec<(&'e Div<S>, NodeId, Point, TextStyle)>,
    in_deferred: bool,
) {
    match el {
        AnyElement::Div(d) => {
            if d.deferred && !in_deferred {
                deferred.push((d, node, origin, style));
                return;
            }
            paint_div(d, node, origin, style, pcx, deferred, false);
        }
        AnyElement::Text(t) => {
            let b = content_bounds(pcx.tree, node, origin);
            if b.intersect(&pcx.scene.clip()).h <= 0.0 {
                return;
            }
            let style = t.overrides.apply(style);
            let scale = pcx.cx.scale;
            let origin = Point { x: b.x, y: b.y };
            pcx.cx.text.paint_with(pcx.scene, &t.content, &style, Some(b.w), t.truncate, t.tabular, origin, scale);
        }
        AnyElement::Input(i) => {
            let outer = abs_bounds(pcx.tree, node, origin);
            let b = content_bounds(pcx.tree, node, origin);
            let focused = pcx.cx.focused == Some(i.id);
            let caret_on = pcx.cx.caret_visible(&i.state.borrow());
            let window_active = pcx.cx.window_focused;
            let scale = pcx.cx.scale;
            let mut state = i.state.borrow_mut();
            let caret = state.paint(
                &mut pcx.cx.text.fonts,
                &mut pcx.cx.text.atlas,
                pcx.scene,
                b,
                focused,
                caret_on,
                window_active,
                scale,
            );
            if focused && state.reveal_caret {
                state.reveal_caret = false;
                if let (Some(caret), Some(&(id, viewport))) = (caret, pcx.scroll_stack.last()) {
                    pcx.reveals.push((id, viewport, caret));
                }
            }
            if focused {
                if let Some(c) = caret {
                    pcx.cx.ime_area = Some(c);
                }
            }
            let clip = pcx.scene.clip();
            pcx.frame.hitboxes.push(Hitbox {
                id: Some(i.id),
                bounds: outer.intersect(&clip),
                layer: pcx.scene.layer(),
                on_mouse_down: None,
                on_click: None,
                on_right_click: None,
                on_middle_click: None,
                on_double_click: None,
                window_drag: false,
                focusable: true,
                cursor: Some(CursorIcon::Text),
                input: Some((i.state.clone(), b)),
                widget: None,
            });
        }
        AnyElement::Canvas(c) => {
            let b = abs_bounds(pcx.tree, node, origin);
            (c.paint)(pcx.scene, b);
        }
        AnyElement::Widget(w) => {
            let b = abs_bounds(pcx.tree, node, origin);
            let focused = pcx.cx.focused == Some(w.id);
            let caret = w.state.borrow_mut().paint(pcx.cx, pcx.scene, b, focused);
            if focused && caret.is_some() {
                pcx.cx.ime_area = caret;
            }
            let clip = pcx.scene.clip();
            pcx.frame.hitboxes.push(Hitbox {
                id: Some(w.id),
                bounds: b.intersect(&clip),
                layer: pcx.scene.layer(),
                on_mouse_down: None,
                on_click: None,
                on_right_click: None,
                on_middle_click: None,
                on_double_click: None,
                window_drag: false,
                focusable: true,
                cursor: w.cursor,
                input: None,
                widget: Some((w.state.clone(), b)),
            });
        }
    }
}

fn paint_div<'e, S>(
    d: &'e Div<S>,
    node: NodeId,
    origin: Point,
    style: TextStyle,
    pcx: &mut PaintCx<'_, S>,
    deferred: &mut Vec<(&'e Div<S>, NodeId, Point, TextStyle)>,
    in_deferred: bool,
) {
    let b = abs_bounds(pcx.tree, node, origin);
    let style = d.text.apply(style);
    let hovered = d.id.is_some_and(|id| pcx.cx.is_hovered(id));
    let faded = (d.opacity < 1.0).then(|| pcx.scene.mark());

    if let Some((c, blur, dy)) = d.shadow {
        pcx.scene.shadow(Bounds::new(b.x, b.y + dy, b.w, b.h), c, d.radius, blur);
    }
    if let Some((c, w)) = d.ring {
        let r = Bounds::new(b.x - w, b.y - w, b.w + 2.0 * w, b.h + 2.0 * w);
        pcx.scene.rect_bordered(r, Color::TRANSPARENT, d.radius + w, c, w);
    }
    let fill = if hovered { d.hover_bg.or(d.bg) } else { d.bg };
    match d.border {
        Some((bc, bw)) => pcx.scene.rect_bordered(b, fill.unwrap_or(Color::TRANSPARENT), d.radius, bc, bw),
        None => {
            if let Some(f) = fill {
                pcx.scene.rect(b, f, d.radius);
            }
        }
    }
    if let Some(c) = d.border_bottom {
        let px = 1.0 / pcx.cx.scale.max(1.0);
        pcx.scene.rect(Bounds::new(b.x, b.bottom() - px, b.w, px), c, 0.0);
    }

    let interactive = d.on_mouse_down.is_some()
        || d.on_click.is_some()
        || d.on_right_click.is_some()
        || d.on_middle_click.is_some()
        || d.on_double_click.is_some()
        || d.window_drag
        || d.focusable
        || d.cursor.is_some()
        || d.occlude
        || d.hover_bg.is_some();
    if interactive {
        let clip = pcx.scene.clip();
        pcx.frame.hitboxes.push(Hitbox {
            id: d.id,
            bounds: b.intersect(&clip),
            layer: pcx.scene.layer(),
            on_mouse_down: d.on_mouse_down.clone(),
            on_click: d.on_click.clone(),
            on_right_click: d.on_right_click.clone(),
            on_middle_click: d.on_middle_click.clone(),
            on_double_click: d.on_double_click.clone(),
            window_drag: d.window_drag,
            focusable: d.focusable,
            cursor: d.cursor,
            input: None,
            widget: None,
        });
    }

    let mut child_origin = Point { x: b.x, y: b.y };
    if d.clip {
        pcx.scene.push_clip(b);
    }
    if let (true, Some(id)) = (d.scroll, d.id) {
        let layout = pcx.tree.layout(node).unwrap();
        let mut content_h: f32 = 0.0;
        for child in pcx.tree.children(node).unwrap() {
            let l = pcx.tree.layout(child).unwrap();
            content_h = content_h.max(l.location.y + l.size.height);
        }
        content_h += layout.padding.bottom;
        let max = (content_h - b.h).max(0.0);
        let off = pcx.cx.clamp_scroll(id, max);
        let clip = pcx.scene.clip();
        let over = pcx.cx.note_region(id, b.intersect(&clip), content_h, max);
        child_origin.y -= off + over;
        pcx.frame.scrolls.push(ScrollRegion { id, bounds: b.intersect(&clip), layer: pcx.scene.layer() });
        pcx.scroll_stack.push((id, b));
    }

    let children = pcx.tree.children(node).unwrap();
    for (child, child_node) in d.children.iter().zip(children) {
        paint(child, child_node, child_origin, style, pcx, deferred, in_deferred);
    }

    if let (true, Some(id)) = (d.scroll, d.id) {
        pcx.cx.paint_scrollbar(pcx.scene, id);
        pcx.scroll_stack.pop();
    }
    if d.clip {
        pcx.scene.pop_clip();
    }
    if let Some(mark) = faded {
        pcx.scene.fade_since(mark, d.opacity);
    }
}
