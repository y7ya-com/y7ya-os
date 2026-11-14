//! Control Center: connectivity, Focus and appearance toggles, display and
//! sound sliders, and Now Playing, as rounded modules in a top-right panel.

use ui::*;

use super::theme::Theme;
use super::{Shell, Slider, icons};
use crate::system::{MediaAction, Request};

const WIDTH: f32 = 320.0;
const PAD: f32 = 10.0;
const HALF: f32 = (WIDTH - 3.0 * PAD) / 2.0;

type Glyph = Box<dyn Fn(&mut Scene, Bounds, Color)>;

impl Shell {
    pub(super) fn control_center(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let (w, _) = cx.window_size;
        let st = self.status.clone();
        let dnd = self.store.lock().unwrap().dnd;
        let module = || div().rounded(16.0).bg(t.module).flex_col();

        // Wi-Fi and Bluetooth share the big module on the left.
        let wifi_on = st.wifi.enabled;
        let wifi_detail = match (&st.wifi.ssid, st.wifi.available, wifi_on) {
            (_, false, _) => "Not Available".to_string(),
            (_, _, false) => "Off".to_string(),
            (Some(ssid), ..) => ssid.clone(),
            (None, ..) => "Not Connected".to_string(),
        };
        let bt = st.bluetooth;
        let connectivity = module()
            .w(HALF)
            .h(HALF * 0.9)
            .px(12.0)
            .justify_center()
            .gap(14.0)
            .child(self.toggle(
                "cc-wifi",
                "Wi-Fi",
                wifi_detail,
                wifi_on,
                st.wifi.available,
                Box::new(|s, b, c| icons::wifi(s, b, c, c.alpha(0.35), 100, true)),
                t,
                |s: &mut Self, _: &mut Cx| {
                    let on = !s.status.wifi.enabled;
                    s.status.wifi.enabled = on;
                    s.services.optimistic(|st| st.wifi.enabled = on);
                    s.send(Request::SetWifi(on));
                },
            ))
            .child(self.toggle(
                "cc-bluetooth",
                "Bluetooth",
                match bt {
                    None => "Not Available".into(),
                    Some(true) => "On".into(),
                    Some(false) => "Off".into(),
                },
                bt == Some(true),
                bt.is_some(),
                Box::new(icons::bluetooth),
                t,
                |s: &mut Self, _: &mut Cx| {
                    let on = s.status.bluetooth != Some(true);
                    s.status.bluetooth = Some(on);
                    s.services.optimistic(|st| st.bluetooth = Some(on));
                    s.send(Request::SetBluetooth(on));
                },
            ));

        let small = |id: &'static str, label: &str, detail: &str, on: bool, glyph: Glyph, f: fn(&mut Self, &mut Cx)| {
            module()
                .w(HALF)
                .flex_1()
                .px(12.0)
                .justify_center()
                .child(self.toggle(id, label, detail.to_string(), on, true, glyph, t, f))
        };
        let right = div()
            .flex_col()
            .gap(PAD)
            .h(HALF * 0.9)
            .child(small("cc-focus", "Focus", if dnd { "Do Not Disturb" } else { "Off" }, dnd, Box::new(icons::moon), |s, _| {
                let on = !s.store.lock().unwrap().dnd;
                s.set_dnd(on);
            }))
            .child(small("cc-dark", "Dark Mode", if cx.dark { "On" } else { "Off" }, cx.dark, Box::new(icons::appearance), |s, cx| {
                cx.dark = !cx.dark;
                s.send(Request::SetDark(cx.dark));
            }));

        let mut col = div()
            .absolute(w - WIDTH - 8.0, 2.0)
            .w(WIDTH)
            .flex_col()
            .gap(PAD)
            .p(PAD)
            .rounded(22.0)
            .bg(t.panel)
            .border(t.border, 0.5)
            .shadow(t.shadow, 30.0, 10.0)
            .occlude()
            .child(div().flex_row().gap(PAD).child(connectivity).child(right));

        if let Some(level) = st.brightness {
            col = col.child(self.slider_module("Display", Slider::Brightness, level, false, t));
        }
        if let Some(v) = st.volume {
            col = col.child(self.slider_module("Sound", Slider::Volume, if v.muted { 0.0 } else { v.level }, v.muted, t));
        }
        col.child(self.now_playing(t))
    }

    #[allow(clippy::too_many_arguments)]
    fn toggle(
        &self,
        id: &'static str,
        label: &str,
        detail: String,
        on: bool,
        enabled: bool,
        glyph: Glyph,
        t: &Theme,
        f: impl Fn(&mut Self, &mut Cx) + 'static,
    ) -> Div<Self> {
        let (bg, ink) = if on { (t.accent, Color::WHITE) } else { (t.fill.alpha(1.6), t.label) };
        let mut circle = div()
            .id(id)
            .size(30.0)
            .shrink_0()
            .rounded(15.0)
            .bg(bg)
            .items_center()
            .justify_center()
            .child(canvas(move |s, b| glyph(s, b, ink)).size(20.0));
        if enabled {
            circle = circle.on_click(f);
        }
        div()
            .flex_row()
            .items_center()
            .gap(9.0)
            .child(circle)
            .child(
                div()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(text(label.to_string()).size(12.5).weight(Weight::Semibold).truncate())
                    .child(text(detail).size(11.0).color(t.secondary).truncate()),
            )
    }

    fn slider_module(&self, title: &str, which: Slider, level: f32, muted: bool, t: &Theme) -> Div<Self> {
        let cell = self.sliders[which as usize].clone();
        let (track, fill, icon_on_fill, icon_off) = if t.dark {
            (rgba(0xffffff, 0.12), rgba(0xffffff, 0.92), rgb(0x3a3a3c), rgba(0xffffff, 0.75))
        } else {
            (rgba(0x000000, 0.13), Color::WHITE, rgba(0x000000, 0.6), rgba(0x000000, 0.6))
        };
        let knob_shadow = t.shadow;
        let paint = move |s: &mut Scene, b: Bounds| {
            cell.set(b);
            let r = b.h / 2.0;
            s.rect(b, track, r);
            // The fill always covers the icon end, like macOS.
            let fw = (b.h + (b.w - b.h) * level.clamp(0.0, 1.0)).min(b.w);
            s.rect(Bounds::new(b.x, b.y, fw, b.h), fill, r);
            let knob = Bounds::new(b.x + fw - b.h, b.y, b.h, b.h);
            s.shadow(knob, knob_shadow.alpha(0.6), r, 4.0);
            s.rect(knob, fill, r);
            let ib = Bounds::new(b.x + 2.0, b.y, b.h, b.h);
            let ic = if fw > b.h + 4.0 { icon_on_fill } else { icon_off };
            match which {
                Slider::Brightness => icons::sun(s, ib, ic),
                Slider::Volume => icons::speaker(s, Bounds::new(ib.x + 1.0, ib.y, ib.w, ib.h), ic, level, muted),
            }
        };
        div()
            .rounded(16.0)
            .bg(t.module)
            .flex_col()
            .gap(8.0)
            .px(12.0)
            .pt(10.0)
            .pb(12.0)
            .child(text(title.to_string()).size(12.5).weight(Weight::Semibold))
            .child(
                div()
                    .id(("cc-slider", which as usize))
                    .h(24.0)
                    .w_full()
                    .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                        s.dragging = Some(which);
                        s.drag(cx);
                    })
                    .child(canvas(paint).w_full().h(24.0)),
            )
    }

    /// Follows the pointer while a slider is held.
    pub(super) fn drag(&mut self, cx: &mut Cx) {
        let Some(which) = self.dragging else { return };
        if !cx.mouse_down {
            self.dragging = None;
            return;
        }
        let b = self.sliders[which as usize].get();
        if b.w <= b.h {
            return;
        }
        // The knob's center tracks the pointer.
        let v = ((cx.mouse.x - b.x - b.h / 2.0) / (b.w - b.h)).clamp(0.0, 1.0);
        match which {
            Slider::Brightness => {
                self.status.brightness = Some(v);
                self.send(Request::SetBrightness(v));
            }
            Slider::Volume => {
                if let Some(vol) = self.status.volume.as_mut() {
                    vol.level = v;
                    vol.muted = false;
                }
                self.send(Request::SetVolume(v));
            }
        }
        let status = self.status.clone();
        self.services.optimistic(|s| {
            s.brightness = status.brightness;
            s.volume = status.volume;
        });
        cx.request_redraw();
    }

    fn now_playing(&self, t: &Theme) -> Div<Self> {
        let media = self.status.media.clone();
        let (title, artist, playing) = match &media {
            Some(m) => (m.title.clone(), m.artist.clone(), m.playing),
            None => ("Not Playing".to_string(), String::new(), false),
        };
        let ink = if media.is_some() { t.label } else { t.tertiary };
        let button = |id: &'static str, glyph: Glyph, action: MediaAction| {
            let player = media.as_ref().map(|m| m.player.clone());
            let mut d = div()
                .id(id)
                .size(28.0)
                .rounded(14.0)
                .items_center()
                .justify_center()
                .child(canvas(move |s, b| glyph(s, b, ink)).size(20.0));
            if let Some(player) = player {
                d = d.hover_bg(t.hover).on_click(move |s: &mut Self, _: &mut Cx| {
                    if action == MediaAction::PlayPause {
                        if let Some(m) = s.status.media.as_mut() {
                            m.playing = !m.playing;
                        }
                    }
                    s.send(Request::Media(player.clone(), action));
                });
            }
            d
        };
        let art = div()
            .size(42.0)
            .shrink_0()
            .rounded(8.0)
            .bg(if media.is_some() { rgb(0xd9475a) } else { t.fill })
            .items_center()
            .justify_center()
            .child_if(media.is_some(), || {
                let white = Color::WHITE.alpha(0.9);
                canvas(move |s, b| {
                    // A beamed note.
                    let c = b.center();
                    s.rect(Bounds::new(c.x - 6.5, c.y + 2.5, 5.0, 4.0), white, 2.0);
                    s.rect(Bounds::new(c.x + 1.5, c.y + 0.5, 5.0, 4.0), white, 2.0);
                    s.segment(pt(c.x - 2.0, c.y + 4.0), pt(c.x - 2.0, c.y - 6.0), 1.4, white);
                    s.segment(pt(c.x + 6.0, c.y + 2.0), pt(c.x + 6.0, c.y - 8.0), 1.4, white);
                    s.segment(pt(c.x - 2.0, c.y - 6.0), pt(c.x + 6.0, c.y - 8.0), 2.0, white);
                })
                .size(24.0)
                .into()
            });
        div()
            .rounded(16.0)
            .bg(t.module)
            .flex_row()
            .items_center()
            .gap(10.0)
            .p(10.0)
            .child(art)
            .child(
                div()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(text(title).size(12.5).weight(Weight::Semibold).color(ink).truncate())
                    .child_if(!artist.is_empty(), || text(artist.clone()).size(11.0).color(t.secondary).truncate().into()),
            )
            .child(button("cc-prev", Box::new(|s, b, c| icons::skip(s, b, c, false)), MediaAction::Previous))
            .child(button(
                "cc-play",
                if playing { Box::new(icons::pause) } else { Box::new(icons::play) },
                MediaAction::PlayPause,
            ))
            .child(button("cc-next", Box::new(|s, b, c| icons::skip(s, b, c, true)), MediaAction::Next))
    }
}
