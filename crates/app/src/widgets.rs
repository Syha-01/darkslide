//! ACR-style widgets: labelled gradient sliders, curve editor, colour wheel,
//! histogram, vectorscope.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use darkslide_core::analysis::Histogram;
use darkslide_core::params::CurvePoints;
use darkslide_core::pipeline::curves::Spline;

pub const ACCENT: Color32 = Color32::from_rgb(0x2d, 0x8c, 0xeb);

/// Set by sliders while a drag is in progress so renders can go fast.
pub fn mark_interacting(ui: &egui::Ui) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("ds_interacting"), true));
}

pub fn take_interacting(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("ds_interacting")).unwrap_or(false))
}

pub struct SliderSpec<'a> {
    pub label: &'a str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub decimals: usize,
    pub gradient: Option<&'a [Color32]>,
    pub suffix: &'a str,
    /// Logarithmic track (temperature).
    pub log: bool,
}

impl<'a> SliderSpec<'a> {
    pub fn new(label: &'a str, min: f32, max: f32, default: f32) -> Self {
        Self { label, min, max, default, decimals: 0, gradient: None, suffix: "", log: false }
    }
    pub fn dec(mut self, d: usize) -> Self {
        self.decimals = d;
        self
    }
    pub fn grad(mut self, g: &'a [Color32]) -> Self {
        self.gradient = Some(g);
        self
    }
    pub fn suffix(mut self, s: &'a str) -> Self {
        self.suffix = s;
        self
    }
    pub fn log(mut self) -> Self {
        self.log = true;
        self
    }
}

fn to_t(spec: &SliderSpec, v: f32) -> f32 {
    if spec.log { ((v / spec.min).ln() / (spec.max / spec.min).ln()).clamp(0.0, 1.0) } else { ((v - spec.min) / (spec.max - spec.min)).clamp(0.0, 1.0) }
}

fn from_t(spec: &SliderSpec, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if spec.log { spec.min * (spec.max / spec.min).powf(t) } else { spec.min + (spec.max - spec.min) * t }
}

fn round_to(v: f32, decimals: usize) -> f32 {
    let p = 10f32.powi(decimals as i32);
    (v * p).round() / p
}

/// What the viewer should show while Alt is held on a slider drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AltPreview {
    Highlights,
    Shadows,
    SharpenMask,
}

fn alt_kind(label: &str) -> Option<AltPreview> {
    match label {
        "Exposure" | "Whites" | "Highlights" => Some(AltPreview::Highlights),
        "Blacks" | "Shadows" => Some(AltPreview::Shadows),
        "Masking" => Some(AltPreview::SharpenMask),
        _ => None,
    }
}

pub fn take_auto_slider(ctx: &egui::Context) -> Option<String> {
    let id = egui::Id::new("ds_auto_slider");
    ctx.data_mut(|d| {
        let v = d.get_temp::<String>(id);
        d.remove::<String>(id);
        v
    })
}

pub fn take_alt_preview(ctx: &egui::Context) -> Option<AltPreview> {
    let id = egui::Id::new("ds_alt_preview");
    ctx.data_mut(|d| {
        let v = d.get_temp::<AltPreview>(id);
        d.remove::<AltPreview>(id);
        v
    })
}

/// Label + value on one line, slider track below. Returns true if changed.
pub fn slider(ui: &mut egui::Ui, value: &mut f32, spec: SliderSpec) -> bool {
    let mut changed = false;
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_width(width);
        let lab = ui.add(egui::Label::new(egui::RichText::new(spec.label).size(12.5)).sense(Sense::click()));
        if lab.double_clicked() && ui.input(|i| i.modifiers.shift) {
            // shift + double-click: auto for this slider only (ACR)
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("ds_auto_slider"), spec.label.to_string()));
        } else if lab.double_clicked() && *value != spec.default {
            *value = spec.default;
            changed = true;
        }
        lab.on_hover_text("Double-click to reset");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut v = *value;
            let speed = if spec.log { (v * 0.002).max(1.0) } else { (spec.max - spec.min) / 400.0 };
            let dv = egui::DragValue::new(&mut v).range(spec.min..=spec.max).speed(speed).fixed_decimals(spec.decimals).custom_formatter(|n, _| {
                let s = format!("{:.*}", spec.decimals, n);
                let s = if spec.min < 0.0 && n > 0.0 { format!("+{s}") } else { s };
                format!("{s}{}", spec.suffix)
            });
            let r = ui.add_sized([56.0, 18.0], dv);
            if r.dragged() {
                mark_interacting(ui);
            }
            if r.changed() && v != *value {
                *value = round_to(v, spec.decimals);
                changed = true;
            }
        });
    });
    // track
    let h = 14.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 10.0, 3.0));
    let painter = ui.painter_at(rect.expand(2.0));
    match spec.gradient {
        Some(stops) if stops.len() >= 2 => {
            let mut mesh = egui::Mesh::default();
            let n = stops.len();
            for (i, c) in stops.iter().enumerate() {
                let x = track.left() + track.width() * i as f32 / (n - 1) as f32;
                mesh.colored_vertex(pos2(x, track.top() - 1.0), *c);
                mesh.colored_vertex(pos2(x, track.bottom() + 1.0), *c);
                if i > 0 {
                    let b = (i * 2) as u32;
                    mesh.add_triangle(b - 2, b - 1, b);
                    mesh.add_triangle(b - 1, b, b + 1);
                }
            }
            painter.add(egui::Shape::mesh(mesh));
        }
        _ => {
            painter.rect_filled(track, 1.5, ui.visuals().widgets.inactive.bg_fill.gamma_multiply(1.6));
            // fill from the default to the current value
            let t0 = to_t(&spec, spec.default);
            let t1 = to_t(&spec, *value);
            let x0 = track.left() + track.width() * t0.min(t1);
            let x1 = track.left() + track.width() * t0.max(t1);
            painter.rect_filled(Rect::from_x_y_ranges(x0..=x1, track.y_range()), 1.5, ACCENT.gamma_multiply(0.7));
        }
    }
    let t = to_t(&spec, *value);
    let hx = track.left() + track.width() * t;
    let handle_col = if resp.hovered() || resp.dragged() { Color32::WHITE } else { Color32::from_gray(210) };
    painter.circle_filled(pos2(hx, rect.center().y), 5.5, handle_col);
    painter.circle_stroke(pos2(hx, rect.center().y), 5.5, Stroke::new(1.0, Color32::from_gray(60)));
    if resp.double_clicked() {
        if *value != spec.default {
            *value = spec.default;
            changed = true;
        }
    } else if resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let fine = ui.input(|i| i.modifiers.shift);
            let nt = if fine && resp.dragged() {
                let d = resp.drag_delta().x / track.width() * 0.1;
                to_t(&spec, *value) + d
            } else {
                (p.x - track.left()) / track.width()
            };
            let mut nv = round_to(from_t(&spec, nt), spec.decimals);
            if spec.log {
                nv = nv.round();
            }
            if nv != *value {
                *value = nv;
                changed = true;
            }
        }
        if resp.dragged() {
            mark_interacting(ui);
            if ui.input(|i| i.modifiers.alt) {
                if let Some(k) = alt_kind(spec.label) {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("ds_alt_preview"), k));
                }
            }
        }
    }
    ui.add_space(2.0);
    changed
}

pub fn grad_temp() -> Vec<Color32> {
    vec![Color32::from_rgb(70, 110, 230), Color32::from_rgb(200, 200, 200), Color32::from_rgb(235, 200, 60)]
}
pub fn grad_tint() -> Vec<Color32> {
    vec![Color32::from_rgb(60, 190, 60), Color32::from_rgb(200, 200, 200), Color32::from_rgb(210, 60, 210)]
}
pub fn grad_bw() -> Vec<Color32> {
    vec![Color32::from_gray(20), Color32::from_gray(235)]
}

pub fn hue_color(h: f32, s: f32, v: f32) -> Color32 {
    let c = darkslide_core::color::hsv_to_rgb([h, s, v]);
    Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)
}

pub fn grad_hue_full() -> Vec<Color32> {
    (0..=12).map(|i| hue_color(i as f32 * 30.0, 0.8, 0.9)).collect()
}

/// Gradient for a mixer band's hue slider: neighbours on each side.
pub fn grad_band_hue(center: f32) -> Vec<Color32> {
    vec![hue_color(center - 30.0, 0.8, 0.9), hue_color(center, 0.8, 0.9), hue_color(center + 30.0, 0.8, 0.9)]
}
pub fn grad_band_sat(center: f32) -> Vec<Color32> {
    vec![hue_color(center, 0.0, 0.7), hue_color(center, 0.9, 0.9)]
}
pub fn grad_band_lum(center: f32) -> Vec<Color32> {
    vec![hue_color(center, 0.8, 0.15), hue_color(center, 0.6, 0.95)]
}

/// Collapsible section with an enable toggle (ACR's eye icon).
pub fn section<R>(ui: &mut egui::Ui, title: &str, enabled: Option<&mut bool>, default_open: bool, body: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let id = ui.make_persistent_id(("section", title));
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    let mut toggled = false;
    let header = ui.horizontal(|ui| {
        let open = state.is_open();
        let arrow = if open { "⏷" } else { "⏵" };
        let r = ui.add(egui::Label::new(egui::RichText::new(format!("{arrow} {title}")).strong().size(13.5)).sense(Sense::click()));
        if r.clicked() {
            state.toggle(ui);
        }
        if let Some(en) = enabled {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let icon = if *en { "👁" } else { "⊘" };
                if ui.add(egui::Button::new(icon).frame(false)).on_hover_text("Toggle panel").clicked() {
                    *en = !*en;
                    toggled = true;
                }
            });
        }
    });
    let _ = header;
    let r = state.show_body_unindented(ui, |ui| {
        ui.add_space(2.0);
        body(ui)
    });
    ui.separator();
    if toggled {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("ds_panel_toggled"), true));
    }
    r.map(|r| r.inner)
}

// ---------------------------------------------------------------------------
// Curve editor

pub struct CurveEdit<'a> {
    pub points: &'a mut CurvePoints,
    pub color: Color32,
    pub hist: Option<&'a [u32]>,
    pub id: egui::Id,
}

/// Point curve editor (0..255). Click adds a point, drag moves, drag out or
/// double-click removes. Returns true if changed.
pub fn curve_editor(ui: &mut egui::Ui, ce: CurveEdit, overlay_luts: &[(&darkslide_core::pipeline::curves::Lut, Color32)]) -> bool {
    let size = ui.available_width().min(300.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(28));
    if let Some(h) = ce.hist {
        let max = h.iter().skip(1).take(254).copied().max().unwrap_or(1).max(1) as f32;
        for (i, &v) in h.iter().enumerate() {
            let x = rect.left() + rect.width() * i as f32 / 255.0;
            let hh = (v as f32 / max).sqrt().min(1.0) * rect.height() * 0.9;
            painter.line_segment([pos2(x, rect.bottom()), pos2(x, rect.bottom() - hh)], Stroke::new(rect.width() / 256.0 + 0.5, Color32::from_gray(60)));
        }
    }
    for i in 1..4 {
        let f = i as f32 / 4.0;
        let g = Stroke::new(1.0, Color32::from_gray(55));
        painter.line_segment([pos2(rect.left() + rect.width() * f, rect.top()), pos2(rect.left() + rect.width() * f, rect.bottom())], g);
        painter.line_segment([pos2(rect.left(), rect.top() + rect.height() * f), pos2(rect.right(), rect.top() + rect.height() * f)], g);
    }
    painter.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.0, Color32::from_gray(70)));
    let to_screen = |p: [f32; 2]| pos2(rect.left() + p[0] / 255.0 * rect.width(), rect.bottom() - p[1] / 255.0 * rect.height());
    let from_screen =
        |p: Pos2| [((p.x - rect.left()) / rect.width() * 255.0).clamp(0.0, 255.0), ((rect.bottom() - p.y) / rect.height() * 255.0).clamp(0.0, 255.0)];

    for (lut, col) in overlay_luts {
        let pts: Vec<Pos2> = (0..=64)
            .map(|i| {
                let x = i as f32 / 64.0;
                to_screen([x * 255.0, lut.eval(x).clamp(0.0, 1.0) * 255.0])
            })
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.0, col.gamma_multiply(0.5))));
    }

    let mut changed = false;
    let drag_id = ce.id.with("drag");
    let mut dragging: Option<usize> = ui.ctx().data(|d| d.get_temp(drag_id));
    let pts = ce.points;
    if resp.drag_started() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let near = pts
                .iter()
                .enumerate()
                .map(|(i, q)| (i, to_screen(*q).distance(p)))
                .filter(|(_, d)| *d < 8.0)
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .map(|(i, _)| i);
            match near {
                Some(i) => dragging = Some(i),
                None => {
                    let np = from_screen(p);
                    pts.push(np);
                    pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
                    dragging = pts.iter().position(|q| *q == np);
                    changed = true;
                }
            }
        }
    }
    if resp.double_clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            if let Some(i) = pts.iter().position(|q| to_screen(*q).distance(p) < 8.0) {
                if i != 0 && i != pts.len() - 1 {
                    pts.remove(i);
                    changed = true;
                    dragging = None;
                }
            }
        }
    }
    if resp.dragged() {
        mark_interacting(ui);
        if let (Some(i), Some(p)) = (dragging, resp.interact_pointer_pos()) {
            if i < pts.len() {
                let mut np = from_screen(p);
                let n = pts.len();
                // keep x order; endpoints may slide vertically and inward
                let lo = if i > 0 { pts[i - 1][0] + 1.0 } else { 0.0 };
                let hi = if i + 1 < n { pts[i + 1][0] - 1.0 } else { 255.0 };
                np[0] = np[0].clamp(lo, hi);
                // drag far outside removes interior points
                let outside = !rect.expand(20.0).contains(p);
                if outside && i != 0 && i != n - 1 {
                    pts.remove(i);
                    dragging = None;
                } else {
                    pts[i] = [np[0].round(), np[1].round()];
                }
                changed = true;
            }
        }
    }
    if resp.drag_stopped() {
        dragging = None;
    }
    ui.ctx().data_mut(|d| match dragging {
        Some(i) => {
            d.insert_temp(drag_id, i);
        }
        None => d.remove::<usize>(drag_id),
    });

    // the curve
    if pts.len() >= 2 {
        let norm: Vec<[f32; 2]> = pts.iter().map(|p| [p[0] / 255.0, p[1] / 255.0]).collect();
        let sp = Spline::new(&norm);
        let (x0, x1) = (norm[0][0], norm[norm.len() - 1][0]);
        let line: Vec<Pos2> = (0..=128)
            .map(|i| {
                let x = i as f32 / 128.0;
                let y = if x <= x0 {
                    norm[0][1]
                } else if x >= x1 {
                    norm[norm.len() - 1][1]
                } else {
                    sp.eval(x).clamp(0.0, 1.0)
                };
                to_screen([x * 255.0, y * 255.0])
            })
            .collect();
        painter.add(egui::Shape::line(line, Stroke::new(1.6, ce.color)));
    }
    for (i, p) in pts.iter().enumerate() {
        let sp = to_screen(*p);
        let active = dragging == Some(i);
        painter.circle_filled(sp, if active { 5.0 } else { 4.0 }, if active { Color32::WHITE } else { ce.color });
        painter.circle_stroke(sp, 4.5, Stroke::new(1.0, Color32::BLACK));
    }
    if let Some(hp) = resp.hover_pos() {
        let v = from_screen(hp);
        painter.text(
            rect.left_top() + vec2(4.0, 4.0),
            egui::Align2::LEFT_TOP,
            format!("{:.0} / {:.0}", v[0], v[1]),
            egui::FontId::monospace(10.0),
            Color32::GRAY,
        );
    }
    changed
}

// ---------------------------------------------------------------------------
// Colour wheel

/// Hue (0..360) / saturation (0..100) wheel. Returns true if changed.
pub fn color_wheel(ui: &mut egui::Ui, hue: &mut f32, sat: &mut f32, size: f32, id: egui::Id) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let c = rect.center();
    let r = size / 2.0 - 3.0;
    let mut mesh = egui::Mesh::default();
    let seg = 72;
    let rings = 4;
    mesh.colored_vertex(c, Color32::from_gray(128));
    for ring in 1..=rings {
        let rr = r * ring as f32 / rings as f32;
        for i in 0..seg {
            let a = i as f32 / seg as f32 * std::f32::consts::TAU;
            let h = a.to_degrees();
            mesh.colored_vertex(c + vec2(a.cos(), -a.sin()) * rr, hue_color(h, ring as f32 / rings as f32 * 0.9, 0.8));
        }
    }
    for i in 0..seg as u32 {
        let n = (i + 1) % seg as u32;
        mesh.add_triangle(0, 1 + i, 1 + n);
    }
    for ring in 1..rings as u32 {
        let a0 = 1 + (ring - 1) * seg as u32;
        let a1 = 1 + ring * seg as u32;
        for i in 0..seg as u32 {
            let n = (i + 1) % seg as u32;
            mesh.add_triangle(a0 + i, a1 + i, a1 + n);
            mesh.add_triangle(a0 + i, a1 + n, a0 + n);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.circle_stroke(c, r, Stroke::new(1.0, Color32::from_gray(40)));
    let mut changed = false;
    if resp.double_clicked() {
        *hue = 0.0;
        *sat = 0.0;
        changed = true;
    } else if resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let d = p - c;
            let fine = ui.input(|i| i.modifiers.shift);
            let mut s = (d.length() / r * 100.0).clamp(0.0, 100.0);
            let mut h = (-d.y).atan2(d.x).to_degrees().rem_euclid(360.0);
            if fine {
                // shift: lock saturation, change hue only
                s = *sat;
            }
            if ui.input(|i| i.modifiers.command) {
                h = *hue;
            }
            *hue = h.round();
            *sat = s.round();
            changed = true;
            mark_interacting(ui);
        }
    }
    let a = hue.to_radians();
    let hp = c + vec2(a.cos(), -a.sin()) * (*sat / 100.0 * r);
    painter.circle_filled(hp, 5.0, hue_color(*hue, *sat / 100.0, 1.0));
    painter.circle_stroke(hp, 5.0, Stroke::new(1.5, Color32::WHITE));
    let _ = id;
    changed
}

// ---------------------------------------------------------------------------
// Histogram

pub struct HistResp {
    pub toggle_shadows: bool,
    pub toggle_highlights: bool,
}

pub fn histogram(ui: &mut egui::Ui, h: Option<&Histogram>, show_s: bool, show_h: bool) -> HistResp {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 110.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_gray(22));
    let mut out = HistResp { toggle_shadows: false, toggle_highlights: false };
    if let Some(h) = h {
        let max = [&h.r, &h.g, &h.b].iter().flat_map(|c| c[1..255].iter()).copied().max().unwrap_or(1).max(1) as f32;
        let chans = [
            (&h.r, Color32::from_rgba_unmultiplied(230, 60, 60, 110)),
            (&h.g, Color32::from_rgba_unmultiplied(60, 210, 60, 110)),
            (&h.b, Color32::from_rgba_unmultiplied(70, 110, 255, 110)),
        ];
        for (data, col) in chans {
            let mut mesh = egui::Mesh::default();
            for i in 0..256 {
                let x = rect.left() + rect.width() * i as f32 / 255.0;
                let v = (data[i] as f32 / max).min(1.0);
                mesh.colored_vertex(pos2(x, rect.bottom()), col);
                mesh.colored_vertex(pos2(x, rect.bottom() - v * (rect.height() - 6.0)), col);
                if i > 0 {
                    let b = (i * 2) as u32;
                    mesh.add_triangle(b - 2, b - 1, b);
                    mesh.add_triangle(b - 1, b, b + 1);
                }
            }
            painter.add(egui::Shape::mesh(mesh));
        }
        // clipping indicators
        let tri = |left: bool, active: bool, clipped: bool| {
            let c = if active {
                Color32::WHITE
            } else if clipped {
                Color32::from_gray(170)
            } else {
                Color32::from_gray(70)
            };
            let (x, dir) = if left { (rect.left() + 4.0, 1.0) } else { (rect.right() - 4.0, -1.0) };
            let y = rect.top() + 4.0;
            painter.add(egui::Shape::convex_polygon(vec![pos2(x, y), pos2(x + dir * 10.0, y), pos2(x, y + 10.0)], c, Stroke::NONE));
        };
        tri(true, show_s, h.clip_shadows);
        tri(false, show_h, h.clip_highlights);
    }
    let resp = ui.interact(rect, ui.id().with("histclick"), Sense::click());
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            if p.y < rect.top() + 18.0 {
                if p.x < rect.left() + 18.0 {
                    out.toggle_shadows = true;
                } else if p.x > rect.right() - 18.0 {
                    out.toggle_highlights = true;
                }
            }
        }
    }
    out
}

pub fn vectorscope(ui: &mut egui::Ui, grid: &[u32], size: usize, skin_line: bool) {
    let w = ui.available_width().min(220.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, w), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_gray(18));
    painter.circle_stroke(rect.center(), w * 0.45, Stroke::new(1.0, Color32::from_gray(50)));
    let max = grid.iter().copied().max().unwrap_or(1).max(1) as f32;
    let cell = w / size as f32;
    for y in 0..size {
        for x in 0..size {
            let v = grid[y * size + x];
            if v == 0 {
                continue;
            }
            let t = (v as f32 / max).sqrt();
            // colour by position (approximate hue of that Cb/Cr)
            let cb = x as f32 / size as f32 - 0.5;
            let cr = 0.5 - y as f32 / size as f32;
            let h = cr.atan2(cb).to_degrees();
            let col = hue_color(h + 100.0, 0.5, (0.3 + t * 0.7).min(1.0));
            painter.rect_filled(Rect::from_min_size(pos2(rect.left() + x as f32 * cell, rect.top() + y as f32 * cell), Vec2::splat(cell.max(1.0))), 0.0, col);
        }
    }
    if skin_line {
        let a = darkslide_core::analysis::SKIN_TONE_ANGLE.to_radians();
        let c = rect.center();
        painter.line_segment([c, c + vec2(a.cos(), -a.sin()) * w * 0.48], Stroke::new(1.0, Color32::from_rgb(230, 170, 130)));
    }
    painter.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_gray(50)), StrokeKind::Inside);
}
