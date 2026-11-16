#!/bin/sh
# Builds and installs y7ya-os for this user, then (with sudo) adds it to the
# login screen. Run from the repo root: desktop/session/install.sh
# `--user-only` skips the sudo step (the login-screen entry).
set -e
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release --workspace
for b in y7-desktop y7-shell y7-syncd y7-portal notes calculator textedit assistant files preview taskmanager clipboard calendar reminders; do
    [ -x "target/release/$b" ] && install -Dm755 "target/release/$b" "$HOME/.local/bin/$b"
done
install -Dm755 desktop/session/y7ya-os-session "$HOME/.local/bin/y7ya-os-session"
install -Dm644 desktop/session/y7ya-os-portals.conf "$HOME/.config/xdg-desktop-portal/y7ya-os-portals.conf"
# Our light/dark portal backend: D-Bus activated, and found by the portal
# through a drop-in that adds its directory to the data dirs.
install -Dm644 crates/y7-portal/y7ya.portal "$HOME/.local/share/y7ya-os/portal/xdg-desktop-portal/portals/y7ya.portal"
sed "s|@BIN@|$HOME/.local/bin/y7-portal|" crates/y7-portal/org.freedesktop.impl.portal.desktop.y7ya.service \
    | install -Dm644 /dev/stdin "$HOME/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.y7ya.service"
dirs="$HOME/.local/share/y7ya-os/portal:$HOME/.local/share/flatpak/exports/share:/var/lib/flatpak/exports/share:/usr/local/share:/usr/share"
sed "s|@DIRS@|$dirs|" desktop/session/xdg-desktop-portal-y7ya.conf \
    | install -Dm644 /dev/stdin "$HOME/.config/systemd/user/xdg-desktop-portal.service.d/y7ya-os.conf"
systemctl --user daemon-reload 2>/dev/null || true
# The Mac look for third-party apps (per user, only used inside y7ya-os).
desktop/session/theme.sh || echo "Couldn't install the look for third-party apps; re-run desktop/session/theme.sh"
for f in apps/*/*.desktop; do
    install -Dm644 "$f" "$HOME/.local/share/applications/$(basename "$f")"
done
# App icons (y7ya-*): render assets/icons into a hicolor tree and install it.
rm -rf target/icons
target/release/render-icons target/icons
mkdir -p "$HOME/.local/share/icons"
cp -r target/icons/hicolor "$HOME/.local/share/icons/"
gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
if [ "$1" = "--user-only" ]; then
    echo "Installed for $USER. Add the login-screen entry with: desktop/session/install.sh"
    exit 0
fi
# The login screen only reads system-wide session files, and needs the full path.
sed "s|^Exec=.*|Exec=$HOME/.local/bin/y7ya-os-session|; s|^TryExec=.*|TryExec=$HOME/.local/bin/y7ya-os-session|" \
    desktop/session/y7ya-os.desktop > /tmp/y7ya-os.desktop
sudo install -Dm644 /tmp/y7ya-os.desktop /usr/share/wayland-sessions/y7ya-os.desktop
echo "Installed. Log out and pick y7ya-os on the login screen."
