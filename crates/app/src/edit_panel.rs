//! The Edit panel: Profile, Basic, Curve, Detail, Color Mixer, Point Color,
//! Color Grading, Optics, Geometry, Effects, Calibration.

use darkslide_core::analysis::Histogram;
use darkslide_core::params::*;
use darkslide_core::pipeline::curves::{Lut, point_curve_lut};
use darkslide_core::profiles::{self, BUILTIN_LOOKS, PROFILES};
use eframe::egui::{self, Color32, RichText};
use egui_phosphor::regular as ic;

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

impl Pick {
    pub fn label(self) -> &'static str {
        match self {
            Pick::None => "",
            Pick::WhiteBalance => "White balance picker",
            Pick::PointColor => "Point color picker",
            Pick::Sampler => "Color sampler",
            Pick::TatCurve => "Targeted curve adjustment",
            Pick::TatMixer(_) => "Targeted color mixer adjustment",
            Pick::Upright => "Guided Upright",
        }
    }
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
    DownloadLensDb,
    /// Ask before resetting every setting to the defaults.
    ResetAll,
}

pub fn edit_panel(ui: &mut egui::Ui, doc: &mut Doc, st: &mut EditUi, pc: &mut PanelCtx) {
    let Some(engine) = doc.engine.clone() else {
        ui.label("Loading…");
        return;
    };
    let is_raw = engine.is_raw();
    let s = &mut doc.settings;

    // ------------------------------------------------------------- Edit header
    // ACR: "Edit" with Auto / B&W, then the profile row (not collapsible)
    let mut profile_block = |ui: &mut egui::Ui| {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Edit").size(15.0).strong().color(Color32::WHITE));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let bw = s.treatment == Treatment::BlackAndWhite;
                if ui.add(egui::Button::new("B&W").selected(bw)).on_hover_text("Convert to black & white").clicked() {
                    if bw {
                        s.treatment = Treatment::Color;
                        // a monochrome profile would keep the image grey
                        if profiles::find(&s.profile).monochrome {
                            s.profile = "Color".into();
                        }
                    } else {
                        s.treatment = Treatment::BlackAndWhite;
                    }
                }
                if ui.button("Reset").on_hover_text("Reset all settings to the defaults (Ctrl+R)").clicked() {
                    pc.actions.push(Action::ResetAll);
                }
                if ui.button("Auto").on_hover_text("Auto tone (Ctrl+U)").clicked() {
                    pc.actions.push(Action::AutoTone);
                }
            });
        });
        ui.add_space(6.0);
        let shown = s.profile.strip_prefix("dcp:").map(|p| dcp_name(p)).unwrap_or_else(|| s.profile.clone());
        ui.horizontal(|ui| {
            // same label width as Look, so the two combos line up
            ui.add_sized([40.0, 18.0], egui::Label::new(RichText::new("Profile").size(12.0)));
            let full_w = ui.available_width();
            egui::ComboBox::from_id_salt("profile").width((full_w - 36.0).max(60.0)).height(520.0).selected_text(shown).show_ui(ui, |ui| {
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
            if ui.button(ic::SQUARES_FOUR).on_hover_text("Browse profiles and looks").clicked() {
                pc.actions.push(Action::BrowseProfiles);
            }
        });
        let prof = profiles::find(&s.profile);
        if prof.adaptive || prof.saturation != 1.0 || prof.contrast != 1.0 {
            slider(ui, &mut s.profile_amount, SliderSpec::new("Amount", 0.0, 200.0, 100.0));
        }
        ui.add_space(4.0);
        let look_name = s.look.as_ref().map(|l| l.name.clone()).unwrap_or_else(|| "None".into());
        ui.horizontal(|ui| {
            ui.add_sized([40.0, 18.0], egui::Label::new(RichText::new("Look").size(12.0)));
            egui::ComboBox::from_id_salt("look").width((ui.available_width() - 36.0).max(60.0)).height(520.0).selected_text(look_name).show_ui(ui, |ui| {
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
            if ui.button(ic::PLUS).on_hover_text("Import a .cube LUT as a look").clicked() {
                pc.actions.push(Action::ImportLook);
            }
        });
        if let Some(l) = s.look.as_mut() {
            slider(ui, &mut l.amount, SliderSpec::new("Look amount", 0.0, 200.0, 100.0));
        }
        ui.add_space(6.0);
        ui.separator();
    };
    profile_block(ui);

    // ------------------------------------------------------------- Light
    let mut light_on = s.panels.basic;
    section(ui, "Light", Some(&mut light_on), true, |ui| {
        let max_ev = if s.hdr.enabled { 10.0 } else { 5.0 };
        use widgets::AltPreview::{Highlights as Hi, Shadows as Sh};
        slider(ui, &mut s.exposure, SliderSpec::new("Exposure", -max_ev, max_ev, 0.0).dec(2).auto().alt(Hi));
        slider(ui, &mut s.contrast, SliderSpec::new("Contrast", -100.0, 100.0, 0.0).auto());
        slider(ui, &mut s.highlights, SliderSpec::new("Highlights", -100.0, 100.0, 0.0).auto().alt(Hi));
        slider(ui, &mut s.shadows, SliderSpec::new("Shadows", -100.0, 100.0, 0.0).auto().alt(Sh));
        slider(ui, &mut s.whites, SliderSpec::new("Whites", -100.0, 100.0, 0.0).auto().alt(Hi));
        slider(ui, &mut s.blacks, SliderSpec::new("Blacks", -100.0, 100.0, 0.0).auto().alt(Sh));

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.hdr.enabled, "HDR").on_hover_text("Edit in high dynamic range (extended highlights)");
            if s.hdr.enabled {
                ui.checkbox(&mut s.hdr.visualize, "Visualize HDR");
            }
        });
        if s.hdr.enabled {
            slider(ui, &mut s.hdr.max_headroom, SliderSpec::new("HDR headroom", 1.0, 4.0, 4.0).dec(1).suffix(" stops"));
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
    s.panels.basic = light_on;

    // ------------------------------------------------------------- Color
    let mut color_on = s.panels.color;
    section(ui, "Color", Some(&mut color_on), false, |ui| {
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
            slider(ui, &mut t, SliderSpec::new("Temperature", 1500.0, 50000.0, def_t).grad(&gt).log().auto())
        } else {
            slider(ui, &mut t, SliderSpec::new("Temperature", -100.0, 100.0, 0.0).grad(&gt).auto())
        };
        let tichg = slider(ui, &mut ti, SliderSpec::new("Tint", -150.0, 150.0, def_ti).grad(&gti).auto());
        if tchg || tichg {
            s.wb_mode = WbMode::Custom;
            s.temperature = t;
            s.tint = ti;
        }

        slider(ui, &mut s.vibrance, SliderSpec::new("Vibrance", -100.0, 100.0, 0.0).auto());
        slider(ui, &mut s.saturation, SliderSpec::new("Saturation", -100.0, 100.0, 0.0));
    });
    s.panels.color = color_on;

    // ------------------------------------------------------------- Effects
    let mut fx_on = s.panels.effects;
    section(ui, "Effects", Some(&mut fx_on), false, |ui| {
        slider(ui, &mut s.texture, SliderSpec::new("Texture", -100.0, 100.0, 0.0));
        slider(ui, &mut s.clarity, SliderSpec::new("Clarity", -100.0, 100.0, 0.0));
        slider(ui, &mut s.dehaze, SliderSpec::new("Dehaze", -100.0, 100.0, 0.0));
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
        slider(ui, &mut s.vignette.amount, SliderSpec::new("Amount", -100.0, 100.0, 0.0).grad(&gv));
        slider(ui, &mut s.vignette.midpoint, SliderSpec::new("Midpoint", 0.0, 100.0, 50.0));
        slider(ui, &mut s.vignette.roundness, SliderSpec::new("Roundness", -100.0, 100.0, 0.0));
        slider(ui, &mut s.vignette.feather, SliderSpec::new("Feather", 0.0, 100.0, 50.0));
        slider(ui, &mut s.vignette.highlights, SliderSpec::new("Highlights", 0.0, 100.0, 0.0));
        ui.label(RichText::new("Grain").strong());
        slider(ui, &mut s.grain.amount, SliderSpec::new("Amount", 0.0, 100.0, 0.0));
        slider(ui, &mut s.grain.size, SliderSpec::new("Size", 0.0, 100.0, 25.0));
        slider(ui, &mut s.grain.roughness, SliderSpec::new("Roughness", 0.0, 100.0, 50.0));
    });
    s.panels.effects = fx_on;

    // ------------------------------------------------------------- Curve
    let mut curve_on = s.panels.curve;
    section(ui, "Curve", Some(&mut curve_on), false, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for (m, lab, col, tip) in [
                (CurveMode::Parametric, ic::CHART_LINE, Color32::from_gray(210), "Parametric curve"),
                (CurveMode::Rgb, "RGB", Color32::WHITE, "Point curve, all channels"),
                (CurveMode::Red, "R", Color32::from_rgb(235, 90, 90), "Red channel"),
                (CurveMode::Green, "G", Color32::from_rgb(90, 205, 90), "Green channel"),
                (CurveMode::Blue, "B", Color32::from_rgb(100, 140, 245), "Blue channel"),
            ] {
                let txt = RichText::new(lab).color(col).strong();
                if ui.add(egui::Button::selectable(st.curve_mode == m, txt).min_size(egui::vec2(28.0, 22.0))).on_hover_text(tip).clicked() {
                    st.curve_mode = m;
                }
            }
            let active = *pc.pick == Pick::TatCurve;
            if ui.selectable_label(active, ic::CROSSHAIR_SIMPLE).on_hover_text("Targeted adjustment: drag up/down in the image").clicked() {
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
                    widgets::CurveEdit { points: &mut pts, color: Color32::from_gray(220), hist: lum_hist, id: ui.id().with("pcurve"), read_only: true },
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
                widgets::curve_editor(ui, widgets::CurveEdit { points: pts, color: col, hist, id: ui.id().with(("curve", mode as u8)), read_only: false }, &others_ref);
                ui.horizontal(|ui| {
                    if ui.small_button("Reset channel").clicked() {
                        *pts = linear_curve();
                    }
                    ui.label(RichText::new("Click to add · drag out or double-click to remove").weak().small());
                });
            }
        }
        slider(ui, &mut s.refine_saturation, SliderSpec::new("Refine saturation", 0.0, 100.0, 100.0));
    });
    s.panels.curve = curve_on;

    // ------------------------------------------------------------- Color Mixer
    let mut mixer_on = s.panels.mixer;
    let title = if s.is_bw() { "B&W Mixer" } else { "Color Mixer" };
    widgets::section_id(ui, "Color Mixer", title, Some(&mut mixer_on), false, |ui| {
        if s.is_bw() {
            ui.horizontal(|ui| {
                if ui.add(egui::Button::selectable(s.auto_gray_mix, "Auto")).on_hover_text("Mix from the photo's colors").clicked() {
                    pc.actions.push(Action::AutoGrayMix);
                }
                if ui.button("Default").clicked() {
                    s.gray_mix = [0.0; 8];
                }
                let active = *pc.pick == Pick::TatMixer(3);
                if ui.selectable_label(active, ic::CROSSHAIR_SIMPLE).on_hover_text("Targeted adjustment").clicked() {
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
                if ui.selectable_label(active, ic::CROSSHAIR_SIMPLE).on_hover_text("Targeted adjustment: drag in the image").clicked() {
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

    // ------------------------------------------------------------- Detail
    let mut detail_on = s.panels.detail;
    section(ui, "Detail", Some(&mut detail_on), false, |ui| {
        ui.label(RichText::new("Sharpening").strong());
        slider(ui, &mut s.sharpen.amount, SliderSpec::new("Amount", 0.0, 150.0, if is_raw { 40.0 } else { 0.0 }));
        slider(ui, &mut s.sharpen.radius, SliderSpec::new("Radius", 0.5, 3.0, 1.0).dec(1));
        slider(ui, &mut s.sharpen.detail, SliderSpec::new("Detail", 0.0, 100.0, 25.0));
        slider(ui, &mut s.sharpen.masking, SliderSpec::new("Masking", 0.0, 100.0, 0.0).alt(widgets::AltPreview::SharpenMask));
        ui.label(RichText::new("Noise reduction").strong());
        slider(ui, &mut s.nr.luminance, SliderSpec::new("Luminance", 0.0, 100.0, 0.0));
        slider(ui, &mut s.nr.luminance_detail, SliderSpec::new("Detail", 0.0, 100.0, 50.0));
        slider(ui, &mut s.nr.luminance_contrast, SliderSpec::new("Contrast", 0.0, 100.0, 0.0));
        slider(ui, &mut s.nr.color, SliderSpec::new("Color", 0.0, 100.0, if is_raw { 25.0 } else { 0.0 }));
        slider(ui, &mut s.nr.color_detail, SliderSpec::new("Detail", 0.0, 100.0, 50.0));
        slider(ui, &mut s.nr.color_smoothness, SliderSpec::new("Smoothness", 0.0, 100.0, 50.0));
        ui.label(RichText::new("Enhance").strong());
        if ui.checkbox(&mut s.enhance.denoise, "Denoise").changed() && s.enhance.denoise && s.enhance.denoise_amount == 0.0 {
            // a zero amount would look like nothing happened
            s.enhance.denoise_amount = 50.0;
        }
        ui.add_enabled_ui(s.enhance.denoise, |ui| slider(ui, &mut s.enhance.denoise_amount, SliderSpec::new("Denoise amount", 0.0, 100.0, 50.0)));
        ui.checkbox(&mut s.enhance.raw_details, "Raw Details").on_hover_text("Higher quality demosaic for finer detail");
        ui.checkbox(&mut s.enhance.super_resolution, "Super Resolution (2×)").on_hover_text("Doubles linear resolution on export");
    });
    s.panels.detail = detail_on;

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
                    (None, true) => "No lens database installed".into(),
                    (None, false) => format!("No profile for {}", engine.source.meta.lens.clone().unwrap_or_default()),
                })
                .small(),
            );
            if db.is_empty() && ui.button("Download lens database").on_hover_text("Fetch lensfun's lens profiles (about 1 MB)").clicked() {
                pc.actions.push(Action::DownloadLensDb);
            }
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
        // a hue range needs low below high
        for r in [&mut s.lens.defringe_purple_hue, &mut s.lens.defringe_green_hue] {
            if r[0] > r[1] - 10.0 {
                r[1] = (r[0] + 10.0).min(100.0);
                r[0] = r[0].min(r[1] - 10.0);
            }
        }
        ui.label(RichText::new("Lens Blur").strong());
        ui.checkbox(&mut s.lens_blur.enabled, "Apply lens blur");
        if s.lens_blur.enabled {
            slider(ui, &mut s.lens_blur.amount, SliderSpec::new("Blur amount", 0.0, 100.0, 50.0));
            ui.horizontal(|ui| {
                ui.label("Bokeh");
                let name = |b: BokehShape| match b {
                    BokehShape::Circle => "Circle",
                    BokehShape::Bubble => "Bubble",
                    BokehShape::FiveBlade => "5-blade",
                    BokehShape::Ring => "Ring",
                    BokehShape::CatEye => "Cat eye",
                };
                egui::ComboBox::from_id_salt("bokeh").selected_text(name(s.lens_blur.shape)).show_ui(ui, |ui| {
                    for b in [BokehShape::Circle, BokehShape::Bubble, BokehShape::FiveBlade, BokehShape::Ring, BokehShape::CatEye] {
                        ui.selectable_value(&mut s.lens_blur.shape, b, name(b));
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
        if s.transform.constrain_crop && !was && s.crop.left == 0.0 && s.crop.top == 0.0 && s.crop.right == 1.0 && s.crop.bottom == 1.0 {
            // an untouched crop starts from the full frame; a user's crop is kept
            crate::tool_panels::reset_crop(s);
        }
        if s.transform.constrain_crop {
            engine.constrain_crop(s);
        }
    });
    s.panels.geometry = geo_on;

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
            if ui.button(format!("{}  Delete swatch", ic::TRASH)).clicked() {
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
    let w = ui.available_width();
    // one even row of tabs that always fits the panel
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let tw = ((w - 8.0) / 5.0).floor();
        let tabs = [("3-way", "Shadows, midtones and highlights"), ("Shadow", "Shadows"), ("Mid", "Midtones"), ("High", "Highlights"), ("Global", "Global")];
        for (i, (t, tip)) in tabs.iter().enumerate() {
            let b = egui::Button::selectable(*view == i as u8, RichText::new(*t).size(11.0)).truncate();
            if ui.add_sized([tw, 22.0], b).on_hover_text(*tip).clicked() {
                *view = i as u8;
            }
        }
    });
    let wheel = |ui: &mut egui::Ui, wh: &mut Wheel, title: &str, size: f32| {
        ui.vertical_centered(|ui| {
            ui.set_max_width(size + 10.0);
            ui.label(RichText::new(title).small());
            widgets::color_wheel(ui, &mut wh.hue, &mut wh.sat, size, ui.id().with((id, title)));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut wh.hue).range(0.0..=359.0).prefix("H ").speed(1.0).fixed_decimals(0));
                ui.add(egui::DragValue::new(&mut wh.sat).range(0.0..=100.0).prefix("S ").speed(0.5).fixed_decimals(0));
            });
            let gl = widgets::grad_bw();
            slider(ui, &mut wh.lum, SliderSpec::new("Luminance", -100.0, 100.0, 0.0).grad(&gl));
        });
    };
    match *view {
        0 => {
            // three wheels side by side, each a card with its own luminance track
            let gap = 6.0;
            let cw = ((w - 2.0 * gap) / 3.0).floor();
            let p = crate::theme::pal();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (title, wh) in [("SHADOWS", &mut g.shadows), ("MIDTONES", &mut g.midtones), ("HIGHLIGHTS", &mut g.highlights)] {
                    let (card, _) = ui.allocate_exact_size(egui::vec2(cw, cw + 40.0), egui::Sense::hover());
                    ui.painter().rect_filled(card, 6.0, p.well);
                    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink2(egui::vec2(4.0, 5.0))).layout(egui::Layout::top_down(egui::Align::Center)));
                    c.spacing_mut().item_spacing.y = 3.0;
                    c.label(RichText::new(title).size(9.5).color(p.text_muted));
                    let sz = cw - 14.0;
                    widgets::color_wheel(&mut c, &mut wh.hue, &mut wh.sat, sz, egui::Id::new((id, title)));
                    widgets::mini_lum(&mut c, &mut wh.lum, cw - 8.0);
                }
            });
            ui.label(RichText::new("Single wheels show exact values.").size(10.5).color(crate::theme::pal().text_muted));
        }
        1 => wheel(ui, &mut g.shadows, "Shadows", (w - 30.0).min(220.0)),
        2 => wheel(ui, &mut g.midtones, "Midtones", (w - 30.0).min(220.0)),
        3 => wheel(ui, &mut g.highlights, "Highlights", (w - 30.0).min(220.0)),
        _ => wheel(ui, &mut g.global, "Global", (w - 30.0).min(220.0)),
    }
    slider(ui, &mut g.blending, SliderSpec::new("Blending", 0.0, 100.0, 50.0));
    slider(ui, &mut g.balance, SliderSpec::new("Balance", -100.0, 100.0, 0.0));
}
