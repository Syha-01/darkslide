//! Tone: exposure, dehaze, highlights/shadows (local tone mapping on an
//! edge-aware base layer), clarity, texture, contrast, whites/blacks and the
//! profile base curve. Operates on luminance in log2 space and rescales RGB by
//! the luminance ratio so hue is preserved.

use rayon::prelude::*;

use super::local::{LP, LocalFields};
use crate::color::{luma_prophoto, srgb_decode, srgb_encode};
use crate::image::{Image3, Plane, Rgb, box_blur, gaussian_blur, min_filter};
use crate::profiles::BaseCurve;

pub const MID_GREY: f32 = 0.18;

#[inline]
pub fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn to_stops(y: f32) -> f32 {
    (y.max(1e-7) / MID_GREY).log2().clamp(-16.0, 10.0)
}

/// Global (whole-image) context computed on a small proxy so any region at
/// any scale renders consistently.
#[derive(Clone, Debug, Default)]
pub struct ToneCtx {
    pub pw: usize,
    pub ph: usize,
    /// Fast guided filter coefficient maps for the highlights/shadows base.
    pub base_a: Plane,
    pub base_b: Plane,
    /// Same for the clarity base layer.
    pub clar_a: Plane,
    pub clar_b: Plane,
    /// Dehaze: refined dark channel (0..1) and atmospheric light.
    pub haze: Option<(Plane, Rgb)>,
    /// Adaptive profile: automatic highlight/shadow amounts (slider units).
    pub adaptive: [f32; 2],
    /// Image statistics for auto tone.
    pub stats: Stats,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Percentiles of log2 luminance (stops re 18% grey): 0.5%, 5%, 50%, 95%, 99.5%
    pub pct: [f32; 5],
}

fn guided_coeffs(i: &Plane, p: &Plane, r: usize, eps: f32) -> (Plane, Plane) {
    let mean_i = box_blur(i, r);
    let mean_p = box_blur(p, r);
    let mean_ip = box_blur(&i.zip_map(p, |a, b| a * b), r);
    let mean_ii = box_blur(&i.map(|a| a * a), r);
    let mut a = Plane::new(i.w, i.h);
    let mut b = Plane::new(i.w, i.h);
    a.data.par_iter_mut().zip(b.data.par_iter_mut()).enumerate().for_each(|(k, (av, bv))| {
        let cov = mean_ip.data[k] - mean_i.data[k] * mean_p.data[k];
        let var = mean_ii.data[k] - mean_i.data[k] * mean_i.data[k];
        *av = cov / (var + eps);
        *bv = mean_p.data[k] - *av * mean_i.data[k];
    });
    (box_blur(&a, r), box_blur(&b, r))
}

/// Sample a proxy-resolution plane at crop-frame coords.
#[inline]
pub fn sample_ctx(p: &Plane, cx: f32, cy: f32) -> f32 {
    p.sample_bilinear(cx * p.w as f32, cy * p.h as f32)
}

pub struct DehazeInput<'a> {
    pub amount: &'a dyn Fn(usize) -> f32,
}

/// Dark channel prior on a proxy: returns refined haze density map and A.
pub fn estimate_haze(img: &Image3) -> (Plane, Rgb) {
    let (w, h) = (img.w, img.h);
    let r = (w.max(h) / 60).max(2);
    let dark0 = Plane { w, h, data: img.data.par_iter().map(|p| p[0].min(p[1]).min(p[2])).collect() };
    let dark = min_filter(&dark0, r);
    // atmospheric light: mean of brightest pixels among top 0.1% dark channel
    let mut idx: Vec<usize> = (0..dark.data.len()).collect();
    idx.sort_unstable_by(|a, b| dark.data[*b].partial_cmp(&dark.data[*a]).unwrap_or(std::cmp::Ordering::Equal));
    let n = (idx.len() / 1000).max(1);
    let mut a = [0.0f32; 3];
    for &i in &idx[..n] {
        for c in 0..3 {
            a[c] += img.data[i][c];
        }
    }
    let a = a.map(|v| (v / n as f32).max(1e-3));
    // normalised dark channel
    let nd0 = Plane { w, h, data: img.data.par_iter().map(|p| (p[0] / a[0]).min(p[1] / a[1]).min(p[2] / a[2]).clamp(0.0, 1.0)).collect() };
    let nd = min_filter(&nd0, r);
    let guide = img.luminance().map(|y| y.max(0.0).sqrt());
    let refined = crate::image::guided_filter(&guide, &nd, r * 2, 1e-3);
    (refined.map(|v| v.clamp(0.0, 1.0)), a)
}

/// Apply dehaze in scene-linear space.
#[inline]
pub fn dehaze_px(p: Rgb, density: f32, a: Rgb, amount: f32) -> Rgb {
    if amount == 0.0 {
        return p;
    }
    if amount > 0.0 {
        let t = (1.0 - 0.95 * amount * density).max(0.08);
        let mut o = [0.0; 3];
        for c in 0..3 {
            o[c] = ((p[c] - a[c] * (1.0 - t)) / t).max(0.0);
        }
        o
    } else {
        let k = (-amount) * (0.35 + 0.65 * density) * 0.85;
        let mut o = [0.0; 3];
        for c in 0..3 {
            o[c] = p[c] * (1.0 - k) + a[c] * k;
        }
        o
    }
}

/// Build the tone context from the proxy. `proxy` is scene-linear ProPhoto
/// after WB/calibration and dehaze; `exposure` gives total exposure (EV) per
/// proxy pixel.
pub fn build_ctx(proxy: &Image3, exposure: &(dyn Fn(usize) -> f32 + Sync), haze: Option<(Plane, Rgb)>, adaptive: bool) -> ToneCtx {
    let (w, h) = (proxy.w, proxy.h);
    let l = Plane { w, h, data: proxy.data.par_iter().enumerate().map(|(i, p)| to_stops(luma_prophoto(*p)) + exposure(i)).collect() };
    let long = w.max(h);
    let r_base = (long / 28).max(2);
    let r_clar = (long / 70).max(2);
    let (base_a, base_b) = guided_coeffs(&l, &l, r_base, 0.35);
    let (clar_a, clar_b) = guided_coeffs(&l, &l, r_clar, 0.12);
    let mut sorted: Vec<f32> = l.data.iter().copied().filter(|v| v.is_finite()).collect();
    sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    let pick = |q: f32| if sorted.is_empty() { 0.0 } else { sorted[((sorted.len() - 1) as f32 * q) as usize] };
    let stats = Stats { pct: [pick(0.005), pick(0.05), pick(0.5), pick(0.95), pick(0.995)] };
    let adaptive_amt = if adaptive { adaptive_amounts(&stats) } else { [0.0, 0.0] };
    ToneCtx { pw: w, ph: h, base_a, base_b, clar_a, clar_b, haze, adaptive: adaptive_amt, stats }
}

/// Adaptive profile: compress the range toward what fits the display.
fn adaptive_amounts(st: &Stats) -> [f32; 2] {
    let hi = st.pct[4];
    let lo = st.pct[1];
    // highlights: pull down when bright tail runs into the shoulder
    let h = -((hi - 2.2) * 30.0).clamp(0.0, 70.0);
    // shadows: lift when the dark part sits low
    let s = ((-2.5 - lo) * 18.0).clamp(0.0, 60.0);
    [h, s]
}

/// Per-pixel tone parameters (global + local).
#[derive(Clone, Copy, Debug, Default)]
pub struct ToneGlobals {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub clarity: f32,
    pub texture: f32,
    pub dehaze: f32,
    pub hdr: bool,
    pub hdr_headroom: f32,
}

/// Contrast as an S curve on the encoded value around a pivot, endpoints fixed.
#[inline]
pub fn contrast_curve(e: f32, c: f32) -> f32 {
    if c == 0.0 {
        return e;
    }
    let p = 0.47;
    let g = (c / 100.0 * 0.85).exp2();
    if e <= 0.0 {
        e
    } else if e < p {
        p * (e / p).powf(g)
    } else if e <= 1.0 {
        1.0 - (1.0 - p) * ((1.0 - e) / (1.0 - p)).powf(g)
    } else {
        1.0 + (e - 1.0) * g
    }
}

#[inline]
fn whites_blacks(e: f32, whites: f32, blacks: f32) -> f32 {
    let mut e = e;
    if whites != 0.0 {
        e += whites / 100.0 * 0.2 * smoothstep(0.35, 1.0, e.min(1.2));
    }
    if blacks != 0.0 && e < 0.55 {
        let t = 1.0 - e / 0.55;
        e += blacks / 100.0 * 0.08 * t * t;
    }
    e
}

/// Map a region from scene-linear ProPhoto to display-linear ProPhoto.
/// `crop_coords` gives crop-frame coords per pixel (for sampling the ctx).
#[allow(clippy::too_many_arguments)]
pub fn apply(
    img: &mut Image3,
    crop_coords: &(dyn Fn(usize) -> (f32, f32) + Sync),
    ctx: &ToneCtx,
    g: &ToneGlobals,
    fields: &LocalFields,
    curve: &BaseCurve,
    scale: f32,
) {
    let (w, h) = (img.w, img.h);
    // dehaze first (scene linear)
    if let Some((density, a)) = &ctx.haze {
        let any = g.dehaze != 0.0 || fields.has(LP::Dehaze);
        if any {
            img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
                let amt = (g.dehaze + fields.get(LP::Dehaze, i)) / 100.0;
                if amt != 0.0 {
                    let (cx, cy) = crop_coords(i);
                    let d = sample_ctx(density, cx, cy);
                    *p = dehaze_px(*p, d, *a, amt.clamp(-1.0, 1.0));
                }
            });
        }
    }

    // log luminance incl. exposure
    let l =
        Plane { w, h, data: img.data.par_iter().enumerate().map(|(i, p)| to_stops(luma_prophoto(*p)) + g.exposure + fields.get(LP::Exposure, i)).collect() };

    // texture: mid-frequency detail at full-res scale
    let tex_any = g.texture != 0.0 || fields.has(LP::Texture);
    let tex_detail = if tex_any {
        let s1 = (1.2 * scale).max(0.0);
        let s2 = (5.0 * scale).max(0.6);
        let b1 = if s1 >= 0.3 { gaussian_blur(&l, s1) } else { l.clone() };
        let b2 = gaussian_blur(&l, s2);
        Some(b1.zip_map(&b2, |a, b| a - b))
    } else {
        None
    };

    let adaptive = ctx.adaptive;
    let hdr_max = if g.hdr { g.hdr_headroom.exp2() } else { 1.0 };

    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let y0 = luma_prophoto(*p).max(1e-7);
        let mut lv = l.data[i];
        let (cx, cy) = crop_coords(i);
        let base = sample_ctx(&ctx.base_a, cx, cy) * lv + sample_ctx(&ctx.base_b, cx, cy);

        let shadows = g.shadows + fields.get(LP::Shadows, i) + adaptive[1];
        let highlights = g.highlights + fields.get(LP::Highlights, i) + adaptive[0];
        if shadows != 0.0 {
            // deep blacks stay anchored; Blacks handles those
            let ws = (1.0 - smoothstep(-3.0, 0.4, base)) * smoothstep(-8.5, -4.0, base);
            lv += shadows / 100.0 * 1.5 * ws;
        }
        if highlights != 0.0 {
            let wh = smoothstep(-0.6, 2.4, base);
            lv += highlights / 100.0 * 1.7 * wh;
        }
        let clarity = g.clarity + fields.get(LP::Clarity, i);
        if clarity != 0.0 {
            let cb = sample_ctx(&ctx.clar_a, cx, cy) * l.data[i] + sample_ctx(&ctx.clar_b, cx, cy);
            let d = (l.data[i] - cb).clamp(-2.5, 2.5);
            let mid = 1.0 - 0.6 * (lv / 4.0).tanh().abs();
            lv += clarity / 100.0 * 0.7 * d * mid;
        }
        if let Some(td) = &tex_detail {
            let t = g.texture + fields.get(LP::Texture, i);
            lv += t / 100.0 * 1.1 * td.data[i].clamp(-1.5, 1.5);
        }

        let mut e = curve.eval(lv);
        e = contrast_curve(e, g.contrast + fields.get(LP::Contrast, i));
        e = whites_blacks(e, g.whites + fields.get(LP::Whites, i), g.blacks + fields.get(LP::Blacks, i));
        let e = e.max(0.0);
        let yd = if e <= 1.0 { srgb_decode(e) } else { 1.0 + (e - 1.0) * 2.4 };
        let yd = yd.min(hdr_max);
        let ratio = yd / y0;
        let mut o = [p[0] * ratio, p[1] * ratio, p[2] * ratio];
        // keep the brightest channel within range by desaturating toward grey
        let m = o[0].max(o[1]).max(o[2]);
        if m > hdr_max {
            if yd >= hdr_max * 0.999 {
                o = [hdr_max; 3];
            } else {
                let k = (hdr_max - yd) / (m - yd);
                for c in 0..3 {
                    o[c] = yd + (o[c] - yd) * k;
                }
            }
        }
        // highlight rolloff to white as we approach the top
        let roll = smoothstep(0.9, 1.0, e.min(1.0)) * 0.5;
        if roll > 0.0 && !g.hdr {
            for c in 0..3 {
                o[c] += (yd - o[c]) * roll;
            }
        }
        *p = o;
    });
}

/// Auto tone (Ctrl+U): slider values derived from stats, ACR style.
pub fn auto_tone(st: &Stats) -> [f32; 6] {
    // exposure: bring median toward slightly above mid grey
    let exposure = (0.25 - st.pct[2]).clamp(-3.0, 3.0) * 0.8;
    let hi = st.pct[4] + exposure;
    let lo = st.pct[0] + exposure;
    let range = st.pct[3] - st.pct[1];
    let contrast = ((7.0 - range) * 6.0).clamp(-30.0, 35.0);
    let highlights = (-(hi - 2.0) * 35.0).clamp(-100.0, 0.0);
    let shadows = ((-3.0 - st.pct[1] - exposure) * 20.0).clamp(0.0, 60.0);
    let whites = ((2.6 - hi) * 18.0).clamp(-40.0, 40.0);
    let blacks = ((-6.5 - lo) * 8.0).clamp(-40.0, 25.0);
    [(exposure * 20.0).round() / 20.0, contrast.round(), highlights.round(), shadows.round(), whites.round(), blacks.round()]
}

/// Encode helper used by later display-domain stages.
#[inline]
pub fn enc(v: f32) -> f32 {
    if v <= 1.0 { srgb_encode(v) } else { 1.0 + (v - 1.0) / 2.4 }
}

#[inline]
pub fn dec(e: f32) -> f32 {
    if e <= 1.0 { srgb_decode(e) } else { 1.0 + (e - 1.0) * 2.4 }
}
