//! ACR-style widgets: labelled gradient sliders, curve editor, colour wheel,
//! histogram, vectorscope.

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use darkslide_core::analysis::Histogram;
use darkslide_core::params::CurvePoints;
use darkslide_core::pipeline::curves::Spline;

pub const ACCENT: Color32 = Color32::from_rgb(20, 115, 230); // Adobe Blue #1473e6
#[allow(dead_code)]
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(13, 102, 208);
#[allow(dead_code)]
pub const ACCENT_ACTIVE: Color32 = Color32::from_rgb(9, 90, 186);

#[allow(dead_code)]
pub const BG_APP: Color32 = Color32::from_rgb(22, 22, 22);
#[allow(dead_code)]
pub const BG_PANEL: Color32 = Color32::from_rgb(34, 34, 34);
#[allow(dead_code)]
pub const BG_HEADER: Color32 = Color32::from_rgb(28, 28, 28);
#[allow(dead_code)]
pub const BG_TOOLBAR: Color32 = Color32::from_rgb(25, 25, 25);
#[allow(dead_code)]
pub const BG_INPUT: Color32 = Color32::from_rgb(18, 18, 18);

#[allow(dead_code)]
pub const BORDER_DARK: Color32 = Color32::from_rgb(46, 46, 46);
#[allow(dead_code)]
pub const BORDER_LIGHT: Color32 = Color32::from_rgb(60, 60, 60);

pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(225, 225, 225);
#[allow(dead_code)]
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(155, 155, 155);
#[allow(dead_code)]
pub const TEXT_MUTED: Color32 = Color32::from_rgb(110, 110, 110);

use crate::tool_panels::Tool;

pub fn draw_tool_icon(painter: &egui::Painter, tool: Tool, center: Pos2, size: f32, color: Color32) {
    match tool {
        Tool::Edit => {
            let w = size * 0.65;
            let h_spacing = size * 0.22;
            let y0 = center.y - h_spacing;
            let y1 = center.y;
            let y2 = center.y + h_spacing;
            let x_left = center.x - w / 2.0;
            let x_right = center.x + w / 2.0;
            let stroke = Stroke::new(1.3, color);

            painter.line_segment([pos2(x_left, y0), pos2(x_right, y0)], stroke);
            let k1_x = center.x - w * 0.20;
            painter.rect_filled(Rect::from_center_size(pos2(k1_x, y0), vec2(3.5, 6.0)), 1.0, color);

            painter.line_segment([pos2(x_left, y1), pos2(x_right, y1)], stroke);
            let k2_x = center.x + w * 0.22;
            painter.rect_filled(Rect::from_center_size(pos2(k2_x, y1), vec2(3.5, 6.0)), 1.0, color);

            painter.line_segment([pos2(x_left, y2), pos2(x_right, y2)], stroke);
            let k3_x = center.x - w * 0.05;
            painter.rect_filled(Rect::from_center_size(pos2(k3_x, y2), vec2(3.5, 6.0)), 1.0, color);
        }
        Tool::Crop => {
            let s = size * 0.32;
            let stroke = Stroke::new(1.5, color);
            let tl_x = center.x - s;
            let tl_y = center.y - s;
            let br_x = center.x + s;
            let br_y = center.y + s;
            let ext = s * 0.40;
            painter.line_segment([pos2(tl_x, tl_y - ext), pos2(tl_x, br_y)], stroke);
            painter.line_segment([pos2(tl_x - ext, tl_y), pos2(br_x, tl_y)], stroke);
            painter.line_segment([pos2(br_x, tl_y), pos2(br_x, br_y + ext)], stroke);
            painter.line_segment([pos2(tl_x, br_y), pos2(br_x + ext, br_y)], stroke);
        }
        Tool::Remove => {
            let a = 40.0f32.to_radians();
            let cos_a = a.cos();
            let sin_a = a.sin();
            let len = size * 0.38;
            let width = size * 0.17;
            let p1 = center - vec2(len * cos_a, len * sin_a);
            let p2 = center + vec2(len * cos_a, len * sin_a);
            painter.line_segment([p1, p2], Stroke::new(width * 2.0, color.gamma_multiply(0.25)));
            painter.add(egui::Shape::line(
                vec![
                    p1 + vec2(-width * sin_a, width * cos_a),
                    p2 + vec2(-width * sin_a, width * cos_a),
                    p2 + vec2(width * sin_a, -width * cos_a),
                    p1 + vec2(width * sin_a, -width * cos_a),
                    p1 + vec2(-width * sin_a, width * cos_a),
                ],
                Stroke::new(1.3, color),
            ));
            painter.circle_filled(center, 1.2, color);
            painter.circle_filled(center - vec2(3.2 * cos_a, 3.2 * sin_a), 1.0, color);
            painter.circle_filled(center + vec2(3.2 * cos_a, 3.2 * sin_a), 1.0, color);
        }
        Tool::Masking => {
            let r = size * 0.35;
            let segs = 12;
            for i in 0..segs {
                if i % 2 == 0 {
                    let a1 = i as f32 / segs as f32 * std::f32::consts::TAU;
                    let a2 = (i as f32 + 0.65) / segs as f32 * std::f32::consts::TAU;
                    painter.line_segment([center + vec2(a1.cos(), a1.sin()) * r, center + vec2(a2.cos(), a2.sin()) * r], Stroke::new(1.4, color));
                }
            }
            painter.circle_filled(center, r * 0.48, color);
        }
        Tool::RedEye => {
            let w = size * 0.40;
            let h = size * 0.22;
            let pts_top = vec![
                center + vec2(-w, 0.0),
                center + vec2(-w * 0.5, -h),
                center + vec2(w * 0.5, -h),
                center + vec2(w, 0.0),
            ];
            let pts_bot = vec![
                center + vec2(w, 0.0),
                center + vec2(w * 0.5, h),
                center + vec2(-w * 0.5, h),
                center + vec2(-w, 0.0),
            ];
            painter.add(egui::Shape::line(pts_top, Stroke::new(1.3, color)));
            painter.add(egui::Shape::line(pts_bot, Stroke::new(1.3, color)));
            painter.circle_stroke(center, h * 0.72, Stroke::new(1.2, color));
            painter.circle_filled(center, h * 0.35, color);
        }
        Tool::Snapshots => {
            let w = size * 0.35;
            let h = size * 0.24;
            let rect = Rect::from_center_size(center + vec2(0.0, 1.0), vec2(w * 2.0, h * 2.0));
            painter.rect_stroke(rect, 2.0, Stroke::new(1.3, color), StrokeKind::Inside);
            painter.circle_stroke(rect.center(), h * 0.55, Stroke::new(1.2, color));
            let top_w = w * 0.42;
            let top_h = 3.0;
            let top_r = Rect::from_min_max(pos2(center.x - top_w, rect.top() - top_h), pos2(center.x + top_w, rect.top()));
            painter.rect_filled(top_r, 1.0, color);
        }
        Tool::Presets => {
            let r = size * 0.22;
            let offset = size * 0.12;
            painter.circle_stroke(center - vec2(offset, 0.0), r, Stroke::new(1.3, color));
            painter.circle_stroke(center + vec2(offset, 0.0), r, Stroke::new(1.3, color));
        }
    }
}

#[allow(dead_code)]
pub fn draw_eyeball(painter: &egui::Painter, center: Pos2, size: f32, open: bool, color: Color32) {
    let w = size * 0.45;
    let h = size * 0.26;
    let pts_top = vec![
        center + vec2(-w, 0.0),
        center + vec2(-w * 0.5, -h),
        center + vec2(w * 0.5, -h),
        center + vec2(w, 0.0),
    ];
    let pts_bot = vec![
        center + vec2(w, 0.0),
        center + vec2(w * 0.5, h),
        center + vec2(-w * 0.5, h),
        center + vec2(-w, 0.0),
    ];
    let stroke = Stroke::new(1.2, color);
    painter.add(egui::Shape::line(pts_top, stroke));
    painter.add(egui::Shape::line(pts_bot, stroke));
    if open {
        painter.circle_filled(center, h * 0.55, color);
    } else {
        painter.line_segment([center + vec2(-w * 0.85, -h * 1.1), center + vec2(w * 0.85, h * 1.1)], Stroke::new(1.3, color));
    }
}

pub fn draw_icon_eyedropper(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let s = size * 0.38;
    let a = (-45.0f32).to_radians();
    let dir = vec2(a.cos(), a.sin());
    let normal = vec2(-dir.y, dir.x);
    let tip = center + dir * s;
    let bulb = center - dir * s;
    let barrel1 = tip - dir * (s * 0.4) + normal * 2.0;
    let barrel2 = tip - dir * (s * 0.4) - normal * 2.0;
    let bulb1 = bulb + normal * 2.8;
    let bulb2 = bulb - normal * 2.8;
    painter.line_segment([tip, barrel1], Stroke::new(1.2, color));
    painter.line_segment([tip, barrel2], Stroke::new(1.2, color));
    painter.line_segment([barrel1, bulb1], Stroke::new(1.2, color));
    painter.line_segment([barrel2, bulb2], Stroke::new(1.2, color));
    painter.line_segment([bulb1, bulb2], Stroke::new(1.2, color));
    painter.circle_filled(tip + dir * 2.5, 1.2, color);
}

pub fn eyedropper_button(ui: &mut egui::Ui, active: bool, tooltip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 20.0), Sense::click());
    let painter = ui.painter_at(rect);
    if active {
        painter.rect_filled(rect, 3.0, ACCENT);
        painter.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_rgb(50, 140, 245)), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        painter.rect_filled(rect, 3.0, Color32::from_rgb(48, 48, 48));
        painter.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_rgb(68, 68, 68)), egui::StrokeKind::Inside);
    } else {
        painter.rect_filled(rect, 3.0, Color32::from_rgb(34, 34, 34));
        painter.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_rgb(48, 48, 48)), egui::StrokeKind::Inside);
    }
    let icon_col = if active {
        Color32::WHITE
    } else if resp.hovered() {
        Color32::from_rgb(240, 240, 240)
    } else {
        Color32::from_rgb(175, 175, 175)
    };
    draw_icon_eyedropper(&painter, rect.center(), 14.0, icon_col);
    resp.on_hover_text(tooltip).clicked()
}

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
/// Single-line Camera Raw slider: [Label] [===== Track =====] [Value]
pub fn slider(ui: &mut egui::Ui, value: &mut f32, spec: SliderSpec) -> bool {
    let mut changed = false;
    let total_w = ui.available_width();
    let row_h = 21.0;
    let (row_rect, _) = ui.allocate_exact_size(vec2(total_w, row_h), Sense::hover());
    
    let label_w = 82.0;
    let val_w = 44.0;
    let gap = 6.0;
    let track_w = (total_w - label_w - val_w - gap * 2.0).max(40.0);

    let label_rect = Rect::from_min_size(row_rect.left_top(), vec2(label_w, row_h));
    let track_rect = Rect::from_min_size(pos2(label_rect.right() + gap, row_rect.top()), vec2(track_w, row_h));
    let val_rect = Rect::from_min_size(pos2(track_rect.right() + gap, row_rect.top() + 1.0), vec2(val_w, row_h - 2.0));

    // 1. Label
    let lab = egui::Label::new(egui::RichText::new(spec.label).size(12.0).color(TEXT_PRIMARY)).sense(Sense::click());
    let lab_resp = ui.put(label_rect, lab);
    if lab_resp.double_clicked() && ui.input(|i| i.modifiers.shift) {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("ds_auto_slider"), spec.label.to_string()));
    } else if lab_resp.double_clicked() && *value != spec.default {
        *value = spec.default;
        changed = true;
    }
    lab_resp.on_hover_text("Double-click to reset, Shift+double-click for Auto");

    // 2. Track & Handle
    let track_line = Rect::from_center_size(track_rect.center(), vec2(track_rect.width(), 3.0));
    let painter = ui.painter_at(track_rect.expand(2.0));
    let track_resp = ui.interact(track_rect, ui.make_persistent_id(("slider_trk", spec.label)), Sense::click_and_drag());

    match spec.gradient {
        Some(stops) if stops.len() >= 2 => {
            let mut mesh = egui::Mesh::default();
            let n = stops.len();
            for (i, c) in stops.iter().enumerate() {
                let x = track_line.left() + track_line.width() * i as f32 / (n - 1) as f32;
                mesh.colored_vertex(pos2(x, track_line.top()), *c);
                mesh.colored_vertex(pos2(x, track_line.bottom()), *c);
                if i > 0 {
                    let b = (i * 2) as u32;
                    mesh.add_triangle(b - 2, b - 1, b);
                    mesh.add_triangle(b - 1, b, b + 1);
                }
            }
            painter.add(egui::Shape::mesh(mesh));
        }
        _ => {
            painter.rect_filled(track_line, 1.5, Color32::from_rgb(22, 22, 22));
            let t0 = to_t(&spec, spec.default);
            let t1 = to_t(&spec, *value);
            let x0 = track_line.left() + track_line.width() * t0.min(t1);
            let x1 = track_line.left() + track_line.width() * t0.max(t1);
            painter.rect_filled(Rect::from_x_y_ranges(x0..=x1, track_line.y_range()), 1.5, ACCENT.gamma_multiply(0.9));
            if spec.min < spec.default && spec.max > spec.default {
                let cx = track_line.left() + track_line.width() * t0;
                painter.line_segment([pos2(cx, track_line.top() - 2.0), pos2(cx, track_line.bottom() + 2.0)], Stroke::new(1.0, Color32::from_rgb(70, 70, 70)));
            }
        }
    }

    let t = to_t(&spec, *value);
    let hx = track_line.left() + track_line.width() * t;
    let handle_col = if track_resp.hovered() || track_resp.dragged() { Color32::WHITE } else { Color32::from_rgb(230, 230, 230) };
    painter.circle_filled(pos2(hx, track_rect.center().y), 4.5, handle_col);
    painter.circle_stroke(pos2(hx, track_rect.center().y), 4.5, Stroke::new(1.0, Color32::from_rgb(24, 24, 24)));

    if track_resp.double_clicked() {
        if *value != spec.default {
            *value = spec.default;
            changed = true;
        }
    } else if track_resp.dragged() || track_resp.clicked() {
        if let Some(p) = track_resp.interact_pointer_pos() {
            let fine = ui.input(|i| i.modifiers.shift);
            let nt = if fine && track_resp.dragged() {
                let d = track_resp.drag_delta().x / track_line.width() * 0.1;
                to_t(&spec, *value) + d
            } else {
                (p.x - track_line.left()) / track_line.width()
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
        if track_resp.dragged() {
            mark_interacting(ui);
            if ui.input(|i| i.modifiers.alt) {
                if let Some(k) = alt_kind(spec.label) {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("ds_alt_preview"), k));
                }
            }
        }
    }

    // 3. Value Box
    let mut v = *value;
    let speed = if spec.log { (v * 0.002).max(1.0) } else { (spec.max - spec.min) / 400.0 };
    let dv = egui::DragValue::new(&mut v).range(spec.min..=spec.max).speed(speed).fixed_decimals(spec.decimals).custom_formatter(|n, _| {
        let s = format!("{:.*}", spec.decimals, n);
        let s = if spec.min < 0.0 && n > 0.0 { format!("+{s}") } else { s };
        format!("{s}{}", spec.suffix)
    });

    let r = ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_fill = Color32::from_rgb(18, 18, 18);
        ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(42, 42, 42));
        ui.visuals_mut().widgets.hovered.bg_fill = Color32::from_rgb(26, 26, 26);
        ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(60, 60, 60));
        ui.visuals_mut().widgets.active.bg_fill = Color32::from_rgb(14, 14, 14);
        ui.visuals_mut().widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
        ui.put(val_rect, dv)
    }).inner;

    if r.dragged() {
        mark_interacting(ui);
    }
    if r.changed() && v != *value {
        *value = round_to(v, spec.decimals);
        changed = true;
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

pub fn sub_header(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    let total_w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(total_w, 15.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let galley = painter.layout_no_wrap(
        title.to_string(),
        egui::FontId::proportional(10.5),
        Color32::from_rgb(140, 140, 140),
    );
    let text_w = galley.size().x;
    painter.galley(rect.left_top(), galley, Color32::from_rgb(140, 140, 140));
    if text_w + 12.0 < rect.width() {
        painter.line_segment(
            [pos2(rect.left() + text_w + 8.0, rect.center().y), pos2(rect.right(), rect.center().y)],
            Stroke::new(1.0, Color32::from_rgb(42, 42, 42)),
        );
    }
    ui.add_space(3.0);
}

pub fn sub_header_tone(ui: &mut egui::Ui, on_auto: impl FnOnce(), on_default: impl FnOnce()) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("TONE").size(10.5).color(Color32::from_rgb(140, 140, 140)));
        let sep_x = ui.cursor().left() + 4.0;
        let sep_y = ui.cursor().center().y;
        let right_limit = ui.max_rect().right() - 110.0;
        if sep_x < right_limit {
            ui.painter().line_segment([pos2(sep_x, sep_y), pos2(right_limit, sep_y)], Stroke::new(1.0, Color32::from_rgb(42, 42, 42)));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let def_btn = egui::Button::new(RichText::new("Default").size(11.0).color(Color32::from_rgb(205, 205, 205)))
                .fill(Color32::from_rgb(38, 38, 38))
                .stroke(Stroke::new(1.0, Color32::from_rgb(52, 52, 52)))
                .corner_radius(2.0);
            if ui.add(def_btn).on_hover_text("Reset Tone sliders").clicked() {
                on_default();
            }
            let auto_btn = egui::Button::new(RichText::new("Auto").size(11.0).color(Color32::from_rgb(205, 205, 205)))
                .fill(Color32::from_rgb(38, 38, 38))
                .stroke(Stroke::new(1.0, Color32::from_rgb(52, 52, 52)))
                .corner_radius(2.0);
            if ui.add(auto_btn).on_hover_text("Auto tone (Ctrl+U)").clicked() {
                on_auto();
            }
        });
    });
    ui.add_space(3.0);
}

/// Collapsible section with an enable toggle (ACR's eye icon).
pub fn section<R>(ui: &mut egui::Ui, title: &str, enabled: Option<&mut bool>, default_open: bool, body: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let id = ui.make_persistent_id(("section", title));
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    let mut toggled = false;
    let open = state.is_open();

    let full_w = ui.available_width();
    let header_h = 28.0;
    let (header_rect, header_resp) = ui.allocate_exact_size(vec2(full_w, header_h), Sense::click());
    let painter = ui.painter_at(header_rect);

    if header_resp.hovered() {
        painter.rect_filled(header_rect, 2.0, Color32::from_rgb(40, 40, 40));
    } else {
        painter.rect_filled(header_rect, 0.0, Color32::from_rgb(32, 32, 32));
    }
    painter.line_segment(
        [pos2(header_rect.left(), header_rect.bottom()), pos2(header_rect.right(), header_rect.bottom())],
        Stroke::new(1.0, Color32::from_rgb(42, 42, 42)),
    );

    let arrow = if open { "▾" } else { "▸" };
    painter.text(
        pos2(header_rect.left() + 6.0, header_rect.center().y),
        egui::Align2::LEFT_CENTER,
        arrow,
        egui::FontId::proportional(12.0),
        Color32::from_rgb(160, 160, 160),
    );

    let galley = painter.layout_no_wrap(title.to_string(), egui::FontId::proportional(12.5), Color32::from_rgb(225, 225, 225));
    let text_w = galley.size().x;
    let label_rect = Rect::from_min_size(pos2(header_rect.left() + 20.0, header_rect.top()), vec2(text_w + 8.0, header_h));
    let lab = egui::Label::new(RichText::new(title).size(12.5).strong().color(Color32::from_rgb(225, 225, 225)));
    ui.put(label_rect, lab);

    let mut eye_clicked = false;
    if let Some(en) = enabled {
        let eye_rect = Rect::from_center_size(pos2(header_rect.right() - 15.0, header_rect.center().y), vec2(20.0, 20.0));
        let eye_resp = ui.interact(eye_rect, id.with("eye"), Sense::click());
        if eye_resp.hovered() {
            painter.rect_filled(eye_rect, 2.0, Color32::from_rgb(52, 52, 52));
        }
        let eye_col = if *en {
            if eye_resp.hovered() { Color32::WHITE } else { Color32::from_rgb(210, 210, 210) }
        } else {
            Color32::from_rgb(90, 90, 90)
        };
        draw_eyeball(&painter, eye_rect.center(), 14.0, *en, eye_col);
        if eye_resp.on_hover_text("Toggle section on/off").clicked() {
            *en = !*en;
            toggled = true;
            eye_clicked = true;
        }
    }

    if header_resp.clicked() && !eye_clicked {
        state.toggle(ui);
    }

    let r = state.show_body_unindented(ui, |ui| {
        ui.add_space(4.0);
        let out = egui::Frame::new()
            .inner_margin(egui::Margin { left: 4, right: 4, top: 4, bottom: 6 })
            .show(ui, |ui| body(ui))
            .inner;
        ui.add_space(4.0);
        out
    });

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
    let (rect, _) = ui.allocate_exact_size(vec2(w, 112.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(18, 18, 18));
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, Color32::from_rgb(44, 44, 44)), StrokeKind::Inside);
    
    // Internal tonal grid lines
    let grid_stroke = Stroke::new(1.0, Color32::from_rgb(26, 26, 26));
    for f in [0.25f32, 0.50, 0.75] {
        let gx = rect.left() + rect.width() * f;
        painter.line_segment([pos2(gx, rect.top() + 1.0), pos2(gx, rect.bottom() - 1.0)], grid_stroke);
    }
    painter.line_segment([pos2(rect.left() + 1.0, rect.center().y), pos2(rect.right() - 1.0, rect.center().y)], grid_stroke);

    let mut out = HistResp { toggle_shadows: false, toggle_highlights: false };
    if let Some(h) = h {
        let max = [&h.r, &h.g, &h.b].iter().flat_map(|c| c[1..255].iter()).copied().max().unwrap_or(1).max(1) as f32;
        let chans = [
            (&h.r, Color32::from_rgba_unmultiplied(235, 60, 60, 110)),
            (&h.g, Color32::from_rgba_unmultiplied(60, 215, 60, 110)),
            (&h.b, Color32::from_rgba_unmultiplied(65, 130, 255, 110)),
        ];
        for (data, col) in chans {
            let mut mesh = egui::Mesh::default();
            for i in 0..256 {
                let x = rect.left() + rect.width() * i as f32 / 255.0;
                let v = (data[i] as f32 / max).min(1.0);
                mesh.colored_vertex(pos2(x, rect.bottom() - 1.0), col);
                mesh.colored_vertex(pos2(x, rect.bottom() - 1.0 - v * (rect.height() - 8.0)), col);
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
            let btn_r = if left {
                Rect::from_min_size(pos2(rect.left() + 3.0, rect.top() + 3.0), vec2(15.0, 15.0))
            } else {
                Rect::from_min_size(pos2(rect.right() - 18.0, rect.top() + 3.0), vec2(15.0, 15.0))
            };
            let bg = if active {
                if left { Color32::from_rgb(20, 60, 120) } else { Color32::from_rgb(120, 25, 25) }
            } else {
                Color32::from_rgb(28, 28, 28)
            };
            painter.rect_filled(btn_r, 2.0, bg);
            painter.rect_stroke(btn_r, 2.0, Stroke::new(1.0, Color32::from_rgb(50, 50, 50)), StrokeKind::Inside);
            
            let c = if active {
                if left { Color32::from_rgb(80, 170, 255) } else { Color32::from_rgb(255, 80, 80) }
            } else if clipped {
                Color32::from_rgb(190, 190, 190)
            } else {
                Color32::from_rgb(85, 85, 85)
            };
            let (tx, dir) = if left { (btn_r.left() + 3.0, 1.0) } else { (btn_r.right() - 3.0, -1.0) };
            let ty = btn_r.top() + 3.0;
            painter.add(egui::Shape::convex_polygon(vec![pos2(tx, ty), pos2(tx + dir * 9.0, ty), pos2(tx, ty + 9.0)], c, Stroke::NONE));
        };
        tri(true, show_s, h.clip_shadows);
        tri(false, show_h, h.clip_highlights);
    }
    let resp = ui.interact(rect, ui.id().with("histclick"), Sense::click());
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            if p.y < rect.top() + 20.0 {
                if p.x < rect.left() + 22.0 {
                    out.toggle_shadows = true;
                } else if p.x > rect.right() - 22.0 {
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
