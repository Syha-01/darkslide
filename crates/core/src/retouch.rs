//! Spot removal: clone, heal (clone + low-frequency match) and content-aware
//! remove (PatchMatch inpainting). Operates on the oriented camera-RGB base,
//! in O-frame normalised coordinates (before geometry).

use rayon::prelude::*;

use crate::image::{Image3, Plane, Rgb, gaussian_blur, gaussian_blur3};
use crate::params::{RetouchMode, RetouchSpot};

fn spot_mask(spot: &RetouchSpot, w: usize, h: usize) -> (Plane, [usize; 4]) {
    let long = w.max(h) as f32;
    let r = spot.radius * long;
    let feather = (spot.feather / 100.0).clamp(0.0, 1.0);
    let pts: Vec<(f32, f32)> = spot.points.iter().map(|p| (p[0] * w as f32, p[1] * h as f32)).collect();
    let mut bb = [usize::MAX, usize::MAX, 0, 0];
    for &(x, y) in &pts {
        bb[0] = bb[0].min((x - r - 2.0).max(0.0) as usize);
        bb[1] = bb[1].min((y - r - 2.0).max(0.0) as usize);
        bb[2] = bb[2].max(((x + r + 2.0).max(0.0) as usize).min(w));
        bb[3] = bb[3].max(((y + r + 2.0).max(0.0) as usize).min(h));
    }
    let mut m = Plane::new(w, h);
    if bb[0] >= bb[2] || bb[1] >= bb[3] {
        return (m, [0, 0, 0, 0]);
    }
    let inner = r * (1.0 - feather);
    for y in bb[1]..bb[3] {
        for x in bb[0]..bb[2] {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // distance to polyline
            let mut d = f32::MAX;
            for (i, &(ax, ay)) in pts.iter().enumerate() {
                let (bx, by) = if i + 1 < pts.len() { pts[i + 1] } else { (ax, ay) };
                let (vx, vy) = (bx - ax, by - ay);
                let l2 = vx * vx + vy * vy;
                let t = if l2 > 0.0 { (((px - ax) * vx + (py - ay) * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                let (qx, qy) = (ax + vx * t, ay + vy * t);
                d = d.min(((px - qx).powi(2) + (py - qy).powi(2)).sqrt());
            }
            let v = if d <= inner {
                1.0
            } else if d >= r {
                0.0
            } else {
                1.0 - (d - inner) / (r - inner).max(1e-3)
            };
            m.data[y * w + x] = v * v * (3.0 - 2.0 * v);
        }
    }
    (m, bb)
}

/// Pick a source offset automatically: nearby patch with the most similar
/// surrounding ring and low texture difference.
pub fn find_source(img: &Image3, spot: &RetouchSpot) -> [f32; 2] {
    let (w, h) = (img.w, img.h);
    let long = w.max(h) as f32;
    let r = (spot.radius * long).max(2.0);
    let c = spot.points.first().copied().unwrap_or([0.5, 0.5]);
    let (cx, cy) = (c[0] * w as f32, c[1] * h as f32);
    // ring samples around the destination
    let ring: Vec<(f32, f32)> = (0..24)
        .map(|k| {
            let a = k as f32 / 24.0 * std::f32::consts::TAU;
            (a.cos() * r * 1.3, a.sin() * r * 1.3)
        })
        .collect();
    let dst_ring: Vec<Rgb> = ring.iter().map(|(dx, dy)| img.sample_bilinear(cx + dx, cy + dy)).collect();
    let mut best = (f32::MAX, [r * 2.5 / w as f32, 0.0]);
    for k in 0..48 {
        let a = k as f32 / 48.0 * std::f32::consts::TAU;
        for dist in [2.2f32, 3.0, 4.0] {
            let (ox, oy) = (a.cos() * r * dist, a.sin() * r * dist);
            let (sx, sy) = (cx + ox, cy + oy);
            if sx - r < 0.0 || sy - r < 0.0 || sx + r >= w as f32 || sy + r >= h as f32 {
                continue;
            }
            let mut cost = 0.0;
            for (i, (dx, dy)) in ring.iter().enumerate() {
                let p = img.sample_bilinear(sx + dx, sy + dy);
                let q = dst_ring[i];
                for ch in 0..3 {
                    let d = (p[ch].max(0.0).sqrt() - q[ch].max(0.0).sqrt()).abs();
                    cost += d;
                }
            }
            cost *= 1.0 + dist * 0.03;
            if cost < best.0 {
                best = (cost, [ox / w as f32, oy / h as f32]);
            }
        }
    }
    best.1
}

pub fn apply(img: &mut Image3, spots: &[RetouchSpot]) {
    for spot in spots {
        match spot.mode {
            RetouchMode::Clone => clone_heal(img, spot, false),
            RetouchMode::Heal => clone_heal(img, spot, true),
            RetouchMode::Remove => remove(img, spot),
        }
    }
}

fn clone_heal(img: &mut Image3, spot: &RetouchSpot, heal: bool) {
    let (w, h) = (img.w, img.h);
    let (mask, bb) = spot_mask(spot, w, h);
    if bb[2] == 0 {
        return;
    }
    let off = (spot.source_offset[0] * w as f32, spot.source_offset[1] * h as f32);
    let op = spot.opacity / 100.0;
    let long = w.max(h) as f32;
    let r = spot.radius * long;
    // pad box for low-frequency estimation
    let pad = (r * 1.5) as usize + 4;
    let x0 = bb[0].saturating_sub(pad);
    let y0 = bb[1].saturating_sub(pad);
    let x1 = (bb[2] + pad).min(w);
    let y1 = (bb[3] + pad).min(h);
    let (bw, bh) = (x1 - x0, y1 - y0);
    let src = Image3::from_fn(bw, bh, |x, y| img.sample_bilinear((x + x0) as f32 + 0.5 + off.0, (y + y0) as f32 + 0.5 + off.1));
    let dst = img.crop(x0, y0, bw, bh);
    let result = if heal {
        // heal: keep source texture, take low frequencies from the destination
        let sigma = (r * 0.5).max(1.5);
        let sl = gaussian_blur3(&src, sigma);
        let dl = gaussian_blur3(&dst, sigma);
        Image3::from_fn(bw, bh, |x, y| {
            let i = y * bw + x;
            let s = src.data[i];
            let mut o = [0.0; 3];
            for c in 0..3 {
                // multiplicative transfer in linear light keeps colour natural
                let ratio = (dl.data[i][c] + 1e-4) / (sl.data[i][c] + 1e-4);
                o[c] = s[c] * ratio.clamp(0.1, 10.0);
            }
            o
        })
    } else {
        src
    };
    for y in y0..y1 {
        for x in x0..x1 {
            let m = mask.data[y * w + x] * op;
            if m <= 0.0 {
                continue;
            }
            let i = (y - y0) * bw + (x - x0);
            let p = &mut img.data[y * w + x];
            for c in 0..3 {
                p[c] += (result.data[i][c] - p[c]) * m;
            }
        }
    }
}

/// Content-aware remove via a small PatchMatch on a working window.
fn remove(img: &mut Image3, spot: &RetouchSpot) {
    if let Some(r) = spot.mask.as_deref().and_then(crate::pipeline::local::decode_raster) {
        remove_region(img, &r, spot.opacity / 100.0);
        return;
    }
    let (w, h) = (img.w, img.h);
    let (mask, bb) = spot_mask(spot, w, h);
    if bb[2] == 0 {
        return;
    }
    let long = w.max(h) as f32;
    let r = spot.radius * long;
    let pad = (r * 3.0) as usize + 8;
    let x0 = bb[0].saturating_sub(pad);
    let y0 = bb[1].saturating_sub(pad);
    let x1 = (bb[2] + pad).min(w);
    let y1 = (bb[3] + pad).min(h);
    let win = img.crop(x0, y0, x1 - x0, y1 - y0);
    let wm = Plane::from_fn(x1 - x0, y1 - y0, |x, y| mask.data[(y + y0) * w + x + x0]);
    let hole = wm.map(|v| if v > 0.02 { 1.0 } else { 0.0 });
    // PatchMatch cost grows with the hole; fill big strokes at reduced size
    let n = hole.data.iter().filter(|&&v| v > 0.5).count();
    let f = ((n as f32 / 150_000.0).sqrt().ceil() as usize).max(1);
    let filled = if f > 1 && win.w / f >= 16 && win.h / f >= 16 {
        let small = win.downscale_int(f);
        let small_hole = hole.resize(small.w, small.h).map(|v| if v > 0.01 { 1.0 } else { 0.0 });
        crate::inpaint::inpaint(&small, &small_hole).resize(win.w, win.h)
    } else {
        crate::inpaint::inpaint(&win, &hole)
    };
    let op = spot.opacity / 100.0;
    let soft = gaussian_blur(&wm, 1.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y - y0) * (x1 - x0) + (x - x0);
            let m = soft.data[i].max(wm.data[i]).min(1.0) * op;
            if m <= 0.0 {
                continue;
            }
            let p = &mut img.data[y * w + x];
            for c in 0..3 {
                p[c] += (filled.data[i][c] - p[c]) * m;
            }
        }
    }
}

/// Content-aware fill of a detected region (raster in normalised image coords).
fn remove_region(img: &mut Image3, raster: &Plane, opacity: f32) {
    let (w, h) = (img.w, img.h);
    let hole =
        Plane::from_fn(w, h, |x, y| raster.sample_bilinear((x as f32 + 0.5) / w as f32 * raster.w as f32, (y as f32 + 0.5) / h as f32 * raster.h as f32));
    let n = hole.data.iter().filter(|&&v| v > 0.3).count();
    if n == 0 {
        return;
    }
    // thin structures (wires) fill fine at full size; big regions at reduced size
    let f = if n > 250_000 { ((n as f32 / 250_000.0).sqrt().ceil() as usize).max(1) } else { 1 };
    let small = img.downscale_int(f);
    let small_hole = hole.resize(small.w, small.h).map(|v| if v > 0.15 { 1.0 } else { 0.0 });
    let filled = crate::inpaint::inpaint(&small, &small_hole);
    let filled = if f > 1 { filled.resize(w, h) } else { filled };
    let soft = gaussian_blur(&hole, 1.0);
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let m = soft.data[i].max(hole.data[i]).min(1.0) * opacity;
        if m > 0.0 {
            let fp = filled.data[i];
            for c in 0..3 {
                p[c] += (fp[c] - p[c]) * m;
            }
        }
    });
}

/// "Visualize spots": high-pass edge map that makes dust stand out.
pub fn visualize_spots(img: &Image3, threshold: f32) -> Plane {
    let l = img.luminance().map(|v| v.max(0.0).sqrt());
    let b = gaussian_blur(&l, 2.0);
    let t = 0.002 + (1.0 - threshold / 100.0) * 0.03;
    let d: Vec<f32> = l.data.par_iter().zip(b.data.par_iter()).map(|(a, b)| if (a - b).abs() > t { 1.0 } else { 0.0 }).collect();
    Plane { w: l.w, h: l.h, data: d }
}

/// Automatic dust detection: small dark round blobs on smooth areas.
pub fn detect_dust(img: &Image3, max_spots: usize) -> Vec<RetouchSpot> {
    let small_f = (img.w.max(img.h) / 1500).max(1);
    let s = img.downscale_int(small_f);
    let l = s.luminance().map(|v| v.max(1e-5).log2());
    let bg = gaussian_blur(&l, 6.0);
    let fine = gaussian_blur(&l, 1.0);
    let tex = gaussian_blur(&l.zip_map(&fine, |a, b| (a - b).abs()), 6.0);
    let mut cands: Vec<(f32, usize, usize)> = vec![];
    let (w, h) = (s.w, s.h);
    for y in 4..h.saturating_sub(4) {
        for x in 4..w.saturating_sub(4) {
            let i = y * w + x;
            let d = bg.data[i] - fine.data[i];
            if d > 0.08 && tex.data[i] < 0.03 {
                // local maximum of darkness
                let mut is_max = true;
                'o: for dy in -2isize..=2 {
                    for dx in -2isize..=2 {
                        let j = (y as isize + dy) as usize * w + (x as isize + dx) as usize;
                        if bg.data[j] - fine.data[j] > d {
                            is_max = false;
                            break 'o;
                        }
                    }
                }
                if is_max {
                    cands.push((d, x, y));
                }
            }
        }
    }
    cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    cands.truncate(max_spots);
    cands
        .into_iter()
        .map(|(_, x, y)| {
            let mut spot = RetouchSpot {
                mode: RetouchMode::Heal,
                points: vec![[(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32]],
                radius: 5.0 / w.max(h) as f32,
                feather: 50.0,
                opacity: 100.0,
                source_offset: [0.0, 0.0],
                mask: None,
            };
            spot.source_offset = find_source(&s, &spot);
            spot
        })
        .collect()
}
