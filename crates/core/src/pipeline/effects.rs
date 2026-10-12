//! Effects and lens light-falloff: post-crop vignette, grain, manual lens
//! vignette, defringe.

use rayon::prelude::*;

use super::local::{LP, LocalFields};
use super::tone::{dec, enc, smoothstep};
use crate::color::{Oklab, hue_diff, lab_to_lch, lch_to_lab, luma_prophoto};
use crate::image::{Image3, Plane};
use crate::params::{Grain, LensCorrection, PostCropVignette, VignetteStyle};

/// Post-crop vignette in the display domain. `crop_xy` gives crop-frame coords
/// per pixel and `aspect` the crop's w/h.
pub fn post_crop_vignette(img: &mut Image3, v: &PostCropVignette, crop_xy: &(dyn Fn(usize) -> (f32, f32) + Sync), aspect: f32) {
    if v.amount == 0.0 {
        return;
    }
    let a = v.amount / 100.0;
    let mid = 0.25 + v.midpoint / 100.0 * 1.0;
    let f = (v.feather / 100.0).max(0.02);
    let round = v.roundness / 100.0;
    // superellipse exponent: -100 -> squarish, 0 -> ellipse, +100 -> circle in pixel space
    let n = if round < 0.0 { 2.0 + (-round) * 6.0 } else { 2.0 };
    let circ = round.max(0.0);
    let hl = v.highlights / 100.0;
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let (cx, cy) = crop_xy(i);
        let mut x = (cx - 0.5) * 2.0;
        let mut y = (cy - 0.5) * 2.0;
        // towards a circle: compress the long axis
        if aspect > 1.0 {
            x *= 1.0 + (aspect - 1.0) * circ;
        } else {
            y *= 1.0 + (1.0 / aspect - 1.0) * circ;
        }
        // normalised so the frame corners sit at r = 1
        let r = (x.abs().powf(n) + y.abs().powf(n)).powf(1.0 / n) / 2f32.powf(1.0 / n);
        let w = smoothstep(mid - f * 0.7, mid + f * 0.7, r);
        if w <= 0.0 {
            return;
        }
        let y0 = luma_prophoto(*p).max(1e-7);
        match v.style {
            VignetteStyle::HighlightPriority => {
                let protect = 1.0 - hl * smoothstep(0.4, 1.0, enc(y0));
                let k = if a < 0.0 { (1.0 + a * w * protect).max(0.0) } else { 1.0 + a * w * 1.5 };
                // exposure-like change: multiply in linear
                *p = p.map(|c| c * k);
            }
            VignetteStyle::ColorPriority => {
                let e = enc(y0);
                let e2 = if a < 0.0 { e * (1.0 + a * w * (1.0 - hl * smoothstep(0.5, 1.0, e))) } else { e + (1.0 - e) * a * w };
                let r = dec(e2.max(0.0)) / y0;
                *p = p.map(|c| c * r);
            }
            VignetteStyle::PaintOverlay => {
                let target = if a < 0.0 { 0.0 } else { 1.0 };
                let t = a.abs() * w;
                *p = p.map(|c| c + (target - c) * t);
            }
        }
    });
}

#[inline]
fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6b343) ^ (y as u32).wrapping_mul(0xd8163841) ^ seed.wrapping_mul(0xcb1ab31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1e995);
    h ^= h >> 15;
    (h & 0xffffff) as f32 / 0xffffff as f32 * 2.0 - 1.0
}

/// Smooth value noise at integer-lattice frequency 1.
#[inline]
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let xi = x.floor();
    let yi = y.floor();
    let fx = x - xi;
    let fy = y - yi;
    let (xi, yi) = (xi as i32, yi as i32);
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let a = hash2(xi, yi, seed);
    let b = hash2(xi + 1, yi, seed);
    let c = hash2(xi, yi + 1, seed);
    let d = hash2(xi + 1, yi + 1, seed);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

/// Film grain. `full_xy` gives full-resolution crop-frame pixel coords and
/// `footprint` how many full-res pixels each output pixel covers.
pub fn grain(img: &mut Image3, g: &Grain, full_xy: &(dyn Fn(usize) -> (f32, f32) + Sync), footprint: f32) {
    if g.amount <= 0.0 {
        return;
    }
    let size = 0.6 + g.size / 100.0 * 4.0; // grain cell in full-res px
    let rough = g.roughness / 100.0;
    // averaging over the footprint reduces visible amplitude
    let atten = (size / footprint.max(size)).min(1.0);
    let amp = g.amount / 100.0 * 0.11 * atten;
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let (x, y) = full_xy(i);
        let fx = x / size;
        let fy = y / size;
        let mut n = value_noise(fx, fy, 1) * 0.7;
        n += value_noise(fx * 2.1 + 17.0, fy * 2.1 + 5.0, 2) * 0.3 * (0.3 + rough);
        n += value_noise(fx * 4.3 + 3.0, fy * 4.3 + 11.0, 3) * 0.25 * rough;
        let y0 = luma_prophoto(*p).max(1e-7);
        let e = enc(y0);
        // strongest in the midtones, like film
        let wt = 0.35 + 0.65 * (4.0 * e * (1.0 - e)).clamp(0.0, 1.0);
        let e2 = (e + n * amp * wt).max(0.0);
        let r = dec(e2) / y0;
        *p = p.map(|c| c * r);
    });
}

/// Manual lens vignetting correction in scene-linear space; `tco` are T-frame
/// coords, `aspect` = w/h of the O frame.
pub fn lens_vignette(img: &mut Image3, lc: &LensCorrection, tco: &[[f32; 2]], aspect: f32) {
    if lc.vignette_amount == 0.0 {
        return;
    }
    let a = lc.vignette_amount / 100.0;
    let mid = lc.vignette_midpoint / 100.0;
    let (sx, sy) = if aspect >= 1.0 { (1.0, 1.0 / aspect) } else { (aspect, 1.0) };
    let norm = (sx * sx + sy * sy).sqrt();
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let t = tco[i];
        let x = (t[0] - 0.5) * 2.0 * sx;
        let y = (t[1] - 0.5) * 2.0 * sy;
        let r = (x * x + y * y).sqrt() / norm;
        let w = smoothstep(mid * 0.8, 1.05, r);
        // positive brightens corners (corrects falloff) by up to ~1.5 EV
        let k = (a * 1.5 * w).exp2();
        *p = p.map(|c| c * k);
    });
}

/// Simple defringe: desaturate purple/green fringes next to strong edges.
pub fn defringe(img: &mut Image3, lc: &LensCorrection, fields: &LocalFields, ok: &Oklab) {
    let local = fields.has(LP::Defringe);
    if lc.defringe_purple_amount <= 0.0 && lc.defringe_green_amount <= 0.0 && !local {
        return;
    }
    let (w, h) = (img.w, img.h);
    let y = Plane { w, h, data: img.data.par_iter().map(|p| enc(luma_prophoto(*p).max(0.0))).collect() };
    // local max contrast in a 5x5 window marks edges
    let edge = Plane::from_fn(w, h, |x, yy| {
        let mut mn = f32::MAX;
        let mut mx = f32::MIN;
        for dy in -2..=2 {
            for dx in -2..=2 {
                let v = y.get_clamped(x as isize + dx, yy as isize + dy);
                mn = mn.min(v);
                mx = mx.max(v);
            }
        }
        smoothstep(0.08, 0.3, mx - mn)
    });
    // ACR hue sliders are 0..100 over each family's span
    let purple = (lc.defringe_purple_hue[0] / 100.0 * 110.0 + 250.0, lc.defringe_purple_hue[1] / 100.0 * 110.0 + 250.0);
    let green = (lc.defringe_green_hue[0] / 100.0 * 100.0 + 90.0, lc.defringe_green_hue[1] / 100.0 * 100.0 + 90.0);
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let e = edge.data[i];
        if e <= 0.0 {
            return;
        }
        let mut lch = lab_to_lch(ok.from_prophoto(*p));
        let in_range = |h: f32, r: (f32, f32)| {
            let c = (r.0 + r.1) * 0.5;
            let hw = (r.1 - r.0).abs() * 0.5 + 5.0;
            1.0 - smoothstep(hw * 0.7, hw, hue_diff(h, c).abs())
        };
        let local_amt = fields.get(LP::Defringe, i) / 100.0;
        let wp = in_range(lch[2], purple) * (lc.defringe_purple_amount / 20.0 + local_amt.max(0.0)).min(1.0);
        let wg = in_range(lch[2], green) * (lc.defringe_green_amount / 20.0 + local_amt.max(0.0)).min(1.0);
        let k = (wp.max(wg) * e).min(1.0);
        if k > 0.0 {
            lch[1] *= 1.0 - k;
            *p = ok.to_prophoto(lch_to_lab(lch)).map(|v| v.max(0.0));
        }
    });
}

/// Depth-driven lens blur in the display domain. `depth` gives 0 (near) ..
/// 1 (far) per pixel; `max_r` is the largest blur radius in output pixels.
pub fn lens_blur(img: &mut Image3, depth: &[f32], lb: &crate::params::LensBlur, max_r: f32) {
    use crate::image::{Plane, gaussian_blur3};
    if !lb.enabled || lb.amount <= 0.0 || max_r < 0.5 {
        return;
    }
    let (near, far) = (lb.focal_range[0].min(lb.focal_range[1]), lb.focal_range[0].max(lb.focal_range[1]));
    // blur radius per pixel: distance outside the focal band
    let rad: Vec<f32> = depth
        .par_iter()
        .map(|&d| {
            let off = if d < near {
                near - d
            } else if d > far {
                d - far
            } else {
                0.0
            };
            (off * 2.5).min(1.0) * max_r
        })
        .collect();
    // bright highlights bloom into bokeh balls
    let boost = 1.0 + lb.boost / 100.0 * 4.0;
    let pre: Image3 = img.map(|p| {
        let y = luma_prophoto(p);
        let k = if y > 0.7 { 1.0 + (boost - 1.0) * smoothstep(0.7, 1.0, y) } else { 1.0 };
        p.map(|c| c * k)
    });
    let levels = 6;
    let mut layers = vec![pre.clone()];
    for l in 1..=levels {
        let r = max_r * l as f32 / levels as f32;
        let sigma = match lb.shape {
            crate::params::BokehShape::Ring | crate::params::BokehShape::Bubble => r * 0.42,
            _ => r * 0.5,
        };
        let mut b = gaussian_blur3(&pre, sigma.max(0.3));
        if matches!(lb.shape, crate::params::BokehShape::Bubble | crate::params::BokehShape::Ring) {
            // emphasise the rim: difference of two blurs sharpens disc edges
            let inner = gaussian_blur3(&pre, (sigma * 0.5).max(0.3));
            b.data.par_iter_mut().zip(inner.data.par_iter()).for_each(|(o, i)| {
                for c in 0..3 {
                    o[c] = (o[c] * 1.25 - i[c] * 0.25).max(0.0);
                }
            });
        }
        layers.push(b);
    }
    let _ = Plane::new(0, 0);
    let amt = (lb.amount / 100.0).min(1.0);
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let r = rad[i] * amt;
        if r <= 0.01 {
            return;
        }
        let f = (r / max_r * levels as f32).min(levels as f32);
        let l0 = f.floor() as usize;
        let l1 = (l0 + 1).min(levels);
        let t = f - l0 as f32;
        let a = layers[l0].data[i];
        let b = layers[l1].data[i];
        let y_orig = luma_prophoto(*p).max(1e-6);
        let mix = [0, 1, 2].map(|c| a[c] * (1.0 - t) + b[c] * t);
        // undo the boost on the in-focus share
        let k = if y_orig > 0.7 { 1.0 + (boost - 1.0) * smoothstep(0.7, 1.0, y_orig) } else { 1.0 };
        let w = (r / max_r).min(1.0);
        *p = [0, 1, 2].map(|c| p[c] * (1.0 - w) + (mix[c] / (1.0 + (k - 1.0) * (1.0 - w))) * w);
    });
}
