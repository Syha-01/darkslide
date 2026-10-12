//! Profiles (base rendering) and creative looks (3D LUTs).
//!
//! A profile sets the scene->display tone curve, a saturation baseline and
//! per-band hue/sat tweaks. Looks are 3D LUTs applied on the display-encoded
//! image with an amount (0..200).

use std::path::Path;

use anyhow::{Result, anyhow};

use crate::image::Rgb;
use crate::pipeline::curves::Spline;

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: &'static str,
    pub group: &'static str,
    /// Contrast scale applied in log space before the base curve.
    pub contrast: f32,
    pub saturation: f32,
    /// Per HSL band (8) saturation multipliers.
    pub band_sat: [f32; 8],
    pub band_hue: [f32; 8],
    pub monochrome: bool,
    /// Adaptive: automatic local tone mapping driven by image statistics.
    pub adaptive: bool,
}

const ONES: [f32; 8] = [1.0; 8];
const ZEROS: [f32; 8] = [0.0; 8];

pub const PROFILES: &[Profile] = &[
    Profile { name: "Color", group: "Basic", contrast: 1.0, saturation: 1.0, band_sat: ONES, band_hue: ZEROS, monochrome: false, adaptive: false },
    Profile { name: "Standard", group: "Basic", contrast: 0.93, saturation: 0.94, band_sat: ONES, band_hue: ZEROS, monochrome: false, adaptive: false },
    Profile {
        name: "Landscape",
        group: "Basic",
        contrast: 1.05,
        saturation: 1.08,
        band_sat: [1.0, 1.0, 1.08, 1.18, 1.12, 1.15, 1.0, 1.0],
        band_hue: [0.0, 0.0, 0.0, 3.0, 0.0, -2.0, 0.0, 0.0],
        monochrome: false,
        adaptive: false,
    },
    Profile {
        name: "Portrait",
        group: "Basic",
        contrast: 0.95,
        saturation: 0.98,
        band_sat: [0.94, 0.9, 0.95, 1.0, 1.0, 1.0, 1.0, 1.0],
        band_hue: [0.0, -2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        monochrome: false,
        adaptive: false,
    },
    Profile { name: "Vivid", group: "Basic", contrast: 1.1, saturation: 1.22, band_sat: ONES, band_hue: ZEROS, monochrome: false, adaptive: false },
    Profile { name: "Neutral", group: "Basic", contrast: 0.82, saturation: 0.9, band_sat: ONES, band_hue: ZEROS, monochrome: false, adaptive: false },
    Profile { name: "Monochrome", group: "Basic", contrast: 1.02, saturation: 0.0, band_sat: ONES, band_hue: ZEROS, monochrome: true, adaptive: false },
    Profile { name: "Adaptive Color", group: "Adaptive", contrast: 1.0, saturation: 1.02, band_sat: ONES, band_hue: ZEROS, monochrome: false, adaptive: true },
    Profile { name: "Adaptive B&W", group: "Adaptive", contrast: 1.0, saturation: 0.0, band_sat: ONES, band_hue: ZEROS, monochrome: true, adaptive: true },
];

pub fn find(name: &str) -> &'static Profile {
    // accept ACR names too
    let n = name.trim_start_matches("Adobe ").trim_end_matches(" (beta)");
    PROFILES.iter().find(|p| p.name.eq_ignore_ascii_case(n)).unwrap_or(&PROFILES[0])
}

/// The base scene->display curve: input is stops relative to 18% grey,
/// output is perceptually encoded (sRGB transfer) display value.
pub struct BaseCurve {
    spline: Spline,
    contrast: f32,
    pub white_stops: f32,
    /// DCP tone curve on linear values (replaces the built-in curve).
    dcp: Option<Spline>,
}

const MID_LINEAR: f32 = 0.18;

const BASE_POINTS: [[f32; 2]; 13] = [
    [-12.0, 0.0],
    [-9.0, 0.002],
    [-6.5, 0.014],
    [-5.0, 0.034],
    [-4.0, 0.062],
    [-3.0, 0.108],
    [-2.0, 0.18],
    [-1.0, 0.305],
    [0.0, 0.47],
    [1.0, 0.64],
    [2.0, 0.795],
    [2.7, 0.905],
    [3.6, 1.0],
];

impl BaseCurve {
    pub fn new(profile: &Profile) -> Self {
        Self { spline: Spline::new(&BASE_POINTS), contrast: profile.contrast, white_stops: 3.6 / profile.contrast, dcp: None }
    }

    pub fn from_dcp(points: &[[f32; 2]]) -> Self {
        let mut c = Self::new(&PROFILES[0]);
        if points.len() >= 2 {
            c.dcp = Some(Spline::new(points));
        }
        c
    }

    /// Stops -> encoded display value. Values above white extend linearly
    /// (in encoded space) so HDR output keeps highlight gradation.
    #[inline]
    pub fn eval(&self, stops: f32) -> f32 {
        if let Some(sp) = &self.dcp {
            // DCP curves map linear scene values (1.0 = white) to linear output
            let y = MID_LINEAR * stops.exp2();
            if y >= 1.0 {
                return 1.0 + (y.log2()) * 0.09;
            }
            return crate::color::srgb_encode(sp.eval(y).clamp(0.0, 1.0));
        }
        let l = stops * self.contrast;
        if l >= 3.6 {
            // gentle continuation for HDR headroom
            return 1.0 + (l - 3.6) * 0.09;
        }
        self.spline.eval(l)
    }
}

// ---------------------------------------------------------------------------
// 3D LUTs

#[derive(Clone, Debug)]
pub struct Lut3d {
    pub size: usize,
    /// r fastest, then g, then b (cube convention).
    pub data: Vec<Rgb>,
    pub domain_min: Rgb,
    pub domain_max: Rgb,
}

impl Lut3d {
    pub fn parse_cube(text: &str) -> Result<Self> {
        let mut size = 0;
        let mut data = vec![];
        let mut dmin = [0.0; 3];
        let mut dmax = [1.0; 3];
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split_whitespace();
            let first = it.next().unwrap();
            match first {
                "LUT_3D_SIZE" => size = it.next().ok_or_else(|| anyhow!("bad size"))?.parse()?,
                "DOMAIN_MIN" => {
                    for c in dmin.iter_mut() {
                        *c = it.next().ok_or_else(|| anyhow!("bad domain"))?.parse()?;
                    }
                }
                "DOMAIN_MAX" => {
                    for c in dmax.iter_mut() {
                        *c = it.next().ok_or_else(|| anyhow!("bad domain"))?.parse()?;
                    }
                }
                "TITLE" | "LUT_1D_SIZE" | "LUT_3D_INPUT_RANGE" | "LUT_1D_INPUT_RANGE" => {}
                _ => {
                    if let Ok(r) = first.parse::<f32>() {
                        let g: f32 = it.next().ok_or_else(|| anyhow!("bad row"))?.parse()?;
                        let b: f32 = it.next().ok_or_else(|| anyhow!("bad row"))?.parse()?;
                        data.push([r, g, b]);
                    }
                }
            }
        }
        if size < 2 || data.len() != size * size * size {
            return Err(anyhow!("unsupported .cube (size {size}, {} entries)", data.len()));
        }
        Ok(Self { size, data, domain_min: dmin, domain_max: dmax })
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::parse_cube(&std::fs::read_to_string(path)?)
    }

    pub fn from_fn(size: usize, f: impl Fn(Rgb) -> Rgb) -> Self {
        let mut data = Vec::with_capacity(size * size * size);
        let n = (size - 1) as f32;
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    data.push(f([r as f32 / n, g as f32 / n, b as f32 / n]));
                }
            }
        }
        Self { size, data, domain_min: [0.0; 3], domain_max: [1.0; 3] }
    }

    /// Trilinear lookup.
    pub fn eval(&self, c: Rgb) -> Rgb {
        let n = (self.size - 1) as f32;
        let mut idx = [0usize; 3];
        let mut frac = [0.0f32; 3];
        for i in 0..3 {
            let t = ((c[i] - self.domain_min[i]) / (self.domain_max[i] - self.domain_min[i])).clamp(0.0, 1.0) * n;
            let f = t.floor().min(n - 1.0);
            idx[i] = f as usize;
            frac[i] = t - f;
        }
        let s = self.size;
        let at = |r: usize, g: usize, b: usize| self.data[(b * s + g) * s + r];
        let mut out = [0.0f32; 3];
        for (db, wb) in [(0, 1.0 - frac[2]), (1, frac[2])] {
            for (dg, wg) in [(0, 1.0 - frac[1]), (1, frac[1])] {
                for (dr, wr) in [(0, 1.0 - frac[0]), (1, frac[0])] {
                    let w = wr * wg * wb;
                    if w == 0.0 {
                        continue;
                    }
                    let p = at(idx[0] + dr, idx[1] + dg, idx[2] + db);
                    for k in 0..3 {
                        out[k] += p[k] * w;
                    }
                }
            }
        }
        out
    }
}

/// Built-in creative looks, generated procedurally (display-encoded RGB in/out).
pub const BUILTIN_LOOKS: &[&str] = &[
    "Artistic 01",
    "Artistic 02",
    "Modern 01",
    "Modern 02",
    "Vintage 01",
    "Vintage 02",
    "B&W 01",
    "B&W 02",
    "B&W 03",
    "Matte",
    "Teal & Orange",
    "Warm Film",
    "Cool Film",
];

pub fn builtin_look(name: &str) -> Option<Lut3d> {
    let luma = |c: Rgb| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let mix = |a: Rgb, b: Rgb, t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
    let sat = move |c: Rgb, s: f32| {
        let l = luma(c);
        [l + (c[0] - l) * s, l + (c[1] - l) * s, l + (c[2] - l) * s]
    };
    let s_curve = |x: f32, k: f32| {
        let x = x.clamp(0.0, 1.0);
        let y = if x < 0.5 { 0.5 * (2.0 * x).powf(k) } else { 1.0 - 0.5 * (2.0 * (1.0 - x)).powf(k) };
        y
    };
    let f: Box<dyn Fn(Rgb) -> Rgb> = match name {
        "Artistic 01" => Box::new(move |c| {
            let c = [c[1] * 0.2 + c[0] * 0.8, c[1], c[2] * 0.9 + c[0] * 0.1];
            sat(c.map(|v| s_curve(v, 1.25)), 1.15)
        }),
        "Artistic 02" => Box::new(move |c| {
            let l = luma(c);
            let tint = mix([0.15, 0.35, 0.55], [1.0, 0.85, 0.55], l);
            mix(c, mix(c, tint, 0.5), 0.6).map(|v| s_curve(v, 1.15))
        }),
        "Modern 01" => Box::new(move |c| sat(c.map(|v| s_curve(v * 0.96 + 0.02, 1.2)), 0.9)),
        "Modern 02" => Box::new(move |c| {
            let l = luma(c);
            let c = mix(c, [c[0] * 1.03, c[1], c[2] * 0.95], l);
            sat(c.map(|v| s_curve(v, 1.3)), 1.05)
        }),
        "Vintage 01" => Box::new(move |c| {
            let c = sat(c, 0.75);
            let c = [c[0] * 0.92 + 0.07, c[1] * 0.9 + 0.06, c[2] * 0.82 + 0.08];
            c.map(|v| s_curve(v, 1.1))
        }),
        "Vintage 02" => Box::new(move |c| {
            let l = luma(c);
            let c = sat(c, 0.6);
            let toned = mix([0.12, 0.1, 0.16], [1.0, 0.93, 0.78], l);
            mix(c, toned, 0.25).map(|v| v * 0.9 + 0.05)
        }),
        "B&W 01" => Box::new(move |c| {
            let l = s_curve(luma(c), 1.2);
            [l, l, l]
        }),
        "B&W 02" => Box::new(move |c| {
            let l = s_curve(0.5 * c[0] + 0.4 * c[1] + 0.1 * c[2], 1.45);
            [l, l, l]
        }),
        "B&W 03" => Box::new(move |c| {
            let l = luma(c) * 0.88 + 0.08;
            [l * 1.02, l, l * 0.94]
        }),
        "Matte" => Box::new(move |c| sat(c, 0.92).map(|v| 0.07 + v * 0.88)),
        "Teal & Orange" => Box::new(move |c| {
            let l = luma(c);
            let toned = mix([0.0, 0.42, 0.48], [1.0, 0.62, 0.32], l);
            let d = mix(c, toned, 0.22);
            sat(d, 1.05)
        }),
        "Warm Film" => Box::new(move |c| [c[0] * 1.04 + 0.01, c[1] * 1.0 + 0.005, c[2] * 0.9].map(|v| s_curve(v, 1.1))),
        "Cool Film" => Box::new(move |c| [c[0] * 0.93, c[1] * 0.99 + 0.01, c[2] * 1.05 + 0.02].map(|v| s_curve(v, 1.1))),
        _ => return None,
    };
    Some(Lut3d::from_fn(33, |c| {
        let o = f(c);
        [o[0].clamp(0.0, 1.0), o[1].clamp(0.0, 1.0), o[2].clamp(0.0, 1.0)]
    }))
}

/// Resolve a look source: "builtin:<name>" or a filesystem path to a .cube.
pub fn load_look(source: &str) -> Option<Lut3d> {
    if let Some(name) = source.strip_prefix("builtin:") {
        return builtin_look(name);
    }
    Lut3d::load(Path::new(source)).ok()
}

/// User profile/look directory (XDG).
pub fn user_looks_dir() -> std::path::PathBuf {
    crate::paths::config_dir().join("profiles")
}

pub fn list_user_looks() -> Vec<(String, String)> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(user_looks_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("cube")).unwrap_or(false) {
                let name = p.file_stem().unwrap().to_string_lossy().to_string();
                out.push((name, p.to_string_lossy().to_string()));
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_curve_monotone_and_mid() {
        let c = BaseCurve::new(&PROFILES[0]);
        assert!((c.eval(0.0) - 0.47).abs() < 1e-4);
        let mut last = -1.0;
        for i in 0..200 {
            let v = c.eval(-12.0 + i as f32 * 0.1);
            assert!(v >= last);
            last = v;
        }
    }

    #[test]
    fn identity_cube_roundtrip() {
        let l = Lut3d::from_fn(17, |c| c);
        let p = l.eval([0.3, 0.6, 0.9]);
        assert!((p[0] - 0.3).abs() < 1e-5 && (p[2] - 0.9).abs() < 1e-5);
        let text = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let l = Lut3d::parse_cube(text).unwrap();
        let p = l.eval([0.25, 0.5, 0.75]);
        assert!((p[1] - 0.5).abs() < 1e-5);
    }
}
