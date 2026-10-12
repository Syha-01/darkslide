#!/bin/sh
# Install a prebuilt Darkslide for the current user (~/.local, or $PREFIX).
set -e
cd "$(dirname "$0")"
prefix="${PREFIX:-$HOME/.local}"
install -Dm755 darkslide "$prefix/bin/darkslide"
install -Dm644 darkslide.desktop "$prefix/share/applications/darkslide.desktop"
install -Dm644 darkslide.svg "$prefix/share/icons/hicolor/scalable/apps/darkslide.svg"
command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true
echo "installed to $prefix/bin/darkslide"
case ":$PATH:" in *":$prefix/bin:"*) ;; *) echo "note: add $prefix/bin to your PATH" ;; esac
