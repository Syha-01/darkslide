#!/bin/sh
# Build and install Darkslide for the current user (~/.local).
set -e
cd "$(dirname "$0")/.."
PATH="$HOME/.cargo/bin:$PATH" cargo build --release ${FEATURES:+--features "$FEATURES"}
prefix="${PREFIX:-$HOME/.local}"
install -Dm755 target/release/darkslide "$prefix/bin/darkslide"
install -Dm644 packaging/darkslide.desktop "$prefix/share/applications/darkslide.desktop"
install -Dm644 packaging/darkslide.svg "$prefix/share/icons/hicolor/scalable/apps/darkslide.svg"
command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true
echo "installed to $prefix/bin/darkslide"
