//! Right-hand panels for the non-Edit tools: Crop, Remove, Masking, Red Eye,
//! Presets and Snapshots.

use darkslide_core::params::*;
use darkslide_core::xmp::Preset;
use eframe::egui::{self, Color32, RichText};
use egui_phosphor::regular as ic;

use crate::doc::Doc;
use crate::edit_panel::{self, EditUi, Pick};
use crate::widgets::{self, SliderSpec, slider};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Edit,
    Crop,
    Remove,
    Masking,
    RedEye,
    Presets,
    Snapshots,
}

impl Tool {
    pub const ALL: [(Tool, &'static str, &'static str); 7] = [
        (Tool::Edit, ic::SLIDERS_HORIZONTAL, "Edit (E)"),
        (Tool::Crop, ic::CROP, "Crop & rotate (C)"),
        (Tool::Remove, ic::BANDAIDS, "Remove (B)"),
        (Tool::Masking, ic::CIRCLE_DASHED, "Masking (M)"),
        (Tool::RedEye, ic::EYE, "Red eye"),
        (Tool::Presets, ic::INTERSECT, "Presets (P)"),
        (Tool::Snapshots, ic::STACK, "Snapshots (S)"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CropOverlay {
    #[default]
    Thirds,
    Grid,
    Golden,
    Diagonal,
    Triangle,
    None,
}

impl CropOverlay {
    pub const ALL: [CropOverlay; 6] = [CropOverlay::Thirds, CropOverlay::Grid, CropOverlay::Golden, CropOverlay::Diagonal, CropOverlay::Triangle, CropOverlay::None];
    pub fn label(self) -> &'static str {
        match self {
            CropOverlay::Thirds => "Rule of thirds",
            CropOverlay::Grid => "Grid",
            CropOverlay::Golden => "Golden ratio",
            CropOverlay::Diagonal => "Diagonal",
            CropOverlay::Triangle => "Golden triangle",
            CropOverlay::None => "None",
        }
    }
    /// O cycles the overlays, as in Lightroom.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&o| o == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MaskTool {
    #[default]
    None,
    Brush,
    Linear,
    Radial,
    /// Drag a box around an object to select it.
    ObjectBox,
}

pub struct BrushParams {
    pub size: f32,
    pub feather: f32,
    pub flow: f32,
    pub density: f32,
    pub auto_mask: bool,
    pub erase: bool,
}

pub struct ToolUi {
    pub tool: Tool,
    pub crop_overlay: CropOverlay,
    pub straighten: bool,
    /// Crop before the angle slider started moving.
    pub angle_base: Option<Crop>,
    pub crop_note: Option<String>,
    pub remove_mode: RetouchMode,
    pub spot_size: f32,
    pub spot_feather: f32,
    pub spot_opacity: f32,
    pub selected_spot: Option<usize>,
    /// The spot whose settings the panel last loaded.
    pub spot_loaded: Option<usize>,
    pub visualize_spots: bool,
    pub visualize_threshold: f32,
    pub show_spot_overlay: bool,
    pub selected_mask: Option<usize>,
    pub selected_comp: Option<usize>,
    pub mask_tool: MaskTool,
    /// A component added from Add / Subtract / Intersect, drawn by the next drag.
    pub place_comp: Option<(usize, usize)>,
    pub show_create: bool,
    pub renaming: Option<usize>,
    pub select_request: Option<usize>,
    pub brush: BrushParams,
    pub show_mask_overlay: bool,
    pub mask_overlay_color: Color32,
    pub pet_eye: bool,
    pub selected_red_eye: Option<usize>,
    pub presets: Vec<Preset>,
    pub presets_loaded: bool,
    pub preset_amount: f32,
    pub new_preset_name: String,
    pub new_preset_group: String,
    pub snapshot_name: String,
    pub mask_grading_view: u8,
    pub mask_point_sel: usize,
    pub request_ai: Option<(usize, AiMaskKind)>,
    /// Settings to show while hovering a preset (not committed).
    pub hover_preview: Option<DevelopSettings>,
    pub status: Option<String>,
    pub preset_ui: crate::presets_panel::PresetUi,
}

impl Default for ToolUi {
    fn default() -> Self {
        Self {
            tool: Tool::Edit,
            crop_overlay: CropOverlay::Thirds,
            straighten: false,
            angle_base: None,
            crop_note: None,
            remove_mode: RetouchMode::Remove,
            spot_size: 0.012,
            spot_feather: 30.0,
            spot_opacity: 100.0,
            selected_spot: None,
            spot_loaded: None,
            visualize_spots: false,
            visualize_threshold: 50.0,
            show_spot_overlay: true,
            selected_mask: None,
            selected_comp: None,
            mask_tool: MaskTool::None,
            place_comp: None,
            show_create: false,
            renaming: None,
            select_request: None,
            brush: BrushParams { size: 0.03, feather: 50.0, flow: 50.0, density: 100.0, auto_mask: false, erase: false },
            show_mask_overlay: true,
            mask_overlay_color: Color32::from_rgba_unmultiplied(255, 40, 40, 120),
            pet_eye: false,
            selected_red_eye: None,
            presets: vec![],
            presets_loaded: false,
            preset_amount: 100.0,
            new_preset_name: String::new(),
            new_preset_group: String::new(),
            snapshot_name: String::new(),
            mask_grading_view: 0,
            mask_point_sel: 0,
            request_ai: None,
            hover_preview: None,
            status: None,
            preset_ui: Default::default(),
        }
    }
}

const ASPECTS: [(&str, Option<[f32; 2]>); 9] = [
    ("Free", None),
    ("As shot", Some([0.0, 0.0])),
    ("1 : 1", Some([1.0, 1.0])),
    ("4 : 5", Some([4.0, 5.0])),
    ("8.5 : 11", Some([8.5, 11.0])),
    ("5 : 7", Some([5.0, 7.0])),
    ("2 : 3", Some([2.0, 3.0])),
    ("16 : 9", Some([16.0, 9.0])),
    ("16 : 10", Some([16.0, 10.0])),
];

pub fn crop_panel(ui: &mut egui::Ui, doc: &mut Doc, t: &mut ToolUi) {
    let Some(engine) = doc.engine.clone() else { return };
    let s = &mut doc.settings;
    ui.horizontal(|ui| {
        ui.heading("Crop & Rotate");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let untouched = s.crop == Crop::default();
            let reset = egui::Button::new(format!("{}  Reset", ic::ARROW_COUNTER_CLOCKWISE));
            if ui.add_enabled(!untouched, reset).on_hover_text("Back to the uncropped photo: no crop, angle, rotation or flip").clicked() {
                s.crop = Crop::default();
            }
        });
    });
    ui.add_space(4.0);
    let (fw, fh) = engine.full_size();
    let landscape = {
        let g = engine.geometry(s, fw, fh, true);
        (s.crop.right - s.crop.left) * g.ow as f32 >= (s.crop.bottom - s.crop.top) * g.oh as f32
    };
    ui.horizontal(|ui| {
        ui.label("Aspect");
        // ratios are listed portrait-style; match them to the box's orientation
        let oriented = |a: Option<[f32; 2]>| match a {
            Some([w, h]) if w > 0.0 && (w > h) != landscape && w != h => Some([h, w]),
            a => a,
        };
        let cur = ASPECTS.iter().find(|(_, a)| oriented(*a) == s.crop.aspect || *a == s.crop.aspect).map(|(n, _)| *n).unwrap_or("Custom");
        egui::ComboBox::from_id_salt("aspect").width(120.0).height(400.0).selected_text(cur).show_ui(ui, |ui| {
            for (n, a) in ASPECTS {
                if ui.selectable_label(cur == n, n).clicked() {
                    s.crop.aspect = oriented(a);
                    apply_aspect(s, &engine);
                }
            }
        });
        let locked = s.crop.aspect.is_some();
        let lock_icon = if locked { ic::LOCK_SIMPLE } else { ic::LOCK_SIMPLE_OPEN };
        if ui.selectable_label(locked, lock_icon).on_hover_text(if locked { "Unlock the ratio" } else { "Lock the current ratio" }).clicked() {
            s.crop.aspect = if locked {
                None
            } else {
                let g = engine.geometry(s, fw, fh, true);
                Some([(s.crop.right - s.crop.left) * g.ow as f32, (s.crop.bottom - s.crop.top) * g.oh as f32])
            };
        }
        if ui.button(ic::ARROWS_LEFT_RIGHT).on_hover_text("Swap orientation (X)").clicked() {
            swap_orientation(s, &engine);
        }
    });
    // always present so the panel never jumps; editable for a fixed ratio
    let mut a = s.crop.aspect.filter(|a| a[0] > 0.0).unwrap_or([0.0, 0.0]);
    let editable = a[0] > 0.0;
    let changed = ui
        .add_enabled_ui(editable, |ui| {
            ui.horizontal(|ui| {
                ui.label("Ratio");
                let c1 = ui.add(egui::DragValue::new(&mut a[0]).range(0.1..=100.0).speed(0.05).max_decimals(2)).changed();
                ui.label(":");
                let c2 = ui.add(egui::DragValue::new(&mut a[1]).range(0.1..=100.0).speed(0.05).max_decimals(2)).changed();
                c1 || c2
            })
            .inner
        })
        .inner;
    if changed && editable {
        s.crop.aspect = Some(a);
        apply_aspect(s, &engine);
    }
    ui.add_space(6.0);
    // the angle slider turns from the box it started with, so it can grow back
    if !ui.input(|i| i.pointer.any_down()) {
        t.angle_base = None;
    }
    let mut angle = s.crop.angle;
    if slider(ui, &mut angle, SliderSpec::new("Angle", -45.0, 45.0, 0.0).dec(2)) {
        let base = *t.angle_base.get_or_insert(s.crop);
        s.crop = base;
        s.crop.angle = angle;
        fit_crop_inside(s, &engine);
    }
    ui.horizontal(|ui| {
        if ui.selectable_label(t.straighten, format!("{}  Straighten tool", ic::RULER)).on_hover_text("Draw a line along the horizon (or Ctrl-drag)").clicked() {
            t.straighten = !t.straighten;
        }
        if ui.button("Auto").on_hover_text("Level the horizon automatically").clicked() {
            match darkslide_core::upright::auto_straighten_angle(&engine, s) {
                Some(a) if (a - s.crop.angle).abs() > 0.005 => {
                    s.crop.angle = a;
                    fit_crop_inside(s, &engine);
                    t.crop_note = Some(format!("Straightened by {a:.2}°"));
                }
                Some(_) => t.crop_note = Some("Already level".into()),
                None => t.crop_note = Some("No clear horizon or verticals found".into()),
            }
        }
    });
    // reserved line, so the note appearing doesn't move anything
    ui.label(RichText::new(t.crop_note.clone().unwrap_or_default()).small().weak());
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if ui.button(ic::ARROW_COUNTER_CLOCKWISE).on_hover_text("Rotate 90° counter-clockwise").clicked() {
            rotate90(s, 3);
        }
        if ui.button(ic::ARROW_CLOCKWISE).on_hover_text("Rotate 90° clockwise").clicked() {
            rotate90(s, 1);
        }
        if ui.button(ic::FLIP_HORIZONTAL).on_hover_text("Flip horizontal").clicked() {
            s.crop.flip_h = !s.crop.flip_h;
        }
        if ui.button(ic::FLIP_VERTICAL).on_hover_text("Flip vertical").clicked() {
            s.crop.flip_v = !s.crop.flip_v;
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Overlay");
        egui::ComboBox::from_id_salt("overlay")
            .width(140.0)
            .selected_text(t.crop_overlay.label())
            .show_ui(ui, |ui| {
                for o in CropOverlay::ALL {
                    ui.selectable_value(&mut t.crop_overlay, o, o.label());
                }
            })
            .response
            .on_hover_text("O cycles the overlays");
    });
    ui.checkbox(&mut s.crop.expand, "Allow crop outside image (expand)").on_hover_text("Areas outside are filled with content-aware fill on export");
    ui.add_space(8.0);
    ui.label(
        RichText::new("Drag corners or edges to resize, inside to move, outside to rotate. Enter or double-click applies, Esc cancels.")
            .weak()
            .small(),
    );
}

/// Turn the photo a quarter turn (1 = clockwise, 3 = counter-clockwise),
/// carrying the crop round with it.
pub fn rotate90(s: &mut DevelopSettings, q: u8) {
    let c = &mut s.crop;
    c.rotate90 = (c.rotate90 + q) % 4;
    let (l, t, r, b) = (c.left, c.top, c.right, c.bottom);
    (c.left, c.top, c.right, c.bottom) = if q == 1 { (1.0 - b, l, 1.0 - t, r) } else { (t, 1.0 - r, b, 1.0 - l) };
    if let Some(a) = c.aspect.as_mut() {
        a.swap(0, 1);
    }
}

/// Swap a locked ratio between landscape and portrait and make the new box
/// as large as fits, around the old centre.
pub fn swap_orientation(s: &mut DevelopSettings, engine: &darkslide_core::Engine) {
    let (fw, fh) = engine.full_size();
    let geo = engine.geometry(s, fw, fh, true);
    let (ow, oh) = (geo.ow as f32, geo.oh as f32);
    let c = &mut s.crop;
    let ratio = match c.aspect {
        Some(a) if a[0] > 0.0 => {
            c.aspect = Some([a[1], a[0]]);
            a[1] / a[0]
        }
        Some(_) => oh / ow,
        // free: swap the current box's shape
        None => ((c.bottom - c.top) * oh) / ((c.right - c.left) * ow),
    };
    if matches!(c.aspect, Some(a) if a[0] <= 0.0) {
        // "As shot" can't swap; lock the swapped ratio instead
        c.aspect = Some([oh, ow]);
    }
    let (cx, cy) = ((c.left + c.right) / 2.0, (c.top + c.bottom) / 2.0);
    let (nw, nh) = if ow / oh > ratio { (oh * ratio, oh) } else { (ow, ow / ratio) };
    c.left = cx - nw / ow / 2.0;
    c.right = cx + nw / ow / 2.0;
    c.top = cy - nh / oh / 2.0;
    c.bottom = cy + nh / oh / 2.0;
    // slide back inside the photo
    let dx = (-c.left).max(0.0) - (c.right - 1.0).max(0.0);
    let dy = (-c.top).max(0.0) - (c.bottom - 1.0).max(0.0);
    c.left += dx;
    c.right += dx;
    c.top += dy;
    c.bottom += dy;
    fit_crop_inside(s, engine);
}

pub fn reset_crop(s: &mut DevelopSettings) {
    s.crop.left = 0.0;
    s.crop.top = 0.0;
    s.crop.right = 1.0;
    s.crop.bottom = 1.0;
}

/// Enforce the aspect lock by shrinking the current crop around its centre.
pub fn apply_aspect(s: &mut DevelopSettings, engine: &darkslide_core::Engine) {
    let Some(a) = s.crop.aspect else { return };
    let (fw, fh) = engine.full_size();
    let geo = engine.geometry(s, fw, fh, true);
    let (ow, oh) = (geo.ow as f32, geo.oh as f32);
    let ratio = if a[0] <= 0.0 { ow / oh } else { a[0] / a[1] };
    let c = &mut s.crop;
    let cw = (c.right - c.left) * ow;
    let ch = (c.bottom - c.top) * oh;
    let (nw, nh) = if cw / ch > ratio { (ch * ratio, ch) } else { (cw, cw / ratio) };
    let cx = (c.left + c.right) / 2.0;
    let cy = (c.top + c.bottom) / 2.0;
    c.left = cx - nw / ow / 2.0;
    c.right = cx + nw / ow / 2.0;
    c.top = cy - nh / oh / 2.0;
    c.bottom = cy + nh / oh / 2.0;
    fit_crop_inside(s, engine);
}

/// Shrink the crop (keeping its centre and aspect) until it fits inside the
/// rotated image.
pub fn fit_crop_inside(s: &mut DevelopSettings, engine: &darkslide_core::Engine) {
    if s.crop.expand {
        return;
    }
    let (fw, fh) = engine.full_size();
    for _ in 0..40 {
        let geo = engine.geometry(s, fw, fh, false);
        let ok = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].iter().all(|&(x, y)| {
            let (u, v) = geo.crop_to_t(x, y);
            (-1e-4..=1.0001).contains(&u) && (-1e-4..=1.0001).contains(&v)
        });
        if ok {
            return;
        }
        let c = &mut s.crop;
        let cx = (c.left + c.right) / 2.0;
        let cy = (c.top + c.bottom) / 2.0;
        let k = 0.97;
        c.left = cx - (cx - c.left) * k;
        c.right = cx + (c.right - cx) * k;
        c.top = cy - (cy - c.top) * k;
        c.bottom = cy + (c.bottom - cy) * k;
    }
}

pub fn remove_panel(ui: &mut egui::Ui, doc: &mut Doc, t: &mut ToolUi, actions: &mut Vec<edit_panel::Action>) {
    let engine = doc.engine.clone();
    let s = &mut doc.settings;
    ui.heading("Remove");
    // a newly selected spot shows its own settings, which the controls then edit
    let sel = t.selected_spot.filter(|&i| i < s.retouch.len());
    if sel != t.spot_loaded {
        if let Some(sp) = sel.map(|i| &s.retouch[i]) {
            t.remove_mode = sp.mode;
            t.spot_size = sp.radius;
            t.spot_feather = sp.feather;
            t.spot_opacity = sp.opacity;
        }
        t.spot_loaded = sel;
    }
    let mode_before = t.remove_mode;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut t.remove_mode, RetouchMode::Remove, "Remove").on_hover_text("Content-aware fill");
        ui.selectable_value(&mut t.remove_mode, RetouchMode::Heal, "Heal").on_hover_text("Blend in texture from a source area");
        ui.selectable_value(&mut t.remove_mode, RetouchMode::Clone, "Clone").on_hover_text("Copy a source area exactly");
    });
    let resource = |sp: &mut RetouchSpot| {
        if sp.mode != RetouchMode::Remove
            && let Some(e) = &engine
        {
            sp.source_offset = darkslide_core::retouch::find_source(&e.base(darkslide_core::demosaic::Quality::Draft), sp);
        }
    };
    if t.remove_mode != mode_before
        && let Some(sp) = sel.map(|i| &mut s.retouch[i])
    {
        let had_source = sp.mode != RetouchMode::Remove;
        sp.mode = t.remove_mode;
        if !had_source {
            resource(sp);
        }
    }
    let mut size = t.spot_size * 1000.0;
    if slider(ui, &mut size, SliderSpec::new("Size", 1.0, 200.0, 12.0)) {
        t.spot_size = size / 1000.0;
        if let Some(sp) = sel.map(|i| &mut s.retouch[i]) {
            sp.radius = t.spot_size;
            // a grown spot can't take from where it now overlaps
            let d = (sp.source_offset[0].powi(2) + sp.source_offset[1].powi(2)).sqrt();
            if sp.mode != RetouchMode::Remove && d < sp.radius * 2.2 {
                resource(sp);
            }
        }
    }
    if slider(ui, &mut t.spot_feather, SliderSpec::new("Feather", 0.0, 100.0, 30.0)) {
        if let Some(sp) = sel.map(|i| &mut s.retouch[i]) {
            sp.feather = t.spot_feather;
        }
    }
    if slider(ui, &mut t.spot_opacity, SliderSpec::new("Opacity", 0.0, 100.0, 100.0)) {
        if let Some(sp) = sel.map(|i| &mut s.retouch[i]) {
            sp.opacity = t.spot_opacity;
        }
    }
    ui.label(
        RichText::new(if sel.is_some() { "Editing the selected spot · Esc deselects" } else { "Settings for the next spot" })
            .small()
            .weak(),
    );
    ui.add_space(4.0);
    ui.checkbox(&mut t.visualize_spots, "Visualize spots").on_hover_text("Show the photo as edges to find dust");
    // always there, so ticking the box doesn't push the panel down
    ui.add_enabled_ui(t.visualize_spots, |ui| slider(ui, &mut t.visualize_threshold, SliderSpec::new("Threshold", 0.0, 100.0, 50.0)));
    ui.checkbox(&mut t.show_spot_overlay, "Show overlay");
    ui.add_space(4.0);
    let n = s.retouch.len();
    ui.label(RichText::new(if n == 1 { "1 spot".to_string() } else { format!("{n} spots") }).weak());
    ui.horizontal(|ui| {
        if ui.add_enabled(sel.is_some(), egui::Button::new(format!("{}  Delete spot", ic::TRASH))).on_hover_text("Delete").clicked() {
            if let Some(i) = sel {
                s.retouch.remove(i);
                t.selected_spot = None;
            }
        }
        if ui.add_enabled(n > 0, egui::Button::new("Clear all")).clicked() {
            s.retouch.clear();
            t.selected_spot = None;
        }
    });
    ui.add_space(4.0);
    ui.label(RichText::new("Distraction removal").strong());
    ui.horizontal_wrapped(|ui| {
        if ui.button("Dust spots").on_hover_text("Find sensor dust and heal it").clicked() {
            actions.push(edit_panel::Action::DetectDust);
        }
        if ui.button("People").on_hover_text("Detect and remove people").clicked() {
            actions.push(edit_panel::Action::RemovePeople);
        }
        if ui.button("Wires & cables").on_hover_text("Detect power lines and cables against the sky").clicked() {
            actions.push(edit_panel::Action::RemoveWires);
        }
    });
    ui.add_space(4.0);
    ui.label(RichText::new("Click or paint over a blemish. Drag a spot or its source to move it.").weak().small());
}

pub fn redeye_panel(ui: &mut egui::Ui, doc: &mut Doc, t: &mut ToolUi) {
    let s = &mut doc.settings;
    ui.heading("Red Eye");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut t.pet_eye, false, "Red eye");
        ui.selectable_value(&mut t.pet_eye, true, "Pet eye");
    });
    ui.label(RichText::new("Drag a box around each eye. Drag inside a box to move it, its edge to resize; Delete removes it.").weak());
    ui.add_space(4.0);
    // the sliders always show, so drawing the first box doesn't move the panel
    let sel = t.selected_red_eye.filter(|&i| i < s.red_eye.len());
    let mut none = RedEyeSpot { center: [0.0; 2], radius: [0.0; 2], pupil_size: 50.0, darken: 50.0, pet: t.pet_eye, catchlight: t.pet_eye };
    let target = match sel {
        Some(i) => &mut s.red_eye[i],
        None => &mut none,
    };
    ui.add_enabled_ui(sel.is_some(), |ui| {
        slider(ui, &mut target.pupil_size, SliderSpec::new("Pupil size", 0.0, 100.0, 50.0));
        slider(ui, &mut target.darken, SliderSpec::new("Darken", 0.0, 100.0, 50.0));
        ui.add_enabled(target.pet, egui::Checkbox::new(&mut target.catchlight, "Add catchlight"));
    });
    let n = s.red_eye.len();
    ui.label(RichText::new(match (n, sel) {
        (0, _) => "No corrections yet".to_string(),
        (_, None) => format!("{n} correction{} — click one to adjust it", if n == 1 { "" } else { "s" }),
        (_, Some(i)) => format!("Correction {} of {n} selected", i + 1),
    }).weak());
    ui.horizontal(|ui| {
        if ui.add_enabled(sel.is_some(), egui::Button::new(format!("{}  Delete", ic::TRASH))).on_hover_text("Delete").clicked() {
            if let Some(i) = sel {
                s.red_eye.remove(i);
                t.selected_red_eye = None;
            }
        }
        if ui.add_enabled(n > 0, egui::Button::new("Clear all")).clicked() {
            s.red_eye.clear();
            t.selected_red_eye = None;
        }
    });
}

pub fn snapshots_panel(ui: &mut egui::Ui, doc: &mut Doc, t: &mut ToolUi) {
    ui.heading("Snapshots");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut t.snapshot_name).hint_text("Snapshot name").desired_width(150.0));
        if ui.button("+ Create").clicked() {
            let name = if t.snapshot_name.trim().is_empty() { format!("Snapshot {}", doc.snapshots.len() + 1) } else { t.snapshot_name.trim().to_string() };
            doc.snapshots.push((name, doc.settings.clone()));
            t.snapshot_name.clear();
            doc.dirty = true;
        }
    });
    ui.separator();
    let mut remove = None;
    let mut apply = None;
    let mut update = None;
    for (i, (name, _)) in doc.snapshots.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.button(name).on_hover_text("Apply snapshot").clicked() {
                apply = Some(i);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(ic::TRASH).clicked() {
                    remove = Some(i);
                }
                if ui.small_button(ic::ARROWS_CLOCKWISE).on_hover_text("Update with current settings").clicked() {
                    update = Some(i);
                }
            });
        });
    }
    if let Some(i) = apply {
        doc.settings = doc.snapshots[i].1.clone();
    }
    if let Some(i) = update {
        doc.snapshots[i].1 = doc.settings.clone();
        doc.dirty = true;
    }
    if let Some(i) = remove {
        doc.snapshots.remove(i);
        doc.dirty = true;
    }
    if doc.snapshots.is_empty() {
        ui.label(RichText::new("No snapshots yet.").weak());
    }
}

/// "Sky 2" for the second sky mask, whatever else exists.
fn mask_name(masks: &[MaskGroup], base: &str) -> String {
    let n = masks.iter().filter(|m| m.name.strip_prefix(base).is_some_and(|r| r.trim().parse::<u32>().is_ok())).count();
    format!("{base} {}", n + 1)
}

fn ai_name(k: AiMaskKind) -> &'static str {
    match k {
        AiMaskKind::Object => "Object",
        AiMaskKind::Person => "Person",
        k => k.label(),
    }
}

const LANDSCAPE: [AiMaskKind; 7] = [
    AiMaskKind::Water,
    AiMaskKind::Vegetation,
    AiMaskKind::Mountains,
    AiMaskKind::Architecture,
    AiMaskKind::NaturalGround,
    AiMaskKind::ArtificialGround,
    AiMaskKind::Snow,
];

fn shape_name(shape: &MaskShape) -> &'static str {
    match shape {
        MaskShape::Brush { .. } => "Brush",
        MaskShape::Linear { .. } => "Linear gradient",
        MaskShape::Radial { .. } => "Radial gradient",
        MaskShape::ColorRange { .. } => "Color range",
        MaskShape::LuminanceRange { .. } => "Luminance range",
        MaskShape::DepthRange { .. } => "Depth range",
        MaskShape::Ai { kind, .. } => ai_name(*kind),
    }
}

/// What the create menu (and Add / Subtract / Intersect) can make.
enum NewShape {
    Ai(AiMaskKind),
    Objects,
    Brush,
    Linear,
    Radial,
    Shape(MaskShape),
}

/// The create entries as menu buttons; returns the one picked.
fn shape_menu(ui: &mut egui::Ui, pick: &mut Pick) -> Option<NewShape> {
    let mut out = None;
    for (icon, label, k) in [(ic::USER, "Subject", AiMaskKind::Subject), (ic::CLOUD, "Sky", AiMaskKind::Sky), (ic::IMAGE, "Background", AiMaskKind::Background)] {
        if ui.button(format!("{icon}  {label}")).clicked() {
            out = Some(NewShape::Ai(k));
        }
    }
    if ui.button(format!("{}  Objects", ic::BOUNDING_BOX)).clicked() {
        out = Some(NewShape::Objects);
    }
    ui.menu_button(format!("{}  Landscape", ic::TREE), |ui| {
        for k in LANDSCAPE {
            if ui.button(k.label()).clicked() {
                out = Some(NewShape::Ai(k));
            }
        }
    });
    ui.separator();
    if ui.button(format!("{}  Brush", ic::PAINT_BRUSH)).clicked() {
        out = Some(NewShape::Brush);
    }
    if ui.button(format!("{}  Linear gradient", ic::GRADIENT)).clicked() {
        out = Some(NewShape::Linear);
    }
    if ui.button(format!("{}  Radial gradient", ic::CIRCLE)).clicked() {
        out = Some(NewShape::Radial);
    }
    ui.separator();
    if ui.button(format!("{}  Color range", ic::PALETTE)).clicked() {
        out = Some(NewShape::Shape(MaskShape::ColorRange { samples: vec![], refine: 50.0 }));
        *pick = Pick::Sampler;
    }
    if ui.button(format!("{}  Luminance range", ic::SUN)).clicked() {
        out = Some(NewShape::Shape(MaskShape::LuminanceRange { range: [0.6, 1.0], smoothness: 30.0 }));
    }
    if ui.button(format!("{}  Depth range", ic::MOUNTAINS)).clicked() {
        out = Some(NewShape::Shape(MaskShape::DepthRange { range: [0.0, 0.3], smoothness: 30.0 }));
    }
    if out.is_some() {
        ui.close();
    }
    out
}

/// Low/high range sliders that can't cross, over a strip showing the range.
fn range_ui(ui: &mut egui::Ui, range: &mut [f32; 2], luminance: bool) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.0), egui::Sense::hover());
    let p = ui.painter();
    let n = 32;
    for i in 0..n {
        let (u0, u1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
        let g = (u0 * 255.0) as u8;
        let col = if luminance { Color32::from_gray(g) } else { Color32::from_rgb(255 - g, 160, g) };
        let x = |u: f32| r.left() + u * r.width();
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(x(u0), r.top()), egui::pos2(x(u1), r.bottom())), 0.0, col);
    }
    let (x0, x1) = (r.left() + range[0] * r.width(), r.left() + range[1] * r.width());
    p.rect_filled(egui::Rect::from_min_max(r.min, egui::pos2(x0, r.bottom())), 0.0, Color32::from_black_alpha(170));
    p.rect_filled(egui::Rect::from_min_max(egui::pos2(x1, r.top()), r.max), 0.0, Color32::from_black_alpha(170));
    p.rect_stroke(egui::Rect::from_min_max(egui::pos2(x0, r.top()), egui::pos2(x1, r.bottom())), 0.0, egui::Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
    let mut lo = range[0] * 100.0;
    let mut hi = range[1] * 100.0;
    if slider(ui, &mut lo, SliderSpec::new("Range low", 0.0, 100.0, if luminance { 60.0 } else { 0.0 })) {
        range[0] = (lo / 100.0).min(range[1]);
    }
    if slider(ui, &mut hi, SliderSpec::new("Range high", 0.0, 100.0, if luminance { 100.0 } else { 30.0 })) {
        range[1] = (hi / 100.0).max(range[0]);
    }
}

pub fn masking_panel(ui: &mut egui::Ui, doc: &mut Doc, t: &mut ToolUi, eu: &mut EditUi, pick: &mut Pick) {
    let s = &mut doc.settings;
    let _ = eu;
    ui.heading("Masking");
    let mut new: Option<NewShape> = None;
    // the create grid folds away once there are masks, so a new mask's sliders show
    let has_masks = !s.masks.is_empty();
    let open = !has_masks || t.show_create;
    if has_masks {
        let label = format!("{}  Create new mask", if open { ic::CARET_DOWN } else { ic::CARET_RIGHT });
        if ui.add(egui::Button::new(RichText::new(label).strong()).frame(false)).clicked() {
            t.show_create = !t.show_create;
        }
    } else {
        ui.label(RichText::new("Create new mask").size(13.5).strong());
    }
    if open {
        let caption = |ui: &mut egui::Ui, text: &str| {
            ui.add_space(4.0);
            ui.label(RichText::new(text).size(11.0).color(crate::theme::pal().text_muted));
        };
        let armed = t.mask_tool;
        let tiles = |ui: &mut egui::Ui, id: &str, cols: usize, items: &[(&str, &str, Option<MaskTool>, &str)]| -> Option<usize> {
            let tw = ((ui.available_width() - 6.0 * (cols - 1) as f32) / cols as f32).floor();
            let mut hit = None;
            egui::Grid::new(id).num_columns(cols).spacing([6.0, 6.0]).show(ui, |ui| {
                for (i, (icon, label, tool, tip)) in items.iter().enumerate() {
                    let on = tool.is_some_and(|m| m == armed && m != MaskTool::None);
                    let b = egui::Button::selectable(on, format!("{icon}  {label}")).min_size(egui::vec2(tw, 28.0));
                    let r = ui.add(b);
                    let r = if tip.is_empty() { r } else { r.on_hover_text(*tip) };
                    if r.clicked() {
                        hit = Some(i);
                    }
                    if (i + 1) % cols == 0 {
                        ui.end_row();
                    }
                }
            });
            hit
        };
        caption(ui, "SELECT");
        match tiles(
            ui,
            "mask_ai",
            2,
            &[
                (ic::USER, "Subject", None, ""),
                (ic::CLOUD, "Sky", None, ""),
                (ic::IMAGE, "Background", None, ""),
                (ic::BOUNDING_BOX, "Objects", Some(MaskTool::ObjectBox), "Drag a box around the object"),
            ],
        ) {
            Some(0) => new = Some(NewShape::Ai(AiMaskKind::Subject)),
            Some(1) => new = Some(NewShape::Ai(AiMaskKind::Sky)),
            Some(2) => new = Some(NewShape::Ai(AiMaskKind::Background)),
            Some(_) => new = Some(NewShape::Objects),
            None => {}
        }
        ui.add_space(6.0);
        let lw = ui.available_width();
        ui.allocate_ui(egui::vec2(lw, 28.0), |ui| {
            ui.menu_button(format!("{}  Landscape  {}", ic::TREE, ic::CARET_RIGHT), |ui| {
                for k in LANDSCAPE {
                    if ui.button(k.label()).clicked() {
                        new = Some(NewShape::Ai(k));
                        ui.close();
                    }
                }
            });
        });
        caption(ui, "SHAPES");
        match tiles(
            ui,
            "mask_shapes",
            3,
            &[(ic::PAINT_BRUSH, "Brush", Some(MaskTool::Brush), ""), (ic::GRADIENT, "Linear", Some(MaskTool::Linear), ""), (ic::CIRCLE, "Radial", Some(MaskTool::Radial), "")],
        ) {
            Some(0) => new = Some(NewShape::Brush),
            Some(1) => new = Some(NewShape::Linear),
            Some(2) => new = Some(NewShape::Radial),
            _ => {}
        }
        caption(ui, "RANGE");
        match tiles(ui, "mask_ranges", 3, &[(ic::PALETTE, "Color", None, "Color range"), (ic::SUN, "Luminance", None, "Luminance range"), (ic::MOUNTAINS, "Depth", None, "Depth range")]) {
            Some(0) => {
                new = Some(NewShape::Shape(MaskShape::ColorRange { samples: vec![], refine: 50.0 }));
                *pick = Pick::Sampler;
            }
            Some(1) => new = Some(NewShape::Shape(MaskShape::LuminanceRange { range: [0.6, 1.0], smoothness: 30.0 })),
            Some(2) => new = Some(NewShape::Shape(MaskShape::DepthRange { range: [0.0, 0.3], smoothness: 30.0 })),
            _ => {}
        }
    }
    // what the armed tool expects next
    let hint = match t.mask_tool {
        MaskTool::Brush if t.selected_mask.is_none() => Some("Paint on the photo to start a brush mask. Alt erases, [ and ] size the brush."),
        MaskTool::Linear => Some("Drag on the photo to draw a linear gradient."),
        MaskTool::Radial => Some("Drag on the photo to draw a radial gradient. Shift makes a circle."),
        MaskTool::ObjectBox => Some("Drag a box around the object to select it."),
        _ if *pick == Pick::Sampler => Some("Click the photo to sample a color. Esc finishes."),
        _ => None,
    };
    if let Some(h) = hint {
        ui.add_space(4.0);
        ui.label(RichText::new(h).small().color(crate::widgets::ACCENT));
    }
    match new {
        Some(NewShape::Ai(k)) => {
            let shape = MaskShape::Ai { kind: k, feather: 0.0, edge: 0.0, hint: None, raster: None };
            let name = mask_name(&s.masks, ai_name(k));
            s.masks.push(MaskGroup { name, components: vec![MaskComponent { shape, ..Default::default() }], ..Default::default() });
            let mi = s.masks.len() - 1;
            t.selected_mask = Some(mi);
            t.selected_comp = Some(0);
            t.mask_tool = MaskTool::None;
            t.request_ai = Some((mi, k));
            t.show_create = false;
        }
        Some(NewShape::Objects) => {
            t.mask_tool = MaskTool::ObjectBox;
        }
        // the first stroke / drag makes the mask, so nothing empty is left behind
        Some(NewShape::Brush) => {
            t.mask_tool = MaskTool::Brush;
            t.selected_mask = None;
            t.selected_comp = None;
        }
        Some(NewShape::Linear) | Some(NewShape::Radial) => {
            t.mask_tool = if matches!(new, Some(NewShape::Linear)) { MaskTool::Linear } else { MaskTool::Radial };
            t.selected_mask = None;
            t.selected_comp = None;
            t.place_comp = None;
        }
        Some(NewShape::Shape(shape)) => {
            let name = mask_name(&s.masks, "Mask");
            s.masks.push(MaskGroup { name, components: vec![MaskComponent { shape, ..Default::default() }], ..Default::default() });
            t.selected_mask = Some(s.masks.len() - 1);
            t.selected_comp = Some(0);
            t.mask_tool = MaskTool::None;
            t.show_create = false;
        }
        None => {}
    }
    ui.separator();

    // mask list
    enum RowAct {
        Delete,
        Duplicate(bool),
        Move(isize),
    }
    let mut act: Option<(usize, RowAct)> = None;
    let n_masks = s.masks.len();
    for (i, m) in s.masks.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let sel = t.selected_mask == Some(i);
            let eye = if m.enabled { ic::EYE } else { ic::EYE_SLASH };
            if ui.add(egui::Button::new(eye).frame(false)).on_hover_text(if m.enabled { "Hide this mask's effect" } else { "Show this mask's effect" }).clicked() {
                m.enabled = !m.enabled;
            }
            if t.renaming == Some(i) {
                let r = ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(150.0));
                r.request_focus();
                if r.lost_focus() {
                    t.renaming = None;
                }
            } else {
                let r = ui.selectable_label(sel, &m.name).on_hover_text("Double-click to rename, right-click for more");
                if r.clicked() {
                    t.select_request = Some(i);
                }
                if r.double_clicked() {
                    t.renaming = Some(i);
                }
                r.context_menu(|ui| {
                    if ui.button(format!("{}  Rename", ic::PENCIL_SIMPLE)).clicked() {
                        t.renaming = Some(i);
                        ui.close();
                    }
                    if ui.button(format!("{}  Duplicate", ic::COPY)).clicked() {
                        act = Some((i, RowAct::Duplicate(false)));
                        ui.close();
                    }
                    if ui.button(format!("{}  Duplicate and invert", ic::CIRCLE_HALF)).clicked() {
                        act = Some((i, RowAct::Duplicate(true)));
                        ui.close();
                    }
                    if ui.add_enabled(i > 0, egui::Button::new(format!("{}  Move up", ic::ARROW_UP))).clicked() {
                        act = Some((i, RowAct::Move(-1)));
                        ui.close();
                    }
                    if ui.add_enabled(i + 1 < n_masks, egui::Button::new(format!("{}  Move down", ic::ARROW_DOWN))).clicked() {
                        act = Some((i, RowAct::Move(1)));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(format!("{}  Delete", ic::TRASH)).clicked() {
                        act = Some((i, RowAct::Delete));
                        ui.close();
                    }
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(ic::TRASH).on_hover_text("Delete mask (Delete)").clicked() {
                    act = Some((i, RowAct::Delete));
                }
            });
        });
    }
    if let Some(i) = t.select_request.take().filter(|&i| i < s.masks.len()) {
        crate::viewer::select_mask(t, s, i);
    }
    match act {
        Some((i, RowAct::Delete)) => {
            s.masks.remove(i);
            t.selected_mask = None;
            t.selected_comp = None;
        }
        Some((i, RowAct::Duplicate(invert))) => {
            let mut m = s.masks[i].clone();
            m.name = format!("{} copy", m.name);
            if invert {
                m.invert = !m.invert;
            }
            s.masks.insert(i + 1, m);
            t.selected_mask = Some(i + 1);
            t.selected_comp = Some(0);
        }
        Some((i, RowAct::Move(d))) => {
            let j = (i as isize + d) as usize;
            s.masks.swap(i, j);
            if t.selected_mask == Some(i) {
                t.selected_mask = Some(j);
            }
        }
        None => {}
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut t.show_mask_overlay, "Show overlay (Y)");
        ui.color_edit_button_srgba(&mut t.mask_overlay_color).on_hover_text("Overlay color");
    });

    let Some(mi) = t.selected_mask.filter(|&i| i < s.masks.len()) else {
        if s.masks.is_empty() {
            ui.label(RichText::new("Pick a mask type above.").weak());
        } else {
            ui.label(RichText::new("Select a mask in the list or by its pin on the photo.").weak());
        }
        return;
    };
    ui.separator();
    let m = &mut s.masks[mi];
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(140.0)).on_hover_text("Mask name");
        ui.checkbox(&mut m.invert, "Invert");
    });
    // components
    ui.label(RichText::new("Components").strong());
    let mut cdel = None;
    let single = m.components.len() == 1;
    for (ci, c) in m.components.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let op = match (ci, c.op) {
                (0, _) => "",
                (_, MaskOp::Add) => "+ ",
                (_, MaskOp::Subtract) => "− ",
                (_, MaskOp::Intersect) => "∩ ",
            };
            if ui.selectable_label(t.selected_comp == Some(ci), format!("{op}{}", shape_name(&c.shape))).clicked() {
                t.selected_comp = Some(ci);
                t.place_comp = None;
                t.mask_tool = if matches!(c.shape, MaskShape::Brush { .. }) { MaskTool::Brush } else { MaskTool::None };
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !single && ui.small_button(ic::TRASH).on_hover_text("Remove this component").clicked() {
                    cdel = Some(ci);
                }
                if ui.selectable_label(c.invert, ic::CIRCLE_HALF).on_hover_text("Invert this component").clicked() {
                    c.invert = !c.invert;
                }
            });
        });
    }
    if let Some(ci) = cdel {
        m.components.remove(ci);
        t.selected_comp = Some(0);
    }
    ui.horizontal(|ui| {
        for (op, lab) in [(MaskOp::Add, "Add"), (MaskOp::Subtract, "Subtract"), (MaskOp::Intersect, "Intersect")] {
            ui.menu_button(lab, |ui| {
                if let Some(n) = shape_menu(ui, pick) {
                    let (shape, tool) = match n {
                        NewShape::Ai(k) => {
                            t.request_ai = Some((mi, k));
                            (MaskShape::Ai { kind: k, feather: 0.0, edge: 0.0, hint: None, raster: None }, MaskTool::None)
                        }
                        NewShape::Objects => (MaskShape::Ai { kind: AiMaskKind::Object, feather: 0.0, edge: 0.0, hint: Some([0.3, 0.3, 0.7, 0.7]), raster: None }, MaskTool::None),
                        NewShape::Brush => (MaskShape::Brush { strokes: vec![] }, MaskTool::Brush),
                        NewShape::Linear => (MaskShape::Linear { start: [0.5, 0.3], end: [0.5, 0.6], two_sided: false }, MaskTool::Linear),
                        NewShape::Radial => (MaskShape::Radial { center: [0.5, 0.5], radius: [0.2, 0.2], angle: 0.0, feather: 50.0 }, MaskTool::Radial),
                        NewShape::Shape(sh) => (sh, MaskTool::None),
                    };
                    m.components.push(MaskComponent { op, shape, ..Default::default() });
                    let ci = m.components.len() - 1;
                    t.selected_comp = Some(ci);
                    t.mask_tool = tool;
                    // a gradient is drawn by the next drag on the photo
                    t.place_comp = matches!(tool, MaskTool::Linear | MaskTool::Radial).then_some((mi, ci));
                }
            });
        }
    });
    // selected component settings
    if let Some(ci) = t.selected_comp.filter(|&c| c < m.components.len()) {
        let c = &mut m.components[ci];
        slider(ui, &mut c.opacity, SliderSpec::new("Component opacity", 0.0, 100.0, 100.0));
        match &mut c.shape {
            MaskShape::Brush { .. } => {
                let b = &mut t.brush;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut b.erase, false, "Paint");
                    ui.selectable_value(&mut b.erase, true, "Erase").on_hover_text("Or hold Alt while painting");
                });
                let mut sz = b.size * 1000.0;
                if slider(ui, &mut sz, SliderSpec::new("Size", 1.0, 300.0, 30.0)) {
                    b.size = sz / 1000.0;
                }
                slider(ui, &mut b.feather, SliderSpec::new("Feather", 0.0, 100.0, 50.0));
                slider(ui, &mut b.flow, SliderSpec::new("Flow", 1.0, 100.0, 50.0));
                slider(ui, &mut b.density, SliderSpec::new("Density", 0.0, 100.0, 100.0));
                ui.checkbox(&mut b.auto_mask, "Auto mask");
                t.mask_tool = MaskTool::Brush;
            }
            MaskShape::Linear { two_sided, .. } => {
                ui.checkbox(two_sided, "Two-sided");
            }
            MaskShape::Radial { feather, angle, .. } => {
                slider(ui, feather, SliderSpec::new("Feather", 0.0, 100.0, 50.0));
                slider(ui, angle, SliderSpec::new("Angle", -180.0, 180.0, 0.0).dec(1));
            }
            MaskShape::ColorRange { samples, refine } => {
                ui.horizontal(|ui| {
                    let active = *pick == Pick::Sampler;
                    if widgets::eyedropper_button(ui, active, "Sample color in image") {
                        *pick = if active { Pick::None } else { Pick::Sampler };
                    }
                    ui.label("Add color");
                    let n = samples.len();
                    ui.label(RichText::new(if n == 1 { "1 sample".to_string() } else { format!("{n} samples") }).weak());
                    if ui.add_enabled(n > 0, egui::Button::new("Clear").small()).clicked() {
                        samples.clear();
                    }
                });
                slider(ui, refine, SliderSpec::new("Refine", 0.0, 100.0, 50.0));
            }
            MaskShape::LuminanceRange { range, smoothness } => {
                range_ui(ui, range, true);
                slider(ui, smoothness, SliderSpec::new("Smoothness", 0.0, 100.0, 30.0));
            }
            MaskShape::DepthRange { range, smoothness } => {
                range_ui(ui, range, false);
                slider(ui, smoothness, SliderSpec::new("Smoothness", 0.0, 100.0, 30.0));
            }
            MaskShape::Ai { kind, feather, edge, raster, .. } => {
                ui.label(RichText::new(if raster.is_some() { format!("{} detected", kind.label()) } else { "Detecting…".to_string() }).weak());
                slider(ui, feather, SliderSpec::new("Feather", 0.0, 100.0, 0.0));
                slider(ui, edge, SliderSpec::new("Edge", -100.0, 100.0, 0.0));
                if ui.button(format!("{}  Detect again", ic::ARROWS_CLOCKWISE)).clicked() {
                    *raster = None;
                    t.request_ai = Some((mi, *kind));
                }
            }
        }
    }
    ui.separator();
    // local adjustments, grouped like the Edit panel
    let a = &mut m.adjust;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Adjustments").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Reset").on_hover_text("Reset this mask's sliders").clicked() {
                *a = LocalAdjust::default();
            }
        });
    });
    slider(ui, &mut m.amount, SliderSpec::new("Amount", 0.0, 200.0, 100.0));
    let group = |ui: &mut egui::Ui, title: &str, open: bool, body: &mut dyn FnMut(&mut egui::Ui)| {
        egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(("madj", title, mi)).default_open(open).show(ui, |ui| body(ui));
    };
    group(ui, "Light", true, &mut |ui| {
        slider(ui, &mut a.exposure, SliderSpec::new("Exposure", -4.0, 4.0, 0.0).dec(2));
        slider(ui, &mut a.contrast, SliderSpec::new("Contrast", -100.0, 100.0, 0.0));
        slider(ui, &mut a.highlights, SliderSpec::new("Highlights", -100.0, 100.0, 0.0));
        slider(ui, &mut a.shadows, SliderSpec::new("Shadows", -100.0, 100.0, 0.0));
        slider(ui, &mut a.whites, SliderSpec::new("Whites", -100.0, 100.0, 0.0));
        slider(ui, &mut a.blacks, SliderSpec::new("Blacks", -100.0, 100.0, 0.0));
    });
    group(ui, "Color", true, &mut |ui| {
        let gt = crate::widgets::grad_temp();
        let gti = crate::widgets::grad_tint();
        let gh = crate::widgets::grad_hue_full();
        slider(ui, &mut a.temperature, SliderSpec::new("Temperature", -100.0, 100.0, 0.0).grad(&gt));
        slider(ui, &mut a.tint, SliderSpec::new("Tint", -100.0, 100.0, 0.0).grad(&gti));
        slider(ui, &mut a.hue, SliderSpec::new("Hue", -100.0, 100.0, 0.0).grad(&gh));
        slider(ui, &mut a.saturation, SliderSpec::new("Saturation", -100.0, 100.0, 0.0));
        ui.horizontal(|ui| {
            ui.label("Color");
            let mut c = egui::Color32::from(crate::widgets::hue_color(a.color[0], a.color[1] / 100.0, 1.0));
            ui.color_edit_button_srgba(&mut c);
            let hsv = darkslide_core::color::rgb_to_hsv([c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0]);
            let nh = hsv[0].round();
            let ns = (hsv[1] * 100.0).round();
            if (nh - a.color[0]).abs() > 0.5 && ns > 0.0 || (ns - a.color[1]).abs() > 0.5 {
                a.color = [nh, ns];
            }
            if ui.small_button(ic::X).on_hover_text("No color").clicked() {
                a.color = [0.0, 0.0];
            }
        });
    });
    group(ui, "Effects", false, &mut |ui| {
        slider(ui, &mut a.texture, SliderSpec::new("Texture", -100.0, 100.0, 0.0));
        slider(ui, &mut a.clarity, SliderSpec::new("Clarity", -100.0, 100.0, 0.0));
        slider(ui, &mut a.dehaze, SliderSpec::new("Dehaze", -100.0, 100.0, 0.0));
    });
    group(ui, "Detail", false, &mut |ui| {
        slider(ui, &mut a.sharpness, SliderSpec::new("Sharpness", -100.0, 100.0, 0.0));
        slider(ui, &mut a.noise, SliderSpec::new("Noise", -100.0, 100.0, 0.0));
        slider(ui, &mut a.moire, SliderSpec::new("Moiré", -100.0, 100.0, 0.0));
        slider(ui, &mut a.defringe, SliderSpec::new("Defringe", -100.0, 100.0, 0.0));
    });
    egui::CollapsingHeader::new(RichText::new("Color grading").strong()).id_salt(("mgrade", mi)).show(ui, |ui| {
        edit_panel::grading_ui(ui, &mut a.grading, &mut t.mask_grading_view, "mask");
    });
    egui::CollapsingHeader::new(RichText::new("Curve").strong()).id_salt(("mcurve", mi)).show(ui, |ui| {
        crate::widgets::curve_editor(
            ui,
            crate::widgets::CurveEdit { points: &mut a.curve.rgb, color: Color32::from_gray(230), hist: None, id: ui.id().with(("mc", mi)), read_only: false },
            &[],
        );
    });
    egui::CollapsingHeader::new(RichText::new("Point color").strong()).id_salt(("mpc", mi)).show(ui, |ui| {
        edit_panel::point_color_ui(ui, &mut a.point_colors, &mut t.mask_point_sel, pick);
    });
}

