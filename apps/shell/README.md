# y7-shell

The y7ya-os system UI in one long-running process: a Windows 10-style taskbar
with Apple's materials and type (Start menu, app buttons, tray, clock, Show
Desktop), Spotlight, Quick Settings, notification banners and the clock's
calendar and notification flyout. It draws on
wlr-layer-shell surfaces through `ui::run_layer`, so it needs a compositor with
layer-shell (the y7ya-os desktop, or any wlroots/KWin session).

```sh
y7-shell                      # run it (one per session)
y7-shell msg <command>        # talk to the running shell
y7-shell --status             # print what the real machine reports (read-only)
y7-shell --snapshot <dir> [--light]   # render every surface to PNGs, offscreen
```

## Commands

`y7-shell msg <command>` writes one line to `$XDG_RUNTIME_DIR/y7-shell.sock`
(`Y7_SHELL_SOCKET` overrides it) and exits 0 on `ok`, 1 if the shell isn't
running or refused, 2 for an unknown command.

| Command | Effect |
|---|---|
| `start` | Toggle the Start menu (for the desktop to bind to Super and Ctrl+Esc) |
| `spotlight` | Toggle Spotlight (the desktop binds Super+Space to this) |
| `quick-settings` | Toggle Quick Settings (`control-center` still works) |
| `calendar` | Toggle the calendar and notifications (`notification-center` still works) |
| `show-desktop` | Minimize every window, or bring them back |
| `lock` | Lock the screen (`loginctl lock-session`) |
| `dismiss` | Close whatever menu, panel or dialog is open |
| `dnd on`, `dnd off`, `dnd toggle` | Do Not Disturb |
| `ping` | Succeeds if the shell is running |

Anything can speak the protocol: connect, send `spotlight\n`, read `ok\n`.

## What talks to what

- **Windows**: `zwlr_foreign_toplevel_manager_v1`, bound on the layer surfaces'
  own connection (a separate event queue and thread) so the taskbar can send
  each button's `set_rectangle` for the minimize animation. Without the
  protocol (KWin) the taskbar shows only pinned apps.
- **Taskbar order**: like Windows 10 with "never combine": pinned apps first,
  a running pinned app's windows in its place, then other apps' windows grouped
  by app. Pins live in `~/.local/share/y7-shell/taskbar-pins` and `start-pins`
  (one desktop-file ID per line); right-click to change them.
- **Icons**: app_id → `.desktop` → `Icon=` → icon themes (the user's
  `~/.local/share/icons` first, then hicolor/Breeze/Adwaita; PNG or SVG via
  resvg), decoded on a background thread. Apps without one get a letter tile.
- **Notifications**: serves `org.freedesktop.Notifications` on the session bus
  (actions, body, persistence). If another daemon owns the name it logs that and
  runs without banners.
- **Status**: battery from sysfs (UPower signals wake it), Wi-Fi via `nmcli`
  (NetworkManager signals, plus a 60 s check for signal strength), Bluetooth via
  `bluetoothctl`, sound via `wpctl` (woken by `pactl subscribe`), brightness via
  sysfs and `brightnessctl` or logind, Now Playing over MPRIS. All behind the
  `System` trait in `src/system.rs`.
- **Spotlight** (and the Start menu's search): apps from `.desktop` files in the XDG data dirs, files from an
  in-memory index of home (hidden, generated and huge folders skipped; refreshed
  incrementally by directory mtime each time Spotlight opens), a calculator and
  unit conversions.

The UI thread sleeps until something happens; the clock wakes it once a minute.
Flyouts slide and fade for 180 ms (140 ms out), ticking only while they move.

## Testing

`Y7_SHELL_FAKE=1` swaps the machine for an in-memory one, so clicking Wi-Fi or
Shut Down in a test changes nothing real. `Y7_SHELL_DATA_DIR` moves the Focus
state and pins. Run it in the headless desktop with a private session bus:

```sh
dbus-daemon --session --print-address --fork   # in a temp dir; export the address
Y7_SHELL_FAKE=1 y7-desktop --headless 1440x900 --scale 2
# then on stdin: spawn y7-shell, key super+space, type 12*12, shot out.png
```

Build the desktop with `--release` for screenshots: a debug build spends seconds
encoding each PNG, during which it can't release clients' buffers.
