#!/bin/sh
# Installs the Mac look that the y7ya-os desktop gives third-party apps (Qt/KDE,
# GTK, libadwaita, cursors, Konsole). Run from anywhere; install.sh calls it.
# Safe to re-run: downloads are pinned and cached, everything else is rebuilt.
#
# Only the apps y7ya-os starts use any of it (see `settings::look`), so other
# desktops such as Plasma are unaffected. Nothing in ~/.config is written, and
# global settings (gsettings, kdeglobals, gtk-*/settings.ini, the default
# cursor) are left alone:
#   ~/.local/share/y7ya-os/look        themes, icons, cursors and the
#                                       session-only config, see below
#   ~/.local/share/themes/WhiteSur-opaque/Kvantum
#                                       the Kvantum theme (inert until chosen)
#   ~/.local/share/fonts/y7ya-os       Inter and JetBrains Mono
#   ~/.cache/y7ya-os/themes            pinned upstream checkouts
#
# The look directory:
#   share/                 WhiteSur GTK themes + icons, and our GTK themes
#                          y7ya-os-light/-dark (WhiteSur with Inter)
#   cursors/               WhiteSur cursors; other theme names point at them
#   light/, dark/          one per appearance:
#     config/              prepended to XDG_CONFIG_DIRS. kdeglobals and
#                          konsolerc lock ([$i], KDE's kiosk) fonts, colors,
#                          icons, widget style and Konsole's profile over the
#                          user's own files; Kvantum/kvantum.kvconfig picks
#                          the theme (the user's ~/.config one, if any, wins)
#     share/               prepended to XDG_DATA_DIRS: the y7ya-os icon theme,
#                          stand-ins for the user's GTK icon theme name, the
#                          color scheme and the Konsole profile
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
LOOK=${Y7YA_LOOK_DIR:-$DATA/y7ya-os/look}
CACHE=${XDG_CACHE_HOME:-$HOME/.cache}/y7ya-os/themes
FONTS=$DATA/fonts/y7ya-os
# Kvantum only looks here (or in ~/.config/Kvantum, which we leave alone).
KVANTUM=$HOME/.local/share/themes/WhiteSur-opaque/Kvantum

# Pinned upstream versions (github.com/vinceliuice).
GTK_REV=99247ecd219bdc1c7409baff062b4aafcf1102a7      # WhiteSur-gtk-theme, tag 2026-09-10
ICONS_REV=73d8040da51a9ed74e47c7366e7e9ff437601a5c    # WhiteSur-icon-theme, tag 2026-09-10
CURSORS_REV=e190baf618ed95ee217d2fd45589bd309b37672b  # WhiteSur-cursors, master
KDE_REV=cf4df59ce91004f7ea39358b1b8ff917d5c329f7      # WhiteSur-kde (Kvantum), master
JBMONO_URL=https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip
JBMONO_SHA256=6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf

UI_FONT="Inter,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"
SMALL_FONT="Inter,8,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"
MONO_FONT="JetBrains Mono,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"

# fetch <repo> <commit>: prints the path of a cached checkout.
fetch() {
    dir=$CACHE/$1-$2
    if [ ! -d "$dir" ]; then
        echo "Downloading $1..." >&2
        rm -rf "$dir.part"
        git init -q "$dir.part"
        git -C "$dir.part" fetch -q --depth 1 "https://github.com/vinceliuice/$1" "$2"
        git -C "$dir.part" -c advice.detachedHead=false checkout -q FETCH_HEAD
        mv "$dir.part" "$dir"
    fi
    echo "$dir"
}

mkdir -p "$CACHE" "$LOOK"
# The desktop only applies a finished look.
rm -f "$LOOK/ready"

# --- GTK: WhiteSur's prebuilt "solid" themes (no translucency: we don't blur).
gtk=$(fetch WhiteSur-gtk-theme $GTK_REV)
rm -rf "$LOOK/share/themes"
mkdir -p "$LOOK/share/themes"
for v in Light Dark; do
    tar -xJf "$gtk/release/WhiteSur-$v-solid.tar.xz" -C "$LOOK/share/themes"
done
# Our own themes reuse WhiteSur's compiled stylesheet (GTK loads the theme
# directory's gtk.gresource) and set Inter as the UI font.
for mode in light dark; do
    case $mode in light) src=WhiteSur-Light-solid ;; dark) src=WhiteSur-Dark-solid ;; esac
    theme=$LOOK/share/themes/y7ya-os-$mode
    mkdir -p "$theme"
    printf '[Desktop Entry]\nType=X-GNOME-Metatheme\nName=y7ya-os-%s\nComment=%s with Inter, for y7ya-os\n' \
        "$mode" "$src" >"$theme/index.theme"
    for g in gtk-3.0 gtk-4.0; do
        mkdir -p "$theme/$g"
        ln -s "../../$src/$g/gtk.gresource" "$theme/$g/gtk.gresource"
        cat >"$theme/$g/gtk.css" <<EOF
/* $src with Inter as the UI font. Made by y7ya-os desktop/session/theme.sh. */
@import url("resource:///org/gnome/theme/gtk.css");

window, tooltip, popover { font-family: "Inter", sans-serif; }
EOF
    done
done

# --- Icons: WhiteSur, WhiteSur-light and WhiteSur-dark. The script only
# writes under -d (and runs gtk-update-icon-cache there).
icons=$(fetch WhiteSur-icon-theme $ICONS_REV)
rm -rf "$LOOK/share/icons"
bash "$icons/install.sh" -d "$LOOK/share/icons" >/dev/null

# --- Cursors. XCURSOR_PATH points only here, so every theme name an app
# might ask for (its toolkit's default, the user's Plasma/GTK choice) is
# made to inherit WhiteSur-cursors.
cursors=$(fetch WhiteSur-cursors $CURSORS_REV)
rm -rf "$LOOK/cursors"
mkdir -p "$LOOK/cursors"
cp -r "$cursors/dist" "$LOOK/cursors/WhiteSur-cursors"
user_cursor_themes() {
    sed -n 's/^cursorTheme=//p' "$HOME/.config/kcminputrc" 2>/dev/null || true
    sed -n 's/^gtk-cursor-theme-name=//p' "$HOME/.config/gtk-3.0/settings.ini" 2>/dev/null || true
    gsettings get org.gnome.desktop.interface cursor-theme 2>/dev/null | tr -d "'" || true
}
for name in default breeze_cursors Breeze_Light Breeze_Dark Adwaita $(user_cursor_themes); do
    case $name in WhiteSur-cursors | */* | .*) continue ;; esac
    mkdir -p "$LOOK/cursors/$name"
    printf '[Icon Theme]\nName=%s (y7ya-os)\nInherits=WhiteSur-cursors\n' "$name" >"$LOOK/cursors/$name/index.theme"
done

# --- Kvantum: WhiteSur-opaque, whose dark variant is WhiteSur-opaqueDark
# (the "kvantum-dark" style picks it).
kde=$(fetch WhiteSur-kde $KDE_REV)
rm -rf "$KVANTUM"
mkdir -p "$KVANTUM"
cp "$kde"/Kvantum/WhiteSur-opaque/* "$KVANTUM"/

# --- Fonts: Inter (bundled with our UI) and JetBrains Mono.
mkdir -p "$FONTS"
cp "$REPO"/crates/ui/assets/fonts/Inter-*.ttf "$FONTS"/
cp "$REPO/crates/ui/assets/fonts/OFL.txt" "$FONTS/Inter-OFL.txt"
zip=$CACHE/$(basename "$JBMONO_URL")
if ! echo "$JBMONO_SHA256  $zip" | sha256sum -c --status 2>/dev/null; then
    echo "Downloading JetBrains Mono..." >&2
    curl -fsSL -o "$zip.part" "$JBMONO_URL"
    echo "$JBMONO_SHA256  $zip.part" | sha256sum -c --status
    mv "$zip.part" "$zip"
fi
tmp=$(mktemp -d)
bsdtar -xf "$zip" -C "$tmp" fonts/ttf/JetBrainsMono-Regular.ttf fonts/ttf/JetBrainsMono-Bold.ttf \
    fonts/ttf/JetBrainsMono-Italic.ttf fonts/ttf/JetBrainsMono-BoldItalic.ttf OFL.txt
cp "$tmp"/fonts/ttf/*.ttf "$FONTS"/
cp "$tmp/OFL.txt" "$FONTS/JetBrainsMono-OFL.txt"
rm -rf "$tmp"
fc-cache -f "$FONTS" >/dev/null 2>&1 || true

# --- Per-appearance config.

# colors <group> <background> <alternate background> <text> <secondary text>
colors() {
    cat <<EOF
[$1][\$i]
BackgroundAlternate=$3
BackgroundNormal=$2
DecorationFocus=$ACCENT
DecorationHover=$ACCENT
ForegroundActive=$ACCENT
ForegroundInactive=$5
ForegroundLink=$ACCENT
ForegroundNegative=$RED
ForegroundNeutral=$ORANGE
ForegroundNormal=$4
ForegroundPositive=$GREEN
ForegroundVisited=$PURPLE

EOF
}

# The color scheme from docs/design.md, in KDE's format.
color_scheme() {
    colors Colors:Window "$WINDOW" "$WINDOW_ALT" "$TEXT" "$TEXT2"
    colors Colors:View "$VIEW" "$VIEW_ALT" "$TEXT" "$TEXT2"
    colors Colors:Button "$BUTTON" "$WINDOW" "$TEXT" "$TEXT2"
    colors Colors:Header "$WINDOW" "$WINDOW_ALT" "$TEXT" "$TEXT2"
    colors "Colors:Header][Inactive" "$WINDOW" "$WINDOW_ALT" "$TEXT2" "$TEXT2"
    colors Colors:Selection "$ACCENT" "$ACCENT" "255,255,255" "230,240,255"
    colors Colors:Tooltip "$TOOLTIP" "$TOOLTIP" "$TEXT" "$TEXT2"
    colors Colors:Complementary "$INVERSE" "$INVERSE" "$INVERSE_TEXT" "$TEXT2"
    cat <<EOF
[ColorEffects:Disabled][\$i]
Color=$WINDOW
ColorAmount=0
ColorEffect=0
ContrastAmount=0.55
ContrastEffect=1
IntensityAmount=0.1
IntensityEffect=2

[ColorEffects:Inactive][\$i]
ChangeSelectionColor=false
Enable=false

[WM][\$i]
activeBackground=$WINDOW
activeBlend=$TEXT
activeForeground=$TEXT
inactiveBackground=$WINDOW
inactiveBlend=$TEXT2
inactiveForeground=$TEXT2

EOF
}

# konsole_colors <name> <description>: Terminal.app's "Basic" profile colors.
konsole_colors() {
    printf '[General]\nDescription=%s\nOpacity=1\nBlur=false\n\n' "$2"
    printf '[Background]\nColor=%s\n\n[BackgroundIntense]\nColor=%s\n\n' "$TERM_BG" "$TERM_BG"
    printf '[Foreground]\nColor=%s\n\n[ForegroundIntense]\nColor=%s\n\n' "$TERM_FG" "$TERM_FG"
    i=0
    for pair in 0,0,0/102,102,102 153,0,0/229,0,0 0,166,0/0,217,0 153,153,0/229,229,0 \
        0,0,178/0,0,255 178,0,178/229,0,229 0,166,178/0,229,229 191,191,191/229,229,229; do
        printf '[Color%s]\nColor=%s\n\n[Color%sIntense]\nColor=%s\n\n' $i "${pair%/*}" $i "${pair#*/}"
        i=$((i + 1))
    done
}

user_icon_themes() {
    sed -n 's/^gtk-icon-theme-name=//p' "$HOME/.config/gtk-3.0/settings.ini" "$HOME/.config/gtk-4.0/settings.ini" 2>/dev/null || true
    gsettings get org.gnome.desktop.interface icon-theme 2>/dev/null | tr -d "'" || true
}

# icon_theme <dir> <name> <inherits>: an icon theme that only inherits. KDE
# ignores themes without a directory, hence the empty one.
icon_theme() {
    mkdir -p "$1/$2/placeholder"
    printf '[Icon Theme]\nName=%s\nComment=Made by y7ya-os desktop/session/theme.sh\nInherits=%s\nDirectories=placeholder\n\n[placeholder]\nSize=16\nType=Fixed\n' \
        "$2" "$3" >"$1/$2/index.theme"
}

for mode in light dark; do
    if [ $mode = light ]; then
        ACCENT=0,122,255 RED=255,59,48 ORANGE=255,159,10 GREEN=52,199,89 PURPLE=165,80,167
        WINDOW=242,242,244 WINDOW_ALT=235,235,237 VIEW=255,255,255 VIEW_ALT=244,245,245
        BUTTON=255,255,255 TOOLTIP=246,246,246 TEXT=38,38,38 TEXT2=128,128,128
        INVERSE=42,42,44 INVERSE_TEXT=222,222,222
        TERM_BG=255,255,255 TERM_FG=0,0,0
        icons=WhiteSur breeze=breeze kvantum=kvantum
    else
        ACCENT=10,132,255 RED=255,69,58 ORANGE=255,159,10 GREEN=48,209,88 PURPLE=191,90,242
        WINDOW=42,42,44 WINDOW_ALT=50,50,52 VIEW=30,30,30 VIEW_ALT=38,38,38
        BUTTON=64,64,66 TOOLTIP=44,44,46 TEXT=222,222,222 TEXT2=140,140,140
        INVERSE=242,242,244 INVERSE_TEXT=38,38,38
        TERM_BG=30,30,30 TERM_FG=229,229,229
        icons=WhiteSur-dark breeze=breeze-dark kvantum=kvantum-dark
    fi
    base=$LOOK/$mode
    rm -rf "$base"
    mkdir -p "$base/config/Kvantum" "$base/share/icons" "$base/share/color-schemes" "$base/share/konsole"

    # Icons: "y7ya-os" falls back to Breeze for the KDE-only icons WhiteSur
    # lacks; GTK asks for the user's own theme name, so that name becomes
    # y7ya-os too (the real Breeze stays reachable through a symlink).
    if [ -d /usr/share/icons/$breeze ]; then
        ln -s /usr/share/icons/$breeze "$base/share/icons/y7ya-os-$breeze"
        icon_theme "$base/share/icons" y7ya-os "$icons,y7ya-os-$breeze,hicolor"
    else
        icon_theme "$base/share/icons" y7ya-os "$icons,hicolor"
    fi
    for name in breeze breeze-dark $(user_icon_themes); do
        case $name in y7ya-os* | WhiteSur* | Adwaita | hicolor | */* | .*) continue ;; esac
        [ -e "$base/share/icons/$name" ] || icon_theme "$base/share/icons" "$name" y7ya-os
    done

    {
        printf '[General]\nName=y7ya-os %s\n\n' $mode
        color_scheme
    } >"$base/share/color-schemes/y7ya-os.colors"

    {
        cat <<EOF
# The y7ya-os look for KDE/Qt apps, locked ([\$i]) over ~/.config/kdeglobals.
# Made by desktop/session/theme.sh; only y7ya-os puts it in XDG_CONFIG_DIRS.
[General]
AccentColor[\$i]=$ACCENT
ColorScheme[\$i]=y7ya-os
fixed[\$i]=$MONO_FONT
font[\$i]=$UI_FONT
menuFont[\$i]=$UI_FONT
smallestReadableFont[\$i]=$SMALL_FONT
toolBarFont[\$i]=$UI_FONT

[Icons]
Theme[\$i]=y7ya-os

[KDE]
widgetStyle[\$i]=$kvantum

EOF
        color_scheme
    } >"$base/config/kdeglobals"

    printf '[General]\ntheme=WhiteSur-opaque\n' >"$base/config/Kvantum/kvantum.kvconfig"

    # Konsole: a Terminal.app-like profile, the default only in y7ya-os.
    printf '[Desktop Entry]\nDefaultProfile[$i]=y7ya-os.profile\n' >"$base/config/konsolerc"
    cat >"$base/share/konsole/y7ya-os.profile" <<EOF
[Appearance]
ColorScheme=y7ya-os
Font=$MONO_FONT
LineSpacing=1

[General]
Name=y7ya-os
Parent=FALLBACK/
TerminalColumns=80
TerminalMargin=6
TerminalRows=24

[Scrolling]
ScrollBarPosition=2

[Terminal Features]
BlinkingCursorEnabled=true
EOF
    konsole_colors y7ya-os "y7ya-os $mode" >"$base/share/konsole/y7ya-os.colorscheme"
done

echo "$GTK_REV $ICONS_REV $CURSORS_REV $KDE_REV" >"$LOOK/ready"
echo "Installed the Mac look for apps in $LOOK"
