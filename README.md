# Darkslide

A non-destructive raw photo developer for Linux, written in Rust, modelled on
Adobe Camera Raw: the same panels, tools and slider semantics, XMP sidecars
compatible with `crs:` settings, presets, masking, healing, HDR and panorama
merging.

See `PLAN.md` for the feature inventory, architecture and progress.

## Build

```sh
# rustup's toolchain must come before any distro rustc on PATH
export PATH=$HOME/.cargo/bin:$PATH
cargo build --release
./target/release/darkslide [FILE|FOLDER]...
```

Install for the current user (binary, desktop entry, icon):
`scripts/install.sh` (set `FEATURES=ml` for ONNX support). A Flatpak manifest
template lives in `packaging/`.

Runtime needs a Vulkan or OpenGL capable GPU driver (wgpu) and, for native
file dialogs, the XDG desktop portal (most desktops ship it).

## Command line

```sh
darkslide --cli -s 2048 -o out/ photos/*.NEF          # batch export
darkslide --cli --xmp preset.xmp --set Exposure2012=+0.5 IMG.CR3
darkslide --cli --merge-hdr --deghost medium a.ARW b.ARW c.ARW
darkslide --cli --merge-pano --projection cylindrical p1.NEF p2.NEF
darkslide --cli --help
```

## Data locations (XDG)

| What | Where |
|------|-------|
| Settings | `IMAGE.xmp` sidecar next to each image |
| Presets (`.xmp`) | `~/.config/darkslide/presets/` |
| Profiles: `.cube` looks, `.dcp` camera profiles | `~/.config/darkslide/profiles/` |
| Camera defaults | `~/.config/darkslide/defaults/` |
| Lens database (lensfun XML) | `~/.local/share/darkslide/lensfun/` (`scripts/fetch-lensfun.sh`) |
| Adobe lens profiles (`.lcp`) | `~/.config/darkslide/lcp/` |
| Snapshots | `~/.local/share/darkslide/snapshots/` |

Camera Raw presets (`.xmp`) and DNG camera profiles (`.dcp`, e.g. from
Adobe's free DNG Converter) can be dropped into those folders.

## Optional: learned AI masks and depth (ONNX)

Without models, Subject/Sky/Background/Objects masks and Lens Blur depth use
classical detectors. For much better masks build with `--features ml`,
install ONNX Runtime ≥ 1.20 (`libonnxruntime.so`; point `ORT_DYLIB_PATH` at
it if it isn't on the library path) and put models into
`~/.local/share/darkslide/models/`:

| File | Kind | Example |
|------|------|---------|
| `subject.onnx` | salient object (320 px U²-Net, or 1024 px ISNet/RMBG) | `u2netp.onnx` from rembg |
| `sky.onnx` | sky segmentation (U²-Net style output) | |
| `depth.onnx` | relative inverse depth, 518 px | Depth Anything V2 small |

```sh
cargo build --release --features ml
ORT_DYLIB_PATH=/opt/onnxruntime/lib/libonnxruntime.so ./target/release/darkslide
```

## Development

```sh
scripts/fetch-testdata.sh                 # CC0 sample raws into testdata/
cargo test
cargo build --release -p darkslide --features screenshot
./target/release/darkslide --screenshot ui.png --tool crop testdata/nikon_d750.nef
```
