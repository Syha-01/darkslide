//! The develop pipeline. `Engine` owns a decoded source and its caches and
//! renders any crop-frame region at any scale for given settings.

pub mod colorops;
pub mod curves;
pub mod detail;
pub mod effects;
pub mod geometry;
pub mod local;
pub mod tone;

use std::sync::{Arc, Mutex};

use rayon::prelude::*;

use crate::color::{self, CameraColor, Mat3, Oklab, luma_prophoto};
use crate::decode::{Source, SourceData, apply_orientation};
use crate::demosaic::{self, Quality};
use crate::image::{Image3, Plane, Rgb};
use crate::lens::{LensProfile, LensfunLens};
use crate::params::{DevelopSettings, WbMode};
use crate::profiles::{self, BaseCurve, Lut3d};

use curves::{Lut, parametric_lut, point_curve_lut};
use geometry::{Geometry, Region};
use local::{LP, LocalFields, MaskEnv};
use tone::{ToneCtx, ToneGlobals, dec, enc};

/// Long edge of the proxy used for whole-image context.
const PROXY_LONG: usize = 640;
/// Long edge of the cached small base used to render the proxy.
const SMALL_LONG: usize = 1600;

#[derive(Default)]
struct Caches {
    draft: Option<Arc<Image3>>,
    full: Option<Arc<Image3>>,
    small: Option<Arc<Image3>>,
    /// (quality is_full, retouch hash) -> retouched base
    retouched: Vec<((bool, u64), Arc<Image3>)>,
    auto_wb: Option<[f32; 2]>,
    auto_ca: Option<[f64; 2]>,
    look: Option<(String, Arc<Lut3d>)>,
    lens_db: Option<Arc<Vec<LensfunLens>>>,
    depth: Option<Arc<Plane>>,
    ctx: Option<(u64, Arc<ToneCtx>)>,
    /// (full quality, denoise amount, raw details) -> enhanced base
    enhanced: Vec<((bool, u32, bool), Arc<Image3>)>,
    dcp: Option<(String, Option<Arc<crate::dcp::Dcp>>)>,
    lcps: Option<Arc<Vec<crate::lens::LcpEntry>>>,
}

/// Hash of the settings that influence the whole-image tone context.
fn ctx_key(s: &DevelopSettings) -> u64 {
    #[derive(serde::Serialize)]
    struct K<'a> {
        wb: (crate::params::WbMode, f32, f32, bool),
        tone: [f32; 9],
        profile: (&'a str, f32),
        panels: (bool, bool, bool, bool),
        lens: &'a crate::params::LensCorrection,
        transform: &'a crate::params::Transform,
        crop: &'a crate::params::Crop,
        calib: &'a crate::params::Calibration,
        masks: &'a [crate::params::MaskGroup],
        retouch: &'a [crate::params::RetouchSpot],
    }
    hash_of(&K {
        wb: (s.wb_mode, s.temperature, s.tint, s.panels.color),
        tone: [s.exposure, s.contrast, s.highlights, s.shadows, s.whites, s.blacks, s.clarity, s.texture, s.dehaze],
        profile: (&s.profile, s.profile_amount),
        panels: (s.panels.basic, s.panels.effects, s.panels.optics, s.panels.calibration),
        lens: &s.lens,
        transform: &s.transform,
        crop: &s.crop,
        calib: &s.calibration,
        masks: &s.masks,
        retouch: &s.retouch,
    })
}

pub struct Engine {
    pub source: Arc<Source>,
    caches: Mutex<Caches>,
    ok: Oklab,
}

#[derive(Clone, Debug)]
pub struct RenderRequest {
    pub region: Region,
    pub quality: Quality,
    /// Render the full uncropped frame (crop tool view).
    pub ignore_crop: bool,
}

pub struct Rendered {
    /// Display-linear ProPhoto RGB (may exceed 1.0 in HDR mode).
    pub image: Image3,
    /// Coverage (0 outside the source after transforms).
    pub alpha: Plane,
    /// T-frame coords per pixel (for overlays).
    pub tco: Vec<[f32; 2]>,
    pub ctx: Arc<ToneCtx>,
}

/// Stage timing, reported with `RUST_LOG=darkslide_core=debug`.
struct Lap {
    t: std::time::Instant,
    marks: Vec<(&'static str, f64)>,
}

impl Lap {
    fn new() -> Self {
        Self { t: std::time::Instant::now(), marks: vec![] }
    }
    fn mark(&mut self, name: &'static str) {
        if log::log_enabled!(log::Level::Debug) {
            self.marks.push((name, self.t.elapsed().as_secs_f64() * 1000.0));
        }
    }
    fn report(&self, w: usize, h: usize) {
        if self.marks.is_empty() {
            return;
        }
        let mut last = 0.0;
        let parts: Vec<String> = self
            .marks
            .iter()
            .map(|(n, t)| {
                let d = t - last;
                last = *t;
                format!("{n} {d:.1}")
            })
            .collect();
        log::debug!("render {w}x{h}: {} (total {last:.1} ms)", parts.join(", "));
    }
}

fn hash_of<T: serde::Serialize>(v: &T) -> u64 {
    use std::hash::{Hash, Hasher};
    let s = serde_json::to_string(v).unwrap_or_default();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

impl Engine {
    pub fn new(source: Source) -> Self {
        Self { source: Arc::new(source), caches: Mutex::new(Caches::default()), ok: Oklab::new() }
    }

    pub fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        Ok(Self::new(crate::decode::load(path)?))
    }

    pub fn is_raw(&self) -> bool {
        self.source.is_raw
    }

    /// Oriented size at full resolution.
    pub fn full_size(&self) -> (usize, usize) {
        self.source.oriented_size()
    }

    /// As-shot WB multipliers (G = 1) applied before demosaic.
    fn as_shot_mul(&self) -> Rgb {
        let n = self.source.as_shot_neutral;
        [1.0 / n[0].max(1e-6), 1.0, 1.0 / n[2].max(1e-6)]
    }

    /// Demosaiced, oriented, as-shot white-balanced camera RGB.
    pub fn base(&self, q: Quality) -> Arc<Image3> {
        {
            let c = self.caches.lock().unwrap();
            let hit = match q {
                Quality::Draft => c.draft.clone().or(c.full.clone()),
                Quality::Full => c.full.clone(),
            };
            if let Some(b) = hit {
                return b;
            }
        }
        let mul = self.as_shot_mul();
        let img = match &self.source.data {
            SourceData::Cfa(cfa) => {
                let (mut img, clip) = demosaic::demosaic(cfa, mul, q);
                demosaic::reconstruct_highlights(&mut img, &clip, mul, demosaic::cfa_clip_levels(cfa));
                img
            }
            SourceData::Rgb(img) => {
                let mut i = img.clone();
                if self.source.is_raw {
                    // linear raw data clips per channel too; rebuild like mosaiced data
                    let mut mx = [0.0f32; 3];
                    for p in img.data.iter().step_by(3) {
                        for c in 0..3 {
                            mx[c] = mx[c].max(p[c]);
                        }
                    }
                    let raw_clip = mx.map(|m| if m >= 0.95 && m <= 1.05 { m } else { f32::MAX });
                    let clip = Plane {
                        w: img.w,
                        h: img.h,
                        data: img.data.par_iter().map(|p| if (0..3).any(|c| p[c] >= raw_clip[c] * 0.995) { 1.0 } else { 0.0 }).collect(),
                    };
                    i.map_inplace(|p| {
                        for c in 0..3 {
                            p[c] *= mul[c];
                        }
                    });
                    demosaic::reconstruct_highlights(&mut i, &clip, mul, raw_clip);
                }
                if q == Quality::Draft && i.w.max(i.h) > 3000 {
                    i = i.downscale_int(2);
                }
                i
            }
        };
        let img = Arc::new(apply_orientation(&img, self.source.meta.orientation));
        let mut c = self.caches.lock().unwrap();
        match q {
            Quality::Draft => c.draft = Some(img.clone()),
            Quality::Full => c.full = Some(img.clone()),
        }
        img
    }

    /// Base with Enhance (denoise / raw details) applied.
    pub fn base_for(&self, q: Quality, s: &DevelopSettings) -> Arc<Image3> {
        let b = self.base(q);
        let e = &s.enhance;
        if !e.denoise && !e.raw_details {
            return b;
        }
        let full = q == Quality::Full && b.w.max(b.h) > 3200;
        let key = (full, if e.denoise { e.denoise_amount.round() as u32 + 1 } else { 0 }, e.raw_details);
        if let Some((_, img)) = self.caches.lock().unwrap().enhanced.iter().find(|(k, _)| *k == key) {
            return img.clone();
        }
        let mut img = (*b).clone();
        if e.denoise {
            img = crate::enhance::denoise(&img, e.denoise_amount.max(1.0));
        }
        if e.raw_details && full {
            img = crate::enhance::raw_details(&img, 0.75, 8);
        }
        let img = Arc::new(img);
        let mut c = self.caches.lock().unwrap();
        c.enhanced.retain(|(k, _)| k.0 != full);
        c.enhanced.push((key, img.clone()));
        img
    }

    fn small(&self) -> Arc<Image3> {
        if let Some(s) = self.caches.lock().unwrap().small.clone() {
            return s;
        }
        let d = self.base(Quality::Draft);
        let long = d.w.max(d.h);
        let s = if long > SMALL_LONG {
            let f = long as f32 / SMALL_LONG as f32;
            Arc::new(d.resize((d.w as f32 / f) as usize, (d.h as f32 / f) as usize))
        } else {
            d
        };
        self.caches.lock().unwrap().small = Some(s.clone());
        s
    }

    /// Base with retouch spots applied (cached per spot list).
    fn retouched(&self, base: Arc<Image3>, s: &DevelopSettings, full: bool) -> Arc<Image3> {
        if s.retouch.is_empty() {
            return base;
        }
        let key_of = |spots: &[crate::params::RetouchSpot]| (full, hash_of(&(spots, &s.enhance)) ^ (base.w as u64) << 32);
        let n = s.retouch.len();
        let key = key_of(&s.retouch);
        let prefix_key = key_of(&s.retouch[..n - 1]);
        let (cached, prefix) = {
            let c = self.caches.lock().unwrap();
            let find = |k: (bool, u64)| c.retouched.iter().find(|(kk, _)| *kk == k).map(|(_, i)| i.clone());
            (find(key), find(prefix_key))
        };
        if let Some(img) = cached {
            return img;
        }
        // editing usually touches only the newest spot: start from the cached
        // result of all the others when we have it
        let prefix = match prefix {
            Some(p) => p,
            None => {
                let mut p = (*base).clone();
                crate::retouch::apply(&mut p, &s.retouch[..n - 1]);
                let p = Arc::new(p);
                self.caches.lock().unwrap().retouched.push((prefix_key, p.clone()));
                p
            }
        };
        let mut img = (*prefix).clone();
        crate::retouch::apply(&mut img, &s.retouch[n - 1..]);
        let img = Arc::new(img);
        let mut c = self.caches.lock().unwrap();
        c.retouched.push((key, img.clone()));
        // keep the prefix entries alive longest; full-res copies are big
        while c.retouched.len() > 4 {
            let drop_at = c.retouched.iter().position(|(k, _)| *k != prefix_key).unwrap_or(0);
            c.retouched.remove(drop_at);
        }
        img
    }

    // ---------------------------------------------------------------------
    // White balance

    /// Resolved white point chromaticity for the settings.
    pub fn white_xy(&self, s: &DevelopSettings) -> [f32; 2] {
        let src = &self.source;
        // a hidden Color panel shows the image as shot
        let mode = if s.panels.color { s.wb_mode } else { WbMode::AsShot };
        if !src.is_raw {
            let (t, tint) = match mode {
                WbMode::AsShot => (0.0, 0.0),
                WbMode::Auto => {
                    let xy = self.auto_wb_xy();
                    return xy;
                }
                _ => (s.temperature, s.tint),
            };
            // relative scale: +100 = about one stop warmer in mired terms
            return color::temp_tint_to_xy(6504.0 * (t / 100.0 * 0.85).exp2(), tint * 1.2);
        }
        match mode {
            WbMode::AsShot => src.as_shot_xy(),
            WbMode::Auto => self.auto_wb_xy(),
            WbMode::Custom => color::temp_tint_to_xy(s.temperature, s.tint),
            m => {
                let (t, tint) = m.preset().unwrap_or((5500.0, 10.0));
                color::temp_tint_to_xy(t, tint)
            }
        }
    }

    /// Temperature/tint that the sliders should show for the settings.
    pub fn resolved_temp_tint(&self, s: &DevelopSettings) -> (f32, f32) {
        if !self.source.is_raw {
            return match s.wb_mode {
                WbMode::AsShot => (0.0, 0.0),
                WbMode::Auto => {
                    let (t, tint) = color::xy_to_temp_tint(self.auto_wb_xy());
                    (((t / 6504.0).log2() / 0.85 * 100.0).clamp(-100.0, 100.0), (tint / 1.2).clamp(-100.0, 100.0))
                }
                _ => (s.temperature, s.tint),
            };
        }
        if s.wb_mode == WbMode::Custom {
            return (s.temperature, s.tint);
        }
        let (t, tint) = color::xy_to_temp_tint(self.white_xy(s));
        (t.round(), tint.round())
    }

    /// The DCP selected in the settings (profile "dcp:<path>"), cached.
    pub fn dcp(&self, s: &DevelopSettings) -> Option<Arc<crate::dcp::Dcp>> {
        let path = s.profile.strip_prefix("dcp:")?;
        if !self.source.is_raw {
            return None;
        }
        {
            let c = self.caches.lock().unwrap();
            if let Some((p, d)) = &c.dcp {
                if p == path {
                    return d.clone();
                }
            }
        }
        let d = crate::dcp::Dcp::load(std::path::Path::new(path)).ok().map(Arc::new);
        self.caches.lock().unwrap().dcp = Some((path.to_string(), d.clone()));
        d
    }

    pub fn base_curve(&self, s: &DevelopSettings) -> BaseCurve {
        match self.dcp(s).and_then(|d| d.tone_curve.clone()) {
            Some(tc) => BaseCurve::from_dcp(&tc),
            None => BaseCurve::new(profiles::find(&s.profile)),
        }
    }

    /// Camera(as-shot WB'd) -> scene-linear ProPhoto for the settings.
    pub fn camera_matrix(&self, s: &DevelopSettings) -> Mat3 {
        let xy = self.white_xy(s);
        let dcp = self.dcp(s);
        let dcp_color = dcp.as_ref().map(|d| d.camera_color());
        let cc: &CameraColor = dcp_color.as_ref().unwrap_or(&self.source.color);
        let n_new = cc.neutral_for_xy(xy);
        // as-shot neutral is in camera space and independent of the profile
        let n_as = self.source.as_shot_neutral;
        let n_new_cam = self.source.color.neutral_for_xy(xy);
        let n_new = if dcp.is_some() { n_new } else { n_new_cam };
        let wbfix = color::diag([n_as[0] / n_new[0].max(1e-6), n_as[1] / n_new[1].max(1e-6), n_as[2] / n_new[2].max(1e-6)]);
        let to_pp = match dcp.as_ref().and_then(|d| d.forward(color::xy_to_temp_tint(xy).0)) {
            // ForwardMatrix maps white-balanced camera RGB straight to XYZ D50
            Some(fm) => {
                let w = color::mul_mv(&fm, [1.0, 1.0, 1.0]);
                color::mul_mm(&color::xyz_d50_to_prophoto(), &color::scale_m(&fm, 1.0 / w[1].max(1e-6)))
            }
            None => cc.wb_camera_to_prophoto(xy),
        };
        let m = color::mul_mm(&to_pp, &wbfix);
        let calib = if s.panels.calibration { colorops::calibration_matrix(&s.calibration) } else { color::IDENTITY };
        color::mul_mm(&calib, &m)
    }

    /// Auto white balance: grey-world on mid-bright, unsaturated pixels,
    /// refined toward bright neutral patches.
    pub fn auto_wb_xy(&self) -> [f32; 2] {
        if let Some(xy) = self.caches.lock().unwrap().auto_wb {
            return xy;
        }
        let small = self.small();
        let cc = &self.source.color;
        let n_as = self.source.as_shot_neutral;
        // camera-raw (un-WB'd) averages in log space
        let mut acc = [0.0f64; 3];
        let mut n = 0.0f64;
        for p in small.data.iter().step_by(3) {
            let raw = [p[0] * n_as[0], p[1] * n_as[1], p[2] * n_as[2]];
            let m = raw[0].max(raw[1]).max(raw[2]);
            let lo = raw[0].min(raw[1]).min(raw[2]);
            if m > 0.9 || lo < 0.002 {
                continue;
            }
            let w = (m as f64).sqrt();
            for c in 0..3 {
                acc[c] += (raw[c] as f64).ln() * w;
            }
            n += w;
        }
        let xy = if n > 0.0 {
            let g = (acc[1] / n).exp();
            let neutral = [((acc[0] / n).exp() / g) as f32, 1.0, ((acc[2] / n).exp() / g) as f32];
            let xy = cc.xy_for_neutral(neutral);
            // keep within a sane range of the daylight locus
            let (t, tint) = color::xy_to_temp_tint(xy);
            color::temp_tint_to_xy(t.clamp(2000.0, 12000.0), tint.clamp(-60.0, 60.0))
        } else {
            self.source.as_shot_xy()
        };
        self.caches.lock().unwrap().auto_wb = Some(xy);
        xy
    }

    /// WB eyedropper: settings temp/tint that make the given crop-frame point
    /// neutral.
    pub fn wb_from_point(&self, s: &DevelopSettings, cx: f32, cy: f32) -> Option<(f32, f32)> {
        let small = self.small();
        let geo = self.geometry(s, small.w, small.h, false);
        let (tu, tv) = geo.crop_to_t(cx as f64, cy as f64);
        let (ou, ov) = geo.t_to_o(tu, tv, 1);
        let (su, sv) = geo.o_to_s(ou, ov);
        // average a small window
        let (px, py) = (su as f32 * small.w as f32, sv as f32 * small.h as f32);
        let mut acc = [0.0f32; 3];
        for dy in -2..=2 {
            for dx in -2..=2 {
                let p = small.sample_bilinear(px + dx as f32, py + dy as f32);
                for c in 0..3 {
                    acc[c] += p[c];
                }
            }
        }
        let n_as = self.source.as_shot_neutral;
        let raw = [acc[0] * n_as[0], acc[1] * n_as[1], acc[2] * n_as[2]];
        if raw[1] <= 0.0 {
            return None;
        }
        let neutral = [raw[0] / raw[1], 1.0, raw[2] / raw[1]];
        let xy = self.source.color.xy_for_neutral(neutral);
        let (t, tint) = color::xy_to_temp_tint(xy);
        if self.source.is_raw {
            Some((t.clamp(1500.0, 50000.0).round(), tint.clamp(-150.0, 150.0).round()))
        } else {
            Some((((t / 6504.0).log2() / 0.85 * 100.0).clamp(-100.0, 100.0).round(), (tint / 1.2).clamp(-100.0, 100.0).round()))
        }
    }

    // ---------------------------------------------------------------------
    // Lens

    pub fn lens_db(&self) -> Arc<Vec<LensfunLens>> {
        if let Some(db) = self.caches.lock().unwrap().lens_db.clone() {
            return db;
        }
        let db = Arc::new(crate::lens::load_lensfun_db());
        self.caches.lock().unwrap().lens_db = Some(db.clone());
        db
    }

    /// Re-read the lens database on next use (after it was installed).
    pub fn forget_lens_db(&self) {
        self.caches.lock().unwrap().lens_db = None;
    }

    pub fn lens_profile(&self, s: &DevelopSettings) -> Option<LensProfile> {
        if !s.lens.profile_enable {
            return None;
        }
        let db = self.lens_db();
        let name = match &s.lens.profile {
            Some(n) => n.clone(),
            None => self.source.meta.lens.clone()?,
        };
        let focal = self.source.meta.focal_length.unwrap_or(50.0) as f64;
        if let Some(l) = crate::lens::find_lens(&db, &name) {
            return Some(l.at(focal, self.source.meta.fnumber.unwrap_or(8.0) as f64));
        }
        // fall back to Adobe LCP profiles
        let lcps = {
            let mut c = self.caches.lock().unwrap();
            c.lcps.get_or_insert_with(|| Arc::new(crate::lens::load_lcps())).clone()
        };
        let (w, h) = self.source.sensor_size();
        crate::lens::lcp_profile(&lcps, &name, focal, w as f64, h as f64)
    }

    fn auto_ca(&self) -> [f64; 2] {
        if let Some(ca) = self.caches.lock().unwrap().auto_ca {
            return ca;
        }
        let ca = crate::lens::estimate_lateral_ca(&self.base(Quality::Draft));
        self.caches.lock().unwrap().auto_ca = Some(ca);
        ca
    }

    pub fn geometry(&self, s: &DevelopSettings, _src_w: usize, _src_h: usize, ignore_crop: bool) -> Geometry {
        let (fw, fh) = self.full_size();
        let profile = self.lens_profile(s);
        let auto_ca = if s.lens.remove_ca && s.panels.optics { Some(self.auto_ca()) } else { None };
        let lens = crate::lens::model_for(&s.lens, profile.as_ref(), auto_ca);
        let g = Geometry::new(s, fw, fh, lens);
        if ignore_crop { g.without_crop() } else { g }
    }

    fn look(&self, s: &DevelopSettings) -> Option<Arc<Lut3d>> {
        let look = s.look.as_ref()?;
        if look.amount == 0.0 {
            return None;
        }
        {
            let c = self.caches.lock().unwrap();
            if let Some((src, l)) = &c.look {
                if *src == look.source {
                    return Some(l.clone());
                }
            }
        }
        let l = Arc::new(profiles::load_look(&look.source)?);
        self.caches.lock().unwrap().look = Some((look.source.clone(), l.clone()));
        Some(l)
    }

    /// "Constrain crop": shrink the crop around its centre, as little as
    /// possible, until it only covers source pixels after transforms and lens
    /// corrections.
    pub fn constrain_crop(&self, s: &mut DevelopSettings) {
        let (fw, fh) = self.full_size();
        if self.geometry(s, fw, fh, false).crop_inside_source() {
            return;
        }
        let c0 = s.crop;
        let (cx, cy) = ((c0.left + c0.right) / 2.0, (c0.top + c0.bottom) / 2.0);
        let scaled = |k: f32| {
            let mut c = c0;
            c.left = cx - (cx - c0.left) * k;
            c.right = cx + (c0.right - cx) * k;
            c.top = cy - (cy - c0.top) * k;
            c.bottom = cy + (c0.bottom - cy) * k;
            c
        };
        // largest scale that fits
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..18 {
            let mid = (lo + hi) / 2.0;
            s.crop = scaled(mid);
            if self.geometry(s, fw, fh, false).crop_inside_source() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        s.crop = scaled(lo.max(0.02));
    }

    pub fn set_depth(&self, depth: Option<Plane>) {
        self.caches.lock().unwrap().depth = depth.map(Arc::new);
    }

    pub fn depth(&self) -> Option<Arc<Plane>> {
        self.caches.lock().unwrap().depth.clone()
    }

    /// Depth map (estimated on first use).
    pub fn depth_for(&self, s: &DevelopSettings) -> Arc<Plane> {
        if let Some(d) = self.depth() {
            return d;
        }
        let d = Arc::new(crate::ai::estimate_depth(self, s));
        self.caches.lock().unwrap().depth = Some(d.clone());
        d
    }

    // ---------------------------------------------------------------------
    // Rendering

    /// Camera RGB -> scene-linear ProPhoto incl. exposure baseline and local
    /// temperature/tint.
    fn to_scene(&self, cam: &Image3, m: &Mat3, fields: &LocalFields, s: &DevelopSettings) -> Image3 {
        let local_wb = fields.has(LP::Temperature) || fields.has(LP::Tint);
        let mut out = Image3 { w: cam.w, h: cam.h, data: cam.data.par_iter().map(|p| color::mul_mv(m, *p)).collect() };
        if let Some(d) = self.dcp(s) {
            let temp = color::xy_to_temp_tint(self.white_xy(s)).0;
            let hsm = d.hue_sat_for(temp);
            let look = d.look.clone();
            if hsm.is_some() || look.is_some() {
                out.data.par_iter_mut().for_each(|p| {
                    let mut q = *p;
                    if let Some(t) = &hsm {
                        q = t.apply(q);
                    }
                    if let Some(t) = &look {
                        q = t.apply(q);
                    }
                    *p = q;
                });
            }
        }
        if local_wb {
            out.data.par_iter_mut().enumerate().for_each(|(i, p)| {
                let t = fields.get(LP::Temperature, i) / 100.0;
                let ti = fields.get(LP::Tint, i) / 100.0;
                if t != 0.0 || ti != 0.0 {
                    // warm/cool along blue-amber; tint along green-magenta
                    let r = (t * 0.45 + ti * 0.2).exp2();
                    let g = (-ti * 0.35).exp2();
                    let b = (-t * 0.55 + ti * 0.2).exp2();
                    let y0 = luma_prophoto(*p);
                    let mut q = [p[0] * r, p[1] * g, p[2] * b];
                    let y1 = luma_prophoto(q).max(1e-7);
                    let k = y0 / y1;
                    q = q.map(|v| v * k);
                    *p = q;
                }
            });
        }
        out.data.iter_mut().for_each(|p| *p = p.map(|v| v.max(0.0)));
        out
    }

    fn baseline_ev(&self, s: &DevelopSettings) -> f32 {
        if !self.source.is_raw {
            return 0.0;
        }
        let dcp_off = self.dcp(s).map(|d| d.baseline_exposure_offset).unwrap_or(0.0);
        0.45 + self.source.baseline_exposure + dcp_off
    }

    /// Tone globals from settings.
    fn tone_globals(&self, s: &DevelopSettings) -> ToneGlobals {
        let b = s.panels.basic;
        let pick = |v: f32| if b { v } else { 0.0 };
        let fx = |v: f32| if s.panels.effects { v } else { 0.0 };
        ToneGlobals {
            exposure: pick(s.exposure) + self.baseline_ev(s),
            contrast: pick(s.contrast),
            highlights: pick(s.highlights),
            shadows: pick(s.shadows),
            whites: pick(s.whites),
            blacks: pick(s.blacks),
            clarity: fx(s.clarity),
            texture: fx(s.texture),
            dehaze: fx(s.dehaze),
            hdr: s.hdr.enabled,
            hdr_headroom: s.hdr.max_headroom,
        }
    }

    /// Whole-image context from the proxy (cached per relevant settings).
    pub fn tone_ctx(&self, s: &DevelopSettings) -> Arc<ToneCtx> {
        let key = ctx_key(s);
        if let Some((k, c)) = &self.caches.lock().unwrap().ctx {
            if *k == key {
                return c.clone();
            }
        }
        let c = self.compute_tone_ctx(s);
        self.caches.lock().unwrap().ctx = Some((key, c.clone()));
        c
    }

    fn compute_tone_ctx(&self, s: &DevelopSettings) -> Arc<ToneCtx> {
        let small = self.small();
        let small = self.retouched(small, s, false);
        let geo = self.geometry(s, small.w, small.h, false);
        let (cw, ch) = geo.crop_size();
        let f = PROXY_LONG as f64 / cw.max(ch);
        let region = Region::full(((cw * f).round() as usize).max(8), ((ch * f).round() as usize).max(8));
        let (cam, _alpha, tco) = geometry::warp(&small, &geo, &region);
        let m = self.camera_matrix(s);
        let profile = profiles::find(&s.profile);
        let curve = self.base_curve(s);
        let g = self.tone_globals(s);
        let scene0 = self.to_scene(&cam, &m, &LocalFields::empty(), s);
        let lab = if local::needs_lab(&s.masks) { local::mask_lab(&scene0, &self.ok, &curve, g.exposure) } else { vec![[0.0; 3]; scene0.data.len()] };
        let depth = self.depth();
        let env = MaskEnv { geo: &geo, region: &region, tco: &tco, lab: &lab, depth: depth.as_deref() };
        let fields = local::build_fields(&s.masks, &env);
        let mut scene = self.to_scene(&cam, &m, &fields, s);
        if s.panels.optics {
            effects::lens_vignette(&mut scene, &s.lens, &tco, (geo.ow / geo.oh) as f32);
        }
        let needs_haze = g.dehaze != 0.0 || fields.has(LP::Dehaze);
        let haze = if needs_haze { Some(tone::estimate_haze(&scene)) } else { None };
        if let Some((d, a)) = &haze {
            scene.data.par_iter_mut().enumerate().for_each(|(i, p)| {
                let amt = ((g.dehaze + fields.get(LP::Dehaze, i)) / 100.0).clamp(-1.0, 1.0);
                *p = tone::dehaze_px(*p, d.data[i], *a, amt);
            });
        }
        let adaptive = profile.adaptive;
        let exp = |i: usize| g.exposure + fields.get(LP::Exposure, i);
        let mut ctx = tone::build_ctx(&scene, &exp, haze, adaptive);
        if adaptive {
            let amt = s.profile_amount / 100.0;
            ctx.adaptive = ctx.adaptive.map(|v| v * amt);
        }
        Arc::new(ctx)
    }

    /// Render a region of the crop frame.
    pub fn render(&self, s: &DevelopSettings, req: &RenderRequest) -> Rendered {
        let ctx = self.tone_ctx(s);
        self.render_with_ctx(s, req, ctx)
    }

    pub fn render_with_ctx(&self, s: &DevelopSettings, req: &RenderRequest, ctx: Arc<ToneCtx>) -> Rendered {
        let mut lap = Lap::new();
        let (fw, fh) = self.full_size();
        let mut geo = self.geometry(s, fw, fh, req.ignore_crop);
        if req.ignore_crop {
            geo = geo.without_crop();
        }
        let region = req.region;
        // output px per full-res px
        let (cw, _) = geo.crop_size();
        let scale = (region.w as f64 / (cw * (region.x1 - region.x0))) as f32;
        let quality = if scale > 0.45 { Quality::Full } else { req.quality };
        let base = self.base_for(quality, s);
        let base = self.retouched(base, s, quality == Quality::Full);
        lap.mark("base");
        let (cam, alpha, tco) = geometry::warp(&base, &geo, &region);
        lap.mark("warp");

        let m = self.camera_matrix(s);
        let profile = profiles::find(&s.profile);
        let curve = self.base_curve(s);
        let g = self.tone_globals(s);

        // masks are evaluated on an un-adjusted rendering of the scene
        let scene0 = self.to_scene(&cam, &m, &LocalFields::empty(), s);
        let lab = if local::needs_lab(&s.masks) { local::mask_lab(&scene0, &self.ok, &curve, g.exposure) } else { vec![[0.0; 3]; scene0.data.len()] };
        let needs_depth = s.masks.iter().any(|m| m.components.iter().any(|c| matches!(c.shape, crate::params::MaskShape::DepthRange { .. })));
        let depth = if needs_depth { Some(self.depth_for(s)) } else { self.depth() };
        let env = MaskEnv { geo: &geo, region: &region, tco: &tco, lab: &lab, depth: depth.as_deref() };
        let fields = local::build_fields(&s.masks, &env);
        lap.mark("scene+masks");
        let mut img = if fields.has(LP::Temperature) || fields.has(LP::Tint) { self.to_scene(&cam, &m, &fields, s) } else { scene0 };
        drop(lab);

        let aspect_o = (geo.ow / geo.oh) as f32;
        if s.panels.optics {
            effects::lens_vignette(&mut img, &s.lens, &tco, aspect_o);
            if let Some(p) = self.lens_profile(s) {
                crate::lens::apply_profile_vignette(&mut img, &p, s.lens.vignette_scale as f64 / 100.0, &tco, aspect_o);
            }
        }
        if s.panels.detail {
            detail::noise_reduction(&mut img, &s.nr, &fields, scale, &self.ok);
        }
        if s.panels.optics {
            effects::defringe(&mut img, &s.lens, &fields, &self.ok);
        }

        let crop_xy = |i: usize| -> (f32, f32) {
            let (x, y) = (i % region.w, i / region.w);
            let (cx, cy) = region.crop_coord(x, y);
            (cx as f32, cy as f32)
        };
        let geo_cropped = if req.ignore_crop { Some(self.geometry(s, fw, fh, false)) } else { None };
        let ctx_coords = |i: usize| -> (f32, f32) {
            // ctx was built on the cropped frame; for the crop tool view map back
            if let Some(g2) = &geo_cropped {
                let t = tco[i];
                let (cx, cy) = g2.t_to_crop(t[0] as f64, t[1] as f64);
                (cx as f32, cy as f32)
            } else {
                crop_xy(i)
            }
        };
        let ctx_cache: Vec<(f32, f32)> = (0..region.w * region.h).into_par_iter().map(ctx_coords).collect();
        let ctx_lookup = |i: usize| ctx_cache[i];
        lap.mark("optics+nr");
        tone::apply(&mut img, &ctx_lookup, &ctx, &g, &fields, &curve, scale);
        lap.mark("tone");

        let bw = s.is_bw() || profile.monochrome;
        let cp = colorops::ColorParams { s, profile, profile_amount: if profile.adaptive { 1.0 } else { s.profile_amount / 100.0 }, ok: &self.ok, bw };
        colorops::apply(&mut img, &cp, &fields);
        lap.mark("color");

        self.apply_curves_and_look(&mut img, s, &fields);
        lap.mark("curves");

        if s.lens_blur.enabled && s.panels.optics {
            let depth = self.depth_for(s);
            let d: Vec<f32> = tco.par_iter().map(|t| depth.sample_bilinear(t[0] * depth.w as f32, t[1] * depth.h as f32)).collect();
            if s.lens_blur.visualize_depth {
                img.data.par_iter_mut().zip(d.par_iter()).for_each(|(p, v)| {
                    let g = crate::color::srgb_decode(1.0 - v);
                    *p = [g, g, g];
                });
            } else {
                let (fw, fh) = self.full_size();
                let max_r = (fw.max(fh) as f32 * 0.012 * s.lens_blur.amount / 100.0 * scale).max(0.0);
                effects::lens_blur(&mut img, &d, &s.lens_blur, max_r);
            }
        }
        if s.panels.effects {
            let (cw, ch) = geo.crop_size();
            effects::post_crop_vignette(&mut img, &s.vignette, &crop_xy, (cw / ch) as f32);
        }
        if s.panels.detail {
            detail::sharpen(&mut img, &s.sharpen, &fields, scale);
        }
        if s.panels.effects {
            let (cw, ch) = geo.crop_size();
            let full_xy = |i: usize| {
                let (cx, cy) = crop_xy(i);
                (cx * cw as f32, cy * ch as f32)
            };
            effects::grain(&mut img, &s.grain, &full_xy, 1.0 / scale);
        }
        if !s.red_eye.is_empty() {
            crate::redeye::apply(&mut img, &s.red_eye, &tco, &self.ok);
        }

        lap.mark("effects+sharpen");
        lap.report(region.w, region.h);
        Rendered { image: img, alpha, tco, ctx }
    }

    fn apply_curves_and_look(&self, img: &mut Image3, s: &DevelopSettings, fields: &LocalFields) {
        let curve_on = s.panels.curve;
        let p = if curve_on { parametric_lut(&s.parametric) } else { Lut::identity() };
        let rgb = if curve_on { point_curve_lut(&s.curve.rgb) } else { Lut::identity() };
        let main = rgb.after(&p);
        let r = if curve_on { point_curve_lut(&s.curve.red) } else { Lut::identity() };
        let gg = if curve_on { point_curve_lut(&s.curve.green) } else { Lut::identity() };
        let b = if curve_on { point_curve_lut(&s.curve.blue) } else { Lut::identity() };
        let refine = s.refine_saturation / 100.0;
        let look = self.look(s);
        let look_amt = s.look.as_ref().map(|l| l.amount / 100.0).unwrap_or(0.0);
        let local_curves: Vec<(usize, [Lut; 4])> = fields
            .weights
            .iter()
            .filter(|(mi, _)| !s.masks[*mi].adjust.curve.is_identity())
            .map(|(mi, _)| {
                let c = &s.masks[*mi].adjust.curve;
                (*mi, [point_curve_lut(&c.rgb), point_curve_lut(&c.red), point_curve_lut(&c.green), point_curve_lut(&c.blue)])
            })
            .collect();
        let global_identity = main.is_identity() && r.is_identity() && gg.is_identity() && b.is_identity();
        if global_identity && look.is_none() && local_curves.is_empty() {
            return;
        }
        let to_srgb = color::prophoto_to(color::ColorSpace::SRgb);
        let from_srgb = color::to_prophoto(color::ColorSpace::SRgb);
        let apply_set = |px: Rgb, luts: [&Lut; 4], refine: f32| -> Rgb {
            let e = px.map(enc);
            let pc = [luts[1].eval(luts[0].eval(e[0])), luts[2].eval(luts[0].eval(e[1])), luts[3].eval(luts[0].eval(e[2]))];
            if refine >= 1.0 {
                return pc.map(dec);
            }
            let y = luma_prophoto(px).max(1e-7);
            let el = enc(y);
            let k = dec(luts[0].eval(el)) / y;
            let lumv = px.map(|v| v * k);
            let per = pc.map(dec);
            [lumv[0] + (per[0] - lumv[0]) * refine, lumv[1] + (per[1] - lumv[1]) * refine, lumv[2] + (per[2] - lumv[2]) * refine]
        };
        img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
            let mut o = *px;
            if !global_identity {
                o = apply_set(o, [&main, &r, &gg, &b], refine);
            }
            for (mi, luts) in &local_curves {
                let wmap = fields.weight(*mi).unwrap();
                let w = wmap.data[i] * s.masks[*mi].amount / 100.0;
                if w > 0.0 {
                    let c = apply_set(o, [&luts[0], &luts[1], &luts[2], &luts[3]], 1.0);
                    for k in 0..3 {
                        o[k] += (c[k] - o[k]) * w.min(1.0);
                    }
                }
            }
            if let Some(l) = &look {
                let srgb = color::mul_mv(&to_srgb, o).map(|v| color::srgb_encode(v.clamp(0.0, 1.0)));
                let lv = l.eval(srgb);
                let mixed = [0, 1, 2].map(|k| srgb[k] + (lv[k] - srgb[k]) * look_amt);
                let lin = mixed.map(|v| color::srgb_decode(v.clamp(0.0, 1.0)));
                let back = color::mul_mv(&from_srgb, lin);
                // keep HDR headroom outside the LUT's range
                let y0 = luma_prophoto(o);
                if y0 > 1.0 {
                    let k = y0 / luma_prophoto(back).max(1e-6);
                    o = back.map(|v| v * k);
                } else {
                    o = back;
                }
            }
            *px = o.map(|v| v.max(0.0));
        });
    }

    /// Convenience: full-frame render fitted into `max_w` x `max_h`.
    pub fn render_fit(&self, s: &DevelopSettings, max_w: usize, max_h: usize, quality: Quality, ignore_crop: bool) -> Rendered {
        let (fw, fh) = self.full_size();
        let mut geo = self.geometry(s, fw, fh, ignore_crop);
        if ignore_crop {
            geo = geo.without_crop();
        }
        let (cw, ch) = geo.crop_size();
        let f = (max_w as f64 / cw).min(max_h as f64 / ch).min(1.0);
        let region = Region::full(((cw * f).round() as usize).max(1), ((ch * f).round() as usize).max(1));
        self.render(s, &RenderRequest { region, quality, ignore_crop })
    }

    /// Statistics-based auto tone values: [exposure, contrast, highlights,
    /// shadows, whites, blacks].
    pub fn auto_tone(&self, s: &DevelopSettings) -> [f32; 6] {
        let mut s2 = s.clone();
        s2.exposure = 0.0;
        s2.contrast = 0.0;
        s2.highlights = 0.0;
        s2.shadows = 0.0;
        s2.whites = 0.0;
        s2.blacks = 0.0;
        let ctx = self.tone_ctx(&s2);
        let mut v = tone::auto_tone(&ctx.stats);
        // the stats already include the baseline exposure
        v[0] = (v[0] * 20.0).round() / 20.0;
        v
    }
}

// ---------------------------------------------------------------------------
// HDR -> SDR

/// Tone-map an HDR rendering (values up to 2^headroom) to SDR using the SDR
/// settings (brightness, contrast, highlights, shadows, whites, blend).
pub fn hdr_to_sdr(img: &Image3, h: &crate::params::HdrSettings) -> Image3 {
    let peak = h.max_headroom.exp2();
    let bright = (h.sdr_brightness / 100.0).exp2();
    let knee = (0.6 + h.sdr_highlights / 100.0 * 0.3 + h.sdr_whites / 100.0 * 0.1).clamp(0.3, 0.95);
    let contrast = h.sdr_contrast;
    let shadows = h.sdr_shadows / 100.0;
    let blend = h.sdr_blend / 100.0;
    img.map(|p| {
        let y = luma_prophoto(p).max(1e-7);
        let yb = y * bright;
        // soft shoulder from the knee up to the HDR peak
        let ym = if yb <= knee {
            yb
        } else {
            let t = (yb - knee) / (peak * bright - knee).max(1e-3);
            knee + (1.0 - knee) * (1.0 - (-3.0 * t).exp()) / (1.0 - (-3.0f32).exp())
        };
        let mut e = enc(ym.min(1.0));
        e = tone::contrast_curve(e, contrast);
        if shadows != 0.0 && e < 0.5 {
            e += shadows * 0.1 * (1.0 - e / 0.5).powi(2);
        }
        let clipped = enc(yb.min(1.0));
        let e = e + (clipped - e) * blend;
        let r = dec(e.clamp(0.0, 1.0)) / y;
        let o = p.map(|c| c * r);
        gamut_clip(o, 1.0)
    })
}

// ---------------------------------------------------------------------------
// Output conversion

/// Hue-preserving gamut clip: pull out-of-range colours toward their luminance.
#[inline]
pub fn gamut_clip(p: Rgb, max: f32) -> Rgb {
    let y = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let y = y.clamp(0.0, max);
    let mut k = 1.0f32;
    for c in 0..3 {
        if p[c] > max {
            k = k.min((max - y) / (p[c] - y).max(1e-7));
        } else if p[c] < 0.0 {
            k = k.min(y / (y - p[c]).max(1e-7));
        }
    }
    let k = k.clamp(0.0, 1.0);
    [y + (p[0] - y) * k, y + (p[1] - y) * k, y + (p[2] - y) * k]
}

/// Display-linear ProPhoto -> encoded output space, f32 0..1 (or above for HDR).
pub fn to_output(img: &Image3, space: color::ColorSpace, hdr: bool) -> Image3 {
    let m = color::prophoto_to(space);
    let max = if hdr { f32::MAX } else { 1.0 };
    img.map(|p| {
        let q = gamut_clip(color::mul_mv(&m, p), max);
        if hdr { q } else { q.map(|v| space.encode(v)) }
    })
}

/// To 8-bit sRGB (with alpha composited over `bg`) for display.
pub fn to_srgb8(img: &Image3, alpha: Option<&Plane>, bg: [u8; 3]) -> Vec<u8> {
    let m = color::prophoto_to(color::ColorSpace::SRgb);
    let bgf = bg.map(|v| v as f32 / 255.0);
    let mut out = vec![0u8; img.w * img.h * 4];
    out.par_chunks_mut(4).enumerate().for_each(|(i, px)| {
        let q = gamut_clip(color::mul_mv(&m, img.data[i]), 1.0);
        let a = alpha.map(|a| a.data[i]).unwrap_or(1.0);
        for c in 0..3 {
            let v = color::srgb_encode(q[c]) * a + bgf[c] * (1.0 - a);
            // ordered dither would be nicer; rounding is fine for preview
            px[c] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
        px[3] = 255;
    });
    out
}

pub use crate::demosaic::Quality as RenderQuality;
