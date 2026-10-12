#!/bin/sh
# CC0 sample raws from raw.pixls.us used for smoke tests.
set -e
dir="$(dirname "$0")/../testdata"
mkdir -p "$dir"
get() { [ -s "$dir/$2" ] || curl -sL -o "$dir/$2" "https://raw.pixls.us/data/$1"; }
get "Canon/Canon%20EOS%20R10/IMG_6310.CR3" canon_r10.cr3
get "Nikon/D750/compressed_12_bit.NEF" nikon_d750.nef
get "Sony/ILCE-7M3/UNCOMPRESSED_14bit.ARW" sony_a7m3.arw
get "Fujifilm/X-T3/AFXT2720.RAF" fuji_xt3.raf
get "Apple/iPhone%2012%20Pro/IMG_1361.DNG" iphone12.dng
get "Panasonic/DC-G9/P1000475.RW2" pana_g9.rw2
ls -la "$dir"
