//! Enhance: Denoise (non-local means), Raw Details (capture sharpening by
//! Richardson–Lucy deconvolution) and Super Resolution helpers.

use rayon::prelude::*;

use crate::color::Oklab;
use crate::image::{Image3, Plane, box_blur, gaussian_blur, guided_filter};

/// Robust noise sigma of a plane (MAD of a Laplacian-like high pass).
pub fn estimate_noise(p: &Plane) -> f32 {
    let (w, h) = (p.w, p.h);
    let step = ((w * h) / 200_000).max(1);
    let mut v: Vec<f32> = (0..w * h)
        .step_by(step)
        .filter_map(|i| {
            let (x, y) = (i % w, i / w);
            if x < 1 || y < 1 || x + 1 >= w || y + 1 >= h {
                return None;
            }
            let c = p.data[i];
            let lap = 4.0 * c - p.data[i - 1] - p.data[i + 1] - p.data[i - w] - p.data[i + w];
            Some(lap.abs())
        })
        .collect();
    if v.is_empty() {
        return 0.0;
    }
    let mid = v.len() / 2;
    v.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap());
    // Laplacian of white noise has sigma * sqrt(20)
    v[mid] / 0.6745 / 20f32.sqrt()
}

/// Fast non-local means: for each search offset, the patch distance is a
/// box-filtered squared difference image.
pub fn nlm(p: &Plane, h_param: f32, search: isize, patch: usize) -> Plane {
    let (w, hh) = (p.w, p.h);
    let mut acc = vec![0f32; w * hh];
    let mut wsum = vec![0f32; w * hh];
    let h2 = (h_param * h_param).max(1e-12);
    let offsets: Vec<(isize, isize)> = (-search..=search).flat_map(|dy| (-search..=search).map(move |dx| (dx, dy))).collect();
    // process offsets in parallel chunks, then reduce
    let partial: Vec<(Vec<f32>, Vec<f32>)> = offsets
        .par_chunks(offsets.len().div_ceil(rayon::current_num_threads()).max(1))
        .map(|chunk| {
            let mut a = vec![0f32; w * hh];
            let mut s = vec![0f32; w * hh];
            for &(dx, dy) in chunk {
                let diff = Plane::from_fn(w, hh, |x, y| {
                    let q = p.get_clamped(x as isize + dx, y as isize + dy);
                    let d = p.data[y * w + x] - q;
                    d * d
                });
                let dist = box_blur(&diff, patch);
                for y in 0..hh {
                    for x in 0..w {
                        let i = y * w + x;
                        let wt = (-(dist.data[i] / h2)).exp();
                        a[i] += wt * p.get_clamped(x as isize + dx, y as isize + dy);
                        s[i] += wt;
                    }
                }
            }
            (a, s)
        })
        .collect();
    for (a, s) in partial {
        acc.iter_mut().zip(a).for_each(|(x, y)| *x += y);
        wsum.iter_mut().zip(s).for_each(|(x, y)| *x += y);
    }
    Plane { w, h: hh, data: acc.iter().zip(wsum.iter()).map(|(a, s)| a / s.max(1e-12)).collect() }
}

/// Strong denoise on camera-space linear RGB. `amount` 0..100.
pub fn denoise(img: &Image3, amount: f32) -> Image3 {
    let ok = Oklab::new();
    let (w, h) = (img.w, img.h);
    let lab: Vec<[f32; 3]> = img.data.par_iter().map(|p| ok.from_prophoto(*p)).collect();
    let l = Plane { w, h, data: lab.iter().map(|v| v[0]).collect() };
    let a = Plane { w, h, data: lab.iter().map(|v| v[1]).collect() };
    let b = Plane { w, h, data: lab.iter().map(|v| v[2]).collect() };
    let sigma = estimate_noise(&l).max(1e-4);
    let k = amount / 100.0;
    let lf = nlm(&l, sigma * (0.6 + 1.6 * k), 3, 1);
    // keep a little grain-like texture at low amounts
    let l2 = l.zip_map(&lf, |o, f| o + (f - o) * (0.5 + 0.5 * k));
    let r = ((w.max(h) as f32 / 600.0) * (1.0 + 2.0 * k)).max(2.0) as usize;
    let eps = (sigma * 2.0).powi(2);
    let a2 = guided_filter(&l2, &a, r, eps);
    let b2 = guided_filter(&l2, &b, r, eps);
    Image3 { w, h, data: (0..w * h).into_par_iter().map(|i| ok.to_prophoto([l2.data[i], a2.data[i], b2.data[i]]).map(|v| v.max(0.0))).collect() }
}

/// Capture sharpening: Richardson–Lucy deconvolution of luminance with a
/// gaussian PSF, applied as a luminance ratio.
pub fn raw_details(img: &Image3, sigma: f32, iterations: usize) -> Image3 {
    let y = img.luminance().map(|v| v.max(1e-6));
    let mut est = y.clone();
    for _ in 0..iterations {
        let conv = gaussian_blur(&est, sigma);
        let ratio = y.zip_map(&conv, |o, c| o / c.max(1e-6));
        let corr = gaussian_blur(&ratio, sigma);
        est = est.zip_map(&corr, |e, c| (e * c).clamp(0.0, 64.0));
    }
    // limit the boost so noise and edges don't explode
    Image3 {
        w: img.w,
        h: img.h,
        data: img
            .data
            .par_iter()
            .enumerate()
            .map(|(i, p)| {
                let r = (est.data[i] / y.data[i]).clamp(0.6, 1.6);
                p.map(|c| c * r)
            })
            .collect(),
    }
}

/// Post-upscale detail restoration for Super Resolution renders.
pub fn super_res_refine(img: &mut Image3) {
    let y = img.luminance().map(|v| v.max(0.0).sqrt());
    let b = gaussian_blur(&y, 1.2);
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let d = (y.data[i] - b.data[i]).clamp(-0.05, 0.05);
        let n = (y.data[i] + d * 0.9).max(0.0);
        let r = (n * n) / (y.data[i] * y.data[i]).max(1e-8);
        *p = p.map(|c| c * r.clamp(0.5, 2.0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nlm_reduces_noise() {
        let mut seed = 1u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let p = Plane::from_fn(64, 64, |x, _| if x < 32 { 0.3 } else { 0.7 });
        let noisy = Plane { w: 64, h: 64, data: p.data.iter().map(|v| v + rnd() * 0.1).collect() };
        let s = estimate_noise(&noisy);
        assert!(s > 0.01 && s < 0.06, "{s}");
        let d = nlm(&noisy, s * 1.5, 3, 1);
        let err = |q: &Plane| q.data.iter().zip(p.data.iter()).map(|(a, b)| (a - b).powi(2)).sum::<f32>();
        assert!(err(&d) < err(&noisy) * 0.5);
    }
}
