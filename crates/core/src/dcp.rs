//! DNG Camera Profiles (.dcp): colour/forward matrices, HueSatMap,
//! LookTable and tone curve, applied the way the DNG SDK does.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::color::{Mat3, rgb_to_hsv};
use crate::image::Rgb;

#[derive(Clone, Debug)]
pub struct HsvTable {
    pub hue_divs: usize,
    pub sat_divs: usize,
    pub val_divs: usize,
    /// (hue shift degrees, sat scale, val scale), index (v*hue + h)*sat + s
    pub data: Vec<[f32; 3]>,
    /// value axis encoded with sRGB gamma
    pub srgb_encoding: bool,
}

impl HsvTable {
    /// Apply to a linear ProPhoto colour.
    pub fn apply(&self, p: Rgb) -> Rgb {
        let [mut h, mut s, mut v] = rgb_to_hsv(p.map(|c| c.max(0.0)));
        if v <= 0.0 {
            return p;
        }
        // DNG hue range is [0,6)
        h /= 60.0;
        let hs = self.hue_divs as f32 / 6.0;
        let ss = (self.sat_divs - 1) as f32;
        let h_scaled = h * hs;
        let s_scaled = s.min(1.0) * ss;
        let h0 = (h_scaled.floor() as usize).min(self.hue_divs - 1);
        let h1 = if h0 + 1 >= self.hue_divs { 0 } else { h0 + 1 };
        let s0 = (s_scaled.floor() as usize).min(self.sat_divs.saturating_sub(2));
        let s1 = (s0 + 1).min(self.sat_divs - 1);
        let hf = h_scaled - h0 as f32;
        let sf = (s_scaled - s0 as f32).clamp(0.0, 1.0);
        let lookup = |vi: usize| -> [f32; 3] {
            let at = |hi: usize, si: usize| self.data[(vi * self.hue_divs + hi) * self.sat_divs + si];
            let (a, b, c, d) = (at(h0, s0), at(h1, s0), at(h0, s1), at(h1, s1));
            let mut o = [0.0; 3];
            for k in 0..3 {
                let top = a[k] * (1.0 - hf) + b[k] * hf;
                let bot = c[k] * (1.0 - hf) + d[k] * hf;
                o[k] = top * (1.0 - sf) + bot * sf;
            }
            o
        };
        let e = if self.val_divs < 2 {
            lookup(0)
        } else {
            let vv = if self.srgb_encoding { crate::color::srgb_encode(v.min(1.0)) } else { v.min(1.0) };
            let vs = vv * (self.val_divs - 1) as f32;
            let v0 = (vs.floor() as usize).min(self.val_divs - 2);
            let vf = vs - v0 as f32;
            let (a, b) = (lookup(v0), lookup(v0 + 1));
            [0, 1, 2].map(|k| a[k] * (1.0 - vf) + b[k] * vf)
        };
        h = (h + e[0] / 60.0).rem_euclid(6.0);
        s = (s * e[1]).min(1.0);
        v *= e[2];
        crate::color::hsv_to_rgb([h * 60.0, s, v])
    }
}

#[derive(Clone, Debug)]
pub struct Dcp {
    pub name: String,
    pub camera_model: String,
    pub illuminants: [u16; 2],
    pub color_matrix: [Option<Mat3>; 2],
    pub forward_matrix: [Option<Mat3>; 2],
    pub hue_sat: [Option<HsvTable>; 2],
    pub look: Option<HsvTable>,
    /// (x, y) pairs in [0,1]
    pub tone_curve: Option<Vec<[f32; 2]>>,
    pub baseline_exposure_offset: f32,
    pub path: PathBuf,
}

struct Reader<'a> {
    b: &'a [u8],
    le: bool,
}

impl Reader<'_> {
    fn u16(&self, o: usize) -> Result<u16> {
        let s = self.b.get(o..o + 2).ok_or_else(|| anyhow!("eof"))?;
        Ok(if self.le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    }
    fn u32(&self, o: usize) -> Result<u32> {
        let s = self.b.get(o..o + 4).ok_or_else(|| anyhow!("eof"))?;
        let a = [s[0], s[1], s[2], s[3]];
        Ok(if self.le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    }
    fn i32(&self, o: usize) -> Result<i32> {
        Ok(self.u32(o)? as i32)
    }
    fn f32(&self, o: usize) -> Result<f32> {
        Ok(f32::from_bits(self.u32(o)?))
    }
}

fn type_size(t: u16) -> usize {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => 1,
    }
}

impl Dcp {
    pub fn load(path: &Path) -> Result<Self> {
        let b = std::fs::read(path)?;
        Self::parse(&b, path)
    }

    pub fn parse(b: &[u8], path: &Path) -> Result<Self> {
        if b.len() < 8 {
            bail!("too short");
        }
        let le = match &b[0..2] {
            b"II" => true,
            b"MM" => false,
            _ => bail!("not a DCP"),
        };
        let r = Reader { b, le };
        let magic = r.u16(2)?;
        if magic != 0x4352 && magic != 42 {
            bail!("bad DCP magic {magic:x}");
        }
        let ifd = r.u32(4)? as usize;
        let n = r.u16(ifd)? as usize;
        let mut dcp = Dcp {
            name: path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            camera_model: String::new(),
            illuminants: [17, 21],
            color_matrix: [None, None],
            forward_matrix: [None, None],
            hue_sat: [None, None],
            look: None,
            tone_curve: None,
            baseline_exposure_offset: 0.0,
            path: path.to_path_buf(),
        };
        let mut hsm_dims = [0usize; 3];
        let mut look_dims = [0usize; 3];
        let mut hsm_raw: [Option<Vec<f32>>; 2] = [None, None];
        let mut look_raw: Option<Vec<f32>> = None;
        let (mut hsm_enc, mut look_enc) = (0u32, 0u32);
        for k in 0..n {
            let e = ifd + 2 + k * 12;
            let tag = r.u16(e)?;
            let typ = r.u16(e + 2)?;
            let count = r.u32(e + 4)? as usize;
            let size = type_size(typ) * count;
            let off = if size <= 4 { e + 8 } else { r.u32(e + 8)? as usize };
            let floats = |n: usize| -> Result<Vec<f32>> {
                (0..n.min(count))
                    .map(|i| match typ {
                        11 => r.f32(off + i * 4),
                        10 => Ok(r.i32(off + i * 8)? as f32 / r.i32(off + i * 8 + 4)?.max(1) as f32),
                        5 => Ok(r.u32(off + i * 8)? as f32 / r.u32(off + i * 8 + 4)?.max(1) as f32),
                        4 => Ok(r.u32(off + i * 4)? as f32),
                        3 => Ok(r.u16(off + i * 2)? as f32),
                        12 => {
                            let s = b.get(off + i * 8..off + i * 8 + 8).ok_or_else(|| anyhow!("eof"))?;
                            let a: [u8; 8] = s.try_into().unwrap();
                            Ok(if le { f64::from_le_bytes(a) } else { f64::from_be_bytes(a) } as f32)
                        }
                        _ => Ok(0.0),
                    })
                    .collect()
            };
            let ascii = || -> String { b.get(off..off + count).map(|s| String::from_utf8_lossy(s).trim_end_matches('\0').to_string()).unwrap_or_default() };
            let mat = |v: Vec<f32>| -> Option<Mat3> { if v.len() >= 9 { Some([[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]]) } else { None } };
            match tag {
                0xC614 => dcp.camera_model = ascii(),
                0xC6F8 => dcp.name = ascii(),
                0xC65A => dcp.illuminants[0] = r.u16(off)?,
                0xC65B => dcp.illuminants[1] = r.u16(off)?,
                0xC621 => dcp.color_matrix[0] = mat(floats(9)?),
                0xC622 => dcp.color_matrix[1] = mat(floats(9)?),
                0xC714 => dcp.forward_matrix[0] = mat(floats(9)?),
                0xC715 => dcp.forward_matrix[1] = mat(floats(9)?),
                0xC6F9 => {
                    let v = floats(3)?;
                    hsm_dims = [v[0] as usize, v[1] as usize, v.get(2).map(|x| *x as usize).unwrap_or(1)];
                }
                0xC6FA => hsm_raw[0] = Some(floats(count)?),
                0xC6FB => hsm_raw[1] = Some(floats(count)?),
                0xC725 => {
                    let v = floats(3)?;
                    look_dims = [v[0] as usize, v[1] as usize, v.get(2).map(|x| *x as usize).unwrap_or(1)];
                }
                0xC726 => look_raw = Some(floats(count)?),
                0xC6FC => {
                    let v = floats(count)?;
                    dcp.tone_curve = Some(v.chunks_exact(2).map(|c| [c[0], c[1]]).collect());
                }
                0xC7A3 => hsm_enc = r.u32(off)?,
                0xC7A4 => look_enc = r.u32(off)?,
                0xC7A5 => dcp.baseline_exposure_offset = floats(1)?.first().copied().unwrap_or(0.0),
                _ => {}
            }
        }
        let table = |dims: [usize; 3], raw: Vec<f32>, enc: u32| -> Option<HsvTable> {
            let [h, s, v] = dims;
            let v = v.max(1);
            if h == 0 || s < 2 || raw.len() < h * s * v * 3 {
                return None;
            }
            Some(HsvTable { hue_divs: h, sat_divs: s, val_divs: v, data: raw.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(), srgb_encoding: enc == 1 })
        };
        for i in 0..2 {
            if let Some(raw) = hsm_raw[i].take() {
                dcp.hue_sat[i] = table(hsm_dims, raw, hsm_enc);
            }
        }
        if let Some(raw) = look_raw {
            dcp.look = table(look_dims, raw, look_enc);
        }
        if dcp.color_matrix[0].is_none() {
            bail!("DCP has no ColorMatrix1");
        }
        Ok(dcp)
    }

    fn illum_temp(i: u16) -> f32 {
        match i {
            17 | 3 | 24 => 2856.0,
            23 => 5003.0,
            20 | 1 | 9 | 4 => 5503.0,
            21 | 10 => 6504.0,
            22 | 11 => 7504.0,
            2 | 14 => 4150.0,
            12 => 6430.0,
            13 => 5000.0,
            15 => 3450.0,
            _ => 5003.0,
        }
    }

    pub fn camera_color(&self) -> crate::color::CameraColor {
        let mut m = vec![];
        for i in 0..2 {
            if let Some(cm) = self.color_matrix[i] {
                m.push((Self::illum_temp(self.illuminants[i]), cm));
            }
        }
        crate::color::CameraColor { matrices: m, is_rgb_space: false }
    }

    /// Weight of the first calibration illuminant for a white temperature.
    pub fn weight1(&self, temp: f32) -> f32 {
        let (t1, t2) = (Self::illum_temp(self.illuminants[0]), Self::illum_temp(self.illuminants[1]));
        if self.color_matrix[1].is_none() || (t1 - t2).abs() < 1.0 {
            return 1.0;
        }
        let (lo, hi, lo_is_1) = if t1 < t2 { (t1, t2, true) } else { (t2, t1, false) };
        let g = if temp <= lo {
            1.0
        } else if temp >= hi {
            0.0
        } else {
            ((1.0 / temp) - (1.0 / hi)) / ((1.0 / lo) - (1.0 / hi))
        };
        if lo_is_1 { g } else { 1.0 - g }
    }

    pub fn forward(&self, temp: f32) -> Option<Mat3> {
        let w = self.weight1(temp);
        match (self.forward_matrix[0], self.forward_matrix[1]) {
            (Some(a), Some(b)) => Some(crate::color::lerp_m(&b, &a, w)),
            (Some(a), None) => Some(a),
            _ => None,
        }
    }

    /// HueSatMap for a temperature (interpolated between the two tables).
    pub fn hue_sat_for(&self, temp: f32) -> Option<HsvTable> {
        match (&self.hue_sat[0], &self.hue_sat[1]) {
            (Some(a), Some(b)) if a.data.len() == b.data.len() => {
                let w = self.weight1(temp);
                let mut t = a.clone();
                for (o, (x, y)) in t.data.iter_mut().zip(a.data.iter().zip(b.data.iter())) {
                    *o = [0, 1, 2].map(|k| x[k] * w + y[k] * (1.0 - w));
                }
                Some(t)
            }
            (Some(a), _) => Some(a.clone()),
            (None, Some(b)) => Some(b.clone()),
            _ => None,
        }
    }

    /// Does the profile's camera model match this camera?
    pub fn matches(&self, make: &str, model: &str) -> bool {
        let cm = self.camera_model.to_lowercase();
        let model = model.to_lowercase();
        cm.is_empty() || cm.contains(&model) || cm.ends_with(model.trim_start_matches(&make.to_lowercase()).trim())
    }
}

/// DCP files from the user profile dir and Adobe's usual locations under ~/.
pub fn list_dcps() -> Vec<PathBuf> {
    let mut out = vec![];
    let mut dirs = vec![crate::profiles::user_looks_dir()];
    if let Some(h) = std::env::var_os("HOME") {
        let h = PathBuf::from(h);
        dirs.push(h.join(".local/share/darkslide/dcp"));
        dirs.push(h.join(".adobe/CameraProfiles"));
    }
    fn walk(d: &Path, out: &mut Vec<PathBuf>, depth: usize) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                walk(&p, out, depth + 1);
            } else if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("dcp")).unwrap_or(false) {
                out.push(p);
            }
        }
    }
    for d in dirs {
        walk(&d, &mut out, 0);
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_table_is_noop() {
        let t = HsvTable { hue_divs: 6, sat_divs: 2, val_divs: 1, data: vec![[0.0, 1.0, 1.0]; 12], srgb_encoding: false };
        let p = [0.3, 0.5, 0.2];
        let o = t.apply(p);
        for k in 0..3 {
            assert!((o[k] - p[k]).abs() < 1e-5);
        }
    }

    #[test]
    fn parses_minimal_dcp() {
        // little-endian "IIRC" with one ColorMatrix1 (SRATIONAL x9)
        let mut b = b"IIRC".to_vec();
        b.extend(8u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(0xC621u16.to_le_bytes());
        b.extend(10u16.to_le_bytes());
        b.extend(9u32.to_le_bytes());
        let data_off = 8 + 2 + 12 + 4;
        b.extend((data_off as u32).to_le_bytes());
        b.extend(0u32.to_le_bytes());
        for i in 0..9 {
            let v: i32 = if i % 4 == 0 { 10000 } else { 0 };
            b.extend(v.to_le_bytes());
            b.extend(10000i32.to_le_bytes());
        }
        let d = Dcp::parse(&b, Path::new("x.dcp")).unwrap();
        assert_eq!(d.color_matrix[0].unwrap()[1][1], 1.0);
    }
}
