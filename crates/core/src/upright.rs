//! Upright: line detection (gradient-oriented Hough), vanishing points and
//! the rectifying homography for Level / Vertical / Full / Auto / Guided.

use crate::image::Plane;
use crate::params::{DevelopSettings, Upright};
use crate::pipeline::Engine;
use crate::pipeline::geometry::{IDENT, Mat3d, apply_h, mat_inv};

/// A detected segment in O-frame centred coords (long edge half-size = 1).
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub a: [f64; 2],
    pub b: [f64; 2],
}

impl Segment {
    pub fn len(&self) -> f64 {
        ((self.b[0] - self.a[0]).powi(2) + (self.b[1] - self.a[1]).powi(2)).sqrt()
    }
    /// Angle in degrees, y down, in (-90, 90].
    pub fn angle(&self) -> f64 {
        let mut a = (self.b[1] - self.a[1]).atan2(self.b[0] - self.a[0]).to_degrees();
        if a > 90.0 {
            a -= 180.0;
        } else if a <= -90.0 {
            a += 180.0;
        }
        a
    }
    pub fn homog(&self) -> [f64; 3] {
        cross([self.a[0], self.a[1], 1.0], [self.b[0], self.b[1], 1.0])
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm3(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / n, v[1] / n, v[2] / n]
}

/// Luminance of the O-frame (user rotate/flip applied) at about `long` px.
fn o_frame_luma(engine: &Engine, s: &DevelopSettings, long: usize) -> (Plane, f64) {
    let base = engine.base(crate::demosaic::Quality::Draft);
    let (fw, fh) = engine.full_size();
    let geo = crate::pipeline::geometry::Geometry::new(s, fw, fh, Default::default());
    let f = long as f64 / geo.ow.max(geo.oh);
    let (w, h) = ((geo.ow * f) as usize, (geo.oh * f) as usize);
    let p = Plane::from_fn(w, h, |x, y| {
        let (su, sv) = geo.o_to_s((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
        let px = base.sample_bilinear((su * base.w as f64) as f32, (sv * base.h as f64) as f32);
        crate::color::luma_prophoto(px).max(1e-6).powf(0.4)
    });
    let half = w.max(h) as f64 / 2.0;
    (p, half)
}

/// Detect straight segments in the O frame.
pub fn detect_segments(engine: &Engine, s: &DevelopSettings) -> Vec<Segment> {
    let (img, half) = o_frame_luma(engine, s, 900);
    let (w, h) = (img.w, img.h);
    let sm = crate::image::gaussian_blur(&img, 1.0);
    // gradients
    let mut gx = vec![0f32; w * h];
    let mut gy = vec![0f32; w * h];
    let mut mag = vec![0f32; w * h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: isize, dy: isize| sm.get((x as isize + dx) as usize, (y as isize + dy) as usize);
            let sx = (p(1, -1) + 2.0 * p(1, 0) + p(1, 1)) - (p(-1, -1) + 2.0 * p(-1, 0) + p(-1, 1));
            let sy = (p(-1, 1) + 2.0 * p(0, 1) + p(1, 1)) - (p(-1, -1) + 2.0 * p(0, -1) + p(1, -1));
            let i = y * w + x;
            gx[i] = sx;
            gy[i] = sy;
            mag[i] = (sx * sx + sy * sy).sqrt();
        }
    }
    let mut sorted: Vec<f32> = mag.iter().copied().filter(|v| *v > 0.0).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let thr = sorted.get(sorted.len() * 88 / 100).copied().unwrap_or(0.1).max(0.02);
    // Hough over (theta, rho); theta is the line normal angle
    let n_theta = 360usize;
    let diag = ((w * w + h * h) as f64).sqrt();
    let n_rho = (diag * 2.0) as usize + 1;
    let mut acc = vec![0u32; n_theta * n_rho];
    let mut edges = vec![];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if mag[i] < thr {
                continue;
            }
            edges.push((x, y));
            let t0 = (gy[i] as f64).atan2(gx[i] as f64).rem_euclid(std::f64::consts::PI);
            let ti = (t0 / std::f64::consts::PI * n_theta as f64) as isize;
            for dt in -4..=4 {
                let t = (ti + dt).rem_euclid(n_theta as isize) as usize;
                let th = t as f64 / n_theta as f64 * std::f64::consts::PI;
                let rho = x as f64 * th.cos() + y as f64 * th.sin();
                let r = (rho + diag) as usize;
                if r < n_rho {
                    acc[t * n_rho + r] += 1;
                }
            }
        }
    }
    // peaks with non-maximum suppression
    let min_votes = (w.min(h) as f64 * 0.08) as u32;
    let mut peaks = vec![];
    for t in 0..n_theta {
        for r in 1..n_rho - 1 {
            let v = acc[t * n_rho + r];
            if v < min_votes {
                continue;
            }
            let mut is_max = true;
            'n: for dt in -3isize..=3 {
                for dr in -3isize..=3 {
                    if dt == 0 && dr == 0 {
                        continue;
                    }
                    let tt = (t as isize + dt).rem_euclid(n_theta as isize) as usize;
                    let rr = r as isize + dr;
                    if rr < 0 || rr >= n_rho as isize {
                        continue;
                    }
                    if acc[tt * n_rho + rr as usize] > v {
                        is_max = false;
                        break 'n;
                    }
                }
            }
            if is_max {
                peaks.push((v, t, r));
            }
        }
    }
    peaks.sort_by(|a, b| b.0.cmp(&a.0));
    peaks.truncate(80);
    // extract the longest supported run along each peak line
    let mut segs = vec![];
    let is_edge = |x: isize, y: isize| -> bool {
        if x < 1 || y < 1 || x >= w as isize - 1 || y >= h as isize - 1 {
            return false;
        }
        for dy in -1..=1 {
            for dx in -1..=1 {
                if mag[((y + dy) as usize) * w + (x + dx) as usize] >= thr {
                    return true;
                }
            }
        }
        false
    };
    for (_, t, r) in peaks {
        let th = t as f64 / n_theta as f64 * std::f64::consts::PI;
        let rho = r as f64 - diag;
        let (c, sn) = (th.cos(), th.sin());
        // point on the line and direction
        let p0 = (rho * c, rho * sn);
        let d = (-sn, c);
        let mut best: Option<(f64, f64)> = None;
        let mut run_start: Option<f64> = None;
        let mut gap = 0;
        let mut k = -diag;
        while k <= diag {
            let x = p0.0 + d.0 * k;
            let y = p0.1 + d.1 * k;
            let on = is_edge(x as isize, y as isize);
            if on {
                if run_start.is_none() {
                    run_start = Some(k);
                }
                gap = 0;
            } else if let Some(st) = run_start {
                gap += 1;
                if gap > 4 {
                    let len = k - gap as f64 - st;
                    if best.map(|b| len > b.1 - b.0).unwrap_or(true) {
                        best = Some((st, k - gap as f64));
                    }
                    run_start = None;
                    gap = 0;
                }
            }
            k += 1.0;
        }
        if let Some(st) = run_start {
            if best.map(|b| diag - st > b.1 - b.0).unwrap_or(true) {
                best = Some((st, diag));
            }
        }
        if let Some((k0, k1)) = best {
            let len = k1 - k0;
            if len < w.min(h) as f64 * 0.1 {
                continue;
            }
            let to_c = |x: f64, y: f64| [(x - w as f64 / 2.0) / half, (y - h as f64 / 2.0) / half];
            segs.push(Segment { a: to_c(p0.0 + d.0 * k0, p0.1 + d.1 * k0), b: to_c(p0.0 + d.0 * k1, p0.1 + d.1 * k1) });
        }
    }
    let _ = edges;
    segs
}

fn weighted_median(mut v: Vec<(f64, f64)>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let total: f64 = v.iter().map(|x| x.1).sum();
    let mut acc = 0.0;
    for (val, w) in &v {
        acc += w;
        if acc >= total / 2.0 {
            return Some(*val);
        }
    }
    v.last().map(|x| x.0)
}

/// Rotation (degrees) that levels the image, from near-horizontal and
/// near-vertical segments.
pub fn level_angle(segs: &[Segment]) -> Option<f64> {
    let mut v = vec![];
    for s in segs {
        let a = s.angle();
        if a.abs() < 25.0 {
            v.push((a, s.len()));
        } else if (a.abs() - 90.0).abs() < 25.0 {
            let dev = if a > 0.0 { a - 90.0 } else { a + 90.0 };
            v.push((dev, s.len() * 0.8));
        }
    }
    weighted_median(v)
}

pub fn auto_straighten_angle(engine: &Engine, s: &DevelopSettings) -> Option<f32> {
    let segs = detect_segments(engine, s);
    level_angle(&segs).map(|a| (a as f32 * 100.0).round() / 100.0).filter(|a| a.abs() <= 45.0)
}

/// Least-squares vanishing point of lines (smallest eigenvector of Σ w l lᵀ).
fn vanishing_point(lines: &[([f64; 3], f64)]) -> Option<[f64; 3]> {
    if lines.len() < 2 {
        return None;
    }
    let mut m = [[0.0f64; 3]; 3];
    for (l, w) in lines {
        let l = norm3(*l);
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += w * l[i] * l[j];
            }
        }
    }
    // inverse power iteration on m (shifted for stability)
    let inv = mat_inv(&[[m[0][0] + 1e-9, m[0][1], m[0][2]], [m[1][0], m[1][1] + 1e-9, m[1][2]], [m[2][0], m[2][1], m[2][2] + 1e-9]]);
    let mut v = [0.3, 0.9, 0.2];
    for _ in 0..60 {
        let nv = [
            inv[0][0] * v[0] + inv[0][1] * v[1] + inv[0][2] * v[2],
            inv[1][0] * v[0] + inv[1][1] * v[1] + inv[1][2] * v[2],
            inv[2][0] * v[0] + inv[2][1] * v[1] + inv[2][2] * v[2],
        ];
        v = norm3(nv);
    }
    Some(v)
}

fn rodrigues(from: [f64; 3], to: [f64; 3]) -> Mat3d {
    let a = norm3(from);
    let b = norm3(to);
    let v = cross(a, b);
    let c = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let s = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if s < 1e-12 {
        return IDENT;
    }
    let k = [v[0] / s, v[1] / s, v[2] / s];
    let kx = [[0.0, -k[2], k[1]], [k[2], 0.0, -k[0]], [-k[1], k[0], 0.0]];
    let mut r = IDENT;
    let mut kx2 = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            kx2[i][j] = (0..3).map(|t| kx[i][t] * kx[t][j]).sum();
        }
    }
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] += s * kx[i][j] + (1.0 - c) * kx2[i][j];
        }
    }
    r
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

fn mv(a: &Mat3d, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| a[i][0] * v[0] + a[i][1] * v[1] + a[i][2] * v[2])
}

const F: f64 = 1.6;

/// Rectifying homography (O -> T, centred coords) for the given mode.
pub fn compute(mode: Upright, segs: &[Segment], guides: &[Segment]) -> Option<Mat3d> {
    compute_with_aspect(mode, segs, guides, 1.5)
}

/// `aspect` is long/short of the frame; coordinates are half-long-edge units.
pub fn compute_with_aspect(mode: Upright, segs: &[Segment], guides: &[Segment], aspect: f64) -> Option<Mat3d> {
    let k = [[F, 0.0, 0.0], [0.0, F, 0.0], [0.0, 0.0, 1.0]];
    let kinv = mat_inv(&k);
    let guided = mode == Upright::Guided;
    // only long lines are trustworthy evidence of verticals/horizontals
    let long: Vec<Segment> = segs.iter().copied().filter(|s| s.len() > 0.25).collect();
    let use_segs: &[Segment] = if guided { guides } else { &long };
    let tol = if guided { 30.0 } else { 20.0 };
    let vsegs: Vec<&Segment> = use_segs.iter().filter(|s| (s.angle().abs() - 90.0).abs() < tol).collect();
    let hsegs: Vec<&Segment> = use_segs.iter().filter(|s| s.angle().abs() < tol).collect();
    // verticals must be spread across the frame to pin down a vanishing point
    let spread = |v: &[&Segment], horiz: bool| -> f64 {
        let c: Vec<f64> = v.iter().map(|s| if horiz { (s.a[0] + s.b[0]) / 2.0 } else { (s.a[1] + s.b[1]) / 2.0 }).collect();
        c.iter().cloned().fold(f64::MIN, f64::max) - c.iter().cloned().fold(f64::MAX, f64::min)
    };
    let verts: Vec<([f64; 3], f64)> =
        if vsegs.len() >= 2 && (guided || spread(&vsegs, true) > 0.3) { vsegs.iter().map(|s| (s.homog(), s.len())).collect() } else { vec![] };
    let hors: Vec<([f64; 3], f64)> =
        if hsegs.len() >= 2 && (guided || spread(&hsegs, false) > 0.2) { hsegs.iter().map(|s| (s.homog(), s.len())).collect() } else { vec![] };
    let rot2d = |deg: f64| -> Mat3d {
        let a = -deg.to_radians();
        [[a.cos(), -a.sin(), 0.0], [a.sin(), a.cos(), 0.0], [0.0, 0.0, 1.0]]
    };
    let mut r = IDENT;
    match mode {
        Upright::Off => return None,
        Upright::Level => {
            let a = level_angle(&long)?;
            return Some(rot2d(a));
        }
        _ => {}
    }
    let max_tilt = if guided { 40f64 } else { 25f64 }.to_radians();
    // vertical rectification
    let mut have_v = false;
    if verts.len() >= 2 {
        if let Some(vp) = vanishing_point(&verts) {
            let mut d = mv(&kinv, vp);
            if d[1] < 0.0 {
                d = d.map(|x| -x);
            }
            let dn = norm3(d);
            let ang = dn[1].clamp(-1.0, 1.0).acos();
            if ang <= max_tilt {
                // Auto corrects strong keystoning only partially, like ACR
                let frac = if mode == Upright::Auto && ang > 12f64.to_radians() { (12f64.to_radians() + (ang - 12f64.to_radians()) * 0.6) / ang } else { 1.0 };
                let so = ang.sin().max(1e-9);
                let a0 = ((1.0 - frac) * ang).sin() / so;
                let a1 = (frac * ang).sin() / so;
                let target = if frac >= 1.0 { [0.0, 1.0, 0.0] } else { [dn[0] * a0, dn[1] * a0 + a1, dn[2] * a0] };
                r = rodrigues(d, target);
                have_v = true;
            }
        }
    }
    if matches!(mode, Upright::Full | Upright::Guided) && hors.len() >= 2 {
        if let Some(hp) = vanishing_point(&hors) {
            let mut d = mv(&r, mv(&kinv, hp));
            if d[0] < 0.0 {
                d = d.map(|x| -x);
            }
            // rotate about y so the horizontal VP goes to infinity along x
            let phi = d[2].atan2(d[0]);
            if phi.abs() <= max_tilt {
                let ry = [[phi.cos(), 0.0, phi.sin()], [0.0, 1.0, 0.0], [-phi.sin(), 0.0, phi.cos()]];
                r = mm(&ry, &r);
                have_v = true;
            }
        }
    }
    if !have_v {
        // nothing to rectify: fall back to levelling
        let a = level_angle(&long)?;
        return Some(rot2d(a));
    }
    let mut h = mm(&k, &mm(&r, &kinv));
    // Vertical/Auto: level using only long, horizon-like lines
    if !matches!(mode, Upright::Full | Upright::Guided) {
        let leveled: Vec<Segment> = segs
            .iter()
            .filter(|s| s.len() > 0.6)
            .map(|s| {
                let a = apply_h(&h, s.a[0], s.a[1]);
                let b = apply_h(&h, s.b[0], s.b[1]);
                Segment { a: [a.0, a.1], b: [b.0, b.1] }
            })
            .filter(|s| s.angle().abs() < 10.0)
            .collect();
        if let Some(a) = level_angle(&leveled) {
            h = mm(&rot2d(a), &h);
        }
    }
    Some(sanitize(h, aspect))
}

/// Keep the centre fixed, preserve the frame's size and back off corrections
/// that would distort the image beyond what Upright would plausibly do.
fn sanitize(h: Mat3d, aspect: f64) -> Mat3d {
    let (ax, ay) = (1.0, 1.0 / aspect.max(1.0));
    let corners = [(-ax, -ay), (ax, -ay), (ax, ay), (-ax, ay)];
    let area = |m: &Mat3d| -> f64 {
        let p: Vec<(f64, f64)> = corners.iter().map(|&(x, y)| apply_h(m, x, y)).collect();
        let mut s = 0.0;
        for i in 0..4 {
            let (a, b) = (p[i], p[(i + 1) % 4]);
            s += a.0 * b.1 - b.0 * a.1;
        }
        s.abs() / 2.0
    };
    let normalise = |m: Mat3d| -> Mat3d {
        let mut m = m;
        let w = m[2][2];
        if w.abs() > 1e-12 {
            m = m.map(|r| r.map(|v| v / w));
        }
        let c = apply_h(&m, 0.0, 0.0);
        let t = [[1.0, 0.0, -c.0], [0.0, 1.0, -c.1], [0.0, 0.0, 1.0]];
        let m = mm(&t, &m);
        // same footprint as the original frame
        let a0 = 4.0 * ax * ay;
        let s = (a0 / area(&m).max(1e-9)).sqrt();
        mm(&[[s, 0.0, 0.0], [0.0, s, 0.0], [0.0, 0.0, 1.0]], &m)
    };
    let max_shift = |m: &Mat3d| -> f64 {
        corners
            .iter()
            .map(|&(x, y)| {
                let (u, v) = apply_h(m, x, y);
                ((u - x).powi(2) + (v - y).powi(2)).sqrt()
            })
            .fold(0.0, f64::max)
    };
    let full = normalise(h);
    let mut t = 1.0;
    let mut best = full;
    while t > 0.05 {
        let mut b = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                b[i][j] = IDENT[i][j] * (1.0 - t) + full[i][j] * t;
            }
        }
        let cand = normalise(b);
        if max_shift(&cand) < 0.45 {
            best = cand;
            break;
        }
        best = cand;
        t -= 0.1;
    }
    best
}

/// Analyse and return the flattened matrix to store in settings.
pub fn analyze(engine: &Engine, s: &DevelopSettings, mode: Upright) -> Option<[f32; 9]> {
    let segs = if mode == Upright::Guided { vec![] } else { detect_segments(engine, s) };
    let (fw, fh) = engine.full_size();
    let geo = crate::pipeline::geometry::Geometry::new(s, fw, fh, Default::default());
    // guides are stored in O-frame normalised coords
    let guides: Vec<Segment> = s
        .transform
        .guides
        .iter()
        .map(|g| {
            let c = |u: f32, v: f32| [((u as f64) - 0.5) * geo.ow / geo.half, ((v as f64) - 0.5) * geo.oh / geo.half];
            Segment { a: c(g[0], g[1]), b: c(g[2], g[3]) }
        })
        .collect();
    let h = compute_with_aspect(mode, &segs, &guides, geo.ow.max(geo.oh) / geo.ow.min(geo.oh))?;
    let mut out = [0f32; 9];
    for i in 0..3 {
        for j in 0..3 {
            out[i * 3 + j] = h[i][j] as f32;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_from_tilted_lines() {
        let t = 3f64.to_radians();
        let segs: Vec<Segment> = (0..5)
            .map(|i| {
                let y = -0.5 + i as f64 * 0.2;
                Segment { a: [-0.8, y - 0.8 * t.tan()], b: [0.8, y + 0.8 * t.tan()] }
            })
            .collect();
        let a = level_angle(&segs).unwrap();
        assert!((a - 3.0).abs() < 0.05, "{a}");
    }

    #[test]
    fn vertical_rectification_makes_lines_vertical() {
        // converging verticals (keystone): lines meeting above the image
        let vp = [0.0, -6.0];
        let segs: Vec<Segment> = [-0.6, -0.2, 0.2, 0.6]
            .iter()
            .map(|&x| {
                let b = [x, 0.8];
                let a = [x + (vp[0] - x) * 0.2, 0.8 + (vp[1] - 0.8) * 0.2];
                Segment { a, b }
            })
            .collect();
        let h = compute(Upright::Vertical, &segs, &[]).unwrap();
        for s in &segs {
            let a = apply_h(&h, s.a[0], s.a[1]);
            let b = apply_h(&h, s.b[0], s.b[1]);
            let ang = (b.1 - a.1).atan2(b.0 - a.0).to_degrees();
            assert!((ang.abs() - 90.0).abs() < 0.5, "{ang}");
        }
    }
}
