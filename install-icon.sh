#!/usr/bin/env bash
set -e

BINARY="$(cargo build --release --message-format=json 2>/dev/null \
    | grep '"executable"' | tail -1 | sed 's/.*"executable":"\(.*\)".*/\1/')"

# Fallback: look in target/release
if [ -z "$BINARY" ] || [ ! -f "$BINARY" ]; then
    BINARY="$(pwd)/target/release/gpui-deb-installer"
fi

ICON_DIR="$HOME/.local/share/icons/hicolor/scalable/apps"
DESKTOP_DIR="$HOME/.local/share/applications"

mkdir -p "$ICON_DIR" "$DESKTOP_DIR"

# Install icon
cp "$(dirname "$0")/icon.svg" "$ICON_DIR/gpui-deb-installer.svg"
echo "Installed icon → $ICON_DIR/gpui-deb-installer.svg"

# Install .desktop file with correct binary path
sed "s|%EXEC%|$BINARY|g" "$(dirname "$0")/gpui-deb-installer.desktop" \
    > "$DESKTOP_DIR/gpui-deb-installer.desktop"
echo "Installed .desktop → $DESKTOP_DIR/gpui-deb-installer.desktop"

# Refresh icon cache
if command -v gtk-update-icon-cache &>/dev/null; then
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
fi

if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
fi

echo "Done. Re-login or run: xdg-icon-resource forceupdate"
