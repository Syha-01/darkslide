//! Right-hand panels for the non-Edit tools: Crop, Remove, Masking, Red Eye,
//! Presets and Snapshots.

use darkslide_core::params::*;
use darkslide_core::xmp::{self, Preset};
use eframe::egui::{self, Color32, RichText};

use crate::doc::Doc;
use crate::edit_panel::{self, EditUi, Pick};
use crate::widgets::{SliderSpec, slider};

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
        (Tool::Edit, "🎚", "Edit (E)"),
        (Tool::Crop, "⛶", "Crop & rotate (C)"),
        (Tool::Remove, "✚", "Remove (B)"),
        (Tool::Masking, "◐", "Masking (M)"),
        (Tool::RedEye, "👁", "Red eye"),
        (Tool::Presets, "★", "Presets (P)"),
        (Tool::Snapshots, "📷", "Snapshots (S)"),
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
    pub remove_mode: RetouchMode,
    pub spot_size: f32,
    pub spot_feather: f32,
    pub spot_opacity: f32,
    pub selected_spot: Option<usize>,
    pub visualize_spots: bool,
    pub visualize_threshold: f32,
    pub show_spot_overlay: bool,
    pub selected_mask: Option<usize>,
    pub selected_comp: Option<usize>,
    pub mask_tool: MaskTool,
    pub brush: BrushParams,
    pub show_mask_overlay: bool,
    pub mask_overlay_color: Color32,
    pub pet_eye: bool,
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
}

impl Default for ToolUi {
    fn default() -> Self {
        Self {
            tool: Tool::Edit,
            crop_overlay: CropOverlay::Thirds,
            straighten: false,
            remove_mode: RetouchMode::Heal,
            spot_size: 0.012,
            spot_feather: 30.0,
            spot_opacity: 100.0,
            selected_spot: None,
            visualize_spots: false,
            visualize_threshold: 50.0,
            show_spot_overlay: true,
            selected_mask: None,
            selected_comp: None,
            mask_tool: MaskTool::None,
            brush: BrushParams { size: 0.03, feather: 50.0, flow: 50.0, density: 100.0, auto_mask: false, erase: false },
            show_mask_overlay: true,
            mask_overlay_color: Color32::from_rgba_unmultiplied(255, 40, 40, 120),
            pet_eye: false,
            presets: vec![],
            presets_loaded: false,
            preset_amount: 100.0,
            new_preset_name: String::new(),
            new_preset_group: "User Presets".into(),
            snapshot_name: String::new(),
            mask_grading_view: 0,
            mask_point_sel: 0,
            request_ai: None,
            hover_preview: None,
            status: None,
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
    ui.heading("Crop & Rotate");
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Aspect");
        let cur = ASPECTS.iter().find(|(_, a)| *a == s.crop.aspect).map(|(n, _)| *n).unwrap_or("Custom");
        egui::ComboBox::from_id_salt("aspect").selected_text(cur).show_ui(ui, |ui| {
            for (n, a) in ASPECTS {
                if ui.selectable_label(s.crop.aspect == a, n).clicked() {
                    s.crop.aspect = a;
                    apply_aspect(s, &engine);
                }
            }
        });
        if s.crop.aspect.is_some() && ui.button("⟲").on_hover_text("Swap orientation (X)").clicked() {
            if let Some(a) = s.crop.aspect.as_mut() {
                a.swap(0, 1);
            }
            apply_aspect(s, &engine);
        }
    });
    if let Some(mut a) = s.crop.aspect {
        if a[0] > 0.0 {
            let changed = ui
                .horizontal(|ui| {
                    ui.label("Custom");
                    let c1 = ui.add(egui::DragValue::new(&mut a[0]).range(0.1..=100.0).speed(0.05)).changed();
                    ui.label(":");
                    let c2 = ui.add(egui::DragValue::new(&mut a[1]).range(0.1..=100.0).speed(0.05)).changed();
                    c1 || c2
                })
                .inner;
            if changed {
                s.crop.aspect = Some(a);
                apply_aspect(s, &engine);
            }
        }
    }
    ui.add_space(6.0);
    let mut angle = s.crop.angle;
    if slider(ui, &mut angle, SliderSpec::new("Angle", -45.0, 45.0, 0.0).dec(2)) {
        s.crop.angle = angle;
        fit_crop_inside(s, &engine);
    }
    ui.horizontal(|ui| {
        if ui.selectable_label(t.straighten, "📐 Straighten tool").on_hover_text("Draw a line along the horizon").clicked() {
            t.straighten = !t.straighten;
        }
        if ui.button("Auto").on_hover_text("Auto straighten").clicked() {
            if let Some(a) = darkslide_core::upright::auto_straighten_angle(&engine, s) {
                s.crop.angle = a;
                fit_crop_inside(s, &engine);
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("⟲ 90°").on_hover_text("Rotate counter-clockwise").clicked() {
            s.crop.rotate90 = (s.crop.rotate90 + 3) % 4;
            reset_crop(s);
        }
        if ui.button("⟳ 90°").on_hover_text("Rotate clockwise").clicked() {
            s.crop.rotate90 = (s.crop.rotate90 + 1) % 4;
            reset_crop(s);
        }
        if ui.button("⇔").on_hover_text("Flip horizontal").clicked() {
            s.crop.flip_h = !s.crop.flip_h;
        }
        if ui.button("⇕").on_hover_text("Flip vertical").clicked() {
            s.crop.flip_v = !s.crop.flip_v;
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Overlay");
        egui::ComboBox::from_id_salt("overlay").selected_text(format!("{:?}", t.crop_overlay)).show_ui(ui, |ui| {
            for o in [CropOverlay::Thirds, CropOverlay::Grid, CropOverlay::Golden, CropOverlay::Diagonal, CropOverlay::Triangle, CropOverlay::None] {
                ui.selectable_value(&mut t.crop_overlay, o, format!("{o:?}"));
            }
        });
    });
    ui.checkbox(&mut s.crop.expand, "Allow crop outside image (expand)").on_hover_text("Areas outside are filled with content-aware fill on export");
    ui.add_space(8.0);
    if ui.button("Reset crop").clicked() {
        reset_crop(s);
        s.crop.angle = 0.0;
    }
    ui.label(RichText::new("Drag corners/edges to resize, inside to move, outside to rotate. Enter or C to finish.").weak().small());
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
    let s = &mut doc.settings;
    ui.heading("Remove");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut t.remove_mode, RetouchMode::Remove, "Remove").on_hover_text("Content-aware fill");
        ui.selectable_value(&mut t.remove_mode, RetouchMode::Heal, "Heal");
        ui.selectable_value(&mut t.remove_mode, RetouchMode::Clone, "Clone");
    });
    let mut size = t.spot_size * 1000.0;
    if slider(ui, &mut size, SliderSpec::new("Size", 1.0, 200.0, 12.0)) {
        t.spot_size = size / 1000.0;
        if let Some(i) = t.selected_spot {
            if let Some(sp) = s.retouch.get_mut(i) {
                sp.radius = t.spot_size;
            }
        }
    }
    if slider(ui, &mut t.spot_feather, SliderSpec::new("Feather", 0.0, 100.0, 30.0)) {
        if let Some(sp) = t.selected_spot.and_then(|i| s.retouch.get_mut(i)) {
            sp.feather = t.spot_feather;
        }
    }
    if slider(ui, &mut t.spot_opacity, SliderSpec::new("Opacity", 0.0, 100.0, 100.0)) {
        if let Some(sp) = t.selected_spot.and_then(|i| s.retouch.get_mut(i)) {
            sp.opacity = t.spot_opacity;
        }
    }
    ui.add_space(4.0);
    ui.checkbox(&mut t.visualize_spots, "Visualize spots");
    if t.visualize_spots {
        slider(ui, &mut t.visualize_threshold, SliderSpec::new("Threshold", 0.0, 100.0, 50.0));
    }
    ui.checkbox(&mut t.show_spot_overlay, "Show overlay");
    ui.add_space(4.0);
    ui.label(format!("{} spot(s)", s.retouch.len()));
    ui.horizontal(|ui| {
        if let Some(i) = t.selected_spot {
            if ui.button("🗑 Delete spot").clicked() && i < s.retouch.len() {
                s.retouch.remove(i);
                t.selected_spot = None;
            }
        }
        if ui.button("Clear all").clicked() {
            s.retouch.clear();
            t.selected_spot = None;
        }
    });
    ui.label(RichText::new("Distraction removal").strong());
    ui.horizontal_wrapped(|ui| {
        if ui.button("Dust spots").clicked() {
            actions.push(edit_panel::Action::DetectDust);
        }
        if ui.button("People").on_hover_text("Detect and remove people").clicked() {
            actions.push(edit_panel::Action::RemovePeople);
        }
        if ui.button("Wires & cables").on_hover_text("Detect power lines and cables against the sky").clicked() {
            actions.push(edit_panel::Action::RemoveWires);
        }
    });
    ui.label(RichText::new("Click or paint over a blemish. Drag a heal/clone source to move it. Del removes the selected spot.").weak().small());
}

pub fn redeye_panel(ui: &mut egui::Ui, doc: &mut Doc, t: &mut ToolUi) {
    let s = &mut doc.settings;
    ui.heading("Red Eye");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut t.pet_eye, false, "Red eye");
        ui.selectable_value(&mut t.pet_eye, true, "Pet eye");
    });
    if let Some(last) = s.red_eye.last_mut() {
        slider(ui, &mut last.pupil_size, SliderSpec::new("Pupil size", 0.0, 100.0, 50.0));
        slider(ui, &mut last.darken, SliderSpec::new("Darken", 0.0, 100.0, 50.0));
        if last.pet {
            ui.checkbox(&mut last.catchlight, "Add catchlight");
        }
    }
    ui.label(format!("{} correction(s)", s.red_eye.len()));
    if ui.button("Clear all").clicked() {
        s.red_eye.clear();
    }
    ui.label(RichText::new("Drag a box around each eye.").weak().small());
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
                if ui.small_button("🗑").clicked() {
                    remove = Some(i);
                }
                if ui.small_button("⟳").on_hover_text("Update with current settings").clicked() {
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

pub fn presets_panel(ui: &mut egui::Ui, doc: &mut Doc, t: &mut ToolUi) {
    t.hover_preview = None;
    if !t.presets_loaded {
        t.presets = xmp::list_presets();
        t.presets_loaded = true;
    }
    ui.heading("Presets");
    slider(ui, &mut t.preset_amount, SliderSpec::new("Amount", 0.0, 200.0, 100.0));
    ui.horizontal(|ui| {
        if ui.button("Import…").clicked() {
            if let Some(files) = rfd::FileDialog::new().add_filter("XMP presets", &["xmp"]).pick_files() {
                let dir = darkslide_core::paths::presets_dir().join("Imported");
                let _ = std::fs::create_dir_all(&dir);
                for f in files {
                    if let Some(name) = f.file_name() {
                        let _ = std::fs::copy(&f, dir.join(name));
                    }
                }
                t.presets_loaded = false;
            }
        }
        if ui.button("⟳").on_hover_text("Rescan presets folder").clicked() {
            t.presets_loaded = false;
        }
        if ui.button("📂").on_hover_text("Open presets folder").clicked() {
            let d = darkslide_core::paths::presets_dir();
            let _ = std::fs::create_dir_all(&d);
            let _ = std::process::Command::new("xdg-open").arg(d).spawn();
        }
    });
    ui.separator();
    ui.collapsing("Create preset from current settings", |ui| {
        ui.add(egui::TextEdit::singleline(&mut t.new_preset_name).hint_text("Name"));
        ui.add(egui::TextEdit::singleline(&mut t.new_preset_group).hint_text("Group"));
        if ui.button("Save preset").clicked() && !t.new_preset_name.trim().is_empty() {
            let mut s = doc.settings.clone();
            // presets never carry image-specific geometry or spot edits
            s.crop = Crop::default();
            s.retouch.clear();
            s.red_eye.clear();
            match xmp::save_preset(t.new_preset_name.trim(), t.new_preset_group.trim(), &s) {
                Ok(p) => t.status = Some(format!("Saved {}", p.display())),
                Err(e) => t.status = Some(format!("Error: {e}")),
            }
            t.new_preset_name.clear();
            t.presets_loaded = false;
        }
    });
    ui.separator();
    let mut group = String::new();
    let mut builtin_applied: Option<fn(&mut DevelopSettings)> = None;
    for bp in darkslide_core::builtin_presets::all() {
        if bp.group != group {
            group = bp.group.to_string();
            ui.label(RichText::new(&group).strong());
        }
        let r = ui.selectable_label(false, format!("  {}", bp.name));
        if r.clicked() {
            builtin_applied = Some(bp.apply);
        } else if r.hovered() {
            let mut target = doc.settings.clone();
            (bp.apply)(&mut target);
            t.hover_preview = Some(xmp::blend(&doc.settings, &target, t.preset_amount / 100.0));
        }
    }
    if let Some(f) = builtin_applied {
        let mut target = doc.settings.clone();
        f(&mut target);
        doc.settings = xmp::blend(&doc.settings, &target, t.preset_amount / 100.0);
    }
    ui.separator();
    if t.presets.is_empty() {
        ui.label(RichText::new(format!("No presets found in {}", darkslide_core::paths::presets_dir().display())).weak().small());
    }
    let mut group = String::new();
    let mut applied = None;
    for (i, p) in t.presets.iter().enumerate() {
        if p.group != group {
            group = p.group.clone();
            ui.label(RichText::new(&group).strong());
        }
        let r = ui.selectable_label(false, format!("  {}", p.name));
        if r.clicked() {
            applied = Some(i);
        } else if r.hovered() {
            t.hover_preview = Some(p.apply(&doc.settings, t.preset_amount));
        }
    }
    if let Some(i) = applied {
        doc.settings = t.presets[i].apply(&doc.settings, t.preset_amount);
    }
    if let Some(st) = &t.status {
        ui.label(RichText::new(st).small());
    }
}

pub fn masking_panel(ui: &mut egui::Ui, doc: &mut Doc, t: &mut ToolUi, eu: &mut EditUi, pick: &mut Pick) {
    let s = &mut doc.settings;
    ui.heading("Masking");
    ui.label(RichText::new("Create new mask").strong());
    let mut create: Option<MaskShape> = None;
    let mut ai: Option<AiMaskKind> = None;
    ui.horizontal_wrapped(|ui| {
        if ui.button("👤 Subject").clicked() {
            ai = Some(AiMaskKind::Subject);
        }
        if ui.button("☁ Sky").clicked() {
            ai = Some(AiMaskKind::Sky);
        }
        if ui.button("▧ Background").clicked() {
            ai = Some(AiMaskKind::Background);
        }
        if ui.button("🎯 Objects").on_hover_text("Drag a box around the object").clicked() {
            t.mask_tool = MaskTool::ObjectBox;
        }
        if ui.button("🖌 Brush").clicked() {
            create = Some(MaskShape::Brush { strokes: vec![] });
            t.mask_tool = MaskTool::Brush;
        }
        if ui.button("▤ Linear").clicked() {
            t.mask_tool = MaskTool::Linear;
            t.selected_mask = None;
        }
        if ui.button("◯ Radial").clicked() {
            t.mask_tool = MaskTool::Radial;
            t.selected_mask = None;
        }
        if ui.button("🎨 Color range").clicked() {
            create = Some(MaskShape::ColorRange { samples: vec![], refine: 50.0 });
            *pick = Pick::Sampler;
        }
        if ui.button("☀ Luminance range").clicked() {
            create = Some(MaskShape::LuminanceRange { range: [0.6, 1.0], smoothness: 30.0 });
        }
        if ui.button("⛰ Depth range").clicked() {
            create = Some(MaskShape::DepthRange { range: [0.0, 0.3], smoothness: 30.0 });
        }
    });
    ui.menu_button("Landscape ▸", |ui| {
        for k in [
            AiMaskKind::Water,
            AiMaskKind::Vegetation,
            AiMaskKind::Mountains,
            AiMaskKind::Architecture,
            AiMaskKind::NaturalGround,
            AiMaskKind::ArtificialGround,
            AiMaskKind::Snow,
        ] {
            if ui.button(k.label()).clicked() {
                ai = Some(k);
                ui.close();
            }
        }
    });
    if let Some(k) = ai {
        let shape = MaskShape::Ai { kind: k, feather: 0.0, edge: 0.0, hint: None, raster: None };
        s.masks.push(MaskGroup {
            name: format!("{} {}", k.label(), s.masks.len() + 1),
            components: vec![MaskComponent { shape, ..Default::default() }],
            ..Default::default()
        });
        let mi = s.masks.len() - 1;
        t.selected_mask = Some(mi);
        t.selected_comp = Some(0);
        t.request_ai = Some((mi, k));
    }
    if let Some(shape) = create {
        s.masks.push(MaskGroup {
            name: format!("Mask {}", s.masks.len() + 1),
            components: vec![MaskComponent { shape, ..Default::default() }],
            ..Default::default()
        });
        t.selected_mask = Some(s.masks.len() - 1);
        t.selected_comp = Some(0);
    }
    ui.separator();

    // mask list
    let mut del = None;
    for (i, m) in s.masks.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let sel = t.selected_mask == Some(i);
            ui.checkbox(&mut m.enabled, "");
            if ui.selectable_label(sel, &m.name).clicked() {
                t.selected_mask = Some(i);
                t.selected_comp = Some(0);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("🗑").clicked() {
                    del = Some(i);
                }
            });
        });
    }
    if let Some(i) = del {
        s.masks.remove(i);
        t.selected_mask = None;
    }
    ui.checkbox(&mut t.show_mask_overlay, "Show overlay (Y)");

    let Some(mi) = t.selected_mask.filter(|&i| i < s.masks.len()) else {
        ui.label(RichText::new("Select or create a mask.").weak());
        return;
    };
    ui.separator();
    let m = &mut s.masks[mi];
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(140.0));
        ui.checkbox(&mut m.invert, "Invert");
    });
    // components
    ui.label(RichText::new("Components").strong());
    let mut cdel = None;
    for (ci, c) in m.components.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let name = match &c.shape {
                MaskShape::Brush { .. } => "Brush".to_string(),
                MaskShape::Linear { .. } => "Linear gradient".into(),
                MaskShape::Radial { .. } => "Radial gradient".into(),
                MaskShape::ColorRange { .. } => "Color range".into(),
                MaskShape::LuminanceRange { .. } => "Luminance range".into(),
                MaskShape::DepthRange { .. } => "Depth range".into(),
                MaskShape::Ai { kind, .. } => kind.label().to_string(),
            };
            let op = match (ci, c.op) {
                (0, _) => "",
                (_, MaskOp::Add) => "+ ",
                (_, MaskOp::Subtract) => "− ",
                (_, MaskOp::Intersect) => "∩ ",
            };
            if ui.selectable_label(t.selected_comp == Some(ci), format!("{op}{name}")).clicked() {
                t.selected_comp = Some(ci);
                t.mask_tool = match c.shape {
                    MaskShape::Brush { .. } => MaskTool::Brush,
                    MaskShape::Linear { .. } => MaskTool::Linear,
                    MaskShape::Radial { .. } => MaskTool::Radial,
                    _ => MaskTool::None,
                };
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if m_comp_count_gt1(ci) && ui.small_button("🗑").clicked() {
                    cdel = Some(ci);
                }
                ui.checkbox(&mut c.invert, "inv");
            });
        });
    }
    if let Some(ci) = cdel {
        if m.components.len() > 1 {
            m.components.remove(ci);
            t.selected_comp = Some(0);
        }
    }
    ui.horizontal(|ui| {
        for (op, lab) in [(MaskOp::Add, "Add"), (MaskOp::Subtract, "Subtract"), (MaskOp::Intersect, "Intersect")] {
            ui.menu_button(lab, |ui| {
                let mut add = |shape: MaskShape, tool: MaskTool, ui: &mut egui::Ui| {
                    m.components.push(MaskComponent { op, shape, ..Default::default() });
                    t.selected_comp = Some(m.components.len() - 1);
                    t.mask_tool = tool;
                    ui.close();
                };
                if ui.button("Brush").clicked() {
                    add(MaskShape::Brush { strokes: vec![] }, MaskTool::Brush, ui);
                }
                if ui.button("Linear gradient").clicked() {
                    add(MaskShape::Linear { start: [0.5, 0.3], end: [0.5, 0.6], two_sided: false }, MaskTool::Linear, ui);
                }
                if ui.button("Radial gradient").clicked() {
                    add(MaskShape::Radial { center: [0.5, 0.5], radius: [0.2, 0.2], angle: 0.0, feather: 50.0 }, MaskTool::Radial, ui);
                }
                if ui.button("Color range").clicked() {
                    add(MaskShape::ColorRange { samples: vec![], refine: 50.0 }, MaskTool::None, ui);
                    *pick = Pick::Sampler;
                }
                if ui.button("Luminance range").clicked() {
                    add(MaskShape::LuminanceRange { range: [0.6, 1.0], smoothness: 30.0 }, MaskTool::None, ui);
                }
                if ui.button("Depth range").clicked() {
                    add(MaskShape::DepthRange { range: [0.0, 0.3], smoothness: 30.0 }, MaskTool::None, ui);
                }
                for k in [AiMaskKind::Subject, AiMaskKind::Sky, AiMaskKind::Background] {
                    if ui.button(k.label()).clicked() {
                        add(MaskShape::Ai { kind: k, feather: 0.0, edge: 0.0, hint: None, raster: None }, MaskTool::None, ui);
                        t.request_ai = Some((mi, k));
                    }
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
                    ui.selectable_value(&mut b.erase, true, "Erase (Alt)");
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
            MaskShape::Radial { feather, .. } => {
                slider(ui, feather, SliderSpec::new("Feather", 0.0, 100.0, 50.0));
            }
            MaskShape::ColorRange { samples, refine } => {
                ui.horizontal(|ui| {
                    let active = *pick == Pick::Sampler;
                    if ui.selectable_label(active, "💧 Add color").clicked() {
                        *pick = if active { Pick::None } else { Pick::Sampler };
                    }
                    ui.label(format!("{} sample(s)", samples.len()));
                    if ui.small_button("Clear").clicked() {
                        samples.clear();
                    }
                });
                slider(ui, refine, SliderSpec::new("Refine", 0.0, 100.0, 50.0));
            }
            MaskShape::LuminanceRange { range, smoothness } | MaskShape::DepthRange { range, smoothness } => {
                let mut lo = range[0] * 100.0;
                let mut hi = range[1] * 100.0;
                if slider(ui, &mut lo, SliderSpec::new("Range low", 0.0, 100.0, 60.0)) {
                    range[0] = lo / 100.0;
                }
                if slider(ui, &mut hi, SliderSpec::new("Range high", 0.0, 100.0, 100.0)) {
                    range[1] = hi / 100.0;
                }
                slider(ui, smoothness, SliderSpec::new("Smoothness", 0.0, 100.0, 30.0));
            }
            MaskShape::Ai { kind, feather, edge, raster, .. } => {
                ui.label(if raster.is_some() { format!("{} detected", kind.label()) } else { "Detecting…".to_string() });
                slider(ui, feather, SliderSpec::new("Feather", 0.0, 100.0, 0.0));
                slider(ui, edge, SliderSpec::new("Edge", -100.0, 100.0, 0.0));
                if ui.button("Redetect").clicked() {
                    *raster = None;
                    t.request_ai = Some((mi, *kind));
                }
            }
        }
    }
    ui.separator();
    // local adjustments
    let a = &mut m.adjust;
    slider(ui, &mut m.amount, SliderSpec::new("Amount", 0.0, 200.0, 100.0));
    if ui.small_button("Reset sliders").clicked() {
        *a = LocalAdjust::default();
    }
    let gt = crate::widgets::grad_temp();
    let gti = crate::widgets::grad_tint();
    slider(ui, &mut a.temperature, SliderSpec::new("Temperature", -100.0, 100.0, 0.0).grad(&gt));
    slider(ui, &mut a.tint, SliderSpec::new("Tint", -100.0, 100.0, 0.0).grad(&gti));
    slider(ui, &mut a.exposure, SliderSpec::new("Exposure", -4.0, 4.0, 0.0).dec(2));
    slider(ui, &mut a.contrast, SliderSpec::new("Contrast", -100.0, 100.0, 0.0));
    slider(ui, &mut a.highlights, SliderSpec::new("Highlights", -100.0, 100.0, 0.0));
    slider(ui, &mut a.shadows, SliderSpec::new("Shadows", -100.0, 100.0, 0.0));
    slider(ui, &mut a.whites, SliderSpec::new("Whites", -100.0, 100.0, 0.0));
    slider(ui, &mut a.blacks, SliderSpec::new("Blacks", -100.0, 100.0, 0.0));
    slider(ui, &mut a.texture, SliderSpec::new("Texture", -100.0, 100.0, 0.0));
    slider(ui, &mut a.clarity, SliderSpec::new("Clarity", -100.0, 100.0, 0.0));
    slider(ui, &mut a.dehaze, SliderSpec::new("Dehaze", -100.0, 100.0, 0.0));
    let gh = crate::widgets::grad_hue_full();
    slider(ui, &mut a.hue, SliderSpec::new("Hue", -100.0, 100.0, 0.0).grad(&gh));
    slider(ui, &mut a.saturation, SliderSpec::new("Saturation", -100.0, 100.0, 0.0));
    slider(ui, &mut a.sharpness, SliderSpec::new("Sharpness", -100.0, 100.0, 0.0));
    slider(ui, &mut a.noise, SliderSpec::new("Noise", -100.0, 100.0, 0.0));
    slider(ui, &mut a.moire, SliderSpec::new("Moiré", -100.0, 100.0, 0.0));
    slider(ui, &mut a.defringe, SliderSpec::new("Defringe", -100.0, 100.0, 0.0));
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
        if ui.small_button("✖").clicked() {
            a.color = [0.0, 0.0];
        }
    });
    egui::CollapsingHeader::new("Color grading").id_salt(("mgrade", mi)).show(ui, |ui| {
        edit_panel::grading_ui(ui, &mut a.grading, &mut t.mask_grading_view, "mask");
    });
    egui::CollapsingHeader::new("Curve").id_salt(("mcurve", mi)).show(ui, |ui| {
        crate::widgets::curve_editor(
            ui,
            crate::widgets::CurveEdit { points: &mut a.curve.rgb, color: Color32::from_gray(230), hist: None, id: ui.id().with(("mc", mi)) },
            &[],
        );
    });
    egui::CollapsingHeader::new("Point color").id_salt(("mpc", mi)).show(ui, |ui| {
        edit_panel::point_color_ui(ui, &mut a.point_colors, &mut t.mask_point_sel, pick);
    });
    let _ = eu;
}

fn m_comp_count_gt1(_ci: usize) -> bool {
    true
}
