# Notes

A lightweight, Apple Notes–style notes app for Linux, written in Rust on a tiny
GPUI-inspired UI framework built from scratch for it, plus an iPhone Notes–style
Android app. The two sync directly with each other: no account, no server.

![Notes, dark](docs/screenshot-dark.png)

- **Small and fast**: a single ~15 MB binary, first frame in about 0.1 s, about 50 MB
  of RAM, 0% CPU when idle (it only redraws when something changes).
- **Seamless window**: no title bar. The app draws your desktop's window buttons
  in its own header, in the order set in KDE's `kwinrc`.
- Folders, pinned notes, search, Recently Deleted (kept for 30 days), undo/redo,
  light and dark themes (follows the system).
- **Syncs with your Android phone**, peer to peer and end-to-end encrypted (see below).
- Notes autosave to `~/.local/share/notes/` (override with `NOTES_DATA_DIR`).

## Build and install

```sh
cargo build --release
install -Dm755 target/release/notes ~/.local/bin/notes
install -Dm644 notes.desktop ~/.local/share/applications/notes.desktop
```

Needs a GPU with Vulkan (falls back to OpenGL ES). Tested on KDE Plasma (Wayland).

## Android app

`android/` is a native Kotlin + Jetpack Compose app laid out like iPhone Notes
(folders, date-grouped notes, swipe actions, a bold first-line title). It shares
the Rust core with the desktop app through [UniFFI](https://mozilla.github.io/uniffi-rs/).

```sh
cd android
./build-rust.sh            # builds the core for Android, generates Kotlin bindings
./gradlew assembleRelease  # app/build/outputs/apk/release/app-release.apk
```

Needs JDK 21, the Android SDK + NDK and [`cargo-ndk`](https://github.com/bbqsrc/cargo-ndk).
Release builds are signed with the key in `android/keystore.properties` (see the
`.example`), or the debug key if there isn't one.

## Sync

On the computer click **Sync with Phone** (bottom left) and scan the QR code from
the phone app (**Sync with Computer**). After that the devices sync whenever both
apps are open, usually within a fraction of a second.

- **No cloud.** Devices connect directly with [iroh](https://iroh.computer): on the
  same Wi-Fi via mDNS, otherwise via UDP hole punching. If no direct path exists,
  traffic goes through one of n0's public relays, which only ever see encrypted
  bytes. Nothing is stored anywhere but your devices, so both need to be online
  at the same time to sync.
- **Secure.** Each device has its own Ed25519 key; connections are QUIC + TLS 1.3
  authenticated by those keys, and a device only talks to the keys it paired with.
  The QR code holds a one-time secret that expires after 10 minutes.
- **No lost edits.** Notes live in an [Automerge](https://automerge.org) document,
  a CRDT: edits made on both devices while apart, even to the same note, merge
  character by character.

`crates/notes-core` holds the notebook and sync; `crates/notes-ffi` exposes it to
Kotlin. `cargo run -p notes-core --example peer -- <dir>` runs a headless device
for testing.

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

`src/notes` is the desktop app itself: the views, theme and icons, on top of the
shared notebook in `crates/notes-core`.

`notes --snapshot out.png [--light]` renders the UI with sample data offscreen.

## Fonts

Bundles [Inter](https://rsms.me/inter/) under the SIL Open Font License
(`assets/fonts/OFL.txt`).
