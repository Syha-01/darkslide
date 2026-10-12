//! Dialog windows: Save Image, copy/paste settings subset, preferences,
//! shortcuts, about.

use egui_phosphor::regular as ic;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use darkslide_core::color::ColorSpace;
use darkslide_core::exif_write::MetadataMode;
use darkslide_core::export::{ExportSettings, Format, Resize, SharpenAmount, SharpenFor, TiffCompression};
use darkslide_core::params::DevelopSettings;
use darkslide_core::{Engine, xmp};
use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};

use crate::app::App;

pub fn pick_files() -> Option<Vec<PathBuf>> {
    let mut exts: Vec<&str> = darkslide_core::decode::RASTER_EXTENSIONS.to_vec();
    let raw: Vec<String> = rawler::decoders::supported_extensions().iter().flat_map(|e| [e.to_lowercase(), e.to_uppercase()]).collect();
    let raw_refs: Vec<&str> = raw.iter().map(|s| s.as_str()).collect();
    exts.extend(raw_refs.iter());
    rfd::FileDialog::new().add_filter("Images", &exts).add_filter("All files", &["*"]).pick_files()
}

// ---------------------------------------------------------------------------
// Preferences

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub autosave_sidecars: bool,
    pub show_vectorscope: bool,
    pub export: ExportSettings,
    pub ui_scale: f32,
    /// Index into theme::NAMES.
    pub theme: u8,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { autosave_sidecars: true, show_vectorscope: false, export: ExportSettings::default(), ui_scale: 1.0, theme: 0 }
    }
}

impl Prefs {
    fn path() -> PathBuf {
        darkslide_core::paths::config_dir().join("prefs.json")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        let p = Self::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, t);
        }
    }
}

// ---------------------------------------------------------------------------
// Save Image

#[derive(Default)]
pub struct ExportDialog {
    pub open: bool,
    pub settings: Option<ExportSettings>,
    pub all_selected: bool,
}

pub struct ExportStatus {
    pub total: usize,
    pub finished: usize,
    pub progress: f32,
    pub done: bool,
    pub message: Option<String>,
}

fn export_window(app: &mut App, ctx: &egui::Context) {
    if !app.export.open {
        return;
    }
    let mut ex = app.export.settings.take().unwrap_or_else(|| app.prefs.export.clone());
    let mut open = true;
    let mut go = false;
    let n_sel = app.docs.iter().filter(|d| d.selected).count();
    egui::Window::new("Save Image").open(&mut open).collapsible(false).resizable(false).default_width(420.0).show(ctx, |ui| {
        egui::Grid::new("export").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Destination");
            ui.horizontal(|ui| {
                let label = ex.dest.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Same folder as original".into());
                ui.label(RichText::new(label).small());
                if ui.button("Choose…").clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        ex.dest = Some(d);
                    }
                }
                if ex.dest.is_some() && ui.small_button(ic::X).clicked() {
                    ex.dest = None;
                }
            });
            ui.end_row();
            ui.label("File name");
            ui.add(egui::TextEdit::singleline(&mut ex.naming).hint_text("{name}  {seq} {date} {camera}"));
            ui.end_row();
            ui.label("Format");
            egui::ComboBox::from_id_salt("fmt").selected_text(format!("{:?}", ex.format)).show_ui(ui, |ui| {
                ui.selectable_value(&mut ex.format, Format::Jpeg, "JPEG");
                ui.selectable_value(&mut ex.format, Format::Tiff, "TIFF");
                ui.selectable_value(&mut ex.format, Format::Png, "PNG");
                ui.selectable_value(&mut ex.format, Format::Exr, "OpenEXR (HDR, linear)");
                ui.selectable_value(&mut ex.format, Format::Psd, "Photoshop (PSD)");
                ui.selectable_value(&mut ex.format, Format::Avif, "AVIF");
                ui.selectable_value(&mut ex.format, Format::Jxl, "JPEG XL (lossless)");
                ui.selectable_value(&mut ex.format, Format::Dng, "DNG (original raw + settings)");
            });
            ui.end_row();
            match ex.format {
                Format::Jpeg => {
                    ui.label("Quality");
                    ui.add(egui::Slider::new(&mut ex.jpeg_quality, 1..=100));
                    ui.end_row();
                    ui.label("HDR");
                    ui.checkbox(&mut ex.hdr_gainmap, "Include gain map for HDR edits (Ultra HDR)");
                    ui.end_row();
                    ui.label("Limit file size (KB)");
                    ui.add(egui::DragValue::new(&mut ex.jpeg_limit_kb).range(0..=100000).suffix(" KB"));
                    ui.end_row();
                    ex.bit_depth = 8;
                }
                Format::Tiff => {
                    ui.label("Bit depth");
                    ui.horizontal(|ui| {
                        for b in [8u8, 16, 32] {
                            ui.selectable_value(&mut ex.bit_depth, b, format!("{b}-bit"));
                        }
                    });
                    ui.end_row();
                    ui.label("Compression");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut ex.tiff_compression, TiffCompression::None, "None");
                        ui.selectable_value(&mut ex.tiff_compression, TiffCompression::Lzw, "LZW");
                        ui.selectable_value(&mut ex.tiff_compression, TiffCompression::Deflate, "ZIP");
                    });
                    ui.end_row();
                }
                Format::Avif => {
                    ui.label("Quality");
                    ui.add(egui::Slider::new(&mut ex.jpeg_quality, 1..=100));
                    ui.end_row();
                }
                Format::Jxl => {
                    ui.label("Bit depth");
                    ui.horizontal(|ui| {
                        for b in [8u8, 16] {
                            ui.selectable_value(&mut ex.bit_depth, b, format!("{b}-bit"));
                        }
                    });
                    ui.end_row();
                }
                Format::Psd => {
                    ui.label("Bit depth");
                    ui.horizontal(|ui| {
                        for b in [8u8, 16] {
                            ui.selectable_value(&mut ex.bit_depth, b, format!("{b}-bit"));
                        }
                    });
                    ui.end_row();
                }
                Format::Dng => {
                    ui.label("");
                    ui.label(RichText::new("Lossless; edits saved in the .xmp sidecar").small());
                    ui.end_row();
                }
                Format::Exr => {
                    ui.label("");
                    ui.label(RichText::new("32-bit float, linear, keeps HDR highlights").small());
                    ui.end_row();
                }
                Format::Png => {
                    ui.label("Bit depth");
                    ui.horizontal(|ui| {
                        for b in [8u8, 16] {
                            ui.selectable_value(&mut ex.bit_depth, b, format!("{b}-bit"));
                        }
                    });
                    ui.end_row();
                }
            }
            ui.label("Color space");
            egui::ComboBox::from_id_salt("space").selected_text(ex.color_space.name()).show_ui(ui, |ui| {
                for s in ColorSpace::ALL {
                    ui.selectable_value(&mut ex.color_space, s, s.name());
                }
            });
            ui.end_row();
            ui.label("Resize");
            ui.horizontal(|ui| {
                let label = match ex.resize {
                    Resize::None => "Off",
                    Resize::LongEdge(_) => "Long edge",
                    Resize::ShortEdge(_) => "Short edge",
                    Resize::WidthHeight(..) => "Width × Height",
                    Resize::Megapixels(_) => "Megapixels",
                    Resize::Percent(_) => "Percentage",
                };
                egui::ComboBox::from_id_salt("resize").selected_text(label).show_ui(ui, |ui| {
                    ui.selectable_value(&mut ex.resize, Resize::None, "Off");
                    ui.selectable_value(&mut ex.resize, Resize::LongEdge(2048), "Long edge");
                    ui.selectable_value(&mut ex.resize, Resize::ShortEdge(1080), "Short edge");
                    ui.selectable_value(&mut ex.resize, Resize::WidthHeight(1920, 1080), "Width × Height");
                    ui.selectable_value(&mut ex.resize, Resize::Megapixels(12.0), "Megapixels");
                    ui.selectable_value(&mut ex.resize, Resize::Percent(50.0), "Percentage");
                });
                match &mut ex.resize {
                    Resize::LongEdge(v) | Resize::ShortEdge(v) => {
                        ui.add(egui::DragValue::new(v).range(16..=60000).suffix(" px"));
                    }
                    Resize::WidthHeight(w, h) => {
                        ui.add(egui::DragValue::new(w).range(16..=60000));
                        ui.label("×");
                        ui.add(egui::DragValue::new(h).range(16..=60000));
                    }
                    Resize::Megapixels(m) => {
                        ui.add(egui::DragValue::new(m).range(0.1..=500.0).speed(0.1).suffix(" MP"));
                    }
                    Resize::Percent(p) => {
                        ui.add(egui::DragValue::new(p).range(1.0..=400.0).suffix(" %"));
                    }
                    Resize::None => {}
                }
            });
            ui.end_row();
            if ex.resize != Resize::None {
                ui.label("");
                ui.checkbox(&mut ex.dont_enlarge, "Don't enlarge");
                ui.end_row();
            }
            ui.label("Resolution");
            ui.add(egui::DragValue::new(&mut ex.ppi).range(1..=4800).suffix(" ppi"));
            ui.end_row();
            ui.label("Output sharpening");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("osharp").selected_text(format!("{:?}", ex.sharpen_for)).show_ui(ui, |ui| {
                    for m in [SharpenFor::None, SharpenFor::Screen, SharpenFor::Matte, SharpenFor::Glossy] {
                        ui.selectable_value(&mut ex.sharpen_for, m, format!("{m:?}"));
                    }
                });
                if ex.sharpen_for != SharpenFor::None {
                    egui::ComboBox::from_id_salt("osharpamt").selected_text(format!("{:?}", ex.sharpen_amount)).show_ui(ui, |ui| {
                        for a in [SharpenAmount::Low, SharpenAmount::Standard, SharpenAmount::High] {
                            ui.selectable_value(&mut ex.sharpen_amount, a, format!("{a:?}"));
                        }
                    });
                }
            });
            ui.end_row();
            ui.label("Metadata");
            egui::ComboBox::from_id_salt("meta").selected_text(format!("{:?}", ex.metadata)).show_ui(ui, |ui| {
                for m in [MetadataMode::All, MetadataMode::AllExceptLocation, MetadataMode::CopyrightOnly, MetadataMode::None] {
                    ui.selectable_value(&mut ex.metadata, m, format!("{m:?}"));
                }
            });
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut ex.write_sidecar, "Write settings sidecar next to output");
            ui.end_row();
        });
        if n_sel > 1 {
            ui.checkbox(&mut app.export.all_selected, format!("Save all {n_sel} selected images"));
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Save").strong()).clicked() {
                go = true;
            }
            if ui.button("Cancel").clicked() {
                app.export.open = false;
            }
        });
    });
    if go {
        start_export(app, ex.clone());
        app.prefs.export = ex.clone();
        app.prefs.save();
        app.export.open = false;
    }
    if !open {
        app.export.open = false;
    }
    app.export.settings = Some(ex);
}

/// Run exports on a background thread (one image at a time, each using all cores).
fn start_export(app: &mut App, ex: ExportSettings) {
    let mut jobs: Vec<(PathBuf, DevelopSettings, Option<Arc<Engine>>)> = vec![];
    let all = app.export.all_selected;
    for (i, d) in app.docs.iter().enumerate() {
        if (all && d.selected) || (!all && Some(i) == app.current) {
            jobs.push((d.path.clone(), d.settings.clone(), d.engine.clone()));
        }
    }
    if jobs.is_empty() {
        return;
    }
    let status = Arc::new(Mutex::new(ExportStatus { total: jobs.len(), finished: 0, progress: 0.0, done: false, message: None }));
    let shared = app.export_status.clone();
    shared.lock().unwrap().retain(|s| !s.done);
    // keep a handle in the app list through an Arc indirection
    let st2 = status.clone();
    std::thread::spawn(move || {
        let mut ok = 0;
        let mut errs = vec![];
        let n = jobs.len();
        for (k, (path, settings, engine)) in jobs.into_iter().enumerate() {
            let engine = match engine {
                Some(e) => e,
                None => match Engine::open(&path) {
                    Ok(e) => {
                        let s = xmp::read_sidecar(&path, e.is_raw()).map(|(s, _)| s).unwrap_or(settings.clone());
                        let _ = s;
                        Arc::new(e)
                    }
                    Err(e) => {
                        errs.push(format!("{}: {e}", path.display()));
                        continue;
                    }
                },
            };
            let st3 = st2.clone();
            let r = darkslide_core::export::save(&engine, &settings, &ex, ex.start_seq + k as u32, &move |p| {
                st3.lock().unwrap().progress = (k as f32 + p) / n as f32;
            });
            match r {
                Ok(_) => ok += 1,
                Err(e) => errs.push(format!("{}: {e}", path.display())),
            }
            st2.lock().unwrap().finished = k + 1;
        }
        let mut s = st2.lock().unwrap();
        s.done = true;
        s.message = Some(if errs.is_empty() { format!("Saved {ok} image(s)") } else { format!("Saved {ok}, {} failed: {}", errs.len(), errs.join("; ")) });
    });
    // mirror into the shared list the bottom bar reads
    let shared2 = shared.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let s = status.lock().unwrap();
            let mut list = shared2.lock().unwrap();
            list.clear();
            list.push(ExportStatus { total: s.total, finished: s.finished, progress: s.progress, done: s.done, message: s.message.clone() });
            if s.done {
                break;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Copy / paste subsets

#[derive(Clone, Debug)]
pub struct Subset {
    pub flags: Vec<(&'static str, bool)>,
}

impl Subset {
    pub fn all() -> Self {
        let mut s = Self::default();
        for f in s.flags.iter_mut() {
            f.1 = !matches!(f.0, "Crop" | "Spot removal" | "Red eye" | "Masks");
        }
        s
    }

    fn on(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, v)| *n == name && *v)
    }

    pub fn apply(&self, dst: &mut DevelopSettings, src: &DevelopSettings) {
        if self.on("Profile") {
            dst.profile = src.profile.clone();
            dst.profile_amount = src.profile_amount;
            dst.look = src.look.clone();
            dst.treatment = src.treatment;
        }
        if self.on("White balance") {
            dst.wb_mode = src.wb_mode;
            dst.temperature = src.temperature;
            dst.tint = src.tint;
        }
        if self.on("Exposure & tone") {
            dst.exposure = src.exposure;
            dst.contrast = src.contrast;
            dst.highlights = src.highlights;
            dst.shadows = src.shadows;
            dst.whites = src.whites;
            dst.blacks = src.blacks;
            dst.hdr = src.hdr;
        }
        if self.on("Presence") {
            dst.texture = src.texture;
            dst.clarity = src.clarity;
            dst.dehaze = src.dehaze;
            dst.vibrance = src.vibrance;
            dst.saturation = src.saturation;
        }
        if self.on("Curve") {
            dst.parametric = src.parametric;
            dst.curve = src.curve.clone();
            dst.refine_saturation = src.refine_saturation;
        }
        if self.on("Detail") {
            dst.sharpen = src.sharpen;
            dst.nr = src.nr;
            dst.enhance = src.enhance;
        }
        if self.on("Color mixer") {
            dst.hsl = src.hsl;
            dst.gray_mix = src.gray_mix;
        }
        if self.on("Point color") {
            dst.point_colors = src.point_colors.clone();
        }
        if self.on("Color grading") {
            dst.grading = src.grading;
        }
        if self.on("Optics") {
            dst.lens = src.lens.clone();
        }
        if self.on("Geometry") {
            dst.transform = src.transform.clone();
        }
        if self.on("Effects") {
            dst.vignette = src.vignette;
            dst.grain = src.grain;
        }
        if self.on("Calibration") {
            dst.calibration = src.calibration;
        }
        if self.on("Crop") {
            dst.crop = src.crop;
        }
        if self.on("Masks") {
            dst.masks = src.masks.clone();
        }
        if self.on("Spot removal") {
            dst.retouch = src.retouch.clone();
        }
        if self.on("Red eye") {
            dst.red_eye = src.red_eye.clone();
        }
        dst.panels = src.panels;
    }
}

impl Default for Subset {
    fn default() -> Self {
        let names = [
            "Profile",
            "White balance",
            "Exposure & tone",
            "Presence",
            "Curve",
            "Detail",
            "Color mixer",
            "Point color",
            "Color grading",
            "Optics",
            "Geometry",
            "Effects",
            "Calibration",
            "Crop",
            "Masks",
            "Spot removal",
            "Red eye",
        ];
        Self { flags: names.iter().map(|n| (*n, true)).collect() }
    }
}

#[derive(Default)]
pub struct CopyDialog {
    pub open: bool,
    pub paste: bool,
    pub subset: Option<Subset>,
}

impl CopyDialog {
    pub fn open_for_copy(&mut self) {
        self.open = true;
        self.paste = false;
    }
    pub fn open_for_paste(&mut self) {
        self.open = true;
        self.paste = true;
    }
}

fn copy_window(app: &mut App, ctx: &egui::Context) {
    if !app.copy.open {
        return;
    }
    let mut subset = app.copy.subset.take().unwrap_or_else(Subset::all);
    let mut open = true;
    let mut apply = false;
    let title = if app.copy.paste { "Paste / sync settings" } else { "Copy settings" };
    egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.small_button("Check all").clicked() {
                subset.flags.iter_mut().for_each(|f| f.1 = true);
            }
            if ui.small_button("Check none").clicked() {
                subset.flags.iter_mut().for_each(|f| f.1 = false);
            }
        });
        egui::Grid::new("subset").num_columns(2).show(ui, |ui| {
            for (i, (name, v)) in subset.flags.iter_mut().enumerate() {
                ui.checkbox(v, *name);
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        ui.separator();
        let n = app.docs.iter().filter(|d| d.selected).count().max(1);
        let label = if app.copy.paste { format!("Paste to {n} image(s)") } else { "OK".into() };
        if ui.button(label).clicked() {
            apply = true;
        }
    });
    if apply {
        if app.copy.paste {
            let s = subset.clone();
            app.paste_settings(Some(&s));
        }
        app.copy.open = false;
    }
    if !open {
        app.copy.open = false;
    }
    app.copy.subset = Some(subset);
}

fn prefs_window(app: &mut App, ctx: &egui::Context) {
    if !app.show_prefs {
        return;
    }
    let mut open = true;
    egui::Window::new("Preferences").open(&mut open).collapsible(false).show(ctx, |ui| {
        ui.checkbox(&mut app.prefs.autosave_sidecars, "Automatically save settings to XMP sidecars");
        ui.checkbox(&mut app.edit.show_scope, "Show vectorscope");
        ui.horizontal(|ui| {
            ui.label("Interface scale");
            if ui.add(egui::Slider::new(&mut app.prefs.ui_scale, 0.75..=2.0).step_by(0.05)).changed() {
                ctx.set_zoom_factor(app.prefs.ui_scale);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Theme");
            let before = app.prefs.theme;
            egui::ComboBox::from_id_salt("theme").selected_text(crate::theme::NAMES[(app.prefs.theme as usize).min(crate::theme::NAMES.len() - 1)]).show_ui(
                ui,
                |ui| {
                    for (i, n) in crate::theme::NAMES.iter().enumerate() {
                        ui.selectable_value(&mut app.prefs.theme, i as u8, *n);
                    }
                },
            );
            if app.prefs.theme != before {
                crate::theme::set(ctx, app.prefs.theme);
            }
        });
        ui.separator();
        ui.label(RichText::new("Locations").strong());
        ui.label(format!("Presets: {}", darkslide_core::paths::presets_dir().display()));
        ui.label(format!("Profiles (.cube): {}", darkslide_core::profiles::user_looks_dir().display()));
        ui.horizontal(|ui| {
            ui.label(format!("Lens database: {} (lensfun XML)", darkslide_core::paths::data_dir().join("lensfun").display()));
            if ui.button("Update").on_hover_text("Download the latest lensfun lens profiles").clicked() {
                app.download_lens_db();
            }
        });
        ui.label(format!("Camera defaults: {}", darkslide_core::paths::config_dir().join("defaults").display()));
        if ui.button("Save").clicked() {
            app.prefs.show_vectorscope = app.edit.show_scope;
            app.prefs.save();
        }
    });
    if !open {
        app.prefs.save();
        app.show_prefs = false;
    }
}

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+O / Ctrl+Shift+O", "Open files / folder"),
    ("Ctrl+S", "Save image"),
    ("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo"),
    ("Ctrl+C / Ctrl+V", "Copy / paste settings"),
    ("Ctrl+Alt+C / Ctrl+Alt+V", "Copy / paste chosen settings"),
    ("Ctrl+U / Ctrl+Shift+U", "Auto tone / auto white balance"),
    ("Ctrl+R", "Reset to defaults"),
    ("E C B M S", "Edit, Crop, Remove, Masking, Snapshots"),
    ("I", "White balance tool"),
    ("U / O", "Shadow / highlight clipping"),
    ("Q", "Cycle before/after views"),
    ("\\ or P", "Toggle before"),
    ("Z, double-click", "Toggle fit / 100%"),
    ("Ctrl+= / Ctrl+-, wheel", "Zoom"),
    ("Space-drag, middle-drag", "Pan"),
    ("← / →", "Previous / next image"),
    ("0–5 / 6–9", "Rating / colour label"),
    ("[ / ]", "Brush size"),
    ("Y", "Mask overlay"),
    ("Tab", "Filmstrip"),
    ("F", "Full screen"),
    ("Enter / Esc", "Commit crop / cancel tool"),
    ("Shift (slider drag)", "Fine adjustment"),
    ("Double-click slider", "Reset slider"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MergeKind {
    Hdr,
    Pano,
}

pub struct MergeDialog {
    pub open: Option<MergeKind>,
    pub deghost: darkslide_core::merge::Deghost,
    pub align: bool,
    pub projection: darkslide_core::merge::Projection,
    pub auto_crop: bool,
    pub fill_edges: bool,
    pub boundary_warp: f32,
    pub result: Arc<Mutex<Option<Result<PathBuf, String>>>>,
    pub running: bool,
    pub progress: Arc<Mutex<f32>>,
}

impl Default for MergeDialog {
    fn default() -> Self {
        Self {
            open: None,
            deghost: darkslide_core::merge::Deghost::Off,
            align: true,
            projection: darkslide_core::merge::Projection::Spherical,
            auto_crop: true,
            fill_edges: false,
            boundary_warp: 0.0,
            result: Arc::new(Mutex::new(None)),
            running: false,
            progress: Arc::new(Mutex::new(0.0)),
        }
    }
}

fn merge_window(app: &mut App, ctx: &egui::Context) {
    use darkslide_core::merge::{self, Deghost, Projection};
    // collect a finished merge
    let finished = app.merge.result.lock().unwrap().take();
    if let Some(r) = finished {
        app.merge.running = false;
        match r {
            Ok(p) => {
                app.status = format!("Created {}", p.display());
                app.open_paths(vec![p]);
            }
            Err(e) => app.status = format!("Merge failed: {e}"),
        }
    }
    let Some(kind) = app.merge.open else { return };
    let paths: Vec<PathBuf> = app.docs.iter().filter(|d| d.selected).map(|d| d.path.clone()).collect();
    let mut open = true;
    let title = if kind == MergeKind::Hdr { "Merge to HDR" } else { "Merge to Panorama" };
    let mut go = false;
    egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        ui.label(format!("{} image(s) selected in the filmstrip (Ctrl-click to select).", paths.len()));
        match kind {
            MergeKind::Hdr => {
                ui.checkbox(&mut app.merge.align, "Auto align");
                ui.horizontal(|ui| {
                    ui.label("Deghost");
                    for (d, l) in [(Deghost::Off, "Off"), (Deghost::Low, "Low"), (Deghost::Medium, "Medium"), (Deghost::High, "High")] {
                        ui.selectable_value(&mut app.merge.deghost, d, l);
                    }
                });
            }
            MergeKind::Pano => {
                ui.horizontal(|ui| {
                    ui.label("Projection");
                    for (p, l) in [(Projection::Spherical, "Spherical"), (Projection::Cylindrical, "Cylindrical"), (Projection::Perspective, "Perspective")] {
                        ui.selectable_value(&mut app.merge.projection, p, l);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Boundary warp");
                    ui.add(egui::Slider::new(&mut app.merge.boundary_warp, 0.0..=100.0));
                });
                ui.checkbox(&mut app.merge.auto_crop, "Auto crop");
                ui.add_enabled(!app.merge.auto_crop, egui::Checkbox::new(&mut app.merge.fill_edges, "Fill edges"));
            }
        }
        ui.separator();
        if app.merge.running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add(egui::ProgressBar::new(*app.merge.progress.lock().unwrap()).desired_width(200.0));
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        } else if ui.add_enabled(paths.len() >= 2, egui::Button::new("Merge")).clicked() {
            go = true;
        }
    });
    if go {
        app.merge.running = true;
        *app.merge.progress.lock().unwrap() = 0.0;
        let res = app.merge.result.clone();
        let prog = app.merge.progress.clone();
        let (align, deghost, projection, auto_crop, fill_edges, boundary_warp) =
            (app.merge.align, app.merge.deghost, app.merge.projection, app.merge.auto_crop, app.merge.fill_edges, app.merge.boundary_warp);
        std::thread::spawn(move || {
            let pr = |p: f32| *prog.lock().unwrap() = p;
            let r = (|| -> anyhow::Result<PathBuf> {
                let (img, first, suffix) = match kind {
                    MergeKind::Hdr => {
                        let (i, f) = merge::merge_hdr(&paths, &merge::HdrOptions { align, deghost }, &pr)?;
                        (i, f, "HDR")
                    }
                    MergeKind::Pano => {
                        let (i, f) = merge::merge_panorama(&paths, &merge::PanoOptions { projection, auto_crop, fill_edges, boundary_warp }, &pr)?;
                        (i, f, "Pano")
                    }
                };
                let out = merge::output_path(&first, suffix);
                merge::write_linear_dng(&out, &img, &first)?;
                Ok(out)
            })();
            *res.lock().unwrap() = Some(r.map_err(|e| e.to_string()));
        });
    }
    if !open {
        app.merge.open = None;
    }
}

/// Grid of the current image rendered with each profile and creative look.
#[derive(Default)]
pub struct ProfileBrowser {
    pub open: bool,
    pub doc: u64,
    /// (label, kind, id, rendered thumbnail)
    pub items: Arc<Mutex<Vec<(String, u8, String, Option<egui::ColorImage>)>>>,
    pub textures: Vec<Option<egui::TextureHandle>>,
}

impl ProfileBrowser {
    pub fn open(&mut self, engine: &Arc<Engine>, s: &DevelopSettings, doc: u64) {
        self.open = true;
        if self.doc == doc && !self.items.lock().unwrap().is_empty() {
            return;
        }
        self.doc = doc;
        self.textures.clear();
        let mut list: Vec<(String, u8, String, Option<egui::ColorImage>)> = vec![];
        for p in darkslide_core::profiles::PROFILES {
            list.push((p.name.to_string(), 0, p.name.to_string(), None));
        }
        for l in darkslide_core::profiles::BUILTIN_LOOKS {
            list.push((l.to_string(), 1, format!("builtin:{l}"), None));
        }
        for (name, path) in darkslide_core::profiles::list_user_looks() {
            list.push((name, 1, path, None));
        }
        *self.items.lock().unwrap() = list.clone();
        let items = self.items.clone();
        let engine = engine.clone();
        let base = s.clone();
        std::thread::spawn(move || {
            for (i, (name, kind, id, _)) in list.into_iter().enumerate() {
                let mut s = base.clone();
                if kind == 0 {
                    s.profile = id.clone();
                    s.treatment = if darkslide_core::profiles::find(&id).monochrome {
                        darkslide_core::params::Treatment::BlackAndWhite
                    } else {
                        darkslide_core::params::Treatment::Color
                    };
                    s.look = None;
                } else {
                    s.look = Some(darkslide_core::params::Look { name: name.clone(), source: id.clone(), amount: 100.0 });
                }
                s.masks.clear();
                let r = engine.render_fit(&s, 180, 140, darkslide_core::demosaic::Quality::Draft, false);
                let px = darkslide_core::pipeline::to_srgb8(&r.image, Some(&r.alpha), [40, 40, 40]);
                let ci = egui::ColorImage::from_rgba_unmultiplied([r.image.w, r.image.h], &px);
                if let Some(it) = items.lock().unwrap().get_mut(i) {
                    it.3 = Some(ci);
                }
            }
        });
    }
}

fn profile_browser(app: &mut App, ctx: &egui::Context) {
    if !app.profiles.open {
        return;
    }
    let mut open = true;
    let mut chosen: Option<(u8, String, String)> = None;
    egui::Window::new("Profiles & looks").open(&mut open).default_size([640.0, 520.0]).show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let items = app.profiles.items.lock().unwrap();
            if app.profiles.textures.len() != items.len() {
                app.profiles.textures.resize(items.len(), None);
            }
            let mut last_kind = 255u8;
            let mut i = 0;
            while i < items.len() {
                let kind = items[i].1;
                if kind != last_kind {
                    ui.heading(if kind == 0 { "Profiles" } else { "Creative looks" });
                    last_kind = kind;
                }
                ui.horizontal_wrapped(|ui| {
                    while i < items.len() && items[i].1 == kind {
                        let (label, _, id, img) = &items[i];
                        if app.profiles.textures[i].is_none() {
                            if let Some(ci) = img {
                                app.profiles.textures[i] = Some(ctx.load_texture(format!("prof{i}"), ci.clone(), egui::TextureOptions::LINEAR));
                            }
                        }
                        ui.vertical(|ui| {
                            ui.set_width(186.0);
                            let resp = match &app.profiles.textures[i] {
                                Some(t) => ui.add(egui::Image::new(t).max_size(egui::vec2(180.0, 140.0)).sense(egui::Sense::click())),
                                None => ui.add_sized([180.0, 120.0], egui::Spinner::new()),
                            };
                            if resp.clicked() {
                                chosen = Some((kind, id.clone(), label.clone()));
                            }
                            ui.label(RichText::new(label).small());
                        });
                        i += 1;
                    }
                });
            }
            if items.iter().any(|it| it.3.is_none()) {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        });
    });
    if let Some((kind, id, label)) = chosen {
        if let Some(d) = app.doc_mut() {
            if kind == 0 {
                d.settings.profile = id.clone();
                d.settings.treatment = if darkslide_core::profiles::find(&id).monochrome {
                    darkslide_core::params::Treatment::BlackAndWhite
                } else {
                    darkslide_core::params::Treatment::Color
                };
            } else {
                let amount = d.settings.look.as_ref().map(|l| l.amount).unwrap_or(100.0);
                d.settings.look = Some(darkslide_core::params::Look { name: label, source: id, amount });
            }
        }
    }
    app.profiles.open = open;
}

fn reset_confirm(app: &mut App, ctx: &egui::Context) {
    if !app.confirm_reset {
        return;
    }
    let mut confirmed = false;
    let mut cancel = false;
    let resp = egui::Modal::new(egui::Id::new("confirm_reset")).show(ctx, |ui| {
        ui.set_max_width(380.0);
        ui.heading("Reset all settings?");
        ui.add_space(6.0);
        ui.label("This puts every setting of this image back to its defaults:");
        ui.label(
            RichText::new("tone, color, curves, detail, optics, geometry, effects, profile, white balance, crop, masks, spot removal and red eye.").weak(),
        );
        ui.add_space(4.0);
        ui.label("You can undo it with Ctrl+Z.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let reset = egui::Button::new(RichText::new("Reset all").strong().color(egui::Color32::WHITE)).fill(egui::Color32::from_rgb(170, 45, 45));
            if ui.add(reset).clicked() {
                confirmed = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            confirmed = true;
        }
    });
    if confirmed {
        app.reset_all();
        app.confirm_reset = false;
    } else if cancel || resp.should_close() {
        app.confirm_reset = false;
    }
}

pub fn show_dialogs(app: &mut App, ctx: &egui::Context) {
    reset_confirm(app, ctx);
    profile_browser(app, ctx);
    merge_window(app, ctx);
    export_window(app, ctx);
    copy_window(app, ctx);
    prefs_window(app, ctx);
    if app.show_shortcuts {
        let mut open = true;
        egui::Window::new("Keyboard shortcuts").open(&mut open).show(ctx, |ui| {
            egui::Grid::new("sc").striped(true).show(ui, |ui| {
                for (k, v) in SHORTCUTS {
                    ui.label(RichText::new(*k).monospace());
                    ui.label(*v);
                    ui.end_row();
                }
            });
        });
        app.show_shortcuts = open;
    }
    if app.show_about {
        let mut open = true;
        egui::Window::new("About Darkslide").open(&mut open).show(ctx, |ui| {
            ui.heading("Darkslide");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.label("A non-destructive raw developer for Linux, written in Rust.");
            ui.label("Raw decoding by rawler (dnglab). UI by egui.");
        });
        app.show_about = open;
    }
}
