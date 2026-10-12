//! Float image buffers and the filtering primitives every stage leans on.

use rayon::prelude::*;

pub type Rgb = [f32; 3];

/// Interleaved RGB f32 image, row-major.
#[derive(Clone, Debug, Default)]
pub struct Image3 {
    pub w: usize,
    pub h: usize,
    pub data: Vec<Rgb>,
}

/// Single channel f32 image.
#[derive(Clone, Debug, Default)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl Image3 {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, data: vec![[0.0; 3]; w * h] }
    }

    pub fn from_fn(w: usize, h: usize, f: impl Fn(usize, usize) -> Rgb + Sync) -> Self {
        let mut img = Self::new(w, h);
        img.data.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
            for (x, p) in row.iter_mut().enumerate() {
                *p = f(x, y);
            }
        });
        img
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> Rgb {
        self.data[y * self.w + x]
    }

    #[inline]
    pub fn get_clamped(&self, x: isize, y: isize) -> Rgb {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.data[y * self.w + x]
    }

    /// Bilinear sample at continuous pixel coords (pixel centers at +0.5).
    pub fn sample_bilinear(&self, fx: f32, fy: f32) -> Rgb {
        let fx = fx - 0.5;
        let fy = fy - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let ax = fx - x0;
        let ay = fy - y0;
        let (x0, y0) = (x0 as isize, y0 as isize);
        let p00 = self.get_clamped(x0, y0);
        let p10 = self.get_clamped(x0 + 1, y0);
        let p01 = self.get_clamped(x0, y0 + 1);
        let p11 = self.get_clamped(x0 + 1, y0 + 1);
        let mut out = [0.0; 3];
        for c in 0..3 {
            let a = p00[c] + (p10[c] - p00[c]) * ax;
            let b = p01[c] + (p11[c] - p01[c]) * ax;
            out[c] = a + (b - a) * ay;
        }
        out
    }

    /// Bicubic (Catmull-Rom) sample at continuous pixel coords.
    pub fn sample_bicubic(&self, fx: f32, fy: f32) -> Rgb {
        let fx = fx - 0.5;
        let fy = fy - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let wx = catmull_weights(tx);
        let wy = catmull_weights(ty);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let mut out = [0.0f32; 3];
        for j in 0..4 {
            let mut row = [0.0f32; 3];
            for i in 0..4 {
                let p = self.get_clamped(x0 - 1 + i as isize, y0 - 1 + j as isize);
                for c in 0..3 {
                    row[c] += p[c] * wx[i];
                }
            }
            for c in 0..3 {
                out[c] += row[c] * wy[j];
            }
        }
        out
    }

    pub fn channel(&self, c: usize) -> Plane {
        Plane { w: self.w, h: self.h, data: self.data.par_iter().map(|p| p[c]).collect() }
    }

    pub fn map(&self, f: impl Fn(Rgb) -> Rgb + Sync) -> Image3 {
        Image3 { w: self.w, h: self.h, data: self.data.par_iter().map(|&p| f(p)).collect() }
    }

    pub fn map_inplace(&mut self, f: impl Fn(&mut Rgb) + Sync) {
        self.data.par_iter_mut().for_each(|p| f(p));
    }

    /// Area-average downscale by an integer factor.
    pub fn downscale_int(&self, f: usize) -> Image3 {
        if f <= 1 {
            return self.clone();
        }
        let w = self.w / f;
        let h = self.h / f;
        let norm = 1.0 / (f * f) as f32;
        Image3::from_fn(w, h, |x, y| {
            let mut acc = [0.0f32; 3];
            for yy in 0..f {
                let row = (y * f + yy) * self.w + x * f;
                for xx in 0..f {
                    let p = self.data[row + xx];
                    acc[0] += p[0];
                    acc[1] += p[1];
                    acc[2] += p[2];
                }
            }
            [acc[0] * norm, acc[1] * norm, acc[2] * norm]
        })
    }

    /// Resample to an arbitrary size. Box-filters when shrinking, bicubic when growing.
    pub fn resize(&self, w: usize, h: usize) -> Image3 {
        if w == self.w && h == self.h {
            return self.clone();
        }
        let mut src = std::borrow::Cow::Borrowed(self);
        let f = (self.w / w.max(1)).min(self.h / h.max(1));
        if f >= 2 {
            src = std::borrow::Cow::Owned(self.downscale_int(f));
        }
        let sx = src.w as f32 / w as f32;
        let sy = src.h as f32 / h as f32;
        if sx > 1.0 || sy > 1.0 {
            // remaining shrink < 2x: average a small footprint
            let src = &*src;
            Image3::from_fn(w, h, |x, y| {
                let cx = (x as f32 + 0.5) * sx;
                let cy = (y as f32 + 0.5) * sy;
                let mut acc = [0.0f32; 3];
                let offs = [-0.25f32, 0.25];
                for oy in offs {
                    for ox in offs {
                        let p = src.sample_bilinear(cx + ox * sx, cy + oy * sy);
                        for c in 0..3 {
                            acc[c] += p[c] * 0.25;
                        }
                    }
                }
                acc
            })
        } else {
            let src = &*src;
            Image3::from_fn(w, h, |x, y| src.sample_bicubic((x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy))
        }
    }

    pub fn crop(&self, x0: usize, y0: usize, w: usize, h: usize) -> Image3 {
        Image3::from_fn(w, h, |x, y| self.data[(y + y0) * self.w + x + x0])
    }

    pub fn luminance(&self) -> Plane {
        Plane { w: self.w, h: self.h, data: self.data.par_iter().map(|p| crate::color::luma_prophoto(*p)).collect() }
    }
}

#[inline]
fn catmull_weights(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2]
}

impl Plane {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, data: vec![0.0; w * h] }
    }

    pub fn filled(w: usize, h: usize, v: f32) -> Self {
        Self { w, h, data: vec![v; w * h] }
    }

    pub fn from_fn(w: usize, h: usize, f: impl Fn(usize, usize) -> f32 + Sync) -> Self {
        let mut p = Self::new(w, h);
        p.data.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
            for (x, v) in row.iter_mut().enumerate() {
                *v = f(x, y);
            }
        });
        p
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.w + x]
    }

    #[inline]
    pub fn get_clamped(&self, x: isize, y: isize) -> f32 {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.data[y * self.w + x]
    }

    pub fn sample_bilinear(&self, fx: f32, fy: f32) -> f32 {
        let fx = fx - 0.5;
        let fy = fy - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let ax = fx - x0;
        let ay = fy - y0;
        let (x0, y0) = (x0 as isize, y0 as isize);
        let a = self.get_clamped(x0, y0) * (1.0 - ax) + self.get_clamped(x0 + 1, y0) * ax;
        let b = self.get_clamped(x0, y0 + 1) * (1.0 - ax) + self.get_clamped(x0 + 1, y0 + 1) * ax;
        a + (b - a) * ay
    }

    pub fn map(&self, f: impl Fn(f32) -> f32 + Sync) -> Plane {
        Plane { w: self.w, h: self.h, data: self.data.par_iter().map(|&v| f(v)).collect() }
    }

    pub fn zip_map(&self, other: &Plane, f: impl Fn(f32, f32) -> f32 + Sync) -> Plane {
        Plane { w: self.w, h: self.h, data: self.data.par_iter().zip(other.data.par_iter()).map(|(&a, &b)| f(a, b)).collect() }
    }

    pub fn resize(&self, w: usize, h: usize) -> Plane {
        if w == self.w && h == self.h {
            return self.clone();
        }
        let sx = self.w as f32 / w as f32;
        let sy = self.h as f32 / h as f32;
        if sx > 1.5 || sy > 1.5 {
            // box average
            Plane::from_fn(w, h, |x, y| {
                let x0 = (x as f32 * sx) as usize;
                let x1 = (((x + 1) as f32 * sx) as usize).clamp(x0 + 1, self.w);
                let y0 = (y as f32 * sy) as usize;
                let y1 = (((y + 1) as f32 * sy) as usize).clamp(y0 + 1, self.h);
                let mut acc = 0.0;
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        acc += self.data[yy * self.w + xx];
                    }
                }
                acc / ((x1 - x0) * (y1 - y0)) as f32
            })
        } else {
            Plane::from_fn(w, h, |x, y| self.sample_bilinear((x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy))
        }
    }
}

// ---------------------------------------------------------------------------
// Blurs

/// Separable box blur with clamped edges, radius r (window 2r+1).
pub fn box_blur(p: &Plane, r: usize) -> Plane {
    if r == 0 || p.w == 0 || p.h == 0 {
        return p.clone();
    }
    let (w, h) = (p.w, p.h);
    let norm = 1.0 / (2 * r + 1) as f32;
    // horizontal
    let mut tmp = vec![0.0f32; w * h];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, out)| {
        let row = &p.data[y * w..(y + 1) * w];
        let at = |i: isize| row[i.clamp(0, w as isize - 1) as usize];
        let mut acc = 0.0f32;
        for i in -(r as isize)..=(r as isize) {
            acc += at(i);
        }
        for x in 0..w {
            out[x] = acc * norm;
            acc += at(x as isize + r as isize + 1) - at(x as isize - r as isize);
        }
    });
    // vertical, processed in column strips for cache friendliness
    let mut out = vec![0.0f32; w * h];
    const STRIP: usize = 64;
    let strips: Vec<usize> = (0..w).step_by(STRIP).collect();
    let out_ptr = SendPtr(out.as_mut_ptr());
    strips.par_iter().for_each(|&x0| {
        let x1 = (x0 + STRIP).min(w);
        let sw = x1 - x0;
        let mut acc = vec![0.0f32; sw];
        let at = |y: isize, x: usize| tmp[(y.clamp(0, h as isize - 1) as usize) * w + x];
        for i in -(r as isize)..=(r as isize) {
            for k in 0..sw {
                acc[k] += at(i, x0 + k);
            }
        }
        for y in 0..h {
            for k in 0..sw {
                // SAFETY: each strip writes a disjoint set of columns
                unsafe { *out_ptr.get().add(y * w + x0 + k) = acc[k] * norm };
                acc[k] += at(y as isize + r as isize + 1, x0 + k) - at(y as isize - r as isize, x0 + k);
            }
        }
    });
    Plane { w, h, data: out }
}

#[derive(Clone, Copy)]
struct SendPtr<T>(*mut T);
unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}
impl<T> SendPtr<T> {
    #[inline]
    fn get(self) -> *mut T {
        self.0
    }
}

/// Gaussian approximated by three box passes.
pub fn gaussian_blur(p: &Plane, sigma: f32) -> Plane {
    if sigma < 0.3 {
        return p.clone();
    }
    if sigma < 2.0 {
        return gaussian_blur_exact(p, sigma);
    }
    let radii = box_radii_for_gauss(sigma, 3);
    let mut out = box_blur(p, radii[0]);
    out = box_blur(&out, radii[1]);
    box_blur(&out, radii[2])
}

/// Direct separable gaussian for small sigmas where box approximations are coarse.
pub fn gaussian_blur_exact(p: &Plane, sigma: f32) -> Plane {
    let r = (sigma * 3.0).ceil() as isize;
    let mut k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let s: f32 = k.iter().sum();
    k.iter_mut().for_each(|v| *v /= s);
    let (w, h) = (p.w, p.h);
    let tmp = Plane::from_fn(w, h, |x, y| {
        let mut acc = 0.0;
        for (i, kv) in k.iter().enumerate() {
            acc += kv * p.get_clamped(x as isize + i as isize - r, y as isize);
        }
        acc
    });
    Plane::from_fn(w, h, |x, y| {
        let mut acc = 0.0;
        for (i, kv) in k.iter().enumerate() {
            acc += kv * tmp.get_clamped(x as isize, y as isize + i as isize - r);
        }
        acc
    })
}

fn box_radii_for_gauss(sigma: f32, n: usize) -> Vec<usize> {
    let w_ideal = (12.0 * sigma * sigma / n as f32 + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - (n as i32 * wl * wl) as f32 - 4.0 * n as f32 * wl as f32 - 3.0 * n as f32) / (-4.0 * wl as f32 - 4.0);
    let m = m_ideal.round() as i32;
    (0..n as i32).map(|i| (((if i < m { wl } else { wu }) - 1) / 2).max(0) as usize).collect()
}

pub fn gaussian_blur3(img: &Image3, sigma: f32) -> Image3 {
    let ch: Vec<Plane> = (0..3).into_par_iter().map(|c| gaussian_blur(&img.channel(c), sigma)).collect();
    merge3(&ch[0], &ch[1], &ch[2])
}

pub fn merge3(r: &Plane, g: &Plane, b: &Plane) -> Image3 {
    Image3 { w: r.w, h: r.h, data: r.data.par_iter().zip(g.data.par_iter()).zip(b.data.par_iter()).map(|((&r, &g), &b)| [r, g, b]).collect() }
}

/// He et al. guided filter: edge-aware smoothing of `p` guided by `i`.
pub fn guided_filter(i: &Plane, p: &Plane, r: usize, eps: f32) -> Plane {
    let mean_i = box_blur(i, r);
    let mean_p = box_blur(p, r);
    let ip = i.zip_map(p, |a, b| a * b);
    let ii = i.map(|a| a * a);
    let mean_ip = box_blur(&ip, r);
    let mean_ii = box_blur(&ii, r);
    let n = i.data.len();
    let mut a = Plane::new(i.w, i.h);
    let mut b = Plane::new(i.w, i.h);
    a.data.par_iter_mut().zip(b.data.par_iter_mut()).enumerate().for_each(|(k, (av, bv))| {
        let cov = mean_ip.data[k] - mean_i.data[k] * mean_p.data[k];
        let var = mean_ii.data[k] - mean_i.data[k] * mean_i.data[k];
        *av = cov / (var + eps);
        *bv = mean_p.data[k] - *av * mean_i.data[k];
    });
    let _ = n;
    let ma = box_blur(&a, r);
    let mb = box_blur(&b, r);
    Plane { w: i.w, h: i.h, data: i.data.par_iter().zip(ma.data.par_iter()).zip(mb.data.par_iter()).map(|((&iv, &a), &b)| a * iv + b).collect() }
}

/// Guided filter that runs on a subsampled copy (fast guided filter, He 2015).
pub fn fast_guided_filter(i: &Plane, p: &Plane, r: usize, eps: f32, sub: usize) -> Plane {
    if sub <= 1 || i.w / sub < 8 || i.h / sub < 8 {
        return guided_filter(i, p, r, eps);
    }
    let (sw, sh) = (i.w / sub, i.h / sub);
    let is = i.resize(sw, sh);
    let ps = p.resize(sw, sh);
    let rs = (r / sub).max(1);
    let mean_i = box_blur(&is, rs);
    let mean_p = box_blur(&ps, rs);
    let mean_ip = box_blur(&is.zip_map(&ps, |a, b| a * b), rs);
    let mean_ii = box_blur(&is.map(|a| a * a), rs);
    let mut a = Plane::new(sw, sh);
    let mut b = Plane::new(sw, sh);
    a.data.par_iter_mut().zip(b.data.par_iter_mut()).enumerate().for_each(|(k, (av, bv))| {
        let cov = mean_ip.data[k] - mean_i.data[k] * mean_p.data[k];
        let var = mean_ii.data[k] - mean_i.data[k] * mean_i.data[k];
        *av = cov / (var + eps);
        *bv = mean_p.data[k] - *av * mean_i.data[k];
    });
    let ma = box_blur(&a, rs).resize(i.w, i.h);
    let mb = box_blur(&b, rs).resize(i.w, i.h);
    Plane { w: i.w, h: i.h, data: i.data.par_iter().zip(ma.data.par_iter()).zip(mb.data.par_iter()).map(|((&iv, &a), &b)| a * iv + b).collect() }
}

/// Min filter (erosion) over a square window, separable.
pub fn min_filter(p: &Plane, r: usize) -> Plane {
    let h1 = Plane::from_fn(p.w, p.h, |x, y| {
        let mut m = f32::MAX;
        for i in -(r as isize)..=(r as isize) {
            m = m.min(p.get_clamped(x as isize + i, y as isize));
        }
        m
    });
    Plane::from_fn(p.w, p.h, |x, y| {
        let mut m = f32::MAX;
        for i in -(r as isize)..=(r as isize) {
            m = m.min(h1.get_clamped(x as isize, y as isize + i));
        }
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_blur_preserves_constant() {
        let p = Plane::filled(37, 23, 0.5);
        let b = box_blur(&p, 4);
        assert!(b.data.iter().all(|v| (v - 0.5).abs() < 1e-5));
        let g = gaussian_blur(&p, 7.0);
        assert!(g.data.iter().all(|v| (v - 0.5).abs() < 1e-4));
    }

    #[test]
    fn guided_preserves_constant() {
        let p = Plane::from_fn(40, 30, |x, _| if x < 20 { 0.1 } else { 0.9 });
        let g = guided_filter(&p, &p, 4, 1e-4);
        // edge preserved
        assert!((g.get(5, 5) - 0.1).abs() < 0.02);
        assert!((g.get(35, 5) - 0.9).abs() < 0.02);
    }
}
