//! Dark theme close to Camera Raw's.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

/// Add common system fonts as fallbacks so symbols used as icons render.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
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
    v.override_text_color = Some(Color32::from_rgb(225, 225, 225));
    v.panel_fill = Color32::from_rgb(34, 34, 34);       // Adobe panel #222222
    v.window_fill = Color32::from_rgb(28, 28, 28);      // Dialog windows #1c1c1c
    v.extreme_bg_color = Color32::from_rgb(18, 18, 18); // Sunken wells, inputs #121212
    v.faint_bg_color = Color32::from_rgb(25, 25, 25);
    
    // Adobe Blue selection (#1473e6)
    v.selection.bg_fill = Color32::from_rgb(20, 115, 230);
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    
    // Inactive buttons / controls
    v.widgets.inactive.bg_fill = Color32::from_rgb(46, 46, 46);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(38, 38, 38);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(56, 56, 56));
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(215, 215, 215));
    
    // Hovered controls
    v.widgets.hovered.bg_fill = Color32::from_rgb(60, 60, 60);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 48, 48);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(76, 76, 76));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    
    // Active / pressed controls
    v.widgets.active.bg_fill = Color32::from_rgb(74, 74, 74);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(60, 60, 60);
    v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(20, 115, 230));
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    
    // Non-interactive containers
    v.widgets.noninteractive.bg_fill = Color32::from_rgb(34, 34, 34);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(44, 44, 44));
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(170, 170, 170));
    
    // Open popups / menus
    v.widgets.open.bg_fill = Color32::from_rgb(42, 42, 42);
    v.widgets.open.bg_stroke = Stroke::new(1.0, Color32::from_rgb(66, 66, 66));
    
    v.window_corner_radius = CornerRadius::same(5);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(52, 52, 52));
    v.menu_corner_radius = CornerRadius::same(3);
    
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.noninteractive,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(3);
    }
    
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.5);
        s.spacing.menu_margin = egui::Margin::symmetric(4, 4);
        s.spacing.slider_width = 150.0;
    });
}
