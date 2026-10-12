//! Camera Raw's grey themes.

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
    // Phosphor icons, regular weight inline with text, fill as its own family
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    egui_phosphor::add_font_bytes_as_family(&mut fonts, "phosphor-fill", egui_phosphor::Variant::Fill.font_bytes());
    ctx.set_fonts(fonts);
}

/// Interface greys: our near-black default, plus Photoshop's Dark and
/// Darkest themes, which Camera Raw inherits.
pub struct Palette {
    /// Behind the image.
    pub canvas: Color32,
    /// Side panel and section headers.
    pub panel: Color32,
    /// Top/bottom bars and the tool strip.
    pub bar: Color32,
    pub filmstrip: Color32,
    /// Sunken areas: histogram, curve, value boxes.
    pub well: Color32,
    pub input: Color32,
    pub input_hover: Color32,
    pub button: Color32,
    pub button_hover: Color32,
    pub header_hover: Color32,
    pub divider: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub track: Color32,
    pub notch: Color32,
    pub grid: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_muted: Color32,
    /// Clickable text such as the workflow line.
    pub link: Color32,
}

const fn g(v: u8) -> Color32 {
    Color32::from_rgb(v, v, v)
}

pub const CLASSIC: Palette = Palette {
    canvas: g(18),
    panel: g(32),
    bar: g(26),
    filmstrip: g(26),
    well: g(22),
    input: g(18),
    input_hover: g(26),
    button: g(42),
    button_hover: g(54),
    header_hover: g(40),
    divider: g(42),
    border: g(52),
    border_strong: g(72),
    track: g(70),
    notch: g(95),
    grid: g(55),
    text: g(222),
    text_dim: g(190),
    text_muted: g(120),
    link: Color32::from_rgb(20, 115, 230),
};

pub const DARK: Palette = Palette {
    canvas: g(40),
    panel: g(83),
    bar: g(83),
    filmstrip: g(62),
    well: g(50),
    input: g(68),
    input_hover: g(76),
    button: g(72),
    button_hover: g(96),
    header_hover: g(92),
    divider: g(56),
    border: g(100),
    border_strong: g(120),
    track: g(140),
    notch: g(175),
    grid: g(70),
    text: g(235),
    text_dim: g(200),
    text_muted: g(165),
    link: g(200),
};

pub const DARKEST: Palette = Palette {
    canvas: g(30),
    panel: g(50),
    bar: g(50),
    filmstrip: g(40),
    well: g(34),
    input: g(40),
    input_hover: g(48),
    button: g(62),
    button_hover: g(80),
    header_hover: g(60),
    divider: g(38),
    border: g(70),
    border_strong: g(90),
    track: g(105),
    notch: g(140),
    grid: g(52),
    text: g(225),
    text_dim: g(185),
    text_muted: g(135),
    link: g(185),
};

static THEME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub const NAMES: [&str; 3] = ["Near-black", "Camera Raw grey", "Camera Raw darkest"];

pub fn pal() -> &'static Palette {
    match THEME.load(std::sync::atomic::Ordering::Relaxed) {
        1 => &DARK,
        2 => &DARKEST,
        _ => &CLASSIC,
    }
}

/// Switch palettes and restyle.
pub fn set(ctx: &egui::Context, theme: u8) {
    THEME.store(theme.min(NAMES.len() as u8 - 1), std::sync::atomic::Ordering::Relaxed);
    visuals(ctx);
}

pub fn apply(ctx: &egui::Context, theme: u8) {
    install_fonts(ctx);
    set(ctx, theme);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.5);
        s.spacing.menu_margin = egui::Margin::symmetric(4, 4);
        s.spacing.slider_width = 150.0;
    });
}

fn visuals(ctx: &egui::Context) {
    let p = pal();
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(p.text);
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.input;
    v.faint_bg_color = p.well;
    v.text_edit_bg_color = Some(p.input);

    // Adobe Blue selection (#1473e6)
    v.selection.bg_fill = Color32::from_rgb(20, 115, 230);
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);

    v.widgets.inactive.bg_fill = p.button;
    v.widgets.inactive.weak_bg_fill = p.button;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text_dim);

    v.widgets.hovered.bg_fill = p.button_hover;
    v.widgets.hovered.weak_bg_fill = p.button_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    v.widgets.active.bg_fill = p.button_hover;
    v.widgets.active.weak_bg_fill = p.button_hover;
    v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(20, 115, 230));
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    v.widgets.noninteractive.bg_fill = p.panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.divider);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text_dim);

    v.widgets.open.bg_fill = p.input;
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.border);

    v.window_corner_radius = CornerRadius::same(4);
    v.window_stroke = Stroke::new(1.0, p.divider);
    v.menu_corner_radius = CornerRadius::same(3);

    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.noninteractive, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(3);
    }
    ctx.set_visuals(v);
}
