//! Distraction removal: find people or wires/cables and turn them into
//! content-aware Remove regions (S-frame rasters for `RetouchSpot::mask`).

use crate::image::{Plane, box_blur, gaussian_blur, min_filter};
use crate::params::{AiMaskKind, DevelopSettings, RetouchMode, RetouchSpot};
use crate::pipeline::Engine;

/// Resample a T-frame plane into the S frame (before geometry).
fn t_to_s(engine: &Engine, s: &DevelopSettings, t: &Plane) -> Plane {
    let (fw, fh) = engine.full_size();
    let geo = crate::pipeline::geometry::Geometry::new(s, fw, fh, Default::default());
    let (w, h) = (t.w, t.h);
    Plane::from_fn(w, h, |x, y| {
        let (ou, ov) = geo.s_to_o((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
        let (tu, tv) = if geo.identity_h { (ou, ov) } else { geo.o_to_t(ou, ov) };
        t.sample_bilinear(tu as f32 * t.w as f32, tv as f32 * t.h as f32)
    })
}

fn dilate(p: &Plane, r: usize) -> Plane {
    // max filter = -min(-x)
    min_filter(&p.map(|v| -v), r).map(|v| -v)
}

fn spot(mask: &Plane) -> RetouchSpot {
    RetouchSpot {
        mode: RetouchMode::Remove,
        points: vec![],
        radius: 0.0,
        feather: 0.0,
        opacity: 100.0,
        source_offset: [0.0, 0.0],
        mask: Some(crate::pipeline::local::encode_raster(mask)),
    }
}

/// People: person segmentation (learned model if installed), dilated a bit
/// so the fill doesn't leave outlines.
pub fn remove_people(engine: &Engine, s: &DevelopSettings) -> Option<RetouchSpot> {
    let m = crate::ai::detect(engine, s, AiMaskKind::Person, None);
    let bin = m.map(|v| if v > 0.5 { 1.0 } else { 0.0 });
    if bin.data.iter().sum::<f32>() < 50.0 {
        return None;
    }
    let grown = dilate(&bin, (bin.w.max(bin.h) / 150).max(2));
    Some(spot(&t_to_s(engine, s, &grown)))
}

/// Wires and cables: thin dark lines against bright, smooth areas (sky).
pub fn remove_wires(engine: &Engine, s: &DevelopSettings) -> Option<RetouchSpot> {
    let mut s2 = s.clone();
    s2.masks.clear();
    let img = engine.render_fit(&s2, 1600, 1600, crate::demosaic::Quality::Draft, true).image;
    let l = img.luminance().map(|v| v.max(0.0).powf(0.45));
    let (w, h) = (l.w, l.h);
    // black-hat: closing minus image picks out thin dark structures
    let r = 3;
    let closed = min_filter(&dilate(&l, r).map(|v| -v), r).map(|v| -v);
    let bh = closed.zip_map(&l, |c, v| (c - v).max(0.0));
    // only where the surroundings are bright and smooth
    let bg = box_blur(&closed, 12);
    let tex = gaussian_blur(&closed.zip_map(&gaussian_blur(&closed, 2.0), |a, b| (a - b).abs()), 6.0);
    let cand = Plane::from_fn(w, h, |x, y| {
        let i = y * w + x;
        let bright = crate::pipeline::tone::smoothstep(0.45, 0.65, bg.data[i]);
        let smooth = 1.0 - crate::pipeline::tone::smoothstep(0.01, 0.03, tex.data[i]);
        if bh.data[i] * bright * smooth > 0.04 { 1.0 } else { 0.0 }
    });
    // keep long, thin components
    let keep = long_components(&cand, (w.max(h) as f32 * 0.06) as usize);
    if keep.data.iter().sum::<f32>() < 30.0 {
        return None;
    }
    Some(spot(&t_to_s(engine, s, &dilate(&keep, 2))))
}

fn long_components(bin: &Plane, min_extent: usize) -> Plane {
    let (w, h) = (bin.w, bin.h);
    let mut label = vec![0u32; w * h];
    let mut out = Plane::new(w, h);
    let mut next = 1u32;
    for i in 0..w * h {
        if bin.data[i] < 0.5 || label[i] != 0 {
            continue;
        }
        let mut stack = vec![i];
        let mut members = vec![];
        label[i] = next;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        while let Some(j) = stack.pop() {
            members.push(j);
            let (x, y) = (j % w, j / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let k = ny as usize * w + nx as usize;
                    if bin.data[k] > 0.5 && label[k] == 0 {
                        label[k] = next;
                        stack.push(k);
                    }
                }
            }
        }
        next += 1;
        let extent = (x1 - x0).max(y1 - y0);
        let area = (x1 - x0 + 1) * (y1 - y0 + 1);
        // wires are nearly straight (or gently sagging): the point cloud is
        // strongly one-dimensional, unlike branching twigs
        let n = members.len() as f64;
        let (mx, my) = members.iter().fold((0.0, 0.0), |(a, b), &j| (a + (j % w) as f64 / n, b + (j / w) as f64 / n));
        let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
        for &j in &members {
            let (dx, dy) = ((j % w) as f64 - mx, (j / w) as f64 - my);
            sxx += dx * dx;
            syy += dy * dy;
            sxy += dx * dy;
        }
        let tr = sxx + syy;
        let det = sxx * syy - sxy * sxy;
        let l1 = tr / 2.0 + ((tr * tr / 4.0) - det).max(0.0).sqrt();
        let l2 = (tr - l1).max(0.0);
        let linear = l1 > 0.0 && l2 / l1 < 0.004;
        if extent >= min_extent && linear && (members.len() as f32) < area as f32 * 0.25 {
            for j in members {
                out.data[j] = 1.0;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_wires_drops_twigs() {
        let (w, h) = (400, 300);
        let mut p = Plane::new(w, h);
        // a gently sagging wire across the frame
        for x in 0..w {
            let y = 60.0 + 0.0004 * (x as f32 - 200.0).powi(2);
            p.data[y as usize * w + x] = 1.0;
        }
        // a branching twig: trunk with side branches
        for y in 150..290 {
            p.data[y * w + 100] = 1.0;
            if y % 20 == 0 {
                for k in 0..40 {
                    p.data[(y - k / 2) * w + 100 + k] = 1.0;
                    p.data[(y - k / 2) * w + 100 - k.min(99)] = 1.0;
                }
            }
        }
        let keep = long_components(&p, 40);
        assert!(keep.get(200, 60) > 0.5, "wire kept");
        assert!(keep.get(100, 200) < 0.5, "twig dropped");
    }
}
