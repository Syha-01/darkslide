//! Local adjustments: mask evaluation and per-pixel parameter fields.

use rayon::prelude::*;

use crate::color::Oklab;
use crate::image::{Image3, Plane};
use crate::params::{BrushStroke, LocalAdjust, MaskGroup, MaskOp, MaskShape};

use super::geometry::{Geometry, Region};
use super::tone::smoothstep;

/// Scalar local parameters that are summed into per-pixel fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum LP {
    Temperature,
    Tint,
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Texture,
    Clarity,
    Dehaze,
    Hue,
    Saturation,
    Sharpness,
    Noise,
    Moire,
    Defringe,
}

pub const LP_COUNT: usize = 17;

impl LP {
    pub fn of(a: &LocalAdjust, p: LP) -> f32 {
        match p {
            LP::Temperature => a.temperature,
            LP::Tint => a.tint,
            LP::Exposure => a.exposure,
            LP::Contrast => a.contrast,
            LP::Highlights => a.highlights,
            LP::Shadows => a.shadows,
            LP::Whites => a.whites,
            LP::Blacks => a.blacks,
            LP::Texture => a.texture,
            LP::Clarity => a.clarity,
            LP::Dehaze => a.dehaze,
            LP::Hue => a.hue,
            LP::Saturation => a.saturation,
            LP::Sharpness => a.sharpness,
            LP::Noise => a.noise,
            LP::Moire => a.moire,
            LP::Defringe => a.defringe,
        }
    }

    pub const ALL: [LP; LP_COUNT] = [
        LP::Temperature,
        LP::Tint,
        LP::Exposure,
        LP::Contrast,
        LP::Highlights,
        LP::Shadows,
        LP::Whites,
        LP::Blacks,
        LP::Texture,
        LP::Clarity,
        LP::Dehaze,
        LP::Hue,
        LP::Saturation,
        LP::Sharpness,
        LP::Noise,
        LP::Moire,
        LP::Defringe,
    ];
}

/// Per-pixel sums of local parameter deltas, plus per-mask weights for the
/// non-scalar adjustments (curves, grading, colour, point colour).
#[derive(Default)]
pub struct LocalFields {
    pub fields: Vec<Option<Plane>>,
    /// (mask index, weight map) for masks with complex adjustments.
    pub weights: Vec<(usize, Plane)>,
}

impl LocalFields {
    pub fn empty() -> Self {
        Self { fields: (0..LP_COUNT).map(|_| None).collect(), weights: vec![] }
    }

    #[inline]
    pub fn has(&self, p: LP) -> bool {
        self.fields.get(p as usize).map(|f| f.is_some()).unwrap_or(false)
    }

    #[inline]
    pub fn get(&self, p: LP, i: usize) -> f32 {
        match self.fields.get(p as usize) {
            Some(Some(f)) => f.data[i],
            _ => 0.0,
        }
    }

    pub fn weight(&self, mask: usize) -> Option<&Plane> {
        self.weights.iter().find(|(m, _)| *m == mask).map(|(_, p)| p)
    }
}

pub fn has_complex(a: &LocalAdjust) -> bool {
    !a.curve.is_identity() || !a.grading.is_identity() || a.color[1] > 0.0 || !a.point_colors.is_empty()
}

/// Context needed to evaluate masks on a rendered region.
pub struct MaskEnv<'a> {
    pub geo: &'a Geometry,
    pub region: &'a Region,
    /// T-frame coords per pixel.
    pub tco: &'a [[f32; 2]],
    /// Scene image (for range masks / auto mask), display-ish encoded Oklab.
    pub lab: &'a [[f32; 3]],
    /// Optional depth map in T-frame (normalised 0..1, near = 0).
    pub depth: Option<&'a Plane>,
}

/// Evaluate a mask group's weight (0..1) per pixel.
pub fn eval_group(g: &MaskGroup, env: &MaskEnv) -> Plane {
    let (w, h) = (env.region.w, env.region.h);
    let mut acc = Plane::new(w, h);
    for (ci, comp) in g.components.iter().enumerate() {
        let mut m = eval_shape(&comp.shape, env);
        let op = comp.opacity / 100.0;
        if comp.invert {
            m.data.par_iter_mut().for_each(|v| *v = 1.0 - *v);
        }
        let op_kind = if ci == 0 { MaskOp::Add } else { comp.op };
        acc.data.par_iter_mut().zip(m.data.par_iter()).for_each(|(a, &v)| {
            let v = v * op;
            *a = match op_kind {
                MaskOp::Add => a.max(v),
                MaskOp::Subtract => *a * (1.0 - v),
                MaskOp::Intersect => *a * v,
            };
        });
    }
    if g.invert {
        acc.data.par_iter_mut().for_each(|v| *v = 1.0 - *v);
    }
    acc
}

fn aspect_xy(env: &MaskEnv, u: f32, v: f32) -> (f32, f32) {
    // T-frame normalised -> pixel-proportional units (long edge = 1)
    let long = env.geo.ow.max(env.geo.oh) as f32;
    (u * env.geo.ow as f32 / long, v * env.geo.oh as f32 / long)
}

pub fn eval_shape(shape: &MaskShape, env: &MaskEnv) -> Plane {
    let (w, h) = (env.region.w, env.region.h);
    match shape {
        MaskShape::Linear { start, end, two_sided } => {
            let (sx, sy) = aspect_xy(env, start[0], start[1]);
            let (ex, ey) = aspect_xy(env, end[0], end[1]);
            let (dx, dy) = (ex - sx, ey - sy);
            let len2 = (dx * dx + dy * dy).max(1e-9);
            let two = *two_sided;
            Plane {
                w,
                h,
                data: env
                    .tco
                    .par_iter()
                    .map(|t| {
                        let (x, y) = aspect_xy(env, t[0], t[1]);
                        let s = ((x - sx) * dx + (y - sy) * dy) / len2;
                        if two { 1.0 - smoothstep(0.0, 1.0, s.abs()) } else { 1.0 - smoothstep(0.0, 1.0, s) }
                    })
                    .collect(),
            }
        }
        MaskShape::Radial { center, radius, angle, feather } => {
            let (cx, cy) = aspect_xy(env, center[0], center[1]);
            let (rx, ry) = aspect_xy(env, radius[0], radius[1]);
            let (rx, ry) = (rx.max(1e-5), ry.max(1e-5));
            let a = angle.to_radians();
            let (sa, ca) = (a.sin(), a.cos());
            let f = (feather / 100.0).clamp(0.0, 1.0);
            Plane {
                w,
                h,
                data: env
                    .tco
                    .par_iter()
                    .map(|t| {
                        let (x, y) = aspect_xy(env, t[0], t[1]);
                        let (px, py) = (x - cx, y - cy);
                        let qx = px * ca + py * sa;
                        let qy = -px * sa + py * ca;
                        let d = ((qx / rx).powi(2) + (qy / ry).powi(2)).sqrt();
                        1.0 - smoothstep(1.0 - f, 1.0 + f * 0.0001, d)
                    })
                    .collect(),
            }
        }
        MaskShape::LuminanceRange { range, smoothness } => {
            let soft = (smoothness / 100.0 * 0.25).max(0.002);
            let (lo, hi) = (range[0].min(range[1]), range[0].max(range[1]));
            Plane {
                w,
                h,
                data: env
                    .lab
                    .par_iter()
                    .map(|l| {
                        let v = l[0];
                        smoothstep(lo - soft, lo, v) * (1.0 - smoothstep(hi, hi + soft, v))
                    })
                    .collect(),
            }
        }
        MaskShape::DepthRange { range, smoothness } => {
            let Some(depth) = env.depth else { return Plane::new(w, h) };
            let soft = (smoothness / 100.0 * 0.25).max(0.002);
            let (lo, hi) = (range[0].min(range[1]), range[0].max(range[1]));
            Plane {
                w,
                h,
                data: env
                    .tco
                    .par_iter()
                    .map(|t| {
                        let v = depth.sample_bilinear(t[0] * depth.w as f32, t[1] * depth.h as f32);
                        smoothstep(lo - soft, lo, v) * (1.0 - smoothstep(hi, hi + soft, v))
                    })
                    .collect(),
            }
        }
        MaskShape::ColorRange { samples, refine } => {
            if samples.is_empty() {
                return Plane::new(w, h);
            }
            let r = 0.02 + (1.0 - refine / 100.0) * 0.12;
            Plane {
                w,
                h,
                data: env
                    .lab
                    .par_iter()
                    .map(|l| {
                        let mut best = 0.0f32;
                        for s in samples {
                            // weight chroma more than lightness so it's a colour selector
                            let d = (0.35 * (l[0] - s[0]).powi(2) + (l[1] - s[1]).powi(2) + (l[2] - s[2]).powi(2)).sqrt();
                            best = best.max(1.0 - smoothstep(r * 0.5, r, d));
                        }
                        best
                    })
                    .collect(),
            }
        }
        MaskShape::Brush { strokes } => rasterize_brush(strokes, env),
        MaskShape::Ai { raster, feather, edge, .. } => {
            let Some(r) = raster.as_ref().and_then(|s| decode_raster(s)) else { return Plane::new(w, h) };
            let r = if *feather > 0.0 { crate::image::gaussian_blur(&r, feather / 100.0 * r.w.max(r.h) as f32 * 0.02) } else { r };
            // edge: positive grows and hardens the mask, negative shrinks it
            let e = (*edge / 100.0).clamp(-1.0, 1.0);
            let shape = |v: f32| -> f32 {
                if e == 0.0 {
                    return v;
                }
                let c = 0.5 - e * 0.35;
                let k = 1.0 + e.abs() * 3.0;
                ((v - c) * k + 0.5).clamp(0.0, 1.0)
            };
            Plane { w, h, data: env.tco.par_iter().map(|t| shape(r.sample_bilinear(t[0] * r.w as f32, t[1] * r.h as f32).clamp(0.0, 1.0))).collect() }
        }
    }
}

/// Stamp brush dabs into region pixel space.
fn rasterize_brush(strokes: &[BrushStroke], env: &MaskEnv) -> Plane {
    let (w, h) = (env.region.w, env.region.h);
    let mut m = Plane::new(w, h);
    let geo = env.geo;
    let reg = env.region;
    let long = geo.ow.max(geo.oh);
    // output px per full-res px
    let crop_w = (geo.crop.right - geo.crop.left) as f64 * (reg.x1 - reg.x0) * geo.ow;
    let scale = w as f64 / crop_w;
    for st in strokes {
        let rad_px = (st.radius as f64 * long * scale) as f32;
        if rad_px <= 0.0 {
            continue;
        }
        let feather = (st.feather / 100.0).clamp(0.0, 1.0);
        let flow = (st.flow / 100.0).clamp(0.01, 1.0);
        let density = (st.density / 100.0).clamp(0.0, 1.0);
        // densify points so dabs overlap at 1/4 radius spacing
        let mut dabs: Vec<(f32, f32)> = vec![];
        let to_px = |p: &[f32; 2]| -> (f32, f32) {
            let (cx, cy) = geo.t_to_crop(p[0] as f64, p[1] as f64);
            (((cx - reg.x0) / (reg.x1 - reg.x0) * w as f64) as f32, ((cy - reg.y0) / (reg.y1 - reg.y0) * h as f64) as f32)
        };
        let pts: Vec<(f32, f32)> = st.points.iter().map(to_px).collect();
        for (i, p) in pts.iter().enumerate() {
            if i == 0 {
                dabs.push(*p);
                continue;
            }
            let q = pts[i - 1];
            let d = ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
            let step = (rad_px * 0.25).max(0.5);
            let n = (d / step).ceil() as usize;
            for k in 1..=n {
                let t = k as f32 / n as f32;
                dabs.push((q.0 + (p.0 - q.0) * t, q.1 + (p.1 - q.1) * t));
            }
        }
        let inner = rad_px * (1.0 - feather);
        for (dx, dy) in dabs {
            let x0 = ((dx - rad_px).floor().max(0.0)) as usize;
            let x1 = ((dx + rad_px).ceil().min(w as f32 - 1.0)).max(0.0) as usize;
            let y0 = ((dy - rad_px).floor().max(0.0)) as usize;
            let y1 = ((dy + rad_px).ceil().min(h as f32 - 1.0)).max(0.0) as usize;
            if dx + rad_px < 0.0 || dy + rad_px < 0.0 || dx - rad_px > w as f32 || dy - rad_px > h as f32 {
                continue;
            }
            let center_lab = if st.auto_mask {
                let ix = (dx as usize).min(w - 1);
                let iy = (dy as usize).min(h - 1);
                Some(env.lab[iy * w + ix])
            } else {
                None
            };
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let d = ((x as f32 + 0.5 - dx).powi(2) + (y as f32 + 0.5 - dy).powi(2)).sqrt();
                    if d > rad_px {
                        continue;
                    }
                    let mut f = if d <= inner { 1.0 } else { 1.0 - smoothstep(inner, rad_px, d) };
                    if let Some(c) = center_lab {
                        let l = env.lab[y * w + x];
                        let dist = ((l[0] - c[0]).powi(2) + (l[1] - c[1]).powi(2) + (l[2] - c[2]).powi(2)).sqrt();
                        f *= 1.0 - smoothstep(0.03, 0.08, dist);
                    }
                    let v = &mut m.data[y * w + x];
                    if st.erase {
                        *v *= 1.0 - flow * f;
                    } else if *v < density {
                        *v += (density - *v) * flow * f;
                    }
                }
            }
        }
    }
    m
}

/// AI mask rasters are stored as base64 8-bit grayscale PNG.
pub fn encode_raster(p: &Plane) -> String {
    use base64::Engine;
    let buf: Vec<u8> = p.data.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
    let img = image::GrayImage::from_raw(p.w as u32, p.h as u32, buf).unwrap();
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Png).ok();
    base64::engine::general_purpose::STANDARD.encode(out.into_inner())
}

pub fn decode_raster(s: &str) -> Option<Plane> {
    use base64::Engine;
    thread_local! {
        static CACHE: std::cell::RefCell<Option<(u64, Plane)>> = const { std::cell::RefCell::new(None) };
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hs);
        hs.finish()
    };
    if let Some(p) = CACHE.with(|c| c.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, p)| p.clone())) {
        return Some(p);
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(s).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_luma8();
    let p = Plane { w: img.width() as usize, h: img.height() as usize, data: img.into_raw().into_iter().map(|v| v as f32 / 255.0).collect() };
    CACHE.with(|c| *c.borrow_mut() = Some((key, p.clone())));
    Some(p)
}

/// Build fields for all enabled masks.
pub fn build_fields(masks: &[MaskGroup], env: &MaskEnv) -> LocalFields {
    let mut lf = LocalFields::empty();
    if masks.is_empty() {
        return lf;
    }
    let n = env.region.w * env.region.h;
    for (mi, g) in masks.iter().enumerate() {
        if !g.enabled || g.components.is_empty() {
            continue;
        }
        let wmap = eval_group(g, env);
        let amt = g.amount / 100.0;
        for p in LP::ALL {
            let v = LP::of(&g.adjust, p) * amt;
            if v == 0.0 {
                continue;
            }
            let f = lf.fields[p as usize].get_or_insert_with(|| Plane::new(env.region.w, env.region.h));
            f.data.par_iter_mut().zip(wmap.data.par_iter()).for_each(|(d, &wv)| *d += v * wv);
        }
        if has_complex(&g.adjust) {
            lf.weights.push((mi, wmap));
        }
    }
    let _ = n;
    lf
}

/// Whether any enabled mask looks at pixel colours (range masks, auto mask).
pub fn needs_lab(masks: &[MaskGroup]) -> bool {
    masks.iter().filter(|m| m.enabled).flat_map(|m| m.components.iter()).any(|c| match &c.shape {
        MaskShape::ColorRange { .. } | MaskShape::LuminanceRange { .. } => true,
        MaskShape::Brush { strokes } => strokes.iter().any(|s| s.auto_mask),
        _ => false,
    })
}

/// Oklab of a display-encoded view of the scene, for range masks.
pub fn mask_lab(scene: &Image3, ok: &Oklab, curve: &crate::profiles::BaseCurve, exposure: f32) -> Vec<[f32; 3]> {
    scene
        .data
        .par_iter()
        .map(|p| {
            let y = crate::color::luma_prophoto(*p).max(1e-7);
            let e = curve.eval(super::tone::to_stops(y) + exposure);
            let yd = crate::color::srgb_decode(e.min(1.0));
            let r = yd / y;
            let lab = ok.from_prophoto([p[0] * r, p[1] * r, p[2] * r]);
            lab
        })
        .collect()
}
