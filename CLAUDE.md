# y7ya-os

A Mac-inspired desktop for Linux (Arch, KDE Plasma today) plus its apps, in one
Cargo workspace. Read `docs/design.md` before building any UI.

## Layout

- `desktop/` — the compositor (Smithay). `y7-desktop` runs nested in a window;
  `y7-desktop --headless 1440x900 --scale 2` renders offscreen and reads commands
  on stdin: `spawn <cmd>`, `move x y`, `click x y`, `down`, `up`, `key super+m`,
  `shot file.png`, `quit`. `Y7_THEME=light|dark` overrides the theme.
- `crates/ui/` — our GPUI-style UI framework (winit + wgpu + cosmic-text + taffy).
  Every app is built on it. `apps/notes` is the reference app: copy its patterns.
- `crates/notes-core/` — Automerge notebook + iroh peer-to-peer sync; `crates/notes-ffi/` exposes it to Kotlin.
- `apps/<name>/` — one crate per app (binary named after the app).
- `android/` — the Android companion app (Kotlin + Compose), see its `build-rust.sh`.

## Commands

Cargo isn't on PATH by default: `export PATH=$HOME/.cargo/bin:$PATH`.

- Build/test everything: `cargo build --workspace`, `cargo test --workspace`
- Offscreen render of an app (no window): `cargo run -p notes -- --snapshot /tmp/x.png [--light]`
- Android toolchain: `JAVA_HOME=$HOME/.local/share/jdk-21`, `ANDROID_HOME=$HOME/Android/Sdk`

## Rules

- Never capture the user's real screen. Verify visuals with `--snapshot` renders or
  the headless desktop's `shot` command, and look at the PNGs.
- Battery matters: no busy loops or continuous redraws. Redraw only on change; use
  `App::next_wake` for timers.
- Match the surrounding code: comment density, naming, idioms. Don't add
  dependencies without a reason.
- Adding to `crates/ui`: put new widgets/features in new files (e.g.
  `crates/ui/src/widgets/<name>.rs`) and keep edits to existing files small, since
  several apps grow the framework in parallel.
- App data lives in `~/.local/share/<app>/`, overridable with `<APP>_DATA_DIR`.
- Each app ships `apps/<name>/<name>.desktop`.
