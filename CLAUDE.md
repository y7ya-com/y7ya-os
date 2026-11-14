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
- `crates/y7-sync/` — peer-to-peer sync for every app: one device identity and peer
  list, many Automerge documents multiplexed per connection (iroh). The `y7-syncd`
  daemon (`crates/y7-syncd/`) runs it on the desktop; apps are `y7_sync::Client`s of
  it over `$XDG_RUNTIME_DIR/y7-sync.sock`. Android embeds the engine in-process.
- `crates/sync-ui/` — the shared "Sync with Phone" row and pairing sheet. Never
  duplicate pairing in an app.
- `crates/{notes,calendar,reminders}-core/` — each app's document schema on top of
  y7-sync; `crates/notes-ffi/` exposes Notes to Kotlin.
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
  Synced documents live in the daemon's `~/.local/share/y7-sync/` instead
  (`Y7_SYNC_DATA_DIR`, socket `Y7_SYNC_SOCKET`); tests use temp dirs and sockets.
- Each app ships `apps/<name>/<name>.desktop`.
