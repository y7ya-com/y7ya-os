# y7-shell

The y7ya-os system UI in one long-running process: the menu bar, Spotlight,
Control Center, notification banners and Notification Center. It draws on
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
| `spotlight` | Toggle Spotlight (the desktop binds Super+Space to this) |
| `control-center` | Toggle Control Center |
| `notification-center` | Toggle Notification Center |
| `lock` | Lock the screen (`loginctl lock-session`) |
| `dismiss` | Close whatever menu, panel or dialog is open |
| `dnd on`, `dnd off`, `dnd toggle` | Do Not Disturb |
| `ping` | Succeeds if the shell is running |

Anything can speak the protocol: connect, send `spotlight\n`, read `ok\n`.

## What talks to what

- **Active app**: `zwlr_foreign_toplevel_manager_v1` on its own Wayland
  connection. Without it (KWin) the menu bar just shows no app.
- **Notifications**: serves `org.freedesktop.Notifications` on the session bus
  (actions, body, persistence). If another daemon owns the name it logs that and
  runs without banners.
- **Status**: battery from sysfs (UPower signals wake it), Wi-Fi via `nmcli`
  (NetworkManager signals, plus a 60 s check for signal strength), Bluetooth via
  `bluetoothctl`, sound via `wpctl` (woken by `pactl subscribe`), brightness via
  sysfs and `brightnessctl` or logind, Now Playing over MPRIS. All behind the
  `System` trait in `src/system.rs`.
- **Spotlight**: apps from `.desktop` files in the XDG data dirs, files from an
  in-memory index of home (hidden, generated and huge folders skipped; refreshed
  incrementally by directory mtime each time Spotlight opens), a calculator and
  unit conversions.

The UI thread sleeps until something happens; the clock wakes it once a minute.

## Testing

`Y7_SHELL_FAKE=1` swaps the machine for an in-memory one, so clicking Wi-Fi or
Shut Down in a test changes nothing real. `Y7_SHELL_DATA_DIR` moves the Focus
state. Run it in the headless desktop with a private session bus:

```sh
dbus-daemon --session --print-address --fork   # in a temp dir; export the address
Y7_SHELL_FAKE=1 y7-desktop --headless 1440x900 --scale 2
# then on stdin: spawn y7-shell, key super+space, type 12*12, shot out.png
```

Build the desktop with `--release` for screenshots: a debug build spends seconds
encoding each PNG, during which it can't release clients' buffers.
