# Darkslide — a Linux raw developer modelled on Adobe Camera Raw

This file is the source of truth for the project. It holds the research (what
Camera Raw does), the architecture, and a phased checklist. **When resuming work,
read "Status / resume notes" at the bottom first**, then pick the first unchecked
item in the current phase.

Target: Linux (X11 + Wayland), x86_64 first. Rust stable (pinned via
`rust-toolchain.toml`). GUI: `eframe`/`egui` (wgpu backend). No macOS/Windows
specific code paths; paths follow XDG (`~/.config/darkslide`,
`~/.local/share/darkslide`, `~/.cache/darkslide`).

---

## 1. Research: what Adobe Camera Raw (ACR) does

ACR is Adobe's raw developer (ships as a Photoshop plug-in; same engine as
Lightroom). Current version at time of writing: **18.4.1 (July 2026)**. Everything
is **non-destructive**: edits are parameters stored in an XMP sidecar
(`file.xmp`, `crs:` namespace, `http://ns.adobe.com/camera-raw-settings/1.0/`)
or embedded in DNG/JPEG/TIFF. Pixels are only rendered on preview / save.

### 1.1 Input formats
- Proprietary raw of ~1000+ cameras (CR2/CR3/CRW, NEF/NRW, ARW/SR2/SRF, RAF
  incl. X-Trans, ORF, RW2, PEF, SRW, 3FR/FFF, IIQ, MOS, ERF, MRW, X3F (limited), …).
- DNG (incl. linear DNG, lossy DNG, JPEG-XL DNG, Apple ProRAW, Samsung Expert RAW,
  HDR/panorama merged float DNG, Enhanced/denoised DNG).
- Raster: JPEG, TIFF (8/16/32-bit), HEIC/HEIF, AVIF, JPEG XL, PNG, PSD (as filter).

### 1.2 Workspace / UI
- Main preview with zoom (fit/fill/100%/zoom levels, scroll + space-drag pan),
  loupe, full-screen and full-screen presentation mode (18.4.1).
- **Before/After** views: left/right, top/bottom, split left/right, split
  top/bottom; swap; copy after→before; toggle (`\`/`Y`/`P` keys).
- **Filmstrip** (multi-image): select, rate (stars), color label, flag, sort,
  filter; **Sync settings** across selection; Select-all; auto-sync.
- **Histogram** (RGB, with clipping indicators for shadows/highlights — `U`/`O`),
  RGB readout under cursor, camera EXIF line (ISO/lens/f/shutter).
  **Vectorscope** + **Skin tone indicator** line (18.4).
- **Autofocus point overlay** (Canon/Nikon/Sony, 18.4.1).
- Right toolbar: **Edit**, **Crop & Rotate**, **Remove** (spot heal/clone/AI
  remove), **Masking**, **Red Eye**, **Presets**, **Snapshots**, **More** (…),
  **Zoom**, **Hand**, **Toggle sampler overlay / grid**.
- Color samplers (up to 9) with live readouts; grid overlays.
- Panel-level enable/disable eye toggles; per-panel reset (double-click label
  resets slider); Alt-drag to show clipping mask while dragging
  exposure/whites/blacks/sharpening/masking.
- Undo/redo history (Ctrl+Z / Ctrl+Shift+Z) ; reset to defaults/open settings.
- Workflow options: output color space (sRGB, Adobe RGB, Display P3, ProPhoto,
  Rec.2020, custom ICC, grayscale gamma), bit depth (8/16/32), size/resolution,
  output sharpening (screen/matte/glossy; low/std/high), open as smart object.
- Preferences: XMP sidecar vs. database, default settings per camera / ISO /
  serial, DNG handling, cache size, GPU usage, technology previews.

### 1.3 Edit panel (global adjustments) — parameter inventory

`crs:` XMP names given in brackets. Ranges: ACR UI ranges.

**Profile** — Adobe Color/Standard/Landscape/Portrait/Vivid/Neutral/Monochrome,
Camera Matching profiles (vendor looks), **Adobe Adaptive Color / Adaptive B&W**
(AI, amount 0–200), Artistic/B&W/Modern/Vintage creative profiles (LUT-based
"Looks" with Amount 0–200) [`CameraProfile`, `Look` struct with Name/Amount/
Parameters]. Profiles are **DCP** files (DNG Camera Profile: ColorMatrix,
ForwardMatrix, HueSatMap 2.5D/3D, LookTable, ProfileToneCurve, baseline
exposure); creative profiles are **XMP looks** with an RGB table (3D LUT).
Favorites, browser with thumbnails. Treatment: Color / Black & White
[`ConvertToGrayscale`].

**Basic / Light**
- White balance: As Shot / Auto / Daylight / Cloudy / Shade / Tungsten /
  Fluorescent / Flash / Custom; eyedropper (`I`) [`WhiteBalance`].
- Temperature 1500–50000 K (raw; 18.3 lowered min to 1500 K) / −100..100
  (non-raw) [`Temperature`]; Tint −150..150 [`Tint`].
- Exposure −5..+5 EV (10 for HDR) [`Exposure2012`], Contrast −100..100
  [`Contrast2012`], Highlights, Shadows, Whites, Blacks (−100..100)
  [`Highlights2012` …]. **Auto** tone (Ctrl+U, also Shift-double-click a slider
  for per-slider auto).
- **Presence**: Texture, Clarity, Dehaze (−100..100) [`Texture`,
  `Clarity2012`, `Dehaze`], Vibrance, Saturation [`Vibrance`, `Saturation`].
- **HDR** (`HDREditMode`): extended-range editing up to +4 stops above SDR white,
  HDR display preview, visualise HDR range, **SDR settings** (Brightness,
  Contrast, Clarity, Highlights, Shadows, Whites, Blend) for HDR→SDR tone map;
  HDR export (AVIF, JXL, JPEG with gain map, TIFF 32-bit).

**Curve** (Tone Curve): **Parametric** (Highlights, Lights, Darks, Shadows ±100
+ 3 region-split points) [`ParametricShadows`…`ParametricHighlightSplit`];
**Point** curve for RGB + Red + Green + Blue with input/output fields
[`ToneCurvePV2012`, `…Red/Green/Blue`]; **Refine Saturation** slider
[`CurveRefineSaturation`]; targeted adjustment tool (drag in image).

**Detail**: Sharpening Amount 0–150, Radius 0.5–3.0, Detail 0–100, Masking
0–100 [`Sharpness`, `SharpenRadius`, `SharpenDetail`, `SharpenEdgeMasking`];
Noise Reduction Luminance 0–100 + Detail + Contrast
[`LuminanceSmoothing`, `LuminanceNoiseReductionDetail`,
`LuminanceNoiseReductionContrast`]; Color NR 0–100 + Detail + Smoothness
[`ColorNoiseReduction…`]. **Denoise (AI)** amount 0–100 and **Raw Details**,
**Super Resolution** (2× linear) — non-destructive since ACR 17 [`EnhanceDenoise…`,
`EnhanceDetails`, `EnhanceSuperResolution`].

**Color Mixer** (HSL): Hue/Saturation/Luminance for Red, Orange, Yellow, Green,
Aqua, Blue, Purple, Magenta [`HueAdjustmentRed` … `LuminanceAdjustmentMagenta`];
in B&W: B&W mix per band [`GrayMixerRed` …]. Targeted adjustment tool.

**Point Color** (ACR 16.x+): pick up to 8 sample colors; for each: Hue shift,
Saturation shift, Luminance shift, Range (hue/sat/lum range sliders, falloff),
visualize range; **Color Variance** slider (18.0) to compress hue spread
(skin harmonisation). Also available inside masks.

**Color Grading**: 3-way wheels Shadows / Midtones / Highlights (+ Global) each
Hue 0–359, Saturation 0–100, Luminance −100..100; Blending 0–100, Balance
−100..100 [`SplitToningShadowHue`, `…Saturation`, `ColorGradeShadowLum`,
`ColorGradeMidtoneHue/Sat/Lum`, `SplitToningHighlightHue/Saturation`,
`ColorGradeHighlightLum`, `ColorGradeGlobalHue/Sat/Lum`, `ColorGradeBlending`,
`SplitToningBalance`].

**Optics** (Lens Corrections): Remove Chromatic Aberration (auto lateral CA)
[`AutoLateralCA`]; Use Profile Corrections (Adobe LCP lens profiles: distortion
+ vignette + lateral CA models) with Distortion/Vignette amount 0–200 scales
[`LensProfileEnable`, `LensProfileDistortionScale`, …]; Manual: Distortion
−100..100, Vignette Amount + Midpoint [`LensManualDistortionAmount`,
`VignetteAmount`, `VignetteMidpoint`]; **Defringe** Purple Amount 0–20 + Hue
range, Green Amount + Hue range [`DefringePurpleAmount`, `DefringePurpleHueLo/Hi`,
`DefringeGreen…`] + fringe selector eyedropper. **Lens Blur** (AI depth-based
bokeh: amount, bokeh shape — circle/bubble/5-blade/ring/cat-eye, focal range,
focus point, boost; visualize depth) [`LensBlur` struct]. **Distraction
removal**: Reflections removal (AI, amount), People removal, Dust removal
(18.0/18.2: auto detect sensor dust).

**Geometry** (Transform): Upright Off / Auto / Level / Vertical / Full / Guided
(draw up to 4 guides) [`PerspectiveUpright`]; manual Vertical, Horizontal,
Rotate, Aspect, Scale, X Offset, Y Offset [`PerspectiveVertical` …];
Constrain crop; Loupe & grid overlay.

**Effects**: Post-crop Vignette — Style (Highlight Priority / Color Priority /
Paint Overlay), Amount −100..100, Midpoint, Roundness, Feather, Highlights
[`PostCropVignette…`]; Grain — Amount, Size, Roughness [`GrainAmount`,
`GrainSize`, `GrainFrequency`].

**Calibration**: Process version (1/2/3/4/5/6 — 2003/2010/2012; current is
"Version 6" = PV2012 w/ updated engine) [`ProcessVersion`]; Shadows Tint;
Red/Green/Blue Primary Hue & Saturation [`ShadowTint`, `RedHue`,
`RedSaturation`, …].

### 1.4 Crop & Rotate (`C`)
Aspect presets (As shot, 1:1, 4:5/8:10, 8.5:11, 5:7, 2:3/4:6, 16:9, 16:10,
custom) and orientation flip; constrain to image (after warps); straighten tool
(draw horizon) + Auto straighten; angle −45..45; rotate 90° CW/CCW; flip
horizontal/vertical; overlays (rule of thirds, grid, golden ratio, golden spiral,
diagonal, triangle); **Generative Expand** (fill outside canvas, AI) [`CropTop`,
`CropLeft`, `CropBottom`, `CropRight`, `CropAngle`, `HasCrop`,
`CropConstrainToWarp`].

### 1.5 Remove (`B`)
- **Remove** (AI content-aware, brush strokes, optional Generative AI with
  variations; detect objects automatically, select shadow/reflection — 18.2).
- **Heal** (texture-matched, source auto-chosen, editable source) and **Clone**
  modes; size, feather, opacity; brush or click; **Visualize spots** (edge
  threshold view to find dust); show overlay toggle [`RetouchAreas`].
- **Distraction removal**: People (detect & remove), Wires & cables, Dust.

### 1.6 Masking (`Shift+W`)
Mask types (combinable with Add / Subtract / Intersect, invert, named mask
groups, opacity "Amount" per mask, overlay color / mode):
- **Subject**, **Sky**, **Background**, **Objects** (brush/lasso to select),
  **People** (individual persons; parts: face skin, body skin, eyebrows, eye
  sclera, iris & pupil, lips, teeth, hair, clothes), **Landscape** (water,
  vegetation, mountains, architecture, natural ground, artificial ground, sky,
  snow) — all AI.
- **Brush** (size, feather, flow, density, auto mask, A/B brushes, eraser).
- **Linear Gradient** (now two-sided, 18.4), **Radial Gradient** (feather,
  invert).
- **Range**: Color range (eyedropper, refine), Luminance range (range +
  smoothness), Depth range (needs depth map).
- AI mask **Feather** and **Edge** refinement controls (18.3).
Local adjustment sliders per mask: Temp, Tint, Exposure, Contrast, Highlights,
Shadows, Whites, Blacks, Texture, Clarity, Dehaze, Hue (with fine mode),
Saturation, **Color Grading 3-way wheels** (18.3), Point Color, Curves (local,
RGB+channels), Sharpness, Noise, Moiré, Defringe, Color (tint swatch), Amount
(0–200 scaling of all). Local presets (e.g. "Whiten teeth"); adaptive
presets that re-run AI masks.

### 1.7 Red Eye
Red eye & Pet eye; pupil size, darken; pet eye adds catchlight.

### 1.8 Presets / Snapshots / Settings
- Presets (XMP files; groups; amount slider for presets; Adaptive presets:
  Subject/Sky/Portrait; Premium presets; recommended presets; import
  .xmp/.lrtemplate/.zip). Saved to `~/.config/darkslide/presets`.
- **Snapshots**: named saved states.
- Copy/paste settings (Ctrl+Alt+C / V) with subset selection dialog;
  Previous; Save new camera raw defaults; Load/Save settings (subset).

### 1.9 Enhance / merge
- **Denoise**, **Raw Details**, **Super Resolution** (see Detail).
- **Merge to HDR** (bracketed raws → float DNG; align, auto tone, deghost
  off/low/med/high), **Merge to Panorama** (spherical/cylindrical/perspective,
  boundary warp, fill edges, auto crop), **HDR Panorama**.

### 1.10 Output
Save Image (`Ctrl+S`): destination, file naming template, format JPEG
(quality, limit file size) / TIFF (compression, bit depth) / PSD / DNG
(compatibility, embed fast-load data, lossy compression, embed original raw) /
PNG / AVIF / JXL (HDR capable). Color space & bit depth, resize (long edge, short
edge, W×H, megapixels, %) with resolution ppi, output sharpening, metadata
(all / copyright / no location), HDR output (gain map JPEG, AVIF/JXL PQ/HLG).
Background batch saving with progress.

### 1.11 Generative / cloud features (not reproducible offline)
Generative Remove, Generative Expand, Adobe Adaptive (cloud-trained model),
AI masks. We substitute: classical inpainting (PatchMatch) for Remove/Expand,
optional ONNX models (user-supplied, e.g. LaMa, U²-Net/RMBG, sky seg, depth
estimation, NAFNet denoise, Real-ESRGAN) via an optional `ml` feature, with
classical fallbacks so every UI control still does something.

---

## 2. Architecture

```
darkslide/                      (cargo workspace)
  crates/core/   darkslide-core  — engine, no GUI deps
     params/        develop settings model (mirrors crs:), defaults, ranges
     xmp/           crs XMP read/write (sidecar + embedded), presets, snapshots
     decode/        raw (rawler) + raster (image crate) loaders, metadata, orientation
     color/         matrices, WB temp/tint<->xy (DNG spec), DCP parser, ICC out,
                    spaces (ProPhoto/sRGB/AdobeRGB/P3/Rec2020), .cube LUT
     pipeline/      stages + scheduler + caches (see below)
     masks/         mask rasterisation (brush, gradients, range, AI hooks)
     retouch/       heal/clone/remove (PatchMatch), red eye
     lens/          lens profile models (lensfun XML, Adobe LCP), manual corr.
     geometry/      transform/upright/crop warp
     enhance/       denoise, raw details, super resolution, HDR & pano merge
     export/        JPEG/TIFF/PNG/DNG/AVIF/JXL writers, resize, output sharpen
     analysis/      histogram, vectorscope, auto tone, auto WB, clipping
  crates/app/    darkslide       — egui GUI + CLI (`darkslide --cli ...`)
```

### 2.1 Processing pipeline (scene-referred, f32)

Working space: **linear ProPhoto RGB (D50)**, float, unbounded (HDR-safe).

1. **Decode**: CFA/linear raw → `RawImage { cfa, black, white, wb_coeffs,
   cam_to_xyz, crop, orientation, exif }`.
2. **Linearise**: subtract black (per-channel/pattern), scale by white,
   optional dark-frame/flat-field (DNG opcodes).
3. **White balance** multipliers from Temp/Tint via camera matrix
   (DNG SDK algorithm: xy → camera neutral via interpolated ColorMatrix by CCT).
4. **Highlight reconstruction** (clip-aware channel inpainting).
5. **Demosaic**: preview = half-size superpixel; final = RCD (Bayer), Markesteijn
   1-pass (X-Trans). (Enhance "Raw Details" = higher quality demosaic + detail.)
6. **Camera→working**: ForwardMatrix/ColorMatrix → XYZ D50 → ProPhoto;
   DCP HueSatMap + LookTable (HSV tables), Calibration sliders (primary
   hue/sat rotations), baseline exposure.
7. **Geometry** (single resample): lens distortion + lateral CA (per-channel
   radial scale) + Upright/manual transform + crop/rotate + flip. Done as an
   inverse map with bicubic sampling. Vignette (lens) multiplies before.
8. **Detail**: NR (luma: wavelet/NLM; chroma: guided blur in opponent space),
   defringe.
9. **Tone (PV2012-like)**: exposure (gain), white/black points, highlights &
   shadows via local tone mapping on a guided-filtered log-luminance base
   layer, contrast (S-curve around 0.18), texture (band-pass detail),
   clarity (larger-radius edge-aware local contrast), dehaze (dark channel
   prior + atmospheric light).
10. **Color**: vibrance, saturation, Color Mixer (8 smooth hue bands in a
    perceptual hue space), Point Color, Color Grading (wheels in log-lum zones),
    B&W mixer.
11. **Profile look** (3D LUT / creative profile with amount) + **base tone
    curve** (the "Adobe Standard"-like filmic curve), parametric curve,
    point curves (RGB then per channel), refine saturation.
12. **Local adjustments**: each mask yields a weight map; local parameter
    deltas are applied inside the relevant stages above (exposure at 9, color at
    10, curves at 11...). Implemented by computing a per-pixel "local param
    field" only for parameters any mask touches.
13. **Effects**: post-crop vignette (3 styles), grain (seeded, size-aware).
14. **Sharpening**: USM on luminance with detail (halo suppression) and edge
    masking.
15. **Output**: working → output space, encode transfer (sRGB TRC, gamma 2.2,
    PQ/HLG for HDR), dither to 8-bit.

Interactivity: the GUI renders a **preview proxy** (long edge ≤ screen size),
cached after step 6 (demosaic + color) so slider changes only re-run 7–15.
Zoomed 100 % views render a full-res tile of the visible region. Rendering runs
on a worker thread with cancellation (generation counter); rayon inside stages.
GPU (wgpu compute) port of stages 9–15 is a later phase.

### 2.2 Params model
`DevelopSettings` is a plain serde struct, field-for-field mappable to `crs:`
names, with defaults = ACR defaults. XMP serialisation produces sidecars ACR
itself can read for the common fields (round-trip compatibility is a goal).

---

## 3. Phased checklist

Legend: `[x]` done, `[~]` partial (see notes), `[ ]` todo.

### Phase 0 — scaffolding
- [x] Workspace, toolchain pin, test data fetch script (`scripts/fetch-testdata.sh`)
- [x] CLI: `darkslide --cli in.raw -o out/ [--xmp preset.xmp] [--set Exposure2012=1]`

### Phase 1 — decode & base render
- [x] Raw decode via rawler (Bayer + X-Trans + linear DNG); raster via `image`
- [x] EXIF summary (camera, lens, ISO, shutter, aperture, focal, date, GPS)
- [x] Orientation handling
- [x] Black/white level, WB, highlight reconstruction (CFA + LinearRaw)
- [x] Demosaic: superpixel (preview), MHC, RCD (Bayer), Markesteijn (X-Trans)
- [x] Camera → ProPhoto via matrices; Temp/Tint ⇄ multipliers (DNG algorithm)
- [x] Base tone curve + sRGB output; JPEG/PNG/TIFF/EXR/PSD/DNG export

### Phase 2 — Basic panel & tone
- [x] Exposure, Contrast, Highlights, Shadows, Whites, Blacks (local tone mapping)
- [x] Texture, Clarity, Dehaze
- [x] Vibrance, Saturation
- [x] Auto tone, Auto WB, WB presets, eyedropper
- [x] Tone curve parametric + point (RGB/R/G/B), refine saturation, TAT

### Phase 3 — Color
- [x] Color Mixer HSL 8 bands + B&W mixer / grayscale treatment, TAT
- [x] Point Color (8 samples, ranges, color variance, visualize)
- [x] Color Grading (3-way + global, blending, balance)
- [x] Calibration (shadow tint, primaries)
- [x] Profiles: built-ins (Color/Standard/Vivid/Neutral/Landscape/Portrait/
      Monochrome/Adaptive), DCP (HueSatMap/LookTable/ToneCurve/ForwardMatrix),
      creative looks (.cube + procedural) with Amount
- [ ] Camera-matching profiles (vendor looks) — only via user-supplied DCPs

### Phase 4 — Detail, Optics, Geometry, Effects
- [x] Sharpening (amount/radius/detail/masking)
- [x] NR luminance (detail/contrast) + color (detail/smoothness)
- [x] Manual lens distortion + vignette; auto lateral CA; defringe
- [x] Lens profiles: lensfun XML DB (auto-match by EXIF) + Adobe LCP (`~/.config/darkslide/lcp`)
- [x] Transform manual (vertical/horizontal/rotate/aspect/scale/offset)
- [x] Upright auto/level/vertical/full/guided (line detection)
- [x] Crop & rotate, flip, straighten, aspect presets, overlays
- [x] Post-crop vignette (3 styles), Grain
- [x] Lens Blur (heuristic depth)

### Phase 5 — GUI (egui)
- [x] Window layout: preview, right panel with tool strip, filmstrip, top bar
- [x] Edit panel sections with sliders (double-click reset, panel toggles)
- [x] Histogram + clipping warnings + RGB readout; vectorscope + skin line
- [x] Zoom/pan, 100 % tile rendering, before/after modes
- [x] Crop tool overlay (handles, aspect, grid overlays, straighten)
- [x] Curve editor widget; color wheels widget; HSL TAT
- [x] Undo/redo history, snapshots, presets browser, copy/paste settings
- [x] Filmstrip: open folder, thumbnails, ratings/labels, filter, sync settings
- [x] Save dialog (formats, sizing, color space, naming, batch, progress)
- [x] Preferences (XDG), keyboard shortcuts matching ACR
- [x] Profile browser with thumbnails, preset hover preview — loupe, AF points [ ]

### Phase 6 — XMP compatibility
- [x] Read/write crs: sidecars for all global params (+ lossless ds:Settings)
- [x] Presets (.xmp) import/export, preset amount, built-in presets
- [x] Masks: crs:MaskGroupBasedCorrections (gradient/radial/brush) read+write; AI/range masks JSON only

### Phase 7 — Masking & local adjustments
- [x] Mask model: groups, components (add/subtract/intersect), invert, amount
- [x] Brush (size/feather/flow/density/auto-mask/eraser)
- [x] Linear gradient (incl. two-sided), radial gradient
- [x] Color range, luminance range, depth range
- [~] AI: subject/sky/background/objects/landscape via heuristics; People parts [ ]
- [x] Local params incl. local curve, color grading wheels, point color, tint
- [x] Overlay visualisation

### Phase 8 — Retouch
- [x] Clone + heal spots (circle + brush), auto source search, visualize spots
- [x] Remove (PatchMatch content-aware inpainting)
- [x] Red eye / pet eye
- [x] Dust auto-detect
- [x] Wires/cables, people removal (heuristic / ML when installed); reflections [ ]

### Phase 9 — Enhance & merge
- [x] Denoise (NLM + guided chroma)
- [x] Raw Details (RL deconvolution), Super Resolution (2x on export)
- [x] Merge to HDR (MTB align, weights, deghost, float DNG out)
- [x] Merge to Panorama (features, homography, spherical/cylindrical/
      perspective, blending, auto crop / fill edges, boundary warp)
- [x] HDR editing mode + HDR export (EXR, float TIFF, gain-map JPEG)
- [x] AVIF output (SDR, 10-bit, pure-Rust ravif; feature `avif`, default on)
- [x] JPEG XL output (lossless 8/16-bit sRGB, pure-Rust zune-jpegxl; feature `jxl`)

### Phase 10 — Performance & polish
- [ ] wgpu compute pipeline for interactive stages
- [x] Thumbnail cache on disk (~/.cache/darkslide/thumbs; refreshed from edited renders)
- [x] DNG export (rawler convert), embedded XMP read
- [x] Packaging: `.desktop`, icon, scripts/install.sh, Flatpak manifest template
- [x] Optional ONNX (`ort`, load-dynamic) backend: subject/sky/objects/depth
      (`ml.rs`, feature `ml`); verified with u2netp + onnxruntime 1.20.1

---

## 4. Conventions
- Engine is pure Rust; GUI never touches pixels except via `Renderer`.
- Pixel buffers: `Image3 { w, h, data: Vec<[f32;3]> }` row-major.
- All slider params use ACR units/ranges; mapping to math lives in the stage.
- Every stage must be a no-op at default params (cheap skip).
- Tests: golden-ish sanity tests on synthetic images + decode smoke tests on
  `testdata/` (fetched by script, git-ignored).

---

## 5. Status / resume notes
(update at the end of every session)

Build: `PATH=$HOME/.cargo/bin:$PATH cargo build --release` (the system
/usr/bin/rustc 1.75 shadows rustup's; always put ~/.cargo/bin first).
Test raws: `testdata/` (CR3, NEF, ARW, RAF X-Trans, iPhone DNG, RW2) from
raw.pixls.us (CC0). CLI smoke test:
`./target/release/darkslide --cli -s 1600 --no-sidecar -o /tmp/out testdata/*`

### Session 1 (2026-10-11)
Done (engine, `crates/core`):
- decode (rawler 0.8 + image crate), orientation, metadata
- demosaic: superpixel draft, MHC Bayer, colour-difference X-Trans;
  highlight reconstruction using observed per-channel clip levels
- colour: DNG-style dual-illuminant matrices, temp/tint (DNG SDK algorithm),
  Oklab, output spaces, ICC v2 writer
- pipeline (`pipeline/`): geometry (rotate/flip, manual perspective,
  upright matrix hook, lens distortion incl. lensfun ptlens, lateral CA,
  straighten+crop), tone (exposure, H/S/W/B with fast-guided-filter base
  layer from a 640px proxy, clarity, texture, dehaze, contrast, base curve),
  colour (profiles, vibrance, saturation, HSL mixer, B&W mix, point colour,
  grading, calibration, shadow tint), curves (parametric/point/RGB, refine
  sat), looks (.cube + procedural built-ins), NR, sharpening, defringe,
  post-crop vignette (3 styles), grain, lens vignette, red eye
- masks: brush, linear (two-sided), radial, colour/luminance/depth range,
  AI raster slot; add/subtract/intersect; local scalar params + local curves,
  grading, colour tint, point colour
- retouch: heal/clone/remove (PatchMatch inpaint), visualize spots, dust detect
- xmp: crs read/write (+ lossless ds:Settings JSON), presets with amount
- export: tiled full-res render, resize modes, output sharpening, JPEG
  (EXIF + ICC, size limit) / PNG 8/16 / TIFF 8/16/32 (ICC)
- analysis: histogram, clipping overlay, vectorscope
- CLI (`--cli`) works on all test files.

GUI (`crates/app`, eframe/egui 0.36, wgpu):
- layout: menu bar, viewer, right panel (histogram, EXIF, tool strip,
  panels), filmstrip, bottom bar (zoom, before/after, clipping, Save/Done)
- render worker thread with job coalescing + low-res pass while dragging;
  detail tile rendering when zoomed; before/after (4 layouts + toggle)
- Edit panel: all sections incl. curve editor, colour wheels, point colour,
  HSL tabs, B&W mixer, optics, geometry (upright buttons), effects, calibration
- tools: crop (rotated box, handles, aspect, straighten, overlays), remove
  (heal/clone/remove, sources, visualize spots, dust), masking (brush,
  linear, radial, colour/lum/depth range, heuristic AI masks, overlay),
  red eye, presets (import/apply/amount/save), snapshots
- WB eyedropper, colour samplers, targeted adjustment (curve + mixer)
- undo/redo, XMP sidecar autosave, ratings/labels, copy/paste/sync subsets,
  camera defaults, Save Image dialog with background batch export, prefs
- headless screenshots: `cargo build --release -p darkslide --features
  screenshot` then `darkslide --screenshot out.png [--tool crop]
  [--set Exposure2012=1] [--json settings.json] FILE` (uses egui_kittest)
- core `upright.rs`: Hough segments, level angle, vanishing-point
  rectification (Level/Vertical/Full/Auto/Guided), auto straighten
- core `ai.rs`: heuristic subject/sky/background/landscape masks + depth

More engine work (session 1, later):
- demosaic: RCD (Bayer, full quality) + rawler Markesteijn (X-Trans)
- `enhance.rs`: Denoise (fast NLM + guided chroma), Raw Details (RL
  deconvolution), Super Resolution (2x render + refine on export)
- HDR: `hdr_to_sdr` (SDR settings) for display/SDR export, HDR visualise
  overlay, OpenEXR export (linear float)
- `dcp.rs`: DNG camera profiles (.dcp) incl. ForwardMatrix, HueSatMap,
  LookTable, ToneCurve; listed in the profile picker when they match
- Lens Blur: heuristic depth (`ai::estimate_depth`), layered blur with
  highlight boost, bokeh shapes, focal range, visualise depth
- `merge.rs`: HDR (MTB align, exposure ratios, weighted merge, deghost) and
  Panorama (Harris + descriptors, RANSAC homographies, spherical/cylindrical/
  perspective, feathered blend, auto-crop / fill edges) → float LinearRaw DNG
  via rawler's DNG writer; GUI File ▸ Merge…; CLI --merge-hdr/--merge-pano
- crop "expand" filled with PatchMatch on export (Generative Expand stand-in)
- LinearRaw DNG highlight reconstruction
- lensfun DB verified (all test lenses match); scripts/fetch-lensfun.sh
- README.md, packaging/darkslide.desktop, scripts/fetch-testdata.sh

Session 1, later still:
- Alt-drag clipping previews (Exposure/Whites/Highlights, Blacks/Shadows,
  sharpening Masking); Point Color "visualize range"
- built-in presets (`builtin_presets.rs`) incl. adaptive Subject/Sky/
  Portrait ones that add AI masks; pending AI masks auto-detect in the UI
- Objects mask (drag a box → hint-restricted subject detection); AI Edge
- settings embedded in DNG/JPEG XMP are read when no sidecar exists
- Save as DNG (original raw via rawler convert, lossless + sidecar),
  PSD (8/16-bit, ICC), HDR JPEG with gain map (`gainmap.rs`, MPF verified)
- GUI smoke test: `darkslide --smoke FILES` (features=screenshot) clicks
  through every tool/panel; passes. `cargo test`: 24 tests pass.
- rustfmt.toml (max_width 160); code formatted; no warnings.

- Distraction removal: People (person segmentation → mask-based Remove spot)
  and Wires & cables (black-hat + linearity filter over bright smooth areas);
  `RetouchSpot.mask` raster fill (`distract.rs`, unit-tested)
- Panorama boundary warp (column/row coverage remap)
- ACR mask interop (`xmp_masks.rs`): crs:MaskGroupBasedCorrections read/write
  for Gradient / CircularGradient / Paint + local sliders (round-trip test).
  Coordinate frame assumed = our T frame (pre-crop, oriented) — verify against
  real Lightroom sidecars when available.
- Perf: stage timing (`RUST_LOG=darkslide_core=debug`); NR skipped below 1/3
  scale and chroma NR on a subsampled guided filter; mask colours only when
  needed → default 1500x1000 preview ≈45 ms render (+≈17 ms tone ctx).
- Shift+double-click per-slider auto; grid overlay (Ctrl+').
- `cargo run --release --example bench -p darkslide-core -- FILE` benchmarks.

- Preset hover preview; profile browser (thumbnails of every profile/look);
  fixed panel-width feedback loop; sidecars only written when autosave is on
- Packaging: icon, scripts/install.sh, Flatpak manifest template
- AVIF export; Adobe LCP lens profiles (parse + convert to our radial model)

- Upright safeguards (long lines only, spread check, tilt limits, corner-shift
  limit with blend-back, footprint-preserving scale); "Constrain crop"
  (`Engine::constrain_crop`, UI checkbox, CLI --constrain); CLI --upright
- Viewer bug fixed: release frames have no interact pos → tools acting on
  release (red eye, straighten, guides, object box) got zero-size input and
  eyedropper clicks could be lost; near-zero drags now count as clicks
- Smoke test now also drives pointer drags for crop, brush/linear/radial/
  object masks, remove, red eye, TAT and the WB eyedropper — passes on all
  six sample cameras

### Session 2 (2026-10-11 evening)
- Merged `camera-raw-ui` (user's ACR-style redesign) into master after fixes:
  slider id clashes (shared labels drove each other), section titles not
  toggling (selectable Label over the header), "Temp" label broke per-slider
  auto, hard-coded workflow line. Sliders follow ACR's two-line layout; Edit
  header with Auto / B&W; workflow line reflects real export settings.
- Remove tool: replaced PatchMatch (its per-pixel matches were incoherent →
  flat fills) with exemplar onion-peel inpainting (Criminisi): coherent
  texture, ~10x faster; retouch results cached per spot prefix; large
  strokes filled at reduced size. Full-res edge stroke 8.7 s → 0.37 s.
- Reference ACR screenshots: jkost.com/blog/?p=17009 (v12.3 UI, still current).

### How to resume
1. `export PATH=$HOME/.cargo/bin:$PATH; scripts/fetch-testdata.sh`
2. `cargo test && cargo build --release -p darkslide --features screenshot`
3. `./target/release/darkslide --smoke testdata/nikon_d750.nef` (no panics)
4. Pick from "Known gaps / next" below or the unchecked boxes in section 3.
   Visual checks: `--screenshot out.png [--tool X] [--json s.json]`, CLI
   renders with `--set` (see the contact-sheet approach in session notes).

Known gaps / next:
- lensfun DB: Optics panel / Preferences download it
  into ~/.local/share/darkslide/lensfun (curl + tar).
- HDR AVIF/JXL (PQ/HLG), lossy JXL, merge HDR/pano, denoise AI,
  super resolution, lens blur, AI masks: not yet.
