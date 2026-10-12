//! Image viewer: zoom/pan, before/after layouts, overlays and on-image tools.

use darkslide_core::params::*;
use darkslide_core::pipeline::geometry::Geometry;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::app::{App, BeforeAfter, Zoom};
use crate::edit_panel::{CurveMode, Pick};
use crate::tool_panels::{self, CropOverlay, MaskTool, Tool};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DragKind {
    #[default]
    None,
    Pan,
    CropMove,
    /// 0..8: corners (tl, tr, br, bl) then edges (t, r, b, l)
    CropHandle(u8),
    CropRotate,
    Straighten,
    Brush,
    LinearNew,
    LinearHandle(u8),
    RadialNew,
    RadialHandle(u8),
    SpotNew,
    SpotSource(usize),
    SpotMove(usize),
    RedEye,
    RedEyeMove(usize),
    RedEyeSize(usize),
    ObjectBox,
    Guide,
    Tat,
}

#[derive(Default)]
pub struct DragState {
    pub kind: DragKind,
    pub start: Pos2,
    pub start_center: [f32; 2],
    pub start_crop: Crop,
    pub start_angle_ref: f32,
    pub tat_value: f32,
    pub tat_last_y: f32,
    pub tat_hue: f32,
    /// Centre and radii of the red eye box being moved or sized.
    pub start_eye: [f32; 4],
    pub hover_rgb: Option<[u8; 3]>,
    /// Last pointer position seen during the drag (release frames have none).
    pub last: Pos2,
}

impl DragState {
    pub fn active(&self) -> bool {
        !matches!(self.kind, DragKind::None | DragKind::Pan)
    }
}

/// Screen <-> image coordinate mapping for the current view.
pub struct ViewMap {
    pub rect: Rect,
    pub crop_tool: bool,
    pub geo: Geometry,
    /// Crop tool: the photo turns under an upright crop box, like ACR.
    pub rot: Option<RotView>,
}

/// Crop-tool screen frame: rotated by the straighten angle so the crop is
/// axis-aligned on screen. Screen = centre + scale * R(-angle) * (T px - T centre).
#[derive(Clone, Copy)]
pub struct RotView {
    pub center: Pos2,
    /// Screen points per T-frame pixel.
    pub scale: f32,
    cos: f64,
    sin: f64,
    ow: f64,
    oh: f64,
}

impl RotView {
    /// Fit the rotated photo's bounding box `rect` (laid out from `rotated_size`).
    pub fn new(rect: Rect, geo: &Geometry) -> Self {
        let a = (geo.crop.angle as f64).to_radians();
        let (bw, _) = rotated_size(geo);
        Self { center: rect.center(), scale: rect.width() / bw as f32, cos: a.cos(), sin: a.sin(), ow: geo.ow, oh: geo.oh }
    }
    pub fn from_t(&self, t: [f64; 2]) -> Pos2 {
        let (x, y) = ((t[0] - 0.5) * self.ow, (t[1] - 0.5) * self.oh);
        let (qx, qy) = (x * self.cos + y * self.sin, -x * self.sin + y * self.cos);
        self.center + vec2(qx as f32, qy as f32) * self.scale
    }
    pub fn to_t(&self, p: Pos2) -> [f64; 2] {
        let q = (p - self.center) / self.scale;
        let (qx, qy) = (q.x as f64, q.y as f64);
        let (x, y) = (qx * self.cos - qy * self.sin, qx * self.sin + qy * self.cos);
        [x / self.ow + 0.5, y / self.oh + 0.5]
    }
    /// Screen corners of the whole photo (tl, tr, br, bl).
    pub fn photo_corners(&self) -> [Pos2; 4] {
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|t| self.from_t(t))
    }
}

/// Size of the photo's bounding box once turned by the straighten angle.
pub fn rotated_size(geo: &Geometry) -> (f64, f64) {
    let a = (geo.crop.angle as f64).to_radians();
    let (c, s) = (a.cos().abs(), a.sin().abs());
    (geo.ow * c + geo.oh * s, geo.ow * s + geo.oh * c)
}

/// Draw a texture onto a (possibly rotated) quad.
fn paint_quad(painter: &egui::Painter, tex: egui::TextureId, corners: [Pos2; 4]) {
    let mut mesh = egui::Mesh::with_texture(tex);
    for (p, uv) in corners.iter().zip([pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)]) {
        mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv, color: Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

impl ViewMap {
    pub fn to_crop(&self, p: Pos2) -> [f64; 2] {
        [((p.x - self.rect.min.x) / self.rect.width()) as f64, ((p.y - self.rect.min.y) / self.rect.height()) as f64]
    }
    pub fn from_crop(&self, c: [f64; 2]) -> Pos2 {
        pos2(self.rect.min.x + c[0] as f32 * self.rect.width(), self.rect.min.y + c[1] as f32 * self.rect.height())
    }
    pub fn to_t(&self, p: Pos2) -> [f64; 2] {
        if let Some(r) = &self.rot {
            return r.to_t(p);
        }
        let c = self.to_crop(p);
        if self.crop_tool {
            c
        } else {
            let (u, v) = self.geo.crop_to_t(c[0], c[1]);
            [u, v]
        }
    }
    pub fn from_t(&self, t: [f64; 2]) -> Pos2 {
        if let Some(r) = &self.rot {
            return r.from_t(t);
        }
        if self.crop_tool {
            self.from_crop(t)
        } else {
            let (x, y) = self.geo.t_to_crop(t[0], t[1]);
            self.from_crop([x, y])
        }
    }
    pub fn to_s(&self, p: Pos2) -> [f64; 2] {
        let t = self.to_t(p);
        let (ou, ov) = self.geo.t_to_o(t[0], t[1], 1);
        let (su, sv) = self.geo.o_to_s(ou, ov);
        [su, sv]
    }
    pub fn from_s(&self, s: [f64; 2]) -> Pos2 {
        let (ou, ov) = self.geo.s_to_o(s[0], s[1]);
        let (tu, tv) = self.geo.o_to_t(ou, ov);
        self.from_t([tu, tv])
    }
    pub fn to_o(&self, p: Pos2) -> [f64; 2] {
        let t = self.to_t(p);
        let (ou, ov) = self.geo.t_to_o(t[0], t[1], 1);
        [ou, ov]
    }
    pub fn from_o(&self, o: [f64; 2]) -> Pos2 {
        let (tu, tv) = self.geo.o_to_t(o[0], o[1]);
        self.from_t([tu, tv])
    }
    /// Screen points per full-resolution pixel.
    pub fn scale(&self) -> f32 {
        let (cw, _) = self.geo.crop_size();
        self.rect.width() / cw as f32
    }
    pub fn long_edge(&self) -> f32 {
        self.geo.ow.max(self.geo.oh) as f32
    }
}

pub fn viewer(app: &mut App, ui: &mut egui::Ui) {
    let view = ui.max_rect();
    app.view_rect = view;
    // the crop box needs the whole photo; side-by-side views have no zoom
    if app.tools.tool == Tool::Crop || app.ba != BeforeAfter::Off {
        app.zoom = Zoom::Fit;
    }
    // the centre only means something when zoomed; Fit -> 100% starts centred
    if app.zoom == Zoom::Fit {
        app.center = [0.5, 0.5];
    }
    if app.doc().is_none() {
        app.start_view(ui, view);
        return;
    }
    let (resp_rect, resp) = (view, ui.interact(view, ui.id().with("viewer"), Sense::click_and_drag()));
    let painter = ui.painter_at(resp_rect);
    let doc = app.doc().unwrap();
    if let Some(err) = &doc.load_error {
        painter.text(
            view.center(),
            egui::Align2::CENTER_CENTER,
            format!("Cannot open {}:\n{err}", doc.name()),
            egui::FontId::proportional(15.0),
            Color32::from_rgb(230, 100, 100),
        );
        return;
    }
    let Some(engine) = doc.engine.clone() else {
        painter.text(view.center(), egui::Align2::CENTER_CENTER, format!("Loading {}…", doc.name()), egui::FontId::proportional(16.0), Color32::GRAY);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        return;
    };
    let crop_tool = app.tools.tool == Tool::Crop;
    let (fw, fh) = engine.full_size();
    let geo_view = {
        let g = engine.geometry(&doc.settings, fw, fh, crop_tool);
        if crop_tool { g.without_crop() } else { g }
    };
    // in the crop tool the layout box is the turned photo's bounds
    let geo_full = engine.geometry(&doc.settings, fw, fh, false);
    let (cw, ch) = if crop_tool { rotated_size(&geo_full) } else { geo_view.crop_size() };
    let ppp = ui.ctx().pixels_per_point();

    // layout
    let mut views: Vec<(Rect, bool)> = vec![];
    let fit_in = |area: Rect| -> Rect {
        let s = (area.width() as f64 / cw).min(area.height() as f64 / ch).min(1.0 / ppp as f64);
        Rect::from_center_size(area.center(), vec2((cw * s) as f32, (ch * s) as f32))
    };
    match app.ba {
        BeforeAfter::LeftRight => {
            let (l, r) = (Rect::from_min_max(view.min, pos2(view.center().x, view.max.y)), Rect::from_min_max(pos2(view.center().x, view.min.y), view.max));
            views.push((fit_in(l.shrink(4.0)), true));
            views.push((fit_in(r.shrink(4.0)), false));
        }
        BeforeAfter::TopBottom => {
            let (t, b) = (Rect::from_min_max(view.min, pos2(view.max.x, view.center().y)), Rect::from_min_max(pos2(view.min.x, view.center().y), view.max));
            views.push((fit_in(t.shrink(4.0)), true));
            views.push((fit_in(b.shrink(4.0)), false));
        }
        _ => {
            let img_rect = match app.zoom {
                // room around the crop handles to grab and to rotate from
                Zoom::Fit => fit_in(view.shrink(if crop_tool { 36.0 } else { 8.0 })),
                Zoom::Scale(z) => {
                    let size = vec2((cw * z as f64 / ppp as f64) as f32, (ch * z as f64 / ppp as f64) as f32);
                    Rect::from_min_size(pos2(view.center().x - app.center[0] * size.x, view.center().y - app.center[1] * size.y), size)
                }
            };
            views.push((img_rect, app.show_before));
        }
    }

    // draw images
    for (rect, before) in &views {
        let tex = if *before { app.before_tex.as_ref().or(app.fit_tex.as_ref()) } else { app.fit_tex.as_ref() };
        if let Some(t) = tex {
            if crop_tool {
                paint_quad(&painter, t.tex.id(), RotView::new(*rect, &geo_full).photo_corners());
            } else {
                painter.rect_filled(rect.expand(1.0), 0.0, Color32::from_black_alpha(140));
                painter.image(t.tex.id(), *rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
        // zoomed tiles are axis-aligned; while straightening the fit preview stands in
        let turned = crop_tool && geo_full.crop.angle.abs() > 0.01;
        if !*before && !turned && matches!(app.zoom, Zoom::Scale(_)) && matches!(app.ba, BeforeAfter::Off) {
            if let Some(d) = &app.detail_tex {
                let r = &d.region;
                let dr = Rect::from_min_max(
                    pos2(rect.min.x + r.x0 as f32 * rect.width(), rect.min.y + r.y0 as f32 * rect.height()),
                    pos2(rect.min.x + r.x1 as f32 * rect.width(), rect.min.y + r.y1 as f32 * rect.height()),
                );
                // a tile from an earlier zoom/pan/crop would land in the wrong
                // place or scale; show the fit preview until the new one arrives
                let vis = rect.intersect(view);
                let matches_view = vis.is_positive()
                    && (dr.min - vis.min).length() < 2.0
                    && (dr.max - vis.max).length() < 2.0
                    && ((dr.width() * ppp) - d.w as f32).abs() <= 2.0;
                if d.ignore_crop == crop_tool && matches_view {
                    painter.image(d.tex.id(), dr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
        }
        if views.len() == 2 {
            let label = if *before { "Before" } else { "After" };
            let at = rect.left_top() + vec2(6.0, 6.0);
            let r = painter.text(at, egui::Align2::LEFT_TOP, label, egui::FontId::proportional(13.0), Color32::TRANSPARENT);
            painter.rect_filled(r.expand2(vec2(5.0, 2.0)), 3.0, Color32::from_black_alpha(150));
            painter.text(at, egui::Align2::LEFT_TOP, label, egui::FontId::proportional(13.0), Color32::WHITE);
        }
    }
    // split views: before on one side of a divider
    if matches!(app.ba, BeforeAfter::SplitLeftRight | BeforeAfter::SplitTopBottom) {
        if let (Some(b), Some((rect, _))) = (app.before_tex.as_ref(), views.first()) {
            let (r, uv) = if app.ba == BeforeAfter::SplitLeftRight {
                (Rect::from_min_max(rect.min, pos2(rect.center().x, rect.max.y)), Rect::from_min_max(pos2(0.0, 0.0), pos2(0.5, 1.0)))
            } else {
                (Rect::from_min_max(rect.min, pos2(rect.max.x, rect.center().y)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 0.5)))
            };
            painter.image(b.tex.id(), r, uv, Color32::WHITE);
            let line = if app.ba == BeforeAfter::SplitLeftRight {
                [pos2(rect.center().x, rect.min.y), pos2(rect.center().x, rect.max.y)]
            } else {
                [pos2(rect.min.x, rect.center().y), pos2(rect.max.x, rect.center().y)]
            };
            painter.line_segment(line, Stroke::new(1.5, Color32::WHITE));
        }
    }

    let Some(&(img_rect, _)) = views.last() else { return };
    let map = ViewMap { rect: img_rect, crop_tool, rot: crop_tool.then(|| RotView::new(img_rect, &geo_full)), geo: geo_full };

    // mask overlay
    if app.tools.tool == Tool::Masking && app.tools.show_mask_overlay {
        if let Some(m) = &app.mask_tex {
            painter.image(m.tex.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    }
    // visualize spots
    if app.tools.tool == Tool::Remove && app.tools.visualize_spots {
        if let Some(tex) = visualize_tex(app, ui.ctx()) {
            painter.image(tex.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    }

    // hover readout
    app.drag.hover_rgb = None;
    if let Some(hp) = resp.hover_pos() {
        // the fit preview is the uncropped photo in the crop tool
        let c = if map.rot.is_some() { map.to_t(hp) } else { map.to_crop(hp) };
        if (0.0..=1.0).contains(&c[0]) && (0.0..=1.0).contains(&c[1]) {
            app.drag.hover_rgb = app.sample_fit([c[0] as f32, c[1] as f32]);
        }
    }

    // zoom with the wheel around the cursor; ctrl-wheel or plain wheel
    let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
    if resp.hovered() && (scroll.abs() > 0.5 || (pinch - 1.0).abs() > 1e-3) && app.ba == BeforeAfter::Off && !crop_tool {
        if let Some(hp) = resp.hover_pos() {
            let fit_z = fit_in(view.shrink(8.0)).width() * ppp / cw as f32;
            let cur = match app.zoom {
                Zoom::Fit => fit_z,
                Zoom::Scale(z) => z,
            };
            // ctrl+wheel arrives as a zoom delta, the plain wheel as a scroll
            let nz = (cur * (1.0 + scroll * 0.0025) * pinch).min(16.0);
            let c = map.to_crop(hp);
            let size = vec2((cw * nz as f64 / ppp as f64) as f32, (ch * nz as f64 / ppp as f64) as f32);
            // keep the point under the cursor fixed
            let new_min = hp - vec2(c[0] as f32 * size.x, c[1] as f32 * size.y);
            app.center = [((view.center().x - new_min.x) / size.x), ((view.center().y - new_min.y) / size.y)];
            // zooming out stops at Fit
            app.zoom = if nz <= fit_z * 1.001 { Zoom::Fit } else { Zoom::Scale(nz) };
        }
    }

    let space = ui.input(|i| i.key_down(egui::Key::Space));
    let middle = ui.input(|i| i.pointer.middle_down());
    let shift = ui.input(|i| i.modifiers.shift);
    let alt = ui.input(|i| i.modifiers.alt);
    let ctrl = ui.input(|i| i.modifiers.command);

    // ---------------------------------------------------------- interactions
    // double-click inside the crop box applies it, as in Lightroom
    if resp.double_clicked() && crop_tool && !app.tools.straighten {
        app.commit_crop();
    }

    let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
    if resp.drag_started() {
        // hit-test where the button went down, not where the drag threshold was crossed
        app.drag.start = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()).or(pointer).unwrap_or_default();
        app.drag.last = app.drag.start;
        app.drag.start_center = app.center;
        let p = app.drag.start;
        app.drag.kind = if space || middle {
            DragKind::Pan
        } else {
            let k = if crop_tool && ctrl { DragKind::Straighten } else { begin_tool_drag(app, &map, p, alt) };
            if k == DragKind::Tat {
                tat_begin(app, &map, p);
            }
            k
        };
    }
    if resp.dragged() {
        let p = resp.interact_pointer_pos().or(pointer).unwrap_or(app.drag.last);
        app.drag.last = p;
        match app.drag.kind {
            DragKind::Pan => {
                let d = p - app.drag.start;
                if let Zoom::Scale(_) = app.zoom {
                    app.center = [app.drag.start_center[0] - d.x / img_rect.width(), app.drag.start_center[1] - d.y / img_rect.height()];
                }
            }
            DragKind::None => {}
            _ => continue_tool_drag(app, &map, p, shift, alt),
        }
    }
    if resp.drag_stopped() {
        let p = pointer.unwrap_or(app.drag.last);
        let kind = app.drag.kind;
        end_tool_drag(app, &map, p);
        // a press that barely moved is a click (eyedroppers, samplers)
        if p.distance(app.drag.start) < 4.0 && matches!(kind, DragKind::Pan | DragKind::None) {
            click(app, &map, p);
        }
        app.drag.kind = DragKind::None;
    }
    // the second click of a double-click already did its job above
    if resp.clicked() && !resp.double_clicked() {
        if let Some(p) = resp.interact_pointer_pos().or(pointer) {
            click(app, &map, p);
        }
    }

    // the view centre always stays over the photo, so it can't be lost off screen
    app.center = [app.center[0].clamp(0.0, 1.0), app.center[1].clamp(0.0, 1.0)];

    // ---------------------------------------------------------- overlays
    draw_overlays(app, &painter, &map, resp.hover_pos());

    // cursor hints: show what a press here would do
    if resp.hovered() || app.drag.active() {
        let at = resp.hover_pos().or(pointer).unwrap_or(app.drag.last);
        let c = if space {
            if app.drag.kind == DragKind::Pan { Cursor::Icon(egui::CursorIcon::Grabbing) } else { Cursor::Icon(egui::CursorIcon::Grab) }
        } else {
            cursor_for(app, &map, at)
        };
        match c {
            Cursor::Icon(i) => ui.ctx().set_cursor_icon(i),
            Cursor::Rotate => {
                ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                draw_rotate_cursor(&painter, at, crop_corners_screen(&map));
            }
        }
    }
}

enum Cursor {
    Icon(egui::CursorIcon),
    /// No system rotate cursor exists, so it's drawn.
    Rotate,
}

/// Double-headed resize cursor closest to a screen direction.
fn resize_cursor(d: Vec2) -> egui::CursorIcon {
    use egui::CursorIcon::*;
    let a = d.y.atan2(d.x).to_degrees().rem_euclid(180.0);
    match ((a + 22.5) / 45.0) as u32 % 4 {
        0 => ResizeHorizontal,
        1 => ResizeNwSe,
        2 => ResizeVertical,
        _ => ResizeNeSw,
    }
}

/// The cursor for what a press at `p` would start (or the drag in progress).
/// Mirrors the hit tests in begin_tool_drag without changing anything.
fn cursor_for(app: &App, map: &ViewMap, p: Pos2) -> Cursor {
    use egui::CursorIcon::*;
    let corners = crop_corners_screen(map);
    let crop_handle = |i: u8| -> egui::CursorIcon {
        let c = corners[0].lerp(corners[2], 0.5);
        let at = if i < 4 { corners[i as usize] } else { corners[(i - 4) as usize].lerp(corners[(i - 3) as usize % 4], 0.5) };
        resize_cursor(at - c)
    };
    let zoomed = matches!(app.zoom, Zoom::Scale(_));
    match app.drag.kind {
        DragKind::Pan if zoomed => return Cursor::Icon(Grabbing),
        DragKind::CropMove | DragKind::SpotMove(_) | DragKind::SpotSource(_) | DragKind::LinearHandle(2) | DragKind::RadialHandle(0) => {
            return Cursor::Icon(Grabbing);
        }
        DragKind::CropHandle(i) => return Cursor::Icon(crop_handle(i)),
        DragKind::CropRotate => return Cursor::Rotate,
        DragKind::LinearHandle(_) | DragKind::RadialHandle(_) | DragKind::RadialNew | DragKind::LinearNew | DragKind::ObjectBox | DragKind::Straighten => {
            return Cursor::Icon(Crosshair);
        }
        DragKind::Tat => return Cursor::Icon(ResizeVertical),
        DragKind::None | DragKind::Pan => {}
        _ => return Cursor::Icon(Crosshair),
    }
    match app.pick {
        Pick::None => {}
        Pick::TatCurve | Pick::TatMixer(_) => return Cursor::Icon(ResizeVertical),
        _ => return Cursor::Icon(Crosshair),
    }
    let Some(doc) = app.doc() else { return Cursor::Icon(Default) };
    match app.tools.tool {
        Tool::Crop => {
            if app.tools.straighten {
                return Cursor::Icon(Crosshair);
            }
            for i in 0..4u8 {
                if corners[i as usize].distance(p) < 14.0 {
                    return Cursor::Icon(crop_handle(i));
                }
            }
            for i in 0..4u8 {
                if corners[i as usize].lerp(corners[(i as usize + 1) % 4], 0.5).distance(p) < 14.0 {
                    return Cursor::Icon(crop_handle(4 + i));
                }
            }
            let mut inside = false;
            for i in 0..4 {
                let (a, b) = (corners[i], corners[(i + 1) % 4]);
                if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                    inside = !inside;
                }
            }
            Cursor::Icon(if inside { Move } else { return Cursor::Rotate })
        }
        Tool::Masking => {
            let tu = &app.tools;
            if let (Some(mi), Some(ci)) = (tu.selected_mask, tu.selected_comp)
                && let Some(c) = doc.settings.masks.get(mi).and_then(|m| m.components.get(ci))
            {
                let hs = gradient_handles(&c.shape, map);
                if let Some(&(h, q)) = hs.iter().find(|(_, q)| q.distance(p) < 12.0) {
                    let centre = hs[0].1;
                    let linear = matches!(c.shape, MaskShape::Linear { .. });
                    return Cursor::Icon(match (linear, h) {
                        (true, 2) | (false, 0) => Move,
                        (true, _) => resize_cursor(hs[1].1 - hs[0].1),
                        (false, 5) => Alias,
                        _ => resize_cursor(q - centre),
                    });
                }
            }
            if pin_at(app, map, p).is_some() {
                return Cursor::Icon(PointingHand);
            }
            Cursor::Icon(match tu.mask_tool {
                MaskTool::None if zoomed => Grab,
                MaskTool::None => Default,
                _ => Crosshair,
            })
        }
        Tool::Remove => {
            for sp in &doc.settings.retouch {
                if let Some(c) = sp.points.first() {
                    let dst = map.from_s([c[0] as f64, c[1] as f64]);
                    let src = map.from_s([(c[0] + sp.source_offset[0]) as f64, (c[1] + sp.source_offset[1]) as f64]);
                    if (sp.mode != RetouchMode::Remove && src.distance(p) < 12.0) || dst.distance(p) < 10.0 {
                        return Cursor::Icon(Move);
                    }
                }
            }
            Cursor::Icon(Crosshair)
        }
        Tool::RedEye => Cursor::Icon(match red_eye_at(app, map, p) {
            Some((_, true)) => ResizeNwSe,
            Some(_) => Move,
            Option::None => Crosshair,
        }),
        _ if zoomed => Cursor::Icon(Grab),
        _ => Cursor::Icon(ZoomIn),
    }
}

/// Curved double arrow around the pointer, tangent to the crop's rotation.
fn draw_rotate_cursor(painter: &egui::Painter, p: Pos2, corners: [Pos2; 4]) {
    let c = corners[0].lerp(corners[2], 0.5);
    let base = (p.y - c.y).atan2(p.x - c.x);
    // arc centred on the crop centre, so it reads as "turn around the middle"
    let r = 11.0;
    let pts: Vec<Pos2> = (0..=12)
        .map(|k| {
            let a = base + std::f32::consts::FRAC_PI_2 + (k as f32 / 12.0 - 0.5) * 1.8;
            p + vec2(a.cos(), a.sin()) * r - vec2(base.cos(), base.sin()) * r
        })
        .collect();
    let head = |tip: Pos2, prev: Pos2| -> [Pos2; 3] {
        let d = (tip - prev).normalized();
        let n = vec2(-d.y, d.x);
        [tip + d * 4.0, tip - d * 2.0 + n * 4.5, tip - d * 2.0 - n * 4.5]
    };
    for (w, col) in [(4.0, Color32::BLACK), (1.8, Color32::WHITE)] {
        painter.add(egui::Shape::line(pts.clone(), egui::Stroke::new(w, col)));
        for (a, b) in [(pts[12], pts[11]), (pts[0], pts[1])] {
            let h = head(a, b);
            if w > 2.0 {
                painter.add(egui::Shape::convex_polygon(h.to_vec(), col, egui::Stroke::new(1.5, col)));
            } else {
                painter.add(egui::Shape::convex_polygon(h.to_vec(), col, egui::Stroke::NONE));
            }
        }
    }
}

fn visualize_tex(app: &App, ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let (rgba, w, h) = app.fit_rgba.as_ref()?;
    let key = egui::Id::new(("visspots", rgba.len(), rgba[rgba.len() / 2], rgba[rgba.len() / 3], (app.tools.visualize_threshold * 10.0) as i32));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(t);
    }
    let img = darkslide_core::image::Image3::from_fn(*w, *h, |x, y| {
        let i = (y * w + x) * 4;
        [rgba[i] as f32 / 255.0, rgba[i + 1] as f32 / 255.0, rgba[i + 2] as f32 / 255.0].map(darkslide_core::color::srgb_decode)
    });
    let v = darkslide_core::retouch::visualize_spots(&img, app.tools.visualize_threshold);
    let px: Vec<u8> = v
        .data
        .iter()
        .flat_map(|&a| {
            let c = if a > 0.5 { 255 } else { 0 };
            [c, c, c, 255]
        })
        .collect();
    let t = ctx.load_texture("visspots", egui::ColorImage::from_rgba_unmultiplied([*w, *h], &px), egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(key, t.clone()));
    Some(t)
}

fn rot_frame(geo: &Geometry, t: [f64; 2]) -> [f64; 2] {
    // T-frame -> the frame where the crop rect is axis-aligned
    let a = (geo.crop.angle as f64).to_radians();
    let x = (t[0] - 0.5) * geo.ow;
    let y = (t[1] - 0.5) * geo.oh;
    let qx = x * a.cos() + y * a.sin();
    let qy = -x * a.sin() + y * a.cos();
    [qx / geo.ow + 0.5, qy / geo.oh + 0.5]
}

fn crop_corners_screen(map: &ViewMap) -> [Pos2; 4] {
    [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, y)| {
        let (u, v) = map.geo.crop_to_t(x, y);
        map.from_t([u, v])
    })
}

fn begin_tool_drag(app: &mut App, map: &ViewMap, p: Pos2, alt: bool) -> DragKind {
    let tool = app.tools.tool;
    let pick = app.pick;
    let Some(cur) = app.current else { return DragKind::None };
    let doc = &mut app.docs[cur];
    if pick == Pick::TatCurve || matches!(pick, Pick::TatMixer(_)) {
        return DragKind::Tat;
    }
    if pick == Pick::Upright {
        return DragKind::Guide;
    }
    match tool {
        Tool::Crop => {
            if app.tools.straighten {
                return DragKind::Straighten;
            }
            app.drag.start_crop = doc.settings.crop;
            let corners = crop_corners_screen(map);
            for (i, c) in corners.iter().enumerate() {
                if c.distance(p) < 14.0 {
                    return DragKind::CropHandle(i as u8);
                }
            }
            for i in 0..4 {
                let m = corners[i].lerp(corners[(i + 1) % 4], 0.5);
                if m.distance(p) < 14.0 {
                    return DragKind::CropHandle(4 + i as u8);
                }
            }
            let poly_contains = {
                let mut inside = false;
                for i in 0..4 {
                    let (a, b) = (corners[i], corners[(i + 1) % 4]);
                    if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                        inside = !inside;
                    }
                }
                inside
            };
            if poly_contains {
                DragKind::CropMove
            } else {
                let c = corners[0].lerp(corners[2], 0.5);
                app.drag.start_angle_ref = (p.y - c.y).atan2(p.x - c.x).to_degrees();
                DragKind::CropRotate
            }
        }
        Tool::Masking => {
            let t = map.to_t(p);
            let t = [t[0] as f32, t[1] as f32];
            let tu = &mut app.tools;
            // existing handles of the selected component first
            if let (Some(mi), Some(ci)) = (tu.selected_mask, tu.selected_comp)
                && let Some(c) = doc.settings.masks.get(mi).and_then(|m| m.components.get(ci))
            {
                let linear = matches!(c.shape, MaskShape::Linear { .. });
                if let Some((h, _)) = gradient_handles(&c.shape, map).into_iter().find(|(_, q)| q.distance(p) < 12.0) {
                    return if linear { DragKind::LinearHandle(h) } else { DragKind::RadialHandle(h) };
                }
            }
            // a placed-by-menu component (Add / Subtract / Intersect) is drawn by the next drag
            let placing = tu.place_comp.is_some() && tu.place_comp == tu.selected_mask.zip(tu.selected_comp);
            match tu.mask_tool {
                MaskTool::Brush => {
                    let (mi, ci) = ensure_component(doc, tu, MaskShape::Brush { strokes: vec![] }, |s| matches!(s, MaskShape::Brush { .. }));
                    let b = &tu.brush;
                    if let MaskShape::Brush { strokes } = &mut doc.settings.masks[mi].components[ci].shape {
                        strokes.push(BrushStroke {
                            points: vec![t],
                            radius: b.size,
                            feather: b.feather,
                            flow: b.flow,
                            density: b.density,
                            erase: b.erase || alt,
                            auto_mask: b.auto_mask,
                        });
                    }
                    DragKind::Brush
                }
                MaskTool::Linear => {
                    if !placing {
                        new_component(doc, tu, MaskShape::Linear { start: t, end: t, two_sided: false });
                    }
                    DragKind::LinearNew
                }
                MaskTool::Radial => {
                    if !placing {
                        new_component(doc, tu, MaskShape::Radial { center: t, radius: [0.001, 0.001], angle: 0.0, feather: 50.0 });
                    }
                    DragKind::RadialNew
                }
                MaskTool::ObjectBox => DragKind::ObjectBox,
                MaskTool::None => DragKind::Pan,
            }
        }
        Tool::Remove => {
            let s = map.to_s(p);
            // grab an existing source or spot
            for (i, sp) in doc.settings.retouch.iter().enumerate() {
                if let Some(c) = sp.points.first() {
                    let dst = map.from_s([c[0] as f64, c[1] as f64]);
                    let src = map.from_s([(c[0] + sp.source_offset[0]) as f64, (c[1] + sp.source_offset[1]) as f64]);
                    if sp.mode != RetouchMode::Remove && src.distance(p) < 12.0 {
                        app.tools.selected_spot = Some(i);
                        return DragKind::SpotSource(i);
                    }
                    if dst.distance(p) < 10.0 {
                        app.tools.selected_spot = Some(i);
                        return DragKind::SpotMove(i);
                    }
                }
            }
            let tu = &app.tools;
            doc.settings.retouch.push(RetouchSpot {
                mode: tu.remove_mode,
                points: vec![[s[0] as f32, s[1] as f32]],
                radius: tu.spot_size,
                feather: tu.spot_feather,
                opacity: tu.spot_opacity,
                source_offset: [0.0, 0.0],
                mask: None,
            });
            app.tools.selected_spot = Some(doc.settings.retouch.len() - 1);
            DragKind::SpotNew
        }
        Tool::RedEye => {
            if let Some((i, rim)) = red_eye_at(app, map, p) {
                app.tools.selected_red_eye = Some(i);
                let re = &app.docs[cur].settings.red_eye[i];
                app.drag.start_eye = [re.center[0], re.center[1], re.radius[0], re.radius[1]];
                if rim { DragKind::RedEyeSize(i) } else { DragKind::RedEyeMove(i) }
            } else {
                DragKind::RedEye
            }
        }
        _ => DragKind::Pan,
    }
}

/// The red eye correction under `p`: (index, true when on its rim).
fn red_eye_at(app: &App, map: &ViewMap, p: Pos2) -> Option<(usize, bool)> {
    let d = app.doc()?;
    let t = map.to_t(p);
    d.settings.red_eye.iter().enumerate().rev().find_map(|(i, re)| {
        let (dx, dy) = ((t[0] as f32 - re.center[0]) / re.radius[0].max(1e-6), (t[1] as f32 - re.center[1]) / re.radius[1].max(1e-6));
        let r = (dx * dx + dy * dy).sqrt();
        // a few screen pixels of rim, whatever the box size
        let rim = 6.0 / (re.radius[0] * map.geo.ow as f32 * map.scale()).max(1.0);
        (r < 1.0 + rim).then_some((i, r > 1.0 - rim))
    })
}

/// Point on a radial gradient's ellipse frame: (u, v) in units of its radii.
fn radial_pt(map: &ViewMap, center: [f32; 2], radius: [f32; 2], angle: f32, u: f32, v: f32) -> Pos2 {
    let (sa, ca) = (angle.to_radians().sin(), angle.to_radians().cos());
    let (x, y) = (u * radius[0], v * radius[1]);
    map.from_t([(center[0] + x * ca - y * sa) as f64, (center[1] + x * sa + y * ca) as f64])
}

/// On-screen handles of a gradient. Linear: 0 start, 1 end, 2 middle.
/// Radial: 0 centre, 1 right, 2 bottom, 3 left, 4 top, 5 rotate.
fn gradient_handles(shape: &MaskShape, map: &ViewMap) -> Vec<(u8, Pos2)> {
    match shape {
        MaskShape::Linear { start, end, .. } => {
            let ps = map.from_t([start[0] as f64, start[1] as f64]);
            let pe = map.from_t([end[0] as f64, end[1] as f64]);
            vec![(0, ps), (1, pe), (2, ps.lerp(pe, 0.5))]
        }
        MaskShape::Radial { center, radius, angle, .. } => {
            let at = |u, v| radial_pt(map, *center, *radius, *angle, u, v);
            let top = at(0.0, -1.0);
            let c = at(0.0, 0.0);
            let rot = top + (top - c).normalized() * 22.0;
            vec![(0, c), (1, at(1.0, 0.0)), (2, at(0.0, 1.0)), (3, at(-1.0, 0.0)), (4, top), (5, rot)]
        }
        _ => vec![],
    }
}

/// Where a mask's pin sits (T frame), for masks that have a place.
fn mask_anchor(m: &MaskGroup) -> Option<[f32; 2]> {
    match &m.components.first()?.shape {
        MaskShape::Linear { start, end, .. } => Some([(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0]),
        MaskShape::Radial { center, .. } => Some(*center),
        MaskShape::Brush { strokes } => strokes.first().and_then(|s| s.points.first().copied()),
        _ => None,
    }
}

/// The mask whose pin is under `p`, other than the selected one.
fn pin_at(app: &App, map: &ViewMap, p: Pos2) -> Option<usize> {
    let d = app.doc()?;
    d.settings.masks.iter().enumerate().find_map(|(i, m)| {
        let a = mask_anchor(m)?;
        (Some(i) != app.tools.selected_mask && map.from_t([a[0] as f64, a[1] as f64]).distance(p) < 10.0).then_some(i)
    })
}

/// Select a mask and arm only what editing it needs (the brush for a brush mask).
pub fn select_mask(tu: &mut crate::tool_panels::ToolUi, s: &DevelopSettings, mi: usize) {
    tu.selected_mask = Some(mi);
    tu.selected_comp = Some(0);
    tu.place_comp = None;
    tu.mask_tool = match s.masks.get(mi).and_then(|m| m.components.first()).map(|c| &c.shape) {
        Some(MaskShape::Brush { .. }) => MaskTool::Brush,
        _ => MaskTool::None,
    };
}

fn ensure_component(doc: &mut crate::doc::Doc, tu: &mut crate::tool_panels::ToolUi, shape: MaskShape, is: impl Fn(&MaskShape) -> bool) -> (usize, usize) {
    let s = &mut doc.settings;
    if let (Some(mi), Some(ci)) = (tu.selected_mask, tu.selected_comp) {
        if s.masks.get(mi).and_then(|m| m.components.get(ci)).map(|c| is(&c.shape)).unwrap_or(false) {
            return (mi, ci);
        }
    }
    new_component(doc, tu, shape)
}

fn new_component(doc: &mut crate::doc::Doc, tu: &mut crate::tool_panels::ToolUi, shape: MaskShape) -> (usize, usize) {
    let s = &mut doc.settings;
    match tu.selected_mask.filter(|&i| i < s.masks.len()) {
        // a selected mask whose current component is the same kind gets a new mask;
        // otherwise add to the selected mask
        Some(mi) if s.masks[mi].components.is_empty() => {
            s.masks[mi].components.push(MaskComponent { shape, ..Default::default() });
            tu.selected_comp = Some(0);
            (mi, 0)
        }
        _ => {
            s.masks.push(MaskGroup {
                name: format!("Mask {}", s.masks.len() + 1),
                components: vec![MaskComponent { shape, ..Default::default() }],
                ..Default::default()
            });
            let mi = s.masks.len() - 1;
            tu.selected_mask = Some(mi);
            tu.selected_comp = Some(0);
            (mi, 0)
        }
    }
}

fn continue_tool_drag(app: &mut App, map: &ViewMap, p: Pos2, shift: bool, alt: bool) {
    let kind = app.drag.kind;
    let start = app.drag.start;
    let pick = app.pick;
    let curve_mode = app.edit.curve_mode;
    let (sel_mask, sel_comp) = (app.tools.selected_mask, app.tools.selected_comp);
    let start_crop = app.drag.start_crop;
    let angle_ref = app.drag.start_angle_ref;
    let tat_value = app.drag.tat_value;
    let tat_hue = app.drag.tat_hue;
    let eye0 = app.drag.start_eye;
    let dy_tat = p.y - app.drag.tat_last_y;
    let Some(doc) = app.doc_mut() else { return };
    let Some(engine) = doc.engine.clone() else { return };
    let s = &mut doc.settings;
    let t = map.to_t(p);
    let tf = [t[0] as f32, t[1] as f32];
    match kind {
        DragKind::CropMove | DragKind::CropHandle(_) => {
            let geo = &map.geo;
            let a = rot_frame(geo, map.to_t(start));
            let b = rot_frame(geo, t);
            let (dx, dy) = ((b[0] - a[0]) as f32, (b[1] - a[1]) as f32);
            let mut c = start_crop;
            match kind {
                DragKind::CropMove => {
                    let (w, h) = (c.right - c.left, c.bottom - c.top);
                    c.left = (c.left + dx).clamp(0.0, (1.0 - w).max(0.0));
                    c.top = (c.top + dy).clamp(0.0, (1.0 - h).max(0.0));
                    c.right = c.left + w;
                    c.bottom = c.top + h;
                    if c.angle != 0.0 {
                        // rotated crops can't be clamped per axis; allow free movement
                        c.left = start_crop.left + dx;
                        c.right = start_crop.right + dx;
                        c.top = start_crop.top + dy;
                        c.bottom = start_crop.bottom + dy;
                    }
                }
                DragKind::CropHandle(h) => {
                    let min = 0.02;
                    match h {
                        0 => {
                            c.left += dx;
                            c.top += dy;
                        }
                        1 => {
                            c.right += dx;
                            c.top += dy;
                        }
                        2 => {
                            c.right += dx;
                            c.bottom += dy;
                        }
                        3 => {
                            c.left += dx;
                            c.bottom += dy;
                        }
                        4 => c.top += dy,
                        5 => c.right += dx,
                        6 => c.bottom += dy,
                        _ => c.left += dx,
                    }
                    // alt resizes about the centre
                    if alt {
                        let (sx, sy) = match h {
                            0 => (-1.0, -1.0),
                            1 => (1.0, -1.0),
                            2 => (1.0, 1.0),
                            3 => (-1.0, 1.0),
                            4 => (0.0, -1.0),
                            5 => (1.0, 0.0),
                            6 => (0.0, 1.0),
                            _ => (-1.0, 0.0),
                        };
                        if sx > 0.0 {
                            c.left -= dx;
                        } else if sx < 0.0 {
                            c.right -= dx;
                        }
                        if sy > 0.0 {
                            c.top -= dy;
                        } else if sy < 0.0 {
                            c.bottom -= dy;
                        }
                    }
                    if !c.expand {
                        c.left = c.left.max(0.0);
                        c.top = c.top.max(0.0);
                        c.right = c.right.min(1.0);
                        c.bottom = c.bottom.min(1.0);
                    }
                    if c.right - c.left < min {
                        c.right = c.left + min;
                    }
                    if c.bottom - c.top < min {
                        c.bottom = c.top + min;
                    }
                    // aspect lock (shift forces the current aspect)
                    let lock = c.aspect.map(|a| if a[0] <= 0.0 { (geo.ow / geo.oh) as f32 } else { a[0] / a[1] }).or(if shift {
                        Some((start_crop.right - start_crop.left) * geo.ow as f32 / ((start_crop.bottom - start_crop.top) * geo.oh as f32))
                    } else {
                        None
                    });
                    if let Some(r) = lock {
                        let (ow, oh) = (geo.ow as f32, geo.oh as f32);
                        let w = (c.right - c.left) * ow;
                        let hh = (c.bottom - c.top) * oh;
                        let horizontal_edge = matches!(h, 4 | 6);
                        let (nw, nh) = if horizontal_edge || (h < 4 && hh * r > w) { (hh * r, hh) } else { (w, w / r) };
                        // anchor the opposite side
                        match h {
                            0 => {
                                c.left = c.right - nw / ow;
                                c.top = c.bottom - nh / oh;
                            }
                            1 => {
                                c.right = c.left + nw / ow;
                                c.top = c.bottom - nh / oh;
                            }
                            3 => {
                                c.left = c.right - nw / ow;
                                c.bottom = c.top + nh / oh;
                            }
                            4 | 6 => {
                                let cx = (c.left + c.right) / 2.0;
                                c.left = cx - nw / ow / 2.0;
                                c.right = cx + nw / ow / 2.0;
                            }
                            5 | 7 => {
                                let cy = (c.top + c.bottom) / 2.0;
                                c.top = cy - nh / oh / 2.0;
                                c.bottom = cy + nh / oh / 2.0;
                            }
                            _ => {
                                c.right = c.left + nw / ow;
                                c.bottom = c.top + nh / oh;
                            }
                        }
                        // the ratio step can push past the photo again: shrink around the anchor until it fits
                        if !c.expand {
                            let (ax, ay) = match h {
                                0 => (c.right, c.bottom),
                                1 => (c.left, c.bottom),
                                3 => (c.right, c.top),
                                4 => ((c.left + c.right) / 2.0, c.bottom),
                                6 => ((c.left + c.right) / 2.0, c.top),
                                5 => (c.left, (c.top + c.bottom) / 2.0),
                                7 => (c.right, (c.top + c.bottom) / 2.0),
                                _ => (c.left, c.top),
                            };
                            let mut k = 1.0f32;
                            if c.left < 0.0 {
                                k = k.min(ax / (ax - c.left));
                            }
                            if c.right > 1.0 {
                                k = k.min((1.0 - ax) / (c.right - ax));
                            }
                            if c.top < 0.0 {
                                k = k.min(ay / (ay - c.top));
                            }
                            if c.bottom > 1.0 {
                                k = k.min((1.0 - ay) / (c.bottom - ay));
                            }
                            let k = k.max(0.0);
                            if k < 1.0 {
                                c.left = ax + (c.left - ax) * k;
                                c.right = ax + (c.right - ax) * k;
                                c.top = ay + (c.top - ay) * k;
                                c.bottom = ay + (c.bottom - ay) * k;
                            }
                        }
                    }
                }
                _ => {}
            }
            let prev = s.crop;
            s.crop = c;
            // reject moves that push a rotated crop outside the image
            if !c.expand && c.angle != 0.0 {
                let (fw, fh) = engine.full_size();
                let g = engine.geometry(s, fw, fh, false);
                let ok = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].iter().all(|&(x, y)| {
                    let (u, v) = g.crop_to_t(x, y);
                    (-1e-3..=1.001).contains(&u) && (-1e-3..=1.001).contains(&v)
                });
                if !ok {
                    s.crop = prev;
                }
            }
        }
        DragKind::CropRotate => {
            let corners = crop_corners_screen(map);
            let c = corners[0].lerp(corners[2], 0.5);
            let ang = (p.y - c.y).atan2(p.x - c.x).to_degrees();
            // the photo turns with the pointer (the box stays upright)
            // fit from the box the drag started with, so turning back grows it again
            s.crop = start_crop;
            s.crop.angle = (start_crop.angle - (ang - angle_ref)).clamp(-45.0, 45.0);
            tool_panels::fit_crop_inside(s, &engine);
        }
        DragKind::RedEyeMove(i) | DragKind::RedEyeSize(i) => {
            let st = map.to_t(start);
            if let Some(re) = s.red_eye.get_mut(i) {
                if matches!(kind, DragKind::RedEyeMove(_)) {
                    re.center = [eye0[0] + (t[0] - st[0]) as f32, eye0[1] + (t[1] - st[1]) as f32];
                } else {
                    // scale about the centre, keeping the box's shape
                    let d = |q: [f64; 2]| (((q[0] as f32 - eye0[0]) * map.geo.ow as f32).powi(2) + ((q[1] as f32 - eye0[1]) * map.geo.oh as f32).powi(2)).sqrt();
                    let k = (d(t) / d(st).max(1e-3)).clamp(0.05, 20.0);
                    re.radius = [(eye0[2] * k).max(0.002), (eye0[3] * k).max(0.002)];
                }
            }
        }
        DragKind::Brush => {
            if let (Some(mi), Some(ci)) = (sel_mask, sel_comp) {
                if let Some(MaskShape::Brush { strokes }) = s.masks.get_mut(mi).and_then(|m| m.components.get_mut(ci)).map(|c| &mut c.shape) {
                    if let Some(st) = strokes.last_mut() {
                        let last = *st.points.last().unwrap();
                        let d = ((tf[0] - last[0]).powi(2) + (tf[1] - last[1]).powi(2)).sqrt();
                        if d > st.radius * 0.15 {
                            st.points.push(tf);
                        }
                    }
                }
            }
        }
        DragKind::LinearNew | DragKind::LinearHandle(_) | DragKind::RadialNew | DragKind::RadialHandle(_) => {
            let st = map.to_t(start);
            if let (Some(mi), Some(ci)) = (sel_mask, sel_comp) {
                if let Some(c) = s.masks.get_mut(mi).and_then(|m| m.components.get_mut(ci)) {
                    match (&mut c.shape, kind) {
                        (MaskShape::Linear { start, end, .. }, DragKind::LinearNew) => {
                            *start = [st[0] as f32, st[1] as f32];
                            *end = tf;
                        }
                        (MaskShape::Linear { start, .. }, DragKind::LinearHandle(0)) => *start = tf,
                        (MaskShape::Linear { end, .. }, DragKind::LinearHandle(1)) => *end = tf,
                        (MaskShape::Linear { start, end, .. }, DragKind::LinearHandle(_)) => {
                            let mid = [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0];
                            let d = [tf[0] - mid[0], tf[1] - mid[1]];
                            for v in [start, end] {
                                v[0] += d[0];
                                v[1] += d[1];
                            }
                        }
                        (MaskShape::Radial { center, radius, .. }, DragKind::RadialNew) => {
                            *center = [st[0] as f32, st[1] as f32];
                            *radius = [(tf[0] - center[0]).abs().max(0.002), (tf[1] - center[1]).abs().max(0.002)];
                            if shift {
                                // circle in pixel space
                                let r = radius[0].max(radius[1] * (map.geo.oh / map.geo.ow) as f32);
                                *radius = [r, r * (map.geo.ow / map.geo.oh) as f32];
                            }
                        }
                        (MaskShape::Radial { center, .. }, DragKind::RadialHandle(0)) => *center = tf,
                        (MaskShape::Radial { center, angle, .. }, DragKind::RadialHandle(5)) => {
                            // the rotate handle sits above the top
                            let d = [tf[0] - center[0], tf[1] - center[1]];
                            *angle = d[1].atan2(d[0]).to_degrees() + 90.0;
                        }
                        (MaskShape::Radial { center, radius, angle, .. }, DragKind::RadialHandle(h)) => {
                            // size along the gradient's own (turned) axes
                            let (sa, ca) = (angle.to_radians().sin(), angle.to_radians().cos());
                            let d = [tf[0] - center[0], tf[1] - center[1]];
                            let (lx, ly) = (d[0] * ca + d[1] * sa, -d[0] * sa + d[1] * ca);
                            if h % 2 == 1 {
                                radius[0] = lx.abs().max(0.002);
                            } else {
                                radius[1] = ly.abs().max(0.002);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        DragKind::SpotNew => {
            let sp = map.to_s(p);
            if let Some(spot) = s.retouch.last_mut() {
                let last = *spot.points.last().unwrap();
                let d = ((sp[0] as f32 - last[0]).powi(2) + (sp[1] as f32 - last[1]).powi(2)).sqrt();
                if d > spot.radius * 0.3 {
                    spot.points.push([sp[0] as f32, sp[1] as f32]);
                }
            }
        }
        DragKind::SpotSource(i) => {
            let sp = map.to_s(p);
            if let Some(spot) = s.retouch.get_mut(i) {
                let c = spot.points[0];
                spot.source_offset = [sp[0] as f32 - c[0], sp[1] as f32 - c[1]];
            }
        }
        DragKind::SpotMove(i) => {
            let a = map.to_s(start);
            let b = map.to_s(p);
            if let Some(spot) = s.retouch.get_mut(i) {
                let first = spot.points[0];
                let target = [first[0] + (b[0] - a[0]) as f32, first[1] + (b[1] - a[1]) as f32];
                let d = [target[0] - first[0], target[1] - first[1]];
                for pt in spot.points.iter_mut() {
                    pt[0] += d[0];
                    pt[1] += d[1];
                }
                app.drag.start = p;
                return;
            }
        }
        DragKind::Tat => {
            let delta = -dy_tat * 0.4;
            match pick {
                Pick::TatCurve => match curve_mode {
                    CurveMode::Parametric => {
                        let v = tat_value / 255.0;
                        let p = &mut s.parametric;
                        let target = if v < p.shadow_split / 100.0 {
                            &mut p.shadows
                        } else if v < p.midtone_split / 100.0 {
                            &mut p.darks
                        } else if v < p.highlight_split / 100.0 {
                            &mut p.lights
                        } else {
                            &mut p.highlights
                        };
                        *target = (*target + delta).clamp(-100.0, 100.0);
                    }
                    m => {
                        let curve = match m {
                            CurveMode::Red => &mut s.curve.red,
                            CurveMode::Green => &mut s.curve.green,
                            CurveMode::Blue => &mut s.curve.blue,
                            _ => &mut s.curve.rgb,
                        };
                        let x = tat_value.round();
                        let idx = match curve.iter().position(|q| (q[0] - x).abs() < 3.0) {
                            Some(i) => i,
                            None => {
                                let y = darkslide_core::pipeline::curves::point_curve_lut(curve).eval(x / 255.0) * 255.0;
                                curve.push([x, y]);
                                curve.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
                                curve.iter().position(|q| q[0] == x).unwrap_or(0)
                            }
                        };
                        curve[idx][1] = (curve[idx][1] + delta * 0.6).clamp(0.0, 255.0);
                    }
                },
                Pick::TatMixer(tab) => {
                    // ACR band hues are HSV hues; weights by distance to each band
                    let bw = darkslide_core::pipeline::colorops::band_weights(tat_hue);
                    for (k, w) in bw.iter().enumerate() {
                        if *w < 0.05 {
                            continue;
                        }
                        let v = match tab {
                            0 => &mut s.hsl[k].hue,
                            1 => &mut s.hsl[k].saturation,
                            2 => &mut s.hsl[k].luminance,
                            _ => &mut s.gray_mix[k],
                        };
                        *v = (*v + delta * w).clamp(-100.0, 100.0).round();
                    }
                }
                _ => {}
            }
            app.drag.tat_last_y = p.y;
        }
        _ => {}
    }
}

fn end_tool_drag(app: &mut App, map: &ViewMap, p: Pos2) {
    let kind = app.drag.kind;
    // once drawn, a gradient is edited by its handles; the next drag doesn't make another
    if matches!(kind, DragKind::LinearNew | DragKind::RadialNew) {
        app.tools.mask_tool = MaskTool::None;
        app.tools.place_comp = None;
    }
    // a new mask folds the create grid away so its sliders are in view
    if matches!(kind, DragKind::LinearNew | DragKind::RadialNew | DragKind::Brush | DragKind::ObjectBox) {
        app.tools.show_create = false;
    }
    let start = app.drag.start;
    let pet = app.tools.pet_eye;
    let Some(doc) = app.doc_mut() else { return };
    let Some(engine) = doc.engine.clone() else { return };
    let s = &mut doc.settings;
    match kind {
        DragKind::Straighten => {
            let a = start;
            let b = p;
            if a.distance(b) > 10.0 {
                // measure in image pixel space
                let ta = map.to_t(a);
                let tb = map.to_t(b);
                let dx = (tb[0] - ta[0]) * map.geo.ow;
                let dy = (tb[1] - ta[1]) * map.geo.oh;
                let mut ang = dy.atan2(dx).to_degrees();
                if ang > 90.0 {
                    ang -= 180.0;
                } else if ang < -90.0 {
                    ang += 180.0;
                }
                if ang.abs() > 45.0 {
                    ang = if ang > 0.0 { ang - 90.0 } else { ang + 90.0 };
                }
                s.crop.angle = (ang as f32 * 100.0).round() / 100.0;
                tool_panels::fit_crop_inside(s, &engine);
            }
            app.tools.straighten = false;
        }
        DragKind::SpotNew => {
            if let Some(spot) = s.retouch.last_mut() {
                if spot.mode != RetouchMode::Remove {
                    let base = engine.base(darkslide_core::demosaic::Quality::Draft);
                    spot.source_offset = darkslide_core::retouch::find_source(&base, spot);
                }
            }
        }
        DragKind::RedEye => {
            let a = map.to_t(start);
            let b = map.to_t(p);
            let c = [((a[0] + b[0]) / 2.0) as f32, ((a[1] + b[1]) / 2.0) as f32];
            // size follows the on-screen box even when the crop is rotated
            let sc = map.scale().max(1e-6) as f64;
            let r = [((p.x - start.x).abs() as f64 / 2.0 / sc / map.geo.ow) as f32, ((p.y - start.y).abs() as f64 / 2.0 / sc / map.geo.oh) as f32];
            if r[0] > 0.002 && r[1] > 0.002 {
                s.red_eye.push(RedEyeSpot { center: c, radius: r, pupil_size: 50.0, darken: 50.0, pet, catchlight: pet });
                app.tools.selected_red_eye = Some(s.red_eye.len() - 1);
            }
        }
        DragKind::ObjectBox => {
            let a = map.to_t(start);
            let b = map.to_t(p);
            if start.distance(p) > 8.0 {
                let hint = [a[0].min(b[0]) as f32, a[1].min(b[1]) as f32, a[0].max(b[0]) as f32, a[1].max(b[1]) as f32];
                let shape = MaskShape::Ai { kind: AiMaskKind::Object, feather: 0.0, edge: 0.0, hint: Some(hint), raster: None };
                s.masks.push(MaskGroup {
                    name: format!("Object {}", s.masks.len() + 1),
                    components: vec![MaskComponent { shape, ..Default::default() }],
                    ..Default::default()
                });
                app.tools.selected_mask = Some(s.masks.len() - 1);
                app.tools.selected_comp = Some(0);
                app.tools.mask_tool = MaskTool::None;
            }
        }
        DragKind::Guide => {
            let a = map.to_o(start);
            let b = map.to_o(p);
            if start.distance(p) > 10.0 && s.transform.guides.len() < 4 {
                s.transform.guides.push([a[0] as f32, a[1] as f32, b[0] as f32, b[1] as f32]);
                s.transform.upright = Upright::Guided;
                if s.transform.guides.len() >= 2 {
                    app.pending_actions.push(crate::edit_panel::Action::UprightAnalyze(Upright::Guided));
                }
            }
        }
        _ => {}
    }
}

fn click(app: &mut App, map: &ViewMap, p: Pos2) {
    let c = map.to_crop(p);
    if !(0.0..=1.0).contains(&c[0]) || !(0.0..=1.0).contains(&c[1]) {
        return;
    }
    let rgb = app.sample_fit([c[0] as f32, c[1] as f32]);
    let pick = app.pick;
    let tool = app.tools.tool;
    let (sel_mask, sel_comp) = (app.tools.selected_mask, app.tools.selected_comp);
    let lch = rgb.map(|rgb| {
        let lin = rgb.map(|v| darkslide_core::color::srgb_decode(v as f32 / 255.0));
        let pp = darkslide_core::color::mul_mv(&darkslide_core::color::to_prophoto(darkslide_core::color::ColorSpace::SRgb), lin);
        darkslide_core::color::lab_to_lch(darkslide_core::color::Oklab::new().from_prophoto(pp))
    });
    if tool == Tool::RedEye {
        app.tools.selected_red_eye = red_eye_at(app, map, p).map(|(i, _)| i);
        return;
    }
    if tool == Tool::Masking && pick == Pick::None {
        if let Some(i) = pin_at(app, map, p) {
            if let Some(d) = app.docs.get(app.current.unwrap_or(0)) {
                let s = d.settings.clone();
                select_mask(&mut app.tools, &s, i);
            }
            return;
        }
    }
    match pick {
        Pick::WhiteBalance => {
            if let Some(doc) = app.doc_mut() {
                if let Some(e) = doc.engine.clone() {
                    if let Some((t, ti)) = e.wb_from_point(&doc.settings, c[0] as f32, c[1] as f32) {
                        doc.settings.wb_mode = WbMode::Custom;
                        doc.settings.temperature = t;
                        doc.settings.tint = ti;
                    }
                }
            }
        }
        Pick::PointColor => {
            if let (Some(lch), Some(doc)) = (lch, app.doc_mut()) {
                let pc = PointColor { sample: lch, ..Default::default() };
                let list = match (tool, sel_mask) {
                    (Tool::Masking, Some(mi)) if mi < doc.settings.masks.len() => &mut doc.settings.masks[mi].adjust.point_colors,
                    _ => &mut doc.settings.point_colors,
                };
                if list.len() < 8 {
                    list.push(pc);
                    let n = list.len() - 1;
                    app.edit.point_sel = n;
                    app.tools.mask_point_sel = n;
                }
            }
        }
        Pick::Sampler => {
            let Some(lch) = lch else { return };
            let mut added = false;
            if let Some(doc) = app.doc_mut() {
                if let (Some(mi), Some(ci)) = (sel_mask, sel_comp) {
                    if let Some(MaskShape::ColorRange { samples, .. }) =
                        doc.settings.masks.get_mut(mi).and_then(|m| m.components.get_mut(ci)).map(|c| &mut c.shape)
                    {
                        samples.push(darkslide_core::color::lch_to_lab(lch));
                        added = true;
                    }
                }
            }
            if !added && app.samplers.len() < 9 {
                app.samplers.push([c[0] as f32, c[1] as f32]);
            }
        }
        Pick::TatCurve | Pick::TatMixer(_) => {}
        _ if matches!(tool, Tool::Edit | Tool::Presets | Tool::Snapshots) => {
            // a click toggles Fit / 100% around the clicked point
            app.zoom = if app.zoom == Zoom::Fit { Zoom::Scale(1.0) } else { Zoom::Fit };
            app.center = [c[0] as f32, c[1] as f32];
        }
        _ => {
            if tool == Tool::Remove {
                // a click without drag already created a spot in begin_tool_drag? clicks don't start drags
                let s = map.to_s(p);
                let tu = &app.tools;
                let spot = RetouchSpot {
                    mode: tu.remove_mode,
                    points: vec![[s[0] as f32, s[1] as f32]],
                    radius: tu.spot_size,
                    feather: tu.spot_feather,
                    opacity: tu.spot_opacity,
                    source_offset: [0.0, 0.0],
                    mask: None,
                };
                // select an existing spot instead if clicked on it
                let mut hit = None;
                if let Some(doc) = app.doc() {
                    for (i, sp) in doc.settings.retouch.iter().enumerate() {
                        if let Some(c0) = sp.points.first() {
                            if map.from_s([c0[0] as f64, c0[1] as f64]).distance(p) < (sp.radius * map.long_edge() * map.scale()).max(6.0) {
                                hit = Some(i);
                            }
                        }
                    }
                }
                if let Some(i) = hit {
                    app.tools.selected_spot = Some(i);
                    return;
                }
                if let Some(doc) = app.doc_mut() {
                    let mut spot = spot;
                    if spot.mode != RetouchMode::Remove {
                        if let Some(e) = &doc.engine {
                            spot.source_offset = darkslide_core::retouch::find_source(&e.base(darkslide_core::demosaic::Quality::Draft), &spot);
                        }
                    }
                    doc.settings.retouch.push(spot);
                    app.tools.selected_spot = Some(doc.settings.retouch.len() - 1);
                }
            }
        }
    }
}

/// Remember the pixel under the pointer when a targeted adjustment starts.
pub fn tat_begin(app: &mut App, map: &ViewMap, p: Pos2) {
    let c = map.to_crop(p);
    if let Some(rgb) = app.sample_fit([c[0] as f32, c[1] as f32]) {
        let y = 0.2126 * rgb[0] as f32 + 0.7152 * rgb[1] as f32 + 0.0722 * rgb[2] as f32;
        app.drag.tat_value = y;
        let lin = rgb.map(|v| darkslide_core::color::srgb_decode(v as f32 / 255.0));
        let pp = darkslide_core::color::mul_mv(&darkslide_core::color::to_prophoto(darkslide_core::color::ColorSpace::SRgb), lin);
        let lch = darkslide_core::color::lab_to_lch(darkslide_core::color::Oklab::new().from_prophoto(pp));
        app.drag.tat_hue = lch[2];
    }
    app.drag.tat_last_y = p.y;
}

fn draw_overlays(app: &mut App, painter: &egui::Painter, map: &ViewMap, hover: Option<Pos2>) {
    let Some(doc) = app.doc() else { return };
    let s = &doc.settings;
    let white = Stroke::new(1.0, Color32::from_white_alpha(220));
    let shadow = Stroke::new(3.0, Color32::from_black_alpha(90));
    // TAT start bookkeeping happens here so it has the map
    if app.drag.kind == DragKind::Tat && app.drag.tat_last_y == 0.0 {
        let p = app.drag.start;
        let _ = p;
    }
    match app.tools.tool {
        Tool::Crop => {
            let corners = crop_corners_screen(map);
            // dim everything outside the (upright) crop box
            let cr = Rect::from_points(&corners);
            let o = painter.clip_rect();
            let dim = Color32::from_black_alpha(185);
            for r in [
                Rect::from_min_max(o.min, pos2(o.max.x, cr.min.y)),
                Rect::from_min_max(pos2(o.min.x, cr.max.y), o.max),
                Rect::from_min_max(pos2(o.min.x, cr.min.y), pos2(cr.min.x, cr.max.y)),
                Rect::from_min_max(pos2(cr.max.x, cr.min.y), pos2(o.max.x, cr.max.y)),
            ] {
                if r.is_positive() {
                    painter.rect_filled(r, 0.0, dim);
                }
            }
            for i in 0..4 {
                let (a, b) = (corners[i], corners[(i + 1) % 4]);
                painter.line_segment([a, b], shadow);
                painter.line_segment([a, b], Stroke::new(1.5, Color32::WHITE));
            }
            // overlay guides
            let lerp2 = |u: f32, v: f32| {
                let top = corners[0].lerp(corners[1], u);
                let bot = corners[3].lerp(corners[2], u);
                top.lerp(bot, v)
            };
            let thin = Stroke::new(0.8, Color32::from_white_alpha(120));
            match app.tools.crop_overlay {
                CropOverlay::Thirds => {
                    for f in [1.0 / 3.0, 2.0 / 3.0] {
                        painter.line_segment([lerp2(f, 0.0), lerp2(f, 1.0)], thin);
                        painter.line_segment([lerp2(0.0, f), lerp2(1.0, f)], thin);
                    }
                }
                CropOverlay::Grid => {
                    for i in 1..8 {
                        let f = i as f32 / 8.0;
                        painter.line_segment([lerp2(f, 0.0), lerp2(f, 1.0)], thin);
                        painter.line_segment([lerp2(0.0, f), lerp2(1.0, f)], thin);
                    }
                }
                CropOverlay::Golden => {
                    for f in [0.382, 0.618] {
                        painter.line_segment([lerp2(f, 0.0), lerp2(f, 1.0)], thin);
                        painter.line_segment([lerp2(0.0, f), lerp2(1.0, f)], thin);
                    }
                }
                CropOverlay::Diagonal => {
                    painter.line_segment([lerp2(0.0, 0.0), lerp2(1.0, 1.0)], thin);
                    painter.line_segment([lerp2(1.0, 0.0), lerp2(0.0, 1.0)], thin);
                }
                CropOverlay::Triangle => {
                    // the diagonal plus perpendiculars to it from the other two corners
                    let (w, h) = (corners[0].distance(corners[1]), corners[0].distance(corners[3]));
                    let (a, b) = (w * w / (w * w + h * h), h * h / (w * w + h * h));
                    painter.line_segment([lerp2(0.0, 0.0), lerp2(1.0, 1.0)], thin);
                    painter.line_segment([lerp2(1.0, 0.0), lerp2(a, a)], thin);
                    painter.line_segment([lerp2(0.0, 1.0), lerp2(1.0 - b, 1.0 - b)], thin);
                }
                CropOverlay::None => {}
            }
            for (i, c) in corners.iter().enumerate() {
                painter.rect_filled(Rect::from_center_size(*c, Vec2::splat(9.0)), 1.0, Color32::WHITE);
                let m = c.lerp(corners[(i + 1) % 4], 0.5);
                painter.rect_filled(Rect::from_center_size(m, Vec2::splat(7.0)), 1.0, Color32::WHITE);
            }
            let label = |at: Pos2, align: egui::Align2, text: String| {
                let font = egui::FontId::proportional(12.0);
                let r = painter.text(at, align, &text, font.clone(), Color32::TRANSPARENT);
                painter.rect_filled(r.expand2(vec2(5.0, 2.0)), 3.0, Color32::from_black_alpha(170));
                painter.text(at, align, text, font, Color32::WHITE);
            };
            if app.drag.kind == DragKind::Straighten {
                if let Some(h) = hover {
                    painter.line_segment([app.drag.start, h], Stroke::new(1.5, Color32::YELLOW));
                    let (ta, tb) = (map.to_t(app.drag.start), map.to_t(h));
                    let mut a = ((tb[1] - ta[1]) * map.geo.oh).atan2((tb[0] - ta[0]) * map.geo.ow).to_degrees();
                    if a > 90.0 {
                        a -= 180.0;
                    } else if a < -90.0 {
                        a += 180.0;
                    }
                    if a.abs() > 45.0 {
                        a = if a > 0.0 { a - 90.0 } else { a + 90.0 };
                    }
                    label(h + vec2(12.0, -12.0), egui::Align2::LEFT_BOTTOM, format!("{a:.2}°"));
                }
            }
            let (cw, ch) = map.geo.crop_size();
            // below the box, or inside it when the box reaches the bottom of the canvas
            let below = corners[2].y + 24.0 < painter.clip_rect().max.y;
            let (at, align) = if below { (corners[2] + vec2(-2.0, 6.0), egui::Align2::RIGHT_TOP) } else { (corners[2] + vec2(-8.0, -8.0), egui::Align2::RIGHT_BOTTOM) };
            label(at, align, format!("{:.0} × {:.0}  {:.2}°", cw, ch, s.crop.angle));
        }
        Tool::Masking => {
            if let (Some(mi), Some(ci)) = (app.tools.selected_mask, app.tools.selected_comp) {
                if let Some(c) = s.masks.get(mi).and_then(|m| m.components.get(ci)) {
                    match &c.shape {
                        MaskShape::Linear { start, end, two_sided } => {
                            let ps = map.from_t([start[0] as f64, start[1] as f64]);
                            let pe = map.from_t([end[0] as f64, end[1] as f64]);
                            let d = pe - ps;
                            let n = vec2(-d.y, d.x).normalized() * 2000.0;
                            for (pt, st) in [(ps, Stroke::new(1.5, Color32::WHITE)), (pe, white)] {
                                painter.line_segment([pt - n, pt + n], st);
                            }
                            if *two_sided {
                                let pm = ps - d;
                                painter.line_segment([pm - n, pm + n], white);
                            }
                            painter.line_segment([ps, pe], Stroke::new(1.0, Color32::from_white_alpha(140)));
                            for pt in [ps, pe, ps.lerp(pe, 0.5)] {
                                painter.circle_filled(pt, 5.0, crate::widgets::ACCENT);
                                painter.circle_stroke(pt, 5.0, Stroke::new(1.0, Color32::WHITE));
                            }
                        }
                        MaskShape::Radial { center, radius, angle, feather } => {
                            let ring = |k: f32| -> Vec<Pos2> {
                                (0..=72)
                                    .map(|i| {
                                        let a = i as f32 / 72.0 * std::f32::consts::TAU;
                                        radial_pt(map, *center, *radius, *angle, a.cos() * k, a.sin() * k)
                                    })
                                    .collect()
                            };
                            painter.add(egui::Shape::line(ring(1.0), Stroke::new(1.5, Color32::WHITE)));
                            // where the feathering starts
                            let f = (feather / 100.0).clamp(0.0, 1.0);
                            if f > 0.02 {
                                let inner = ring(1.0 - f);
                                for seg in inner.chunks(2) {
                                    if let [a, b] = seg {
                                        painter.line_segment([*a, *b], Stroke::new(1.0, Color32::from_white_alpha(150)));
                                    }
                                }
                            }
                            let hs = gradient_handles(&c.shape, map);
                            painter.line_segment([hs[4].1, hs[5].1], Stroke::new(1.0, Color32::from_white_alpha(150)));
                            for (h, pt) in hs {
                                let r = match h {
                                    0 => 6.0,
                                    5 => 4.0,
                                    _ => 4.5,
                                };
                                painter.circle_filled(pt, r, if h == 5 { Color32::WHITE } else { crate::widgets::ACCENT });
                                painter.circle_stroke(pt, r, Stroke::new(1.0, Color32::WHITE));
                            }
                        }
                        _ => {}
                    }
                }
            }
            // a pin for every other mask that has a place, to pick it on the photo
            for (i, m) in s.masks.iter().enumerate() {
                if Some(i) == app.tools.selected_mask {
                    continue;
                }
                if let Some(a) = mask_anchor(m) {
                    let pt = map.from_t([a[0] as f64, a[1] as f64]);
                    painter.circle_filled(pt, 5.5, Color32::from_gray(40));
                    painter.circle_stroke(pt, 5.5, Stroke::new(1.5, Color32::WHITE));
                }
            }
            if app.tools.mask_tool == MaskTool::Brush {
                if let Some(h) = hover {
                    let r = app.tools.brush.size * map.long_edge() * map.scale();
                    painter.circle_stroke(h, r, Stroke::new(1.0, Color32::WHITE));
                    painter.circle_stroke(h, r * (1.0 - app.tools.brush.feather / 100.0), Stroke::new(1.0, Color32::from_white_alpha(100)));
                    // erasing shows a minus in the ring
                    if app.tools.brush.erase || painter.ctx().input(|i| i.modifiers.alt) {
                        painter.line_segment([h - vec2(5.0, 0.0), h + vec2(5.0, 0.0)], Stroke::new(1.5, Color32::WHITE));
                    } else {
                        painter.line_segment([h - vec2(4.0, 0.0), h + vec2(4.0, 0.0)], Stroke::new(1.0, Color32::from_white_alpha(160)));
                        painter.line_segment([h - vec2(0.0, 4.0), h + vec2(0.0, 4.0)], Stroke::new(1.0, Color32::from_white_alpha(160)));
                    }
                }
            }
        }
        Tool::Remove => {
            if app.tools.show_spot_overlay {
                for (i, sp) in s.retouch.iter().enumerate() {
                    let sel = app.tools.selected_spot == Some(i);
                    let r = sp.radius * map.long_edge() * map.scale();
                    let col = if sel { Color32::WHITE } else { Color32::from_white_alpha(140) };
                    let dst: Vec<Pos2> = sp.points.iter().map(|q| map.from_s([q[0] as f64, q[1] as f64])).collect();
                    if dst.len() == 1 {
                        painter.circle_stroke(dst[0], r, Stroke::new(if sel { 1.8 } else { 1.0 }, col));
                    } else {
                        painter.add(egui::Shape::line(dst.clone(), Stroke::new((r * 2.0).max(1.0), Color32::from_white_alpha(if sel { 70 } else { 35 }))));
                    }
                    if sel && sp.mode != RetouchMode::Remove {
                        let src: Vec<Pos2> =
                            sp.points.iter().map(|q| map.from_s([(q[0] + sp.source_offset[0]) as f64, (q[1] + sp.source_offset[1]) as f64])).collect();
                        if src.len() == 1 {
                            painter.circle_stroke(src[0], r, Stroke::new(1.2, crate::widgets::ACCENT));
                        } else {
                            painter.add(egui::Shape::line(src.clone(), Stroke::new(1.2, crate::widgets::ACCENT)));
                        }
                        painter.arrow(src[0], dst[0] - src[0], Stroke::new(1.0, Color32::from_white_alpha(160)));
                    }
                }
            }
            if let Some(h) = hover {
                let r = app.tools.spot_size * map.long_edge() * map.scale();
                painter.circle_stroke(h, r, Stroke::new(1.0, Color32::WHITE));
            }
        }
        Tool::RedEye => {
            for (i, re) in s.red_eye.iter().enumerate() {
                let sel = app.tools.selected_red_eye == Some(i);
                let pts: Vec<Pos2> = (0..=48)
                    .map(|i| {
                        let a = i as f32 / 48.0 * std::f32::consts::TAU;
                        map.from_t([(re.center[0] + a.cos() * re.radius[0]) as f64, (re.center[1] + a.sin() * re.radius[1]) as f64])
                    })
                    .collect();
                if sel {
                    painter.add(egui::Shape::line(pts.clone(), Stroke::new(3.0, Color32::from_black_alpha(120))));
                }
                painter.add(egui::Shape::line(pts, Stroke::new(if sel { 2.0 } else { 1.2 }, if sel { crate::widgets::ACCENT } else { Color32::WHITE })));
            }
            if app.drag.kind == DragKind::RedEye || app.drag.kind == DragKind::ObjectBox {
                if let Some(h) = hover {
                    painter.rect_stroke(Rect::from_two_pos(app.drag.start, h), 0.0, white, StrokeKind::Middle);
                }
            }
        }
        _ => {}
    }
    if app.drag.kind == DragKind::ObjectBox {
        if let Some(h) = hover {
            painter.rect_stroke(Rect::from_two_pos(app.drag.start, h), 0.0, Stroke::new(1.5, crate::widgets::ACCENT), StrokeKind::Middle);
        }
    }
    if app.grid && app.tools.tool != Tool::Crop {
        let r = map.rect;
        let st = Stroke::new(0.6, Color32::from_white_alpha(70));
        for i in 1..6 {
            let f = i as f32 / 6.0;
            painter.line_segment([pos2(r.left() + r.width() * f, r.top()), pos2(r.left() + r.width() * f, r.bottom())], st);
            painter.line_segment([pos2(r.left(), r.top() + r.height() * f), pos2(r.right(), r.top() + r.height() * f)], st);
        }
    }
    // guided upright lines
    if s.transform.upright == Upright::Guided || app.pick == Pick::Upright {
        for g in &s.transform.guides {
            let a = map.from_o([g[0] as f64, g[1] as f64]);
            let b = map.from_o([g[2] as f64, g[3] as f64]);
            painter.line_segment([a, b], Stroke::new(1.5, Color32::from_rgb(80, 220, 120)));
        }
        if app.drag.kind == DragKind::Guide {
            if let Some(h) = hover {
                painter.line_segment([app.drag.start, h], Stroke::new(1.5, Color32::from_rgb(80, 220, 120)));
            }
        }
    }
    // samplers
    for (i, sm) in app.samplers.iter().enumerate() {
        let p = map.from_crop([sm[0] as f64, sm[1] as f64]);
        painter.circle_stroke(p, 6.0, Stroke::new(1.5, Color32::WHITE));
        painter.text(p + vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, format!("{}", i + 1), egui::FontId::proportional(11.0), Color32::WHITE);
    }
    // HDR visualisation hint
    if s.hdr.enabled && s.hdr.visualize {
        painter.text(
            map.rect.left_bottom() + vec2(6.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            "HDR preview is tone-mapped to this display",
            egui::FontId::proportional(11.0),
            Color32::from_white_alpha(160),
        );
    }
}
