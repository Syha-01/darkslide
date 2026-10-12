//! Curves: monotone splines, LUTs, the parametric curve, point curves.

use crate::params::{CurvePoints, ParametricCurve};

/// Monotone cubic (Fritsch–Carlson) interpolation through sorted points.
#[derive(Clone, Debug)]
pub struct Spline {
    xs: Vec<f32>,
    ys: Vec<f32>,
    ms: Vec<f32>,
}

impl Spline {
    pub fn new(points: &[[f32; 2]]) -> Self {
        let mut pts: Vec<[f32; 2]> = points.to_vec();
        pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
        pts.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-6);
        if pts.len() == 1 {
            pts.push([pts[0][0] + 1.0, pts[0][1] + 1.0]);
        }
        let n = pts.len();
        let xs: Vec<f32> = pts.iter().map(|p| p[0]).collect();
        let ys: Vec<f32> = pts.iter().map(|p| p[1]).collect();
        let d: Vec<f32> = (0..n - 1).map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i])).collect();
        let mut ms = vec![0.0f32; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            ms[i] = if d[i - 1] * d[i] <= 0.0 { 0.0 } else { (d[i - 1] + d[i]) * 0.5 };
        }
        for i in 0..n - 1 {
            if d[i] == 0.0 {
                ms[i] = 0.0;
                ms[i + 1] = 0.0;
                continue;
            }
            let a = ms[i] / d[i];
            let b = ms[i + 1] / d[i];
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                ms[i] = t * a * d[i];
                ms[i + 1] = t * b * d[i];
            }
        }
        Self { xs, ys, ms }
    }

    pub fn eval(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0] + (x - self.xs[0]) * self.ms[0].min(0.0).max(0.0);
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1];
        }
        let i = match self.xs.binary_search_by(|v| v.partial_cmp(&x).unwrap()) {
            Ok(i) => return self.ys[i],
            Err(i) => i - 1,
        };
        let h = self.xs[i + 1] - self.xs[i];
        let t = (x - self.xs[i]) / h;
        let t2 = t * t;
        let t3 = t2 * t;
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        h00 * self.ys[i] + h10 * h * self.ms[i] + h01 * self.ys[i + 1] + h11 * h * self.ms[i + 1]
    }
}

/// 1D lookup table on [0,1]; values beyond 1 extrapolate with the end slope.
#[derive(Clone, Debug)]
pub struct Lut {
    pub v: Vec<f32>,
    identity: bool,
}

pub const LUT_SIZE: usize = 4096;

impl Lut {
    pub fn identity() -> Self {
        Self { v: (0..LUT_SIZE).map(|i| i as f32 / (LUT_SIZE - 1) as f32).collect(), identity: true }
    }

    pub fn from_fn(f: impl Fn(f32) -> f32) -> Self {
        Self { v: (0..LUT_SIZE).map(|i| f(i as f32 / (LUT_SIZE - 1) as f32)).collect(), identity: false }
    }

    pub fn is_identity(&self) -> bool {
        self.identity
    }

    #[inline]
    pub fn eval(&self, x: f32) -> f32 {
        if self.identity {
            return x;
        }
        let n = (LUT_SIZE - 1) as f32;
        if x <= 0.0 {
            return self.v[0] + x * (self.v[1] - self.v[0]) * n;
        }
        if x >= 1.0 {
            let slope = (self.v[LUT_SIZE - 1] - self.v[LUT_SIZE - 2]) * n;
            return self.v[LUT_SIZE - 1] + (x - 1.0) * slope;
        }
        let f = x * n;
        let i = f as usize;
        let t = f - i as f32;
        self.v[i] + (self.v[(i + 1).min(LUT_SIZE - 1)] - self.v[i]) * t
    }

    /// Compose: self(other(x)).
    pub fn after(&self, other: &Lut) -> Lut {
        if self.identity {
            return other.clone();
        }
        if other.identity {
            return self.clone();
        }
        Self { v: other.v.iter().map(|&x| self.eval(x)).collect(), identity: false }
    }
}

/// Point curve (ACR 0..255 control points) as a LUT on [0,1].
pub fn point_curve_lut(points: &CurvePoints) -> Lut {
    if crate::params::is_linear_curve(points) || points.len() < 2 {
        return Lut::identity();
    }
    let pts: Vec<[f32; 2]> = points.iter().map(|p| [p[0] / 255.0, p[1] / 255.0]).collect();
    let s = Spline::new(&pts);
    let x_lo = pts.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
    let x_hi = pts.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
    let y_lo = s.eval(x_lo);
    let y_hi = s.eval(x_hi);
    Lut::from_fn(|x| {
        if x <= x_lo {
            y_lo
        } else if x >= x_hi {
            y_hi
        } else {
            s.eval(x).clamp(0.0, 1.0)
        }
    })
}

/// The parametric curve: four region sliders separated by three splits.
pub fn parametric_lut(p: &ParametricCurve) -> Lut {
    if p.is_identity() {
        return Lut::identity();
    }
    let s1 = (p.shadow_split / 100.0).clamp(0.05, 0.9);
    let s2 = (p.midtone_split / 100.0).clamp(s1 + 0.02, 0.95);
    let s3 = (p.highlight_split / 100.0).clamp(s2 + 0.02, 0.98);
    // region centres and half-widths; each bump spreads into its neighbours
    let regions = [(p.shadows, 0.0, s1), (p.darks, s1, s2), (p.lights, s2, s3), (p.highlights, s3, 1.0)];
    let bump = |x: f32, a: f32, b: f32| -> f32 {
        let c = (a + b) * 0.5;
        let hw = (b - a) * 0.5 + 0.12;
        let t = ((x - c) / hw).clamp(-1.0, 1.0);
        let w = (std::f32::consts::FRAC_PI_2 * t).cos();
        w * w
    };
    let raw: Vec<f32> = (0..LUT_SIZE)
        .map(|i| {
            let x = i as f32 / (LUT_SIZE - 1) as f32;
            let mut y = x;
            for (amt, a, b) in regions {
                if amt != 0.0 {
                    // keep endpoints pinned
                    let pin = (x * (1.0 - x) * 4.0).sqrt().min(1.0);
                    y += amt / 100.0 * 0.16 * bump(x, a, b) * pin;
                }
            }
            y
        })
        .collect();
    // enforce monotonicity
    let mut v = raw;
    for i in 1..v.len() {
        if v[i] < v[i - 1] {
            v[i] = v[i - 1];
        }
    }
    for x in v.iter_mut() {
        *x = x.clamp(0.0, 1.0);
    }
    Lut { v, identity: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_passes_through_points_and_is_monotone() {
        let s = Spline::new(&[[0.0, 0.0], [0.25, 0.15], [0.75, 0.85], [1.0, 1.0]]);
        assert!((s.eval(0.25) - 0.15).abs() < 1e-5);
        let mut last = -1.0;
        for i in 0..=100 {
            let y = s.eval(i as f32 / 100.0);
            assert!(y >= last - 1e-6);
            last = y;
        }
    }

    #[test]
    fn parametric_identity_and_direction() {
        let mut p = ParametricCurve::default();
        assert!(parametric_lut(&p).is_identity());
        p.shadows = 100.0;
        let l = parametric_lut(&p);
        assert!(l.eval(0.12) > 0.12);
        assert!((l.eval(0.0)).abs() < 1e-3);
        assert!((l.eval(1.0) - 1.0).abs() < 1e-3);
    }
}
