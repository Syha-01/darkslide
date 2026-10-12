//! Dark theme close to Camera Raw's.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

/// Add common system fonts as fallbacks so symbols used as icons render.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSerif.ttf",
    ];
    for (i, path) in candidates.iter().enumerate() {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("sys{i}");
            fonts.font_data.insert(name.clone(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().push(name.clone());
            }
        }
    }
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_gray(30);
    v.window_fill = Color32::from_gray(34);
    v.extreme_bg_color = Color32::from_gray(22);
    v.faint_bg_color = Color32::from_gray(36);
    v.selection.bg_fill = crate::widgets::ACCENT.gamma_multiply(0.8);
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.inactive.bg_fill = Color32::from_gray(48);
    v.widgets.inactive.weak_bg_fill = Color32::from_gray(44);
    v.widgets.hovered.bg_fill = Color32::from_gray(62);
    v.widgets.active.bg_fill = Color32::from_gray(72);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.noninteractive] {
        w.corner_radius = CornerRadius::same(3);
    }
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.slider_width = 150.0;
    });
}
