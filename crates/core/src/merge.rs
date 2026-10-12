//! Merge to HDR and Merge to Panorama. Results are linear camera-space float
//! images written as DNG, so they stay fully editable.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use rayon::prelude::*;

use crate::demosaic::Quality;
use crate::image::{Image3, Plane, gaussian_blur};
use crate::pipeline::Engine;
use crate::pipeline::geometry::{Mat3d, apply_h, mat_inv};

// ---------------------------------------------------------------------------
// shared helpers

fn luma(img: &Image3) -> Plane {
    Plane { w: img.w, h: img.h, data: img.data.par_iter().map(|p| 0.25 * p[0] + 0.5 * p[1] + 0.25 * p[2]).collect() }
}

fn shrink(p: &Plane) -> Plane {
    let (w, h) = (p.w / 2, p.h / 2);
    Plane::from_fn(w.max(1), h.max(1), |x, y| {
        (p.get_clamped(2 * x as isize, 2 * y as isize)
            + p.get_clamped(2 * x as isize + 1, 2 * y as isize)
            + p.get_clamped(2 * x as isize, 2 * y as isize + 1)
            + p.get_clamped(2 * x as isize + 1, 2 * y as isize + 1))
            * 0.25
    })
}

/// Undo the as-shot WB so data is in raw camera space again.
fn to_camera_space(engine: &Engine, img: &Image3) -> Image3 {
    let n = engine.source.as_shot_neutral;
    img.map(|p| [p[0] * n[0], p[1] * n[1], p[2] * n[2]])
}

/// Write camera-space linear RGB as a float LinearRaw DNG using the first
/// source raw as the metadata/colour template.
pub fn write_linear_dng(out: &Path, img: &Image3, template: &Path) -> Result<()> {
    use rawler::decoders::RawDecodeParams;
    use rawler::dng::writer::DngWriter;
    use rawler::dng::{CropMode, DNG_VERSION_V1_4, DngCompression, DngPhotometricConversion};
    use rawler::rawimage::{BlackLevel, RawImageData, RawPhotometricInterpretation, WhiteLevel};
    use rawler::rawsource::RawSource;
    let src = RawSource::new(template)?;
    let dec = rawler::get_decoder(&src).map_err(|e| anyhow!("{e}"))?;
    let params = RawDecodeParams::default();
    let mut raw = dec.raw_image(&src, &params, false).map_err(|e| anyhow!("{e}"))?;
    let mut md = dec.raw_metadata(&src, &params).map_err(|e| anyhow!("{e}"))?;
    md.exif.orientation = Some(1);
    raw.width = img.w;
    raw.height = img.h;
    raw.cpp = 3;
    raw.bps = 32;
    raw.photometric = RawPhotometricInterpretation::LinearRaw;
    raw.data = RawImageData::Float(img.data.iter().flat_map(|p| *p).collect());
    raw.whitelevel = WhiteLevel(vec![1, 1, 1]);
    raw.blacklevel = BlackLevel::zero(1, 1, 3);
    raw.active_area = None;
    raw.crop_area = None;
    raw.blackareas.clear();
    raw.orientation = rawler::Orientation::Normal;
    let f = std::io::BufWriter::new(std::fs::File::create(out)?);
    let mut dng = DngWriter::new(f, DNG_VERSION_V1_4).map_err(|e| anyhow!("{e:?}"))?;
    {
        let mut sub = dng.subframe_on_root(0);
        sub.raw_image(&raw, CropMode::None, DngCompression::Uncompressed, DngPhotometricConversion::Original, 1).map_err(|e| anyhow!("{e:?}"))?;
        sub.finalize().map_err(|e| anyhow!("{e:?}"))?;
    }
    dng.load_base_tags(&raw).map_err(|e| anyhow!("{e:?}"))?;
    dng.load_metadata(&md).map_err(|e| anyhow!("{e:?}"))?;
    dng.close().map_err(|e| anyhow!("{e:?}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// HDR

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deghost {
    Off,
    Low,
    Medium,
    High,
}

pub struct HdrOptions {
    pub align: bool,
    pub deghost: Deghost,
}

/// Median threshold bitmap alignment (Ward 2003): integer translation of
/// `img` relative to `reference`.
pub fn mtb_align(reference: &Plane, img: &Plane, levels: usize) -> (i32, i32) {
    fn bitmaps(p: &Plane) -> (Vec<bool>, Vec<bool>) {
        let mut s: Vec<f32> = p.data.clone();
        let mid = s.len() / 2;
        s.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap());
        let med = s[mid];
        let tb = p.data.iter().map(|&v| v > med).collect();
        let eb = p.data.iter().map(|&v| (v - med).abs() > med * 0.04 + 1e-4).collect();
        (tb, eb)
    }
    let mut pyr_a = vec![reference.clone()];
    let mut pyr_b = vec![img.clone()];
    for _ in 0..levels {
        let a = shrink(pyr_a.last().unwrap());
        let b = shrink(pyr_b.last().unwrap());
        if a.w < 32 || a.h < 32 {
            break;
        }
        pyr_a.push(a);
        pyr_b.push(b);
    }
    let (mut sx, mut sy) = (0i32, 0i32);
    for l in (0..pyr_a.len()).rev() {
        let (a, b) = (&pyr_a[l], &pyr_b[l]);
        let (ta, ea) = bitmaps(a);
        let (tb, eb) = bitmaps(b);
        let (w, h) = (a.w as i32, a.h as i32);
        sx *= 2;
        sy *= 2;
        let mut best = (u64::MAX, 0, 0);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (ox, oy) = (sx + dx, sy + dy);
                let mut err = 0u64;
                for y in 0..h {
                    let yy = y + oy;
                    if yy < 0 || yy >= h {
                        continue;
                    }
                    for x in 0..w {
                        let xx = x + ox;
                        if xx < 0 || xx >= w {
                            continue;
                        }
                        let i = (y * w + x) as usize;
                        let j = (yy * w + xx) as usize;
                        if (ta[i] != tb[j]) && ea[i] && eb[j] {
                            err += 1;
                        }
                    }
                }
                if err < best.0 {
                    best = (err, ox, oy);
                }
            }
        }
        sx = best.1;
        sy = best.2;
    }
    (sx, sy)
}

fn shift(img: &Image3, dx: i32, dy: i32) -> Image3 {
    Image3::from_fn(img.w, img.h, |x, y| img.get_clamped(x as isize + dx as isize, y as isize + dy as isize))
}

pub fn merge_hdr(paths: &[PathBuf], opts: &HdrOptions, progress: &(dyn Fn(f32) + Sync)) -> Result<(Image3, PathBuf)> {
    if paths.len() < 2 {
        bail!("select at least two bracketed images");
    }
    let mut imgs = vec![];
    let mut engines = vec![];
    for (i, p) in paths.iter().enumerate() {
        let e = Engine::open(p)?;
        if !e.is_raw() {
            bail!("{} is not a raw file", p.display());
        }
        let base = e.base(Quality::Full);
        imgs.push(to_camera_space(&e, &base));
        engines.push(e);
        progress(0.4 * (i + 1) as f32 / paths.len() as f32);
    }
    let (w, h) = (imgs[0].w, imgs[0].h);
    if imgs.iter().any(|i| i.w != w || i.h != h) {
        bail!("images have different sizes");
    }
    // brightness order; reference is the middle exposure
    let lum: Vec<Plane> = imgs.iter().map(luma).collect();
    let mean: Vec<f32> = lum.iter().map(|l| l.data.iter().step_by(97).sum::<f32>()).collect();
    let mut order: Vec<usize> = (0..imgs.len()).collect();
    order.sort_by(|a, b| mean[*a].partial_cmp(&mean[*b]).unwrap());
    let r = order[order.len() / 2];
    // relative exposure from well-exposed pixels of both images
    let ratio: Vec<f32> = (0..imgs.len())
        .map(|i| {
            if i == r {
                return 1.0;
            }
            let mut v: Vec<f32> = (0..w * h)
                .step_by(13)
                .filter_map(|k| {
                    let (a, b) = (lum[i].data[k], lum[r].data[k]);
                    if a > 0.03 && a < 0.7 && b > 0.03 && b < 0.7 { Some(a / b) } else { None }
                })
                .collect();
            if v.is_empty() {
                return 1.0;
            }
            let m = v.len() / 2;
            v.select_nth_unstable_by(m, |a, b| a.partial_cmp(b).unwrap());
            v[m]
        })
        .collect();
    // alignment
    if opts.align {
        let ref_l = lum[r].map(|v| v.max(1e-5).ln());
        for i in 0..imgs.len() {
            if i == r {
                continue;
            }
            let li = lum[i].map(|v| v.max(1e-5).ln());
            let (dx, dy) = mtb_align(&ref_l, &li, 6);
            if dx != 0 || dy != 0 {
                imgs[i] = shift(&imgs[i], dx, dy);
            }
        }
    }
    progress(0.6);
    let clip = 0.97f32;
    let thr = match opts.deghost {
        Deghost::Off => f32::MAX,
        Deghost::Low => 0.6,
        Deghost::Medium => 0.35,
        Deghost::High => 0.2,
    };
    let refimg = &imgs[r];
    let mut out = Image3::new(w, h);
    out.data.par_iter_mut().enumerate().for_each(|(k, o)| {
        let rp = refimg.data[k];
        let rr = rp.map(|v| v / ratio[r]);
        let rm = rp[0].max(rp[1]).max(rp[2]);
        let mut acc = [0.0f32; 3];
        let mut ws = 0.0f32;
        for i in 0..imgs.len() {
            let p = imgs[i].data[k];
            let m = p[0].max(p[1]).max(p[2]);
            let mut wgt = crate::pipeline::tone::smoothstep(0.0, 0.04, m) * (1.0 - crate::pipeline::tone::smoothstep(0.8, clip, m));
            // longer exposures have less noise
            wgt *= ratio[i].sqrt();
            if wgt <= 0.0 {
                continue;
            }
            let rad = p.map(|v| v / ratio[i]);
            if i != r && thr < f32::MAX && rm > 0.02 && rm < 0.9 {
                let y1 = rad[1].max(1e-6);
                let y0 = rr[1].max(1e-6);
                if (y1 / y0).ln().abs() > thr {
                    continue;
                }
            }
            for c in 0..3 {
                acc[c] += rad[c] * wgt;
            }
            ws += wgt;
        }
        *o = if ws > 1e-6 {
            acc.map(|v| v / ws)
        } else {
            // everything clipped or black: take the darkest / brightest frame
            let i = if rm > 0.5 { order[0] } else { order[order.len() - 1] };
            imgs[i].data[k].map(|v| v / ratio[i])
        };
    });
    progress(0.9);
    // normalise so the reference exposure maps to the same level
    let first = paths[r].clone();
    Ok((out, first))
}

// ---------------------------------------------------------------------------
// Panorama

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Spherical,
    Cylindrical,
    Perspective,
}

pub struct PanoOptions {
    pub projection: Projection,
    pub auto_crop: bool,
    pub fill_edges: bool,
    /// 0..100: stretch the stitched result to fill the rectangle.
    pub boundary_warp: f32,
}

#[derive(Clone, Copy, Debug)]
struct Feature {
    x: f32,
    y: f32,
    desc: [f32; 64],
}

fn harris(p: &Plane, max: usize) -> Vec<(f32, f32)> {
    let (w, h) = (p.w, p.h);
    let gx = Plane::from_fn(w, h, |x, y| p.get_clamped(x as isize + 1, y as isize) - p.get_clamped(x as isize - 1, y as isize));
    let gy = Plane::from_fn(w, h, |x, y| p.get_clamped(x as isize, y as isize + 1) - p.get_clamped(x as isize, y as isize - 1));
    let xx = gaussian_blur(&gx.zip_map(&gx, |a, b| a * b), 1.5);
    let yy = gaussian_blur(&gy.zip_map(&gy, |a, b| a * b), 1.5);
    let xy = gaussian_blur(&gx.zip_map(&gy, |a, b| a * b), 1.5);
    let r = Plane::from_fn(w, h, |x, y| {
        let i = y * w + x;
        let det = xx.data[i] * yy.data[i] - xy.data[i] * xy.data[i];
        let tr = xx.data[i] + yy.data[i];
        det - 0.05 * tr * tr
    });
    let mut pts = vec![];
    let border = 12;
    for y in border..h.saturating_sub(border) {
        for x in border..w.saturating_sub(border) {
            let v = r.get(x, y);
            if v <= 1e-7 {
                continue;
            }
            let mut is_max = true;
            'n: for dy in -3isize..=3 {
                for dx in -3isize..=3 {
                    if (dx != 0 || dy != 0) && r.get_clamped(x as isize + dx, y as isize + dy) >= v {
                        is_max = false;
                        break 'n;
                    }
                }
            }
            if is_max {
                pts.push((v, x, y));
            }
        }
    }
    pts.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    pts.truncate(max);
    pts.into_iter().map(|(_, x, y)| (x as f32, y as f32)).collect()
}

fn describe(p: &Plane, pts: &[(f32, f32)]) -> Vec<Feature> {
    let b = gaussian_blur(p, 1.2);
    pts.iter()
        .map(|&(x, y)| {
            let mut d = [0f32; 64];
            for j in 0..8 {
                for i in 0..8 {
                    d[j * 8 + i] = b.sample_bilinear(x + (i as f32 - 3.5) * 1.5 + 0.5, y + (j as f32 - 3.5) * 1.5 + 0.5);
                }
            }
            let mean = d.iter().sum::<f32>() / 64.0;
            let sd = (d.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / 64.0).sqrt().max(1e-6);
            for v in d.iter_mut() {
                *v = (*v - mean) / sd;
            }
            Feature { x, y, desc: d }
        })
        .collect()
}

fn match_features(a: &[Feature], b: &[Feature]) -> Vec<((f32, f32), (f32, f32))> {
    a.par_iter()
        .filter_map(|fa| {
            let mut best = (f32::MAX, 0usize);
            let mut second = f32::MAX;
            for (j, fb) in b.iter().enumerate() {
                let d: f32 = fa.desc.iter().zip(fb.desc.iter()).map(|(x, y)| (x - y).powi(2)).sum();
                if d < best.0 {
                    second = best.0;
                    best = (d, j);
                } else if d < second {
                    second = d;
                }
            }
            if best.0 < 0.6 * 0.6 * second {
                let fb = &b[best.1];
                Some(((fa.x, fa.y), (fb.x, fb.y)))
            } else {
                None
            }
        })
        .collect()
}

/// Homography from 4+ correspondences (DLT, normalised).
fn homography(pairs: &[((f64, f64), (f64, f64))]) -> Option<Mat3d> {
    if pairs.len() < 4 {
        return None;
    }
    // 8x8 normal equations with h33 = 1
    let mut ata = [[0f64; 8]; 8];
    let mut atb = [0f64; 8];
    for &((x, y), (u, v)) in pairs {
        let rows = [([x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u), ([0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v)];
        for (r, b) in rows {
            for i in 0..8 {
                for j in 0..8 {
                    ata[i][j] += r[i] * r[j];
                }
                atb[i] += r[i] * b;
            }
        }
    }
    // gaussian elimination
    let mut m = ata;
    let mut bb = atb;
    for i in 0..8 {
        let piv = (i..8).max_by(|a, b| m[*a][i].abs().partial_cmp(&m[*b][i].abs()).unwrap())?;
        if m[piv][i].abs() < 1e-12 {
            return None;
        }
        m.swap(i, piv);
        bb.swap(i, piv);
        for k in i + 1..8 {
            let f = m[k][i] / m[i][i];
            for j in i..8 {
                m[k][j] -= f * m[i][j];
            }
            bb[k] -= f * bb[i];
        }
    }
    let mut h = [0f64; 8];
    for i in (0..8).rev() {
        let mut s = bb[i];
        for j in i + 1..8 {
            s -= m[i][j] * h[j];
        }
        h[i] = s / m[i][i];
    }
    Some([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]])
}

fn ransac(pairs: &[((f32, f32), (f32, f32))], affine_only: bool) -> Option<(Mat3d, usize)> {
    let pairs: Vec<((f64, f64), (f64, f64))> = pairs.iter().map(|(a, b)| ((a.0 as f64, a.1 as f64), (b.0 as f64, b.1 as f64))).collect();
    if pairs.len() < 8 {
        return None;
    }
    let mut best: Option<(Mat3d, usize)> = None;
    let mut seed = 0x2545F4914F6CDD1Du64;
    let mut rnd = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    for _ in 0..1500 {
        let s: Vec<_> = (0..4).map(|_| pairs[rnd(pairs.len())]).collect();
        let h = if affine_only {
            // translation + small rotation/scale (similarity) from two points
            let ((x0, y0), (u0, v0)) = s[0];
            let ((x1, y1), (u1, v1)) = s[1];
            let (dx, dy, du, dv) = (x1 - x0, y1 - y0, u1 - u0, v1 - v0);
            let den = dx * dx + dy * dy;
            if den < 1.0 {
                continue;
            }
            let a = (dx * du + dy * dv) / den;
            let b = (dx * dv - dy * du) / den;
            Some([[a, -b, u0 - a * x0 + b * y0], [b, a, v0 - b * x0 - a * y0], [0.0, 0.0, 1.0]])
        } else {
            homography(&s)
        };
        let Some(h) = h else { continue };
        let inl = pairs
            .iter()
            .filter(|((x, y), (u, v))| {
                let (px, py) = apply_h(&h, *x, *y);
                (px - u).powi(2) + (py - v).powi(2) < 9.0
            })
            .count();
        if best.map(|b| inl > b.1).unwrap_or(true) {
            best = Some((h, inl));
        }
    }
    let (h, n) = best?;
    // refit on inliers
    let inl: Vec<_> = pairs
        .iter()
        .filter(|((x, y), (u, v))| {
            let (px, py) = apply_h(&h, *x, *y);
            (px - u).powi(2) + (py - v).powi(2) < 9.0
        })
        .copied()
        .collect();
    let refit = if affine_only { Some(h) } else { homography(&inl) };
    Some((refit.unwrap_or(h), n))
}

pub fn merge_panorama(paths: &[PathBuf], opts: &PanoOptions, progress: &(dyn Fn(f32) + Sync)) -> Result<(Image3, PathBuf)> {
    if paths.len() < 2 {
        bail!("select at least two overlapping images");
    }
    let mut imgs = vec![];
    for (i, p) in paths.iter().enumerate() {
        let e = Engine::open(p)?;
        let base = e.base(Quality::Full);
        imgs.push(if e.is_raw() { to_camera_space(&e, &base) } else { (*base).clone() });
        progress(0.3 * (i + 1) as f32 / paths.len() as f32);
    }
    // registration on small copies
    let reg_long = 1000.0f32;
    let scale = reg_long / imgs[0].w.max(imgs[0].h) as f32;
    let small: Vec<Plane> = imgs
        .iter()
        .map(|im| {
            let s = im.resize((im.w as f32 * scale) as usize, (im.h as f32 * scale) as usize);
            luma(&s).map(|v| v.max(1e-5).powf(0.45))
        })
        .collect();
    let feats: Vec<Vec<Feature>> = small.par_iter().map(|p| describe(p, &harris(p, 1500))).collect();
    // pairwise homographies between neighbours (input order)
    let n = imgs.len();
    let mut rel: Vec<Mat3d> = vec![]; // maps image i+1 -> image i (small coords)
    for i in 0..n - 1 {
        let m = match_features(&feats[i + 1], &feats[i]);
        let (h, inl) = ransac(&m, false).ok_or_else(|| anyhow!("images {} and {} don't overlap enough", i + 1, i + 2))?;
        if inl < 12 {
            bail!("images {} and {} don't overlap enough ({inl} matches)", i + 1, i + 2);
        }
        rel.push(h);
    }
    progress(0.45);
    // chain to the centre image
    let c = n / 2;
    let mut to_c: Vec<Mat3d> = vec![crate::pipeline::geometry::IDENT; n];
    for i in (0..c).rev() {
        // i -> i+1 is inverse of rel[i] (i+1 -> i)
        to_c[i] = mm(&to_c[i + 1], &mat_inv(&rel[i]));
    }
    for i in c + 1..n {
        to_c[i] = mm(&to_c[i - 1], &rel[i - 1]);
    }
    // full-res homographies
    let s_up = [[1.0 / scale as f64, 0.0, 0.0], [0.0, 1.0 / scale as f64, 0.0], [0.0, 0.0, 1.0]];
    let s_dn = [[scale as f64, 0.0, 0.0], [0.0, scale as f64, 0.0], [0.0, 0.0, 1.0]];
    let hs: Vec<Mat3d> = to_c.iter().map(|h| mm(&s_up, &mm(h, &s_dn))).collect();
    let (cw, ch) = (imgs[c].w as f64, imgs[c].h as f64);
    // focal estimate (pixels): from the horizontal spread of a neighbour, else 1.1 * width
    let f = cw.max(ch) * 1.1;
    // projection functions: centre-image plane coords -> output coords
    let proj = |x: f64, y: f64| -> (f64, f64) {
        let (dx, dy) = (x - cw / 2.0, y - ch / 2.0);
        match opts.projection {
            Projection::Perspective => (dx, dy),
            Projection::Cylindrical => {
                let th = dx.atan2(f);
                (f * th, f * dy / (dx * dx + f * f).sqrt())
            }
            Projection::Spherical => {
                let th = dx.atan2(f);
                let ph = dy.atan2((dx * dx + f * f).sqrt());
                (f * th, f * ph)
            }
        }
    };
    let unproj = |u: f64, v: f64| -> (f64, f64) {
        let (dx, dy) = match opts.projection {
            Projection::Perspective => (u, v),
            Projection::Cylindrical => {
                let th = u / f;
                let dx = f * th.tan();
                (dx, v / f * (dx * dx + f * f).sqrt())
            }
            Projection::Spherical => {
                let th = u / f;
                let ph = v / f;
                let dx = f * th.tan();
                (dx, ph.tan() * (dx * dx + f * f).sqrt())
            }
        };
        (dx + cw / 2.0, dy + ch / 2.0)
    };
    // output bounds
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (i, im) in imgs.iter().enumerate() {
        for k in 0..=20 {
            let t = k as f64 / 20.0;
            for (x, y) in [(t * im.w as f64, 0.0), (t * im.w as f64, im.h as f64), (0.0, t * im.h as f64), (im.w as f64, t * im.h as f64)] {
                let (px, py) = apply_h(&hs[i], x, y);
                let (u, v) = proj(px, py);
                minx = minx.min(u);
                miny = miny.min(v);
                maxx = maxx.max(u);
                maxy = maxy.max(v);
            }
        }
    }
    let (ow, oh) = ((maxx - minx).ceil() as usize, (maxy - miny).ceil() as usize);
    if ow * oh > 400_000_000 {
        bail!("panorama would be too large ({ow}x{oh}); try cylindrical or spherical projection");
    }
    let inv: Vec<Mat3d> = hs.iter().map(mat_inv).collect();
    // feathered blending: weight = distance to the image edge
    let mut out = Image3::new(ow, oh);
    let mut cover = Plane::new(ow, oh);
    out.data.par_chunks_mut(ow).zip(cover.data.par_chunks_mut(ow)).enumerate().for_each(|(y, (row, crow))| {
        for x in 0..ow {
            let (u, v) = (x as f64 + minx + 0.5, y as f64 + miny + 0.5);
            let (cx, cy) = unproj(u, v);
            let mut acc = [0.0f32; 3];
            let mut ws = 0.0f32;
            for (i, im) in imgs.iter().enumerate() {
                let (sx, sy) = apply_h(&inv[i], cx, cy);
                if sx < 0.0 || sy < 0.0 || sx >= im.w as f64 || sy >= im.h as f64 {
                    continue;
                }
                let ex = (sx.min(im.w as f64 - sx) / im.w as f64).min(sy.min(im.h as f64 - sy) / im.h as f64) as f32;
                let wgt = (ex * 4.0).min(1.0).powi(2) + 1e-4;
                let p = im.sample_bilinear(sx as f32, sy as f32);
                for k in 0..3 {
                    acc[k] += p[k] * wgt;
                }
                ws += wgt;
            }
            if ws > 0.0 {
                row[x] = acc.map(|v| v / ws);
                crow[x] = 1.0;
            }
        }
    });
    progress(0.85);
    let mut out = out;
    if opts.boundary_warp > 0.0 {
        let (o, c) = boundary_warp(&out, &cover, opts.boundary_warp / 100.0);
        out = o;
        cover = c;
    }
    if opts.auto_crop {
        let (x0, y0, x1, y1) = largest_covered_rect(&cover);
        out = out.crop(x0, y0, x1 - x0, y1 - y0);
    } else if opts.fill_edges {
        let hole = cover.map(|v| if v > 0.5 { 0.0 } else { 1.0 });
        // fill at reduced size for speed, then upsample into the holes
        let f = (out.w.max(out.h) / 1500).max(1);
        let small_img = out.downscale_int(f);
        let small_hole = hole.resize(small_img.w, small_img.h).map(|v| if v > 0.01 { 1.0 } else { 0.0 });
        let filled = crate::inpaint::inpaint(&small_img, &small_hole).resize(out.w, out.h);
        out.data.par_iter_mut().zip(hole.data.par_iter()).zip(filled.data.par_iter()).for_each(|((o, h), f)| {
            if *h > 0.5 {
                *o = *f;
            }
        });
    }
    Ok((out, paths[c].clone()))
}

/// Stretch each column (then each row) so its covered span reaches the frame
/// edges, blended with the identity by `amount`.
fn boundary_warp(img: &Image3, cover: &Plane, amount: f32) -> (Image3, Plane) {
    let (w, h) = (img.w, img.h);
    let smooth = |v: Vec<f32>| -> Vec<f32> {
        let p = Plane { w: v.len(), h: 1, data: v };
        gaussian_blur(&p, (p.w as f32 / 60.0).max(2.0)).data
    };
    // vertical pass
    let mut top = vec![0f32; w];
    let mut bot = vec![h as f32; w];
    for x in 0..w {
        let col: Vec<usize> = (0..h).filter(|&y| cover.get(x, y) > 0.5).collect();
        if let (Some(a), Some(b)) = (col.first(), col.last()) {
            top[x] = *a as f32;
            bot[x] = *b as f32 + 1.0;
        }
    }
    let (top, bot) = (smooth(top), smooth(bot));
    let remap_v = |src: &Image3, cov: &Plane| -> (Image3, Plane) {
        let o = Image3::from_fn(w, h, |x, y| {
            let t = (y as f32 + 0.5) / h as f32;
            let sy = top[x] + t * (bot[x] - top[x]);
            let yy = (y as f32 + 0.5) + (sy - (y as f32 + 0.5)) * amount;
            src.sample_bilinear(x as f32 + 0.5, yy)
        });
        let c = Plane::from_fn(w, h, |x, y| {
            let t = (y as f32 + 0.5) / h as f32;
            let sy = top[x] + t * (bot[x] - top[x]);
            let yy = (y as f32 + 0.5) + (sy - (y as f32 + 0.5)) * amount;
            cov.sample_bilinear(x as f32 + 0.5, yy)
        });
        (o, c)
    };
    let (img2, cov2) = remap_v(img, cover);
    // horizontal pass
    let mut left = vec![0f32; h];
    let mut right = vec![w as f32; h];
    for y in 0..h {
        let row: Vec<usize> = (0..w).filter(|&x| cov2.get(x, y) > 0.5).collect();
        if let (Some(a), Some(b)) = (row.first(), row.last()) {
            left[y] = *a as f32;
            right[y] = *b as f32 + 1.0;
        }
    }
    let (left, right) = (smooth(left), smooth(right));
    let o = Image3::from_fn(w, h, |x, y| {
        let t = (x as f32 + 0.5) / w as f32;
        let sx = left[y] + t * (right[y] - left[y]);
        img2.sample_bilinear((x as f32 + 0.5) + (sx - (x as f32 + 0.5)) * amount, y as f32 + 0.5)
    });
    let c = Plane::from_fn(w, h, |x, y| {
        let t = (x as f32 + 0.5) / w as f32;
        let sx = left[y] + t * (right[y] - left[y]);
        cov2.sample_bilinear((x as f32 + 0.5) + (sx - (x as f32 + 0.5)) * amount, y as f32 + 0.5)
    });
    (o, c)
}

fn mm(a: &Mat3d, b: &Mat3d) -> Mat3d {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

/// Largest axis-aligned rectangle fully covered (histogram method on rows).
fn largest_covered_rect(c: &Plane) -> (usize, usize, usize, usize) {
    let (w, h) = (c.w, c.h);
    let mut heights = vec![0usize; w];
    let mut best = (0usize, 0, 0, 0, 0); // area, x0, y0, x1, y1
    for y in 0..h {
        for x in 0..w {
            heights[x] = if c.get(x, y) > 0.5 { heights[x] + 1 } else { 0 };
        }
        let mut stack: Vec<usize> = vec![];
        for x in 0..=w {
            let cur = if x < w { heights[x] } else { 0 };
            while let Some(&top) = stack.last() {
                if heights[top] <= cur {
                    break;
                }
                stack.pop();
                let hgt = heights[top];
                let left = stack.last().map(|v| v + 1).unwrap_or(0);
                let area = hgt * (x - left);
                if area > best.0 {
                    best = (area, left, y + 1 - hgt, x, y + 1);
                }
            }
            stack.push(x);
        }
    }
    if best.0 == 0 {
        return (0, 0, w, h);
    }
    (best.1, best.2, best.3, best.4)
}

/// Output path next to the first source: "<stem>-HDR.dng" / "<stem>-Pano.dng".
pub fn output_path(first: &Path, suffix: &str) -> PathBuf {
    let stem = first.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "merge".into());
    let dir = first.parent().unwrap_or(Path::new("."));
    let mut p = dir.join(format!("{stem}-{suffix}.dng"));
    let mut n = 1;
    while p.exists() {
        p = dir.join(format!("{stem}-{suffix}-{n}.dng"));
        n += 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtb_finds_translation() {
        let base = Plane::from_fn(256, 256, |x, y| (((x / 16) + (y / 16)) % 2) as f32 * 0.5 + 0.1 + (x as f32 * 0.001));
        let shifted = Plane::from_fn(256, 256, |x, y| base.get_clamped(x as isize - 5, y as isize + 3) * 0.5);
        let (dx, dy) = mtb_align(&base, &shifted, 4);
        assert_eq!((dx, dy), (5, -3));
    }

    #[test]
    fn homography_recovers_translation() {
        let pairs: Vec<((f64, f64), (f64, f64))> =
            [(0.0, 0.0), (100.0, 0.0), (0.0, 80.0), (100.0, 80.0), (50.0, 40.0)].iter().map(|&(x, y)| ((x, y), (x + 7.0, y - 3.0))).collect();
        let h = homography(&pairs).unwrap();
        let (u, v) = apply_h(&h, 20.0, 30.0);
        assert!((u - 27.0).abs() < 1e-6 && (v - 27.0).abs() < 1e-6);
    }

    #[test]
    fn largest_rect() {
        let c = Plane::from_fn(10, 8, |x, y| if (2..9).contains(&x) && (1..6).contains(&y) { 1.0 } else { 0.0 });
        assert_eq!(largest_covered_rect(&c), (2, 1, 9, 6));
    }
}
