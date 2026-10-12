//! Demosaicing. All functions take the normalised CFA plus white-balance
//! multipliers and return white-balanced camera RGB.

use rayon::prelude::*;

use crate::decode::CfaImage;
use crate::image::{Image3, Plane, Rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Half (Bayer) or third (X-Trans) resolution superpixels. Fast previews.
    Draft,
    /// Full resolution, gradient-corrected interpolation.
    Full,
}

const CLIP: f32 = 0.995;

/// Returns the demosaiced image and a per-pixel clipped fraction (0..1).
pub fn demosaic(cfa: &CfaImage, mul: Rgb, quality: Quality) -> (Image3, Plane) {
    let (img, clip) = match (quality, cfa.pattern.is_bayer()) {
        (Quality::Draft, _) => superpixel(cfa, mul),
        (Quality::Full, true) => (rcd_bayer(cfa, mul), clip_plane(cfa)),
        (Quality::Full, false) => (markesteijn(cfa, mul).unwrap_or_else(|| color_difference_generic(cfa, mul)), clip_plane(cfa)),
    };
    (img, clip)
}

/// Any clipped sample in the 3x3 neighbourhood marks the pixel, since every
/// output pixel draws on its neighbours' samples.
fn clip_plane(cfa: &CfaImage) -> Plane {
    let raw = Plane { w: cfa.w, h: cfa.h, data: cfa.data.par_iter().map(|&v| if v >= CLIP { 1.0 } else { 0.0 }).collect() };
    Plane::from_fn(cfa.w, cfa.h, |x, y| {
        let mut m: f32 = 0.0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                m = m.max(raw.get_clamped(x as isize + dx, y as isize + dy));
            }
        }
        m
    })
}

/// Block-average each pattern tile into one pixel.
fn superpixel(cfa: &CfaImage, mul: Rgb) -> (Image3, Plane) {
    let pat = &cfa.pattern;
    // Bayer: 2x2 tiles; X-Trans: 3x3 tiles (each holds all three colours)
    let t = if pat.is_bayer() { 2 } else { 3 };
    let (w, h) = (cfa.w / t, cfa.h / t);
    let mut clip = Plane::new(w, h);
    let mut img = Image3::new(w, h);
    img.data.par_chunks_mut(w).zip(clip.data.par_chunks_mut(w)).enumerate().for_each(|(y, (row, crow))| {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            let mut n = [0u32; 3];
            let mut mx = [0.0f32; 3];
            let mut clipped = 0u32;
            for dy in 0..t {
                for dx in 0..t {
                    let sx = x * t + dx;
                    let sy = y * t + dy;
                    let c = pat.color_at(sx, sy);
                    let v = cfa.data[sy * cfa.w + sx];
                    if v >= CLIP {
                        clipped += 1;
                    }
                    mx[c] = mx[c].max(v);
                    acc[c] += v;
                    n[c] += 1;
                }
            }
            for c in 0..3 {
                // a clipped sample makes the whole colour unknown: keep it at the clip level
                let v = if mx[c] >= CLIP {
                    mx[c]
                } else if n[c] > 0 {
                    acc[c] / n[c] as f32
                } else {
                    0.0
                };
                row[x][c] = v * mul[c];
            }
            crow[x] = (clipped as f32 / (t * t) as f32 * 2.0).min(1.0);
        }
    });
    (img, clip)
}

/// Malvar-He-Cutler gradient-corrected bilinear interpolation (Bayer).
fn mhc_bayer(cfa: &CfaImage, mul: Rgb) -> Image3 {
    let (w, h) = (cfa.w, cfa.h);
    let pat = &cfa.pattern;
    let at = |x: isize, y: isize| -> f32 {
        let x = x.clamp(0, w as isize - 1) as usize;
        let y = y.clamp(0, h as isize - 1) as usize;
        // mirror to same-colour neighbours at borders would be nicer; clamping is fine for 2px
        cfa.data[y * w + x] * mul[pat.color_at(x, y)]
    };
    Image3::from_fn(w, h, |x, y| {
        let (xi, yi) = (x as isize, y as isize);
        let c = pat.color_at(x, y);
        let v = at(xi, yi);
        let p = |dx: isize, dy: isize| at(xi + dx, yi + dy);
        // G at R/B
        let g_at_rb = (4.0 * v + 2.0 * (p(0, -1) + p(-1, 0) + p(1, 0) + p(0, 1)) - (p(0, -2) + p(-2, 0) + p(2, 0) + p(0, 2))) / 8.0;
        // R/B at G in R row (horizontal neighbours are the colour we want)
        let rb_g_row =
            (5.0 * v + 4.0 * (p(-1, 0) + p(1, 0)) - (p(-2, 0) + p(2, 0) + p(-1, -1) + p(1, -1) + p(-1, 1) + p(1, 1)) + 0.5 * (p(0, -2) + p(0, 2))) / 8.0;
        let rb_g_col =
            (5.0 * v + 4.0 * (p(0, -1) + p(0, 1)) - (p(0, -2) + p(0, 2) + p(-1, -1) + p(1, -1) + p(-1, 1) + p(1, 1)) + 0.5 * (p(-2, 0) + p(2, 0))) / 8.0;
        // B at R / R at B
        let rb_diag = (6.0 * v + 2.0 * (p(-1, -1) + p(1, -1) + p(-1, 1) + p(1, 1)) - 1.5 * (p(0, -2) + p(-2, 0) + p(2, 0) + p(0, 2))) / 8.0;
        let out = match c {
            1 => {
                // which colour is in this row?
                let row_c = pat.color_at(x + 1, y);
                if row_c == 0 { [rb_g_row, v, rb_g_col] } else { [rb_g_col, v, rb_g_row] }
            }
            0 => [v, g_at_rb, rb_diag],
            _ => [rb_diag, g_at_rb, v],
        };
        [out[0].max(0.0), out[1].max(0.0), out[2].max(0.0)]
    })
}

/// X-Trans via rawler's Markesteijn implementation.
fn markesteijn(cfa: &CfaImage, mul: Rgb) -> Option<Image3> {
    use rawler::imgop::sensor::Demosaic;
    if !cfa.pattern.is_xtrans() {
        return None;
    }
    let name: String = cfa.pattern.colors.iter().map(|c| ['R', 'G', 'B'][*c as usize]).collect();
    let rcfa = rawler::CFA::new(&name);
    let data: Vec<f32> = cfa.data.par_iter().enumerate().map(|(i, v)| v * mul[cfa.pattern.color_at(i % cfa.w, i / cfa.w)]).collect();
    let pix = rawler::pixarray::Pix2D::new_with(data, cfa.w, cfa.h);
    let roi = rawler::imgop::Rect::new(rawler::imgop::Point { x: 0, y: 0 }, rawler::imgop::Dim2 { w: cfa.w, h: cfa.h });
    let out =
        rawler::imgop::sensor::xtrans::markesteijn::XTransMarkesteijnDemosaic::new_pass_1().demosaic(&pix, &rcfa, &rawler::cfa::PlaneColor::new("RGB"), roi);
    if out.width != cfa.w || out.data.len() != cfa.w * cfa.h {
        return None;
    }
    Some(Image3 { w: cfa.w, h: cfa.h, data: out.data.into_iter().map(|p| [p[0].max(0.0), p[1].max(0.0), p[2].max(0.0)]).collect() })
}

/// RCD (Ratio Corrected Demosaicing, L. Sanz Rodríguez) for Bayer sensors.
/// Borders fall back to MHC.
fn rcd_bayer(cfa: &CfaImage, mul: Rgb) -> Image3 {
    let (w, h) = (cfa.w, cfa.h);
    if w < 16 || h < 16 {
        return mhc_bayer(cfa, mul);
    }
    let pat = &cfa.pattern;
    let fc = |x: usize, y: usize| pat.color_at(x, y);
    let c: Vec<f32> = cfa.data.par_iter().enumerate().map(|(i, v)| (v * mul[fc(i % w, i / w)]).max(0.0)).collect();
    let eps = 1e-5f32;
    let sq = |v: f32| v * v;
    let (w1, w2, w3, w4) = (w, 2 * w, 3 * w, 4 * w);
    let border = 4;

    // step 1: vertical / horizontal discrimination
    let mut bv = vec![0f32; w * h];
    let mut bh = vec![0f32; w * h];
    bv.par_chunks_mut(w).zip(bh.par_chunks_mut(w)).enumerate().for_each(|(y, (rv, rh))| {
        if y < 3 || y >= h - 3 {
            return;
        }
        for x in 3..w - 3 {
            let i = y * w + x;
            rv[x] = sq((c[i - w3] - c[i - w1] - c[i + w1] + c[i + w3]) - 3.0 * (c[i - w2] + c[i + w2]) + 6.0 * c[i]);
            rh[x] = sq((c[i - 3] - c[i - 1] - c[i + 1] + c[i + 3]) - 3.0 * (c[i - 2] + c[i + 2]) + 6.0 * c[i]);
        }
    });
    let mut vh = vec![0.5f32; w * h];
    vh.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        if y < border || y >= h - border {
            return;
        }
        for x in border..w - border {
            let i = y * w + x;
            let vs = (bv[i - w1] + bv[i] + bv[i + w1]).max(eps);
            let hs = (bh[i - 1] + bh[i] + bh[i + 1]).max(eps);
            row[x] = vs / (vs + hs);
        }
    });
    drop(bv);
    drop(bh);

    // step 2: low pass filter at R/B sites
    let mut lpf = vec![0f32; w * h];
    lpf.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        if y < 2 || y >= h - 2 {
            return;
        }
        for x in 2..w - 2 {
            if fc(x, y) == 1 {
                continue;
            }
            let i = y * w + x;
            row[x] = c[i] + 0.5 * (c[i - w1] + c[i + w1] + c[i - 1] + c[i + 1]) + 0.25 * (c[i - w1 - 1] + c[i - w1 + 1] + c[i + w1 - 1] + c[i + w1 + 1]);
        }
    });

    let base = mhc_bayer(cfa, mul);
    let mut r = vec![0f32; w * h];
    let mut g = vec![0f32; w * h];
    let mut b = vec![0f32; w * h];
    for i in 0..w * h {
        let (x, y) = (i % w, i / w);
        let col = fc(x, y);
        let bp = base.data[i];
        r[i] = if col == 0 { c[i] } else { bp[0] };
        g[i] = if col == 1 { c[i] } else { bp[1] };
        b[i] = if col == 2 { c[i] } else { bp[2] };
    }

    let disc = |d: &[f32], i: usize| -> f32 {
        let central = d[i];
        let neigh = 0.25 * (d[i - w1 - 1] + d[i - w1 + 1] + d[i + w1 - 1] + d[i + w1 + 1]);
        if (0.5 - central).abs() < (0.5 - neigh).abs() { neigh } else { central }
    };

    // step 3: green at R/B
    g.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        if y < border || y >= h - border {
            return;
        }
        for x in border..w - border {
            if fc(x, y) == 1 {
                continue;
            }
            let i = y * w + x;
            let vd = disc(&vh, i);
            let n_grad = eps + (c[i - w1] - c[i + w1]).abs() + (c[i] - c[i - w2]).abs() + (c[i - w1] - c[i - w3]).abs() + (c[i - w2] - c[i - w4]).abs();
            let s_grad = eps + (c[i - w1] - c[i + w1]).abs() + (c[i] - c[i + w2]).abs() + (c[i + w1] - c[i + w3]).abs() + (c[i + w2] - c[i + w4]).abs();
            let w_grad = eps + (c[i - 1] - c[i + 1]).abs() + (c[i] - c[i - 2]).abs() + (c[i - 1] - c[i - 3]).abs() + (c[i - 2] - c[i - 4]).abs();
            let e_grad = eps + (c[i - 1] - c[i + 1]).abs() + (c[i] - c[i + 2]).abs() + (c[i + 1] - c[i + 3]).abs() + (c[i + 2] - c[i + 4]).abs();
            let n_est = c[i - w1] * 2.0 * lpf[i] / (eps + lpf[i] + lpf[i - w2]);
            let s_est = c[i + w1] * 2.0 * lpf[i] / (eps + lpf[i] + lpf[i + w2]);
            let w_est = c[i - 1] * 2.0 * lpf[i] / (eps + lpf[i] + lpf[i - 2]);
            let e_est = c[i + 1] * 2.0 * lpf[i] / (eps + lpf[i] + lpf[i + 2]);
            let v_est = (s_grad * n_est + n_grad * s_est) / (n_grad + s_grad);
            let h_est = (w_grad * e_est + e_grad * w_est) / (e_grad + w_grad);
            row[x] = (vd * h_est + (1.0 - vd) * v_est).max(0.0);
        }
    });
    drop(lpf);

    // step 4.1/4.2: diagonal discrimination
    let mut bp = vec![0f32; w * h];
    let mut bq = vec![0f32; w * h];
    bp.par_chunks_mut(w).zip(bq.par_chunks_mut(w)).enumerate().for_each(|(y, (rp, rq))| {
        if y < 3 || y >= h - 3 {
            return;
        }
        for x in 3..w - 3 {
            let i = y * w + x;
            rp[x] = sq((c[i - w3 - 3] - c[i - w1 - 1] - c[i + w1 + 1] + c[i + w3 + 3]) - 3.0 * (c[i - w2 - 2] + c[i + w2 + 2]) + 6.0 * c[i]);
            rq[x] = sq((c[i - w3 + 3] - c[i - w1 + 1] - c[i + w1 - 1] + c[i + w3 - 3]) - 3.0 * (c[i - w2 + 2] + c[i + w2 - 2]) + 6.0 * c[i]);
        }
    });
    let mut pq = vec![0.5f32; w * h];
    pq.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        if y < border || y >= h - border {
            return;
        }
        for x in border..w - border {
            let i = y * w + x;
            let ps = (bp[i - w1 - 1] + bp[i] + bp[i + w1 + 1]).max(eps);
            let qs = (bq[i - w1 + 1] + bq[i] + bq[i + w1 - 1]).max(eps);
            row[x] = ps / (ps + qs);
        }
    });
    drop(bp);
    drop(bq);

    // step 4.3: R at B and B at R
    let pass43 = |target: &mut Vec<f32>, src_color: usize, at_color: usize, chan: &Vec<f32>| {
        let gref = &g;
        target.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            if y < border || y >= h - border {
                return;
            }
            for x in border..w - border {
                if fc(x, y) != at_color {
                    continue;
                }
                let i = y * w + x;
                let pd = disc(&pq, i);
                let cc = chan;
                let nw_g = eps + (cc[i - w1 - 1] - cc[i + w1 + 1]).abs() + (cc[i - w1 - 1] - cc[i - w3 - 3]).abs() + (gref[i] - gref[i - w2 - 2]).abs();
                let ne_g = eps + (cc[i - w1 + 1] - cc[i + w1 - 1]).abs() + (cc[i - w1 + 1] - cc[i - w3 + 3]).abs() + (gref[i] - gref[i - w2 + 2]).abs();
                let sw_g = eps + (cc[i + w1 - 1] - cc[i - w1 + 1]).abs() + (cc[i + w1 - 1] - cc[i + w3 - 3]).abs() + (gref[i] - gref[i + w2 - 2]).abs();
                let se_g = eps + (cc[i + w1 + 1] - cc[i - w1 - 1]).abs() + (cc[i + w1 + 1] - cc[i + w3 + 3]).abs() + (gref[i] - gref[i + w2 + 2]).abs();
                let nw_e = cc[i - w1 - 1] - gref[i - w1 - 1];
                let ne_e = cc[i - w1 + 1] - gref[i - w1 + 1];
                let sw_e = cc[i + w1 - 1] - gref[i + w1 - 1];
                let se_e = cc[i + w1 + 1] - gref[i + w1 + 1];
                let p_est = (nw_g * se_e + se_g * nw_e) / (nw_g + se_g);
                let q_est = (ne_g * sw_e + sw_g * ne_e) / (ne_g + sw_g);
                row[x] = (gref[i] + (1.0 - pd) * p_est + pd * q_est).max(0.0);
            }
        });
        let _ = src_color;
    };
    let rsrc = r.clone();
    pass43(&mut r, 0, 2, &rsrc);
    let bsrc = b.clone();
    pass43(&mut b, 2, 0, &bsrc);

    // step 4.4: R and B at G sites
    let pass44 = |target: &mut Vec<f32>, chan: &Vec<f32>| {
        let gref = &g;
        target.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            if y < border || y >= h - border {
                return;
            }
            for x in border..w - border {
                if fc(x, y) != 1 {
                    continue;
                }
                let i = y * w + x;
                let vd = disc(&vh, i);
                let cc = chan;
                let n_g = eps + (gref[i] - gref[i - w2]).abs() + (cc[i - w1] - cc[i + w1]).abs() + (cc[i - w1] - cc[i - w3]).abs();
                let s_g = eps + (gref[i] - gref[i + w2]).abs() + (cc[i + w1] - cc[i - w1]).abs() + (cc[i + w1] - cc[i + w3]).abs();
                let w_g = eps + (gref[i] - gref[i - 2]).abs() + (cc[i - 1] - cc[i + 1]).abs() + (cc[i - 1] - cc[i - 3]).abs();
                let e_g = eps + (gref[i] - gref[i + 2]).abs() + (cc[i + 1] - cc[i - 1]).abs() + (cc[i + 1] - cc[i + 3]).abs();
                let n_e = cc[i - w1] - gref[i - w1];
                let s_e = cc[i + w1] - gref[i + w1];
                let w_e = cc[i - 1] - gref[i - 1];
                let e_e = cc[i + 1] - gref[i + 1];
                let v_est = (n_g * s_e + s_g * n_e) / (n_g + s_g);
                let h_est = (e_g * w_e + w_g * e_e) / (e_g + w_g);
                row[x] = (gref[i] + (1.0 - vd) * v_est + vd * h_est).max(0.0);
            }
        });
    };
    let rsrc = r.clone();
    pass44(&mut r, &rsrc);
    let bsrc = b.clone();
    pass44(&mut b, &bsrc);

    Image3 { w, h, data: (0..w * h).into_par_iter().map(|i| [r[i], g[i], b[i]]).collect() }
}

/// Green interpolation by weighted neighbour average, then R/B via colour
/// differences. Works for any pattern (used for X-Trans).
fn color_difference_generic(cfa: &CfaImage, mul: Rgb) -> Image3 {
    let (w, h) = (cfa.w, cfa.h);
    let pat = &cfa.pattern;
    let val = |x: usize, y: usize| cfa.data[y * w + x] * mul[pat.color_at(x, y)];
    // pass 1: green everywhere
    let green = Plane::from_fn(w, h, |x, y| {
        if pat.color_at(x, y) == 1 {
            return val(x, y);
        }
        let mut acc = 0.0;
        let mut wsum = 0.0;
        let c = val(x, y);
        for dy in -2isize..=2 {
            for dx in -2isize..=2 {
                let sx = x as isize + dx;
                let sy = y as isize + dy;
                if sx < 0 || sy < 0 || sx >= w as isize || sy >= h as isize {
                    continue;
                }
                let (sx, sy) = (sx as usize, sy as usize);
                if pat.color_at(sx, sy) != 1 {
                    continue;
                }
                let d2 = (dx * dx + dy * dy) as f32;
                let gv = val(sx, sy);
                // edge-aware: penalise neighbours across strong gradients
                let wgt = 1.0 / (d2 * (1.0 + 20.0 * (gv - c).abs()));
                acc += gv * wgt;
                wsum += wgt;
            }
        }
        if wsum > 0.0 { acc / wsum } else { c }
    });
    // pass 2: colour differences
    let img = Image3::from_fn(w, h, |x, y| {
        let c = pat.color_at(x, y);
        let g = green.get(x, y);
        let mut out = [0.0f32; 3];
        out[1] = g;
        for want in [0usize, 2] {
            if want == c {
                out[want] = val(x, y);
                continue;
            }
            let mut acc = 0.0;
            let mut wsum = 0.0;
            for dy in -2isize..=2 {
                for dx in -2isize..=2 {
                    let sx = x as isize + dx;
                    let sy = y as isize + dy;
                    if sx < 0 || sy < 0 || sx >= w as isize || sy >= h as isize {
                        continue;
                    }
                    let (sx, sy) = (sx as usize, sy as usize);
                    if pat.color_at(sx, sy) != want {
                        continue;
                    }
                    let d2 = (dx * dx + dy * dy) as f32;
                    let wgt = 1.0 / d2;
                    acc += (val(sx, sy) - green.get(sx, sy)) * wgt;
                    wsum += wgt;
                }
            }
            out[want] = if wsum > 0.0 { g + acc / wsum } else { g };
        }
        [out[0].max(0.0), out[1].max(0.0), out[2].max(0.0)]
    });
    img
}

/// Per-colour saturation level actually reached in the data (normalised).
pub fn cfa_clip_levels(cfa: &CfaImage) -> Rgb {
    let mut mx = [0.0f32; 3];
    // sample every 3rd row; saturated areas are never that small
    for y in (0..cfa.h).step_by(3) {
        for x in 0..cfa.w {
            let c = cfa.pattern.color_at(x, y);
            mx[c] = mx[c].max(cfa.data[y * cfa.w + x]);
        }
    }
    // only trust a max as a clip level if it's near the nominal white
    mx.map(|m| if m >= 0.95 { m } else { f32::MAX })
}

/// Highlight reconstruction in white-balanced camera space. Channels at their
/// clip level are rebuilt from the unclipped ones using colour ratios of
/// nearby bright pixels; pixels clipped in every channel become neutral.
pub fn reconstruct_highlights(img: &mut Image3, clip: &Plane, mul: Rgb, raw_clip: Rgb) {
    if !clip.data.iter().any(|&c| c > 0.0) {
        return;
    }
    let (w, h) = (img.w, img.h);
    let lvl = [0, 1, 2].map(|c| if raw_clip[c] == f32::MAX { f32::MAX } else { raw_clip[c] * mul[c] * 0.97 });
    let is_clipped = |p: &Rgb, c: usize, i: usize| p[c] >= lvl[c] || (clip.data[i] > 0.0 && p[c] >= lvl[c] * 0.8);
    // ratio fields from bright unclipped pixels
    let sigma = (w.max(h) as f32 / 120.0).max(2.0);
    let lmin = lvl[0].min(lvl[1]).min(lvl[2]).min(4.0);
    // ratios near clipped samples are contaminated by interpolation
    let near_clip = crate::image::box_blur(clip, 3);
    let mut wgt = Plane::new(w, h);
    let mut rr = Plane::new(w, h);
    let mut br = Plane::new(w, h);
    wgt.data.par_iter_mut().zip(rr.data.par_iter_mut()).zip(br.data.par_iter_mut()).enumerate().for_each(|(i, ((wv, r), b))| {
        let p = img.data[i];
        if near_clip.data[i] > 0.0 || (0..3).any(|c| is_clipped(&p, c, i)) || p[1] <= 1e-4 {
            return;
        }
        // brightness weight: prefer pixels close to the clipping level
        let y = (p[0] + p[1] + p[2]) / 3.0 / lmin;
        let k = (y * y).min(1.0);
        *wv = k;
        *r = (p[0] / p[1]).clamp(0.1, 10.0) * k;
        *b = (p[2] / p[1]).clamp(0.1, 10.0) * k;
    });
    let wb = crate::image::gaussian_blur(&wgt, sigma);
    let rrb = crate::image::gaussian_blur(&rr, sigma);
    let brb = crate::image::gaussian_blur(&br, sigma);
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let cl = [is_clipped(p, 0, i), is_clipped(p, 1, i), is_clipped(p, 2, i)];
        let n = cl.iter().filter(|&&c| c).count();
        if n == 0 {
            return;
        }
        let m = p[0].max(p[1]).max(p[2]);
        if n == 3 {
            *p = [m, m, m];
            return;
        }
        let wv = wb.data[i];
        let (mut r, mut b) = if wv > 1e-4 { (rrb.data[i] / wv, brb.data[i] / wv) } else { (1.0, 1.0) };
        // little nearby evidence -> assume neutral
        let trust = (wv * 3.0).min(1.0);
        r = 1.0 + (r - 1.0) * trust;
        b = 1.0 + (b - 1.0) * trust;
        let ratio = [r, 1.0, b];
        // green estimate from every unclipped channel, never below what we saw
        let mut g = p[1];
        for c in 0..3 {
            if !cl[c] {
                g = g.max(p[c] / ratio[c]);
            }
        }
        for c in 0..3 {
            if cl[c] {
                p[c] = p[c].max(g * ratio[c]);
            }
        }
        // fade toward neutral as the pixel approaches full clipping
        let top = p[0].max(p[1]).max(p[2]);
        let t = ((n as f32) / 3.0).powi(2);
        for c in 0..3 {
            p[c] += (top - p[c]) * t * 0.5;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::CfaPattern;

    fn flat_cfa(v: [f32; 3], pat: CfaPattern, w: usize, h: usize) -> CfaImage {
        let data = (0..w * h).map(|i| v[pat.color_at(i % w, i / w)]).collect();
        CfaImage { w, h, data, pattern: pat }
    }

    #[test]
    fn flat_field_is_preserved() {
        let bayer = CfaPattern { width: 2, height: 2, colors: vec![0, 1, 1, 2] };
        let cfa = flat_cfa([0.2, 0.4, 0.3], bayer, 32, 32);
        for q in [Quality::Draft, Quality::Full] {
            let (img, _) = demosaic(&cfa, [1.0, 1.0, 1.0], q);
            let p = img.get(img.w / 2, img.h / 2);
            assert!((p[0] - 0.2).abs() < 1e-4 && (p[1] - 0.4).abs() < 1e-4 && (p[2] - 0.3).abs() < 1e-4, "{q:?} {p:?}");
        }
        let xt: Vec<u8> = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG"
            .chars()
            .map(|c| match c {
                'R' => 0,
                'G' => 1,
                _ => 2,
            })
            .collect();
        let cfa = flat_cfa([0.2, 0.4, 0.3], CfaPattern { width: 6, height: 6, colors: xt }, 36, 36);
        for q in [Quality::Draft, Quality::Full] {
            let (img, _) = demosaic(&cfa, [1.0, 1.0, 1.0], q);
            let p = img.get(img.w / 2, img.h / 2);
            assert!((p[0] - 0.2).abs() < 1e-4 && (p[1] - 0.4).abs() < 1e-4 && (p[2] - 0.3).abs() < 1e-4, "{q:?} {p:?}");
        }
    }
}
