//! Renders offscreen and takes scripted input from stdin, for automated
//! tests and screenshots without touching a real screen.
//!
//! Commands, one per line: `spawn <cmd>`, `move <x> <y>`, `click <x> <y>`,
//! `down`, `up`, `key <mods+key>` (e.g. `key super+m`), `shot <file.png>`, `quit`.

use std::cell::RefCell;
use std::io::BufRead;
use std::rc::Rc;

use smithay::backend::allocator::Fourcc;
use smithay::backend::egl::native::EGLSurfacelessDisplay;
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::{ButtonState, KeyState};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::input::keyboard::Keycode;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::channel::{Event as ChannelEvent, channel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{Rectangle, Transform};

use crate::render::{self, Look};
use crate::state::Desk;

const BTN_LEFT: u32 = 0x110;

struct Headless {
    renderer: GlesRenderer,
    target: GlesTexture,
    look: Look,
    damage: OutputDamageTracker,
    size: (i32, i32),
}

impl Headless {
    fn render(&mut self, desk: &mut Desk) {
        let Ok(mut fb) = self.renderer.bind(&mut self.target) else { return };
        let elements = render::elements(desk, &mut self.look, &mut self.renderer);
        // The target keeps its contents, so only damage needs redrawing.
        if let Err(e) = self.damage.render_output(&mut self.renderer, &mut fb, 1, &elements, [0.0, 0.0, 0.0, 1.0]) {
            tracing::warn!("render failed: {e:?}");
        }
        drop(fb);
        render::send_frames(desk);
        desk.space.refresh();
        desk.popups.cleanup();
        let _ = desk.display_handle.flush_clients();
    }
}

pub fn init(event_loop: &mut EventLoop<'static, Desk>, desk: &mut Desk, size: (i32, i32), scale: f64) -> anyhow::Result<()> {
    let display = unsafe { EGLDisplay::new(EGLSurfacelessDisplay)? };
    let context = EGLContext::new(&display)?;
    let mut renderer = unsafe { GlesRenderer::new(context)? };
    let output = super::add_output(desk, "headless", size, scale, Transform::Normal);
    let look = Look::new(&mut renderer)?;
    let damage = OutputDamageTracker::from_output(&output);
    let phys = ((size.0 as f64 * scale) as i32, (size.1 as f64 * scale) as i32);
    let target: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, phys.into())?;
    let state = Rc::new(RefCell::new(Headless { renderer, target, look, damage, size: phys }));

    let (tx, rx) = channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut time = 0u32;
    let st = state.clone();
    event_loop
        .handle()
        .insert_source(rx, move |event, _, desk: &mut Desk| {
            let ChannelEvent::Msg(line) = event else { return };
            time += 10;
            let parts: Vec<&str> = line.split_whitespace().collect();
            let num = |i: usize| parts.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            match parts.first().copied() {
                Some("spawn") => desk.spawn(&parts[1..].join(" ")),
                Some("move") => desk.pointer_to((num(1), num(2)).into(), time),
                Some("click") => {
                    desk.pointer_to((num(1), num(2)).into(), time);
                    desk.pointer_button(BTN_LEFT, ButtonState::Pressed, time);
                    desk.pointer_button(BTN_LEFT, ButtonState::Released, time + 5);
                }
                Some("down") => desk.pointer_button(BTN_LEFT, ButtonState::Pressed, time),
                Some("up") => desk.pointer_button(BTN_LEFT, ButtonState::Released, time),
                Some("key") => press_combo(desk, parts.get(1).copied().unwrap_or(""), time),
                Some("quit") => desk.loop_signal.stop(),
                Some("shot") => {
                    let path = parts.get(1).copied().unwrap_or("shot.png");
                    let mut h = st.borrow_mut();
                    // Make sure the picture is current.
                    h.render(desk);
                    let size = h.size;
                    let Headless { renderer, target, .. } = &mut *h;
                    match shot(renderer, target, size, path) {
                        Ok(()) => println!("saved {path}"),
                        Err(e) => eprintln!("screenshot failed: {e}"),
                    }
                }
                _ => eprintln!("unknown command: {line}"),
            }
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // A 60 Hz clock that only renders when something changed.
    event_loop
        .handle()
        .insert_source(Timer::immediate(), move |_, _, desk: &mut Desk| {
            if desk.needs_redraw {
                desk.needs_redraw = false;
                state.borrow_mut().render(desk);
            }
            TimeoutAction::ToDuration(std::time::Duration::from_millis(16))
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

fn shot(renderer: &mut GlesRenderer, target: &mut GlesTexture, size: (i32, i32), path: &str) -> anyhow::Result<()> {
    let fb = renderer.bind(target)?;
    let mapping = renderer.copy_framebuffer(&fb, Rectangle::from_size(size.into()), Fourcc::Abgr8888)?;
    drop(fb);
    let data = renderer.map_texture(&mapping)?;
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size.0 as u32, size.1 as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(data)?;
    Ok(())
}

/// Presses and releases a key combo like `super+shift+m`.
fn press_combo(desk: &mut Desk, combo: &str, time: u32) {
    let codes: Vec<u32> = combo.split('+').filter_map(evdev_code).collect();
    for &c in &codes {
        desk.key(Keycode::new(c + 8), KeyState::Pressed, time);
    }
    for &c in codes.iter().rev() {
        desk.key(Keycode::new(c + 8), KeyState::Released, time + 1);
    }
}

fn evdev_code(name: &str) -> Option<u32> {
    Some(match name.to_lowercase().as_str() {
        "super" | "logo" => 125,
        "shift" => 42,
        "ctrl" => 29,
        "alt" => 56,
        "tab" => 15,
        "enter" | "return" => 28,
        "space" => 57,
        "backspace" => 14,
        "esc" => 1,
        k if k.len() == 1 => {
            let c = k.as_bytes()[0];
            const ROW: &[(u8, u32)] = &[
                (b'q', 16), (b'w', 17), (b'e', 18), (b'r', 19), (b't', 20), (b'y', 21), (b'u', 22), (b'i', 23),
                (b'o', 24), (b'p', 25), (b'a', 30), (b's', 31), (b'd', 32), (b'f', 33), (b'g', 34), (b'h', 35),
                (b'j', 36), (b'k', 37), (b'l', 38), (b'z', 44), (b'x', 45), (b'c', 46), (b'v', 47), (b'b', 48),
                (b'n', 49), (b'm', 50),
            ];
            ROW.iter().find(|(k, _)| *k == c)?.1
        }
        _ => return None,
    })
}
