//! Presets shipped with the app. Adaptive ones add AI masks that the UI
//! detects after applying.

use crate::params::*;

pub struct BuiltinPreset {
    pub group: &'static str,
    pub name: &'static str,
    pub apply: fn(&mut DevelopSettings),
}

fn ai_mask(s: &mut DevelopSettings, kind: AiMaskKind, name: &str, adj: LocalAdjust) {
    s.masks.push(MaskGroup {
        name: name.into(),
        components: vec![MaskComponent { shape: MaskShape::Ai { kind, feather: 10.0, edge: 0.0, hint: None, raster: None }, ..Default::default() }],
        adjust: adj,
        ..Default::default()
    });
}

fn look(s: &mut DevelopSettings, name: &str, amount: f32) {
    s.look = Some(Look { name: name.into(), source: format!("builtin:{name}"), amount });
}

pub fn all() -> Vec<BuiltinPreset> {
    vec![
        BuiltinPreset {
            group: "Adaptive: Portrait",
            name: "Subject pop",
            apply: |s| ai_mask(s, AiMaskKind::Subject, "Subject pop", LocalAdjust { exposure: 0.3, clarity: 10.0, saturation: 10.0, ..Default::default() }),
        },
        BuiltinPreset {
            group: "Adaptive: Portrait",
            name: "Soft skin",
            apply: |s| ai_mask(s, AiMaskKind::Person, "Soft skin", LocalAdjust { texture: -35.0, clarity: -10.0, exposure: 0.1, ..Default::default() }),
        },
        BuiltinPreset {
            group: "Adaptive: Portrait",
            name: "Darken background",
            apply: |s| ai_mask(s, AiMaskKind::Background, "Darker background", LocalAdjust { exposure: -0.6, saturation: -15.0, ..Default::default() }),
        },
        BuiltinPreset {
            group: "Adaptive: Sky",
            name: "Blue sky",
            apply: |s| {
                ai_mask(
                    s,
                    AiMaskKind::Sky,
                    "Blue sky",
                    LocalAdjust { exposure: -0.4, saturation: 25.0, dehaze: 15.0, temperature: -10.0, ..Default::default() },
                )
            },
        },
        BuiltinPreset {
            group: "Adaptive: Sky",
            name: "Dramatic sky",
            apply: |s| {
                ai_mask(s, AiMaskKind::Sky, "Dramatic sky", LocalAdjust { exposure: -0.7, contrast: 30.0, clarity: 25.0, dehaze: 25.0, ..Default::default() })
            },
        },
        BuiltinPreset {
            group: "Adaptive: Sky",
            name: "Sunset glow",
            apply: |s| {
                ai_mask(
                    s,
                    AiMaskKind::Sky,
                    "Sunset glow",
                    LocalAdjust { temperature: 35.0, tint: 10.0, saturation: 25.0, exposure: -0.2, ..Default::default() },
                )
            },
        },
        BuiltinPreset {
            group: "Style",
            name: "Punchy",
            apply: |s| {
                s.contrast = 25.0;
                s.vibrance = 25.0;
                s.clarity = 10.0;
            },
        },
        BuiltinPreset {
            group: "Style",
            name: "High key",
            apply: |s| {
                s.exposure = 0.6;
                s.highlights = -30.0;
                s.shadows = 40.0;
                s.contrast = -15.0;
                s.whites = 15.0;
            },
        },
        BuiltinPreset {
            group: "Style",
            name: "Low key",
            apply: |s| {
                s.exposure = -0.5;
                s.blacks = -30.0;
                s.contrast = 30.0;
                s.vignette.amount = -30.0;
            },
        },
        BuiltinPreset { group: "Style", name: "Matte", apply: |s| look(s, "Matte", 100.0) },
        BuiltinPreset { group: "Style", name: "Warm film", apply: |s| look(s, "Warm Film", 100.0) },
        BuiltinPreset { group: "Style", name: "Cool film", apply: |s| look(s, "Cool Film", 100.0) },
        BuiltinPreset { group: "Style", name: "Teal & orange", apply: |s| look(s, "Teal & Orange", 100.0) },
        BuiltinPreset { group: "Style", name: "Vintage", apply: |s| look(s, "Vintage 01", 100.0) },
        BuiltinPreset {
            group: "Black & White",
            name: "B&W contrast",
            apply: |s| {
                s.treatment = Treatment::BlackAndWhite;
                s.contrast = 40.0;
                s.clarity = 20.0;
                s.blacks = -10.0;
            },
        },
        BuiltinPreset {
            group: "Black & White",
            name: "B&W soft",
            apply: |s| {
                s.treatment = Treatment::BlackAndWhite;
                s.contrast = -20.0;
                s.highlights = -20.0;
                s.shadows = 20.0;
            },
        },
        BuiltinPreset {
            group: "Black & White",
            name: "Selenium",
            apply: |s| {
                s.treatment = Treatment::BlackAndWhite;
                s.contrast = 20.0;
                s.grading.shadows = Wheel { hue: 280.0, sat: 18.0, lum: 0.0 };
                s.grading.highlights = Wheel { hue: 45.0, sat: 8.0, lum: 0.0 };
            },
        },
        BuiltinPreset {
            group: "Landscape",
            name: "Vivid landscape",
            apply: |s| {
                s.profile = "Landscape".into();
                s.vibrance = 20.0;
                s.dehaze = 10.0;
                s.clarity = 15.0;
            },
        },
        BuiltinPreset {
            group: "Landscape",
            name: "Rich foliage",
            apply: |s| {
                s.hsl[3].saturation = 20.0;
                s.hsl[3].luminance = -10.0;
                s.hsl[2].saturation = 10.0;
                s.hsl[2].hue = -5.0;
            },
        },
        BuiltinPreset {
            group: "Detail",
            name: "Sharpen – landscape",
            apply: |s| {
                s.sharpen = Sharpening { amount: 60.0, radius: 1.0, detail: 40.0, masking: 20.0 };
            },
        },
        BuiltinPreset {
            group: "Detail",
            name: "Sharpen – portrait",
            apply: |s| {
                s.sharpen = Sharpening { amount: 35.0, radius: 1.4, detail: 15.0, masking: 60.0 };
            },
        },
        BuiltinPreset {
            group: "Detail",
            name: "High ISO noise",
            apply: |s| {
                s.nr.luminance = 40.0;
                s.nr.color = 40.0;
            },
        },
        BuiltinPreset {
            group: "Effects",
            name: "Grain – light",
            apply: |s| {
                s.grain = Grain { amount: 20.0, size: 20.0, roughness: 40.0 };
            },
        },
        BuiltinPreset {
            group: "Effects",
            name: "Grain – heavy",
            apply: |s| {
                s.grain = Grain { amount: 60.0, size: 40.0, roughness: 70.0 };
            },
        },
        BuiltinPreset {
            group: "Effects",
            name: "Vignette – subtle",
            apply: |s| {
                s.vignette.amount = -20.0;
                s.vignette.feather = 70.0;
            },
        },
        BuiltinPreset {
            group: "Effects",
            name: "Vignette – strong",
            apply: |s| {
                s.vignette.amount = -55.0;
                s.vignette.midpoint = 35.0;
            },
        },
    ]
}
