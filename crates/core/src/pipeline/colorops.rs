//! Colour adjustments in display-referred space, done in Oklab/Oklch:
//! profile saturation, vibrance, saturation, colour mixer, B&W mix, point
//! colour, colour grading, calibration and local colour.

use rayon::prelude::*;

use crate::color::{self, Mat3, Oklab, hue_diff, lab_to_lch, lch_to_lab};
use crate::image::{Image3, Rgb};
use crate::params::{Calibration, ColorGrading, DevelopSettings, Hsl, PointColor};
use crate::profiles::Profile;

use super::local::{LP, LocalFields};
use super::tone::smoothstep;

/// Oklch hue (degrees) of the eight colour mixer bands.
pub const BAND_HUES: [f32; 8] = [24.0, 55.0, 105.0, 142.0, 195.0, 262.0, 295.0, 335.0];

/// Weights of each band for a hue (sum to 1).
#[inline]
pub fn band_weights(h: f32) -> [f32; 8] {
    let mut w = [0.0f32; 8];
    // find the bracketing bands on the circle
    for i in 0..8 {
        let a = BAND_HUES[i];
        let b = BAND_HUES[(i + 1) % 8];
        let span = (b - a).rem_euclid(360.0);
        let d = (h - a).rem_euclid(360.0);
        if d <= span {
            let t = d / span;
            let t = t * t * (3.0 - 2.0 * t);
            w[i] = 1.0 - t;
            w[(i + 1) % 8] = t;
            return w;
        }
    }
    w[0] = 1.0;
    w
}

/// Oklab hue direction for an ACR colour-wheel hue (HSV degrees).
pub fn wheel_dir(hsv_hue: f32, ok: &Oklab) -> [f32; 2] {
    let srgb = color::hsv_to_rgb([hsv_hue, 1.0, 1.0]);
    let pp = color::mul_mv(&color::to_prophoto(color::ColorSpace::SRgb), srgb.map(color::srgb_decode));
    let lab = ok.from_prophoto(pp);
    let n = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt().max(1e-6);
    [lab[1] / n, lab[2] / n]
}

/// Calibration as a matrix in linear ProPhoto (applied scene-linear).
pub fn calibration_matrix(c: &Calibration) -> Mat3 {
    if c.is_identity() {
        return color::IDENTITY;
    }
    // work in linear sRGB primaries so the primaries are real colours
    let to = color::prophoto_to(color::ColorSpace::SRgb);
    let from = color::to_prophoto(color::ColorSpace::SRgb);
    let k = 0.25;
    let prim = |idx: usize, hue: f32, sat: f32| -> [f32; 3] {
        let mut v = [0.0f32; 3];
        v[idx] = 1.0;
        let h = hue / 100.0 * k;
        // +hue rotates R->Y, G->C, B->M
        let (pos, neg) = match idx {
            0 => (1, 2),
            1 => (2, 0),
            _ => (0, 1),
        };
        if h > 0.0 {
            v[pos] += h;
        } else {
            v[neg] += -h;
        }
        let s = 1.0 + sat / 100.0 * 0.5;
        let mean = (v[0] + v[1] + v[2]) / 3.0;
        v.map(|x| mean + (x - mean) * s)
    };
    let r = prim(0, c.red_hue, c.red_sat);
    let g = prim(1, c.green_hue, c.green_sat);
    let b = prim(2, c.blue_hue, c.blue_sat);
    let mut m: Mat3 = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    // white stays white
    for row in m.iter_mut() {
        let s: f32 = row.iter().sum();
        if s.abs() > 1e-6 {
            for v in row.iter_mut() {
                *v /= s;
            }
        }
    }
    color::mul_mm(&from, &color::mul_mm(&m, &to))
}

pub struct ColorParams<'a> {
    pub s: &'a DevelopSettings,
    pub profile: &'a Profile,
    pub profile_amount: f32,
    pub ok: &'a Oklab,
    pub bw: bool,
}

struct GradeDirs {
    sh: [f32; 2],
    mid: [f32; 2],
    hi: [f32; 2],
    glob: [f32; 2],
}

fn grade_dirs(g: &ColorGrading, ok: &Oklab) -> GradeDirs {
    GradeDirs { sh: wheel_dir(g.shadows.hue, ok), mid: wheel_dir(g.midtones.hue, ok), hi: wheel_dir(g.highlights.hue, ok), glob: wheel_dir(g.global.hue, ok) }
}

#[inline]
fn apply_grading(lab: &mut Rgb, g: &ColorGrading, d: &GradeDirs, weight: f32) {
    let l = lab[0].clamp(0.0, 1.2);
    // zone weights in Oklab L; balance shifts the shadow/highlight split
    let bal = g.balance / 100.0;
    let blend = 0.12 + g.blending / 100.0 * 0.3;
    let split_lo = 0.38 + bal * 0.15;
    let split_hi = 0.68 + bal * 0.15;
    let ws = 1.0 - smoothstep(split_lo - blend, split_lo + blend, l);
    let wh = smoothstep(split_hi - blend, split_hi + blend, l);
    let wm = (1.0 - ws - wh).max(0.0);
    let zones = [(g.shadows, ws, d.sh), (g.midtones, wm, d.mid), (g.highlights, wh, d.hi), (g.global, 1.0, d.glob)];
    for (wheel, zw, dir) in zones {
        let zw = zw * weight;
        if zw <= 0.0 || wheel.is_identity() {
            continue;
        }
        let amt = wheel.sat / 100.0 * 0.075 * zw;
        lab[1] += dir[0] * amt;
        lab[2] += dir[1] * amt;
        lab[0] += wheel.lum / 100.0 * 0.12 * zw * (1.0 - (2.0 * l - 1.0).powi(4)).max(0.15);
    }
}

#[inline]
fn apply_point_colors(lch: &mut Rgb, pcs: &[PointColor], weight: f32) {
    for pc in pcs {
        let [sl, sc, sh] = pc.sample;
        let r = pc.range / 50.0;
        let hr = (8.0 + pc.hue_range / 100.0 * 40.0) * r;
        let cr = (0.02 + pc.sat_range / 100.0 * 0.12) * r;
        let lr = (0.05 + pc.lum_range / 100.0 * 0.4) * r;
        let dh = hue_diff(lch[2], sh);
        let wh = 1.0 - smoothstep(hr * 0.5, hr, dh.abs());
        let wc = 1.0 - smoothstep(cr * 0.5, cr, (lch[1] - sc).abs());
        let wl = 1.0 - smoothstep(lr * 0.5, lr, (lch[0] - sl).abs());
        // neutrals have no meaningful hue
        let wn = smoothstep(0.005, 0.03, lch[1]);
        let w = wh * wc * wl * wn * weight;
        if w <= 0.0 {
            continue;
        }
        if pc.variance != 0.0 {
            let k = (pc.variance / 100.0).clamp(-1.0, 1.0);
            lch[2] = (sh + dh * (1.0 + k * w)).rem_euclid(360.0);
        }
        lch[2] = (lch[2] + pc.hue_shift / 100.0 * 30.0 * w).rem_euclid(360.0);
        lch[1] = (lch[1] * (1.0 + pc.sat_shift / 100.0 * w)).max(0.0);
        lch[0] += pc.lum_shift / 100.0 * 0.2 * w;
    }
}

/// Main colour stage. `img` is display-linear ProPhoto.
pub fn apply(img: &mut Image3, cp: &ColorParams, fields: &LocalFields) {
    let s = cp.s;
    let ok = cp.ok;
    let panels = &s.panels;
    let prof = cp.profile;
    let pa = cp.profile_amount;
    let prof_sat = 1.0 + (prof.saturation - 1.0) * pa;
    let band_sat: [f32; 8] = prof.band_sat.map(|v| 1.0 + (v - 1.0) * pa);
    let band_hue: [f32; 8] = prof.band_hue.map(|v| v * pa);
    let vib = if panels.basic { s.vibrance / 100.0 } else { 0.0 };
    let sat = if panels.basic { s.saturation / 100.0 } else { 0.0 };
    let hsl: [Hsl; 8] = if panels.mixer { s.hsl } else { [Hsl::default(); 8] };
    let hsl_active = hsl.iter().any(|h| *h != Hsl::default());
    let gray_mix = if panels.mixer { s.gray_mix } else { [0.0; 8] };
    let points: &[PointColor] = if panels.point_color { &s.point_colors } else { &[] };
    let grading = if panels.grading { s.grading } else { ColorGrading::default() };
    let gdirs = grade_dirs(&grading, ok);
    let grading_on = !grading.is_identity();
    let shadow_tint = if panels.calibration { s.calibration.shadow_tint / 100.0 } else { 0.0 };
    let bw = cp.bw;

    let local_hue = fields.has(LP::Hue);
    let local_sat = fields.has(LP::Saturation);
    let local_complex: Vec<(usize, &crate::image::Plane)> = fields.weights.iter().map(|(m, p)| (*m, p)).collect();
    let local_dirs: Vec<GradeDirs> = local_complex.iter().map(|(m, _)| grade_dirs(&s.masks[*m].adjust.grading, ok)).collect();
    let local_tints: Vec<[f32; 2]> = local_complex.iter().map(|(m, _)| wheel_dir(s.masks[*m].adjust.color[0], ok)).collect();

    let any = prof_sat != 1.0
        || band_sat.iter().any(|v| *v != 1.0)
        || band_hue.iter().any(|v| *v != 0.0)
        || vib != 0.0
        || sat != 0.0
        || hsl_active
        || bw
        || !points.is_empty()
        || grading_on
        || shadow_tint != 0.0
        || local_hue
        || local_sat
        || !local_complex.is_empty();
    if !any {
        return;
    }

    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let mut lab = ok.from_prophoto(*p);
        // shadow tint (calibration): green(-) / magenta(+) in the shadows
        if shadow_tint != 0.0 {
            let w = (1.0 - lab[0]).clamp(0.0, 1.0).powi(2);
            lab[1] += shadow_tint * 0.03 * w;
            lab[2] -= shadow_tint * 0.012 * w;
        }
        let mut lch = lab_to_lch(lab);
        let bw_w = band_weights(lch[2]);

        // profile tweaks
        if prof_sat != 1.0 || band_sat.iter().any(|v| *v != 1.0) || band_hue.iter().any(|v| *v != 0.0) {
            let mut bs = 0.0;
            let mut bh = 0.0;
            for k in 0..8 {
                bs += bw_w[k] * band_sat[k];
                bh += bw_w[k] * band_hue[k];
            }
            lch[1] *= prof_sat * bs;
            lch[2] = (lch[2] + bh).rem_euclid(360.0);
        }

        // colour mixer
        if hsl_active {
            let mut dh = 0.0;
            let mut ds = 0.0;
            let mut dl = 0.0;
            for k in 0..8 {
                dh += bw_w[k] * hsl[k].hue;
                ds += bw_w[k] * hsl[k].saturation;
                dl += bw_w[k] * hsl[k].luminance;
            }
            let chroma_w = smoothstep(0.0, 0.08, lch[1]);
            lch[2] = (lch[2] + dh / 100.0 * 30.0 * chroma_w).rem_euclid(360.0);
            lch[1] *= (1.0 + ds / 100.0).max(0.0);
            lch[0] += dl / 100.0 * 0.18 * chroma_w * (lch[0] * (1.2 - lch[0])).max(0.0) * 3.0;
        }

        if !points.is_empty() {
            apply_point_colors(&mut lch, points, 1.0);
        }

        // vibrance: favour muted colours, protect skin hues
        let v = vib;
        if v != 0.0 {
            let muted = 1.0 - smoothstep(0.0, 0.22, lch[1]);
            let skin = 1.0 - 0.55 * (1.0 - smoothstep(15.0, 35.0, hue_diff(lch[2], 60.0).abs()));
            let k = if v > 0.0 { v * 1.1 * muted * skin } else { v };
            lch[1] *= (1.0 + k).max(0.0);
        }
        let sat_total = sat + if local_sat { fields.get(LP::Saturation, i) / 100.0 } else { 0.0 };
        if sat_total != 0.0 {
            lch[1] *= (1.0 + sat_total).max(0.0);
        }
        if local_hue {
            let h = fields.get(LP::Hue, i);
            lch[2] = (lch[2] + h / 100.0 * 60.0).rem_euclid(360.0);
        }

        lab = lch_to_lab(lch);

        if grading_on {
            apply_grading(&mut lab, &grading, &gdirs, 1.0);
        }

        for (k, (mi, wmap)) in local_complex.iter().enumerate() {
            let wv = wmap.data[i] * s.masks[*mi].amount / 100.0;
            if wv <= 0.0 {
                continue;
            }
            let adj = &s.masks[*mi].adjust;
            if !adj.grading.is_identity() {
                apply_grading(&mut lab, &adj.grading, &local_dirs[k], wv);
            }
            if adj.color[1] > 0.0 {
                let d = local_tints[k];
                let a = adj.color[1] / 100.0 * 0.06 * wv;
                lab[1] += d[0] * a;
                lab[2] += d[1] * a;
            }
            if !adj.point_colors.is_empty() {
                let mut l2 = lab_to_lch(lab);
                apply_point_colors(&mut l2, &adj.point_colors, wv);
                lab = lch_to_lab(l2);
            }
        }

        if bw {
            // B&W mix: band-weighted luminance offsets from the original colour
            let mut dl = 0.0;
            for k in 0..8 {
                dl += bw_w[k] * gray_mix[k];
            }
            let cw = smoothstep(0.0, 0.12, lch[1]);
            let l = lab[0] + dl / 100.0 * 0.25 * cw;
            // keep any toning that grading applied after conversion
            let tone_a = if grading_on { lab[1] - lch_to_lab(lch)[1] } else { 0.0 };
            let tone_b = if grading_on { lab[2] - lch_to_lab(lch)[2] } else { 0.0 };
            lab = [l, tone_a, tone_b];
        }

        *p = ok.to_prophoto(lab).map(|v| v.max(0.0));
    });
}

/// Auto B&W mix from the image: darken dominant hues a little, lift others.
pub fn auto_gray_mix(img: &Image3, ok: &Oklab) -> [f32; 8] {
    let mut acc = [0.0f32; 8];
    let step = (img.data.len() / 20000).max(1);
    for p in img.data.iter().step_by(step) {
        let lch = lab_to_lch(ok.from_prophoto(*p));
        let w = band_weights(lch[2]);
        for k in 0..8 {
            acc[k] += w[k] * lch[1];
        }
    }
    let total: f32 = acc.iter().sum::<f32>().max(1e-6);
    // defaults loosely modelled on ACR's auto mix: warm tones lighter, blues darker
    let base = [-8.0, 0.0, 12.0, -10.0, -18.0, -5.0, 6.0, 0.0];
    let mut out = [0.0f32; 8];
    for k in 0..8 {
        let share = acc[k] / total;
        out[k] = (base[k] * (0.5 + share * 2.0)).round();
    }
    out
}
