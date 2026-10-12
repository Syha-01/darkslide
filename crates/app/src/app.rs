//! Application state, layout, shortcuts and render scheduling.

use egui_phosphor::regular as ic;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use darkslide_core::analysis::Histogram;
use darkslide_core::params::{DevelopSettings, WbMode};
use darkslide_core::pipeline::geometry::Region;
use eframe::egui::{self, Color32, Rect, RichText, Stroke, Vec2, pos2, vec2};

use crate::dialogs::{CopyDialog, ExportDialog, ExportStatus, Prefs};
use crate::doc::{ColorLabel, Doc, Loaded, Loader, scan_folder};
use crate::edit_panel::{self, Action, EditUi, PanelCtx, Pick};
use crate::tool_panels::{self, Tool, ToolUi};
use crate::worker::{Job, Kind, Output, Worker};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    Fit,
    /// Physical pixels per full-resolution pixel.
    Scale(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BeforeAfter {
    #[default]
    Off,
    LeftRight,
    TopBottom,
    SplitLeftRight,
    SplitTopBottom,
}

impl BeforeAfter {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::LeftRight,
            Self::LeftRight => Self::TopBottom,
            Self::TopBottom => Self::SplitLeftRight,
            Self::SplitLeftRight => Self::SplitTopBottom,
            Self::SplitTopBottom => Self::Off,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Single view",
            Self::LeftRight => "Before/After left-right",
            Self::TopBottom => "Before/After top-bottom",
            Self::SplitLeftRight => "Split left-right",
            Self::SplitTopBottom => "Split top-bottom",
        }
    }
}

pub struct Tex {
    pub tex: egui::TextureHandle,
    pub region: Region,
    pub ignore_crop: bool,
    pub doc: u64,
    pub w: usize,
    pub h: usize,
}

#[derive(Clone, PartialEq)]
pub struct JobKey {
    pub doc: u64,
    pub settings: DevelopSettings,
    pub region: Region,
    pub ignore_crop: bool,
    pub clip: (bool, bool),
    pub interactive: bool,
    pub mask: Option<usize>,
    pub alt: Option<crate::widgets::AltPreview>,
    pub point_range: Option<usize>,
}

pub struct App {
    pub docs: Vec<Doc>,
    pub current: Option<usize>,
    pub next_id: u64,
    pub worker: Worker,
    pub loader: Loader,
    pub fit_tex: Option<Tex>,
    pub detail_tex: Option<Tex>,
    pub before_tex: Option<Tex>,
    pub mask_tex: Option<Tex>,
    pub fit_rgba: Option<(Vec<u8>, usize, usize)>,
    /// Edited thumbnail waiting to be written to the disk cache.
    thumb_save: Option<(std::time::Instant, PathBuf, image::RgbImage)>,
    pub hist: Option<Histogram>,
    pub scope: Option<Vec<u32>>,
    pub render_ms: u128,
    pub zoom: Zoom,
    /// View centre in crop-frame normalised coords.
    pub center: [f32; 2],
    pub ba: BeforeAfter,
    pub show_before: bool,
    pub clip_s: bool,
    pub clip_h: bool,
    pub show_filmstrip: bool,
    pub samplers: Vec<[f32; 2]>,
    pub edit: EditUi,
    pub tools: ToolUi,
    pub pick: Pick,
    pub last_fit: Option<JobKey>,
    pub last_detail: Option<JobKey>,
    pub last_before: Option<JobKey>,
    pub export: ExportDialog,
    pub export_status: Arc<Mutex<Vec<ExportStatus>>>,
    pub copy: CopyDialog,
    pub clipboard: Option<DevelopSettings>,
    pub prefs: Prefs,
    pub show_prefs: bool,
    pub show_about: bool,
    pub show_shortcuts: bool,
    pub status: String,
    pub open_path: String,
    pub drag: crate::viewer::DragState,
    pub view_rect: egui::Rect,
    pub fullscreen: bool,
    /// Finished detections: doc, mask, kind, raster, and the message to show.
    pub ai_jobs: Arc<Mutex<Vec<(u64, usize, darkslide_core::params::AiMaskKind, String, String)>>>,
    pub filter_rating: i32,
    pub filter_label: ColorLabel,
    pub pending_actions: Vec<Action>,
    pub busy: Arc<Mutex<Option<String>>>,
    pub merge: crate::dialogs::MergeDialog,
    pub grid: bool,
    pub profiles: crate::dialogs::ProfileBrowser,
    /// Finished distraction removals: doc, spot, and what to say when nothing was found.
    pub distract_jobs: Arc<Mutex<Vec<(u64, Option<darkslide_core::params::RetouchSpot>, &'static str)>>>,
    /// Finished lens database download: lens count or error.
    pub lens_dl: Arc<Mutex<Option<Result<usize, String>>>>,
    /// "Reset all settings?" confirmation is showing.
    pub confirm_reset: bool,
    /// The crop when the crop tool was entered, for Esc to restore.
    pub crop_on_enter: Option<darkslide_core::params::Crop>,
    prev_tool: Tool,
    /// Tab pressed outside a text field (taken from egui before it moves focus).
    tab_pressed: bool,
    /// Subset chosen when the clipboard was filled; plain paste uses it.
    pub clipboard_subset: Option<crate::dialogs::Subset>,
    last_overlay: Color32,
    /// Status text and when it was first shown, so it can fade out.
    status_seen: (String, std::time::Instant),
}

impl App {
    pub fn new(cc: &eframe::CreationContext, paths: Vec<PathBuf>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let prefs = Prefs::load();
        crate::theme::apply(&ctx, prefs.theme);
        let mut app = Self {
            docs: vec![],
            current: None,
            next_id: 1,
            worker: Worker::new(ctx.clone()),
            loader: Loader::new(ctx.clone()),
            fit_tex: None,
            detail_tex: None,
            before_tex: None,
            mask_tex: None,
            fit_rgba: None,
            thumb_save: None,
            hist: None,
            scope: None,
            render_ms: 0,
            zoom: Zoom::Fit,
            center: [0.5, 0.5],
            ba: BeforeAfter::Off,
            show_before: false,
            clip_s: false,
            clip_h: false,
            show_filmstrip: true,
            samplers: vec![],
            edit: EditUi::default(),
            tools: ToolUi::default(),
            pick: Pick::None,
            last_fit: None,
            last_detail: None,
            last_before: None,
            export: ExportDialog::default(),
            export_status: Arc::new(Mutex::new(vec![])),
            copy: CopyDialog::default(),
            clipboard: None,
            show_prefs: false,
            show_about: false,
            show_shortcuts: false,
            prefs,
            status: String::new(),
            open_path: String::new(),
            drag: Default::default(),
            view_rect: egui::Rect::NOTHING,
            fullscreen: false,
            ai_jobs: Arc::new(Mutex::new(vec![])),
            filter_rating: 0,
            filter_label: ColorLabel::None,
            pending_actions: vec![],
            busy: Arc::new(Mutex::new(None)),
            merge: Default::default(),
            grid: false,
            profiles: Default::default(),
            distract_jobs: Arc::new(Mutex::new(vec![])),
            confirm_reset: false,
            lens_dl: Arc::new(Mutex::new(None)),
            crop_on_enter: None,
            prev_tool: Tool::Edit,
            tab_pressed: false,
            clipboard_subset: None,
            status_seen: (String::new(), std::time::Instant::now()),
            last_overlay: Color32::TRANSPARENT,
        };
        // Ctrl+=/− zoom the photo, not the whole interface
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        app.edit.show_scope = app.prefs.show_vectorscope;
        app.open_paths(paths);
        app
    }

    pub fn open_paths(&mut self, paths: Vec<PathBuf>) {
        let mut files = vec![];
        for p in paths {
            if p.is_dir() {
                files.extend(scan_folder(&p));
            } else if p.is_file() {
                files.push(p);
            }
        }
        if files.is_empty() {
            return;
        }
        let first_new = self.docs.len();
        for f in files {
            if let Some(i) = self.docs.iter().position(|d| d.path == f) {
                if self.current.is_none() {
                    self.select(i);
                }
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            self.docs.push(Doc::new(id, f));
        }
        if first_new < self.docs.len() {
            self.select(first_new);
        }
    }

    pub fn doc(&self) -> Option<&Doc> {
        self.current.and_then(|i| self.docs.get(i))
    }

    pub fn doc_mut(&mut self) -> Option<&mut Doc> {
        self.current.and_then(|i| self.docs.get_mut(i))
    }

    pub fn select(&mut self, i: usize) {
        if Some(i) == self.current {
            return;
        }
        // persist the image we're leaving
        let autosave = self.prefs.autosave_sidecars;
        if let Some(d) = self.doc_mut() {
            if d.dirty && autosave {
                d.save_sidecar();
            }
            // free memory of images we're not looking at
            d.engine = None;
        }
        self.current = Some(i);
        for d in self.docs.iter_mut() {
            d.selected = false;
        }
        let d = &mut self.docs[i];
        d.selected = true;
        if d.engine.is_none() && d.load_error.is_none() {
            self.loader.load(d.id, d.path.clone());
        }
        self.fit_tex = None;
        self.detail_tex = None;
        self.before_tex = None;
        self.mask_tex = None;
        self.fit_rgba = None;
        self.hist = None;
        self.last_fit = None;
        self.last_detail = None;
        self.last_before = None;
        self.zoom = Zoom::Fit;
        self.center = [0.5, 0.5];
        self.tools.selected_mask = None;
        self.tools.selected_spot = None;
        self.pick = Pick::None;
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(l) = self.loader.rx.try_recv() {
            match l {
                Loaded::Engine(id, r) => {
                    if let Some(d) = self.docs.iter_mut().find(|d| d.id == id) {
                        match r {
                            Ok(e) => {
                                // a reload of the doc we're looking at keeps current edits
                                if d.dirty {
                                    d.engine = Some(Arc::new(e));
                                } else {
                                    d.attach(e);
                                }
                            }
                            Err(e) => d.load_error = Some(e),
                        }
                    }
                }
                Loaded::Thumb(id, img) => {
                    // don't replace a thumbnail already taken from the edited render
                    if let (Some(d), Some(img)) = (self.docs.iter_mut().find(|d| d.id == id && d.thumb.is_none()), img) {
                        let ci = egui::ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw());
                        d.thumb = Some(ctx.load_texture(format!("thumb{id}"), ci, egui::TextureOptions::LINEAR));
                    }
                }
            }
        }
        while let Ok(out) = self.worker.rx.try_recv() {
            self.accept(ctx, out);
        }
        let dl = self.lens_dl.lock().unwrap().take();
        if let Some(r) = dl {
            match r {
                Ok(n) => {
                    for d in &self.docs {
                        if let Some(e) = &d.engine {
                            e.forget_lens_db();
                        }
                    }
                    self.last_fit = None;
                    self.last_detail = None;
                    self.status = format!("Lens database installed ({n} lenses)");
                }
                Err(e) => self.status = format!("Lens database download failed: {e}"),
            }
        }
        if let Some((at, ..)) = &self.thumb_save {
            let wait = std::time::Duration::from_secs(1);
            if at.elapsed() >= wait {
                self.flush_thumb();
            } else {
                ctx.request_repaint_after(wait);
            }
        }
        // finished distraction detections
        let found = std::mem::take(&mut *self.distract_jobs.lock().unwrap());
        for (id, spot, none) in found {
            match (self.docs.iter_mut().find(|d| d.id == id), spot) {
                (Some(d), Some(sp)) => {
                    d.settings.retouch.push(sp);
                    self.status = "Distraction removed".into();
                }
                (_, None) => self.status = none.into(),
                _ => {}
            }
        }
        // finished AI mask jobs
        let done = std::mem::take(&mut *self.ai_jobs.lock().unwrap());
        for (id, mi, kind, raster, msg) in done {
            if let Some(d) = self.docs.iter_mut().find(|d| d.id == id) {
                d.fill_ai_raster(mi, kind, &raster);
                self.status = msg;
            }
        }
    }

    fn accept(&mut self, ctx: &egui::Context, out: Output) {
        let Some(doc) = self.doc() else { return };
        if out.doc != doc.id {
            return;
        }
        let ci = egui::ColorImage::from_rgba_unmultiplied([out.w, out.h], &out.rgba);
        let opts = egui::TextureOptions { magnification: egui::TextureFilter::Nearest, minification: egui::TextureFilter::Linear, ..Default::default() };
        let slot = match out.kind {
            Kind::Fit => &mut self.fit_tex,
            Kind::Detail => &mut self.detail_tex,
            Kind::Before => &mut self.before_tex,
        };
        match slot {
            Some(t) if t.w == out.w && t.h == out.h => {
                t.tex.set(ci, opts);
                t.region = out.region;
                t.ignore_crop = out.ignore_crop;
                t.doc = out.doc;
            }
            _ => {
                *slot = Some(Tex {
                    tex: ctx.load_texture(format!("{:?}", out.kind), ci, opts),
                    region: out.region,
                    ignore_crop: out.ignore_crop,
                    doc: out.doc,
                    w: out.w,
                    h: out.h,
                });
            }
        }
        if out.kind == Kind::Fit {
            if let Some(h) = out.hist {
                self.hist = Some(h);
            }
            if out.scope.is_some() {
                self.scope = out.scope;
            }
            self.render_ms = out.ms;
            if let Some(m) = out.mask {
                let rgba: Vec<u8> = m
                    .data
                    .iter()
                    .flat_map(|v| {
                        let c = self.tools.mask_overlay_color;
                        let a = (v.clamp(0.0, 1.0) * c.a() as f32) as u8;
                        [c.r(), c.g(), c.b(), a]
                    })
                    .collect();
                let mi = egui::ColorImage::from_rgba_unmultiplied([m.w, m.h], &rgba);
                self.mask_tex = Some(Tex {
                    tex: ctx.load_texture("mask", mi, egui::TextureOptions::LINEAR),
                    region: out.region,
                    ignore_crop: out.ignore_crop,
                    doc: out.doc,
                    w: m.w,
                    h: m.h,
                });
            } else {
                self.mask_tex = None;
            }
            if !out.ignore_crop {
                self.refresh_thumb(ctx, &out.rgba, out.w, out.h);
            }
            self.fit_rgba = Some((out.rgba, out.w, out.h));
        }
    }

    pub fn download_lens_db(&mut self) {
        let busy = self.busy.clone();
        if busy.lock().unwrap().is_some() {
            return;
        }
        *busy.lock().unwrap() = Some("Downloading lens database…".into());
        let out = self.lens_dl.clone();
        std::thread::spawn(move || {
            let r = darkslide_core::lens::download_lensfun_db().map_err(|e| e.to_string());
            *out.lock().unwrap() = Some(r);
            *busy.lock().unwrap() = None;
        });
    }

    /// Shown in the canvas before any image is open: the whole canvas is a
    /// drop zone, with buttons to browse in the middle.
    pub fn start_view(&mut self, ui: &mut egui::Ui, view: Rect) {
        let zone = view.shrink(24.0);
        let resp = ui.interact(zone, ui.id().with("drop_zone"), egui::Sense::click());
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let p = ui.painter_at(view);
        let (fill, line) = if hovering {
            (crate::widgets::ACCENT.gamma_multiply(0.12), crate::widgets::ACCENT)
        } else if resp.hovered() {
            (Color32::from_white_alpha(5), Color32::from_gray(120))
        } else {
            (Color32::TRANSPARENT, crate::theme::pal().border_strong)
        };
        p.rect_filled(zone, 10.0, fill);
        let corners = [zone.left_top(), zone.right_top(), zone.right_bottom(), zone.left_bottom(), zone.left_top()];
        for seg in corners.windows(2) {
            p.extend(egui::Shape::dashed_line(&[seg[0], seg[1]], Stroke::new(1.5, line), 8.0, 6.0));
        }
        let c = zone.center();
        // tray with a down arrow
        let s = Stroke::new(2.5, line);
        let (top, y) = (c.y - 120.0, c.y - 70.0);
        p.line_segment([pos2(c.x, top), pos2(c.x, y)], s);
        p.line_segment([pos2(c.x - 12.0, y - 12.0), pos2(c.x, y)], s);
        p.line_segment([pos2(c.x + 12.0, y - 12.0), pos2(c.x, y)], s);
        p.line_segment([pos2(c.x - 28.0, y - 8.0), pos2(c.x - 28.0, y + 10.0)], s);
        p.line_segment([pos2(c.x - 28.0, y + 10.0), pos2(c.x + 28.0, y + 10.0)], s);
        p.line_segment([pos2(c.x + 28.0, y + 10.0), pos2(c.x + 28.0, y - 8.0)], s);
        let msg = if hovering { "Release to open" } else { "Drag & drop photos here" };
        p.text(pos2(c.x, c.y - 32.0), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(20.0), Color32::from_gray(220));
        p.text(pos2(c.x, c.y - 8.0), egui::Align2::CENTER_CENTER, "or", egui::FontId::proportional(13.0), Color32::from_gray(140));
        let bw = 200.0;
        let area = Rect::from_min_size(pos2(c.x - bw / 2.0, c.y + 8.0), vec2(bw, 110.0));
        let mut open: Option<Vec<PathBuf>> = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            if ui
                .add_sized([bw, 34.0], egui::Button::new(RichText::new("Add photo(s)…").size(15.0).color(Color32::WHITE)).fill(crate::widgets::ACCENT))
                .on_hover_text("Ctrl+O")
                .clicked()
            {
                open = crate::dialogs::pick_files();
            }
            ui.add_space(4.0);
            if ui.add_sized([bw, 28.0], egui::Button::new("Open folder…")).on_hover_text("Ctrl+Shift+O").clicked() {
                open = rfd::FileDialog::new().pick_folder().map(|d| vec![d]);
            }
        });
        p.text(
            pos2(c.x, area.bottom() + 16.0),
            egui::Align2::CENTER_CENTER,
            "Raw files from most cameras, DNG, JPEG, TIFF, PNG, HEIC and more",
            egui::FontId::proportional(12.0),
            Color32::from_gray(130),
        );
        if open.is_none() && resp.clicked() {
            open = crate::dialogs::pick_files();
        }
        if let Some(files) = open {
            self.open_paths(files);
        }
    }

    /// Filmstrip thumbnail from the edited render; cached once edits settle.
    fn refresh_thumb(&mut self, ctx: &egui::Context, rgba: &[u8], w: usize, h: usize) {
        let Some(src) = image::RgbaImage::from_raw(w as u32, h as u32, rgba.to_vec()) else { return };
        let t = image::DynamicImage::ImageRgba8(src).thumbnail(256, 256).to_rgb8();
        let Some(d) = self.doc_mut() else { return };
        let ci = egui::ColorImage::from_rgb([t.width() as usize, t.height() as usize], t.as_raw());
        match &mut d.thumb {
            Some(tex) => tex.set(ci, egui::TextureOptions::LINEAR),
            None => d.thumb = Some(ctx.load_texture(format!("thumb{}", d.id), ci, egui::TextureOptions::LINEAR)),
        }
        let path = d.path.clone();
        if let Some((_, p, _)) = &self.thumb_save
            && *p != path
        {
            self.flush_thumb();
        }
        self.thumb_save = Some((std::time::Instant::now(), path, t));
    }

    fn flush_thumb(&mut self) {
        if let Some((_, p, t)) = self.thumb_save.take() {
            std::thread::spawn(move || darkslide_core::thumbcache::put(&p, &t));
        }
    }

    /// Full-res crop size for the current tool view.
    pub fn crop_dims(&self) -> Option<(f64, f64, bool)> {
        let d = self.doc()?;
        let e = d.engine.as_ref()?;
        let ignore = self.tools.tool == Tool::Crop;
        let (fw, fh) = e.full_size();
        let mut g = e.geometry(&d.settings, fw, fh, ignore);
        if ignore {
            g = g.without_crop();
        }
        let (w, h) = g.crop_size();
        Some((w, h, ignore))
    }

    fn schedule(&mut self, ctx: &egui::Context, interacting: bool, alt: Option<crate::widgets::AltPreview>) {
        let Some(d) = self.doc() else { return };
        let Some(engine) = d.engine.clone() else { return };
        // a hovered preset previews without touching the document
        let shown = match (&self.tools.hover_preview, self.tools.tool) {
            (Some(p), Tool::Presets) => p.clone(),
            _ => d.settings.clone(),
        };
        let (doc_id, settings, opened) = (d.id, shown, d.opened.clone());
        let Some((cw, ch, ignore)) = self.crop_dims() else { return };
        let ppp = ctx.pixels_per_point();
        let view = self.view_rect;
        if !view.is_positive() {
            return;
        }
        let (vw, vh) = match self.ba {
            BeforeAfter::LeftRight => (view.width() / 2.0, view.height()),
            BeforeAfter::TopBottom => (view.width(), view.height() / 2.0),
            _ => (view.width(), view.height()),
        };
        let f = ((vw * ppp) as f64 / cw).min((vh * ppp) as f64 / ch).min(1.0);
        let fit_region = Region::full(((cw * f).round() as usize).max(1), ((ch * f).round() as usize).max(1));
        let mask = if self.tools.tool == Tool::Masking && self.tools.show_mask_overlay { self.tools.selected_mask } else { None };
        let point_range = if self.edit.visualize_point && self.tools.tool == Tool::Edit { Some(self.edit.point_sel) } else { None };
        let key = JobKey {
            doc: doc_id,
            settings: settings.clone(),
            region: fit_region,
            ignore_crop: ignore,
            clip: (self.clip_s, self.clip_h),
            interactive: interacting,
            mask,
            alt,
            point_range,
        };
        if self.last_fit.as_ref() != Some(&key) {
            self.worker.submit(Job {
                doc: doc_id,
                kind: Kind::Fit,
                engine: engine.clone(),
                settings: settings.clone(),
                region: fit_region,
                ignore_crop: ignore,
                clip_shadows: self.clip_s,
                clip_highlights: self.clip_h,
                interactive: interacting,
                mask_overlay: mask,
                want_scope: self.edit.show_scope,
                alt_preview: alt,
                point_range,
            });
            self.last_fit = Some(key);
        }
        // before
        let wants_before = self.ba != BeforeAfter::Off || self.show_before;
        if wants_before {
            let key = JobKey {
                doc: doc_id,
                settings: opened.clone(),
                region: fit_region,
                ignore_crop: false,
                clip: (false, false),
                interactive: false,
                mask: None,
                alt: None,
                point_range: None,
            };
            if self.last_before.as_ref() != Some(&key) {
                let mut before = opened.clone();
                // the before view shares the current crop so the frames line up
                before.crop = settings.crop;
                before.transform = settings.transform.clone();
                self.worker.submit(Job {
                    doc: doc_id,
                    kind: Kind::Before,
                    engine: engine.clone(),
                    settings: before,
                    region: fit_region,
                    ignore_crop: false,
                    clip_shadows: false,
                    clip_highlights: false,
                    interactive: false,
                    mask_overlay: None,
                    want_scope: false,
                    alt_preview: None,
                    point_range: None,
                });
                self.last_before = Some(key);
            }
        }
        // detail when zoomed beyond the fit texture
        if let Zoom::Scale(z) = self.zoom {
            if self.ba == BeforeAfter::Off && (z as f64) > f * 1.05 {
                let disp_w = cw * z as f64 / ppp as f64;
                let disp_h = ch * z as f64 / ppp as f64;
                let img_rect = egui::Rect::from_min_size(
                    egui::pos2(view.center().x - self.center[0] * disp_w as f32, view.center().y - self.center[1] * disp_h as f32),
                    egui::vec2(disp_w as f32, disp_h as f32),
                );
                let vis = img_rect.intersect(view);
                if vis.is_positive() {
                    let x0 = ((vis.min.x - img_rect.min.x) / img_rect.width()) as f64;
                    let y0 = ((vis.min.y - img_rect.min.y) / img_rect.height()) as f64;
                    let x1 = ((vis.max.x - img_rect.min.x) / img_rect.width()) as f64;
                    let y1 = ((vis.max.y - img_rect.min.y) / img_rect.height()) as f64;
                    let region = Region { x0, y0, x1, y1, w: ((vis.width() * ppp).round() as usize).max(1), h: ((vis.height() * ppp).round() as usize).max(1) };
                    let key = JobKey {
                        doc: doc_id,
                        settings: settings.clone(),
                        region,
                        ignore_crop: ignore,
                        clip: (self.clip_s, self.clip_h),
                        interactive: false,
                        mask: None,
                        alt: None,
                        point_range: None,
                    };
                    // while dragging, keep the old detail tile and skip full-res work
                    if !interacting && self.last_detail.as_ref() != Some(&key) {
                        self.worker.submit(Job {
                            doc: doc_id,
                            kind: Kind::Detail,
                            engine,
                            settings: settings.clone(),
                            region,
                            ignore_crop: ignore,
                            clip_shadows: self.clip_s,
                            clip_highlights: self.clip_h,
                            interactive: false,
                            mask_overlay: None,
                            want_scope: false,
                            alt_preview: None,
                            point_range: None,
                        });
                        self.last_detail = Some(key);
                    }
                }
            }
        }
    }

    /// Reset every setting of the current image to the defaults (the saved
    /// camera defaults when there are any). Undoable.
    pub fn reset_all(&mut self) {
        let Some(d) = self.doc_mut() else { return };
        let raw = d.is_raw();
        let def = d.engine.as_ref().and_then(|e| crate::doc::load_camera_default(e)).unwrap_or_else(|| DevelopSettings::defaults_for(raw));
        d.settings = def;
        self.tools.selected_mask = None;
        self.tools.selected_spot = None;
        self.status = "All settings reset (Ctrl+Z to undo)".into();
    }

    /// Shift + double-click on a Basic slider label sets just that slider.
    fn auto_one(&mut self, label: &str) {
        let Some(d) = self.doc_mut() else { return };
        let Some(e) = d.engine.clone() else { return };
        let v = e.auto_tone(&d.settings);
        let s = &mut d.settings;
        match label {
            "Exposure" => s.exposure = v[0],
            "Contrast" => s.contrast = v[1],
            "Highlights" => s.highlights = v[2],
            "Shadows" => s.shadows = v[3],
            "Whites" => s.whites = v[4],
            "Blacks" => s.blacks = v[5],
            "Vibrance" => s.vibrance = if e.is_raw() { 15.0 } else { 10.0 },
            "Temperature" | "Tint" => s.wb_mode = WbMode::Auto,
            _ => {}
        }
    }

    fn run_actions(&mut self) {
        let actions = std::mem::take(&mut self.pending_actions);
        for a in actions {
            let Some(d) = self.doc_mut() else { continue };
            let Some(e) = d.engine.clone() else { continue };
            match a {
                Action::AutoTone => {
                    let v = e.auto_tone(&d.settings);
                    let s = &mut d.settings;
                    s.exposure = v[0];
                    s.contrast = v[1];
                    s.highlights = v[2];
                    s.shadows = v[3];
                    s.whites = v[4];
                    s.blacks = v[5];
                    s.vibrance = s.vibrance.max(if e.is_raw() { 10.0 } else { 0.0 });
                }
                Action::AutoWb => {
                    d.settings.wb_mode = WbMode::Auto;
                }
                Action::AutoGrayMix => {
                    let r = e.render_fit(&d.settings, 400, 400, darkslide_core::demosaic::Quality::Draft, false);
                    let ok = darkslide_core::color::Oklab::new();
                    d.settings.gray_mix = darkslide_core::pipeline::colorops::auto_gray_mix(&r.image, &ok);
                    d.settings.auto_gray_mix = false;
                }
                Action::DetectDust => {
                    let base = e.base(darkslide_core::demosaic::Quality::Draft);
                    // a second run only adds spots that aren't covered yet
                    let have: Vec<([f32; 2], f32)> = d.settings.retouch.iter().filter_map(|s| s.points.first().map(|p| (*p, s.radius))).collect();
                    let spots: Vec<_> = darkslide_core::retouch::detect_dust(&base, 40)
                        .into_iter()
                        .filter(|n| n.points.first().is_some_and(|p| have.iter().all(|(q, r)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt() > r.max(n.radius) * 2.0)))
                        .collect();
                    let n = spots.len();
                    d.settings.retouch.extend(spots);
                    self.status = match n {
                        0 if have.is_empty() => "No dust spots found".into(),
                        0 => "No new dust spots found".into(),
                        1 => "Found 1 dust spot".into(),
                        _ => format!("Found {n} dust spots"),
                    };
                }
                Action::ResetAll => self.confirm_reset = true,
                Action::DownloadLensDb => self.download_lens_db(),
                Action::BrowseProfiles => {
                    let (st, id) = (d.settings.clone(), d.id);
                    self.profiles.open(&e, &st, id);
                }
                Action::RemovePeople | Action::RemoveWires => {
                    // detection + fill run in the background; the spot lands when ready
                    let people = matches!(a, Action::RemovePeople);
                    let s = d.settings.clone();
                    let id = d.id;
                    let busy = self.busy.clone();
                    let out = self.distract_jobs.clone();
                    *busy.lock().unwrap() = Some(if people { "Finding people…".into() } else { "Finding wires…".into() });
                    std::thread::spawn(move || {
                        let spot = if people { darkslide_core::distract::remove_people(&e, &s) } else { darkslide_core::distract::remove_wires(&e, &s) };
                        let none = if people { "No people found (this needs the AI model installed)" } else { "No wires or cables found" };
                        out.lock().unwrap().push((id, spot, none));
                        *busy.lock().unwrap() = None;
                    });
                }
                Action::UprightAnalyze(mode) => {
                    let mut s = d.settings.clone();
                    s.transform.upright_matrix = None;
                    match darkslide_core::upright::analyze(&e, &s, mode) {
                        Some(m) => {
                            d.settings.transform.upright_matrix = Some(m);
                            d.settings.transform.upright = mode;
                            self.status = "Upright applied".into();
                        }
                        None => {
                            // nothing applied, so don't leave the mode looking selected
                            d.settings.transform.upright = darkslide_core::params::Upright::Off;
                            d.settings.transform.upright_matrix = None;
                            self.status = "Upright: not enough straight lines found. Try Guided and draw the lines.".into();
                        }
                    }
                }
                Action::ImportLook => {
                    if let Some(files) = rfd::FileDialog::new().add_filter("3D LUT", &["cube"]).pick_files() {
                        let dir = darkslide_core::profiles::user_looks_dir();
                        let _ = std::fs::create_dir_all(&dir);
                        for f in files {
                            if let Some(n) = f.file_name() {
                                let _ = std::fs::copy(&f, dir.join(n));
                            }
                        }
                    }
                }
            }
        }
        // any AI mask without a raster (new, or added by a preset) gets detected
        if self.tools.request_ai.is_none() && self.busy.lock().unwrap().is_none() {
            if let Some(d) = self.doc() {
                'find: for (mi, m) in d.settings.masks.iter().enumerate() {
                    for c in &m.components {
                        if let darkslide_core::params::MaskShape::Ai { kind, raster: None, .. } = &c.shape {
                            self.tools.request_ai = Some((mi, *kind));
                            break 'find;
                        }
                    }
                }
            }
        }
        // AI mask detection requests
        if let Some((mi, kind)) = self.tools.request_ai.take() {
            if let Some(d) = self.doc() {
                if let Some(e) = d.engine.clone() {
                    let s = d.settings.clone();
                    let hint = s.masks.get(mi).and_then(|m| {
                        m.components.iter().find_map(|c| match &c.shape {
                            darkslide_core::params::MaskShape::Ai { raster: None, hint, .. } => *hint,
                            _ => None,
                        })
                    });
                    let id = d.id;
                    let jobs = self.ai_jobs.clone();
                    let busy = self.busy.clone();
                    *busy.lock().unwrap() = Some(format!("Detecting {}…", kind.label()));
                    std::thread::spawn(move || {
                        let (plane, model) = darkslide_core::ai::detect_with_source(&e, &s, kind, hint);
                        let cover = plane.data.iter().sum::<f32>() / plane.data.len().max(1) as f32;
                        let name = kind.label();
                        // say plainly when nothing was found or no model was used
                        let msg = if cover < 0.003 {
                            format!("No {} found in this photo", name.to_lowercase())
                        } else if model {
                            format!("{name} detected")
                        } else {
                            format!("{name} estimated from colour and texture (no AI model installed)")
                        };
                        let raster = darkslide_core::pipeline::local::encode_raster(&plane);
                        jobs.lock().unwrap().push((id, mi, kind, raster, msg));
                        *busy.lock().unwrap() = None;
                    });
                }
            }
        }
    }

    pub fn copy_settings(&mut self, subset: Option<crate::dialogs::Subset>) {
        if let Some(d) = self.doc() {
            self.clipboard = Some(d.settings.clone());
            self.clipboard_subset = subset;
            self.status = "Settings copied".into();
        }
    }

    /// A dialog that owns the keyboard is open, so app shortcuts wait.
    pub fn dialog_open(&self) -> bool {
        self.export.open || self.copy.open || self.merge.open.is_some() || self.show_prefs || self.show_about || self.confirm_reset
    }

    pub fn paste_settings(&mut self, subset: Option<&crate::dialogs::Subset>) {
        let Some(src) = self.clipboard.clone() else { return };
        let targets: Vec<usize> = {
            let sel: Vec<usize> = self.docs.iter().enumerate().filter(|(_, d)| d.selected).map(|(i, _)| i).collect();
            if sel.is_empty() { self.current.into_iter().collect() } else { sel }
        };
        let full = self.clipboard_subset.clone().unwrap_or_else(crate::dialogs::Subset::all);
        let subset = subset.unwrap_or(&full);
        for i in targets {
            let d = &mut self.docs[i];
            subset.apply(&mut d.settings, &src);
            if d.engine.is_none() {
                // not loaded: write straight to the sidecar
                let mut s = darkslide_core::xmp::read_sidecar(&d.path, true).map(|(s, _)| s).unwrap_or_default();
                subset.apply(&mut s, &src);
                let _ = darkslide_core::xmp::write_sidecar(&d.path, &s);
            } else {
                d.dirty = true;
            }
        }
        self.status = "Settings pasted".into();
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let tab = std::mem::take(&mut self.tab_pressed);
        // only a focused text field takes the keys; a focused button doesn't
        if ctx.text_edit_focused() || self.dialog_open() {
            return;
        }
        use egui::Key;
        let (cmd, shift, alt) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift, i.modifiers.alt));
        let pressed = |k: Key| ctx.input_mut(|i| i.key_pressed(k));
        if cmd && pressed(Key::Z) {
            if shift {
                if let Some(d) = self.doc_mut() {
                    d.redo();
                }
            } else if let Some(d) = self.doc_mut() {
                d.undo();
            }
            return;
        }
        if cmd && pressed(Key::Y) {
            if let Some(d) = self.doc_mut() {
                d.redo();
            }
        }
        if cmd && pressed(Key::U) {
            if shift {
                self.pending_actions.push(Action::AutoWb);
            } else {
                self.pending_actions.push(Action::AutoTone);
            }
        }
        // Ctrl+Shift+S syncs settings to the selected photos, as in Lightroom
        if cmd && shift && pressed(Key::S) && self.doc().is_some() {
            self.copy_settings(None);
            self.copy.open_for_paste();
        } else if cmd && pressed(Key::S) && self.doc().is_some() {
            self.export.open = true;
        }
        if cmd && pressed(Key::C) && self.doc().is_some() {
            if alt {
                self.copy.open_for_copy();
            } else {
                self.copy_settings(None);
                // egui-winit only sends Ctrl+V on while the system clipboard holds text
                ctx.copy_text("Darkslide develop settings".into());
            }
        }
        if cmd && pressed(Key::V) && self.clipboard.is_some() {
            if alt {
                self.copy.open_for_paste();
            } else {
                self.paste_settings(None);
            }
        }
        if cmd && pressed(Key::O) {
            if shift {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.open_paths(vec![dir]);
                }
            } else if let Some(files) = crate::dialogs::pick_files() {
                self.open_paths(files);
            }
        }
        if cmd && pressed(Key::Q) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if cmd && pressed(Key::Quote) {
            self.grid = !self.grid;
        }
        if cmd && pressed(Key::R) && self.doc().is_some() {
            self.confirm_reset = true;
        }
        if cmd && pressed(Key::Num0) {
            self.zoom = if alt { Zoom::Scale(1.0) } else { Zoom::Fit };
        }
        if cmd && (pressed(Key::Plus) || pressed(Key::Equals)) {
            self.zoom_step(1);
        }
        if cmd && pressed(Key::Minus) {
            self.zoom_step(-1);
        }
        if cmd || alt {
            return;
        }
        let tool = self.tools.tool;
        if pressed(Key::Z) {
            self.zoom = if self.zoom == Zoom::Fit { Zoom::Scale(1.0) } else { Zoom::Fit };
        }
        if pressed(Key::U) {
            self.clip_s = !self.clip_s;
        }
        // in the crop tool O and X work the crop, as in Lightroom
        if pressed(Key::O) {
            if tool == Tool::Crop {
                self.tools.crop_overlay = self.tools.crop_overlay.next();
            } else {
                self.clip_h = !self.clip_h;
            }
        }
        if pressed(Key::X) && tool == Tool::Crop {
            self.swap_crop_orientation();
        }
        if pressed(Key::Q) {
            self.ba = self.ba.next();
        }
        if pressed(Key::P) {
            self.set_tool(if tool == Tool::Presets { Tool::Edit } else { Tool::Presets });
        }
        if pressed(Key::Backslash) {
            self.show_before = !self.show_before;
        }
        if pressed(Key::Questionmark) || (shift && pressed(Key::Slash)) || pressed(Key::F1) {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if pressed(Key::I) {
            self.pick = if self.pick == Pick::WhiteBalance { Pick::None } else { Pick::WhiteBalance };
        }
        if pressed(Key::E) {
            self.set_tool(Tool::Edit);
        }
        // tool keys toggle back to Edit, like C always did
        for (k, t) in [(Key::C, Tool::Crop), (Key::B, Tool::Remove), (Key::M, Tool::Masking)] {
            if pressed(k) {
                self.set_tool(if tool == t { Tool::Edit } else { t });
            }
        }
        if pressed(Key::S) && !shift {
            self.set_tool(if tool == Tool::Snapshots { Tool::Edit } else { Tool::Snapshots });
        }
        if pressed(Key::Y) {
            self.tools.show_mask_overlay = !self.tools.show_mask_overlay;
        }
        if pressed(Key::F) {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        if tab {
            self.show_filmstrip = !self.show_filmstrip;
        }
        if pressed(Key::Enter) && tool == Tool::Crop {
            self.commit_crop();
        }
        if pressed(Key::Escape) {
            self.escape();
        }
        if pressed(Key::ArrowRight) {
            self.step_image(1);
        }
        if pressed(Key::ArrowLeft) {
            self.step_image(-1);
        }
        // brackets size the brush of the tool in use (the selected spot too)
        for (k, f) in [(Key::OpenBracket, 0.85f32), (Key::CloseBracket, 1.18)] {
            if pressed(k) {
                match tool {
                    Tool::Masking => self.tools.brush.size = (self.tools.brush.size * f).clamp(0.001, 0.3),
                    Tool::Remove => {
                        self.tools.spot_size = (self.tools.spot_size * f).clamp(0.001, 0.2);
                        let (sel, r) = (self.tools.selected_spot, self.tools.spot_size);
                        if let Some(sp) = sel.and_then(|i| self.doc_mut().and_then(|d| d.settings.retouch.get_mut(i))) {
                            sp.radius = r;
                        }
                    }
                    _ => {}
                }
            }
        }
        if pressed(Key::Delete) || pressed(Key::Backspace) {
            self.delete_selected();
        }
        // ratings 0-5, labels 6-9
        let keys = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
        for (n, k) in keys.iter().enumerate() {
            if pressed(*k) {
                if let Some(d) = self.doc_mut() {
                    if n <= 5 {
                        d.rating = n as i32;
                        d.rejected = false;
                    } else {
                        let l = [ColorLabel::Red, ColorLabel::Yellow, ColorLabel::Green, ColorLabel::Blue][n - 6];
                        d.label = if d.label == l { ColorLabel::None } else { l };
                    }
                    d.dirty = true;
                    d.last_change = Some(std::time::Instant::now());
                }
            }
        }
    }

    pub fn set_tool(&mut self, t: Tool) {
        self.tools.tool = t;
        self.pick = Pick::None;
    }

    /// Keep the crop and go back to editing.
    pub fn commit_crop(&mut self) {
        self.tools.straighten = false;
        self.crop_on_enter = None;
        self.set_tool(Tool::Edit);
    }

    pub fn swap_crop_orientation(&mut self) {
        if let Some(d) = self.doc_mut()
            && let Some(e) = d.engine.clone()
        {
            tool_panels::swap_orientation(&mut d.settings, &e);
        }
    }

    /// Esc backs out one layer: picker, then the tool's selection or armed
    /// sub-tool, then the tool itself (the crop tool also undoes its changes).
    fn escape(&mut self) {
        if self.profiles.open {
            self.profiles.open = false;
            return;
        }
        if self.show_shortcuts {
            self.show_shortcuts = false;
            return;
        }
        if self.pick != Pick::None {
            self.pick = Pick::None;
            return;
        }
        let t = &mut self.tools;
        match t.tool {
            Tool::Crop if t.straighten => t.straighten = false,
            Tool::Crop => {
                if let Some(c) = self.crop_on_enter.take()
                    && let Some(d) = self.doc_mut()
                {
                    d.settings.crop = c;
                }
                self.set_tool(Tool::Edit);
            }
            Tool::Remove if t.selected_spot.is_some() => t.selected_spot = None,
            Tool::Masking if t.mask_tool != tool_panels::MaskTool::None => t.mask_tool = tool_panels::MaskTool::None,
            Tool::Masking if t.selected_mask.is_some() => {
                t.selected_mask = None;
                t.selected_comp = None;
            }
            Tool::RedEye if t.selected_red_eye.is_some() => t.selected_red_eye = None,
            Tool::Edit => {}
            _ => self.set_tool(Tool::Edit),
        }
    }

    fn delete_selected(&mut self) {
        let t = &mut self.tools;
        let (tool, spot, mask, eye) = (t.tool, t.selected_spot.take(), t.selected_mask, t.selected_red_eye.take());
        match tool {
            Tool::Masking => {
                t.selected_mask = None;
                t.selected_comp = None;
            }
            Tool::Remove | Tool::RedEye => {}
            _ => return,
        }
        let Some(d) = self.doc_mut() else { return };
        let s = &mut d.settings;
        match tool {
            Tool::Remove => {
                if let Some(i) = spot.filter(|&i| i < s.retouch.len()) {
                    s.retouch.remove(i);
                }
            }
            Tool::Masking => {
                if let Some(i) = mask.filter(|&i| i < s.masks.len()) {
                    s.masks.remove(i);
                }
            }
            _ => {
                if let Some(i) = eye.filter(|&i| i < s.red_eye.len()) {
                    s.red_eye.remove(i);
                }
            }
        }
    }

    pub fn zoom_step(&mut self, dir: i32) {
        const LEVELS: [f32; 12] = [0.0625, 0.125, 0.25, 0.333, 0.5, 0.667, 1.0, 2.0, 3.0, 4.0, 8.0, 16.0];
        let cur = match self.zoom {
            Zoom::Fit => self.fit_scale(),
            Zoom::Scale(z) => z,
        };
        let next = if dir > 0 { LEVELS.iter().copied().find(|&l| l > cur * 1.01) } else { LEVELS.iter().rev().copied().find(|&l| l < cur * 0.99) };
        if let Some(n) = next {
            self.zoom = Zoom::Scale(n);
        }
    }

    pub fn fit_scale(&self) -> f32 {
        let Some((cw, ch, _)) = self.crop_dims() else { return 1.0 };
        let v = self.view_rect;
        ((v.width() as f64 / cw).min(v.height() as f64 / ch) as f32).max(1e-4)
    }

    pub fn step_image(&mut self, dir: i32) {
        let visible = self.visible_indices();
        if visible.is_empty() {
            return;
        }
        let cur = self.current.and_then(|c| visible.iter().position(|&i| i == c)).unwrap_or(0) as i32;
        let n = (cur + dir).clamp(0, visible.len() as i32 - 1) as usize;
        self.select(visible[n]);
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        self.docs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.rating >= self.filter_rating && (self.filter_label == ColorLabel::None || d.label == self.filter_label))
            .map(|(i, _)| i)
            .collect()
    }

    fn autosave(&mut self) {
        let delay = std::time::Duration::from_millis(1500);
        if !self.prefs.autosave_sidecars {
            return;
        }
        for d in self.docs.iter_mut() {
            if d.dirty && d.last_change.map(|t| t.elapsed() > delay).unwrap_or(true) {
                d.save_sidecar();
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let has_doc = self.doc().is_some();
        let has_clip = has_doc && self.clipboard.is_some();
        let can_redo = self.doc().map(|d| !d.redo.is_empty()).unwrap_or(false);
        let can_undo = self.doc().map(|d| !d.undo.is_empty()).unwrap_or(false);
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if menu_item(ui, true, "Open files…", "Ctrl+O") {
                    if let Some(files) = crate::dialogs::pick_files() {
                        self.open_paths(files);
                    }
                    ui.close();
                }
                if menu_item(ui, true, "Open folder…", "Ctrl+Shift+O") {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.open_paths(vec![dir]);
                    }
                    ui.close();
                }
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut self.open_path).hint_text("path to file or folder").desired_width(220.0));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button("Open").clicked() || enter) && !self.open_path.is_empty() {
                        let p = PathBuf::from(shellexpand(&self.open_path));
                        self.open_paths(vec![p]);
                        ui.close();
                    }
                });
                ui.separator();
                if menu_item(ui, has_doc, "Save image…", "Ctrl+S") {
                    self.export.open = true;
                    ui.close();
                }
                if menu_item(ui, has_doc, "Save settings (XMP sidecar)", "") {
                    self.save_settings();
                    ui.close();
                }
                if menu_item(ui, has_doc, "Export settings as XMP…", "") {
                    if let Some(d) = self.doc() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("XMP", &["xmp"]).set_file_name("settings.xmp").save_file() {
                            let x = darkslide_core::xmp::to_xmp(
                                &d.settings,
                                d.is_raw(),
                                &darkslide_core::xmp::XmpExtra { rating: None, label: None, preset_name: None, preset_group: None },
                            );
                            let _ = std::fs::write(p, x);
                        }
                    }
                    ui.close();
                }
                if menu_item(ui, has_doc, "Load settings from XMP…", "") {
                    if let Some(p) = rfd::FileDialog::new().add_filter("XMP", &["xmp"]).pick_file() {
                        if let Some(pr) = darkslide_core::xmp::load_preset(&p) {
                            if let Some(d) = self.doc_mut() {
                                d.settings = pr.apply(&d.settings, 100.0);
                            }
                        }
                    }
                    ui.close();
                }
                ui.separator();
                let n_sel = self.docs.iter().filter(|d| d.selected).count();
                if menu_item(ui, n_sel >= 2, "Merge to HDR…", "") {
                    self.merge.open = Some(crate::dialogs::MergeKind::Hdr);
                    ui.close();
                }
                if menu_item(ui, n_sel >= 2, "Merge to Panorama…", "") {
                    self.merge.open = Some(crate::dialogs::MergeKind::Pano);
                    ui.close();
                }
                ui.separator();
                if menu_item(ui, true, "Preferences…", "") {
                    self.show_prefs = true;
                    ui.close();
                }
                ui.separator();
                if menu_item(ui, true, "Quit", "Ctrl+Q") {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if menu_item(ui, can_undo, "Undo", "Ctrl+Z") {
                    if let Some(d) = self.doc_mut() {
                        d.undo();
                    }
                }
                if menu_item(ui, can_redo, "Redo", "Ctrl+Shift+Z") {
                    if let Some(d) = self.doc_mut() {
                        d.redo();
                    }
                }
                ui.separator();
                if menu_item(ui, has_doc, "Copy settings", "Ctrl+C") {
                    self.copy_settings(None);
                    ui.close();
                }
                if menu_item(ui, has_doc, "Copy settings (choose)…", "Ctrl+Alt+C") {
                    self.copy.open_for_copy();
                    ui.close();
                }
                if menu_item(ui, has_clip, "Paste settings", "Ctrl+V") {
                    self.paste_settings(None);
                    ui.close();
                }
                if menu_item(ui, has_clip, "Paste settings (choose)…", "Ctrl+Alt+V") {
                    self.copy.open_for_paste();
                    ui.close();
                }
                if menu_item(ui, has_doc, "Sync settings to selected…", "Ctrl+Shift+S") {
                    self.copy_settings(None);
                    self.copy.open_for_paste();
                    ui.close();
                }
                ui.separator();
                if menu_item(ui, self.docs.len() > 1, "Select all", "") {
                    for d in self.docs.iter_mut() {
                        d.selected = true;
                    }
                    ui.close();
                }
            });
            ui.menu_button("Image", |ui| {
                if menu_item(ui, has_doc, "Auto tone", "Ctrl+U") {
                    self.pending_actions.push(Action::AutoTone);
                    ui.close();
                }
                if menu_item(ui, has_doc, "Auto white balance", "Ctrl+Shift+U") {
                    self.pending_actions.push(Action::AutoWb);
                    ui.close();
                }
                if menu_item(ui, has_doc, "Reset all settings…", "Ctrl+R") {
                    self.confirm_reset = true;
                    ui.close();
                }
                if menu_item(ui, has_doc, "Reset to open settings", "") {
                    if let Some(d) = self.doc_mut() {
                        d.settings = d.opened.clone();
                    }
                    ui.close();
                }
                ui.separator();
                if menu_item(ui, has_doc, "Rotate 90° CW", "") {
                    if let Some(d) = self.doc_mut() {
                        tool_panels::rotate90(&mut d.settings, 1);
                    }
                }
                if menu_item(ui, has_doc, "Rotate 90° CCW", "") {
                    if let Some(d) = self.doc_mut() {
                        tool_panels::rotate90(&mut d.settings, 3);
                    }
                }
                if menu_item(ui, has_doc, "Flip horizontal", "") {
                    if let Some(d) = self.doc_mut() {
                        d.settings.crop.flip_h = !d.settings.crop.flip_h;
                    }
                }
                ui.separator();
                if menu_item(ui, has_doc, "Save new defaults for this camera", "") {
                    if let Some(d) = self.doc() {
                        if let Some(e) = &d.engine {
                            let r = crate::doc::save_camera_default(e, &d.settings, false);
                            self.status = match r {
                                Ok(_) => "Saved camera defaults".into(),
                                Err(e) => format!("Couldn't save camera defaults: {e}"),
                            };
                        }
                    }
                    ui.close();
                }
                if menu_item(ui, has_doc, "Reset camera defaults", "") {
                    if let Some(e) = self.doc().and_then(|d| d.engine.clone()) {
                        crate::doc::reset_camera_default(&e);
                    }
                    ui.close();
                }
                if menu_item(ui, has_doc, "Copy after to before", "") {
                    if let Some(d) = self.doc_mut() {
                        d.opened = d.settings.clone();
                    }
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                if menu_item(ui, has_doc, "Fit", "Ctrl+0") {
                    self.zoom = Zoom::Fit;
                }
                if menu_item(ui, has_doc, "100%", "Ctrl+Alt+0") {
                    self.zoom = Zoom::Scale(1.0);
                }
                if menu_item(ui, has_doc, "Zoom in", "Ctrl+=") {
                    self.zoom_step(1);
                }
                if menu_item(ui, has_doc, "Zoom out", "Ctrl+−") {
                    self.zoom_step(-1);
                }
                ui.separator();
                for m in [BeforeAfter::Off, BeforeAfter::LeftRight, BeforeAfter::TopBottom, BeforeAfter::SplitLeftRight, BeforeAfter::SplitTopBottom] {
                    ui.radio_value(&mut self.ba, m, m.label());
                }
                ui.checkbox(&mut self.show_before, "Show before  \\");
                ui.separator();
                ui.checkbox(&mut self.clip_s, "Shadow clipping  U");
                ui.checkbox(&mut self.clip_h, "Highlight clipping  O");
                ui.checkbox(&mut self.edit.show_scope, "Vectorscope");
                ui.checkbox(&mut self.show_filmstrip, "Filmstrip  Tab");
                ui.checkbox(&mut self.grid, "Grid overlay  Ctrl+'");
                if ui.checkbox(&mut self.fullscreen, "Full screen  F").changed() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                }
                ui.separator();
                let mut sampling = self.pick == Pick::Sampler && self.tools.tool == Tool::Edit;
                if ui.add_enabled(has_doc, egui::Checkbox::new(&mut sampling, "Color sampler tool")).on_hover_text("Click the photo to pin up to 9 RGB readouts").changed() {
                    self.set_tool(Tool::Edit);
                    self.pick = if sampling { Pick::Sampler } else { Pick::None };
                }
                if menu_item(ui, !self.samplers.is_empty(), "Clear color samplers", "") {
                    self.samplers.clear();
                }
            });
            ui.menu_button("Help", |ui| {
                if menu_item(ui, true, "Keyboard shortcuts", "?") {
                    self.show_shortcuts = true;
                    ui.close();
                }
                if menu_item(ui, true, "About Darkslide", "") {
                    self.show_about = true;
                    ui.close();
                }
            });
            ui.separator();
            if let Some(d) = self.doc() {
                ui.label(RichText::new(d.name()).strong().color(Color32::from_rgb(230, 230, 230)));
                if let Some(e) = &d.engine {
                    let m = &e.source.meta;
                    ui.label(RichText::new(format!("•  {} {}", m.make, m.model)).color(Color32::from_rgb(150, 150, 150)));
                }
                if d.dirty {
                    ui.label(RichText::new("●").color(Color32::from_rgb(240, 175, 55))).on_hover_text("Unsaved settings");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let icon = |ui: &mut egui::Ui, enabled: bool, glyph: &str, tip: &str| {
                    ui.add_enabled(enabled, egui::Button::new(RichText::new(glyph).size(15.0)).frame(false)).on_hover_text(tip).clicked()
                };
                if icon(ui, true, ic::QUESTION, "Keyboard shortcuts (?)") {
                    self.show_shortcuts = true;
                }
                if icon(ui, true, ic::GEAR_SIX, "Preferences…") {
                    self.show_prefs = true;
                }
                ui.separator();
                if icon(ui, can_redo, ic::ARROW_U_UP_RIGHT, "Redo (Ctrl+Shift+Z)") {
                    if let Some(d) = self.doc_mut() {
                        d.redo();
                    }
                }
                if icon(ui, can_undo, ic::ARROW_U_UP_LEFT, "Undo (Ctrl+Z)") {
                    if let Some(d) = self.doc_mut() {
                        d.undo();
                    }
                }
            });
        });
    }

    /// Save the sidecar and say where it went.
    pub fn save_settings(&mut self) {
        if let Some(d) = self.doc_mut() {
            d.save_sidecar();
            let name = darkslide_core::xmp::sidecar_path(&d.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.status = format!("Settings saved to {name}");
        }
    }

    fn toolbar_strip(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            for (t, _icon, tip) in Tool::ALL {
                let sel = self.tools.tool == t;
                let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::click());
                let p = ui.painter_at(rect);

                if sel {
                    p.rect_filled(rect, 4.0, crate::theme::pal().button_hover);
                    p.rect_filled(Rect::from_min_size(pos2(rect.left(), rect.top() + 7.0), vec2(2.5, 22.0)), 1.0, crate::widgets::ACCENT);
                } else if resp.hovered() {
                    p.rect_filled(rect, 4.0, crate::theme::pal().button);
                }

                let icon_col = if sel {
                    Color32::WHITE
                } else if resp.hovered() {
                    Color32::from_rgb(225, 225, 225)
                } else {
                    Color32::from_rgb(200, 200, 200)
                };

                crate::widgets::draw_tool_icon(&p, t, rect.center(), 22.0, icon_col);

                if resp.on_hover_text(tip).clicked() {
                    self.tools.tool = t;
                    self.pick = Pick::None;
                }
                ui.add_space(4.0);
            }

            ui.add_space(8.0);
            let sep_y = ui.cursor().top();
            let w = ui.available_width();
            ui.painter().line_segment(
                [pos2(ui.cursor().left() + 6.0, sep_y), pos2(ui.cursor().left() + w - 6.0, sep_y)],
                Stroke::new(1.0, crate::theme::pal().divider),
            );
            ui.add_space(8.0);

            let (f_rect, f_resp) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::click());
            if f_resp.hovered() {
                ui.painter().rect_filled(f_rect, 4.0, crate::theme::pal().button);
            }
            let f_col = if f_resp.hovered() || self.fullscreen { Color32::WHITE } else { Color32::from_rgb(145, 145, 145) };
            ui.painter().text(f_rect.center(), egui::Align2::CENTER_CENTER, ic::CORNERS_OUT, egui::FontId::proportional(19.0), f_col);
            if f_resp.on_hover_text("Full screen (F)").clicked() {
                self.fullscreen = !self.fullscreen;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
        });
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        // histogram + info
        let hr = crate::widgets::histogram(ui, self.hist.as_ref(), self.clip_s, self.clip_h);
        if hr.toggle_shadows {
            self.clip_s = !self.clip_s;
        }
        if hr.toggle_highlights {
            self.clip_h = !self.clip_h;
        }
        if let Some(d) = self.doc() {
            if let Some(e) = &d.engine {
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(e.source.meta.exposure_summary()).size(11.0).color(Color32::from_rgb(175, 175, 175)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{} ms", self.render_ms)).size(10.0).color(crate::theme::pal().text_muted))
                            .on_hover_text("Last preview render time");
                    });
                });
                if let Some(l) = &e.source.meta.lens {
                    ui.label(RichText::new(l).size(10.5).color(Color32::from_rgb(130, 130, 130)));
                }
            }
        }
        // always present (blank off the photo) so the panel below never jumps
        let rgb = match self.drag.hover_rgb {
            Some(c) => format!("R {:>3}  G {:>3}  B {:>3}", c[0], c[1], c[2]),
            None => "R      G      B    ".into(),
        };
        ui.label(RichText::new(rgb).monospace().size(10.5).color(Color32::from_rgb(150, 150, 150)));
        if self.edit.show_scope {
            if let Some(sc) = &self.scope {
                crate::widgets::vectorscope(ui, sc, crate::worker::SCOPE_SIZE, true);
            }
        }
        if !self.samplers.is_empty() {
            for (i, s) in self.samplers.iter().enumerate() {
                if let Some(rgb) = self.sample_fit(*s) {
                    ui.label(RichText::new(format!("#{}  R {:>3} G {:>3} B {:>3}", i + 1, rgb[0], rgb[1], rgb[2])).monospace().size(10.5));
                }
            }
        }
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(2.0);

        let Some(ci) = self.current else {
            ui.label(RichText::new("Add photos to start editing.").color(Color32::from_gray(150)));
            return;
        };
        let hist = self.hist.clone();
        let mut actions = vec![];
        // a scrollbar that's always there says when more is below
        egui::ScrollArea::vertical().auto_shrink([false, false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show(ui, |ui| {
            let doc = &mut self.docs[ci];
            if let Some(err) = &doc.load_error {
                ui.colored_label(Color32::from_rgb(230, 90, 90), format!("Cannot open: {err}"));
                return;
            }
            if doc.engine.is_none() {
                ui.spinner();
                return;
            }
            match self.tools.tool {
                Tool::Edit => {
                    let mut pc = PanelCtx { hist: hist.as_ref(), pick: &mut self.pick, actions: &mut actions };
                    edit_panel::edit_panel(ui, doc, &mut self.edit, &mut pc);
                }
                Tool::Crop => tool_panels::crop_panel(ui, doc, &mut self.tools),
                Tool::Remove => tool_panels::remove_panel(ui, doc, &mut self.tools, &mut actions),
                Tool::Masking => tool_panels::masking_panel(ui, doc, &mut self.tools, &mut self.edit, &mut self.pick),
                Tool::RedEye => tool_panels::redeye_panel(ui, doc, &mut self.tools),
                Tool::Presets => crate::presets_panel::presets_panel(ui, doc, &mut self.tools),
                Tool::Snapshots => tool_panels::snapshots_panel(ui, doc, &mut self.tools),
            }
        });
        self.pending_actions.extend(actions);
    }

    /// RGB of the fit render at crop-frame coords.
    pub fn sample_fit(&self, c: [f32; 2]) -> Option<[u8; 3]> {
        let (rgba, w, h) = self.fit_rgba.as_ref()?;
        let x = ((c[0] * *w as f32) as isize).clamp(0, *w as isize - 1) as usize;
        let y = ((c[1] * *h as f32) as isize).clamp(0, *h as isize - 1) as usize;
        let i = (y * w + x) * 4;
        Some([rgba[i], rgba[i + 1], rgba[i + 2]])
    }

    /// ACR's workflow line: what "Save image" will produce with the current
    /// export settings, e.g. "Adobe RGB (1998) - 16 bit - 6016 x 4016 (24.2MP) - 300 ppi".
    fn workflow_text(&self, short: bool) -> Option<String> {
        use darkslide_core::export::Format;
        let ex = self.export.settings.as_ref().unwrap_or(&self.prefs.export);
        let (cw, ch, _) = self.crop_dims_for_export()?;
        let sr = if self.doc()?.settings.enhance.super_resolution { 2.0 } else { 1.0 };
        let (w, h) = darkslide_core::export::output_size(cw * sr, ch * sr, ex.resize, ex.dont_enlarge && sr == 1.0);
        let bits = match ex.format {
            Format::Jpeg | Format::Avif => 8,
            Format::Exr => 32,
            Format::Dng => 16,
            _ => ex.bit_depth,
        };
        let mp = w as f64 * h as f64 / 1e6;
        if short {
            let space = ex.color_space.name().split([' ', '(']).next().unwrap_or("");
            return Some(format!("{space} · {bits} bit · {mp:.1}MP"));
        }
        Some(format!("{} - {bits} bit - {w} x {h} ({mp:.1}MP) - {} ppi", ex.color_space.name(), ex.ppi))
    }

    /// Crop size ignoring the crop tool's uncropped view.
    fn crop_dims_for_export(&self) -> Option<(f64, f64, bool)> {
        let d = self.doc()?;
        let e = d.engine.as_ref()?;
        let (fw, fh) = e.full_size();
        let (w, h) = e.geometry(&d.settings, fw, fh, false).crop_size();
        Some((w, h, false))
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        let has_doc = self.doc().is_some();
        // status messages fade after a few seconds
        if self.status != self.status_seen.0 {
            self.status_seen = (self.status.clone(), std::time::Instant::now());
        } else if !self.status.is_empty() {
            let left = std::time::Duration::from_secs(5).saturating_sub(self.status_seen.1.elapsed());
            if left.is_zero() {
                self.status.clear();
            } else {
                ui.ctx().request_repaint_after(left);
            }
        }
        ui.horizontal(|ui| {
            ui.add_enabled_ui(has_doc, |ui| {
                let zl = match self.zoom {
                    Zoom::Fit => "Fit".to_string(),
                    Zoom::Scale(z) => format!("{:.0}%", z * 100.0),
                };
                egui::ComboBox::from_id_salt("zoom")
                    .selected_text(zl)
                    .width(68.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.zoom, Zoom::Fit, "Fit");
                        for z in [0.25f32, 0.5, 1.0, 2.0, 4.0, 8.0] {
                            ui.selectable_value(&mut self.zoom, Zoom::Scale(z), format!("{:.0}%", z * 100.0));
                        }
                    })
                    .response
                    .on_hover_text("Zoom (Z toggles Fit / 100%)");
                let small = |ui: &mut egui::Ui, glyph: &str, tip: &str| ui.add_sized([24.0, 22.0], egui::Button::new(glyph)).on_hover_text(tip).clicked();
                if small(ui, ic::MAGNIFYING_GLASS_MINUS, "Zoom out (Ctrl+−)") {
                    self.zoom_step(-1);
                }
                if small(ui, ic::MAGNIFYING_GLASS_PLUS, "Zoom in (Ctrl+=)") {
                    self.zoom_step(1);
                }
                ui.separator();
                // fixed sizes so switching a toggle on never shifts what follows
                let toggle = |ui: &mut egui::Ui, on: bool, text: RichText, w: f32, tip: &str| {
                    ui.add_sized([w, 22.0], egui::Button::selectable(on, text)).on_hover_text(tip).clicked()
                };
                let ba_icon = match self.ba {
                    BeforeAfter::TopBottom | BeforeAfter::SplitTopBottom => ic::SQUARE_SPLIT_VERTICAL,
                    _ => ic::SQUARE_SPLIT_HORIZONTAL,
                };
                if toggle(ui, self.ba != BeforeAfter::Off, RichText::new(ba_icon).size(15.0), 28.0, &format!("{} — Q cycles the views", self.ba.label())) {
                    self.ba = self.ba.next();
                }
                let single = self.ba == BeforeAfter::Off;
                if ui
                    .add_enabled_ui(single, |ui| toggle(ui, self.show_before, RichText::new("Before"), 58.0, "Show the photo before editing (\\)"))
                    .inner
                {
                    self.show_before = !self.show_before;
                }
                ui.separator();
                if toggle(ui, self.clip_s, RichText::new(ic::ARROW_LINE_DOWN_LEFT).size(14.0), 28.0, "Shadow clipping warning (U)") {
                    self.clip_s = !self.clip_s;
                }
                if toggle(ui, self.clip_h, RichText::new(ic::ARROW_LINE_UP_RIGHT).size(14.0), 28.0, "Highlight clipping warning (O)") {
                    self.clip_h = !self.clip_h;
                }
            });
            if self.pick != Pick::None {
                ui.separator();
                ui.label(RichText::new(format!("{} — Esc to finish", self.pick.label())).color(crate::widgets::ACCENT));
            }
            if let Some(b) = self.busy.lock().unwrap().clone() {
                ui.spinner();
                ui.label(b);
            }
            if !self.status.is_empty() {
                ui.label(RichText::new(&self.status).weak());
            }
            for st in self.export_status.lock().unwrap().iter() {
                if !st.done {
                    ui.spinner();
                    let n = if st.total > 1 { format!(" {} of {}", st.finished + 1, st.total) } else { String::new() };
                    ui.label(format!("Saving{n}…"));
                    ui.add(egui::ProgressBar::new(st.progress.max(0.02)).desired_width(120.0).animate(true));
                } else if let Some(msg) = &st.message {
                    if st.done_at.map(|t| t.elapsed().as_secs() < 8).unwrap_or(true) {
                        ui.label(RichText::new(msg).weak());
                        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tool = self.tools.tool;
                let done_btn = egui::Button::new(RichText::new("Done").strong().color(Color32::from_rgb(225, 225, 225)))
                    .fill(crate::theme::pal().button_hover)
                    .stroke(Stroke::new(1.0, crate::theme::pal().border_strong))
                    .corner_radius(3.0)
                    .min_size(vec2(72.0, 24.0));
                let tip = if tool == Tool::Edit { "Save settings to the XMP sidecar" } else { "Finish this tool and save settings to the XMP sidecar" };
                if ui.add_enabled(has_doc, done_btn).on_hover_text(tip).clicked() {
                    if tool == Tool::Crop {
                        self.commit_crop();
                    } else {
                        self.set_tool(Tool::Edit);
                    }
                    self.save_settings();
                }
                let save_btn = egui::Button::new(RichText::new("Save image…").strong().color(Color32::WHITE))
                    .fill(crate::widgets::ACCENT)
                    .corner_radius(3.0)
                    .min_size(vec2(96.0, 24.0));
                if ui.add_enabled(has_doc, save_btn).on_hover_text("Ctrl+S").clicked() {
                    self.export.open = true;
                }
                ui.add_space(12.0);
                // the long line only when there's room for it beside the buttons
                let room = ui.available_width();
                let text = if room > 360.0 { self.workflow_text(false) } else if room > 150.0 { self.workflow_text(true) } else { None };
                if let Some(text) = text {
                    if ui
                        .link(RichText::new(text).size(11.5).underline().color(crate::theme::pal().link))
                        .on_hover_text("Output settings (Save image)")
                        .clicked()
                    {
                        self.export.open = true;
                    }
                }
            });
        });
    }
}

/// Menu entry with its shortcut right-aligned in a muted colour.
fn menu_item(ui: &mut egui::Ui, enabled: bool, label: &str, shortcut: &str) -> bool {
    let mut b = egui::Button::new(label);
    if !shortcut.is_empty() {
        b = b.shortcut_text(shortcut);
    }
    ui.add_enabled(enabled, b.min_size(vec2(220.0, 0.0))).clicked()
}

fn shellexpand(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = std::env::var_os("HOME") {
            return format!("{}/{}", h.to_string_lossy(), rest);
        }
    }
    p.to_string()
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        // drag & drop
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty() {
            self.open_paths(dropped);
        }
        // the crop the tool started with, for Esc to restore
        if self.tools.tool != self.prev_tool {
            self.crop_on_enter = if self.tools.tool == Tool::Crop { self.doc().map(|d| d.settings.crop) } else { None };
            if self.prev_tool == Tool::Crop {
                self.tools.straighten = false;
            }
            self.prev_tool = self.tools.tool;
        }
        crate::widgets::set_solo(&ctx, self.prefs.solo_sections);
        if crate::widgets::take_solo_toggle(&ctx) {
            self.prefs.solo_sections = !self.prefs.solo_sections;
            self.prefs.save();
        }
        self.shortcuts(&ctx);
        // the mask overlay is coloured when a render arrives; a new colour needs one
        if self.tools.mask_overlay_color != self.last_overlay {
            self.last_overlay = self.tools.mask_overlay_color;
            self.last_fit = None;
        }

        egui::Panel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::pal().bar)
                    .stroke(Stroke::new(1.0, crate::theme::pal().divider))
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| self.top_bar(ui));

        egui::Panel::bottom("bottom")
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::pal().bar)
                    .stroke(Stroke::new(1.0, crate::theme::pal().divider))
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ui, |ui| self.bottom_bar(ui));

        if self.show_filmstrip && self.docs.len() > 1 {
            egui::Panel::bottom("filmstrip")
                .resizable(true)
                .default_size(118.0)
                .frame(
                    egui::Frame::new()
                        .fill(crate::theme::pal().filmstrip)
                        .stroke(Stroke::new(1.0, crate::theme::pal().divider))
                        .inner_margin(egui::Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| crate::filmstrip::filmstrip(self, ui));
        }

        // Toolbar strip docked at the far right
        egui::Panel::right("toolbar_strip")
            .resizable(false)
            .default_size(44.0)
            .size_range(44.0..=44.0)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::pal().bar)
                    .stroke(Stroke::new(1.0, crate::theme::pal().divider))
                    .inner_margin(egui::Margin::symmetric(4, 6)),
            )
            .show(ui, |ui| self.toolbar_strip(ui));

        // Adjustment panel docked to the left of the toolbar strip
        egui::Panel::right("edit_panel")
            .resizable(true)
            .default_size(325.0)
            .size_range(280.0..=560.0)
            .frame(egui::Frame::new().fill(crate::theme::pal().panel).stroke(Stroke::new(1.0, crate::theme::pal().divider)).inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 8,
                bottom: 6,
            }))
            .show(ui, |ui| self.right_panel(ui));

        // Central image canvas
        egui::CentralPanel::default().frame(egui::Frame::new().fill(crate::theme::pal().canvas).inner_margin(egui::Margin::same(4))).show(ui, |ui| {
            crate::viewer::viewer(self, ui);
        });

        crate::dialogs::show_dialogs(self, &ctx);
        if let Some(l) = crate::widgets::take_auto_slider(&ctx) {
            self.auto_one(&l);
        }
        self.run_actions();

        // history (commit when nothing is being dragged)
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        if let Some(d) = self.doc_mut() {
            d.track_history(pointer_down);
        }
        self.autosave();
        let interacting = crate::widgets::take_interacting(&ctx) || (pointer_down && self.drag.active());
        let alt = crate::widgets::take_alt_preview(&ctx);
        self.schedule(&ctx, interacting && alt.is_none(), alt);
        let _ = Vec2::ZERO;
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if ctx.text_edit_focused() {
            return;
        }
        // Tab toggles the filmstrip; egui would also move focus to the menu bar
        let mut tab = false;
        raw.events.retain(|e| match e {
            egui::Event::Key { key: egui::Key::Tab, pressed, modifiers, .. } if modifiers.is_none() => {
                tab |= *pressed;
                false
            }
            _ => true,
        });
        self.tab_pressed |= tab;
        // egui-winit turns Ctrl+C / Ctrl+V into clipboard events; settings copy needs the keys
        let m = ctx.input(|i| i.modifiers);
        let mut keys = vec![];
        raw.events.retain(|e| match e {
            egui::Event::Copy => {
                keys.push(egui::Key::C);
                false
            }
            egui::Event::Paste(_) => {
                keys.push(egui::Key::V);
                false
            }
            _ => true,
        });
        for key in keys {
            for pressed in [true, false] {
                raw.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: m });
            }
        }
    }

    fn on_exit(&mut self) {
        if let Some((_, p, t)) = self.thumb_save.take() {
            darkslide_core::thumbcache::put(&p, &t);
        }
        let autosave = self.prefs.autosave_sidecars;
        for d in self.docs.iter_mut() {
            if d.dirty && autosave {
                d.save_sidecar();
            }
        }
        self.prefs.show_vectorscope = self.edit.show_scope;
        self.prefs.save();
    }
}
