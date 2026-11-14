# Design language

Faithful to macOS 26 (Tahoe) where it matters; no Dock, no Liquid Glass (battery).
Light and dark follow the system (`cx.dark`). Inter stands in for SF Pro.

## Windows

- Apps draw their own unified toolbar and use `WindowOptions { decorations: false }`.
  Put `ui::chrome::window_buttons(true, cx, ink)` at the left of the toolbar or
  sidebar header; it shows traffic lights under y7ya-os and KDE buttons elsewhere.
  Mark toolbars `.window_drag()`.
- The y7ya-os desktop rounds corners (14pt) and draws the shadow. Don't draw your own.
- Toolbar height 52pt; traffic lights start 20pt from the left edge, vertically
  centered. Sidebar 220pt wide, its rows 28pt with 6pt-rounded selection.

## Type (Inter)

| Use | Size | Weight |
|---|---|---|
| Body, list rows, controls | 13 | Regular |
| Secondary / captions | 11 | Regular |
| Section headers in sidebars | 11 | Semibold, secondary color |
| Window/toolbar title | 13–15 | Bold |
| Large titles (content) | 22–26 | Bold |

## Color

| Token | Light | Dark |
|---|---|---|
| Window background | `#ffffff` | `#1e1e1e` |
| Sidebar / secondary bg | `#f2f2f4` | `#2a2a2c` |
| Label | `rgba(0,0,0,.85)` | `rgba(255,255,255,.85)` |
| Secondary label | `rgba(0,0,0,.50)` | `rgba(255,255,255,.55)` |
| Tertiary label | `rgba(0,0,0,.26)` | `rgba(255,255,255,.25)` |
| Separator | `rgba(0,0,0,.10)` | `rgba(255,255,255,.10)` |
| Control fill (fields, segmented) | `rgba(0,0,0,.05)` | `rgba(255,255,255,.08)` |
| Accent (system blue) | `#007aff` | `#0a84ff` |
| Selection (focused list) | accent | accent |
| Destructive | `#ff3b30` | `#ff453a` |
| Success / warning | `#34c759` / `#ff9f0a` | `#30d158` / `#ff9f0a` |

Apps may have their own accent where macOS does (Notes is yellow `#e2a800`).

## Controls

- Buttons: 22–28pt tall, 6pt radius; push buttons filled with accent for the
  default action, `Control fill` otherwise. Toolbar icon buttons 32×28, hover fill.
- Text fields and search fields: `Control fill`, 7pt radius, magnifier icon.
- Menus/popovers: `#f6f6f6`/`#2c2c2e`, 8pt radius, hairline border, soft shadow,
  rows 24pt with accent highlight and white text on hover.
- Icons: thin rounded line icons in the SF Symbols spirit (see `apps/notes/src/notes/icons.rs`).
- Motion: short (150–250ms), ease-out; nothing animates while idle.
