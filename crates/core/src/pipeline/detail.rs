//! Detail: noise reduction (scene-linear) and sharpening (display domain).

use rayon::prelude::*;

use super::local::{LP, LocalFields};
use super::tone::{dec, enc, smoothstep};
use crate::color::{Oklab, luma_prophoto};
use crate::image::{Image3, Plane, gaussian_blur, guided_filter};
use crate::params::{NoiseReduction, Sharpening};

/// Luminance + colour noise reduction on scene-linear ProPhoto.
/// `scale` is output pixels per full-resolution pixel.
pub fn noise_reduction(img: &mut Image3, nr: &NoiseReduction, fields: &LocalFields, scale: f32, ok: &Oklab) {
    let local = fields.has(LP::Noise);
    let lum = nr.luminance / 100.0;
    let col = nr.color / 100.0;
    if lum <= 0.0 && col <= 0.0 && !local {
        return;
    }
    // below ~1/3 size the downscale has already averaged the noise away
    if scale < 0.34 {
        return;
    }
    let (w, h) = (img.w, img.h);
    let eff = scale.clamp(0.05, 1.0);

    // Oklab: the cube root is a decent variance stabiliser for shot noise
    let lab: Vec<[f32; 3]> = img.data.par_iter().map(|p| ok.from_prophoto(*p)).collect();
    let mut l = Plane { w, h, data: lab.iter().map(|v| v[0]).collect() };
    let mut a = Plane { w, h, data: lab.iter().map(|v| v[1]).collect() };
    let mut b = Plane { w, h, data: lab.iter().map(|v| v[2]).collect() };
    let mut changed = false;

    if lum > 0.0 || local {
        let max_strength = (lum
            + if local { fields.fields[LP::Noise as usize].as_ref().map(|f| f.data.iter().cloned().fold(0.0, f32::max) / 100.0).unwrap_or(0.0) } else { 0.0 })
        .clamp(0.0, 1.0);
        if max_strength > 0.0 {
            let detail = nr.luminance_detail / 100.0;
            let sigma = 0.004 + max_strength * 0.045 * eff.sqrt();
            let eps = (sigma * (1.4 - detail)).powi(2);
            let r1 = ((1.5 * eff).round() as usize).max(1);
            let r2 = ((4.0 * eff).round() as usize).max(1);
            let f1 = guided_filter(&l, &l, r1, eps);
            let f2 = guided_filter(&f1, &f1, r2, eps * 0.5);
            let contrast = nr.luminance_contrast / 100.0;
            let lf = &l;
            let smooth = Plane {
                w,
                h,
                data: (0..w * h)
                    .into_par_iter()
                    .map(|i| {
                        let s = lum + fields.get(LP::Noise, i) / 100.0;
                        let s = s.clamp(0.0, 1.0);
                        let filt = f2.data[i] + (lf.data[i] - f2.data[i]) * contrast * 0.35;
                        lf.data[i] + (filt - lf.data[i]) * (s / max_strength.max(1e-6)).min(1.0)
                    })
                    .collect(),
            };
            l = smooth;
            changed = true;
        }
    }

    if col > 0.0 {
        let detail = nr.color_detail / 100.0;
        let smooth = nr.color_smoothness / 100.0;
        let r = (((2.0 + col * 10.0) * (1.0 + smooth * 1.5)) * eff).round().max(1.0) as usize;
        let eps = (0.002 + (1.0 - detail) * 0.02).powi(2);
        // chroma is smooth: filter it on a subsampled grid
        let sub = if r >= 8 {
            4
        } else if r >= 3 {
            2
        } else {
            1
        };
        let fa = crate::image::fast_guided_filter(&l, &a, r, eps, sub);
        let fb = crate::image::fast_guided_filter(&l, &b, r, eps, sub);
        let k = col.min(1.0);
        a = a.zip_map(&fa, |o, f| o + (f - o) * k);
        b = b.zip_map(&fb, |o, f| o + (f - o) * k);
        changed = true;
    }

    if changed {
        img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
            *p = ok.to_prophoto([l.data[i], a.data[i], b.data[i]]).map(|v| v.max(0.0));
        });
    }
}

/// Unsharp-mask style sharpening on display-encoded luminance with halo
/// suppression (detail) and edge masking.
pub fn sharpen(img: &mut Image3, sh: &Sharpening, fields: &LocalFields, scale: f32) {
    let local = fields.has(LP::Sharpness);
    if sh.amount <= 0.0 && !local {
        return;
    }
    let sigma = sh.radius * scale.min(1.0);
    if sigma < 0.25 {
        return;
    }
    let (w, h) = (img.w, img.h);
    let e = Plane { w, h, data: img.data.par_iter().map(|p| enc(luma_prophoto(*p).max(0.0))).collect() };
    let blur = gaussian_blur(&e, sigma);
    let detail = sh.detail / 100.0;
    let mask = if sh.masking > 0.0 {
        let s = gaussian_blur(&e, 1.0 * scale.max(0.5));
        let t = sh.masking / 100.0 * 0.06;
        Some(Plane::from_fn(w, h, |x, y| {
            let gx = s.get_clamped(x as isize + 1, y as isize) - s.get_clamped(x as isize - 1, y as isize);
            let gy = s.get_clamped(x as isize, y as isize + 1) - s.get_clamped(x as isize, y as isize - 1);
            let g = (gx * gx + gy * gy).sqrt();
            smoothstep(t * 0.5, t * 1.5 + 1e-4, g)
        }))
    } else {
        None
    };
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let amt = (sh.amount + fields.get(LP::Sharpness, i) * 1.5) / 100.0 * 1.6;
        if amt == 0.0 {
            return;
        }
        let d = e.data[i] - blur.data[i];
        // low detail damps large (halo-prone) differences
        let damp = detail + (1.0 - detail) * (-d.abs() / 0.04).exp();
        let mut dv = d * damp;
        if let Some(m) = &mask {
            dv *= m.data[i];
        }
        let e2 = (e.data[i] + amt * dv).max(0.0);
        let y0 = dec(e.data[i]).max(1e-6);
        let y1 = dec(e2);
        let r = y1 / y0;
        *p = [p[0] * r, p[1] * r, p[2] * r];
    });
}
