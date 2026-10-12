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
    pub done_at: Option<std::time::Instant>,
}

fn format_label(f: Format) -> &'static str {
    match f {
        Format::Jpeg => "JPEG",
        Format::Tiff => "TIFF",
        Format::Png => "PNG",
        Format::Exr => "OpenEXR (HDR, linear)",
        Format::Psd => "Photoshop (PSD)",
        Format::Avif => "AVIF",
        Format::Jxl => "JPEG XL (lossless)",
        Format::Dng => "DNG (original raw + settings)",
    }
}

fn bit_depths(f: Format) -> &'static [u8] {
    match f {
        Format::Tiff => &[8, 16, 32],
        Format::Png | Format::Psd | Format::Jxl => &[8, 16],
        Format::Exr => &[32],
        Format::Dng => &[16],
        Format::Jpeg | Format::Avif => &[8],
    }
}

fn sharpen_label(m: SharpenFor) -> &'static str {
    match m {
        SharpenFor::None => "None",
        SharpenFor::Screen => "Screen",
        SharpenFor::Matte => "Matte paper",
        SharpenFor::Glossy => "Glossy paper",
    }
}

fn amount_label(a: SharpenAmount) -> &'static str {
    match a {
        SharpenAmount::Low => "Low",
        SharpenAmount::Standard => "Standard",
        SharpenAmount::High => "High",
    }
}

fn metadata_label(m: MetadataMode) -> &'static str {
    match m {
        MetadataMode::All => "All",
        MetadataMode::AllExceptLocation => "All except camera & location info",
        MetadataMode::CopyrightOnly => "Copyright only",
        MetadataMode::None => "None",
    }
}

/// A grid row whose controls grey out instead of disappearing, so the
/// dialog keeps its size and the Save button stays put.
fn row<R>(ui: &mut egui::Ui, enabled: bool, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) {
    ui.add_enabled(enabled, egui::Label::new(label));
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| add(ui));
    });
    ui.end_row();
}

fn export_window(app: &mut App, ctx: &egui::Context) {
    if !app.export.open {
        return;
    }
    // edit a draft; only Save commits it
    let mut ex = app.export.settings.take().unwrap_or_else(|| app.prefs.export.clone());
    let mut go = false;
    let mut cancel = false;
    let n_sel = app.docs.iter().filter(|d| d.selected).count();
    let example = app.doc().and_then(|d| d.engine.as_ref().map(|e| darkslide_core::export::expand_name(&ex.naming, &d.path, ex.start_seq, &e.source.meta)));
    let resp = egui::Modal::new(egui::Id::new("export")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.heading("Save Image");
        ui.add_space(6.0);
        let f = ex.format;
        egui::Grid::new("export").num_columns(2).spacing([12.0, 7.0]).min_col_width(110.0).show(ui, |ui| {
            row(ui, true, "Destination", |ui| {
                let label = ex.dest.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Same folder as original".into());
                ui.add(egui::Label::new(&label).truncate()).on_hover_text(&label);
                if ui.button(format!("{}  Choose…", ic::FOLDER_OPEN)).clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        ex.dest = Some(d);
                    }
                }
                if ex.dest.is_some() && ui.small_button(ic::X).on_hover_text("Back to the original's folder").clicked() {
                    ex.dest = None;
                }
            });
            row(ui, true, "File name", |ui| {
                ui.vertical(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut ex.naming).hint_text("{name}").desired_width(260.0))
                        .on_hover_text("Tokens: {name} {seq} {date} {camera}");
                    if let Some(n) = &example {
                        ui.label(RichText::new(format!("{n}.{}", f.ext())).small().weak());
                    }
                });
            });
            row(ui, true, "Format", |ui| {
                egui::ComboBox::from_id_salt("fmt").width(240.0).selected_text(format_label(f)).show_ui(ui, |ui| {
                    for v in [Format::Jpeg, Format::Tiff, Format::Png, Format::Exr, Format::Psd, Format::Avif, Format::Jxl, Format::Dng] {
                        ui.selectable_value(&mut ex.format, v, format_label(v));
                    }
                });
            });
            let f = ex.format;
            let (dng, exr) = (f == Format::Dng, f == Format::Exr);
            // a depth from the last format may not exist in this one
            let depths = bit_depths(f);
            if !depths.contains(&ex.bit_depth) {
                ex.bit_depth = depths.iter().copied().filter(|&b| b <= 16).max().unwrap_or(depths[0]);
            }
            row(ui, matches!(f, Format::Jpeg | Format::Avif), "Quality", |ui| ui.add(egui::Slider::new(&mut ex.jpeg_quality, 1..=100)));
            row(ui, depths.len() > 1, "Bit depth", |ui| {
                for &b in depths {
                    ui.selectable_value(&mut ex.bit_depth, b, format!("{b}-bit"));
                }
            });
            row(ui, f == Format::Tiff, "Compression", |ui| {
                ui.selectable_value(&mut ex.tiff_compression, TiffCompression::None, "None");
                ui.selectable_value(&mut ex.tiff_compression, TiffCompression::Lzw, "LZW");
                ui.selectable_value(&mut ex.tiff_compression, TiffCompression::Deflate, "ZIP");
            });
            row(ui, f == Format::Jpeg, "HDR", |ui| ui.checkbox(&mut ex.hdr_gainmap, "Include gain map (Ultra HDR)"));
            row(ui, f == Format::Jpeg, "Limit file size", |ui| ui.add(egui::DragValue::new(&mut ex.jpeg_limit_kb).range(0..=100000).suffix(" KB")).on_hover_text("0 = no limit"));
            row(ui, !dng, "Color space", |ui| {
                egui::ComboBox::from_id_salt("space").width(240.0).selected_text(ex.color_space.name()).show_ui(ui, |ui| {
                    for s in ColorSpace::ALL {
                        ui.selectable_value(&mut ex.color_space, s, s.name());
                    }
                });
            });
            row(ui, !dng, "Resize", |ui| {
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
                ui.add_enabled(ex.resize != Resize::None, egui::Checkbox::new(&mut ex.dont_enlarge, "Don't enlarge"));
            });
            row(ui, !dng, "Resolution", |ui| ui.add(egui::DragValue::new(&mut ex.ppi).range(1..=4800).suffix(" ppi")));
            row(ui, !dng && !exr, "Output sharpening", |ui| {
                egui::ComboBox::from_id_salt("osharp").selected_text(sharpen_label(ex.sharpen_for)).show_ui(ui, |ui| {
                    for m in [SharpenFor::None, SharpenFor::Screen, SharpenFor::Matte, SharpenFor::Glossy] {
                        ui.selectable_value(&mut ex.sharpen_for, m, sharpen_label(m));
                    }
                });
                ui.add_enabled_ui(ex.sharpen_for != SharpenFor::None, |ui| {
                    egui::ComboBox::from_id_salt("osharpamt").selected_text(amount_label(ex.sharpen_amount)).show_ui(ui, |ui| {
                        for a in [SharpenAmount::Low, SharpenAmount::Standard, SharpenAmount::High] {
                            ui.selectable_value(&mut ex.sharpen_amount, a, amount_label(a));
                        }
                    });
                });
            });
            row(ui, !dng, "Metadata", |ui| {
                egui::ComboBox::from_id_salt("meta").width(240.0).selected_text(metadata_label(ex.metadata)).show_ui(ui, |ui| {
                    for m in [MetadataMode::All, MetadataMode::AllExceptLocation, MetadataMode::CopyrightOnly, MetadataMode::None] {
                        ui.selectable_value(&mut ex.metadata, m, metadata_label(m));
                    }
                });
            });
            row(ui, true, "", |ui| ui.checkbox(&mut ex.write_sidecar, "Write settings sidecar next to output"));
        });
        if n_sel > 1 {
            ui.checkbox(&mut app.export.all_selected, format!("Save all {n_sel} selected images"));
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save = egui::Button::new(RichText::new("Save").strong().color(egui::Color32::WHITE)).fill(crate::widgets::ACCENT).min_size(egui::vec2(80.0, 0.0));
                if ui.add(save).on_hover_text("Enter").clicked() {
                    go = true;
                }
                if ui.button("Cancel").on_hover_text("Esc").clicked() {
                    cancel = true;
                }
            });
        });
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !ctx.any_popup_open() {
            go = true;
        }
    });
    if go {
        start_export(app, ex.clone());
        app.prefs.export = ex;
        app.prefs.save();
        app.export.open = false;
    } else if cancel || resp.should_close() {
        app.export.open = false;
    } else {
        app.export.settings = Some(ex);
    }
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
    let status = Arc::new(Mutex::new(ExportStatus { total: jobs.len(), finished: 0, progress: 0.0, done: false, message: None, done_at: None }));
    let shared = app.export_status.clone();
    shared.lock().unwrap().retain(|s| !s.done);
    // keep a handle in the app list through an Arc indirection
    let st2 = status.clone();
    std::thread::spawn(move || {
        let mut ok = 0;
        let mut errs = vec![];
        let mut dir: Option<PathBuf> = None;
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
                Ok(p) => {
                    ok += 1;
                    dir = p.parent().map(|d| d.to_path_buf());
                }
                Err(e) => errs.push(format!("{}: {e}", path.display())),
            }
            st2.lock().unwrap().finished = k + 1;
        }
        let mut s = st2.lock().unwrap();
        s.done = true;
        s.done_at = Some(std::time::Instant::now());
        let images = if ok == 1 { "1 image".to_string() } else { format!("{ok} images") };
        let to = dir.map(|d| format!(" to {}", d.display())).unwrap_or_default();
        s.message = Some(if errs.is_empty() { format!("Saved {images}{to}") } else { format!("Saved {images}, {} failed: {}", errs.len(), errs.join("; ")) });
    });
    // mirror into the shared list the bottom bar reads
    let shared2 = shared.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let s = status.lock().unwrap();
            let mut list = shared2.lock().unwrap();
            list.clear();
            list.push(ExportStatus { total: s.total, finished: s.finished, progress: s.progress, done: s.done, message: s.message.clone(), done_at: s.done_at });
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
        if self.on("Light") {
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
            "Light",
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
    let mut apply = false;
    let mut cancel = false;
    let resp = egui::Modal::new(egui::Id::new("copy_settings")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading(if app.copy.paste { "Paste settings" } else { "Copy settings" });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.small_button("Check all").clicked() {
                subset.flags.iter_mut().for_each(|f| f.1 = true);
            }
            if ui.small_button("Check none").clicked() {
                subset.flags.iter_mut().for_each(|f| f.1 = false);
            }
        });
        ui.add_space(4.0);
        egui::Grid::new("subset").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
            for (i, (name, v)) in subset.flags.iter_mut().enumerate() {
                ui.checkbox(v, *name);
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        ui.separator();
        let n = app.docs.iter().filter(|d| d.selected).count().max(1);
        let label = if !app.copy.paste {
            "Copy".to_string()
        } else if n == 1 {
            "Paste".to_string()
        } else {
            format!("Paste to {n} images")
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let ok = egui::Button::new(RichText::new(label).strong().color(egui::Color32::WHITE)).fill(crate::widgets::ACCENT).min_size(egui::vec2(80.0, 0.0));
            if ui.add(ok).clicked() {
                apply = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            apply = true;
        }
    });
    if apply {
        if app.copy.paste {
            app.paste_settings(Some(&subset));
        } else {
            // plain Ctrl+V pastes this choice too
            app.copy_settings(Some(subset.clone()));
            ctx.copy_text("Darkslide develop settings".into());
        }
        app.copy.open = false;
    } else if cancel || resp.should_close() {
        app.copy.open = false;
    }
    app.copy.subset = Some(subset);
}

fn prefs_window(app: &mut App, ctx: &egui::Context) {
    if !app.show_prefs {
        return;
    }
    let mut close = false;
    let heading = |ui: &mut egui::Ui, t: &str| {
        ui.add_space(8.0);
        ui.label(RichText::new(t).strong());
        ui.add_space(2.0);
    };
    let resp = egui::Modal::new(egui::Id::new("prefs")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.heading("Preferences");
        heading(ui, "General");
        ui.checkbox(&mut app.prefs.autosave_sidecars, "Save settings to XMP sidecars automatically");
        ui.checkbox(&mut app.edit.show_scope, "Show vectorscope under the histogram");
        heading(ui, "Appearance");
        egui::Grid::new("prefs_look").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Theme");
            let before = app.prefs.theme;
            let cur = crate::theme::NAMES[(app.prefs.theme as usize).min(crate::theme::NAMES.len() - 1)];
            egui::ComboBox::from_id_salt("theme").selected_text(cur).show_ui(ui, |ui| {
                for (i, n) in crate::theme::NAMES.iter().enumerate() {
                    ui.selectable_value(&mut app.prefs.theme, i as u8, *n);
                }
            });
            if app.prefs.theme != before {
                crate::theme::set(ctx, app.prefs.theme);
            }
            ui.end_row();
            ui.label("Interface scale");
            let mut pct = (app.prefs.ui_scale * 100.0).round();
            if ui.add(egui::Slider::new(&mut pct, 75.0..=200.0).step_by(5.0).max_decimals(0).suffix("%")).changed() {
                app.prefs.ui_scale = pct / 100.0;
                ctx.set_zoom_factor(app.prefs.ui_scale);
            }
            ui.end_row();
        });
        heading(ui, "Locations");
        let lens = darkslide_core::paths::data_dir().join("lensfun");
        let rows = [
            ("Presets", darkslide_core::paths::presets_dir()),
            ("Looks (.cube)", darkslide_core::profiles::user_looks_dir()),
            ("Lens database", lens),
            ("Camera defaults", darkslide_core::paths::config_dir().join("defaults")),
        ];
        egui::Grid::new("prefs_paths").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
            for (i, (name, path)) in rows.iter().enumerate() {
                ui.label(*name);
                let text = path.display().to_string();
                ui.add_sized([330.0, 18.0], egui::Label::new(RichText::new(&text).small().weak()).truncate()).on_hover_text(&text);
                ui.horizontal(|ui| {
                    if ui.small_button(ic::FOLDER_OPEN).on_hover_text("Open folder").clicked() {
                        let _ = std::fs::create_dir_all(path);
                        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
                    }
                    if i == 2 && ui.small_button(ic::DOWNLOAD_SIMPLE).on_hover_text("Download the latest lensfun lens profiles").clicked() {
                        app.download_lens_db();
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    });
    if close || resp.should_close() {
        app.prefs.show_vectorscope = app.edit.show_scope;
        app.prefs.save();
        app.show_prefs = false;
    }
}

/// Grouped like the menus; keep in step with App::shortcuts.
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Files",
        &[
            ("Ctrl+O / Ctrl+Shift+O", "Open files / folder"),
            ("Ctrl+S", "Save image"),
            ("← / →", "Previous / next image"),
            ("0–5 / 6–9", "Rating / colour label"),
            ("Tab", "Show or hide the filmstrip"),
            ("Ctrl+Q", "Quit"),
        ],
    ),
    (
        "Editing",
        &[
            ("Ctrl+Z / Ctrl+Shift+Z, Ctrl+Y", "Undo / redo"),
            ("Ctrl+C / Ctrl+V", "Copy / paste settings"),
            ("Ctrl+Alt+C / Ctrl+Alt+V", "Copy / paste chosen settings"),
            ("Ctrl+U / Ctrl+Shift+U", "Auto tone / auto white balance"),
            ("Ctrl+R", "Reset all settings"),
            ("Double-click slider", "Reset slider"),
            ("Shift (slider drag)", "Fine adjustment"),
            ("I", "White balance picker"),
        ],
    ),
    (
        "Tools",
        &[
            ("E", "Edit"),
            ("C / B / M / S", "Crop, Remove, Masking, Snapshots (again for Edit)"),
            ("Shift+P", "Presets"),
            ("Enter", "Apply crop"),
            ("Esc", "Cancel crop, deselect, or leave the tool"),
            ("Delete", "Delete selected spot, mask or red eye"),
            ("X / O (Crop)", "Swap orientation / cycle overlay"),
            ("[ / ]", "Brush or spot size"),
            ("Y", "Mask overlay"),
        ],
    ),
    (
        "View",
        &[
            ("Z, double-click", "Fit / 100%"),
            ("Ctrl+0 / Ctrl+Alt+0", "Fit / 100%"),
            ("Ctrl+= / Ctrl+−, Ctrl+wheel, wheel", "Zoom"),
            ("Space-drag, middle-drag", "Pan"),
            ("Q", "Cycle before/after views"),
            ("\\ or P", "Show before"),
            ("U / O", "Shadow / highlight clipping"),
            ("Ctrl+'", "Grid overlay"),
            ("F", "Full screen"),
        ],
    ),
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
    let n = paths.len();
    let mut cancel = false;
    egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
        ui.label(format!("{n} photo{} selected in the filmstrip (Ctrl-click to select).", if n == 1 { "" } else { "s" }));
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
        } else {
            ui.horizontal(|ui| {
                if ui.add_enabled(paths.len() >= 2, egui::Button::new("Merge")).on_disabled_hover_text("Select at least two photos").clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
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
    if !open || cancel || (!app.merge.running && ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
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
    let (cur_profile, cur_look) = app.doc().map(|d| (d.settings.profile.clone(), d.settings.look.as_ref().map(|l| l.source.clone()))).unwrap_or_default();
    // fit the screen: the tile grid wraps inside this width instead of growing the window
    let screen = ctx.content_rect();
    let w = (screen.width() - 120.0).clamp(400.0, 4.0 * 192.0 + 24.0);
    egui::Window::new("Profiles & looks")
        .open(&mut open)
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .fixed_size([w, (screen.height() - 140.0).max(300.0)])
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let items = app.profiles.items.lock().unwrap();
                if app.profiles.textures.len() != items.len() {
                    app.profiles.textures.resize(items.len(), None);
                }
                let mut last_kind = 255u8;
                let mut i = 0;
                while i < items.len() {
                    let kind = items[i].1;
                    if kind != last_kind {
                        ui.add_space(4.0);
                        ui.label(RichText::new(if kind == 0 { "Profiles" } else { "Creative looks" }).strong());
                        last_kind = kind;
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.set_max_width(w - 16.0);
                        while i < items.len() && items[i].1 == kind {
                            let (label, _, id, img) = &items[i];
                            if app.profiles.textures[i].is_none() {
                                if let Some(ci) = img {
                                    app.profiles.textures[i] = Some(ctx.load_texture(format!("prof{i}"), ci.clone(), egui::TextureOptions::LINEAR));
                                }
                            }
                            let current = if kind == 0 { *id == cur_profile } else { cur_look.as_deref() == Some(id.as_str()) };
                            ui.vertical(|ui| {
                                ui.set_width(180.0);
                                let resp = match &app.profiles.textures[i] {
                                    Some(t) => ui.add(egui::Image::new(t).fit_to_exact_size(egui::vec2(180.0, 120.0)).sense(egui::Sense::click())),
                                    None => ui.add_sized([180.0, 120.0], egui::Spinner::new()),
                                };
                                if current {
                                    ui.painter().rect_stroke(resp.rect.expand(2.0), 3.0, egui::Stroke::new(2.0, crate::widgets::ACCENT), egui::StrokeKind::Outside);
                                }
                                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    chosen = Some((kind, id.clone(), label.clone()));
                                }
                                let text = if current { RichText::new(label).small().color(crate::widgets::ACCENT) } else { RichText::new(label).small() };
                                ui.label(text);
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
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 120.0).show(ui, |ui| {
                    for (group, rows) in SHORTCUTS {
                        ui.label(RichText::new(*group).strong());
                        egui::Grid::new(("sc", *group)).striped(true).min_col_width(230.0).show(ui, |ui| {
                            for (k, v) in rows.iter() {
                                ui.label(RichText::new(*k).monospace());
                                ui.label(*v);
                                ui.end_row();
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
            });
        app.show_shortcuts = open;
    }
    if app.show_about {
        let resp = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.vertical_centered(|ui| {
                ui.heading("Darkslide");
                ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).weak());
                ui.add_space(8.0);
                ui.label("A non-destructive raw photo editor for Linux, written in Rust.");
                ui.add_space(6.0);
                ui.label(RichText::new("Raw decoding by rawler (dnglab) · lens profiles by lensfun · UI by egui").small().weak());
                ui.label(RichText::new("GPL-3.0-or-later").small().weak());
                ui.hyperlink_to("github.com/Syha-01/darkslide", env!("CARGO_PKG_REPOSITORY"));
                ui.add_space(8.0);
                if ui.button("Close").clicked() {
                    ui.close();
                }
            });
        });
        if resp.should_close() {
            app.show_about = false;
        }
    }
}
