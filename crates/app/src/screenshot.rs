//! Headless UI screenshots for development (`--features screenshot`):
//! `darkslide --screenshot out.png [--tool crop] [--set Exposure2012=1] [--mask sky] FILE...`

use egui_phosphor::regular as ic;
use std::collections::HashMap;
use std::path::PathBuf;

use darkslide_core::xmp::{self, CrsValue};

use crate::app::App;
use crate::tool_panels::Tool;
use eframe::egui;

/// Click through every tool and many buttons to catch panics:
/// `darkslide --smoke FILE...`
pub fn smoke(args: Vec<String>) -> anyhow::Result<()> {
    use egui_kittest::kittest::Queryable;
    let files: Vec<PathBuf> = args.into_iter().skip(1).map(PathBuf::from).collect();
    let mut harness = egui_kittest::Harness::builder().with_size([1500.0, 950.0]).wgpu().build_eframe(|cc| App::new(cc, files));
    harness.state_mut().prefs.autosave_sidecars = false;
    let settle = |h: &mut egui_kittest::Harness<'_, App>, n: usize| {
        for _ in 0..n {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    };
    for _ in 0..400 {
        harness.step();
        if harness.state().doc().map(|d| d.engine.is_some()).unwrap_or(true) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    settle(&mut harness, 20);
    let plan: &[(Tool, &[&str])] = &[
        (
            Tool::Edit,
            &[
                "Auto",
                "B&W",
                "B&W",
                "Light",
                "Color",
                "Curve",
                "Detail",
                "Color Mixer",
                "Point Color",
                "Color Grading",
                "Optics",
                "Geometry",
                "Effects",
                "Calibration",
                "Level",
                "Vertical",
                "Full",
            ],
        ),
        (Tool::Crop, &["Auto", "90°", ic::FLIP_HORIZONTAL, "Reset crop", "Straighten tool"]),
        (Tool::Remove, &["Remove", "Clone", "Heal", "Visualize spots", "Dust spots", "Clear all"]),
        (Tool::Masking, &["Sky", "Linear", "Radial", "Luminance range", "Color range", "Background", "Brush"]),
        (Tool::RedEye, &["Pet eye", "Red eye", "Clear all"]),
        (Tool::Presets, &["Create preset"]),
        (Tool::Snapshots, &["Create"]),
    ];
    let mut clicked = 0;
    for (tool, labels) in plan {
        harness.state_mut().tools.tool = *tool;
        settle(&mut harness, 10);
        for l in labels.iter() {
            let found = harness.query_all_by_label_contains(l).next().map(|n| n.click()).is_some();
            if found {
                clicked += 1;
            } else {
                println!("  (not found: {l} in {tool:?})");
            }
            settle(&mut harness, 8);
        }
    }
    // pointer drags in the viewer for each on-image tool
    let drag = |h: &mut egui_kittest::Harness<'_, App>, from: egui::Pos2, to: egui::Pos2| {
        h.event(egui::Event::PointerMoved(from));
        h.step();
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        for k in 1..=8 {
            let p = from + (to - from) * (k as f32 / 8.0);
            h.event(egui::Event::PointerMoved(p));
            h.step();
        }
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(30));
    };
    let view = harness.state().view_rect;
    let c = view.center();
    // crop: drag the top-left corner inwards, then rotate from outside
    harness.state_mut().tools.tool = Tool::Crop;
    settle(&mut harness, 15);
    harness.state_mut().tools.straighten = false;
    harness.state_mut().pick = crate::edit_panel::Pick::None;
    settle(&mut harness, 3);
    // cursor hints: walk outwards from the centre, record distinct cursors
    for (name, dir) in [("diagonal", egui::vec2(-1.0, -0.66)), ("left", egui::vec2(-1.0, 0.0))] {
        let mut seen: Vec<egui::CursorIcon> = vec![];
        for k in 0..60 {
            harness.hover_at(c + dir * (k as f32 * 12.0));
            harness.step();
            let icon = harness.output().platform_output.cursor_icon;
            if seen.last() != Some(&icon) {
                seen.push(icon);
            }
        }
        println!("crop cursors {name}: {seen:?}");
    }
    // turned crop: edge drag resizes without touching the angle; dragging
    // clockwise outside turns the photo clockwise (angle goes down)
    if let Some(d) = harness.state_mut().doc_mut() {
        d.settings.crop = darkslide_core::params::Crop { angle: 8.0, left: 0.1, top: 0.12, right: 0.9, bottom: 0.88, ..Default::default() };
    }
    settle(&mut harness, 10);
    {
        let before = harness.state().doc().unwrap().settings.crop;
        // left edge midpoint: walk left from the centre until the cursor says resize
        let mut edge = None;
        for k in 0..80 {
            let p = c - egui::vec2(k as f32 * 6.0, 0.0);
            harness.hover_at(p);
            harness.step();
            if harness.output().platform_output.cursor_icon == egui::CursorIcon::ResizeHorizontal {
                edge = Some(p);
                break;
            }
        }
        if let Some(e) = edge {
            drag(&mut harness, e, e + egui::vec2(40.0, 0.0));
        }
        let mid = harness.state().doc().unwrap().settings.crop;
        drag(&mut harness, c + egui::vec2(0.0, -view.height() * 0.47), c + egui::vec2(view.width() * 0.2, -view.height() * 0.45));
        let after = harness.state().doc().unwrap().settings.crop;
        println!(
            "turned crop: edge found={} left {:.3}->{:.3} angle {:.2}->{:.2}; rotate drag angle {:.2}->{:.2}",
            edge.is_some(),
            before.left,
            mid.left,
            before.angle,
            mid.angle,
            mid.angle,
            after.angle
        );
    }
    if let Some(d) = harness.state_mut().doc_mut() {
        d.settings.crop = Default::default();
    }
    settle(&mut harness, 5);
    let img_tl = c - egui::vec2(view.width() * 0.45, view.height() * 0.3);
    drag(&mut harness, img_tl, img_tl + egui::vec2(80.0, 60.0));
    drag(&mut harness, c, c + egui::vec2(40.0, 20.0));
    // masking: brush stroke, linear gradient, radial
    harness.state_mut().tools.tool = Tool::Masking;
    for mt in [
        crate::tool_panels::MaskTool::Brush,
        crate::tool_panels::MaskTool::Linear,
        crate::tool_panels::MaskTool::Radial,
        crate::tool_panels::MaskTool::ObjectBox,
    ] {
        harness.state_mut().tools.mask_tool = mt;
        settle(&mut harness, 5);
        drag(&mut harness, c - egui::vec2(100.0, 50.0), c + egui::vec2(120.0, 40.0));
        settle(&mut harness, 5);
    }
    // remove: a brush spot
    harness.state_mut().tools.tool = Tool::Remove;
    settle(&mut harness, 5);
    drag(&mut harness, c, c + egui::vec2(30.0, 10.0));
    // red eye box
    harness.state_mut().tools.tool = Tool::RedEye;
    settle(&mut harness, 5);
    drag(&mut harness, c, c + egui::vec2(20.0, 20.0));
    println!("after red-eye drag: kind={:?} redeye={}", harness.state().drag.kind, harness.state().doc().unwrap().settings.red_eye.len());
    // targeted adjustment and white balance eyedropper
    harness.state_mut().tools.tool = Tool::Edit;
    harness.state_mut().pick = crate::edit_panel::Pick::TatCurve;
    settle(&mut harness, 5);
    drag(&mut harness, c, c - egui::vec2(0.0, 40.0));
    harness.state_mut().pick = crate::edit_panel::Pick::WhiteBalance;
    drag(&mut harness, c, c);
    settle(&mut harness, 40);
    {
        let st = harness.state();
        let d = st.doc().unwrap();
        println!(
            "after drags: crop=({:.2},{:.2},{:.2},{:.2}) angle={:.1} masks={} spots={} redeye={} wb={:?}",
            d.settings.crop.left,
            d.settings.crop.top,
            d.settings.crop.right,
            d.settings.crop.bottom,
            d.settings.crop.angle,
            d.settings.masks.len(),
            d.settings.retouch.len(),
            d.settings.red_eye.len(),
            d.settings.wb_mode
        );
    }
    // reset-all asks first: cancel keeps edits, confirm resets, undo restores
    {
        use egui_kittest::kittest::Queryable;
        harness.state_mut().tools.tool = Tool::Edit;
        if let Some(d) = harness.state_mut().doc_mut() {
            d.settings.exposure = 1.0;
            d.settings.clarity = 30.0;
        }
        settle(&mut harness, 5);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
        settle(&mut harness, 5);
        let shown = harness.state().confirm_reset;
        if let Some(n) = harness.query_by_label("Cancel") {
            n.click();
        }
        settle(&mut harness, 5);
        let kept = harness.state().doc().map(|d| d.settings.exposure == 1.0).unwrap_or(false);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
        settle(&mut harness, 5);
        if let Some(n) = harness.query_by_label("Reset all") {
            n.click();
        }
        settle(&mut harness, 5);
        let reset = harness.state().doc().map(|d| d.settings.exposure == 0.0 && d.settings.clarity == 0.0).unwrap_or(false);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        settle(&mut harness, 5);
        let undone = harness.state().doc().map(|d| d.settings.exposure == 1.0).unwrap_or(false);
        println!("reset-all: dialog={shown} cancel-keeps={kept} reset={reset} undo={undone}");
    }
    // keyboard shortcuts
    for k in [egui::Key::U, egui::Key::O, egui::Key::Q, egui::Key::Q, egui::Key::Z, egui::Key::Z, egui::Key::ArrowRight, egui::Key::ArrowLeft] {
        harness.key_press(k);
        settle(&mut harness, 6);
    }
    // undo everything
    for _ in 0..30 {
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.step();
    }
    settle(&mut harness, 60);
    println!(
        "smoke ok: clicked {clicked} controls; masks={} spots={}",
        harness.state().doc().map(|d| d.settings.masks.len()).unwrap_or(0),
        harness.state().doc().map(|d| d.settings.retouch.len()).unwrap_or(0)
    );
    Ok(())
}

pub fn run(args: Vec<String>) -> anyhow::Result<()> {
    let mut out = PathBuf::from("screenshot.png");
    let mut files = vec![];
    let mut tool = Tool::Edit;
    let mut sets: HashMap<String, CrsValue> = HashMap::new();
    let mut json: Option<String> = None;
    let mut size = [1500.0f32, 950.0];
    let mut browse_profiles = false;
    let mut zoom: Option<f32> = None;
    let mut center: Option<[f32; 2]> = None;
    let mut ba: Option<u8> = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--screenshot" => out = it.next().unwrap_or_default().into(),
            "--tool" => {
                tool = match it.next().unwrap_or_default().as_str() {
                    "crop" => Tool::Crop,
                    "remove" => Tool::Remove,
                    "mask" | "masking" => Tool::Masking,
                    "presets" => Tool::Presets,
                    "snapshots" => Tool::Snapshots,
                    "redeye" => Tool::RedEye,
                    _ => Tool::Edit,
                }
            }
            "--set" => {
                if let Some((k, v)) = it.next().unwrap_or_default().split_once('=') {
                    sets.insert(k.to_string(), CrsValue::Text(v.to_string()));
                }
            }
            "--json" => json = it.next(),
            "--profiles" => browse_profiles = true,
            "--zoom" => zoom = it.next().and_then(|v| v.parse().ok()),
            "--center" => center = it.next().and_then(|v| v.split_once(',').map(|(a, b)| [a.parse().unwrap_or(0.5), b.parse().unwrap_or(0.5)])),
            "--ba" => ba = it.next().and_then(|v| v.parse().ok()),
            "--size" => {
                let v = it.next().unwrap_or_default();
                if let Some((w, h)) = v.split_once('x') {
                    size = [w.parse().unwrap_or(1500.0), h.parse().unwrap_or(950.0)];
                }
            }
            _ => files.push(PathBuf::from(a)),
        }
    }
    let mut harness = egui_kittest::Harness::builder().with_size(size).wgpu().build_eframe(|cc| App::new(cc, files));
    // wait for the image to load
    for _ in 0..400 {
        harness.step();
        if harness.state().doc().map(|d| d.engine.is_some()).unwrap_or(true) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    {
        let app = harness.state_mut();
        app.tools.tool = tool;
        if let Some(d) = app.doc_mut() {
            if let Some(j) = &json {
                if let Ok(s) = serde_json::from_str(&std::fs::read_to_string(j)?) {
                    d.settings = s;
                }
            }
            xmp::apply_crs(&sets, &mut d.settings);
        }
        app.prefs.autosave_sidecars = false;
        if let Some(z) = zoom {
            app.zoom = crate::app::Zoom::Scale(z);
        }
        if let Some(c) = center {
            app.center = c;
        }
        for _ in 0..ba.unwrap_or(0) {
            app.ba = app.ba.next();
        }
        if browse_profiles {
            if let Some(d) = app.doc() {
                let (e, st, id) = (d.engine.clone().unwrap(), d.settings.clone(), d.id);
                app.profiles.open(&e, &st, id);
            }
        }
        if tool == Tool::Masking {
            app.tools.selected_mask = Some(0);
            app.tools.selected_comp = Some(0);
        }
    }
    // let renders settle: wait until the worker has nothing queued for a while
    let mut idle = 0;
    for _ in 0..600 {
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(25));
        let st = harness.state();
        let pending_ai = st.busy.lock().unwrap().is_some()
            || st
                .doc()
                .map(|d| {
                    d.settings.masks.iter().any(|m| m.components.iter().any(|c| matches!(c.shape, darkslide_core::params::MaskShape::Ai { raster: None, .. })))
                })
                .unwrap_or(false);
        let thumbs_pending = (st.profiles.open && st.profiles.items.lock().unwrap().iter().any(|i| i.3.is_none())) || st.tools.preset_ui.thumbs_pending();
        if st.fit_tex.is_some() && !pending_ai && !thumbs_pending {
            idle += 1;
            if idle > 40 {
                break;
            }
        }
    }
    let img = harness.render().map_err(|e| anyhow::anyhow!(e))?;
    img.save(&out)?;
    println!("wrote {}", out.display());
    Ok(())
}
