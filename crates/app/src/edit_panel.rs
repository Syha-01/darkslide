//! The Edit panel: Profile, Basic, Curve, Detail, Color Mixer, Point Color,
//! Color Grading, Optics, Geometry, Effects, Calibration.

use darkslide_core::analysis::Histogram;
use darkslide_core::params::*;
use darkslide_core::pipeline::curves::{Lut, point_curve_lut};
use darkslide_core::profiles::{self, BUILTIN_LOOKS, PROFILES};
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::doc::Doc;
use crate::widgets::{self, SliderSpec, section, slider};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Pick {
    #[default]
    None,
    WhiteBalance,
    PointColor,
    Sampler,
    /// Targeted adjustment: drag in the image to move the point curve.
    TatCurve,
    /// Targeted adjustment for the colour mixer (which tab).
    TatMixer(u8),
    Upright,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CurveMode {
    #[default]
    Parametric,
    Rgb,
    Red,
    Green,
    Blue,
}

#[derive(Default)]
pub struct EditUi {
    pub curve_mode: CurveMode,
    /// 0 hue, 1 sat, 2 lum, 3 all
    pub mixer_tab: u8,
    /// 0 three-way, 1 shadows, 2 mid, 3 highlights, 4 global
    pub grading_view: u8,
    pub point_sel: usize,
    pub show_scope: bool,
    pub visualize_point: bool,
}

pub const BAND_HSV: [f32; 8] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 300.0];

pub struct PanelCtx<'a> {
    pub hist: Option<&'a Histogram>,
    pub pick: &'a mut Pick,
    /// Actions the app should perform (needs engine / async work).
    pub actions: &'a mut Vec<Action>,
}

#[derive(Clone, Debug)]
pub enum Action {
    AutoTone,
    AutoWb,
    DetectDust,
    RemovePeople,
    BrowseProfiles,
    RemoveWires,
    UprightAnalyze(Upright),
    ImportLook,
    AutoGrayMix,
}

pub fn edit_panel(ui: &mut egui::Ui, doc: &mut Doc, st: &mut EditUi, pc: &mut PanelCtx) {
    let Some(engine) = doc.engine.clone() else {
        ui.label("Loading…");
        return;
    };
    let is_raw = engine.is_raw();
    let s = &mut doc.settings;

    // ------------------------------------------------------------- Profile
    section(ui, "Profile", None, true, |ui| {
        ui.horizontal(|ui| {
            let c_sel = s.treatment == Treatment::Color;
            let bw_sel = s.treatment == Treatment::BlackAndWhite;
            let c_btn = egui::Button::new(RichText::new("Color").size(12.0).color(if c_sel { Color32::WHITE } else { Color32::from_rgb(165, 165, 165) }))
                .fill(if c_sel { Color32::from_rgb(52, 52, 52) } else { Color32::from_rgb(32, 32, 32) })
                .stroke(if c_sel { Stroke::new(1.0, Color32::from_rgb(72, 72, 72)) } else { Stroke::new(1.0, Color32::from_rgb(44, 44, 44)) })
                .corner_radius(3.0)
                .min_size(egui::vec2(60.0, 22.0));
            if ui.add(c_btn).clicked() {
                s.treatment = Treatment::Color;
            }
            let bw_btn = egui::Button::new(RichText::new("Black & White").size(12.0).color(if bw_sel { Color32::WHITE } else { Color32::from_rgb(165, 165, 165) }))
                .fill(if bw_sel { Color32::from_rgb(52, 52, 52) } else { Color32::from_rgb(32, 32, 32) })
                .stroke(if bw_sel { Stroke::new(1.0, Color32::from_rgb(72, 72, 72)) } else { Stroke::new(1.0, Color32::from_rgb(44, 44, 44)) })
                .corner_radius(3.0)
                .min_size(egui::vec2(100.0, 22.0));
            if ui.add(bw_btn).clicked() {
                s.treatment = Treatment::BlackAndWhite;
            }
        });
        ui.add_space(4.0);
        let shown = s.profile.strip_prefix("dcp:").map(|p| dcp_name(p)).unwrap_or_else(|| s.profile.clone());
        let full_w = ui.available_width();
        egui::ComboBox::from_id_salt("profile").width((full_w - 12.0).max(60.0)).selected_text(shown).show_ui(ui, |ui| {
            let mut group = "";
            for p in PROFILES {
                if p.group != group {
                    group = p.group;
                    ui.label(RichText::new(group).weak().small());
                }
                if ui.selectable_label(s.profile == p.name, p.name).clicked() {
                    s.profile = p.name.to_string();
                    if p.monochrome {
                        s.treatment = Treatment::BlackAndWhite;
                    } else if s.treatment == Treatment::BlackAndWhite && !p.monochrome {
                        s.treatment = Treatment::Color;
                    }
                }
            }
            if is_raw {
                let m = &engine.source.meta;
                let dcps: Vec<(String, String)> =
                    dcp_catalog().iter().filter(|d| d.matches(&m.make, &m.model)).map(|d| (d.name.clone(), d.path.to_string_lossy().to_string())).collect();
                if !dcps.is_empty() {
                    ui.label(RichText::new("Camera profiles (DCP)").weak().small());
                }
                for (name, path) in dcps {
                    let id = format!("dcp:{path}");
                    if ui.selectable_label(s.profile == id, name).clicked() {
                        s.profile = id;
                        s.treatment = Treatment::Color;
                    }
                }
            }
        });
        let prof = profiles::find(&s.profile);
        if prof.adaptive || prof.saturation != 1.0 || prof.contrast != 1.0 {
            slider(ui, &mut s.profile_amount, SliderSpec::new("Amount", 0.0, 200.0, 100.0));
        }
        ui.add_space(4.0);
        let look_name = s.look.as_ref().map(|l| l.name.clone()).unwrap_or_else(|| "None".into());
        ui.horizontal(|ui| {
            ui.label("Look");
            egui::ComboBox::from_id_salt("look").width((ui.available_width() - 80.0).max(60.0)).selected_text(look_name).show_ui(ui, |ui| {
                if ui.selectable_label(s.look.is_none(), "None").clicked() {
                    s.look = None;
                }
                ui.label(RichText::new("Creative").weak().small());
                for name in BUILTIN_LOOKS {
                    let src = format!("builtin:{name}");
                    let sel = s.look.as_ref().map(|l| l.source == src).unwrap_or(false);
                    if ui.selectable_label(sel, *name).clicked() {
                        let amount = s.look.as_ref().map(|l| l.amount).unwrap_or(100.0);
                        s.look = Some(Look { name: name.to_string(), source: src, amount });
                    }
                }
                let user = profiles::list_user_looks();
                if !user.is_empty() {
                    ui.label(RichText::new("User (.cube)").weak().small());
                }
                for (name, path) in user {
                    let sel = s.look.as_ref().map(|l| l.source == path).unwrap_or(false);
                    if ui.selectable_label(sel, &name).clicked() {
                        let amount = s.look.as_ref().map(|l| l.amount).unwrap_or(100.0);
                        s.look = Some(Look { name, source: path, amount });
                    }
                }
            });
            if ui.button("▦").on_hover_text("Browse profiles and looks").clicked() {
                pc.actions.push(Action::BrowseProfiles);
            }
            if ui.button("+").on_hover_text("Import a .cube LUT as a profile").clicked() {
                pc.actions.push(Action::ImportLook);
            }
        });
        if let Some(l) = s.look.as_mut() {
            slider(ui, &mut l.amount, SliderSpec::new("Look amount", 0.0, 200.0, 100.0));
        }
    });

    // ------------------------------------------------------------- Basic
    let mut basic_on = s.panels.basic;
    section(ui, "Basic", Some(&mut basic_on), true, |ui| {
        widgets::sub_header(ui, "WHITE BALANCE");
        ui.horizontal(|ui| {
            let full_w = ui.available_width();
            egui::ComboBox::from_id_salt("wb").selected_text(s.wb_mode.label()).width((full_w - 36.0).max(60.0)).show_ui(ui, |ui| {
                let modes: &[WbMode] = if is_raw { &WbMode::ALL } else { &[WbMode::AsShot, WbMode::Auto, WbMode::Custom] };
                for m in modes {
                    if ui.selectable_label(s.wb_mode == *m, m.label()).clicked() {
                        if *m == WbMode::Custom {
                            let (t, ti) = engine.resolved_temp_tint(s);
                            s.temperature = t;
                            s.tint = ti;
                        } else if *m == WbMode::Auto {
                            pc.actions.push(Action::AutoWb);
                        }
                        s.wb_mode = *m;
                    }
                }
            });
            let active = *pc.pick == Pick::WhiteBalance;
            if widgets::eyedropper_button(ui, active, "White balance tool (I)") {
                *pc.pick = if active { Pick::None } else { Pick::WhiteBalance };
            }
        });
        let (mut t, mut ti) = engine.resolved_temp_tint(s);
        let gt = widgets::grad_temp();
        let gti = widgets::grad_tint();
        let (def_t, def_ti) = engine.resolved_temp_tint(&DevelopSettings { wb_mode: WbMode::AsShot, ..s.clone() });
        let tchg = if is_raw {
            slider(ui, &mut t, SliderSpec::new("Temp", 1500.0, 50000.0, def_t).grad(&gt).log())
        } else {
            slider(ui, &mut t, SliderSpec::new("Temp", -100.0, 100.0, 0.0).grad(&gt))
        };
        let tichg = slider(ui, &mut ti, SliderSpec::new("Tint", -150.0, 150.0, def_ti).grad(&gti));
        if tchg || tichg {
            s.wb_mode = WbMode::Custom;
            s.temperature = t;
            s.tint = ti;
        }

        widgets::sub_header_tone(
            ui,
            || pc.actions.push(Action::AutoTone),
            || {
                s.exposure = 0.0;
                s.contrast = 0.0;
                s.highlights = 0.0;
                s.shadows = 0.0;
                s.whites = 0.0;
                s.blacks = 0.0;
            },
        );
        let max_ev = if s.hdr.enabled { 10.0 } else { 5.0 };
        slider(ui, &mut s.exposure, SliderSpec::new("Exposure", -max_ev, max_ev, 0.0).dec(2));
        slider(ui, &mut s.contrast, SliderSpec::new("Contrast", -100.0, 100.0, 0.0));
        slider(ui, &mut s.highlights, SliderSpec::new("Highlights", -100.0, 100.0, 0.0));
        slider(ui, &mut s.shadows, SliderSpec::new("Shadows", -100.0, 100.0, 0.0));
        slider(ui, &mut s.whites, SliderSpec::new("Whites", -100.0, 100.0, 0.0));
        slider(ui, &mut s.blacks, SliderSpec::new("Blacks", -100.0, 100.0, 0.0));

        widgets::sub_header(ui, "PRESENCE");
        slider(ui, &mut s.texture, SliderSpec::new("Texture", -100.0, 100.0, 0.0));
        slider(ui, &mut s.clarity, SliderSpec::new("Clarity", -100.0, 100.0, 0.0));
        slider(ui, &mut s.dehaze, SliderSpec::new("Dehaze", -100.0, 100.0, 0.0));
        slider(ui, &mut s.vibrance, SliderSpec::new("Vibrance", -100.0, 100.0, 0.0));
        slider(ui, &mut s.saturation, SliderSpec::new("Saturation", -100.0, 100.0, 0.0));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.hdr.enabled, "HDR").on_hover_text("Edit in high dynamic range (extended highlights)");
            if s.hdr.enabled {
                ui.checkbox(&mut s.hdr.visualize, "Visualize HDR");
            }
        });
        if s.hdr.enabled {
            slider(ui, &mut s.hdr.max_headroom, SliderSpec::new("HDR headroom", 1.0, 4.0, 4.0).dec(1).suffix(" st"));
            ui.collapsing("SDR settings", |ui| {
                slider(ui, &mut s.hdr.sdr_brightness, SliderSpec::new("Brightness", -100.0, 100.0, 0.0));
                slider(ui, &mut s.hdr.sdr_contrast, SliderSpec::new("Contrast", -100.0, 100.0, 0.0));
                slider(ui, &mut s.hdr.sdr_clarity, SliderSpec::new("Clarity", -100.0, 100.0, 0.0));
                slider(ui, &mut s.hdr.sdr_highlights, SliderSpec::new("Highlights", -100.0, 100.0, 0.0));
                slider(ui, &mut s.hdr.sdr_shadows, SliderSpec::new("Shadows", -100.0, 100.0, 0.0));
                slider(ui, &mut s.hdr.sdr_whites, SliderSpec::new("Whites", -100.0, 100.0, 0.0));
                slider(ui, &mut s.hdr.sdr_blend, SliderSpec::new("Blend", 0.0, 100.0, 0.0));
            });
        }
    });
    s.panels.basic = basic_on;

    // ------------------------------------------------------------- Curve
    let mut curve_on = s.panels.curve;
    section(ui, "Curve", Some(&mut curve_on), false, |ui| {
        ui.horizontal(|ui| {
            for (m, lab, col) in [
                (CurveMode::Parametric, "◐", Color32::GRAY),
                (CurveMode::Rgb, "⚪", Color32::WHITE),
                (CurveMode::Red, "●", Color32::from_rgb(230, 70, 70)),
                (CurveMode::Green, "●", Color32::from_rgb(70, 200, 70)),
                (CurveMode::Blue, "●", Color32::from_rgb(80, 120, 240)),
            ] {
                let txt = RichText::new(lab).color(col).size(15.0);
                if ui.selectable_label(st.curve_mode == m, txt).clicked() {
                    st.curve_mode = m;
                }
            }
            let active = *pc.pick == Pick::TatCurve;
            if ui.selectable_label(active, "⊕").on_hover_text("Targeted adjustment: drag up/down in the image").clicked() {
                *pc.pick = if active { Pick::None } else { Pick::TatCurve };
            }
        });
        let lum_hist = pc.hist.map(|h| h.l.as_slice());
        match st.curve_mode {
            CurveMode::Parametric => {
                let lut = darkslide_core::pipeline::curves::parametric_lut(&s.parametric);
                // draw the parametric curve read-only
                let mut pts: CurvePoints = (0..=16)
                    .map(|i| {
                        let x = i as f32 / 16.0;
                        [x * 255.0, lut.eval(x) * 255.0]
                    })
                    .collect();
                let snapshot = pts.clone();
                widgets::curve_editor(
                    ui,
                    widgets::CurveEdit { points: &mut pts, color: Color32::from_gray(220), hist: lum_hist, id: ui.id().with("pcurve") },
                    &[],
                );
                let _ = snapshot;
                slider(ui, &mut s.parametric.highlights, SliderSpec::new("Highlights", -100.0, 100.0, 0.0));
                slider(ui, &mut s.parametric.lights, SliderSpec::new("Lights", -100.0, 100.0, 0.0));
                slider(ui, &mut s.parametric.darks, SliderSpec::new("Darks", -100.0, 100.0, 0.0));
                slider(ui, &mut s.parametric.shadows, SliderSpec::new("Shadows", -100.0, 100.0, 0.0));
                ui.collapsing("Region splits", |ui| {
                    slider(ui, &mut s.parametric.shadow_split, SliderSpec::new("Shadows split", 10.0, 70.0, 25.0));
                    slider(ui, &mut s.parametric.midtone_split, SliderSpec::new("Midtones split", 20.0, 80.0, 50.0));
                    slider(ui, &mut s.parametric.highlight_split, SliderSpec::new("Highlights split", 30.0, 90.0, 75.0));
                });
            }
            mode => {
                let others: Vec<(Lut, Color32)> = match mode {
                    CurveMode::Rgb => vec![],
                    _ => vec![(point_curve_lut(&s.curve.rgb), Color32::WHITE)],
                };
                let others_ref: Vec<(&Lut, Color32)> = others.iter().map(|(l, c)| (l, *c)).collect();
                let (pts, col, hist) = match mode {
                    CurveMode::Rgb => (&mut s.curve.rgb, Color32::from_gray(230), lum_hist),
                    CurveMode::Red => (&mut s.curve.red, Color32::from_rgb(230, 70, 70), pc.hist.map(|h| h.r.as_slice())),
                    CurveMode::Green => (&mut s.curve.green, Color32::from_rgb(70, 200, 70), pc.hist.map(|h| h.g.as_slice())),
                    _ => (&mut s.curve.blue, Color32::from_rgb(80, 120, 240), pc.hist.map(|h| h.b.as_slice())),
                };
                widgets::curve_editor(ui, widgets::CurveEdit { points: pts, color: col, hist, id: ui.id().with(("curve", mode as u8)) }, &others_ref);
                ui.horizontal(|ui| {
                    if ui.small_button("Reset channel").clicked() {
                        *pts = linear_curve();
                    }
                    ui.label(RichText::new("click: add · drag out: remove").weak().small());
                });
            }
        }
        slider(ui, &mut s.refine_saturation, SliderSpec::new("Refine saturation", 0.0, 100.0, 100.0));
    });
    s.panels.curve = curve_on;

    // ------------------------------------------------------------- Detail
    let mut detail_on = s.panels.detail;
    section(ui, "Detail", Some(&mut detail_on), false, |ui| {
        ui.label(RichText::new("Sharpening").strong());
        slider(ui, &mut s.sharpen.amount, SliderSpec::new("Sharpening", 0.0, 150.0, if is_raw { 40.0 } else { 0.0 }));
        slider(ui, &mut s.sharpen.radius, SliderSpec::new("Radius", 0.5, 3.0, 1.0).dec(1));
        slider(ui, &mut s.sharpen.detail, SliderSpec::new("Detail", 0.0, 100.0, 25.0));
        slider(ui, &mut s.sharpen.masking, SliderSpec::new("Masking", 0.0, 100.0, 0.0));
        ui.label(RichText::new("Noise reduction").strong());
        slider(ui, &mut s.nr.luminance, SliderSpec::new("Noise reduction", 0.0, 100.0, 0.0));
        slider(ui, &mut s.nr.luminance_detail, SliderSpec::new("Detail", 0.0, 100.0, 50.0));
        slider(ui, &mut s.nr.luminance_contrast, SliderSpec::new("Contrast", 0.0, 100.0, 0.0));
        slider(ui, &mut s.nr.color, SliderSpec::new("Color noise reduction", 0.0, 100.0, if is_raw { 25.0 } else { 0.0 }));
        slider(ui, &mut s.nr.color_detail, SliderSpec::new("Detail", 0.0, 100.0, 50.0));
        slider(ui, &mut s.nr.color_smoothness, SliderSpec::new("Smoothness", 0.0, 100.0, 50.0));
        ui.label(RichText::new("Enhance").strong());
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.enhance.denoise, "Denoise");
            if s.enhance.denoise {
                ui.add(egui::Slider::new(&mut s.enhance.denoise_amount, 0.0..=100.0).show_value(true));
            }
        });
        ui.checkbox(&mut s.enhance.raw_details, "Raw Details").on_hover_text("Higher quality demosaic for finer detail");
        ui.checkbox(&mut s.enhance.super_resolution, "Super Resolution (2×)").on_hover_text("Doubles linear resolution on export");
    });
    s.panels.detail = detail_on;

    // ------------------------------------------------------------- Color Mixer
    let mut mixer_on = s.panels.mixer;
    let title = if s.is_bw() { "B&W Mixer" } else { "Color Mixer" };
    section(ui, title, Some(&mut mixer_on), false, |ui| {
        if s.is_bw() {
            ui.horizontal(|ui| {
                if ui.button("Auto").clicked() {
                    pc.actions.push(Action::AutoGrayMix);
                }
                if ui.button("Default").clicked() {
                    s.gray_mix = [0.0; 8];
                }
                let active = *pc.pick == Pick::TatMixer(3);
                if ui.selectable_label(active, "⊕").on_hover_text("Targeted adjustment").clicked() {
                    *pc.pick = if active { Pick::None } else { Pick::TatMixer(3) };
                }
            });
            for (i, b) in HSL_BANDS.iter().enumerate() {
                let g = widgets::grad_band_lum(BAND_HSV[i]);
                slider(ui, &mut s.gray_mix[i], SliderSpec::new(b, -100.0, 100.0, 0.0).grad(&g));
            }
        } else {
            ui.horizontal(|ui| {
                for (i, t) in ["Hue", "Saturation", "Luminance", "All"].iter().enumerate() {
                    ui.selectable_value(&mut st.mixer_tab, i as u8, *t);
                }
                let active = matches!(*pc.pick, Pick::TatMixer(_));
                if ui.selectable_label(active, "⊕").on_hover_text("Targeted adjustment: drag in the image").clicked() {
                    *pc.pick = if active { Pick::None } else { Pick::TatMixer(st.mixer_tab.min(2)) };
                }
            });
            for tab in 0..3u8 {
                if st.mixer_tab != tab && st.mixer_tab != 3 {
                    continue;
                }
                if st.mixer_tab == 3 {
                    ui.label(RichText::new(["Hue", "Saturation", "Luminance"][tab as usize]).strong());
                }
                for (i, b) in HSL_BANDS.iter().enumerate() {
                    let c = BAND_HSV[i];
                    let (v, g) = match tab {
                        0 => (&mut s.hsl[i].hue, widgets::grad_band_hue(c)),
                        1 => (&mut s.hsl[i].saturation, widgets::grad_band_sat(c)),
                        _ => (&mut s.hsl[i].luminance, widgets::grad_band_lum(c)),
                    };
                    slider(ui, v, SliderSpec::new(b, -100.0, 100.0, 0.0).grad(&g));
                }
            }
        }
    });
    s.panels.mixer = mixer_on;

    // ------------------------------------------------------------- Point Color
    let mut pc_on = s.panels.point_color;
    section(ui, "Point Color", Some(&mut pc_on), false, |ui| {
        point_color_ui_ext(ui, &mut s.point_colors, &mut st.point_sel, pc.pick, Some(&mut st.visualize_point));
    });
    s.panels.point_color = pc_on;

    // ------------------------------------------------------------- Color Grading
    let mut grade_on = s.panels.grading;
    section(ui, "Color Grading", Some(&mut grade_on), false, |ui| {
        grading_ui(ui, &mut s.grading, &mut st.grading_view, "global");
    });
    s.panels.grading = grade_on;

    // ------------------------------------------------------------- Optics
    let mut optics_on = s.panels.optics;
    section(ui, "Optics", Some(&mut optics_on), false, |ui| {
        ui.checkbox(&mut s.lens.remove_ca, "Remove chromatic aberration");
        ui.checkbox(&mut s.lens.profile_enable, "Use profile corrections");
        if s.lens.profile_enable {
            let db = engine.lens_db();
            let lens = engine.lens_profile(s);
            ui.label(
                RichText::new(match (&lens, db.is_empty()) {
                    (Some(p), _) => format!("{} {}", p.maker, p.model),
                    (None, true) => "No lens database found (install lensfun)".into(),
                    (None, false) => format!("No profile for {}", engine.source.meta.lens.clone().unwrap_or_default()),
                })
                .small(),
            );
            let mut name = s.lens.profile.clone().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label("Lens");
                if ui.add(egui::TextEdit::singleline(&mut name).hint_text("auto from EXIF").desired_width(160.0)).changed() {
                    s.lens.profile = if name.trim().is_empty() { None } else { Some(name.clone()) };
                }
            });
            slider(ui, &mut s.lens.distortion_scale, SliderSpec::new("Distortion", 0.0, 200.0, 100.0));
            slider(ui, &mut s.lens.vignette_scale, SliderSpec::new("Vignette", 0.0, 200.0, 100.0));
        }
        ui.label(RichText::new("Manual").strong());
        slider(ui, &mut s.lens.manual_distortion, SliderSpec::new("Distortion", -100.0, 100.0, 0.0));
        slider(ui, &mut s.lens.vignette_amount, SliderSpec::new("Vignetting", -100.0, 100.0, 0.0));
        slider(ui, &mut s.lens.vignette_midpoint, SliderSpec::new("Midpoint", 0.0, 100.0, 50.0));
        ui.label(RichText::new("Defringe").strong());
        let gp = vec![widgets::hue_color(260.0, 0.8, 0.9), widgets::hue_color(300.0, 0.8, 0.9)];
        let gg = vec![widgets::hue_color(60.0, 0.8, 0.9), widgets::hue_color(150.0, 0.8, 0.9)];
        slider(ui, &mut s.lens.defringe_purple_amount, SliderSpec::new("Purple amount", 0.0, 20.0, 0.0));
        slider(ui, &mut s.lens.defringe_purple_hue[0], SliderSpec::new("Purple hue (low)", 0.0, 90.0, 30.0).grad(&gp));
        slider(ui, &mut s.lens.defringe_purple_hue[1], SliderSpec::new("Purple hue (high)", 10.0, 100.0, 70.0).grad(&gp));
        slider(ui, &mut s.lens.defringe_green_amount, SliderSpec::new("Green amount", 0.0, 20.0, 0.0));
        slider(ui, &mut s.lens.defringe_green_hue[0], SliderSpec::new("Green hue (low)", 0.0, 90.0, 40.0).grad(&gg));
        slider(ui, &mut s.lens.defringe_green_hue[1], SliderSpec::new("Green hue (high)", 10.0, 100.0, 60.0).grad(&gg));
        ui.label(RichText::new("Lens Blur").strong());
        ui.checkbox(&mut s.lens_blur.enabled, "Apply lens blur");
        if s.lens_blur.enabled {
            slider(ui, &mut s.lens_blur.amount, SliderSpec::new("Blur amount", 0.0, 100.0, 50.0));
            ui.horizontal(|ui| {
                ui.label("Bokeh");
                egui::ComboBox::from_id_salt("bokeh").selected_text(format!("{:?}", s.lens_blur.shape)).show_ui(ui, |ui| {
                    for b in [BokehShape::Circle, BokehShape::Bubble, BokehShape::FiveBlade, BokehShape::Ring, BokehShape::CatEye] {
                        ui.selectable_value(&mut s.lens_blur.shape, b, format!("{b:?}"));
                    }
                });
            });
            let mut near = s.lens_blur.focal_range[0] * 100.0;
            let mut far = s.lens_blur.focal_range[1] * 100.0;
            if slider(ui, &mut near, SliderSpec::new("Focal range near", 0.0, 100.0, 0.0)) {
                s.lens_blur.focal_range[0] = near / 100.0;
            }
            if slider(ui, &mut far, SliderSpec::new("Focal range far", 0.0, 100.0, 20.0)) {
                s.lens_blur.focal_range[1] = far / 100.0;
            }
            slider(ui, &mut s.lens_blur.boost, SliderSpec::new("Boost", 0.0, 100.0, 50.0));
            ui.checkbox(&mut s.lens_blur.visualize_depth, "Visualize depth");
        }
        ui.label(RichText::new("Distraction removal").strong());
        ui.horizontal(|ui| {
            if ui.button("Remove dust spots").on_hover_text("Detect sensor dust and add heal spots").clicked() {
                pc.actions.push(Action::DetectDust);
            }
        });
    });
    s.panels.optics = optics_on;

    // ------------------------------------------------------------- Geometry
    let mut geo_on = s.panels.geometry;
    section(ui, "Geometry", Some(&mut geo_on), false, |ui| {
        ui.label("Upright");
        ui.horizontal_wrapped(|ui| {
            for (u, lab, tip) in [
                (Upright::Off, "Off", "Disable upright"),
                (Upright::Auto, "A", "Auto: balanced level, aspect and perspective"),
                (Upright::Level, "Level", "Level the horizon"),
                (Upright::Vertical, "Vertical", "Level and fix converging verticals"),
                (Upright::Full, "Full", "Level, vertical and horizontal perspective"),
                (Upright::Guided, "Guided", "Draw up to four guides on the image"),
            ] {
                if ui.selectable_label(s.transform.upright == u, lab).on_hover_text(tip).clicked() {
                    s.transform.upright = u;
                    match u {
                        Upright::Off => s.transform.upright_matrix = None,
                        Upright::Guided => *pc.pick = Pick::Upright,
                        _ => pc.actions.push(Action::UprightAnalyze(u)),
                    }
                }
            }
        });
        if s.transform.upright == Upright::Guided {
            ui.horizontal(|ui| {
                ui.label(format!("{} guide(s)", s.transform.guides.len()));
                if ui.small_button("Clear").clicked() {
                    s.transform.guides.clear();
                    s.transform.upright_matrix = None;
                }
                if !s.transform.guides.is_empty() && ui.small_button("Apply").clicked() {
                    pc.actions.push(Action::UprightAnalyze(Upright::Guided));
                }
            });
        }
        slider(ui, &mut s.transform.vertical, SliderSpec::new("Vertical", -100.0, 100.0, 0.0));
        slider(ui, &mut s.transform.horizontal, SliderSpec::new("Horizontal", -100.0, 100.0, 0.0));
        slider(ui, &mut s.transform.rotate, SliderSpec::new("Rotate", -10.0, 10.0, 0.0).dec(1));
        slider(ui, &mut s.transform.aspect, SliderSpec::new("Aspect", -100.0, 100.0, 0.0));
        slider(ui, &mut s.transform.scale, SliderSpec::new("Scale", 50.0, 150.0, 100.0));
        slider(ui, &mut s.transform.offset_x, SliderSpec::new("Offset X", -100.0, 100.0, 0.0).dec(1));
        slider(ui, &mut s.transform.offset_y, SliderSpec::new("Offset Y", -100.0, 100.0, 0.0).dec(1));
        let was = s.transform.constrain_crop;
        ui.checkbox(&mut s.transform.constrain_crop, "Constrain crop");
        if s.transform.constrain_crop && !was {
            // start from the full frame so the crop can grow back
            crate::tool_panels::reset_crop(s);
        }
        if s.transform.constrain_crop {
            engine.constrain_crop(s);
        }
    });
    s.panels.geometry = geo_on;

    // ------------------------------------------------------------- Effects
    let mut fx_on = s.panels.effects;
    section(ui, "Effects", Some(&mut fx_on), false, |ui| {
        ui.label(RichText::new("Vignette").strong());
        egui::ComboBox::from_id_salt("vigstyle")
            .selected_text(match s.vignette.style {
                VignetteStyle::HighlightPriority => "Highlight priority",
                VignetteStyle::ColorPriority => "Color priority",
                VignetteStyle::PaintOverlay => "Paint overlay",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut s.vignette.style, VignetteStyle::HighlightPriority, "Highlight priority");
                ui.selectable_value(&mut s.vignette.style, VignetteStyle::ColorPriority, "Color priority");
                ui.selectable_value(&mut s.vignette.style, VignetteStyle::PaintOverlay, "Paint overlay");
            });
        let gv = widgets::grad_bw();
        slider(ui, &mut s.vignette.amount, SliderSpec::new("Vignette", -100.0, 100.0, 0.0).grad(&gv));
        slider(ui, &mut s.vignette.midpoint, SliderSpec::new("Midpoint", 0.0, 100.0, 50.0));
        slider(ui, &mut s.vignette.roundness, SliderSpec::new("Roundness", -100.0, 100.0, 0.0));
        slider(ui, &mut s.vignette.feather, SliderSpec::new("Feather", 0.0, 100.0, 50.0));
        slider(ui, &mut s.vignette.highlights, SliderSpec::new("Highlights", 0.0, 100.0, 0.0));
        ui.label(RichText::new("Grain").strong());
        slider(ui, &mut s.grain.amount, SliderSpec::new("Grain", 0.0, 100.0, 0.0));
        slider(ui, &mut s.grain.size, SliderSpec::new("Size", 0.0, 100.0, 25.0));
        slider(ui, &mut s.grain.roughness, SliderSpec::new("Roughness", 0.0, 100.0, 50.0));
    });
    s.panels.effects = fx_on;

    // ------------------------------------------------------------- Calibration
    let mut cal_on = s.panels.calibration;
    section(ui, "Calibration", Some(&mut cal_on), false, |ui| {
        ui.label(RichText::new(format!("Process: Version 6 ({})", s.process_version)).weak());
        let gti = widgets::grad_tint();
        slider(ui, &mut s.calibration.shadow_tint, SliderSpec::new("Shadows tint", -100.0, 100.0, 0.0).grad(&gti));
        for (name, hue, h, sat) in [
            ("Red primary", 0.0, &mut s.calibration.red_hue, &mut s.calibration.red_sat),
            ("Green primary", 120.0, &mut s.calibration.green_hue, &mut s.calibration.green_sat),
            ("Blue primary", 240.0, &mut s.calibration.blue_hue, &mut s.calibration.blue_sat),
        ] {
            ui.label(RichText::new(name).strong());
            let gh = widgets::grad_band_hue(hue);
            let gs = widgets::grad_band_sat(hue);
            slider(ui, h, SliderSpec::new("Hue", -100.0, 100.0, 0.0).grad(&gh));
            slider(ui, sat, SliderSpec::new("Saturation", -100.0, 100.0, 0.0).grad(&gs));
        }
    });
    s.panels.calibration = cal_on;
}

/// DCP files found on disk, parsed once.
fn dcp_catalog() -> &'static Vec<darkslide_core::dcp::Dcp> {
    static CAT: std::sync::OnceLock<Vec<darkslide_core::dcp::Dcp>> = std::sync::OnceLock::new();
    CAT.get_or_init(|| darkslide_core::dcp::list_dcps().iter().filter_map(|p| darkslide_core::dcp::Dcp::load(p).ok()).collect())
}

fn dcp_name(path: &str) -> String {
    dcp_catalog()
        .iter()
        .find(|d| d.path.to_string_lossy() == path)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())
}

pub fn point_color_ui(ui: &mut egui::Ui, pcs: &mut Vec<PointColor>, sel: &mut usize, pick: &mut Pick) {
    point_color_ui_ext(ui, pcs, sel, pick, None)
}

pub fn point_color_ui_ext(ui: &mut egui::Ui, pcs: &mut Vec<PointColor>, sel: &mut usize, pick: &mut Pick, visualize: Option<&mut bool>) {
    ui.horizontal(|ui| {
        let active = *pick == Pick::PointColor;
        if widgets::eyedropper_button(ui, active, "Click in the image to sample a color") {
            *pick = if active { Pick::None } else { Pick::PointColor };
        }
        ui.label(RichText::new("Pick color").size(12.0));
        ui.label(RichText::new(format!("{}/8", pcs.len())).weak());
    });
    if pcs.is_empty() {
        ui.label(RichText::new("Sample a color to adjust it precisely.").weak().small());
        return;
    }
    ui.horizontal_wrapped(|ui| {
        let ok = darkslide_core::color::Oklab::new();
        for (i, p) in pcs.iter().enumerate() {
            let pp = ok.to_prophoto(darkslide_core::color::lch_to_lab(p.sample));
            let s = darkslide_core::color::mul_mv(&darkslide_core::color::prophoto_to(darkslide_core::color::ColorSpace::SRgb), pp);
            let c = s.map(|v| (darkslide_core::color::srgb_encode(v.clamp(0.0, 1.0)) * 255.0) as u8);
            let (rect, r) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(c[0], c[1], c[2]));
            if *sel == i {
                ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Outside);
            }
            if r.clicked() {
                *sel = i;
            }
        }
    });
    *sel = (*sel).min(pcs.len() - 1);
    let mut delete = false;
    {
        let p = &mut pcs[*sel];
        let h = p.sample[2];
        let ghue = widgets::grad_band_hue(h);
        let gsat = widgets::grad_band_sat(h);
        let glum = widgets::grad_band_lum(h);
        slider(ui, &mut p.hue_shift, SliderSpec::new("Hue shift", -100.0, 100.0, 0.0).grad(&ghue));
        slider(ui, &mut p.sat_shift, SliderSpec::new("Sat shift", -100.0, 100.0, 0.0).grad(&gsat));
        slider(ui, &mut p.lum_shift, SliderSpec::new("Lum shift", -100.0, 100.0, 0.0).grad(&glum));
        slider(ui, &mut p.variance, SliderSpec::new("Variance", -100.0, 100.0, 0.0));
        slider(ui, &mut p.range, SliderSpec::new("Range", 0.0, 100.0, 50.0));
        ui.collapsing("Fine ranges", |ui| {
            slider(ui, &mut p.hue_range, SliderSpec::new("Hue range", 0.0, 100.0, 50.0));
            slider(ui, &mut p.sat_range, SliderSpec::new("Sat range", 0.0, 100.0, 50.0));
            slider(ui, &mut p.lum_range, SliderSpec::new("Lum range", 0.0, 100.0, 50.0));
        });
        ui.horizontal(|ui| {
            if let Some(v) = visualize {
                ui.checkbox(v, "Visualize range");
            }
            if ui.button("🗑 Delete swatch").clicked() {
                delete = true;
            }
        });
    }
    if delete {
        pcs.remove(*sel);
        *sel = sel.saturating_sub(1);
    }
}

pub fn grading_ui(ui: &mut egui::Ui, g: &mut ColorGrading, view: &mut u8, id: &str) {
    ui.horizontal(|ui| {
        for (i, t) in ["◔◑◕", "Shadows", "Midtones", "Highlights", "Global"].iter().enumerate() {
            ui.selectable_value(view, i as u8, *t);
        }
    });
    let w = ui.available_width();
    let wheel = |ui: &mut egui::Ui, wh: &mut Wheel, title: &str, size: f32| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).small());
            widgets::color_wheel(ui, &mut wh.hue, &mut wh.sat, size, ui.id().with((id, title)));
            ui.add(egui::DragValue::new(&mut wh.hue).range(0.0..=359.0).prefix("H ").speed(1.0));
            ui.add(egui::DragValue::new(&mut wh.sat).range(0.0..=100.0).prefix("S ").speed(0.5));
            let gl = widgets::grad_bw();
            ui.set_max_width(size + 10.0);
            slider(ui, &mut wh.lum, SliderSpec::new("Lum", -100.0, 100.0, 0.0).grad(&gl));
        });
    };
    match *view {
        0 => {
            let sz = ((w - 20.0) / 2.0).min(110.0);
            ui.vertical_centered(|ui| wheel(ui, &mut g.midtones, "Midtones", sz));
            ui.horizontal(|ui| {
                wheel(ui, &mut g.shadows, "Shadows", sz);
                wheel(ui, &mut g.highlights, "Highlights", sz);
            });
        }
        1 => wheel(ui, &mut g.shadows, "Shadows", (w - 20.0).min(220.0)),
        2 => wheel(ui, &mut g.midtones, "Midtones", (w - 20.0).min(220.0)),
        3 => wheel(ui, &mut g.highlights, "Highlights", (w - 20.0).min(220.0)),
        _ => wheel(ui, &mut g.global, "Global", (w - 20.0).min(220.0)),
    }
    slider(ui, &mut g.blending, SliderSpec::new("Blending", 0.0, 100.0, 50.0));
    slider(ui, &mut g.balance, SliderSpec::new("Balance", -100.0, 100.0, 0.0));
}
