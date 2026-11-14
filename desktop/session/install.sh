#!/bin/sh
# Builds and installs y7ya-os for this user, then (with sudo) adds it to the
# login screen. Run from the repo root: desktop/session/install.sh
set -e
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release --workspace
bins="y7-desktop notes calculator textedit assistant"
for b in $bins y7-shell y7-syncd files preview taskmanager clipboard calendar reminders; do
    [ -x "target/release/$b" ] && install -Dm755 "target/release/$b" "$HOME/.local/bin/$b"
done
install -Dm755 desktop/session/y7ya-os-session "$HOME/.local/bin/y7ya-os-session"
for f in apps/*/*.desktop; do
    install -Dm644 "$f" "$HOME/.local/share/applications/$(basename "$f")"
done
# The login screen only reads system-wide session files, and needs the full path.
sed "s|^Exec=.*|Exec=$HOME/.local/bin/y7ya-os-session|; s|^TryExec=.*|TryExec=$HOME/.local/bin/y7ya-os-session|" \
    desktop/session/y7ya-os.desktop > /tmp/y7ya-os.desktop
sudo install -Dm644 /tmp/y7ya-os.desktop /usr/share/wayland-sessions/y7ya-os.desktop
echo "Installed. Log out and pick y7ya-os on the login screen."
