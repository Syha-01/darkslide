//! Colour science: matrices, colour spaces, white balance (DNG SDK style
//! temperature/tint), Oklab, transfer curves.

use crate::image::Rgb;

pub type Mat3 = [[f32; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

#[inline]
pub fn mul_mv(m: &Mat3, v: Rgb) -> Rgb {
    [m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2], m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2], m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2]]
}

pub fn mul_mm(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut o = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] as f64 * b[k][j] as f64).sum::<f64>() as f32;
        }
    }
    o
}

pub fn invert(m: &Mat3) -> Mat3 {
    let m: [[f64; 3]; 3] = m.map(|r| r.map(|v| v as f64));
    let det = m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return IDENTITY;
    }
    let inv = 1.0 / det;
    let o = [
        [(m[1][1] * m[2][2] - m[2][1] * m[1][2]) * inv, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv],
        [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv, (m[1][0] * m[0][2] - m[0][0] * m[1][2]) * inv],
        [(m[1][0] * m[2][1] - m[2][0] * m[1][1]) * inv, (m[2][0] * m[0][1] - m[0][0] * m[2][1]) * inv, (m[0][0] * m[1][1] - m[1][0] * m[0][1]) * inv],
    ];
    o.map(|r| r.map(|v| v as f32))
}

pub fn diag(v: Rgb) -> Mat3 {
    [[v[0], 0.0, 0.0], [0.0, v[1], 0.0], [0.0, 0.0, v[2]]]
}

pub fn scale_m(m: &Mat3, s: f32) -> Mat3 {
    m.map(|r| r.map(|v| v * s))
}

pub fn lerp_m(a: &Mat3, b: &Mat3, t: f32) -> Mat3 {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = a[i][j] * (1.0 - t) + b[i][j] * t;
        }
    }
    o
}

// ---------------------------------------------------------------------------
// Chromaticities and colour spaces

pub const D50_XY: [f32; 2] = [0.34567, 0.35850];
pub const D65_XY: [f32; 2] = [0.31271, 0.32902];

pub fn xy_to_xyz(xy: [f32; 2]) -> Rgb {
    let [x, y] = xy;
    let y = y.max(1e-6);
    [x / y, 1.0, (1.0 - x - y) / y]
}

pub fn xyz_to_xy(v: Rgb) -> [f32; 2] {
    let s = v[0] + v[1] + v[2];
    if s <= 0.0 {
        return D50_XY;
    }
    [v[0] / s, v[1] / s]
}

const BRADFORD: Mat3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];

/// Chromatic adaptation XYZ(src white) -> XYZ(dst white).
pub fn bradford(src: [f32; 2], dst: [f32; 2]) -> Mat3 {
    let s = mul_mv(&BRADFORD, xy_to_xyz(src));
    let d = mul_mv(&BRADFORD, xy_to_xyz(dst));
    let scale = diag([d[0] / s[0], d[1] / s[1], d[2] / s[2]]);
    mul_mm(&invert(&BRADFORD), &mul_mm(&scale, &BRADFORD))
}

/// RGB->XYZ for given primaries and white, adapted to D50 (the PCS).
pub fn rgb_to_xyz_d50(r: [f32; 2], g: [f32; 2], b: [f32; 2], white: [f32; 2]) -> Mat3 {
    let pr = xy_to_xyz(r);
    let pg = xy_to_xyz(g);
    let pb = xy_to_xyz(b);
    let m = [[pr[0], pg[0], pb[0]], [pr[1], pg[1], pb[1]], [pr[2], pg[2], pb[2]]];
    let s = mul_mv(&invert(&m), xy_to_xyz(white));
    let m = mul_mm(&m, &diag(s));
    if (white[0] - D50_XY[0]).abs() < 1e-4 && (white[1] - D50_XY[1]).abs() < 1e-4 { m } else { mul_mm(&bradford(white, D50_XY), &m) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, Default)]
pub enum ColorSpace {
    #[default]
    SRgb,
    AdobeRgb,
    DisplayP3,
    ProPhoto,
    Rec2020,
}

impl ColorSpace {
    pub const ALL: [ColorSpace; 5] = [Self::SRgb, Self::AdobeRgb, Self::DisplayP3, Self::ProPhoto, Self::Rec2020];

    pub fn name(self) -> &'static str {
        match self {
            Self::SRgb => "sRGB IEC61966-2.1",
            Self::AdobeRgb => "Adobe RGB (1998)",
            Self::DisplayP3 => "Display P3",
            Self::ProPhoto => "ProPhoto RGB",
            Self::Rec2020 => "Rec. 2020",
        }
    }

    /// Matrix RGB(linear) -> XYZ D50.
    pub fn to_xyz(self) -> Mat3 {
        match self {
            Self::SRgb => rgb_to_xyz_d50([0.64, 0.33], [0.30, 0.60], [0.15, 0.06], D65_XY),
            Self::AdobeRgb => rgb_to_xyz_d50([0.64, 0.33], [0.21, 0.71], [0.15, 0.06], D65_XY),
            Self::DisplayP3 => rgb_to_xyz_d50([0.680, 0.320], [0.265, 0.690], [0.150, 0.060], D65_XY),
            Self::ProPhoto => rgb_to_xyz_d50([0.734699, 0.265301], [0.159597, 0.840403], [0.036598, 0.000105], D50_XY),
            Self::Rec2020 => rgb_to_xyz_d50([0.708, 0.292], [0.170, 0.797], [0.131, 0.046], D65_XY),
        }
    }

    pub fn encode(self, v: f32) -> f32 {
        match self {
            Self::SRgb | Self::DisplayP3 => srgb_encode(v),
            Self::AdobeRgb => v.max(0.0).powf(1.0 / 2.19921875),
            Self::ProPhoto => {
                let v = v.max(0.0);
                if v < 1.0 / 512.0 { v * 16.0 } else { v.powf(1.0 / 1.8) }
            }
            Self::Rec2020 => {
                let v = v.max(0.0);
                if v < 0.018053968 { 4.5 * v } else { 1.0993 * v.powf(0.45) - 0.0993 }
            }
        }
    }

    pub fn decode(self, v: f32) -> f32 {
        match self {
            Self::SRgb | Self::DisplayP3 => srgb_decode(v),
            Self::AdobeRgb => v.max(0.0).powf(2.19921875),
            Self::ProPhoto => {
                if v < 16.0 / 512.0 {
                    v / 16.0
                } else {
                    v.powf(1.8)
                }
            }
            Self::Rec2020 => {
                if v < 0.081243 {
                    v / 4.5
                } else {
                    ((v + 0.0993) / 1.0993).powf(1.0 / 0.45)
                }
            }
        }
    }
}

/// ProPhoto (linear, D50) -> output space (linear).
pub fn prophoto_to(space: ColorSpace) -> Mat3 {
    mul_mm(&invert(&space.to_xyz()), &ColorSpace::ProPhoto.to_xyz())
}

pub fn to_prophoto(space: ColorSpace) -> Mat3 {
    mul_mm(&invert(&ColorSpace::ProPhoto.to_xyz()), &space.to_xyz())
}

pub fn xyz_d50_to_prophoto() -> Mat3 {
    invert(&ColorSpace::ProPhoto.to_xyz())
}

#[inline]
pub fn srgb_encode(v: f32) -> f32 {
    if v <= 0.0031308 { 12.92 * v.max(0.0) } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

#[inline]
pub fn srgb_decode(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Luminance (Y) of a linear ProPhoto value.
#[inline]
pub fn luma_prophoto(p: Rgb) -> f32 {
    0.2880402 * p[0] + 0.7118741 * p[1] + 0.0000857 * p[2]
}

// ---------------------------------------------------------------------------
// Temperature / tint, after the DNG SDK (dng_temperature.cpp).

const TINT_SCALE: f64 = -3000.0;

// (mired, u, v, isotherm slope) — Robertson's table
const TEMP_TABLE: [[f64; 4]; 31] = [
    [0.0, 0.18006, 0.26352, -0.24341],
    [10.0, 0.18066, 0.26589, -0.25479],
    [20.0, 0.18133, 0.26846, -0.26876],
    [30.0, 0.18208, 0.27119, -0.28539],
    [40.0, 0.18293, 0.27407, -0.30470],
    [50.0, 0.18388, 0.27709, -0.32675],
    [60.0, 0.18494, 0.28021, -0.35156],
    [70.0, 0.18611, 0.28342, -0.37915],
    [80.0, 0.18740, 0.28668, -0.40955],
    [90.0, 0.18880, 0.28997, -0.44278],
    [100.0, 0.19032, 0.29326, -0.47888],
    [125.0, 0.19462, 0.30141, -0.58204],
    [150.0, 0.19962, 0.30921, -0.70471],
    [175.0, 0.20525, 0.31647, -0.84901],
    [200.0, 0.21142, 0.32312, -1.0182],
    [225.0, 0.21807, 0.32909, -1.2168],
    [250.0, 0.22511, 0.33439, -1.4512],
    [275.0, 0.23247, 0.33904, -1.7298],
    [300.0, 0.24010, 0.34308, -2.0637],
    [325.0, 0.24702, 0.34655, -2.4681],
    [350.0, 0.25591, 0.34951, -2.9641],
    [375.0, 0.26400, 0.35200, -3.5814],
    [400.0, 0.27218, 0.35407, -4.3633],
    [425.0, 0.28039, 0.35577, -5.3762],
    [450.0, 0.28863, 0.35714, -6.7262],
    [475.0, 0.29685, 0.35823, -8.5955],
    [500.0, 0.30505, 0.35907, -11.324],
    [525.0, 0.31320, 0.35968, -15.628],
    [550.0, 0.32129, 0.36011, -23.325],
    [575.0, 0.32931, 0.36038, -40.770],
    [600.0, 0.33724, 0.36051, -116.45],
];

pub fn xy_to_temp_tint(xy: [f32; 2]) -> (f32, f32) {
    let (x, y) = (xy[0] as f64, xy[1] as f64);
    let u = 2.0 * x / (1.5 - x + 6.0 * y);
    let v = 3.0 * y / (1.5 - x + 6.0 * y);
    let (mut last_dt, mut last_du, mut last_dv) = (0.0, 0.0, 0.0);
    for index in 1..=30 {
        let mut du = 1.0;
        let mut dv = TEMP_TABLE[index][3];
        let len = (1.0 + dv * dv).sqrt();
        du /= len;
        dv /= len;
        let uu = u - TEMP_TABLE[index][1];
        let vv = v - TEMP_TABLE[index][2];
        let mut dt = -uu * dv + vv * du;
        if dt <= 0.0 || index == 30 {
            if dt > 0.0 {
                dt = 0.0;
            }
            dt = -dt;
            let f = if index == 1 { 0.0 } else { dt / (last_dt + dt) };
            let temp = 1.0e6 / (TEMP_TABLE[index - 1][0] * f + TEMP_TABLE[index][0] * (1.0 - f));
            let uu = u - (TEMP_TABLE[index - 1][1] * f + TEMP_TABLE[index][1] * (1.0 - f));
            let vv = v - (TEMP_TABLE[index - 1][2] * f + TEMP_TABLE[index][2] * (1.0 - f));
            let mut du = du * (1.0 - f) + last_du * f;
            let mut dv = dv * (1.0 - f) + last_dv * f;
            let len = (du * du + dv * dv).sqrt();
            du /= len;
            dv /= len;
            let tint = (uu * du + vv * dv) * TINT_SCALE;
            return (temp.min(1.0e6) as f32, tint as f32);
        }
        last_dt = dt;
        last_du = du;
        last_dv = dv;
    }
    (5000.0, 0.0)
}

pub fn temp_tint_to_xy(temp: f32, tint: f32) -> [f32; 2] {
    let r = 1.0e6 / temp.max(1000.0) as f64;
    let offset = tint as f64 * (1.0 / TINT_SCALE);
    for index in 0..=29 {
        if r < TEMP_TABLE[index + 1][0] || index == 29 {
            let f = (TEMP_TABLE[index + 1][0] - r) / (TEMP_TABLE[index + 1][0] - TEMP_TABLE[index][0]);
            let mut u = TEMP_TABLE[index][1] * f + TEMP_TABLE[index + 1][1] * (1.0 - f);
            let mut v = TEMP_TABLE[index][2] * f + TEMP_TABLE[index + 1][2] * (1.0 - f);
            let (mut uu1, mut vv1) = (1.0, TEMP_TABLE[index][3]);
            let (mut uu2, mut vv2) = (1.0, TEMP_TABLE[index + 1][3]);
            let l1 = (1.0f64 + vv1 * vv1).sqrt();
            let l2 = (1.0f64 + vv2 * vv2).sqrt();
            uu1 /= l1;
            vv1 /= l1;
            uu2 /= l2;
            vv2 /= l2;
            let mut uu3 = uu1 * f + uu2 * (1.0 - f);
            let mut vv3 = vv1 * f + vv2 * (1.0 - f);
            let l3 = (uu3 * uu3 + vv3 * vv3).sqrt();
            uu3 /= l3;
            vv3 /= l3;
            u += uu3 * offset;
            v += vv3 * offset;
            let d = u - 4.0 * v + 2.0;
            return [(1.5 * u / d) as f32, (v / d) as f32];
        }
    }
    D50_XY
}

// ---------------------------------------------------------------------------
// Camera colour model (DNG style: ColorMatrix per calibration illuminant)

#[derive(Clone, Debug)]
pub struct CameraColor {
    /// (illuminant temperature K, XYZ->camera matrix)
    pub matrices: Vec<(f32, Mat3)>,
    /// True when data is already in a known RGB space (raster input).
    pub is_rgb_space: bool,
}

impl CameraColor {
    pub fn srgb() -> Self {
        // XYZ(D50) -> linear sRGB, so the "camera" is sRGB itself
        Self { matrices: vec![(6504.0, invert(&ColorSpace::SRgb.to_xyz()))], is_rgb_space: true }
    }

    /// Interpolated XYZ->camera matrix for a white point (DNG spec: linear in 1/T).
    pub fn xyz_to_camera(&self, xy: [f32; 2]) -> Mat3 {
        if self.matrices.len() == 1 {
            return self.matrices[0].1;
        }
        let (t, _) = xy_to_temp_tint(xy);
        let mut ms = self.matrices.clone();
        ms.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let (t1, m1) = ms[0];
        let (t2, m2) = ms[ms.len() - 1];
        if t <= t1 {
            return m1;
        }
        if t >= t2 {
            return m2;
        }
        let g = ((1.0 / t) - (1.0 / t2)) / ((1.0 / t1) - (1.0 / t2));
        lerp_m(&m2, &m1, g)
    }

    /// Camera neutral (camera RGB of a white with chromaticity xy), normalised to G=1.
    pub fn neutral_for_xy(&self, xy: [f32; 2]) -> Rgb {
        let n = mul_mv(&self.xyz_to_camera(xy), xy_to_xyz(xy));
        let g = n[1].max(1e-6);
        [n[0] / g, 1.0, n[2] / g]
    }

    /// Inverse of `neutral_for_xy`, iterative as in dng_color_spec::NeutralToXY.
    pub fn xy_for_neutral(&self, neutral: Rgb) -> [f32; 2] {
        let mut last = D50_XY;
        for pass in 0..30 {
            let m = invert(&self.xyz_to_camera(last));
            let next = xyz_to_xy(mul_mv(&m, neutral));
            if (next[0] - last[0]).abs() + (next[1] - last[1]).abs() < 1e-7 {
                return next;
            }
            last = if pass == 29 { [(last[0] + next[0]) * 0.5, (last[1] + next[1]) * 0.5] } else { next };
        }
        last
    }

    /// Matrix taking white-balanced camera RGB (white = 1,1,1) to linear ProPhoto.
    pub fn wb_camera_to_prophoto(&self, xy: [f32; 2]) -> Mat3 {
        let neutral = self.neutral_for_xy(xy);
        let cam_to_xyz = invert(&self.xyz_to_camera(xy));
        // white-balanced camera -> XYZ(white)
        let m = mul_mm(&cam_to_xyz, &diag(neutral));
        let w = mul_mv(&m, [1.0, 1.0, 1.0]);
        let m = scale_m(&m, 1.0 / w[1].max(1e-6));
        let m = mul_mm(&bradford(xy, D50_XY), &m);
        mul_mm(&xyz_d50_to_prophoto(), &m)
    }
}

// ---------------------------------------------------------------------------
// Oklab from linear ProPhoto

fn prophoto_to_lms() -> Mat3 {
    const M1: Mat3 = [[0.818_933, 0.361_866_74, -0.128_859_71], [0.032_984_544, 0.929_311_9, 0.036_145_64], [0.048_200_3, 0.264_366_27, 0.633_851_7]];
    mul_mm(&M1, &mul_mm(&bradford(D50_XY, D65_XY), &ColorSpace::ProPhoto.to_xyz()))
}

const M2: Mat3 = [[0.210_454_26, 0.793_617_8, -0.004_072_047], [1.977_998_5, -2.428_592_2, 0.450_593_7], [0.025_904_037, 0.782_771_77, -0.808_675_77]];

pub struct Oklab {
    to_lms: Mat3,
    from_lms: Mat3,
    m2_inv: Mat3,
}

impl Default for Oklab {
    fn default() -> Self {
        Self::new()
    }
}

impl Oklab {
    pub fn new() -> Self {
        let to_lms = prophoto_to_lms();
        Self { to_lms, from_lms: invert(&to_lms), m2_inv: invert(&M2) }
    }

    #[inline]
    pub fn from_prophoto(&self, p: Rgb) -> Rgb {
        let lms = mul_mv(&self.to_lms, p);
        let lms = [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()];
        mul_mv(&M2, lms)
    }

    #[inline]
    pub fn to_prophoto(&self, lab: Rgb) -> Rgb {
        let l = mul_mv(&self.m2_inv, lab);
        mul_mv(&self.from_lms, [l[0] * l[0] * l[0], l[1] * l[1] * l[1], l[2] * l[2] * l[2]])
    }
}

/// Lab -> (L, C, h in degrees 0..360)
#[inline]
pub fn lab_to_lch(lab: Rgb) -> Rgb {
    let c = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    let mut h = lab[2].atan2(lab[1]).to_degrees();
    if h < 0.0 {
        h += 360.0;
    }
    [lab[0], c, h]
}

#[inline]
pub fn lch_to_lab(lch: Rgb) -> Rgb {
    let h = lch[2].to_radians();
    [lch[0], lch[1] * h.cos(), lch[1] * h.sin()]
}

/// Signed hue difference a-b in degrees, in (-180, 180].
#[inline]
pub fn hue_diff(a: f32, b: f32) -> f32 {
    let mut d = (a - b) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

pub fn rgb_to_hsv(c: Rgb) -> Rgb {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let d = max - min;
    let v = max;
    let s = if max > 0.0 { d / max } else { 0.0 };
    let h = if d <= 0.0 {
        0.0
    } else if max == c[0] {
        60.0 * (((c[1] - c[2]) / d).rem_euclid(6.0))
    } else if max == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    [h, s, v]
}

pub fn hsv_to_rgb(hsv: Rgb) -> Rgb {
    let [h, s, v] = hsv;
    let h = h.rem_euclid(360.0) / 60.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_tint_roundtrip() {
        for &(t, tint) in &[(2850.0, 0.0), (5000.0, 10.0), (6500.0, -20.0), (9000.0, 30.0), (2000.0, 5.0)] {
            let xy = temp_tint_to_xy(t, tint);
            let (t2, tint2) = xy_to_temp_tint(xy);
            assert!((t - t2).abs() / t < 0.002, "{t} -> {t2}");
            assert!((tint - tint2).abs() < 0.3, "{tint} -> {tint2}");
        }
        let (t, _) = xy_to_temp_tint(D65_XY);
        assert!((t - 6504.0).abs() < 30.0, "{t}");
    }

    #[test]
    fn oklab_roundtrip_and_white() {
        let ok = Oklab::new();
        let w = ok.from_prophoto([1.0, 1.0, 1.0]);
        assert!((w[0] - 1.0).abs() < 1e-3 && w[1].abs() < 1e-3 && w[2].abs() < 1e-3, "{w:?}");
        let c = [0.3, 0.1, 0.6];
        let back = ok.to_prophoto(ok.from_prophoto(c));
        for i in 0..3 {
            assert!((back[i] - c[i]).abs() < 1e-4);
        }
    }

    #[test]
    fn srgb_white_maps_to_prophoto_white() {
        let m = to_prophoto(ColorSpace::SRgb);
        let w = mul_mv(&m, [1.0, 1.0, 1.0]);
        for c in w {
            assert!((c - 1.0).abs() < 2e-3, "{w:?}");
        }
    }
}
