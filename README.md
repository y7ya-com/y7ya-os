# Notes

A lightweight, Apple Notes–style notes app for Linux, written in Rust on a tiny
GPUI-inspired UI framework built from scratch for it.

![Notes, dark](docs/screenshot-dark.png)

- **Small and fast**: a single ~15 MB binary, first frame in about 0.1 s, about 50 MB
  of RAM, 0% CPU when idle (it only redraws when something changes).
- **Seamless window**: no title bar. The app draws your desktop's window buttons
  in its own header, in the order set in KDE's `kwinrc`.
- Folders, pinned notes, search, Recently Deleted (kept for 30 days), undo/redo,
  light and dark themes (follows the system).
- Notes autosave to `~/.local/share/notes/notes.json` (override with `NOTES_DATA_DIR`).

## Build and install

```sh
cargo build --release
install -Dm755 target/release/notes ~/.local/bin/notes
install -Dm644 notes.desktop ~/.local/share/applications/notes.desktop
```

Needs a GPU with Vulkan (falls back to OpenGL ES). Tested on KDE Plasma (Wayland).

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| Ctrl+N | New note |
| Ctrl+Shift+N | New folder |
| Ctrl+F | Search |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| ↑ / ↓, Delete | Move through / delete notes (when the list is focused) |
| Ctrl+S | Save now |
| Ctrl+W / Ctrl+Q | Close |

## How it's built

`src/ui` is a ~3,000-line immediate-mode UI framework in the spirit of
[GPUI](https://www.gpui.rs): the element tree is rebuilt every frame, laid out with
[taffy](https://github.com/DioxusLabs/taffy) flexbox, and drawn on the GPU by
[wgpu](https://wgpu.rs) with a single instanced pipeline (SDF rounded rects,
shadows, line icons and glyphs from an atlas). Text shaping and editing use
[cosmic-text](https://github.com/pop-os/cosmic-text); windowing uses
[winit](https://github.com/rust-windowing/winit).

`src/notes` is the app itself: the data model, theme, icons and views.

`notes --snapshot out.png [--light]` renders the UI with sample data offscreen.

## Fonts

Bundles [Inter](https://rsms.me/inter/) under the SIL Open Font License
(`assets/fonts/OFL.txt`).
