//! XMP: Camera Raw (`crs:`) settings read/write for sidecars and presets.
//! Our full settings are also stored as JSON in the `ds:` namespace so
//! everything (masks, retouch, ...) round-trips losslessly.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use quick_xml::events::Event;

use crate::params::*;

const CRS_NS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
const DS_NS: &str = "https://darkslide.org/ns/1.0/";

#[derive(Clone, Debug, PartialEq)]
pub enum CrsValue {
    Text(String),
    Seq(Vec<String>),
}

impl CrsValue {
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            CrsValue::Text(t) => t.trim().trim_start_matches('+').parse().ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            CrsValue::Text(t) => Some(t),
            CrsValue::Seq(v) => v.first().map(|s| s.as_str()),
        }
    }
}

/// Parsed XMP: crs properties and an optional embedded full settings JSON.
#[derive(Clone, Debug, Default)]
pub struct ParsedXmp {
    pub crs: HashMap<String, CrsValue>,
    pub ds_json: Option<String>,
    pub rating: Option<i32>,
    pub label: Option<String>,
    /// Local corrections written by Camera Raw / Lightroom.
    pub masks: Vec<MaskGroup>,
}

fn resolve_entity(name: &str) -> String {
    match name {
        "quot" => "\"".into(),
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "apos" => "'".into(),
        n if n.starts_with("#x") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32).map(|c| c.to_string()).unwrap_or_default(),
        n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32).map(|c| c.to_string()).unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn parse(xml: &str) -> ParsedXmp {
    let mut r = quick_xml::Reader::from_str(xml);
    let mut out = ParsedXmp::default();
    if xml.contains("MaskGroupBasedCorrections") {
        out.masks = crate::xmp_masks::read(xml);
    }
    let mut buf = vec![];
    // innermost crs/ds/xmp property element currently open
    let mut prop: Option<String> = None;
    let mut seq: Vec<String> = vec![];
    let mut in_li = false;
    let mut text = String::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(ev @ (Event::Start(_) | Event::Empty(_))) => {
                let is_empty = matches!(ev, Event::Empty(_));
                let e = match ev {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "rdf:Description" {
                    for a in e.attributes().flatten() {
                        let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
                        let v = a.unescape_value().map(|v| v.to_string()).unwrap_or_default();
                        if let Some(n) = k.strip_prefix("crs:") {
                            out.crs.insert(n.to_string(), CrsValue::Text(v));
                        } else if k == "xmp:Rating" {
                            out.rating = v.parse().ok();
                        } else if k == "xmp:Label" {
                            out.label = Some(v);
                        } else if k == "ds:Settings" {
                            out.ds_json = Some(v);
                        }
                    }
                } else if is_empty {
                } else if name.starts_with("crs:") || name == "ds:Settings" || name == "xmp:Rating" || name == "xmp:Label" {
                    prop = Some(name);
                    seq.clear();
                    text.clear();
                } else if name == "rdf:li" {
                    in_li = true;
                    text.clear();
                }
            }
            Ok(Event::Text(t)) => {
                if prop.is_some() {
                    text.push_str(&t.decode().map(|c| c.to_string()).unwrap_or_default());
                }
            }
            Ok(Event::GeneralRef(e)) => {
                if prop.is_some() {
                    let n = e.decode().map(|c| c.to_string()).unwrap_or_default();
                    text.push_str(&resolve_entity(&n));
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "rdf:li" && in_li {
                    in_li = false;
                    seq.push(std::mem::take(&mut text));
                } else if prop.as_deref() == Some(name.as_str()) {
                    let t = std::mem::take(&mut text).trim().to_string();
                    match name.as_str() {
                        "ds:Settings" => out.ds_json = Some(t),
                        "xmp:Rating" => out.rating = t.parse().ok(),
                        "xmp:Label" => out.label = Some(t),
                        n => {
                            let key = n.trim_start_matches("crs:").to_string();
                            if !seq.is_empty() {
                                out.crs.insert(key, CrsValue::Seq(std::mem::take(&mut seq)));
                            } else if !t.is_empty() {
                                out.crs.insert(key, CrsValue::Text(t));
                            }
                        }
                    }
                    prop = None;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

fn curve_from_seq(v: &CrsValue) -> Option<CurvePoints> {
    match v {
        CrsValue::Seq(items) => {
            let pts: Vec<[f32; 2]> = items
                .iter()
                .filter_map(|s| {
                    let mut it = s.split(',').map(|t| t.trim().parse::<f32>());
                    Some([it.next()?.ok()?, it.next()?.ok()?])
                })
                .collect();
            if pts.len() >= 2 { Some(pts) } else { None }
        }
        _ => None,
    }
}

/// Simple numeric fields: (crs name, accessor).
type Acc = fn(&mut DevelopSettings) -> &mut f32;

fn numeric_fields() -> Vec<(&'static str, Acc)> {
    vec![
        ("Temperature", |s| &mut s.temperature),
        ("Tint", |s| &mut s.tint),
        ("Exposure2012", |s| &mut s.exposure),
        ("Contrast2012", |s| &mut s.contrast),
        ("Highlights2012", |s| &mut s.highlights),
        ("Shadows2012", |s| &mut s.shadows),
        ("Whites2012", |s| &mut s.whites),
        ("Blacks2012", |s| &mut s.blacks),
        ("Texture", |s| &mut s.texture),
        ("Clarity2012", |s| &mut s.clarity),
        ("Dehaze", |s| &mut s.dehaze),
        ("Vibrance", |s| &mut s.vibrance),
        ("Saturation", |s| &mut s.saturation),
        ("ParametricShadows", |s| &mut s.parametric.shadows),
        ("ParametricDarks", |s| &mut s.parametric.darks),
        ("ParametricLights", |s| &mut s.parametric.lights),
        ("ParametricHighlights", |s| &mut s.parametric.highlights),
        ("ParametricShadowSplit", |s| &mut s.parametric.shadow_split),
        ("ParametricMidtoneSplit", |s| &mut s.parametric.midtone_split),
        ("ParametricHighlightSplit", |s| &mut s.parametric.highlight_split),
        ("CurveRefineSaturation", |s| &mut s.refine_saturation),
        ("Sharpness", |s| &mut s.sharpen.amount),
        ("SharpenRadius", |s| &mut s.sharpen.radius),
        ("SharpenDetail", |s| &mut s.sharpen.detail),
        ("SharpenEdgeMasking", |s| &mut s.sharpen.masking),
        ("LuminanceSmoothing", |s| &mut s.nr.luminance),
        ("LuminanceNoiseReductionDetail", |s| &mut s.nr.luminance_detail),
        ("LuminanceNoiseReductionContrast", |s| &mut s.nr.luminance_contrast),
        ("ColorNoiseReduction", |s| &mut s.nr.color),
        ("ColorNoiseReductionDetail", |s| &mut s.nr.color_detail),
        ("ColorNoiseReductionSmoothness", |s| &mut s.nr.color_smoothness),
        ("SplitToningShadowHue", |s| &mut s.grading.shadows.hue),
        ("SplitToningShadowSaturation", |s| &mut s.grading.shadows.sat),
        ("ColorGradeShadowLum", |s| &mut s.grading.shadows.lum),
        ("ColorGradeMidtoneHue", |s| &mut s.grading.midtones.hue),
        ("ColorGradeMidtoneSat", |s| &mut s.grading.midtones.sat),
        ("ColorGradeMidtoneLum", |s| &mut s.grading.midtones.lum),
        ("SplitToningHighlightHue", |s| &mut s.grading.highlights.hue),
        ("SplitToningHighlightSaturation", |s| &mut s.grading.highlights.sat),
        ("ColorGradeHighlightLum", |s| &mut s.grading.highlights.lum),
        ("ColorGradeGlobalHue", |s| &mut s.grading.global.hue),
        ("ColorGradeGlobalSat", |s| &mut s.grading.global.sat),
        ("ColorGradeGlobalLum", |s| &mut s.grading.global.lum),
        ("ColorGradeBlending", |s| &mut s.grading.blending),
        ("SplitToningBalance", |s| &mut s.grading.balance),
        ("LensProfileDistortionScale", |s| &mut s.lens.distortion_scale),
        ("LensProfileVignettingScale", |s| &mut s.lens.vignette_scale),
        ("LensManualDistortionAmount", |s| &mut s.lens.manual_distortion),
        ("VignetteAmount", |s| &mut s.lens.vignette_amount),
        ("VignetteMidpoint", |s| &mut s.lens.vignette_midpoint),
        ("DefringePurpleAmount", |s| &mut s.lens.defringe_purple_amount),
        ("DefringePurpleHueLo", |s| &mut s.lens.defringe_purple_hue[0]),
        ("DefringePurpleHueHi", |s| &mut s.lens.defringe_purple_hue[1]),
        ("DefringeGreenAmount", |s| &mut s.lens.defringe_green_amount),
        ("DefringeGreenHueLo", |s| &mut s.lens.defringe_green_hue[0]),
        ("DefringeGreenHueHi", |s| &mut s.lens.defringe_green_hue[1]),
        ("PerspectiveVertical", |s| &mut s.transform.vertical),
        ("PerspectiveHorizontal", |s| &mut s.transform.horizontal),
        ("PerspectiveRotate", |s| &mut s.transform.rotate),
        ("PerspectiveAspect", |s| &mut s.transform.aspect),
        ("PerspectiveScale", |s| &mut s.transform.scale),
        ("PerspectiveX", |s| &mut s.transform.offset_x),
        ("PerspectiveY", |s| &mut s.transform.offset_y),
        ("PostCropVignetteAmount", |s| &mut s.vignette.amount),
        ("PostCropVignetteMidpoint", |s| &mut s.vignette.midpoint),
        ("PostCropVignetteFeather", |s| &mut s.vignette.feather),
        ("PostCropVignetteRoundness", |s| &mut s.vignette.roundness),
        ("PostCropVignetteHighlightContrast", |s| &mut s.vignette.highlights),
        ("GrainAmount", |s| &mut s.grain.amount),
        ("GrainSize", |s| &mut s.grain.size),
        ("GrainFrequency", |s| &mut s.grain.roughness),
        ("ShadowTint", |s| &mut s.calibration.shadow_tint),
        ("RedHue", |s| &mut s.calibration.red_hue),
        ("RedSaturation", |s| &mut s.calibration.red_sat),
        ("GreenHue", |s| &mut s.calibration.green_hue),
        ("GreenSaturation", |s| &mut s.calibration.green_sat),
        ("BlueHue", |s| &mut s.calibration.blue_hue),
        ("BlueSaturation", |s| &mut s.calibration.blue_sat),
        ("CropTop", |s| &mut s.crop.top),
        ("CropLeft", |s| &mut s.crop.left),
        ("CropBottom", |s| &mut s.crop.bottom),
        ("CropRight", |s| &mut s.crop.right),
        ("CropAngle", |s| &mut s.crop.angle),
        ("HDRMaxValue", |s| &mut s.hdr.max_headroom),
        ("SDRBrightness", |s| &mut s.hdr.sdr_brightness),
        ("SDRContrast", |s| &mut s.hdr.sdr_contrast),
        ("SDRClarity", |s| &mut s.hdr.sdr_clarity),
        ("SDRHighlights", |s| &mut s.hdr.sdr_highlights),
        ("SDRShadows", |s| &mut s.hdr.sdr_shadows),
        ("SDRWhites", |s| &mut s.hdr.sdr_whites),
        ("SDRBlend", |s| &mut s.hdr.sdr_blend),
    ]
}

fn hsl_names(kind: &str) -> [String; 8] {
    HSL_BANDS.map(|b| format!("{kind}Adjustment{b}"))
}

/// Apply the crs values present in `map` on top of `s` (presets apply only
/// what they contain).
pub fn apply_crs(map: &HashMap<String, CrsValue>, s: &mut DevelopSettings) {
    for (name, acc) in numeric_fields() {
        if let Some(v) = map.get(name).and_then(|v| v.as_f32()) {
            *acc(s) = v;
        }
    }
    for (k, kind) in ["Hue", "Saturation", "Luminance"].iter().enumerate() {
        for (i, n) in hsl_names(kind).iter().enumerate() {
            if let Some(v) = map.get(n).and_then(|v| v.as_f32()) {
                match k {
                    0 => s.hsl[i].hue = v,
                    1 => s.hsl[i].saturation = v,
                    _ => s.hsl[i].luminance = v,
                }
            }
        }
    }
    for (i, b) in HSL_BANDS.iter().enumerate() {
        if let Some(v) = map.get(&format!("GrayMixer{b}")).and_then(|v| v.as_f32()) {
            s.gray_mix[i] = v;
            s.auto_gray_mix = false;
        }
    }
    if let Some(v) = map.get("WhiteBalance").and_then(|v| v.as_str()) {
        s.wb_mode = WbMode::from_crs(v);
    }
    if let Some(v) = map.get("ConvertToGrayscale").and_then(|v| v.as_str()) {
        s.treatment = if v.eq_ignore_ascii_case("true") { Treatment::BlackAndWhite } else { Treatment::Color };
    }
    if let Some(v) = map.get("CameraProfile").and_then(|v| v.as_str()) {
        s.profile = crate::profiles::find(v).name.to_string();
    }
    if let Some(v) = map.get("ProcessVersion").and_then(|v| v.as_str()) {
        s.process_version = v.to_string();
    }
    let flag = |k: &str| map.get(k).and_then(|v| v.as_str()).map(|v| v.eq_ignore_ascii_case("true") || v == "1");
    if let Some(v) = flag("AutoLateralCA") {
        s.lens.remove_ca = v;
    }
    if let Some(v) = flag("LensProfileEnable") {
        s.lens.profile_enable = v;
    }
    if let Some(v) = flag("HDREditMode") {
        s.hdr.enabled = v;
    }
    if let Some(v) = map.get("PerspectiveUpright").and_then(|v| v.as_f32()) {
        s.transform.upright = match v as i32 {
            1 => Upright::Auto,
            2 => Upright::Level,
            3 => Upright::Vertical,
            4 => Upright::Full,
            5 => Upright::Guided,
            _ => Upright::Off,
        };
    }
    if let Some(v) = map.get("PostCropVignetteStyle").and_then(|v| v.as_f32()) {
        s.vignette.style = match v as i32 {
            2 => VignetteStyle::ColorPriority,
            3 => VignetteStyle::PaintOverlay,
            _ => VignetteStyle::HighlightPriority,
        };
    }
    if let Some(false) = flag("HasCrop") {
        s.crop = Crop { rotate90: s.crop.rotate90, flip_h: s.crop.flip_h, flip_v: s.crop.flip_v, ..Crop::default() };
    }
    for (key, curve) in [
        ("ToneCurvePV2012", &mut s.curve.rgb),
        ("ToneCurvePV2012Red", &mut s.curve.red),
        ("ToneCurvePV2012Green", &mut s.curve.green),
        ("ToneCurvePV2012Blue", &mut s.curve.blue),
    ] {
        if let Some(c) = map.get(key).and_then(curve_from_seq) {
            *curve = c;
        }
    }
}

fn fmt_num(v: f32) -> String {
    if (v - v.round()).abs() < 1e-4 {
        let r = v.round() as i64;
        if r > 0 { format!("+{r}") } else { format!("{r}") }
    } else {
        format!("{v:+.2}")
    }
}

/// crs attributes and sequences for the settings.
pub fn to_crs(s: &DevelopSettings, is_raw: bool) -> (Vec<(String, String)>, Vec<(String, Vec<String>)>) {
    let mut attrs: Vec<(String, String)> = vec![];
    attrs.push(("Version".into(), "18.4".into()));
    attrs.push(("ProcessVersion".into(), s.process_version.clone()));
    attrs.push(("WhiteBalance".into(), s.wb_mode.crs_name().into()));
    let mut tmp = s.clone();
    let unsigned = [
        "Temperature",
        "Sharpness",
        "SharpenRadius",
        "SharpenDetail",
        "SharpenEdgeMasking",
        "CropTop",
        "CropLeft",
        "CropBottom",
        "CropRight",
        "CropAngle",
        "PerspectiveScale",
        "GrainAmount",
        "GrainSize",
        "GrainFrequency",
        "ColorGradeBlending",
        "LensProfileDistortionScale",
        "LensProfileVignettingScale",
        "HDRMaxValue",
    ];
    for (name, acc) in numeric_fields() {
        if !is_raw && name == "Temperature" && s.wb_mode == WbMode::AsShot {
            continue;
        }
        let v = *acc(&mut tmp);
        let txt = if unsigned.contains(&name) || name.starts_with("Crop") { format!("{}", (v * 1e6).round() / 1e6) } else { fmt_num(v) };
        attrs.push((name.into(), txt));
    }
    for (k, kind) in ["Hue", "Saturation", "Luminance"].iter().enumerate() {
        for (i, n) in hsl_names(kind).iter().enumerate() {
            let v = match k {
                0 => s.hsl[i].hue,
                1 => s.hsl[i].saturation,
                _ => s.hsl[i].luminance,
            };
            attrs.push((n.clone(), fmt_num(v)));
        }
    }
    if s.is_bw() {
        for (i, b) in HSL_BANDS.iter().enumerate() {
            attrs.push((format!("GrayMixer{b}"), fmt_num(s.gray_mix[i])));
        }
    }
    attrs.push(("ConvertToGrayscale".into(), if s.is_bw() { "True" } else { "False" }.into()));
    attrs.push(("CameraProfile".into(), format!("Adobe {}", s.profile)));
    attrs.push(("AutoLateralCA".into(), if s.lens.remove_ca { "1" } else { "0" }.into()));
    attrs.push(("LensProfileEnable".into(), if s.lens.profile_enable { "1" } else { "0" }.into()));
    attrs.push((
        "PerspectiveUpright".into(),
        match s.transform.upright {
            Upright::Off => "0",
            Upright::Auto => "1",
            Upright::Level => "2",
            Upright::Vertical => "3",
            Upright::Full => "4",
            Upright::Guided => "5",
        }
        .into(),
    ));
    attrs.push((
        "PostCropVignetteStyle".into(),
        match s.vignette.style {
            VignetteStyle::HighlightPriority => "1",
            VignetteStyle::ColorPriority => "2",
            VignetteStyle::PaintOverlay => "3",
        }
        .into(),
    ));
    attrs.push(("HasCrop".into(), if s.crop.has_crop() { "True" } else { "False" }.into()));
    attrs.push(("HDREditMode".into(), if s.hdr.enabled { "1" } else { "0" }.into()));
    let curve_seq = |c: &CurvePoints| c.iter().map(|p| format!("{}, {}", p[0].round() as i32, p[1].round() as i32)).collect::<Vec<_>>();
    let seqs = vec![
        ("ToneCurvePV2012".to_string(), curve_seq(&s.curve.rgb)),
        ("ToneCurvePV2012Red".to_string(), curve_seq(&s.curve.red)),
        ("ToneCurvePV2012Green".to_string(), curve_seq(&s.curve.green)),
        ("ToneCurvePV2012Blue".to_string(), curve_seq(&s.curve.blue)),
    ];
    (attrs, seqs)
}

fn esc(s: &str) -> String {
    quick_xml::escape::escape(s).to_string()
}

pub struct XmpExtra {
    pub rating: Option<i32>,
    pub label: Option<String>,
    pub preset_name: Option<String>,
    pub preset_group: Option<String>,
}

/// Serialise settings as a complete XMP packet.
pub fn to_xmp(s: &DevelopSettings, is_raw: bool, extra: &XmpExtra) -> String {
    let (attrs, seqs) = to_crs(s, is_raw);
    let mut x = String::new();
    x.push_str("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Darkslide\">\n");
    x.push_str(" <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n");
    x.push_str(&format!(
        "  <rdf:Description rdf:about=\"\"\n    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n    xmlns:crs=\"{CRS_NS}\"\n    xmlns:ds=\"{DS_NS}\""
    ));
    if let Some(r) = extra.rating {
        x.push_str(&format!("\n    xmp:Rating=\"{r}\""));
    }
    if let Some(l) = &extra.label {
        x.push_str(&format!("\n    xmp:Label=\"{}\"", esc(l)));
    }
    if extra.preset_name.is_some() {
        x.push_str("\n    crs:PresetType=\"Normal\"");
    }
    for (k, v) in &attrs {
        x.push_str(&format!("\n    crs:{k}=\"{}\"", esc(v)));
    }
    x.push_str(">\n");
    if let Some(n) = &extra.preset_name {
        x.push_str(&format!("   <crs:Name><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></crs:Name>\n", esc(n)));
    }
    if let Some(g) = &extra.preset_group {
        x.push_str(&format!("   <crs:Group><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></crs:Group>\n", esc(g)));
    }
    for (k, items) in &seqs {
        x.push_str(&format!("   <crs:{k}>\n    <rdf:Seq>\n"));
        for it in items {
            x.push_str(&format!("     <rdf:li>{}</rdf:li>\n", esc(it)));
        }
        x.push_str(&format!("    </rdf:Seq>\n   </crs:{k}>\n"));
    }
    x.push_str(&crate::xmp_masks::write(&s.masks));
    let json = serde_json::to_string(s).unwrap_or_default();
    x.push_str(&format!("   <ds:Settings>{}</ds:Settings>\n", esc(&json)));
    x.push_str("  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n");
    x
}

pub fn sidecar_path(image: &Path) -> PathBuf {
    image.with_extension("xmp")
}

pub fn write_sidecar(image: &Path, s: &DevelopSettings) -> Result<()> {
    write_sidecar_full(image, s, true, None, None)
}

pub fn write_sidecar_full(image: &Path, s: &DevelopSettings, is_raw: bool, rating: Option<i32>, label: Option<String>) -> Result<()> {
    let xmp = to_xmp(s, is_raw, &XmpExtra { rating, label, preset_name: None, preset_group: None });
    let p = sidecar_path(image);
    let tmp = p.with_extension("xmp.tmp");
    std::fs::write(&tmp, xmp)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// A sidecar with only a rating and label, for photos not opened yet.
pub fn write_meta_sidecar(image: &Path, rating: i32, label: Option<String>) -> Result<()> {
    let mut x = String::from("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Darkslide\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n");
    x.push_str(&format!("  <rdf:Description rdf:about=\"\"\n    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n    xmp:Rating=\"{rating}\""));
    if let Some(l) = label {
        x.push_str(&format!("\n    xmp:Label=\"{}\"", esc(&l)));
    }
    x.push_str("/>\n </rdf:RDF>\n</x:xmpmeta>\n");
    let p = sidecar_path(image);
    let tmp = p.with_extension("xmp.tmp");
    std::fs::write(&tmp, x)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// Load settings from a sidecar if present. Prefers our full JSON, falls back
/// to crs values (e.g. sidecars written by Camera Raw / Lightroom).
pub fn read_sidecar(image: &Path, is_raw: bool) -> Option<(DevelopSettings, ParsedXmp)> {
    let text = std::fs::read_to_string(sidecar_path(image)).ok()?;
    let parsed = parse(&text);
    let s = settings_from_parsed(&parsed, is_raw);
    Some((s, parsed))
}

pub fn settings_from_parsed(parsed: &ParsedXmp, is_raw: bool) -> DevelopSettings {
    if let Some(json) = &parsed.ds_json {
        if let Ok(s) = serde_json::from_str::<DevelopSettings>(json) {
            return s;
        }
    }
    let mut s = DevelopSettings::defaults_for(is_raw);
    apply_crs(&parsed.crs, &mut s);
    s.masks = parsed.masks.clone();
    s
}

// ---------------------------------------------------------------------------
// Presets

#[derive(Clone, Debug)]
pub struct Preset {
    pub name: String,
    pub group: String,
    pub path: PathBuf,
    pub parsed: ParsedXmp,
}

impl Preset {
    /// Apply this preset to settings at an amount (0..200). The amount scales
    /// numeric deltas relative to the current values, like ACR's preset amount.
    pub fn apply(&self, s: &DevelopSettings, amount: f32) -> DevelopSettings {
        let mut target = s.clone();
        if let Some(json) = &self.parsed.ds_json {
            if let Ok(full) = serde_json::from_str::<DevelopSettings>(json) {
                target = full;
                // presets never carry crops/geometry/spots from the source image
                target.crop = s.crop;
                target.retouch = s.retouch.clone();
                target.red_eye = s.red_eye.clone();
            }
        } else {
            apply_crs(&self.parsed.crs, &mut target);
        }
        blend(s, &target, amount / 100.0)
    }
}

/// Interpolate numeric settings from `a` toward `b` by `k` (0..2); everything
/// non-numeric comes from `b`. Used for preset amounts.
pub fn blend(a: &DevelopSettings, b: &DevelopSettings, k: f32) -> DevelopSettings {
    {
        if (k - 1.0).abs() < 1e-4 {
            return b.clone();
        }
        let mut out = b.clone();
        let mut a = a.clone();
        let mut b = b.clone();
        for (_, acc) in numeric_fields() {
            let va = *acc(&mut a);
            let vb = *acc(&mut b);
            *acc(&mut out) = va + (vb - va) * k;
        }
        for i in 0..8 {
            out.hsl[i].hue = a.hsl[i].hue + (b.hsl[i].hue - a.hsl[i].hue) * k;
            out.hsl[i].saturation = a.hsl[i].saturation + (b.hsl[i].saturation - a.hsl[i].saturation) * k;
            out.hsl[i].luminance = a.hsl[i].luminance + (b.hsl[i].luminance - a.hsl[i].luminance) * k;
        }
        if let (Some(la), Some(lb)) = (a.look.as_ref(), out.look.as_mut()) {
            lb.amount = la.amount + (lb.amount - la.amount) * k;
        } else if let Some(lb) = out.look.as_mut() {
            lb.amount *= k;
        }
        // scale the local adjustments of masks the preset added
        for m in out.masks.iter_mut().skip(a.masks.len()) {
            m.amount *= k;
        }
        out
    }
}

pub fn load_preset(path: &Path) -> Option<Preset> {
    let text = std::fs::read_to_string(path).ok()?;
    let parsed = parse(&text);
    let name = parsed
        .crs
        .get("Name")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
    let group = parsed
        .crs
        .get("Group")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .or_else(|| path.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "User Presets".into());
    Some(Preset { name, group, path: path.to_path_buf(), parsed })
}

/// All presets from the user preset dir (recursively).
pub fn list_presets() -> Vec<Preset> {
    let mut out = vec![];
    fn walk(dir: &Path, out: &mut Vec<Preset>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("xmp")).unwrap_or(false) {
                if let Some(pr) = load_preset(&p) {
                    out.push(pr);
                }
            }
        }
    }
    walk(&crate::paths::presets_dir(), &mut out);
    out.sort_by(|a, b| (a.group.as_str(), a.name.as_str()).cmp(&(b.group.as_str(), b.name.as_str())));
    out
}

/// Save a preset with only the chosen subset of settings (`keep` returns the
/// settings to store — callers zero what they exclude).
pub fn save_preset(name: &str, group: &str, s: &DevelopSettings) -> Result<PathBuf> {
    let dir = crate::paths::presets_dir().join(sanitize(group));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.xmp", sanitize(name)));
    let xmp = to_xmp(s, true, &XmpExtra { rating: None, label: None, preset_name: Some(name.into()), preset_group: Some(group.into()) });
    std::fs::write(&path, xmp)?;
    Ok(path)
}

pub fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || " -_().&".contains(c) { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_sidecar_has_no_settings() {
        let dir = std::env::temp_dir().join(format!("ds_meta_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("a.nef");
        write_meta_sidecar(&img, 4, Some("Red".into())).unwrap();
        let (_, p) = read_sidecar(&img, true).unwrap();
        assert_eq!((p.rating, p.label.as_deref()), (Some(4), Some("Red")));
        assert!(p.ds_json.is_none() && p.crs.is_empty() && p.masks.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn roundtrip_crs_and_json() {
        let mut s = DevelopSettings::default();
        s.exposure = 1.25;
        s.hsl[2].saturation = -30.0;
        s.curve.rgb = vec![[0.0, 0.0], [128.0, 140.0], [255.0, 255.0]];
        s.masks.push(MaskGroup::default());
        let xmp = to_xmp(&s, true, &XmpExtra { rating: Some(3), label: None, preset_name: None, preset_group: None });
        let p = parse(&xmp);
        assert_eq!(p.rating, Some(3));
        assert_eq!(p.crs.get("Exposure2012").and_then(|v| v.as_f32()), Some(1.25));
        let s2 = settings_from_parsed(&p, true);
        assert_eq!(s2, s);
        // crs-only path
        let mut p2 = p.clone();
        p2.ds_json = None;
        let s3 = settings_from_parsed(&p2, true);
        assert_eq!(s3.exposure, 1.25);
        assert_eq!(s3.hsl[2].saturation, -30.0);
        assert_eq!(s3.curve.rgb.len(), 3);
    }

    #[test]
    fn parses_acr_element_form() {
        let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
        <rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="+0.50">
        <crs:Contrast2012>+12</crs:Contrast2012>
        <crs:ToneCurvePV2012Red><rdf:Seq><rdf:li>0, 0</rdf:li><rdf:li>255, 240</rdf:li></rdf:Seq></crs:ToneCurvePV2012Red>
        </rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let p = parse(xml);
        let mut s = DevelopSettings::default();
        apply_crs(&p.crs, &mut s);
        assert_eq!(s.exposure, 0.5);
        assert_eq!(s.contrast, 12.0);
        assert_eq!(s.curve.red, vec![[0.0, 0.0], [255.0, 240.0]]);
    }
}
