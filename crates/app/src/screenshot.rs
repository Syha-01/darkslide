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

/// Drive the UI from a step file and take screenshots along the way:
/// `darkslide --script steps.txt --out DIR [--size WxH] FILE...`
/// One command per line, `#` comments. Coordinates are logical points.
pub fn script(args: Vec<String>) -> anyhow::Result<()> {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut steps = PathBuf::new();
    let mut out = PathBuf::from(".");
    let mut size = [1500.0f32, 950.0];
    let mut files = vec![];
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--script" => steps = it.next().unwrap_or_default().into(),
            "--out" => out = it.next().unwrap_or_default().into(),
            "--size" => {
                if let Some((w, h)) = it.next().unwrap_or_default().split_once('x') {
                    size = [w.parse().unwrap_or(1500.0), h.parse().unwrap_or(950.0)];
                }
            }
            _ => files.push(PathBuf::from(a)),
        }
    }
    std::fs::create_dir_all(&out)?;
    let text = std::fs::read_to_string(&steps)?;
    let mut h = egui_kittest::Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).wgpu().build_eframe(|cc| App::new(cc, files));
    h.state_mut().prefs.autosave_sidecars = false;
    let settle = |h: &mut egui_kittest::Harness<'_, App>, n: usize| {
        for _ in 0..n {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    };
    // wait until loaded and the worker has gone quiet
    let idle = |h: &mut egui_kittest::Harness<'_, App>, max: usize| {
        let mut quiet = 0;
        for _ in 0..max {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(25));
            let st = h.state();
            let loading = st.doc().map(|d| d.engine.is_none()).unwrap_or(false);
            let busy = st.busy.lock().unwrap().is_some() || !st.ai_jobs.lock().unwrap().is_empty();
            let thumbs = st.tools.preset_ui.thumbs_pending() || (st.profiles.open && st.profiles.items.lock().unwrap().iter().any(|i| i.3.is_none()));
            if !loading && !busy && !thumbs && (st.doc().is_none() || st.fit_tex.is_some()) {
                quiet += 1;
                if quiet > 30 {
                    break;
                }
            } else {
                quiet = 0;
            }
        }
    };
    idle(&mut h, 800);
    let mods = |s: &str| -> (egui::Modifiers, String) {
        let mut m = egui::Modifiers::NONE;
        let parts: Vec<&str> = s.split('+').collect();
        for p in &parts[..parts.len() - 1] {
            match p.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" => m = m | egui::Modifiers::COMMAND,
                "shift" => m = m | egui::Modifiers::SHIFT,
                "alt" => m = m | egui::Modifiers::ALT,
                _ => {}
            }
        }
        (m, parts[parts.len() - 1].to_string())
    };
    let pt = |a: &[&str], i: usize| -> egui::Pos2 { egui::pos2(a.get(i).and_then(|v| v.parse().ok()).unwrap_or(0.0), a.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0.0)) };
    let mut pointer = egui::Pos2::ZERO;
    let mut held = egui::Modifiers::NONE;
    let button = |h: &egui_kittest::Harness<'_, App>, p: egui::Pos2, b: egui::PointerButton, down: bool, m: egui::Modifiers| {
        h.event(egui::Event::PointerButton { pos: p, button: b, pressed: down, modifiers: m });
    };
    // find a widget by label: exact match first, then contains; `#n` picks the nth match
    fn find_rect(h: &egui_kittest::Harness<'_, App>, q: &str) -> Option<egui::Rect> {
        // icon glyphs as printed by `labels`: \u{e2d8}
        let mut q = q.to_string();
        while let Some(i) = q.find("\\u{") {
            let Some(j) = q[i..].find('}') else { break };
            let c = u32::from_str_radix(&q[i + 3..i + j], 16).ok().and_then(char::from_u32).unwrap_or('?');
            q.replace_range(i..i + j + 1, &c.to_string());
        }
        let q = q.as_str();
        let (q, n) = match q.rsplit_once('#') {
            Some((a, b)) if b.parse::<usize>().is_ok() => (a, b.parse().unwrap()),
            _ => (q, 0),
        };
        let exact: Vec<_> = h.query_all_by_label(q).collect();
        if exact.len() > n {
            return Some(exact[n].rect());
        }
        h.query_all_by_label_contains(q).nth(n).map(|n| n.rect())
    }
    for (ln, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (cmd, rest) = line.split_once(char::is_whitespace).map(|(a, b)| (a, b.trim())).unwrap_or((line, ""));
        let a: Vec<&str> = rest.split_whitespace().collect();
        let mut ok = true;
        match cmd {
            "wait" => idle(&mut h, rest.parse().unwrap_or(400)),
            "step" => settle(&mut h, rest.parse().unwrap_or(5)),
            "size" => {
                h.set_size(egui::vec2(pt(&a, 0).x, pt(&a, 0).y));
                settle(&mut h, 10);
            }
            "tool" => {
                h.state_mut().tools.tool = match rest {
                    "crop" => Tool::Crop,
                    "remove" => Tool::Remove,
                    "mask" | "masking" => Tool::Masking,
                    "presets" => Tool::Presets,
                    "snapshots" => Tool::Snapshots,
                    "redeye" => Tool::RedEye,
                    _ => Tool::Edit,
                };
                settle(&mut h, 8);
            }
            "click" | "rclick" | "dblclick" | "hover" | "shiftclick" | "ctrlclick" | "altclick" => match find_rect(&h, rest) {
                Some(r) => {
                    let p = r.center();
                    pointer = p;
                    h.event(egui::Event::PointerMoved(p));
                    h.step();
                    let m = match cmd {
                        "shiftclick" => egui::Modifiers::SHIFT,
                        "ctrlclick" => egui::Modifiers::COMMAND,
                        "altclick" => egui::Modifiers::ALT,
                        _ => held,
                    };
                    if m != held {
                        h.event(egui::Event::ModifiersChanged(m));
                    }
                    let b = if cmd == "rclick" { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                    if cmd != "hover" {
                        for _ in 0..if cmd == "dblclick" { 2 } else { 1 } {
                            button(&h, p, b, true, m);
                            h.step();
                            button(&h, p, b, false, m);
                            h.step();
                        }
                    }
                    if m != held {
                        h.event(egui::Event::ModifiersChanged(held));
                    }
                    settle(&mut h, 6);
                }
                None => ok = false,
            },
            "move" | "hover_at" => {
                pointer = pt(&a, 0);
                h.event(egui::Event::PointerMoved(pointer));
                settle(&mut h, 3);
            }
            "down" | "up" | "rdown" | "rup" => {
                if a.len() >= 2 {
                    pointer = pt(&a, 0);
                    h.event(egui::Event::PointerMoved(pointer));
                    h.step();
                }
                let b = if cmd.starts_with('r') { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                button(&h, pointer, b, cmd.ends_with("down"), held);
                settle(&mut h, 2);
            }
            "click_at" | "rclick_at" | "dblclick_at" => {
                pointer = pt(&a, 0);
                h.event(egui::Event::PointerMoved(pointer));
                h.step();
                let b = if cmd == "rclick_at" { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                for _ in 0..if cmd == "dblclick_at" { 2 } else { 1 } {
                    button(&h, pointer, b, true, held);
                    h.step();
                    button(&h, pointer, b, false, held);
                    h.step();
                }
                settle(&mut h, 6);
            }
            // drag x1 y1 x2 y2 [steps]   |   drag_label LABEL dx dy
            "drag" | "rdrag" | "drag_label" => {
                let (from, to, n) = if cmd == "drag_label" {
                    let k = a.len().saturating_sub(2);
                    match find_rect(&h, &a[..k].join(" ")) {
                        Some(r) => (r.center(), r.center() + pt(&a, k).to_vec2(), 12),
                        None => {
                            ok = false;
                            (egui::Pos2::ZERO, egui::Pos2::ZERO, 0)
                        }
                    }
                } else {
                    (pt(&a, 0), pt(&a, 2), a.get(4).and_then(|v| v.parse().ok()).unwrap_or(12))
                };
                if ok {
                    let b = if cmd == "rdrag" { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                    h.event(egui::Event::PointerMoved(from));
                    h.step();
                    button(&h, from, b, true, held);
                    h.step();
                    for k in 1..=n {
                        h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / n as f32)));
                        h.step();
                    }
                    button(&h, to, b, false, held);
                    pointer = to;
                    settle(&mut h, 6);
                }
            }
            "scroll" => {
                // scroll x y dy [dx]  (positive dy scrolls content up / zooms in with ctrl)
                pointer = pt(&a, 0);
                h.event(egui::Event::PointerMoved(pointer));
                h.step();
                let dy: f32 = a.get(2).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                let dx: f32 = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(dx, dy), modifiers: held, phase: egui::TouchPhase::Move });
                settle(&mut h, 6);
            }
            "hold" => {
                held = mods(&format!("{rest}+x")).0;
                h.event(egui::Event::ModifiersChanged(held));
                settle(&mut h, 2);
            }
            "release" => {
                held = egui::Modifiers::NONE;
                h.event(egui::Event::ModifiersChanged(held));
                settle(&mut h, 2);
            }
            "key" | "keydown" | "keyup" => {
                let (m, k) = mods(rest);
                match egui::Key::from_name(&k) {
                    Some(key) => {
                        let m = m | held;
                        // go through the app's input hook like eframe does, with
                        // egui-winit's clipboard translation
                        let mut raw = egui::RawInput::default();
                        if m != egui::Modifiers::NONE {
                            raw.events.push(egui::Event::ModifiersChanged(m));
                        }
                        let ev = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: m };
                        match (m.command, key) {
                            (true, egui::Key::C) if cmd != "keyup" => raw.events.push(egui::Event::Copy),
                            (true, egui::Key::V) if cmd != "keyup" => raw.events.push(egui::Event::Paste("x".into())),
                            _ => {
                                if cmd != "keyup" {
                                    raw.events.push(ev(true));
                                }
                                if cmd != "keydown" {
                                    raw.events.push(ev(false));
                                }
                            }
                        }
                        let ctx = h.ctx.clone();
                        eframe::App::raw_input_hook(h.state_mut(), &ctx, &mut raw);
                        for e in raw.events {
                            h.event(e);
                        }
                        h.step();
                        if m != egui::Modifiers::NONE {
                            h.event(egui::Event::ModifiersChanged(held));
                        }
                        settle(&mut h, 6);
                    }
                    None => ok = false,
                }
            }
            "type" => {
                h.event(egui::Event::Text(rest.to_string()));
                settle(&mut h, 4);
            }
            "shot" => {
                let name = if rest.is_empty() { format!("step{ln}") } else { rest.to_string() };
                // shot NAME [x y w h]  crops to a region (logical points)
                let (name, region) = match name.split_whitespace().collect::<Vec<_>>()[..] {
                    [n, x, y, w, hh] => (n.to_string(), [x, y, w, hh].map(|v| v.parse::<f32>().unwrap_or(0.0)).into()),
                    _ => (name.clone(), None::<[f32; 4]>),
                };
                let p = out.join(format!("{name}.png"));
                let mut img = h.render().map_err(|e| anyhow::anyhow!(e))?;
                if let Some([x, y, w, hh]) = region {
                    let ppp = img.width() as f32 / h.ctx.content_rect().width().max(1.0);
                    let (x, y) = ((x * ppp) as u32, (y * ppp) as u32);
                    let w = ((w * ppp) as u32).min(img.width().saturating_sub(x));
                    let hh = ((hh * ppp) as u32).min(img.height().saturating_sub(y));
                    img = image::imageops::crop_imm(&img, x, y, w, hh).to_image();
                }
                img.save(&p)?;
                println!("[{}] shot {}", ln + 1, p.display());
            }
            // list visible widgets with label, role and rect; optional substring filter
            "labels" => {
                let f = rest.to_lowercase();
                for n in h.query_all(egui_kittest::kittest::by().predicate(|_| true).include_labels()) {
                    let ak = n.accesskit_node();
                    let label = ak.label().unwrap_or_default();
                    let value = ak.value().or_else(|| ak.numeric_value().map(|v| format!("{v}"))).unwrap_or_default();
                    if label.is_empty() && value.is_empty() {
                        continue;
                    }
                    if !f.is_empty() && !label.to_lowercase().contains(&f) {
                        continue;
                    }
                    let r = n.rect();
                    let extra = if ak.is_disabled() { " disabled" } else { "" };
                    println!(
                        "  {:?} {:?}{} value={:?} rect=({:.0},{:.0})-({:.0},{:.0}){}",
                        ak.role(),
                        label,
                        ak.toggled().map(|t| format!(" toggled={t:?}")).unwrap_or_default(),
                        value,
                        r.min.x,
                        r.min.y,
                        r.max.x,
                        r.max.y,
                        extra
                    );
                }
            }
            // print develop settings that differ from defaults, plus app state
            "state" => {
                let st = h.state();
                println!(
                    "[{}] state: tool={:?} zoom={:?} center={:?} view_rect={:?} pick={:?} cursor={:?} status={:?} docs={} current={:?} prefs_open={} confirm_reset={} filmstrip={}",
                    ln + 1,
                    st.tools.tool,
                    st.zoom,
                    st.center,
                    st.view_rect,
                    st.pick,
                    h.output().platform_output.cursor_icon,
                    st.status,
                    st.docs.len(),
                    st.current,
                    st.show_prefs,
                    st.confirm_reset,
                    st.show_filmstrip
                );
                if let Some(d) = st.doc() {
                    let cur = serde_json::to_value(&d.settings)?;
                    let def = serde_json::to_value(darkslide_core::params::DevelopSettings::default())?;
                    if let (Some(c), Some(z)) = (cur.as_object(), def.as_object()) {
                        for (k, v) in c {
                            if z.get(k) != Some(v) {
                                let s = v.to_string();
                                println!("    {k} = {}", if s.len() > 300 { format!("{}…", &s[..300]) } else { s });
                            }
                        }
                    }
                }
            }
            "cursor" => println!("[{}] cursor at {:?}: {:?}", ln + 1, pointer, h.output().platform_output.cursor_icon),
            "set" => {
                let mut sets = HashMap::new();
                if let Some((k, v)) = rest.split_once('=') {
                    sets.insert(k.to_string(), CrsValue::Text(v.to_string()));
                }
                if let Some(d) = h.state_mut().doc_mut() {
                    xmp::apply_crs(&sets, &mut d.settings);
                }
                settle(&mut h, 5);
            }
            "echo" => println!("[{}] {rest}", ln + 1),
            _ => ok = false,
        }
        if !ok {
            println!("[{}] FAILED: {line}", ln + 1);
        }
    }
    Ok(())
}
