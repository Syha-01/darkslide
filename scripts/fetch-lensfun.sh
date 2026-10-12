#!/bin/sh
# Download the lensfun lens database (CC BY-SA 3.0) for profile corrections.
set -e
dest="${XDG_DATA_HOME:-$HOME/.local/share}/darkslide/lensfun"
mkdir -p "$dest"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -sL -o "$tmp/lensfun.tar.gz" https://github.com/lensfun/lensfun/archive/refs/heads/master.tar.gz
tar -xzf "$tmp/lensfun.tar.gz" -C "$tmp"
cp "$tmp"/lensfun-master/data/db/*.xml "$dest"/
echo "installed $(ls "$dest" | wc -l) files into $dest"
