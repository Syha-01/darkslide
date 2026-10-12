//! Geometry: user rotate/flip, perspective transform (Upright + manual),
//! lens distortion & lateral CA, straighten + crop. Everything is expressed as
//! an inverse map from output (crop frame) to source pixels so one resample
//! does all of it.
//!
//! Frames:
//! - S: source as cached (EXIF-oriented), pixels.
//! - O: S plus user rotate90/flip. Normalised (u,v) in [0,1].
//! - T: O after perspective + lens correction (same normalised extent as O).
//!   Masks live here.
//! - C: crop frame, normalised [0,1]; the crop rect is axis-aligned in T after
//!   rotating T by the straighten angle about its centre.

use rayon::prelude::*;

use crate::image::{Image3, Plane, Rgb};
use crate::params::{Crop, DevelopSettings};

pub type Mat3d = [[f64; 3]; 3];

fn mat_mul(a: &Mat3d, b: &Mat3d) -> Mat3d {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

pub fn mat_inv(m: &Mat3d) -> Mat3d {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-15 {
        return IDENT;
    }
    let i = 1.0 / det;
    [
        [(m[1][1] * m[2][2] - m[2][1] * m[1][2]) * i, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * i, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * i],
        [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) * i, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * i, (m[1][0] * m[0][2] - m[0][0] * m[1][2]) * i],
        [(m[1][0] * m[2][1] - m[2][0] * m[1][1]) * i, (m[2][0] * m[0][1] - m[0][0] * m[2][1]) * i, (m[0][0] * m[1][1] - m[1][0] * m[0][1]) * i],
    ]
}

pub const IDENT: Mat3d = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

#[inline]
pub fn apply_h(h: &Mat3d, x: f64, y: f64) -> (f64, f64) {
    let w = h[2][0] * x + h[2][1] * y + h[2][2];
    let w = if w.abs() < 1e-12 { 1e-12 } else { w };
    ((h[0][0] * x + h[0][1] * y + h[0][2]) / w, (h[1][0] * x + h[1][1] * y + h[1][2]) / w)
}

/// Builds the manual perspective homography in centred coords (long edge
/// half-size = 1), mapping O -> T.
pub fn manual_homography(t: &crate::params::Transform, aspect: f64) -> Mat3d {
    let f = 1.6; // focal length in half-long-edge units
    let k = [[f, 0.0, 0.0], [0.0, f, 0.0], [0.0, 0.0, 1.0]];
    let kinv = mat_inv(&k);
    let th_v = (t.vertical as f64 / 100.0) * 25f64.to_radians();
    let th_h = (t.horizontal as f64 / 100.0) * 25f64.to_radians();
    let rx = [[1.0, 0.0, 0.0], [0.0, th_v.cos(), -th_v.sin()], [0.0, th_v.sin(), th_v.cos()]];
    let ry = [[th_h.cos(), 0.0, th_h.sin()], [0.0, 1.0, 0.0], [-th_h.sin(), 0.0, th_h.cos()]];
    let persp = mat_mul(&k, &mat_mul(&ry, &mat_mul(&rx, &kinv)));
    let a = t.rotate as f64 * std::f64::consts::PI / 180.0;
    let rot = [[a.cos(), -a.sin(), 0.0], [a.sin(), a.cos(), 0.0], [0.0, 0.0, 1.0]];
    let asp = 1.0 + t.aspect as f64 / 100.0 * 0.5;
    let s = t.scale as f64 / 100.0;
    let scale = [[s * asp.sqrt(), 0.0, 0.0], [0.0, s / asp.sqrt(), 0.0], [0.0, 0.0, 1.0]];
    let (ox, oy) = if aspect >= 1.0 { (1.0, 1.0 / aspect) } else { (aspect, 1.0) };
    let off = [[1.0, 0.0, t.offset_x as f64 / 100.0 * ox], [0.0, 1.0, t.offset_y as f64 / 100.0 * oy], [0.0, 0.0, 1.0]];
    let mut h = mat_mul(&off, &mat_mul(&scale, &mat_mul(&rot, &persp)));
    if let Some(u) = t.upright_matrix {
        let um = [[u[0] as f64, u[1] as f64, u[2] as f64], [u[3] as f64, u[4] as f64, u[5] as f64], [u[6] as f64, u[7] as f64, u[8] as f64]];
        h = mat_mul(&h, &um);
    }
    h
}

/// Radial lens model applied when mapping corrected -> source coords.
#[derive(Clone, Debug, Default)]
pub struct LensModel {
    /// Polynomial r_src = r * (1 + k1 r^2 + k2 r^4 + k3 r^6) per channel scale
    pub k: [f64; 3],
    /// Lateral CA: radial scale of R and B relative to G.
    pub ca_scale: [f64; 2],
    /// lensfun ptlens (a, b, c), radius normalised to the half short side.
    pub ptlens: Option<[f64; 3]>,
}

impl LensModel {
    pub fn is_identity(&self) -> bool {
        self.k == [0.0; 3] && self.ca_scale == [0.0; 2] && self.ptlens.is_none()
    }

    #[inline]
    fn radial(&self, r2: f64) -> f64 {
        1.0 + self.k[0] * r2 + self.k[1] * r2 * r2 + self.k[2] * r2 * r2 * r2
    }
}

#[derive(Clone, Debug)]
pub struct Geometry {
    /// O-frame size in full-resolution pixels.
    pub ow: f64,
    pub oh: f64,
    /// EXIF-oriented source size in full-resolution pixels.
    pub sw: f64,
    pub sh: f64,
    pub rotate90: u8,
    pub flip_h: bool,
    pub flip_v: bool,
    /// T -> O inverse homography (centred coords).
    pub h_inv: Mat3d,
    pub identity_h: bool,
    pub lens: LensModel,
    pub crop: Crop,
    /// Cached centred-coordinate scale (half the long edge).
    pub half: f64,
    angle_sin: f64,
    angle_cos: f64,
}

impl Geometry {
    pub fn new(s: &DevelopSettings, src_w: usize, src_h: usize, lens: LensModel) -> Self {
        let (sw, sh) = (src_w as f64, src_h as f64);
        let r90 = s.crop.rotate90 % 4;
        let (ow, oh) = if r90 % 2 == 1 { (sh, sw) } else { (sw, sh) };
        let geometry_on = s.panels.geometry;
        let h = if geometry_on { manual_homography(&s.transform, ow / oh) } else { IDENT };
        let identity_h = h == IDENT;
        let a = (s.crop.angle as f64).to_radians();
        let lens = if s.panels.optics { lens } else { LensModel::default() };
        Self {
            ow,
            oh,
            sw,
            sh,
            rotate90: r90,
            flip_h: s.crop.flip_h,
            flip_v: s.crop.flip_v,
            h_inv: mat_inv(&h),
            identity_h,
            lens,
            crop: s.crop,
            half: ow.max(oh) / 2.0,
            angle_sin: a.sin(),
            angle_cos: a.cos(),
        }
    }

    /// Crop size in full-resolution pixels.
    pub fn crop_size(&self) -> (f64, f64) {
        ((self.crop.right - self.crop.left) as f64 * self.ow, (self.crop.bottom - self.crop.top) as f64 * self.oh)
    }

    /// Same as `new` but ignoring crop (for the crop tool's full view).
    pub fn without_crop(mut self) -> Self {
        self.crop = Crop { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0, angle: 0.0, ..self.crop };
        self.angle_sin = 0.0;
        self.angle_cos = 1.0;
        self
    }

    /// Crop-frame normalised -> T-frame normalised.
    #[inline]
    pub fn crop_to_t(&self, cx: f64, cy: f64) -> (f64, f64) {
        let c = &self.crop;
        let qx = (c.left as f64 + cx * (c.right - c.left) as f64 - 0.5) * self.ow;
        let qy = (c.top as f64 + cy * (c.bottom - c.top) as f64 - 0.5) * self.oh;
        // rotate about the centre by the straighten angle
        let rx = qx * self.angle_cos - qy * self.angle_sin;
        let ry = qx * self.angle_sin + qy * self.angle_cos;
        (rx / self.ow + 0.5, ry / self.oh + 0.5)
    }

    /// T-frame normalised -> crop-frame normalised (inverse of crop_to_t).
    pub fn t_to_crop(&self, u: f64, v: f64) -> (f64, f64) {
        let c = &self.crop;
        let x = (u - 0.5) * self.ow;
        let y = (v - 0.5) * self.oh;
        let qx = x * self.angle_cos + y * self.angle_sin;
        let qy = -x * self.angle_sin + y * self.angle_cos;
        let nx = qx / self.ow + 0.5;
        let ny = qy / self.oh + 0.5;
        ((nx - c.left as f64) / (c.right - c.left) as f64, (ny - c.top as f64) / (c.bottom - c.top) as f64)
    }

    /// T-frame normalised -> O-frame normalised, for channel `ch` (CA aware).
    #[inline]
    pub fn t_to_o(&self, u: f64, v: f64, ch: usize) -> (f64, f64) {
        let mut x = (u - 0.5) * self.ow / self.half;
        let mut y = (v - 0.5) * self.oh / self.half;
        if !self.identity_h {
            let p = apply_h(&self.h_inv, x, y);
            x = p.0;
            y = p.1;
        }
        if !self.lens.is_identity() {
            // normalise radius to the half-diagonal
            let rn = (self.ow * self.ow + self.oh * self.oh).sqrt() / 2.0 / self.half;
            let r2 = (x * x + y * y) / (rn * rn);
            let mut f = self.lens.radial(r2);
            if let Some([a, b, c]) = self.lens.ptlens {
                // radius in half-short-side units
                let rs = (x * x + y * y).sqrt() * self.half / (self.ow.min(self.oh) / 2.0);
                f *= a * rs * rs * rs + b * rs * rs + c * rs + 1.0 - a - b - c;
            }
            if ch == 0 {
                f *= 1.0 + self.lens.ca_scale[0];
            } else if ch == 2 {
                f *= 1.0 + self.lens.ca_scale[1];
            }
            x *= f;
            y *= f;
        }
        (x * self.half / self.ow + 0.5, y * self.half / self.oh + 0.5)
    }

    /// O-frame normalised -> S-frame normalised (undo user rotate/flip).
    #[inline]
    pub fn o_to_s(&self, u: f64, v: f64) -> (f64, f64) {
        let (mut u, mut v) = (u, v);
        if self.flip_h {
            u = 1.0 - u;
        }
        if self.flip_v {
            v = 1.0 - v;
        }
        match self.rotate90 {
            1 => (v, 1.0 - u),
            2 => (1.0 - u, 1.0 - v),
            3 => (1.0 - v, u),
            _ => (u, v),
        }
    }

    /// S-frame normalised -> O-frame normalised.
    pub fn s_to_o(&self, u: f64, v: f64) -> (f64, f64) {
        let (mut u, mut v) = match self.rotate90 {
            1 => (1.0 - v, u),
            2 => (1.0 - u, 1.0 - v),
            3 => (v, 1.0 - u),
            _ => (u, v),
        };
        if self.flip_h {
            u = 1.0 - u;
        }
        if self.flip_v {
            v = 1.0 - v;
        }
        (u, v)
    }

    /// True when every point along the crop border maps inside the source.
    pub fn crop_inside_source(&self) -> bool {
        let n = 12;
        (0..4 * n).all(|k| {
            let t = (k % n) as f64 / n as f64;
            let (cx, cy) = match k / n {
                0 => (t, 0.0),
                1 => (1.0, t),
                2 => (1.0 - t, 1.0),
                _ => (0.0, 1.0 - t),
            };
            let (tu, tv) = self.crop_to_t(cx, cy);
            let (ou, ov) = self.t_to_o(tu, tv, 1);
            (-1e-3..=1.001).contains(&ou) && (-1e-3..=1.001).contains(&ov)
        })
    }

    /// Forward O -> T (iterative inverse of t_to_o, used for UI overlays).
    pub fn o_to_t(&self, u: f64, v: f64) -> (f64, f64) {
        let (mut tu, mut tv) = (u, v);
        for _ in 0..20 {
            let (ou, ov) = self.t_to_o(tu, tv, 1);
            let (du, dv) = (u - ou, v - ov);
            tu += du;
            tv += dv;
            if du.abs() + dv.abs() < 1e-9 {
                break;
            }
        }
        (tu, tv)
    }
}

/// A rectangular region of the crop frame rendered at a given pixel size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub w: usize,
    pub h: usize,
}

impl Region {
    pub fn full(w: usize, h: usize) -> Self {
        Self { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0, w, h }
    }

    #[inline]
    pub fn crop_coord(&self, x: usize, y: usize) -> (f64, f64) {
        (self.x0 + (x as f64 + 0.5) / self.w as f64 * (self.x1 - self.x0), self.y0 + (y as f64 + 0.5) / self.h as f64 * (self.y1 - self.y0))
    }
}

/// Resample `src` (S frame, any resolution) into the region. Returns the
/// image, coverage alpha and the T-frame coords of every output pixel.
pub fn warp(src: &Image3, geo: &Geometry, region: &Region) -> (Image3, Plane, Vec<[f32; 2]>) {
    let (w, h) = (region.w, region.h);
    let mut out = Image3::new(w, h);
    let mut alpha = Plane::new(w, h);
    let mut tco = vec![[0.0f32; 2]; w * h];
    // source pixels per output pixel, to pick a filter
    let crop_w = (geo.crop.right - geo.crop.left) as f64 * (region.x1 - region.x0);
    let src_per_out = crop_w * geo.ow / w as f64 * (src.w as f64 / geo.sw);
    let downsampling = src_per_out > 1.6;
    let pre: std::borrow::Cow<Image3> = if downsampling {
        // pre-blur proportional to the shrink so the bilinear tap doesn't alias
        let f = (src_per_out / 1.2).floor() as usize;
        if f >= 2 { std::borrow::Cow::Owned(src.downscale_int(f)) } else { std::borrow::Cow::Borrowed(src) }
    } else {
        std::borrow::Cow::Borrowed(src)
    };
    let src = &*pre;
    // residual footprint (in `src` pixels) of one output pixel after the integer pre-shrink
    let residual = (crop_w * geo.ow / w as f64 * (src.w as f64 / geo.sw)) as f32;
    let no_ca = geo.lens.ca_scale == [0.0; 2];
    let (fw, fh) = (src.w as f64, src.h as f64);
    out.data.par_chunks_mut(w).zip(alpha.data.par_chunks_mut(w)).zip(tco.par_chunks_mut(w)).enumerate().for_each(|(y, ((row, arow), trow))| {
        for x in 0..w {
            let (cx, cy) = region.crop_coord(x, y);
            let (tu, tv) = geo.crop_to_t(cx, cy);
            trow[x] = [tu as f32, tv as f32];
            let mut px: Rgb = [0.0; 3];
            let mut inside = true;
            let chans: &[usize] = if no_ca { &[1] } else { &[0, 1, 2] };
            for &ch in chans {
                let (ou, ov) = geo.t_to_o(tu, tv, ch);
                let (su, sv) = geo.o_to_s(ou, ov);
                if !(-0.001..=1.001).contains(&su) || !(-0.001..=1.001).contains(&sv) {
                    inside = false;
                }
                let fx = (su * fw) as f32;
                let fy = (sv * fh) as f32;
                let p = if downsampling && residual > 1.05 {
                    // average four taps over the footprint so the result doesn't depend on sub-pixel phase
                    let o = residual * 0.25;
                    let a = src.sample_bilinear(fx - o, fy - o);
                    let b = src.sample_bilinear(fx + o, fy - o);
                    let c = src.sample_bilinear(fx - o, fy + o);
                    let d = src.sample_bilinear(fx + o, fy + o);
                    [0, 1, 2].map(|k| (a[k] + b[k] + c[k] + d[k]) * 0.25)
                } else if downsampling {
                    src.sample_bilinear(fx, fy)
                } else {
                    src.sample_bicubic(fx, fy)
                };
                if no_ca {
                    px = p;
                } else {
                    px[ch] = p[ch];
                }
            }
            row[x] = [px[0].max(0.0), px[1].max(0.0), px[2].max(0.0)];
            arow[x] = if inside { 1.0 } else { 0.0 };
        }
    });
    (out, alpha, tco)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_geometry_roundtrip() {
        let s = DevelopSettings::default();
        let g = Geometry::new(&s, 600, 400, LensModel::default());
        let (u, v) = g.crop_to_t(0.25, 0.75);
        assert!((u - 0.25).abs() < 1e-9 && (v - 0.75).abs() < 1e-9);
        let (a, b) = g.t_to_o(u, v, 1);
        assert!((a - 0.25).abs() < 1e-9 && (b - 0.75).abs() < 1e-9);
    }

    #[test]
    fn crop_inverse() {
        let mut s = DevelopSettings::default();
        s.crop = Crop { left: 0.1, top: 0.2, right: 0.8, bottom: 0.9, angle: 7.0, ..Default::default() };
        let g = Geometry::new(&s, 600, 400, LensModel::default());
        let (u, v) = g.crop_to_t(0.3, 0.6);
        let (cx, cy) = g.t_to_crop(u, v);
        assert!((cx - 0.3).abs() < 1e-9 && (cy - 0.6).abs() < 1e-9);
    }

    #[test]
    fn rotate90_maps_corners() {
        let mut s = DevelopSettings::default();
        s.crop.rotate90 = 1;
        let g = Geometry::new(&s, 600, 400, LensModel::default());
        // top-left of a clockwise-rotated image comes from bottom-left of the source
        let (u, v) = g.o_to_s(0.0, 0.0);
        assert!((u - 0.0).abs() < 1e-9 && (v - 1.0).abs() < 1e-9);
        let (a, b) = g.s_to_o(u, v);
        assert!(a.abs() < 1e-9 && b.abs() < 1e-9);
    }
}
