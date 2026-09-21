#!/usr/bin/env bash
#
# Installs DBDelve for the current user, out of the unpacked release tarball.
#
# Everything lands under ~/.local, so this never needs root and never fights
# the distribution's package manager over a path it owns.
#
# Usage: ./install.sh [--uninstall]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$HOME/.local/bin"
ICONS="$HOME/.local/share/icons/hicolor"
APPS="$HOME/.local/share/applications"
SIZES=(16 32 64 128 256 512)

refresh() {
  # Both are optional: a desktop without them still reads the directories on
  # its own schedule, and a headless box has neither installed.
  if command -v update-desktop-database >/dev/null; then
    update-desktop-database "$APPS" || true
  fi
  if command -v gtk-update-icon-cache >/dev/null; then
    gtk-update-icon-cache -qtf "$ICONS" || true
  fi
}

if [[ "${1:-}" == --uninstall ]]; then
  rm -f "$BIN/dbdelve" "$APPS/dbdelve.desktop"
  for size in "${SIZES[@]}"; do
    rm -f "$ICONS/${size}x${size}/apps/dbdelve.png"
  done
  refresh
  # Connections, query history and saved passwords are deliberately left: an
  # uninstall here is usually a reinstall, and they live outside all of this.
  echo "removed dbdelve -- data in ~/.local/share/dbdelve was left alone"
  exit 0
fi

mkdir -p "$BIN" "$APPS"
install -m 755 "$HERE/dbdelve" "$BIN/dbdelve"
install -m 644 "$HERE/dbdelve.desktop" "$APPS/dbdelve.desktop"
for size in "${SIZES[@]}"; do
  mkdir -p "$ICONS/${size}x${size}/apps"
  install -m 644 "$HERE/icons/dbdelve-$size.png" "$ICONS/${size}x${size}/apps/dbdelve.png"
done
refresh

echo "installed $BIN/dbdelve"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "warning: $BIN is not on your PATH -- add it, or the desktop entry will fail to launch" >&2 ;;
esac
