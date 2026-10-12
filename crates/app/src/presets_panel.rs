//! Presets: searchable, grouped cards with a live preview of the current
//! photo under each preset, favourites, and hover preview on the canvas.

use egui_phosphor::regular as ic;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use darkslide_core::params::{Crop, DevelopSettings};
use darkslide_core::pipeline::Engine;
use darkslide_core::xmp::{self, Preset};

use crate::doc::Doc;
use crate::theme::pal;
use crate::tool_panels::ToolUi;
use crate::widgets::{ACCENT, SliderSpec, slider};

const THUMB_W: usize = 220;

#[derive(Clone)]
enum Source {
    Builtin(fn(&mut DevelopSettings)),
    User(Box<Preset>),
}

#[derive(Clone)]
struct Entry {
    group: String,
    name: String,
    src: Source,
}

impl Entry {
    fn key(&self) -> String {
        format!("{}/{}", self.group, self.name)
    }

    /// The settings this preset produces on top of `s` at `amount` (0..200).
    fn apply(&self, s: &DevelopSettings, amount: f32) -> DevelopSettings {
        match &self.src {
            Source::Builtin(f) => {
                let mut target = s.clone();
                f(&mut target);
                xmp::blend(s, &target, amount / 100.0)
            }
            Source::User(p) => p.apply(s, amount),
        }
    }
}

#[derive(Default)]
pub struct PresetUi {
    entries: Vec<Entry>,
    search: String,
    favorites: BTreeSet<String>,
    favorites_loaded: bool,
    creating: bool,
    /// Last applied preset key, shown as selected.
    applied: Option<String>,
    /// Thumbnails rendered for `thumb_base`, by entry index.
    thumbs: Arc<Mutex<Vec<Option<egui::ColorImage>>>>,
    textures: Vec<Option<egui::TextureHandle>>,
    thumb_base: Option<(u64, DevelopSettings)>,
    generation: Arc<AtomicU64>,
}

fn favorites_path() -> std::path::PathBuf {
    darkslide_core::paths::config_dir().join("preset_favorites.json")
}

impl PresetUi {
    /// Thumbnails still rendering (screenshots wait for them).
    #[allow(dead_code)]
    pub fn thumbs_pending(&self) -> bool {
        self.thumb_base.is_some() && self.thumbs.lock().unwrap().iter().any(|t| t.is_none())
    }

    fn load_favorites(&mut self) {
        if !self.favorites_loaded {
            self.favorites_loaded = true;
            if let Ok(t) = std::fs::read_to_string(favorites_path()) {
                self.favorites = serde_json::from_str(&t).unwrap_or_default();
            }
        }
    }

    fn toggle_favorite(&mut self, key: String) {
        if !self.favorites.remove(&key) {
            self.favorites.insert(key);
        }
        let _ = std::fs::create_dir_all(darkslide_core::paths::config_dir());
        let _ = std::fs::write(favorites_path(), serde_json::to_string(&self.favorites).unwrap_or_default());
    }

    fn rebuild(&mut self, user: &[Preset]) {
        let mut v: Vec<Entry> = darkslide_core::builtin_presets::all()
            .iter()
            .map(|b| Entry { group: b.group.to_string(), name: b.name.to_string(), src: Source::Builtin(b.apply) })
            .collect();
        v.extend(user.iter().map(|p| Entry {
            group: if p.group.is_empty() { "User presets".into() } else { p.group.clone() },
            name: p.name.clone(),
            src: Source::User(Box::new(p.clone())),
        }));
        self.entries = v;
        self.thumb_base = None;
    }

    /// (Re)render thumbnails in the background when the photo or its settings change.
    fn refresh_thumbs(&mut self, doc: &Doc) {
        let Some(engine) = doc.engine.clone() else { return };
        if self.thumb_base.as_ref().is_some_and(|(id, s)| *id == doc.id && *s == doc.settings) {
            return;
        }
        self.thumb_base = Some((doc.id, doc.settings.clone()));
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // keep the old pictures until new ones arrive, so the grid doesn't flash
        {
            let mut t = self.thumbs.lock().unwrap();
            if t.len() != self.entries.len() {
                *t = vec![None; self.entries.len()];
                self.textures = vec![None; self.entries.len()];
            }
        }
        let entries = self.entries.clone();
        let base = doc.settings.clone();
        let out = self.thumbs.clone();
        let current = self.generation.clone();
        let ctx_gen = generation;
        std::thread::spawn(move || {
            // a quick look while settings settle (sliders being dragged)
            std::thread::sleep(std::time::Duration::from_millis(250));
            for (i, e) in entries.iter().enumerate() {
                if current.load(Ordering::SeqCst) != ctx_gen {
                    return;
                }
                let ci = render_thumb(&engine, &e.apply(&base, 100.0));
                if let Some(slot) = out.lock().unwrap().get_mut(i) {
                    *slot = Some(ci);
                }
            }
        });
    }
}

fn render_thumb(engine: &Engine, s: &DevelopSettings) -> egui::ColorImage {
    let r = engine.render_fit(s, THUMB_W, THUMB_W * 2 / 3, darkslide_core::demosaic::Quality::Draft, false);
    let px = darkslide_core::pipeline::to_srgb8(&r.image, Some(&r.alpha), [40, 40, 40]);
    egui::ColorImage::from_rgba_unmultiplied([r.image.w, r.image.h], &px)
}

fn icon_button(ui: &mut egui::Ui, icon: &str, tip: &str) -> bool {
    ui.add(egui::Button::new(RichText::new(icon).size(14.0)).min_size(vec2(26.0, 24.0))).on_hover_text(tip).clicked()
}

pub fn presets_panel(ui: &mut egui::Ui, doc: &mut Doc, t: &mut ToolUi) {
    t.hover_preview = None;
    let p = pal();
    if !t.presets_loaded {
        t.presets = xmp::list_presets();
        t.presets_loaded = true;
        t.preset_ui.rebuild(&t.presets);
    }
    t.preset_ui.load_favorites();
    t.preset_ui.refresh_thumbs(doc);

    // header
    ui.horizontal(|ui| {
        ui.label(RichText::new("Presets").size(16.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, ic::FOLDER_OPEN, "Open presets folder") {
                let d = darkslide_core::paths::presets_dir();
                let _ = std::fs::create_dir_all(&d);
                let _ = std::process::Command::new("xdg-open").arg(d).spawn();
            }
            if icon_button(ui, ic::ARROWS_CLOCKWISE, "Rescan presets folder") {
                t.presets_loaded = false;
            }
            if icon_button(ui, ic::DOWNLOAD_SIMPLE, "Import XMP presets…")
                && let Some(files) = rfd::FileDialog::new().add_filter("XMP presets", &["xmp"]).pick_files()
            {
                let dir = darkslide_core::paths::presets_dir().join("Imported");
                let _ = std::fs::create_dir_all(&dir);
                let n = files.iter().filter(|f| f.file_name().is_some_and(|name| std::fs::copy(f, dir.join(name)).is_ok())).count();
                t.status = Some(format!("Imported {n} preset{}", if n == 1 { "" } else { "s" }));
                t.presets_loaded = false;
            }
        });
    });
    ui.add_space(6.0);

    // search
    let w = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut t.preset_ui.search)
            .hint_text(format!("{}  Search presets", ic::MAGNIFYING_GLASS))
            .desired_width(w)
            .margin(vec2(8.0, 5.0)),
    );
    ui.add_space(4.0);
    slider(ui, &mut t.preset_amount, SliderSpec::new("Amount", 0.0, 200.0, 100.0));
    ui.add_space(4.0);

    // create
    if !t.preset_ui.creating {
        if ui.add_sized([w, 26.0], egui::Button::new(format!("{}  Create preset from current settings", ic::PLUS))).clicked() {
            t.preset_ui.creating = true;
        }
    } else {
        egui::Frame::new().fill(p.well).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("New preset").strong());
            ui.add_space(4.0);
            ui.add(egui::TextEdit::singleline(&mut t.new_preset_name).hint_text("Name").desired_width(f32::INFINITY));
            ui.add(egui::TextEdit::singleline(&mut t.new_preset_group).hint_text("Group (optional)").desired_width(f32::INFINITY));
            ui.label(RichText::new("Crop, spot removal and red eye are left out.").size(11.0).color(p.text_muted));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let ok = !t.new_preset_name.trim().is_empty();
                if ui.add_enabled(ok, egui::Button::new(RichText::new("Save").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                    let mut s = doc.settings.clone();
                    // presets never carry image-specific geometry or spot edits
                    s.crop = Crop::default();
                    s.retouch.clear();
                    s.red_eye.clear();
                    match xmp::save_preset(t.new_preset_name.trim(), t.new_preset_group.trim(), &s) {
                        Ok(_) => t.status = Some(format!("Saved “{}”", t.new_preset_name.trim())),
                        Err(e) => t.status = Some(format!("Error: {e}")),
                    }
                    t.new_preset_name.clear();
                    t.presets_loaded = false;
                    t.preset_ui.creating = false;
                }
                if ui.button("Cancel").clicked() {
                    t.preset_ui.creating = false;
                }
            });
        });
    }
    if let Some(st) = &t.status {
        ui.label(RichText::new(st).size(11.0).color(p.text_muted));
    }
    ui.add_space(6.0);

    // upload finished thumbnails
    {
        let thumbs = t.preset_ui.thumbs.lock().unwrap();
        if t.preset_ui.textures.len() != thumbs.len() {
            t.preset_ui.textures = vec![None; thumbs.len()];
        }
        for (i, th) in thumbs.iter().enumerate() {
            if let Some(ci) = th {
                match &mut t.preset_ui.textures[i] {
                    Some(tex) if tex.size() == ci.size => tex.set(ci.clone(), egui::TextureOptions::LINEAR),
                    slot => *slot = Some(ui.ctx().load_texture(format!("preset{i}"), ci.clone(), egui::TextureOptions::LINEAR)),
                }
            }
        }
        if thumbs.iter().any(|t| t.is_none()) || t.preset_ui.generation.load(Ordering::SeqCst) > 0 {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    // grouped cards; favourites first
    let q = t.preset_ui.search.trim().to_lowercase();
    let matches = |e: &Entry| q.is_empty() || e.name.to_lowercase().contains(&q) || e.group.to_lowercase().contains(&q);
    let mut groups: Vec<(String, Vec<usize>)> = vec![];
    let favs: Vec<usize> =
        (0..t.preset_ui.entries.len()).filter(|&i| t.preset_ui.favorites.contains(&t.preset_ui.entries[i].key()) && matches(&t.preset_ui.entries[i])).collect();
    if !favs.is_empty() {
        groups.push((format!("{}  Favorites", ic::STAR), favs));
    }
    for (i, e) in t.preset_ui.entries.iter().enumerate() {
        if !matches(e) {
            continue;
        }
        match groups.iter_mut().find(|(g, _)| *g == e.group) {
            Some((_, v)) => v.push(i),
            None => groups.push((e.group.clone(), vec![i])),
        }
    }
    if groups.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("No presets match your search").color(p.text_muted)));
    }

    let mut apply: Option<usize> = None;
    let mut fav_toggle: Option<String> = None;
    let col_gap = 8.0;
    for (g, idx) in &groups {
        let id = ui.make_persistent_id(("preset_group", g));
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
        // group header: chevron, name, count
        let (hr, hresp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
        if hresp.hovered() {
            ui.painter().rect_filled(hr, 4.0, p.header_hover);
        }
        let chev = if state.is_open() { "▾" } else { "▸" };
        ui.painter().text(pos2(hr.left() + 6.0, hr.center().y), egui::Align2::LEFT_CENTER, chev, egui::FontId::proportional(12.0), p.text_dim);
        ui.painter().text(pos2(hr.left() + 22.0, hr.center().y), egui::Align2::LEFT_CENTER, g, egui::FontId::proportional(13.5), p.text);
        ui.painter().text(
            pos2(hr.right() - 6.0, hr.center().y),
            egui::Align2::RIGHT_CENTER,
            idx.len().to_string(),
            egui::FontId::proportional(11.0),
            p.text_muted,
        );
        if hresp.clicked() {
            state.toggle(ui);
        }
        state.store(ui.ctx());
        if !state.is_open() && q.is_empty() {
            continue;
        }
        ui.add_space(4.0);
        let cw = ((ui.available_width() - col_gap) / 2.0).floor();
        let ch = (cw * 2.0 / 3.0).round();
        for row in idx.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = col_gap;
                for &i in row {
                    let e = &t.preset_ui.entries[i];
                    let key = e.key();
                    let (rect, resp) = ui.allocate_exact_size(vec2(cw, ch + 22.0), Sense::click());
                    let img_r = Rect::from_min_size(rect.min, vec2(cw, ch));
                    let painter = ui.painter_at(rect.expand(2.0));
                    painter.rect_filled(img_r, 6.0, p.well);
                    match t.preset_ui.textures.get(i).and_then(|t| t.as_ref()) {
                        Some(tex) => {
                            // fit inside the card, centred
                            let [tw, th] = tex.size().map(|v| v as f32);
                            let s = (cw / tw).min(ch / th);
                            let r = Rect::from_center_size(img_r.center(), vec2(tw * s, th * s));
                            egui::Image::new(tex).corner_radius(5.0).paint_at(ui, r);
                        }
                        None => {
                            egui::Spinner::new().size(16.0).color(p.text_muted).paint_at(ui, Rect::from_center_size(img_r.center(), vec2(16.0, 16.0)));
                        }
                    }
                    let selected = t.preset_ui.applied.as_deref() == Some(key.as_str());
                    if selected {
                        painter.rect_stroke(img_r, 6.0, Stroke::new(2.0, ACCENT), StrokeKind::Outside);
                    } else if resp.hovered() {
                        painter.rect_stroke(img_r, 6.0, Stroke::new(1.5, p.text_dim), StrokeKind::Outside);
                    }
                    let name_col = if selected { Color32::WHITE } else { p.text_dim };
                    let galley = painter.layout(e.name.clone(), egui::FontId::proportional(12.0), name_col, cw - 4.0);
                    painter.galley(pos2(rect.left() + 2.0, img_r.bottom() + 4.0), galley, name_col);
                    // favourite star: always shown when set, otherwise on hover
                    let fav = t.preset_ui.favorites.contains(&key);
                    let star_r = Rect::from_min_size(pos2(img_r.right() - 24.0, img_r.top() + 4.0), vec2(20.0, 20.0));
                    let star_resp = ui.interact(star_r, ui.id().with(("fav", i)), Sense::click());
                    if fav || resp.hovered() || star_resp.hovered() {
                        painter.circle_filled(star_r.center(), 10.0, Color32::from_black_alpha(140));
                        let col = if fav { Color32::from_rgb(255, 200, 60) } else { Color32::from_gray(220) };
                        let font = if fav { egui::FontId::new(13.0, egui::FontFamily::Name("phosphor-fill".into())) } else { egui::FontId::proportional(13.0) };
                        painter.text(star_r.center(), egui::Align2::CENTER_CENTER, ic::STAR, font, col);
                    }
                    if star_resp.on_hover_text(if fav { "Remove from favorites" } else { "Add to favorites" }).clicked() {
                        fav_toggle = Some(key.clone());
                    } else if resp.clicked() {
                        apply = Some(i);
                    } else if resp.hovered() {
                        t.hover_preview = Some(e.apply(&doc.settings, t.preset_amount));
                    }
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand).widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &e.name));
                }
            });
            ui.add_space(6.0);
        }
        ui.add_space(4.0);
    }
    if let Some(k) = fav_toggle {
        t.preset_ui.toggle_favorite(k);
    }
    if let Some(i) = apply {
        let e = &t.preset_ui.entries[i];
        doc.settings = e.apply(&doc.settings, t.preset_amount);
        t.preset_ui.applied = Some(e.key());
        t.status = Some(format!("Applied “{}” — Ctrl+Z to undo", e.name));
    }
    if t.presets.is_empty() && q.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("Add your own .xmp presets with Import, or put them in {}", darkslide_core::paths::presets_dir().display()))
                .size(11.0)
                .color(p.text_muted),
        );
    }
}
