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

/// The last applied preset, kept while it is still the latest edit so
/// Amount can re-blend it.
struct Applied {
    key: String,
    base: DevelopSettings,
    result: DevelopSettings,
}

#[derive(Default)]
pub struct PresetUi {
    entries: Vec<Entry>,
    search: String,
    favorites: BTreeSet<String>,
    favorites_loaded: bool,
    favorites_only: bool,
    creating: bool,
    focus_name: bool,
    /// Name typed into the form already exists; Save asks before replacing.
    confirm_replace: bool,
    applied: Option<Applied>,
    /// A card that doesn't preview until the pointer leaves it (just clicked,
    /// or the settings changed under it by undo).
    hover_block: Option<usize>,
    block_hovered: bool,
    seen: Option<DevelopSettings>,
    scroll_to: Option<String>,
    /// Thumbnails rendered for `thumb_base`, by entry index.
    thumbs: Arc<Mutex<Vec<Option<egui::ColorImage>>>>,
    textures: Vec<Option<egui::TextureHandle>>,
    thumb_base: Option<(u64, DevelopSettings, f32)>,
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

    /// Esc closes the create form, then clears the search; false when there was nothing to back out of.
    pub fn escape(&mut self) -> bool {
        if self.creating {
            self.creating = false;
        } else if !self.search.is_empty() {
            self.search.clear();
        } else {
            return false;
        }
        true
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
    fn refresh_thumbs(&mut self, doc: &Doc, amount: f32) {
        let Some(engine) = doc.engine.clone() else { return };
        if self.thumb_base.as_ref().is_some_and(|(id, s, a)| *id == doc.id && *s == doc.settings && *a == amount) {
            return;
        }
        self.thumb_base = Some((doc.id, doc.settings.clone(), amount));
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
                let ci = render_thumb(&engine, &e.apply(&base, amount));
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
    t.preset_ui.refresh_thumbs(doc, t.preset_amount);
    // settings changed from outside (undo, another panel): the applied mark is
    // stale, and the card under the pointer shouldn't hide the change
    if t.preset_ui.seen.as_ref() != Some(&doc.settings) {
        if t.preset_ui.seen.is_some() {
            t.preset_ui.block_hovered = true;
        }
        t.preset_ui.seen = Some(doc.settings.clone());
    }
    if t.preset_ui.applied.as_ref().is_some_and(|a| a.result != doc.settings) {
        t.preset_ui.applied = None;
    }

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

    let w = ui.available_width();
    // search, with the icon and clear button drawn inside the field so typing never shifts anything;
    // laid out from the right so the field takes exactly what the star leaves
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let on = t.preset_ui.favorites_only;
        let star = if on { RichText::new(ic::STAR).family(egui::FontFamily::Name("phosphor-fill".into())).color(Color32::from_rgb(255, 200, 60)) } else { RichText::new(ic::STAR) };
        if ui
            .add(egui::Button::new(star.size(14.0)).selected(on).min_size(vec2(28.0, 26.0)))
            .on_hover_text(if on { "Show all presets" } else { "Show favorites only" })
            .clicked()
        {
            t.preset_ui.favorites_only = !on;
        }
        let r = ui.add(
            egui::TextEdit::singleline(&mut t.preset_ui.search)
                .hint_text("Search presets")
                .desired_width(ui.available_width())
                .margin(egui::Margin { left: 26, right: 24, top: 5, bottom: 5 }),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            t.preset_ui.search.clear();
        }
        ui.painter().text(pos2(r.rect.left() + 9.0, r.rect.center().y), egui::Align2::LEFT_CENTER, ic::MAGNIFYING_GLASS, egui::FontId::proportional(13.0), p.text_muted);
        if !t.preset_ui.search.is_empty() {
            let xr = Rect::from_center_size(pos2(r.rect.right() - 12.0, r.rect.center().y), vec2(18.0, 18.0));
            let xresp = ui.interact(xr, ui.id().with("search_clear"), Sense::click()).on_hover_text("Clear search");
            let col = if xresp.hovered() { p.text } else { p.text_muted };
            ui.painter().text(xr.center(), egui::Align2::CENTER_CENTER, ic::X, egui::FontId::proportional(12.0), col);
            if xresp.clicked() {
                t.preset_ui.search.clear();
            }
        }
    });
    ui.add_space(4.0);
    // Amount re-blends the last preset while it's still the latest edit
    let before = t.preset_amount;
    slider(ui, &mut t.preset_amount, SliderSpec::new("Amount", 0.0, 200.0, 100.0));
    if t.preset_amount != before
        && let Some(a) = &mut t.preset_ui.applied
        && let Some(e) = t.preset_ui.entries.iter().find(|e| e.key() == a.key)
    {
        a.result = e.apply(&a.base, t.preset_amount);
        doc.settings = a.result.clone();
        t.preset_ui.seen = Some(doc.settings.clone());
    }
    ui.add_space(4.0);

    // create
    if !t.preset_ui.creating {
        if ui.add_sized([w, 26.0], egui::Button::new(format!("{}  Create preset from current settings", ic::PLUS))).clicked() {
            t.preset_ui.creating = true;
            t.preset_ui.focus_name = true;
            t.preset_ui.confirm_replace = false;
            t.status = None;
        }
    } else {
        let mut save = false;
        let mut cancel = false;
        egui::Frame::new().fill(p.well).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("New preset").strong());
            ui.add_space(4.0);
            let field = |ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str| {
                ui.horizontal(|ui| {
                    ui.add_sized([46.0, 20.0], egui::Label::new(RichText::new(label).color(p.text_dim)));
                    ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY))
                })
                .inner
            };
            let name_r = field(ui, "Name", &mut t.new_preset_name, "");
            if std::mem::take(&mut t.preset_ui.focus_name) {
                name_r.request_focus();
            }
            if name_r.changed() {
                t.preset_ui.confirm_replace = false;
            }
            let group_r = field(ui, "Group", &mut t.new_preset_group, "User presets");
            let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            if (name_r.lost_focus() || group_r.lost_focus()) && enter {
                save = true;
            }
            if (name_r.lost_focus() || group_r.lost_focus()) && esc {
                cancel = true;
            }
            // one note line, reused for the replace warning so the form keeps its size
            let note = if t.preset_ui.confirm_replace { "A preset with this name exists. Replace it?" } else { "Crop, spot removal and red eye are left out." };
            ui.add(egui::Label::new(RichText::new(note).size(11.0).color(p.text_muted)).truncate());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let ok = !t.new_preset_name.trim().is_empty();
                let label = if t.preset_ui.confirm_replace { "Replace" } else { "Save" };
                if ui.add_enabled(ok, egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(ACCENT)).on_disabled_hover_text("Type a name first").clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        let (name, group) = (t.new_preset_name.trim().to_string(), t.new_preset_group.trim().to_string());
        if save && !name.is_empty() {
            let path = darkslide_core::paths::presets_dir().join(xmp::sanitize(&group)).join(format!("{}.xmp", xmp::sanitize(&name)));
            if path.exists() && !t.preset_ui.confirm_replace {
                // a second Enter replaces
                t.preset_ui.confirm_replace = true;
                t.preset_ui.focus_name = true;
            } else {
                let mut s = doc.settings.clone();
                // presets never carry image-specific geometry or spot edits
                s.crop = Crop::default();
                s.retouch.clear();
                s.red_eye.clear();
                match xmp::save_preset(&name, &group, &s) {
                    Ok(_) => {
                        t.status = Some(format!("Saved “{name}”"));
                        let key = format!("{}/{name}", if group.is_empty() { "User presets" } else { &group });
                        t.preset_ui.applied = Some(Applied { key: key.clone(), base: doc.settings.clone(), result: doc.settings.clone() });
                        t.preset_ui.scroll_to = Some(key);
                        t.preset_ui.favorites_only = false;
                        t.preset_ui.search.clear();
                    }
                    Err(e) => t.status = Some(format!("Error: {e}")),
                }
                t.new_preset_name.clear();
                t.presets_loaded = false;
                t.preset_ui.creating = false;
            }
        } else if cancel {
            t.preset_ui.creating = false;
        }
    }
    // a line kept even when empty, so the grid doesn't move when it fills
    ui.label(RichText::new(t.status.as_deref().unwrap_or(" ")).size(11.0).color(p.text_muted));
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

    // grouped cards, or one flat Favorites group when filtering
    let q = t.preset_ui.search.trim().to_lowercase();
    let matches = |e: &Entry| q.is_empty() || e.name.to_lowercase().contains(&q) || e.group.to_lowercase().contains(&q);
    let mut groups: Vec<(String, Vec<usize>)> = vec![];
    if t.preset_ui.favorites_only {
        let favs: Vec<usize> =
            (0..t.preset_ui.entries.len()).filter(|&i| t.preset_ui.favorites.contains(&t.preset_ui.entries[i].key()) && matches(&t.preset_ui.entries[i])).collect();
        if !favs.is_empty() {
            groups.push(("Favorites".into(), favs));
        }
    } else {
        for (i, e) in t.preset_ui.entries.iter().enumerate() {
            if !matches(e) {
                continue;
            }
            match groups.iter_mut().find(|(g, _)| *g == e.group) {
                Some((_, v)) => v.push(i),
                None => groups.push((e.group.clone(), vec![i])),
            }
        }
    }
    if groups.is_empty() {
        ui.add_space(20.0);
        let msg = if t.preset_ui.favorites_only && q.is_empty() { "Star a preset to add it to your favorites" } else { "No presets match your search" };
        ui.vertical_centered(|ui| ui.label(RichText::new(msg).color(p.text_muted)));
    }

    let mut apply: Option<usize> = None;
    let mut fav_toggle: Option<String> = None;
    let mut hovered_card: Option<usize> = None;
    let col_gap = 8.0;
    for (g, idx) in &groups {
        let id = ui.make_persistent_id(("preset_group", g));
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
        if t.preset_ui.scroll_to.as_ref().is_some_and(|k| idx.iter().any(|&i| t.preset_ui.entries[i].key() == *k)) {
            state.set_open(true);
        }
        // group header: chevron, name, count
        let (hr, hresp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
        if hresp.hovered() {
            ui.painter().rect_filled(hr, 4.0, p.header_hover);
        }
        let chev = if state.is_open() { ic::CARET_DOWN } else { ic::CARET_RIGHT };
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
                    if t.preset_ui.scroll_to.as_deref() == Some(key.as_str()) {
                        resp.scroll_to_me(Some(egui::Align::Center));
                        t.preset_ui.scroll_to = None;
                    }
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
                    let selected = t.preset_ui.applied.as_ref().is_some_and(|a| a.key == key);
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
                    if resp.hovered() || star_resp.hovered() {
                        hovered_card = Some(i);
                    }
                    if star_resp.on_hover_text(if fav { "Remove from favorites" } else { "Add to favorites" }).clicked() {
                        fav_toggle = Some(key.clone());
                    } else if resp.clicked() {
                        apply = Some(i);
                    } else if resp.hovered() && t.preset_ui.hover_block != Some(i) && !t.preset_ui.block_hovered {
                        t.hover_preview = Some(e.apply(&doc.settings, t.preset_amount));
                    }
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand).widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &e.name));
                }
            });
            ui.add_space(6.0);
        }
        ui.add_space(4.0);
    }
    // a card stays quiet until the pointer leaves it
    if std::mem::take(&mut t.preset_ui.block_hovered) && hovered_card.is_some() {
        t.preset_ui.hover_block = hovered_card;
    }
    if t.preset_ui.hover_block != hovered_card {
        t.preset_ui.hover_block = None;
    }
    if let Some(k) = fav_toggle {
        t.preset_ui.toggle_favorite(k);
    }
    if let Some(i) = apply {
        let e = &t.preset_ui.entries[i];
        let base = doc.settings.clone();
        doc.settings = e.apply(&base, t.preset_amount);
        t.preset_ui.applied = Some(Applied { key: e.key(), base, result: doc.settings.clone() });
        t.preset_ui.seen = Some(doc.settings.clone());
        t.preset_ui.hover_block = Some(i);
        t.hover_preview = None;
        t.status = Some(format!("Applied “{}” — Ctrl+Z to undo", e.name));
    }
    if t.presets.is_empty() && q.is_empty() && !t.preset_ui.favorites_only {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("Add your own .xmp presets with Import, or put them in {}", darkslide_core::paths::presets_dir().display()))
                .size(11.0)
                .color(p.text_muted),
        );
    }
}
